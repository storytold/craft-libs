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

## External oracle programs: `oracle`

`craft_testkit::oracle` finds the external programs that tests compare against, such as ffmpeg, ffprobe, qpdf, `pdftoppm` and `pdftotext`. It is not built for `wasm32`.

`find(program, version_arg)` takes a bare file name: a name that is empty, `.` or `..`, or contains `/`, `\`, `:` or NUL returns `OracleError::InvalidName`. It looks in this order:

1. The file that `CRAFT_<PROGRAM>` names, for example `CRAFT_FFMPEG`. `find_with` takes the variable name from the caller, such as `FILMCRAFT_FFMPEG` or `CRAFT_MEDIA_FFMPEG`. An empty value counts as unset. When the value is not a file, the lookup returns `OracleError::InvalidExplicitPath` and does not search further.
2. Each directory on `PATH`, in order. Empty entries are skipped, and a relative entry is resolved against the current directory at the time of the lookup. On Windows the search tries `<name>.exe`, then each `.com`, `.bat` or `.cmd` suffix that `PATHEXT` lists, then the bare name. Elsewhere it tries the bare name, then `<name>.exe`.
3. The install folders `known_dirs()` returns: `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/opt/local/bin` and `/snap/bin`, or on Windows `C:\ffmpeg\bin`, `C:\Program Files\ffmpeg\bin` and `C:\ProgramData\chocolatey\bin`.

The program found is run once with `version_arg` and standard input closed. `Oracle::version()` is the first non-blank line of its standard output, or of its standard error when standard output is blank. `ProbeLimits` bounds the run: each output stream may hold at most `max_output_bytes`, and the child is killed and reaped when it passes `timeout` or the byte limit. On Unix the child runs in its own process group, and the whole group is killed. On Windows only the child is killed. `ProbeLimits::VERSION` (the default) allows 64 KiB per stream and 5 seconds, and `ProbeLimits::LISTING` allows 1 MiB per stream and 5 seconds. These values are this crate's own defaults. A timeout too long to set a deadline, such as `Duration::MAX`, returns `OracleError::InvalidTimeout` without starting the program. A successful lookup is cached for the process under every input it read; a failed lookup is not cached, so the next call searches again.

`find_in` takes the explicit path, the `PATH` list and the install folders as arguments, so a test can search folders it made without changing the process environment.

`ffmpeg_supports_format(ff, name, limits)` runs `ffmpeg -hide_banner -formats` and looks for `name` in the name column only. A run that fails to start, passes `limits` or exits with a failure status returns an error. `ffmpeg_version(ff)` reads the version from ffmpeg's banner line.

No function panics. `require_oracle!`, `require_ffmpeg!` and `require_ffprobe!` expand in the calling test:

```rust
let qpdf = craft_testkit::require_oracle!(craft_testkit::oracle::find("qpdf", "--version"), "PDFCRAFT_ORACLES_REQUIRED");
let ff = craft_testkit::require_ffmpeg!("FILMCRAFT_FFMPEG", "FILMCRAFT_REQUIRE_ORACLES");
```

A missing program prints a `SKIPPED (oracle)` line and returns from the test. When the required variable (`CRAFT_REQUIRE_ORACLES` by default) is exactly `1`, a missing program fails the test instead. Other values, such as `true`, `yes` or `0`, do not count. Every other error, such as a wrong explicit path, a timeout or output over the limit, fails the test.

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
| `oracle` (search order, `find_in`, format and version parsing, `require_ffmpeg!`, `require_ffprobe!`) | storytold/filmcraft | `crates/testkit/src/oracle.rs`, `crates/testkit/src/lib.rs` | `e4ab3c6a` |
| `oracle` (`Oracle`, version probe, install hints, `or_skip!`) | storytold/pdfcraft | `crates/oracle/src/lib.rs` | `eead4541` |
| `oracle` (the same probe, with `CRAFT_PDF_ORACLES_REQUIRED`) | storytold/craft-libs | `crates/pdf/tests/cos/oracle.rs` | `75a654f4` |
| `oracle` (search order and install folders) | storytold/craft-libs | `crates/media-testkit/src/lib.rs` | `75a654f4` |

## Migration

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |
| craft-libs `media-testkit` | `oracle` | `tool`, `ffmpeg`, `ffprobe`, `oracles_required` | Not started |
| craft-libs `pdf` | `oracle` | `tests/cos/oracle.rs` | Not started |
| FilmCraft | `oracle` | `filmcraft-testkit` `oracle` | Not started |
| PdfCraft | `oracle` | `pdfcraft-oracle` | Not started |
| EffectCraft | `oracle` | `crates/opusenc/tests/it/ffmpeg.rs` and the tests that call ffmpeg or ffprobe | Not started |
| SoundCraft | `oracle` | `crates/video/src/tests.rs` | Not started |

Each `oracle` switch keeps the variable names the repository uses today. A repository whose required variable accepts other values than `1` today changes that reading in its own pull request.
