//! The real thing, end to end: the `craft` binary generates plugins from the templates, builds
//! them with cargo for wasm32, inspects them through the plugin-abi gate, and runs them in the
//! plugin-host wasmi engine. Nothing here is mocked. The first build compiles the SDK for wasm32,
//! so the first run of this file takes a while.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::OnceLock;
use std::sync::atomic::AtomicBool;
use std::sync::mpsc;
use std::time::Duration;

use serde_json::{Value, json};

fn craft_bin() -> &'static str {
    env!("CARGO_BIN_EXE_craft")
}

/// One cargo target directory for every plugin these tests build, so the SDK compiles once.
fn shared_target() -> PathBuf {
    Path::new(env!("CARGO_TARGET_TMPDIR")).join("plugin-target")
}

fn work_dir(name: &str) -> PathBuf {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("e2e").join(name);
    if dir.exists() {
        std::fs::remove_dir_all(&dir).unwrap();
    }
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn craft(args: &[&str], cwd: &Path) -> Output {
    Command::new(craft_bin()).args(args).current_dir(cwd).env("CARGO_TARGET_DIR", shared_target()).output().expect("run craft")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(out: &Output) -> String {
    assert!(out.status.success(), "exit {:?}\nstdout:\n{}\nstderr:\n{}", out.status.code(), text(&out.stdout), text(&out.stderr));
    text(&out.stdout)
}

/// A plugin with all four kinds, created and built once for the whole file.
fn everything() -> &'static PathBuf {
    static DIR: OnceLock<PathBuf> = OnceLock::new();
    DIR.get_or_init(|| {
        let parent = work_dir("everything");
        ok(&craft(&["new", "everything", "--kind", "command,hook,effect,panel"], &parent));
        let dir = parent.join("everything");
        let built = ok(&craft(&["build"], &dir));
        assert!(built.contains("gate:         ACCEPTED"), "{built}");
        dir
    })
}

fn wasm() -> PathBuf {
    everything().join("target/everything.wasm")
}

fn run_plugin(args: &[&str]) -> Output {
    let w = wasm();
    let mut all = vec!["run", "--headless", "--app", "demo", w.to_str().unwrap()];
    all.extend_from_slice(args);
    craft(&all, everything())
}

#[test]
fn new_writes_the_package_and_refuses_to_overwrite() {
    let parent = work_dir("new");
    let out = ok(&craft(&["new", "demo", "--kind", "command,panel", "--id", "org.test.demo"], &parent));
    assert!(out.contains("created"), "{out}");
    let dir = parent.join("demo");
    for f in ["Cargo.toml", "craft.toml", "src/lib.rs", ".cargo/config.toml", ".gitignore", "Cargo.lock"] {
        assert!(dir.join(f).is_file(), "missing {f}");
    }
    let manifest = std::fs::read_to_string(dir.join("craft.toml")).unwrap();
    assert!(manifest.contains("id = \"org.test.demo\""));
    assert!(manifest.contains("kind = \"command\"") && manifest.contains("kind = \"panel\""));
    assert!(!manifest.contains("kind = \"effect\""));
    assert!(manifest.contains("required = [\"ui.contribute\", \"state.local\"]"), "{manifest}");
    assert!(!std::fs::read_to_string(dir.join("src/lib.rs")).unwrap().contains("{{"), "no template marker left");

    let again = craft(&["new", "demo"], &parent);
    assert!(!again.status.success());
    assert!(text(&again.stderr).contains("already exists"), "{}", text(&again.stderr));

    for bad in ["Bad_Name", "9x", "", "a-"] {
        let out = craft(&["new", bad], &parent);
        assert!(!out.status.success(), "{bad:?} should be refused");
    }
    let out = craft(&["new", "k", "--kind", "bogus"], &parent);
    assert!(text(&out.stderr).contains("unknown template kind"));
}

#[test]
fn build_makes_a_wasm_the_gate_accepts() {
    let bytes = std::fs::read(wasm()).unwrap();
    assert_eq!(&bytes[..4], b"\0asm");
    let report = craft_plugin_abi::check_package(&bytes, &craft_plugin_abi::HostPolicy::native());
    assert!(report.is_accepted(), "{:?}", report.violations);
    let m = report.manifest.unwrap();
    assert_eq!(m.id, "dev.local.everything");
    let kinds: Vec<&str> = m.contributions.iter().map(|c| c.kind.name()).collect();
    assert_eq!(kinds, ["command", "hook", "effect", "panel"]);
    assert_eq!(m.contributions.iter().map(|c| c.entry).collect::<Vec<_>>(), [0, 1, 2, 3]);
}

#[test]
fn inspect_reports_manifest_ops_permissions_and_exports() {
    let out = craft(&["inspect", "--json", wasm().to_str().unwrap()], everything());
    let v: Value = serde_json::from_str(&ok(&out)).unwrap();
    assert_eq!(v["accepted"], true);
    assert_eq!(v["violations"], json!([]));
    assert_eq!(v["manifest"]["id"], "dev.local.everything");
    assert_eq!(v["manifest"]["contributions"].as_array().unwrap().len(), 4);
    assert_eq!(v["manifest"]["permissions"]["required"], json!(["commands.observe", "ui.contribute", "state.local"]));
    let exports: Vec<&str> = v["exports"].as_array().unwrap().iter().map(|e| e.as_str().unwrap()).collect();
    for e in ["memory", "craft_abi", "craft_alloc", "craft_free", "craft_invoke", "craft_process"] {
        assert!(exports.contains(&e), "{e} not exported: {exports:?}");
    }
    assert_eq!(v["imports"], json!(["craft_host.call"]));
    assert_eq!(v["memories"][0]["maximum_pages"], 1024);

    let text_out = ok(&craft(&["inspect", wasm().to_str().unwrap()], everything()));
    for want in ["gate:         ACCEPTED", "commands.observe", "[normal prompt]", "state.put", "via craft_process", "maximum 1024 pages = 64 MiB"] {
        assert!(text_out.contains(want), "{want:?} missing from:\n{text_out}");
    }
}

#[test]
fn inspect_exits_1_on_a_module_the_gate_rejects() {
    let dir = work_dir("inspect-reject");
    let path = dir.join("bad.wasm");
    std::fs::write(&path, wat::parse_str("(module (memory 1))").unwrap()).unwrap();
    let out = craft(&["inspect", path.to_str().unwrap()], &dir);
    assert_eq!(out.status.code(), Some(1));
    let t = text(&out.stdout);
    assert!(t.contains("REJECTED") && t.contains("craft.manifest is missing"), "{t}");
}

#[test]
fn check_prints_one_verdict_per_file_and_exits_1_if_any_is_rejected() {
    let good = wasm();
    let out = ok(&craft(&["check", good.to_str().unwrap()], everything()));
    assert_eq!(out.trim(), format!("{}: ACCEPTED", good.display()));

    let dir = work_dir("check-reject");
    let bad = dir.join("bad.wasm");
    std::fs::write(&bad, wat::parse_str("(module (memory 1))").unwrap()).unwrap();
    let out = craft(&["check", good.to_str().unwrap(), bad.to_str().unwrap(), dir.join("missing.wasm").to_str().unwrap()], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stdout = text(&out.stdout);
    assert!(stdout.contains("ACCEPTED") && stdout.contains("REJECTED") && stdout.contains("craft.manifest is missing"), "{stdout}");
    assert!(text(&out.stderr).contains("missing.wasm"), "an unreadable file is reported: {}", text(&out.stderr));

    assert!(!craft(&["check"], &dir).status.success(), "at least one file is required");
}

#[test]
fn run_calls_a_command_through_the_registry_and_prints_its_json() {
    let out = run_plugin(&["--invoke", "everything.say", r#"{"name":"Ada"}"#]);
    let stdout = ok(&out);
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), json!({ "greeting": "hello, Ada" }));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("[dev.local.everything info] saying hello to Ada"), "{stderr}");
    assert!(stderr.contains("granted commands.observe, ui.contribute, state.local"), "the headless grant is announced: {stderr}");
    assert!(stderr.contains("[call end] completed"), "{stderr}");
}

#[test]
fn run_accepts_the_bare_contribution_id_and_defaults_params_to_an_empty_object() {
    let stdout = ok(&run_plugin(&["--invoke", "say"]));
    assert_eq!(serde_json::from_str::<Value>(&stdout).unwrap(), json!({ "greeting": "hello, world" }));
}

#[test]
fn run_sends_a_panel_its_request_and_prints_the_tree() {
    let stdout = ok(&run_plugin(&["--invoke", "counter", r#"{"event":"action","widget":"bump"}"#]));
    let v: Value = serde_json::from_str(&stdout).unwrap();
    assert_eq!(v["tree"]["children"][0]["text"], "Clicked 1 times", "the panel read and wrote state.local through the real host");
}

#[test]
fn run_sends_a_hook_its_request() {
    let out = run_plugin(&["--invoke", "log-commands", r#"{"call":{"id":"clip.add"},"outcome":"ok"}"#]);
    assert_eq!(serde_json::from_str::<Value>(&ok(&out)).unwrap(), json!({}));
    assert!(text(&out.stderr).contains("clip.add finished: ok"));
}

#[test]
fn run_explains_what_it_cannot_run_and_fails() {
    let out = run_plugin(&["--invoke", "invert"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("craft_process"), "{}", text(&out.stderr));

    let out = run_plugin(&["--invoke", "nope"]);
    assert_eq!(out.status.code(), Some(1));
    let e = text(&out.stderr);
    assert!(e.contains("no contribution matches") && e.contains("say (command)") && e.contains("invert (effect)"), "{e}");
}

#[test]
fn run_without_headless_or_invoke_is_refused() {
    let w = wasm();
    let out = craft(&["run", w.to_str().unwrap(), "--invoke", "say"], everything());
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("only --headless is implemented"));
    let out = craft(&["run", "--headless", w.to_str().unwrap()], everything());
    assert!(!out.status.success(), "--invoke is required");
}

#[test]
fn grant_only_accepts_optional_permissions() {
    let out = run_plugin(&["--invoke", "say", "--grant", "state.local"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("does not list it as optional"), "{}", text(&out.stderr));
    let out = run_plugin(&["--invoke", "say", "--grant", "not.a.permission"]);
    assert_eq!(out.status.code(), Some(1));
}

#[test]
fn a_plugin_that_traps_is_stopped_and_reported_not_hung() {
    // A command that panics in the guest: wasm32 aborts, which the host sees as a trap.
    let parent = work_dir("panics");
    ok(&craft(&["new", "boom"], &parent));
    let dir = parent.join("boom");
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        lib.replace("let name =", "if call.params.get(\"panic\").is_some() { panic!(\"on purpose\"); }\n        let name ="),
    )
    .unwrap();
    ok(&craft(&["build"], &dir));
    let out = craft(&["run", "--headless", "target/boom.wasm", "--invoke", "say", r#"{"panic":true}"#], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = text(&out.stderr);
    assert!(stderr.contains("plugin panicked: panicked at"), "the panic hook reports through host.log: {stderr}");
    assert!(stderr.contains("on purpose"), "{stderr}");
    assert!(stderr.contains("faulted"), "the call lifecycle reports the fault: {stderr}");
}

#[test]
fn the_generated_plugin_tests_pass_with_cargo_test() {
    let out = Command::new("cargo")
        .args(["test", "--manifest-path"])
        .arg(everything().join("Cargo.toml"))
        .env("CARGO_TARGET_DIR", shared_target().join("native"))
        .output()
        .unwrap();
    let t = text(&out.stdout);
    assert!(out.status.success(), "{t}\n{}", text(&out.stderr));
    assert!(t.contains("test result: ok. 5 passed"), "all five template tests ran: {t}");
}

#[test]
fn build_rejects_a_module_the_gate_rejects_and_removes_the_old_output() {
    let parent = work_dir("nomax");
    ok(&craft(&["new", "nomax"], &parent));
    let dir = parent.join("nomax");
    ok(&craft(&["build"], &dir));
    assert!(dir.join("target/nomax.wasm").is_file());

    // Without the linker flag the module has no declared memory maximum, which the gate refuses.
    std::fs::remove_file(dir.join(".cargo/config.toml")).unwrap();
    let out = craft(&["build"], &dir);
    assert_eq!(out.status.code(), Some(1));
    let t = text(&out.stdout);
    assert!(t.contains("REJECTED") && t.contains("memory has no declared maximum"), "{t}");
    assert!(!dir.join("target/nomax.wasm").exists(), "a rejected build must not leave an older good module in place");
}

#[test]
fn build_outside_a_plugin_says_so() {
    let dir = work_dir("not-a-plugin");
    let out = craft(&["build"], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("not a plugin package"), "{}", text(&out.stderr));
}

#[test]
fn build_reports_a_compile_error_from_cargo() {
    let parent = work_dir("broken-source");
    ok(&craft(&["new", "broken"], &parent));
    let dir = parent.join("broken");
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    std::fs::write(dir.join("src/lib.rs"), lib.replace("fn say(", "fn not_say(")).unwrap();
    let out = craft(&["build"], &dir);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).contains("no function `say`"), "the macro error reaches the developer: {}", text(&out.stderr));
}

/// Drive the raw ABI of the built module in the wasmi engine, the way `craft_process` is called
/// by the host. The command-line `run` cannot reach the data plane, so this is the test for it.
mod data_plane {
    use super::*;
    use craft_plugin_abi::descriptor::{Alpha, Kind, Sample};
    use craft_plugin_abi::{HostPolicy, ProcessDescriptor, check_package};
    use craft_plugin_host::engine::wasmi::{WasmiEngine, WasmiInstance};
    use craft_plugin_host::engine::{Budget, CompileOptions, Export, HostDispatch, HostResult, Instance, Trust};
    use std::sync::Arc;

    fn instance() -> (WasmiInstance, Budget) {
        let bytes = std::fs::read(wasm()).unwrap();
        let report = check_package(&bytes, &HostPolicy::native());
        let engine = WasmiEngine::system();
        let module = engine.compile_sync(&bytes, &report, CompileOptions { trust: Trust::Untrusted, stack_bytes: 256 * 1024 }).unwrap();
        let host: HostDispatch = Arc::new(|_op, _inp: &[u8], _out: &mut [u8]| -> HostResult { Ok(0) });
        let budget = Budget::new(64 * 1024 * 1024, 256 * 1024);
        (module.instantiate_sync(host, &budget).unwrap(), budget)
    }

    fn alloc(i: &mut WasmiInstance, b: &Budget, data: &[u8]) -> u32 {
        let ptr = i.call(Export::Alloc, &[data.len() as i32, 1], b).unwrap() as u32;
        assert_ne!(ptr, 0);
        i.write(ptr, data).unwrap();
        ptr
    }

    fn descriptor(entry: u32, in_ptr: u32, out_ptr: u32, params: (u32, u32)) -> ProcessDescriptor {
        ProcessDescriptor {
            kind: Kind::Image,
            entry,
            w: 2,
            h: 1,
            stride_bytes: 8,
            channels: 4,
            sample: Sample::U8,
            alpha: Alpha::Straight,
            color_mode: 0,
            depth: 8,
            origin_x: 0,
            origin_y: 0,
            overlap: 0,
            scale: 1.0,
            time: 0.0,
            sample_rate: 0,
            frames: 0,
            in_ptr,
            out_ptr,
            params_ptr: params.0,
            params_len: params.1,
        }
    }

    #[test]
    fn craft_process_inverts_pixels_in_guest_memory() {
        let (mut i, b) = instance();
        let input = alloc(&mut i, &b, &[10, 20, 30, 255, 0, 128, 255, 7]);
        let output = alloc(&mut i, &b, &[0; 8]);
        let params_json = br#"{"amount":1.0}"#;
        let params = alloc(&mut i, &b, params_json);
        let desc = descriptor(2, input, output, (params, params_json.len() as u32));
        let desc_ptr = alloc(&mut i, &b, &desc.encode());
        let rc = i.call(Export::Process, &[desc_ptr as i32, 96], &b).unwrap();
        assert_eq!(rc, 0);
        let mut out = [0u8; 8];
        i.read(output, &mut out).unwrap();
        assert_eq!(out, [245, 235, 225, 255, 255, 127, 0, 7]);
    }

    #[test]
    fn craft_process_refuses_overlapping_buffers_a_wrong_entry_and_a_bad_descriptor() {
        let (mut i, b) = instance();
        let buf = alloc(&mut i, &b, &[1; 8]);
        // Output aliases input: the guest would hold &[u8] and &mut [u8] over the same bytes.
        let desc_ptr = alloc(&mut i, &b, &descriptor(2, buf, buf, (0, 0)).encode());
        assert_eq!(i.call(Export::Process, &[desc_ptr as i32, 96], &b).unwrap(), 3, "BAD_POINTER");

        let other = alloc(&mut i, &b, &[0; 8]);
        let desc_ptr = alloc(&mut i, &b, &descriptor(0, buf, other, (0, 0)).encode());
        assert_eq!(i.call(Export::Process, &[desc_ptr as i32, 96], &b).unwrap(), 4, "entry 0 is a command, not a data plane entry");

        let junk = alloc(&mut i, &b, &[0xFF; 96]);
        assert_eq!(i.call(Export::Process, &[junk as i32, 96], &b).unwrap(), 2, "BAD_DESCRIPTOR");

        // A pointer past the end of memory.
        let desc_ptr = alloc(&mut i, &b, &descriptor(2, 0x7FFF_0000, other, (0, 0)).encode());
        assert_eq!(i.call(Export::Process, &[desc_ptr as i32, 96], &b).unwrap(), 3);
    }

    #[test]
    fn craft_alloc_follows_the_abi_for_zero_and_impossible_sizes() {
        let (mut i, b) = instance();
        assert_ne!(i.call(Export::Alloc, &[0, 8], &b).unwrap(), 0, "a zero length block is a non-null address");
        assert_eq!(i.call(Export::Alloc, &[0x7000_0000, 1], &b).unwrap(), 0, "an allocation the memory maximum cannot hold returns 0");
        assert_eq!(i.call(Export::Alloc, &[16, 3], &b).unwrap(), 0, "a non power of two alignment returns 0");
        assert_eq!(i.call(Export::Abi, &[], &b).unwrap(), 1);
    }

    #[test]
    fn craft_free_returns_memory_so_a_loop_of_calls_does_not_grow() {
        let (mut i, b) = instance();
        // The first block makes the allocator ask for memory. Measure after that.
        let warm = i.call(Export::Alloc, &[4096, 1], &b).unwrap() as i32;
        i.call(Export::Free, &[warm, 4096, 1], &b).unwrap();
        let before = i.mem_len();
        for _ in 0..2000 {
            let p = i.call(Export::Alloc, &[4096, 1], &b).unwrap() as i32;
            i.call(Export::Free, &[p, 4096, 1], &b).unwrap();
        }
        assert_eq!(i.mem_len(), before, "2000 alloc/free pairs of 4 KiB reuse the same block");
    }
}

/// The developer loop, whole: edit source, `watch` rebuilds with cargo, the build lands in the dev
/// folder, the dev folder watcher reloads it in the plugin host, and the new code answers.
#[test]
fn edit_rebuild_reload_loop_changes_what_the_plugin_answers() {
    use craft_cli::devfolder::{DevEvent, DevFolder};
    use craft_cli::run::{DevHost, Headless};
    use craft_cli::watch::{WatchOptions, watch};
    use std::sync::Arc;

    let parent = work_dir("loop");
    ok(&craft(&["new", "loopy"], &parent));
    let dir = parent.join("loopy");
    let folder = parent.join("dev-folder");
    std::fs::create_dir_all(&folder).unwrap();

    let host = Arc::new(Headless::new("devtest", false));
    let mut dev = DevFolder::new(&folder, DevHost { host: Arc::clone(&host), optional: Vec::new() }).unwrap();
    let stop_dev = Arc::new(AtomicBool::new(false));
    let (dev_tx, dev_rx) = mpsc::channel();
    let stop_in_dev = Arc::clone(&stop_dev);
    let dev_thread = std::thread::spawn(move || dev.watch(&stop_in_dev, Duration::from_secs(60), |events| drop(dev_tx.send(events))));

    let stop_watch = Arc::new(AtomicBool::new(false));
    let (build_tx, build_rx) = mpsc::channel();
    let stop_in_watch = Arc::clone(&stop_watch);
    let opts = WatchOptions { dir: dir.clone(), install_into: Some(folder.clone()), debounce: Duration::from_millis(200) };
    let watch_thread = std::thread::spawn(move || {
        watch(&opts, &stop_in_watch, |result| {
            let accepted = result.as_ref().map(|o| o.accepted()).map_err(ToString::to_string);
            let _ = build_tx.send(accepted);
        })
    });

    // First build and first load.
    assert_eq!(build_rx.recv_timeout(Duration::from_secs(600)).expect("first build"), Ok(true));
    let events = dev_rx.recv_timeout(Duration::from_secs(30)).expect("first load");
    assert!(matches!(events.as_slice(), [DevEvent::Loaded { installed, .. }] if installed.generation == 1), "{events:?}");
    let first = host.invoke("dev.local.loopy", "say", &json!({ "name": "Ada" })).unwrap();
    assert_eq!(first, json!({ "greeting": "hello, Ada" }));

    // The developer edits the source.
    let lib = std::fs::read_to_string(dir.join("src/lib.rs")).unwrap();
    assert!(lib.contains("\"hello, {name}\""));
    std::fs::write(dir.join("src/lib.rs"), lib.replace("\"hello, {name}\"", "\"howdy, {name}\"")).unwrap();

    assert_eq!(build_rx.recv_timeout(Duration::from_secs(600)).expect("rebuild after the edit"), Ok(true));
    let events = dev_rx.recv_timeout(Duration::from_secs(30)).expect("reload after the rebuild");
    assert!(
        matches!(events.as_slice(), [DevEvent::Loaded { installed, .. }] if installed.generation == 2 && installed.replaced == Some(1)),
        "{events:?}"
    );
    let second = host.invoke("dev.local.loopy", "say", &json!({ "name": "Ada" })).unwrap();
    assert_eq!(second, json!({ "greeting": "howdy, Ada" }), "the running plugin is the edited one");

    // A compile error does not stop the loop and does not unload the working version.
    std::fs::write(dir.join("src/lib.rs"), lib.replace("fn say(", "fn not_say(")).unwrap();
    let broken = build_rx.recv_timeout(Duration::from_secs(600)).expect("build after the break");
    assert!(broken.is_err(), "{broken:?}");
    assert_eq!(host.invoke("dev.local.loopy", "say", &json!({})).unwrap(), json!({ "greeting": "howdy, world" }));

    stop_watch.store(true, std::sync::atomic::Ordering::Relaxed);
    stop_dev.store(true, std::sync::atomic::Ordering::Relaxed);
    // Touch both watched places so each loop wakes up and sees its flag.
    std::fs::write(dir.join("src/lib.rs"), lib).unwrap();
    std::fs::write(folder.join("wake.wasm"), b"x").unwrap();
    watch_thread.join().unwrap().unwrap();
    dev_thread.join().unwrap().unwrap();
}

#[test]
fn bindgen_output_compiles_into_a_plugin_for_wasm32() {
    let parent = work_dir("bindgen");
    ok(&craft(&["new", "withbind"], &parent));
    let dir = parent.join("withbind");
    let fixture = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/filmcraft-commands.json");
    let out =
        craft(&["bindgen", "--app", "filmcraft", "--from", fixture.to_str().unwrap(), "--out", dir.join("src/commands.rs").to_str().unwrap()], &dir);
    ok(&out);
    assert!(text(&out.stderr).contains("wrote 45 commands"), "{}", text(&out.stderr));

    // A handler that calls a generated function, so the code is exercised and not just compiled.
    std::fs::write(
        dir.join("src/lib.rs"),
        r#"use craft_plugin_sdk::craft_plugin;

mod commands;

#[craft_plugin]
mod plugin {
    use super::commands;
    use craft_plugin_sdk::prelude::*;

    fn say(_call: CommandCall) -> Result<Value, String> {
        let made = commands::file_new_bin(&commands::FileNewBinParams { name: json!("x"), parent: None });
        Ok(json!({ "bin": made.map_err(|e| e.to_string()).err() }))
    }
}
"#,
    )
    .unwrap();
    let manifest = std::fs::read_to_string(dir.join("craft.toml")).unwrap();
    let manifest =
        manifest.replace("ops = [\"host.log\"]", "ops = [\"host.log\", \"cmd.execute\"]") + "\n[permissions]\nrequired = [\"commands.run:file.*\"]\n";
    std::fs::write(dir.join("craft.toml"), manifest).unwrap();
    let built = ok(&craft(&["build"], &dir));
    assert!(built.contains("ACCEPTED"), "{built}");

    // The headless host has no app commands, so the host answers "unknown id" (E_BAD_ARG). That error
    // can only come back if the typed client sent a request the host parsed and looked up.
    let out = craft(&["run", "--headless", "target/withbind.wasm", "--invoke", "say", r#"{"name":"x"}"#], &dir);
    let v: Value = serde_json::from_str(&ok(&out)).unwrap();
    let bin = v["bin"].as_str().unwrap_or_default();
    assert!(bin.contains("E_BAD_ARG"), "{v}");
}
