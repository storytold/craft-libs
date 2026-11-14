# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::`, in the qualified form `<muda::PredefinedMenuItem>::quit`, and through a `use muda::PredefinedMenuItem as ...` rename. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
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

## Test fonts

The `fonts` module makes test fonts from font bytes the caller supplies. It reads no file and no environment variable, prints nothing, and builds for `wasm32-unknown-unknown`.

- **`fonts::rename_family(bytes, from, to)`** returns a copy of one standalone face with the family name `from` replaced by `to`, which must have the same length in UTF-16 code units. It changes the family name records (name IDs 1, 16 and 21) whose value is `from`, and full names (ID 4) that are `from` or start with `from` and a space. At least one family record must hold `from`. It reads UTF-16BE and Macintosh Roman records, writes only ASCII names into Macintosh Roman records, leaves the PostScript name (ID 6), other names and other tables as they are, and writes the `name` table checksum and the `head` table's `checkSumAdjustment`.
- **`fonts::collection(faces)`** returns a TrueType collection (TTC 1.0) holding standalone faces. Each face gets its own copy of every table, tables start on four-byte boundaries, offsets count from the start of the collection, `checkSumAdjustment` is zero, and every table checksum matches the bytes in the collection.

Both return a `fonts::FontError` instead of panicking: names of different lengths, a name that no family record holds, a face shorter than its table directory, a table or name string out of range, a collection given as a face, or output past the 4 GiB a collection's offsets reach. The layouts come from the OpenType font file and `name` table chapters: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff> and <https://learn.microsoft.com/en-us/typography/opentype/spec/name>.

The tests that call the copies in VectorCraft, DesignCraft and WordCraft need each app's font database and its bundled fonts, so they stay in the apps. `tests/fonts.rs` tests the module on faces it assembles from bytes.

## Origin

| Module | Source | Path | Commit |
|---|---|---|---|
| `guards` | storytold/craft-libs#163 (zachshallbetter), with the rules of PhotoCraft's shader float test and FilmCraft's web clock test | `crates/testkit/src/guards.rs` | `e92825e427` |
| `fonts` | storytold/vectorcraft, `rename` and `collection` | `crates/text/src/tests/sysfonts.rs` | `bada8e1710` |
| `fonts` | storytold/vectorcraft, `renamed` | `crates/testkit/src/fonts.rs` | `bada8e1710` |
| `fonts` | storytold/designcraft, the family renaming and `collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` | `8b169f8ee7` |
| `fonts` | storytold/wordcraft, `renamed_to` and `collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` | `f8c66d0692` |
| `fonts` | storytold/wordcraft, `renamed` | `crates/fonts/src/fontdb/user/tests.rs` | `f8c66d0692` |

## Migration

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |
| VectorCraft | `fonts::rename_family`, `fonts::collection` | `crates/text/src/tests/sysfonts.rs` and `crates/testkit/src/fonts.rs` helpers | Not started |
| DesignCraft | `fonts::rename_family`, `fonts::collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` helpers | Not started |
| WordCraft | `fonts::rename_family`, `fonts::collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` and `crates/fonts/src/fontdb/user/tests.rs` helpers | Not started |
