# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources using PhotoCraft's decimal float scanner and flags literals whose absolute value exceeds `f32::MAX` (such as `3.40282347e38`). While native compilers (`naga`) round these down, browser WebGPU compilers (`Tint` in Chromium) reject them with fatal pipeline compilation errors, forcing applications into CPU fallback rendering. Exact values equal to `f32::MAX` must be written as exact hex float literals (`0x1.fffffep+127f`).
3. **`std_instant`**: Flags lines naming `std::time::Instant` outside comments without `web_time`. On `wasm32-unknown-unknown`, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`), and build artifacts (`target/`, `.git/`).

## Log capture: `log`

The `log` feature adds `craft_testkit::log` and the `log` dependency. The module is built on every target except `wasm32`.

| Function | Captures | Returns |
|---|---|---|
| `warnings(f)` | warnings and errors logged on the calling thread while `f` runs | `Result<(T, Vec<String>), CaptureError>` with the messages |
| `records(level, f)` | records at `level` or more severe logged on the calling thread while `f` runs | `Result<(T, Vec<CapturedRecord>), CaptureError>` with level, target and message |
| `capture_global(level)` | records at `level` or more severe logged on any thread | `Result<GlobalCapture, CaptureError>`; `drain()` takes the records captured since the last drain |

`log::set_logger` can be called once per process, so the first capture to install the logger sets the mode for the whole test binary: per thread (`warnings`, `records`) or process wide (`capture_global`). The functions return these errors without running the closure or changing the maximum level:

| Error | When |
|---|---|
| `LoggerAlreadySet` | another logger was installed first |
| `ModeConflict` | the binary already captures in the other mode |
| `NestedCapture` | `warnings` or `records` is called inside a capture on the same thread |
| `GlobalCaptureActive` | another `GlobalCapture` is alive; dropping it ends that capture, and the logger stays installed |

A capture raises `log::max_level()` to the level it asks for and never lowers it. When the closure panics, the capture on that thread ends and the panic continues. Records from other threads do not appear in a per-thread capture.

A capture keeps at most `MAX_RECORDS` (4,096) records and `MAX_BYTES` (1 MiB) of message and target text. Formatting a message stops once it passes `MAX_BYTES`. A record over either limit is dropped and counted, and the capture returns `CaptureError::Overflow { dropped }` instead of a partial list. For `drain`, the limits apply to each interval between drains.

Run process-wide captures in a binary of their own, for example a `[[test]]` with `harness = false`:

```rust
fn main() {
    let capture = craft_testkit::log::capture_global(log::LevelFilter::Warn).expect("capture");
    std::thread::Builder::new()
        .name("audio".into())
        .spawn(|| drop_the_plugin())
        .expect("spawn")
        .join()
        .expect("join");
    let records = capture.drain().expect("drain");
    assert!(records.iter().any(|r| r.message.contains("off the main thread")));
}
```

### Origin

| Module | Source repository | Path | Revision |
|---|---|---|---|
| `log` (per thread) | storytold/craft-libs | `crates/docx/src/test_log.rs` (the same file as `crates/wordmodel/src/test_log.rs` and WordCraft's `crates/doc/src/test_log.rs` and `crates/formats/src/test_log.rs`) | `e3e31693` |
| `log` (process wide) | storytold/soundcraft | `crates/clap-host/tests/editor_main_thread.rs:18-41` | `e6cbf761` |

### Migration

| Consumer | Uses | State |
|---|---|---|
| craft-libs `docx`, `wordmodel` | their private `test_log.rs` | Pending: a separate pull request switches both |
| WordCraft `doc`, `formats` | their private `test_log.rs` | Pending: a separate pull request in WordCraft |
| SoundCraft `clap-host` | the logger in `tests/editor_main_thread.rs` | Pending: a separate pull request in SoundCraft switches it to `capture_global` |

## Epistemic Scope: Syntactic Guards vs Runtime Validation

Source-level guards provide deterministic syntactic evidence about codebase structure:

### What They Establish
- Checked source files do not contain forbidden symbol invocations, known out-of-range literal representations, or platform-hazardous imports.
- Reusable, deterministic checks that run in milliseconds as regular Cargo tests without external runners.

### What They Cannot Establish
- **Runtime execution safety**: Syntactic checks do not verify whether code branches actually execute at runtime.
- **Semantic type resolution**: The scanners do not resolve arbitrary type aliases (e.g. `use std::time as t; t::Instant::now()`) or expand procedural macros.
- **Complex target predicates**: While `std_instant` recognizes direct item-level non-wasm target conditions (e.g. `#[cfg(not(target_arch = "wasm32"))]`), it does not evaluate arbitrary multi-crate boolean expressions. Applications should scope `std_instant` tests to the specific crates that compile into their web distribution.
- **Driver and host OS behavior**: Syntactic checks do not verify WebGPU driver compatibility or host window manager event dispatch.

Source guards serve as fast, zero-dependency regression gates in application test suites, complementing platform integration tests and CI workflows.

## Consumer Adoption Examples

### 1. PhotoCraft: Adopt `wgsl_float_literals` and `predefined_quit`

In `photocraft/crates/gpu/src/lib.rs`, replace the in-tree `decimal_float_literals` helper with `craft_testkit::wgsl_float_literals`:

```rust
#[test]
fn shader_float_literals_fit_f32() {
    let bad = craft_testkit::wgsl_float_literals(SHADER);
    assert!(
        bad.is_empty(),
        "float literals outside f32 finite range reject on WebGPU (Tint):\n{}",
        bad.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```

In `photocraft/apps/photocraft/tests/native_menu_tests.rs`:

```rust
#[test]
fn no_unintercepted_predefined_quit_in_menu() {
    let sources = craft_testkit::rust_sources("src").expect("scan sources");
    let findings = craft_testkit::predefined_quit(&sources);
    assert!(
        findings.is_empty(),
        "predefined muda Quit bypasses dirty document save prompts:\n{}",
        findings.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```

### 2. FilmCraft: Adopt `std_instant` and `wgsl_float_literals`

In `filmcraft/crates/engine/src/wasm_clock_tests.rs`, replace in-tree textual search with `craft_testkit::std_instant`:

```rust
#[test]
fn no_std_instant_in_code_the_web_build_runs() {
    let sources = craft_testkit::rust_sources(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
        .expect("scan engine sources");
    let findings = craft_testkit::std_instant(&sources);
    assert!(
        findings.is_empty(),
        "std::time::Instant panics on wasm32; use web_time::Instant:\n{}",
        findings.iter().map(|f| f.to_string()).collect::<Vec<_>>().join("\n")
    );
}
```
