//! Builds a small plugin for wasm32 with `cargo`, runs the package through the install gate, then
//! runs it in a wasmi instance with a fake `craft_host.call`.
//!
//! This is the only test that compiles and executes the `unsafe` wasm32 glue (`backend::imp`,
//! `rt::wasm` and the macro exports). It needs the `wasm32-unknown-unknown` target. When the target is missing the
//! test prints why and returns, so `cargo test --workspace` still passes on a host without it.
//! The fixture crate is written from this file into `CARGO_TARGET_TMPDIR`, so no wasm artifact
//! lives in the repository.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::OnceLock;

use craft_plugin_abi::descriptor::{Alpha, Kind, Sample};
use craft_plugin_abi::ops::{pack_call_return, unpack_invoke_return};
use craft_plugin_abi::{HostPolicy, ProcessDescriptor, Status, check_package};
use serde_json::{Value, json};
use wasmi::{Caller, Engine, Extern, Instance, Linker, Memory, Module, Store};

const TARGET: &str = "wasm32-unknown-unknown";

const CRAFT_TOML: &str = r#"id = "org.example.wasm-fixture"
name = "Wasm fixture"
version = "0.1.0"

[requires]
ops = ["host.log"]

[[contributions]]
kind = "command"
id = "say"
label = "Say"
handler = "say"

[[contributions]]
kind = "hook"
id = "watch"
phase = "after"
handler = "watch"

[[contributions]]
kind = "effect"
id = "invert"
handler = "invert"
"#;

const LIB_RS: &str = r#"use craft_plugin_sdk::prelude::*;

#[craft_plugin(manifest = "craft.toml")]
mod plugin {
    use craft_plugin_sdk::prelude::*;

    fn say(call: CommandCall) -> Result<Value, String> {
        host::info("hello");
        Ok(json!({ "said": call.params }))
    }

    fn watch(_hook: HookCall) -> HookVerdict {
        HookVerdict::Pass
    }

    fn invert(band: &mut Band) -> Result<(), String> {
        for (src, dst) in band.input.iter().zip(band.output.iter_mut()) {
            *dst = 255 - *src;
        }
        Ok(())
    }
}
"#;

// The gate rejects a memory with no declared maximum, and the linker writes one only when asked.
const CARGO_CONFIG: &str = r#"[target.wasm32-unknown-unknown]
rustflags = ["-C", "link-arg=--max-memory=67108864", "-C", "link-arg=-zstack-size=262144"]
"#;

fn target_installed() -> bool {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_owned());
    let Ok(out) = Command::new(rustc).args(["--print", "target-libdir", "--target", TARGET]).output() else {
        return false;
    };
    let dir = PathBuf::from(String::from_utf8_lossy(&out.stdout).trim());
    out.status.success() && std::fs::read_dir(&dir).is_ok_and(|mut d| d.next().is_some())
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

fn fixture_manifest() -> String {
    let sdk = env!("CARGO_MANIFEST_DIR");
    format!(
        "[package]\nname = \"wasm-fixture\"\nversion = \"0.1.0\"\nedition = \"2024\"\npublish = false\n\n\
         [lib]\ncrate-type = [\"cdylib\"]\n\n\
         [dependencies]\ncraft-plugin-sdk = {{ path = {sdk:?} }}\n\n\
         [lints.rust]\nunsafe_code = \"deny\"\n\n\
         [profile.release]\nopt-level = \"s\"\nlto = true\ncodegen-units = 1\npanic = \"abort\"\n\n\
         [workspace]\n"
    )
}

fn build_fixture(dir: &Path) -> PathBuf {
    write(&dir.join("Cargo.toml"), &fixture_manifest());
    write(&dir.join("craft.toml"), CRAFT_TOML);
    write(&dir.join("src/lib.rs"), LIB_RS);
    write(&dir.join(".cargo/config.toml"), CARGO_CONFIG);
    // Start from the workspace lock file so the fixture resolves to the versions the tests use.
    let lock = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.lock");
    if let Ok(text) = std::fs::read_to_string(lock) {
        write(&dir.join("Cargo.lock"), &text);
    }
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_owned());
    let status = Command::new(cargo)
        .current_dir(dir)
        .args(["build", "--release", "--target", TARGET])
        .env("CARGO_TARGET_DIR", dir.join("target"))
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .status()
        .expect("cargo starts");
    assert!(status.success(), "the fixture plugin must build for {TARGET}");
    dir.join("target").join(TARGET).join("release").join("wasm_fixture.wasm")
}

/// The fixture plugin, built once per test process. `None` when the wasm32 target is missing.
fn built_module() -> Option<Vec<u8>> {
    static MODULE: OnceLock<Option<Vec<u8>>> = OnceLock::new();
    MODULE
        .get_or_init(|| {
            if !target_installed() {
                eprintln!("skipped: the {TARGET} target is not installed (rustup target add {TARGET})");
                return None;
            }
            let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("wasm-fixture");
            Some(std::fs::read(build_fixture(&dir)).expect("the build wrote a module"))
        })
        .clone()
}

#[test]
fn a_plugin_built_for_wasm32_passes_the_install_gate() {
    let Some(wasm) = built_module() else { return };
    let report = check_package(&wasm, &HostPolicy::native());
    assert!(report.violations.is_empty(), "the gate rejected the module: {:?}", report.violations);
    let manifest = report.manifest.expect("the gate parsed craft.manifest");
    assert_eq!(manifest.id, "org.example.wasm-fixture");
    assert_eq!(manifest.contributions.len(), 3);
}

/// A running guest and the log lines it sent through `host.log`.
struct Guest {
    store: Store<Vec<String>>,
    memory: Memory,
    instance: Instance,
}

/// `craft_host.call` for the test: serves `host.log` (op 1) and answers `{}`.
fn host_call(mut caller: Caller<'_, Vec<String>>, op: i32, in_ptr: i32, in_len: i32, out_ptr: i32, out_cap: i32) -> i64 {
    let Some(memory) = caller.get_export("memory").and_then(Extern::into_memory) else {
        return pack_call_return(Status::BadArg, 0);
    };
    let (start, len) = (in_ptr as usize, in_len as usize);
    let input = memory.data(&caller).get(start..start + len).map(<[u8]>::to_vec).unwrap_or_default();
    if op != 1 {
        return pack_call_return(Status::Unsupported, 0);
    }
    let message = serde_json::from_slice::<Value>(&input).ok().and_then(|v| v["msg"].as_str().map(str::to_owned)).unwrap_or_default();
    caller.data_mut().push(message);
    let (out, cap) = (out_ptr as usize, out_cap as usize);
    match memory.data_mut(&mut caller).get_mut(out..out + cap.min(2)) {
        Some(dst) if dst.len() == 2 => {
            dst.copy_from_slice(b"{}");
            pack_call_return(Status::Ok, 2)
        }
        _ => pack_call_return(Status::TooSmall, 2),
    }
}

fn start(wasm: &[u8]) -> Guest {
    let engine = Engine::default();
    let module = Module::new(&engine, wasm).expect("the module validates");
    let mut store = Store::new(&engine, Vec::new());
    let mut linker = Linker::<Vec<String>>::new(&engine);
    linker.func_wrap("craft_host", "call", host_call).expect("the import links");
    let instance = linker.instantiate_and_start(&mut store, &module).expect("the module starts");
    let memory = instance.get_memory(&store, "memory").expect("memory is exported");
    Guest { store, memory, instance }
}

impl Guest {
    fn alloc(&mut self, bytes: &[u8]) -> u32 {
        let alloc = self.instance.get_typed_func::<(i32, i32), i32>(&self.store, "craft_alloc").expect("craft_alloc");
        let ptr = alloc.call(&mut self.store, (bytes.len() as i32, 8)).expect("craft_alloc runs");
        assert_ne!(ptr, 0, "craft_alloc failed");
        self.memory.write(&mut self.store, ptr as usize, bytes).expect("the block is inside memory");
        ptr as u32
    }

    fn read(&self, ptr: u32, len: u32) -> Vec<u8> {
        self.memory.data(&self.store)[ptr as usize..ptr as usize + len as usize].to_vec()
    }

    /// `craft_invoke`, then read and free the response. `Err` is the negative status.
    fn invoke(&mut self, entry: i32, request: &Value) -> Result<Option<Value>, i64> {
        let bytes = serde_json::to_vec(request).unwrap();
        let req = self.alloc(&bytes);
        let invoke = self.instance.get_typed_func::<(i32, i32, i32, i32, i32), i64>(&self.store, "craft_invoke").expect("craft_invoke");
        let packed = invoke.call(&mut self.store, (entry, req as i32, bytes.len() as i32, 0, 0)).expect("craft_invoke does not trap");
        if packed == 0 {
            return Ok(None);
        }
        let (ptr, len) = unpack_invoke_return(packed)?;
        let body = self.read(ptr, len);
        let free = self.instance.get_typed_func::<(i32, i32, i32), ()>(&self.store, "craft_free").expect("craft_free");
        free.call(&mut self.store, (ptr as i32, len as i32, 1)).expect("craft_free runs");
        Ok(Some(serde_json::from_slice(&body).expect("the response is JSON")))
    }

    /// `craft_process` with the pointers given in `desc`. Returns the status.
    fn process(&mut self, desc: &ProcessDescriptor) -> i32 {
        let raw = self.alloc(&desc.encode());
        let process = self.instance.get_typed_func::<(i32, i32), i32>(&self.store, "craft_process").expect("craft_process");
        process.call(&mut self.store, (raw as i32, desc.encode().len() as i32)).expect("craft_process does not trap")
    }
}

fn band(entry: u32, w: u32, ptrs: (u32, u32)) -> ProcessDescriptor {
    ProcessDescriptor {
        kind: Kind::Image,
        entry,
        w,
        h: 1,
        stride_bytes: w,
        channels: 1,
        sample: Sample::U8,
        alpha: Alpha::None,
        color_mode: 0,
        depth: 8,
        origin_x: 0,
        origin_y: 0,
        overlap: 0,
        scale: 1.0,
        time: 0.0,
        sample_rate: 0,
        frames: 0,
        in_ptr: ptrs.0,
        out_ptr: ptrs.1,
        params_ptr: 0,
        params_len: 0,
    }
}

#[test]
fn the_exports_run_in_an_engine() {
    let Some(wasm) = built_module() else { return };
    let mut guest = start(&wasm);
    let abi = guest.instance.get_typed_func::<(), i32>(&guest.store, "craft_abi").expect("craft_abi");
    assert_eq!(abi.call(&mut guest.store, ()).unwrap(), 1);

    let out = guest.invoke(0, &json!({ "params": { "a": 1 }, "origin": "ui" })).unwrap();
    assert_eq!(out, Some(json!({ "result": { "said": { "a": 1 } } })));
    assert_eq!(guest.store.data().as_slice(), ["hello"], "the command logged through craft_host.call");

    let hook = guest.invoke(1, &json!({ "call": { "id": "x" }, "outcome": "ok" })).unwrap();
    assert_eq!(hook, Some(json!({})), "an after hook answers an empty object");
}

#[test]
fn a_bad_entry_and_bad_json_come_back_as_statuses() {
    let Some(wasm) = built_module() else { return };
    let mut guest = start(&wasm);
    assert_eq!(guest.invoke(2, &json!({})).unwrap_err(), i64::from(Status::Unsupported.code()), "an effect is not a craft_invoke entry");
    assert_eq!(guest.invoke(99, &json!({})).unwrap_err(), i64::from(Status::Unsupported.code()));
    let junk = guest.alloc(b"{not json");
    let invoke = guest.instance.get_typed_func::<(i32, i32, i32, i32, i32), i64>(&guest.store, "craft_invoke").unwrap();
    let packed = invoke.call(&mut guest.store, (0, junk as i32, 9, 0, 0)).unwrap();
    assert_eq!(packed, i64::from(Status::BadArg.code()));
    let wild = invoke.call(&mut guest.store, (0, i32::MAX, 16, 0, 0)).unwrap();
    assert_eq!(wild, i64::from(Status::BadArg.code()), "a request outside memory is refused, not read");
}

#[test]
fn craft_process_runs_a_band_and_refuses_bad_pointers() {
    let Some(wasm) = built_module() else { return };
    let mut guest = start(&wasm);
    let input = guest.alloc(&[0, 10, 200, 255]);
    let output = guest.alloc(&[7; 4]);
    assert_eq!(guest.process(&band(2, 4, (input, output))), 0);
    assert_eq!(guest.read(output, 4), [255, 245, 55, 0]);

    assert_eq!(guest.process(&band(2, 4, (input, input + 2))), 3, "the output may not overlap the input");
    assert_eq!(guest.process(&band(2, 4, (input, 0))), 3, "address 0 is not a valid output");
    assert_eq!(guest.process(&band(2, 4, (input, u32::MAX - 1))), 3, "a range past the end of memory is refused");
    assert_eq!(guest.process(&band(0, 4, (input, output))), 4, "a command is not a craft_process entry");
}

#[test]
fn craft_alloc_and_free_round_trip() {
    let Some(wasm) = built_module() else { return };
    let mut guest = start(&wasm);
    let alloc = guest.instance.get_typed_func::<(i32, i32), i32>(&guest.store, "craft_alloc").unwrap();
    let free = guest.instance.get_typed_func::<(i32, i32, i32), ()>(&guest.store, "craft_free").unwrap();
    let ptr = alloc.call(&mut guest.store, (64, 8)).unwrap();
    assert_ne!(ptr, 0);
    assert_eq!(ptr % 8, 0, "the block has the requested alignment");
    free.call(&mut guest.store, (ptr, 64, 8)).unwrap();
    assert_ne!(alloc.call(&mut guest.store, (0, 4)).unwrap(), 0, "a zero length block is a non-null dangling address");
    assert_eq!(alloc.call(&mut guest.store, (16, 3)).unwrap(), 0, "an alignment that is not a power of two fails");
}
