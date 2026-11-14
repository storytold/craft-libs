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

## Origin

| Module | Source | Path | Commit |
|---|---|---|---|
| `guards` | storytold/craft-libs#163 (zachshallbetter), with the rules of PhotoCraft's shader float test and FilmCraft's web clock test | `crates/testkit/src/guards.rs` | `e92825e427` |

## Migration

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |

## `image` feature

`craft-testkit = { ..., features = ["image"] }` adds `craft_testkit::image` and `assert_similar!`. It depends on `png` 0.18.

| Item | What it does |
|---|---|
| `Rgba8`, `Gray8` | Borrowed 8-bit views. `new` returns an error when a side is 0 or the buffer is not exactly `width × height × channels` bytes. |
| `diff_rgba`, `diff_gray` | `DiffStats`: differing pixels, maximum and mean absolute sample difference, nearest-rank p99 of each pixel's largest difference, and PSNR with a peak of 255. RGBA counts alpha as a fourth sample. |
| `psnr(a, b, peak)` | PSNR of two `u8` or `u16` planes. The caller passes the peak (255, 1023, 4095, 65535). Equal planes give `f64::INFINITY`; empty or unequal planes and a peak that is not finite and positive are errors. |
| `Tolerance`, `compare_rgba`, `compare_gray` | Gates for maximum difference, p99, minimum PSNR and the share of bad pixels, each inclusive. With `neighborhood`, a pixel is bad only when nothing in the other image's 3×3 window is within `channel`, checked in both directions. |
| `diff_image` | An opaque RGBA image of each absolute sample difference times 8, with alpha differences shown in every color channel. |
| `decode_png`, `encode_png`, `encode_gray_png`, `read_png`, `write_png`, `write_gray_png` | 8-bit PNG reading and writing under `PngLimits` (default 16,384 × 16,384 pixels, 64 MiB encoded, 256 MiB decoded). Palette, low bit depths, 16-bit samples, `tRNS` and interlacing are converted to 8-bit gray or RGBA. Gamma, ICC and sRGB chunks are not applied. |
| `check_golden`, `assert_similar!` | Compare an image with a reference PNG. The reference is written only when the variable named in `GoldenOptions::bless_var` (default `CRAFT_BLESS`) is exactly `1`; otherwise a missing reference is an error. A failed check writes `<name>.actual.png` and `<name>.diff.png` into `GoldenOptions::failures_dir`, using `<name>-1`, `<name>-2` and so on rather than replacing a file. `assert_similar!` fails the calling test. |

The views, statistics, PSNR, tolerances, the difference image and the in-memory PNG functions build for `wasm32`. The file functions, the bless variable and the golden check are native only.

```rust
use craft_testkit::image::{GoldenOptions, Rgba8, Tolerance};

let options = GoldenOptions { bless_var: "FILMCRAFT_BLESS", ..GoldenOptions::new(&failures, "title_card") };
let tolerance = Tolerance { min_psnr: Some(45.0), max_abs: Some(12), max_p99: Some(2), ..Tolerance::EXACT };
craft_testkit::assert_similar!(&reference, Rgba8::new(width, height, &pixels)?, tolerance, options);
```

### Origin

| Code | Source |
|---|---|
| Statistics, PNG reading and writing, golden check | FilmCraft `crates/testkit/src/golden.rs` at `e4ab3c6a99` |
| Per-pixel channel tolerance and bad-pixel share | AniCraft `crates/engine/tests/golden/mod.rs` at `3435e98681` |
| Gray view and 3×3 neighborhood comparison | PdfCraft `crates/oracle/src/pixels.rs` at `eead454105` |

### Migration

| Consumer | Today | State |
|---|---|---|
| FilmCraft | `golden.rs`, `FILMCRAFT_BLESS` | Not started |
| AniCraft | `crates/engine/tests/golden/mod.rs`, `UPDATE_GOLDENS` | Not started |
| PdfCraft | `pdfcraft-oracle` `pixels.rs` | Not started |
| VectorCraft | `crates/testkit/src/raster.rs` (comparison only) | Not started |
| DesignCraft | `crates/render/tests/it/effects.rs`, `DESIGNCRAFT_BLESS` | Not started |
