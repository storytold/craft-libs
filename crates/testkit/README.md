# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::` and through a `use muda::PredefinedMenuItem as ...` rename. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources and flags decimal and hexadecimal float literals whose value, rounded to `f64`, exceeds `f32::MAX` (such as `3.40282347e38` or `0x1p+500f`), or 65504, the largest finite `f16`, for a literal with an `h` suffix. A literal that exceeds the maximum by less than `f64` precision rounds to the maximum and is not reported. Integer literals are not checked. Some native compilers round such literals down, but browser WebGPU compilers reject them with pipeline compilation errors, forcing applications into CPU fallback rendering. `f32::MAX` can be written as the hex float `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
3. **`std_instant`**: Flags `std::time::Instant` outside comments and strings: the full path, `<m>::Instant` where a `use` binds `m` to `std::time`, and each `use` that imports `std::time::Instant` or `std::time::*`. Other crates' `Instant` (such as `web_time::Instant`) is not flagged, and neither is an item whose `cfg` attribute is false on `wasm32-unknown-unknown`. A `mod name;` inside such an item also skips the module's file (`name.rs` or `name/mod.rs`, or the file a `#[path]` attribute names) and the files of the modules declared in it, when they are among the files passed in and no declaration outside such items also reaches them. On that target, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`), build artifacts (`target/`, any folder holding a `CACHEDIR.TAG` file that starts with the line `Signature: 8a477f597d28d172789f06886806bc55`, `.git/` and other folders whose name starts with `.`) and symbolic links, which it does not follow. A file over 8 MiB, more than 256 MiB in all or more than 50,000 files returns an error; `rust_sources_with_limits` takes other limits. It is not built for `wasm32`.

## Pinned corpora: `corpus`

`craft_testkit::corpus` is behind the `corpus` feature and is native only. It parses a corpus manifest and checks a folder of files against it.

A manifest has one row per file, with tab-separated columns: the SHA-256 as 64 hexadecimal digits in either case, the path relative to the corpus folder with `/` between its parts, and optional license and URL columns. Blank lines and lines whose first character other than a space or a tab is `#` are skipped. LF and CRLF line endings are both read.

- `Manifest::parse(text)` returns a `ManifestError` naming the line for a row with fewer than 2 or more than 4 columns, a digest that is not 64 hexadecimal digits, a path that is empty, starts with `/`, has an empty, `.` or `..` part, or holds a `\`, a `:` or a NUL character, and a path listed twice. An empty optional column is `None`.
- `Manifest::read(path, &limits)` reads a manifest file, stopping one byte past the size limit. A path that is not a regular file, such as a folder or a named pipe, returns `ManifestError::NotAFile` without being opened.
- `verify(dir, &manifest)` checks that each listed file resolves inside `dir` after symbolic links are followed, is a regular file within the size limit, and has the listed SHA-256. It returns a `VerifyError` naming the path. A manifest with no rows is an error.
- `pinned(var, manifest)` is the gate a test calls. `var` names the folder variable, and `None` reads `CRAFT_CORPUS`. The corpus is required when the variable of the same name plus `_REQUIRED` (by default `CRAFT_CORPUS_REQUIRED`) is exactly `1`. With the folder variable unset and the corpus optional, it prints `SKIPPED: corpus not given: set <var> to its folder` to stderr and returns `None`. With the folder variable unset and the corpus required, or set to an empty value, a manifest that does not parse or a folder that does not match, it fails the test. Otherwise it returns the folder.

`Limits` sets the size limits: by default a manifest of at most 1 MiB and 10,000 rows, and files of at most 8 GiB each, read 64 KiB at a time. No function downloads a file or changes the process environment.

### Origin

| Module | Source repository | Path | Revision |
|---|---|---|---|
| `corpus` | storytold/craft-libs | `crates/affinity/tests/support/corpus.rs:18-35` | `75a654f4` |

### Migration

| Consumer | Today | State |
|---|---|---|
| craft-libs `affinity` | `tests/support/corpus.rs` reads `corpus.tsv` (already tab-separated) from `AFFINITY_CORPUS` | Pending: a separate pull request switches it |
| craft-libs `raw` | `docs/raf-corpus.sha256` separates digest and path with two spaces | Pending: the list is converted to tabs in a separate pull request |
| LightCraft | `crates/raw/tests/it/corpus.rs` checks the RAF list | Pending: a separate pull request switches it |
| PhotoCraft | `crates/io/tests/it/corpus.rs` and `xtask/*-corpus.sha256` (two-space lists) | Pending: the lists are converted to tabs in a separate pull request |
| CADCraft | `crates/io/src/roundtrip/tests.rs` scans `CADCRAFT_CORPUS_DIR` without a checksum list | Pending: needs a pinned list first |
| VectorCraft | `crates/affinity/tests/support/corpus.rs` reads a two-space list | Pending: the list is converted to tabs in a separate pull request |

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
