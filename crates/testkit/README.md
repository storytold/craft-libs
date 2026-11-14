# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::` and through a `use muda::PredefinedMenuItem as ...` rename. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources and flags decimal and hexadecimal float literals whose value, rounded to `f64`, exceeds `f32::MAX` (such as `3.40282347e38` or `0x1p+500f`), or 65504, the largest finite `f16`, for a literal with an `h` suffix. A literal that exceeds the maximum by less than `f64` precision rounds to the maximum and is not reported. Integer literals are not checked. Some native compilers round such literals down, but browser WebGPU compilers reject them with pipeline compilation errors, forcing applications into CPU fallback rendering. `f32::MAX` can be written as the hex float `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
3. **`std_instant`**: Flags `std::time::Instant` outside comments and strings: the full path, `<m>::Instant` where a `use` binds `m` to `std::time`, and each `use` that imports `std::time::Instant` or `std::time::*`. Other crates' `Instant` (such as `web_time::Instant`) is not flagged, and neither is an item whose `cfg` attribute is false on `wasm32-unknown-unknown`. A `mod name;` inside such an item also skips the module's file (`name.rs` or `name/mod.rs`, or the file a `#[path]` attribute names) and the files of the modules declared in it, when they are among the files passed in and no declaration outside such items also reaches them. On that target, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`), build artifacts (`target/`, any folder holding a `CACHEDIR.TAG` file that starts with the line `Signature: 8a477f597d28d172789f06886806bc55`, `.git/` and other folders whose name starts with `.`) and symbolic links, which it does not follow. A file over 8 MiB, more than 256 MiB in all or more than 50,000 files returns an error; `rust_sources_with_limits` takes other limits. It is not built for `wasm32`.

## Epistemic Scope: Syntactic Guards vs Runtime Validation

Source-level guards provide deterministic syntactic evidence about codebase structure:

### What They Establish
- Checked source files do not contain forbidden symbol invocations, known out-of-range literal representations, or platform-hazardous imports.
- Reusable, deterministic checks that run in milliseconds as regular Cargo tests without external runners.

### What They Cannot Establish
- **Runtime execution safety**: Syntactic checks do not verify whether code branches actually execute at runtime.
- **Semantic type resolution**: The scanners follow `use` renames within one file only. They do not resolve paths through other modules or re-exports, a rename of `std` itself, or expand macros.
- **Complex target predicates**: `std_instant` evaluates `cfg` predicates (`all`, `any`, `not`, `unix`, `windows` and the `target_*` keys) for `wasm32-unknown-unknown`, on outer attributes for the item that follows and on inner attributes (`#![cfg(...)]`) for the rest of the module, and on `mod name;` declarations for the module's file and its descendants. It compares file paths as text, so the files must come from one `rust_sources` call or be named the same way. A predicate that depends on a feature or on `test` is not known from the source, so its item is checked. Applications should scope `std_instant` tests to the specific crates that compile into their web distribution.
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

## deadline

The `deadline` module is not built for `wasm32`.

`with_deadline(limit, label, f)` runs `f` on its own thread and returns its value. When `f` does not
finish within the limit, the test fails with a message that names `label`, the limit and the variable
that sets it. A panic in `f` is raised again in the test with its original payload. The limit counts
from the call, including the time spent starting the thread. When `f` returns or panics after the
limit has passed, the test fails with the label and the limit, and with the panic's message if it has
one.

`wait_until(limit, what, cond)` calls `cond` on the test's thread until it returns `true`. It calls
`cond` at once, then after pauses that start at 1 ms and double up to 50 ms, and does not call it
again once the limit has passed. When the limit passes
first, or `cond` returns `true` only after the limit has passed, the test fails with a message that
names `what`.

```rust
use std::time::Duration;
use craft_testkit::deadline::{DeadlineLimit, wait_until, with_deadline};

// The default of 30 s; CRAFT_TEST_TIMEOUT_SECS replaces it.
let pixels = with_deadline(Duration::from_secs(30), "decode seed 7", move || decode(&bytes));

// The app's own variable replaces the default instead.
let limit = DeadlineLimit::new(Duration::from_secs(30)).with_variable("FILMCRAFT_HOSTILE_TIMEOUT_SECS");
with_deadline(limit, "hostile input 12", move || parse(&input));

wait_until(Duration::from_secs(10), "the export to finish", || job.is_done());
```

### The limit variable

A `Duration` reads `CRAFT_TEST_TIMEOUT_SECS`. `DeadlineLimit::with_variable` names another variable.
The variable is read on every call; neither function sets or caches it.

| Value | Result |
|---|---|
| Not set | The default limit applies. |
| Decimal digits above zero, such as `30` or `007` | That many seconds replace the default. |
| Empty, signed (`+5`, `-5`), fractional (`1.5`), with spaces or units (`5s`), zero, larger than a `u64`, or not Unicode | The test fails with the variable's name and value, before `f` runs or `cond` is called. |

A zero default, a variable name that is empty or holds `=` or a NUL character, and a limit that ends past
the latest time the platform's clock can represent also fail the test before any work starts.

`craft-media-testkit`'s `with_deadline` and FilmCraft's `filmcraft-testkit` use the default when the
variable holds something other than a number. FilmCraft also uses the default for zero, and
`craft-media-testkit` accepts zero. Both fail the test here.

### Limits

- A body that runs past its limit fails the test, but its thread is left running until the test
  process exits. A body that never returns can keep the test binary alive after its tests finish.
- `wait_until` checks its limit only between calls of `cond`. A `cond` that blocks is not interrupted,
  so it must return promptly.
- Neither function returns elapsed time. Tests of this crate use channel handshakes, and the only time
  bounds in them are the limits under test.

## Origin

### guards

| Module | Source | Path | Commit |
|---|---|---|---|
| `guards` | storytold/craft-libs#163 (zachshallbetter), with the rules of PhotoCraft's shader float test and FilmCraft's web clock test | `crates/testkit/src/guards.rs` | `e92825e427` |

### deadline

| Code | Repository | Path | SHA |
|---|---|---|---|
| `with_deadline`, its panic and timeout tests (`a_slow_body_passes`, `a_panic_in_the_body_is_re_raised`, `a_hang_fails_and_names_the_input`) | storytold/filmcraft | `crates/testkit/src/deadline.rs` | `cff7e144` |
| `with_deadline` under `CRAFT_TEST_TIMEOUT_SECS`, tests `with_deadline_completes` and `with_deadline_times_out` | storytold/craft-libs | `crates/media-testkit/src/lib.rs` | `75a654f4` |
| `within`, a deadline that returns the body's value | storytold/lightcraft | `crates/engine/src/demo/tests.rs` | `487833ad` |
| `within`, a deadline that returns the body's value | storytold/gridcraft | `crates/math/src/lib.rs` | `c2aba7a3` |
| `wait_until` | storytold/photocraft | `crates/ui-egui/src/canvas/paint_thread_tests.rs` | `0d3b1e85` |
| `wait_until` | storytold/lightcraft | `crates/engine/src/availability.rs` | `7ebbb17a` |
| `wait_until` | storytold/effectcraft | `crates/engine/src/footage/footage_reload.rs` | `53576d22` |
| `wait_until` | storytold/filmcraft | `crates/ui-egui/tests/it/export_ui.rs` | `cff7e144` |

## Migration

### guards

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |

### deadline

| Consumer | Code it replaces | State |
|---|---|---|
| FilmCraft | `filmcraft-testkit`'s `with_deadline` (`FILMCRAFT_HOSTILE_TIMEOUT_SECS`) and the export UI tests' `wait_until` | Pending |
| LightCraft | `within` in the demo tests and `wait_until` in the availability tests | Pending |
| GridCraft | `within` in the tests of its `crates/math` | Pending |
| PhotoCraft | `wait_until` in the paint thread tests | Pending |
| EffectCraft | `wait_until` in the footage reload tests | Pending |
| `craft-media-testkit` | `with_deadline` (`CRAFT_TEST_TIMEOUT_SECS`) | Pending |

An adoption keeps the variable name the app uses today.
