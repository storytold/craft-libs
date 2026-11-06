//! Benchmark: one call of a 1080p pixel kernel on wasmi, wasmtime, and native Rust.
//!
//! Run in release mode:
//!
//!     cargo run --release -p craft-plugin-host --features wasmtime --example engines_bench -- 5 > build1.csv
//!     cargo run --release -p craft-plugin-host --features wasmtime,indirect-dispatch --example engines_bench -- 5 > build2.csv
//!
//! CSV goes to stdout. Notes (load average, governor) go to stderr. Every timed output is compared
//! to the native result, and a mismatch aborts the run, so a CSV row means the bytes were right.
//! The plugins go straight to `Engine::compile` with an accepting report. The policy gate is not
//! what this measures. `tests/conformance.rs` checks both engines against the same expected outcomes.

#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[path = "../tests/fixtures/conformance/reference.rs"]
mod reference;

use std::future::Future;
use std::sync::Arc;
use std::task::{Context, Poll, Waker};
use std::time::{Duration, Instant};

use craft_plugin_abi::PolicyReport;
use craft_plugin_abi::descriptor::{Alpha, Kind, ProcessDescriptor, Sample};
use craft_plugin_host::engine::wasmi::WasmiEngine;
use craft_plugin_host::engine::wasmtime::WasmtimeEngine;
use craft_plugin_host::engine::{Budget, CompileOptions, Engine, Export, HostDispatch, HostResult, Instance, Module, Trust};
use reference::{Image, blur_reference, noise_image, posterize_reference};

const W: u32 = 1920;
const H: u32 = 1080;
const LEVELS: u32 = 4;
const MIB: u64 = 1024 * 1024;
const STACK: u32 = 64 * 1024;
const FRESH_REPS: usize = 200;
const COMPILE_REPS: usize = 5;

fn block_on<F: Future>(fut: F) -> F::Output {
    let mut fut = std::pin::pin!(fut);
    let mut cx = Context::from_waker(Waker::noop());
    match fut.as_mut().poll(&mut cx) {
        Poll::Ready(v) => v,
        Poll::Pending => panic!("engine future was pending"),
    }
}

fn wasmi_label() -> &'static str {
    if cfg!(feature = "indirect-dispatch") { "wasmi-portable+indirect" } else { "wasmi-portable" }
}

fn build_label() -> &'static str {
    if cfg!(feature = "indirect-dispatch") { "build2" } else { "build1" }
}

fn engines() -> Vec<(&'static str, Box<dyn Engine>)> {
    vec![(wasmi_label(), Box::new(WasmiEngine::system())), ("wasmtime-48", Box::new(WasmtimeEngine::system().expect("epoch thread")))]
}

fn wat(name: &str) -> Vec<u8> {
    let path = format!("{}/tests/fixtures/conformance/{name}.wat", env!("CARGO_MANIFEST_DIR"));
    wat::parse_str(std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).expect("wat")
}

fn accept() -> PolicyReport {
    PolicyReport { violations: vec![], legacy: None, manifest: None }
}

fn no_host() -> HostDispatch {
    Arc::new(|_op, _inp: &[u8], _out: &mut [u8]| -> HostResult { Ok(0) })
}

fn ns(d: Duration) -> u128 {
    d.as_nanos()
}

struct Kernel {
    name: &'static str,
    plugin: &'static str,
    params: Vec<u8>,
}

struct Timing {
    copy_in: Duration,
    call: Duration,
    copy_out: Duration,
    out: Vec<u8>,
}

/// One full cycle on a fresh instance: allocate, copy in, call, copy out.
fn cycle(module: &dyn Module, img: &Image, params: &[u8], budget: &Budget) -> Timing {
    let mut inst: Box<dyn Instance> = block_on(module.instantiate(no_host(), budget)).expect("instantiate");
    let len = img.pixels.len();
    let alloc = |inst: &mut dyn Instance, n: usize| -> u32 {
        let p = inst.call(Export::Alloc, &[n as i32, 4], budget).expect("alloc");
        assert_ne!(p, 0, "guest allocation of {n} bytes failed");
        p as u32
    };
    let desc_ptr = alloc(inst.as_mut(), 96);
    let in_ptr = alloc(inst.as_mut(), len);
    let out_ptr = alloc(inst.as_mut(), len);
    let params_ptr = alloc(inst.as_mut(), params.len().max(1));
    let t0 = Instant::now();
    inst.write(in_ptr, &img.pixels).expect("write pixels");
    let copy_in = t0.elapsed();
    inst.write(params_ptr, params).expect("write params");
    let desc = ProcessDescriptor {
        kind: Kind::Image,
        entry: 0,
        w: img.w,
        h: img.h,
        stride_bytes: img.stride,
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
        params_ptr,
        params_len: params.len() as u32,
    };
    inst.write(desc_ptr, &desc.encode()).expect("write descriptor");
    let t1 = Instant::now();
    let code = inst.call(Export::Process, &[desc_ptr as i32, 96], budget).expect("craft_process");
    let call = t1.elapsed();
    assert_eq!(code, 0, "kernel returned an error code");
    let mut out = vec![0u8; len];
    let t2 = Instant::now();
    inst.read(out_ptr, &mut out).expect("read pixels");
    Timing { copy_in, call, copy_out: t2.elapsed(), out }
}

/// Native posterize, written the way a Rust effect would be: a lookup table and one pass.
fn posterize_native(img: &Image, levels: u32) -> Vec<u8> {
    let mut out = vec![0u8; img.pixels.len()];
    let lut: Vec<u8> = (0..256u32).map(|v| (((v * levels) >> 8) * 255 / (levels - 1)) as u8).collect();
    let row_bytes = img.w as usize * 4;
    for (src, dst) in img.pixels.chunks(img.stride as usize).zip(out.chunks_mut(img.stride as usize)) {
        for (s, d) in src[..row_bytes].chunks_exact(4).zip(dst[..row_bytes].chunks_exact_mut(4)) {
            d[0] = lut[s[0] as usize];
            d[1] = lut[s[1] as usize];
            d[2] = lut[s[2] as usize];
            d[3] = s[3];
        }
    }
    out
}

/// Native blur with sliding window sums, the same algorithm as the plugin, so bytes match.
fn blur_native(img: &Image) -> Vec<u8> {
    let (w, h, stride) = (img.w as usize, img.h as usize, img.stride as usize);
    let rowb = w * 4;
    let mut tmp = vec![0u8; rowb * h];
    for y in 0..h {
        let row = &img.pixels[y * stride..y * stride + rowb];
        let trow = &mut tmp[y * rowb..(y + 1) * rowb];
        for c in 0..4 {
            let at = |x: i64| u32::from(row[(x.clamp(0, w as i64 - 1) as usize) * 4 + c]);
            let mut sum: u32 = (-4..=4).map(at).sum();
            for x in 0..w {
                trow[x * 4 + c] = (sum / 9) as u8;
                sum = sum + at(x as i64 + 5) - at(x as i64 - 4);
            }
        }
    }
    let mut sums = vec![0u32; rowb];
    for k in -4i64..=4 {
        let yy = k.clamp(0, h as i64 - 1) as usize;
        for (s, t) in sums.iter_mut().zip(&tmp[yy * rowb..(yy + 1) * rowb]) {
            *s += u32::from(*t);
        }
    }
    let mut out = vec![0u8; img.pixels.len()];
    for y in 0..h {
        let add = &tmp[(y + 5).min(h - 1) * rowb..][..rowb];
        let sub = &tmp[y.saturating_sub(4) * rowb..][..rowb];
        let dst = &mut out[y * stride..y * stride + rowb];
        for i in 0..rowb {
            dst[i] = (sums[i] / 9) as u8;
            sums[i] = sums[i] + u32::from(add[i]) - u32::from(sub[i]);
        }
    }
    out
}

fn note(msg: &str) {
    eprintln!("# {msg}");
}

fn loadavg() -> String {
    std::fs::read_to_string("/proc/loadavg").unwrap_or_default().trim().to_owned()
}

fn governor() -> String {
    std::fs::read_to_string("/sys/devices/system/cpu/cpu0/cpufreq/scaling_governor").unwrap_or_default().trim().to_owned()
}

fn main() {
    let reps: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(5);
    note(&format!("{} reps={reps} loadavg_start={} governor={}", build_label(), loadavg(), governor()));
    let img = noise_image(W, H, 0, 0xE1E1_E1E1);
    let kernels = [
        Kernel { name: "posterize", plugin: "kernel-posterize", params: LEVELS.to_le_bytes().to_vec() },
        Kernel { name: "blur9x9", plugin: "kernel-blur", params: vec![] },
    ];
    // Truth for the output check. The straightforward references are slow on 1080p blur but run once.
    let truth_posterize = posterize_reference(&img, LEVELS);
    let truth_blur = blur_reference(&img);
    assert_eq!(posterize_native(&img, LEVELS), truth_posterize, "native posterize differs from the reference");
    assert_eq!(blur_native(&img), truth_blur, "native blur differs from the reference");
    let truth = |kernel: &str| if kernel == "posterize" { &truth_posterize } else { &truth_blur };

    println!("build,kind,engine,kernel,rep,call_ns,copy_in_ns,copy_out_ns");
    let opts = CompileOptions { trust: Trust::Untrusted, stack_bytes: STACK };
    let budget = Budget::new(96 * MIB, STACK);
    let report = accept();
    let engines = engines();
    let mut modules: Vec<(&str, &str, Arc<dyn Module>)> = Vec::new();
    for (label, engine) in &engines {
        for k in &kernels {
            let bytes = wat(k.plugin);
            let module = block_on(engine.compile(&bytes, &report, opts)).expect("compile");
            // Warm-up: one untimed full cycle, checked.
            let warm = cycle(module.as_ref(), &img, &k.params, &budget);
            assert!(&warm.out == truth(k.name), "{label} {} warm-up output differs from the reference", k.name);
            modules.push((label, k.name, module));
        }
    }
    for rep in 0..reps {
        for (label, kernel, module) in &modules {
            let params = &kernels.iter().find(|k| k.name == *kernel).expect("kernel").params;
            let t = cycle(module.as_ref(), &img, params, &budget);
            assert!(&t.out == truth(kernel), "{label} {kernel} rep {rep} output differs from the reference");
            println!("{},rep,{label},{kernel},{rep},{},{},{}", build_label(), ns(t.call), ns(t.copy_in), ns(t.copy_out));
        }
        let t = Instant::now();
        let out = posterize_native(&img, LEVELS);
        let d = t.elapsed();
        assert!(&out == truth("posterize"));
        println!("{},rep,native-rust,posterize,{rep},{},0,0", build_label(), ns(d));
        let t = Instant::now();
        let out = blur_native(&img);
        let d = t.elapsed();
        assert!(&out == truth("blur9x9"));
        println!("{},rep,native-rust,blur9x9,{rep},{},0,0", build_label(), ns(d));
    }
    // Compile time of each plugin on each engine.
    for (label, engine) in &engines {
        for k in &kernels {
            let bytes = wat(k.plugin);
            for rep in 0..COMPILE_REPS {
                let t = Instant::now();
                let module = block_on(engine.compile(&bytes, &report, opts)).expect("compile");
                let d = t.elapsed();
                drop(module);
                println!("{},compile,{label},{},{rep},{},0,0", build_label(), k.name, ns(d));
            }
        }
    }
    // Fresh instance per call on a small band, as the host does for `fresh` contributions.
    let band = noise_image(64, 64, 0, 7);
    let band_truth = posterize_reference(&band, LEVELS);
    for (label, engine) in &engines {
        let bytes = wat("kernel-posterize");
        let module = block_on(engine.compile(&bytes, &report, opts)).expect("compile");
        for rep in 0..FRESH_REPS {
            let t = Instant::now();
            let timing = cycle(module.as_ref(), &band, &LEVELS.to_le_bytes(), &Budget::new(16 * MIB, STACK));
            let d = t.elapsed();
            assert!(timing.out == band_truth, "{label} fresh band output differs");
            println!("{},fresh64,{label},posterize,{rep},{},{},{}", build_label(), ns(d), ns(timing.call), 0);
        }
    }
    note(&format!("loadavg_end={}", loadavg()));
}
