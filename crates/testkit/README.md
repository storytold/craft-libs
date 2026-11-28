# craft-testkit

Test support shared by the Craft apps and the craft-libs crates. Apps and craft-libs crates take it
under `[dev-dependencies]`:

```toml
[dev-dependencies]
craft-testkit = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## `bin`

`bin` is native only: it is not built for `wasm32`.

`bin_path!("my-program")` expands in the calling integration test and returns the path of the
package's binary `my-program`. The name is the binary target's name with its hyphens. It reads, in
order:

1. `NEXTEST_BIN_EXE_my_program` (nextest 0.9.113 and later),
2. `NEXTEST_BIN_EXE_my-program`,
3. `CARGO_BIN_EXE_my-program` when the test runs (Cargo 1.94 and later, nextest 0.9.130 and later),
4. `CARGO_BIN_EXE_my-program` as Cargo set it when it compiled the test.

A value counts only when it is an absolute path to an existing regular file; an empty, relative or
missing path, or a folder, is passed over. A test run from an extracted nextest archive therefore
starts the extracted binary, and not the build machine's path that Cargo compiled into the test
([nextest: archiving](https://nexte.st/docs/ci-features/archiving/),
[nextest: environment variables](https://nexte.st/docs/configuration/env-vars/),
[Cargo: environment variables](https://doc.rust-lang.org/cargo/reference/environment-variables.html)).
When no variable names a file, the macro fails the test with a message that lists each variable,
its value and why it was passed over. `.exe` is never added. `resolve_binary` returns that message
as a `BinPathError` instead.

`run(path, args, env, max_output)` starts the program with stdin connected to nothing and returns
its `Output`. The child inherits the test's environment with the `env` pairs set on top; the test's
own environment is not changed. stdout and stderr are read at the same time, each into at most
`max_output` bytes. A stream that passes the limit stops the child, waits for it and returns an
`io::Error` holding `OutputLimitExceeded`. `DEFAULT_MAX_OUTPUT` is 1 MiB for each stream, a value of
this crate. A non-zero exit status is returned in the `Output`. `run` has no time limit.

```rust
use std::ffi::OsString;

#[test]
fn help_prints_usage() {
    let output = craft_testkit::bin::run(
        &craft_testkit::bin_path!("soundcraft-cli"),
        &[OsString::from("--help")],
        &[(OsString::from("SOUNDCRAFT_NO_PREFS"), OsString::from("1"))],
        craft_testkit::bin::DEFAULT_MAX_OUTPUT,
    )
    .unwrap();
    assert!(output.status.success());
}
```

Shared test support for the Craft apps. The crate is not published, and apps and craft-libs crates
take it under `[dev-dependencies]`.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::`, in the qualified form `<muda::PredefinedMenuItem>::quit`, and through a `use muda::PredefinedMenuItem as ...` rename. A type of the same name reached through another crate or module, such as `other::PredefinedMenuItem::quit`, is not flagged. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources and flags decimal and hexadecimal float literals whose value, rounded to `f64`, exceeds `f32::MAX` (such as `3.40282347e38` or `0x1p+500f`), or 65504, the largest finite `f16`, for a literal with an `h` suffix. A literal that exceeds the maximum by less than `f64` precision rounds to the maximum and is not reported. Integer literals are not checked. Some native compilers round such literals down, but browser WebGPU compilers reject them with pipeline compilation errors, forcing applications into CPU fallback rendering. `f32::MAX` can be written as the hex float `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
3. **`std_instant`**: Flags `std::time::Instant` outside comments and strings: the full path, `<m>::Instant` where a `use` binds `m` to `std::time`, and each `use` that imports `std::time::Instant` or `std::time::*`, under any name. The parts of a path may be split by whitespace, comments or line breaks. Other crates' `Instant` (such as `web_time::Instant` or `tokio::time::Instant`) is not flagged, and neither is an item whose `cfg` attribute is false on `wasm32-unknown-unknown`; code before that attribute or after its item on the same line is still checked. A `mod name;` inside such an item also skips the module's file (`name.rs` or `name/mod.rs`, or the file a `#[path]` attribute names) and the files of the modules declared in it, when they are among the files passed in and no declaration outside such items also reaches them. On that target, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), benchmarks (`benches/`), examples (`examples/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`, `test.rs`), build artifacts (`target/`, any folder holding a `CACHEDIR.TAG` file that starts with the line `Signature: 8a477f597d28d172789f06886806bc55`, `.git/` and other folders whose name starts with `.`) and symbolic links, which it does not follow. A file over 8 MiB, more than 256 MiB in all or more than 50,000 files returns an error; `rust_sources_with_limits` takes other limits. It is not built for `wasm32`.

## Random test input: `rng`

`craft_testkit::rng` holds two seeded generators for repeatable test input. They are not suitable for cryptography.

| Type | Recurrence | Output |
|---|---|---|
| `XorShift64` | `x ^= x << 13; x ^= x >> 7; x ^= x << 17` | the new state |
| `XorShift64Star` | `x ^= x >> 12; x ^= x << 25; x ^= x >> 27` | the new state multiplied by `0x2545_F491_4F6C_DD1D` |

Each type has `new(seed) -> Result<Self, SeedError>`, which returns `SeedError::ZeroSeed` for a seed of 0, and these methods:

- `next()` advances the state and returns one draw.
- `below(n)` returns one draw modulo `n`. A bound of 0 returns 0 and still takes a draw. A draw is never 0, so it is one of the 2^64 - 1 values from 1 to 2^64 - 1. When `n` divides 2^64 - 1, every result is equally likely: `below(3)` is unbiased over all nonzero states. For any other `n`, the results from 1 to (2^64 - 1) mod `n` each come from one more state than the other results, and the bias can be large: on a 64-bit target, `below(2^63 + 1)` returns 1 twice as often as 0.
- `byte()` returns the low 8 bits of one draw.
- `fill(out)` sets each byte of `out` to one `byte()`, in index order. An empty slice takes no draw.

A suite that switches to `rng` keeps the generator and seed it uses today, so its inputs do not change. The module builds for `wasm32-unknown-unknown`.

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
again once the limit has passed. When the limit passes first, or `cond` returns `true` only after the
limit has passed, the test fails with a message that names `what`.

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
| Empty, signed (`+5`, `-5`), fractional (`1.5`), with spaces or units (`5s`), zero, or larger than a `u64` | The test fails with the variable's name and value, before `f` runs or `cond` is called. |
| Not Unicode | The test fails with the variable's name, before `f` runs or `cond` is called. |

A zero default, a variable name that is empty or holds `=` or a NUL character, and a limit that ends past
the latest time the platform's clock can represent also fail the test before any work starts.

FilmCraft's `filmcraft-testkit` uses the default when the variable holds something other than a
number or holds zero. This module fails the test instead. `craft-media-testkit`'s `with_deadline`
calls this module's.

### Limits

- A body that runs past its limit fails the test, but its thread is left running until the test
  process exits. A body that never returns can keep the test binary alive after its tests finish.
- `wait_until` checks its limit only between calls of `cond`. A `cond` that blocks is not interrupted,
  so it must return promptly.
- Neither function returns elapsed time. Tests of this crate use channel handshakes, and the only time
  bounds in them are the limits under test.

## hostile

`mutations(seed, rounds)` yields `rounds` damaged copies of a valid seed. Each round makes one to four
edits: a bit flip, a random byte, a 1, 2, 4 or 8-byte overwrite with an extreme value
(0, 1, 0x7F, 0x80, 0xFF, 0xFFFF, 0x8000_0000 or `u64::MAX`) in either byte order, a deletion of up to
16 bytes, or a truncation. Each copy carries a label that lists its edits, such as
`flip[3].5 del[7..9]`. The xorshift64* generator behind it is seeded from the seed's length, so the
same seed length and round count give the same edits on every run and every platform. An empty seed
yields no copies. `mutations` and `Rng` build for `wasm32`.

`hammer(&config, seed, parse)` runs `parse` on the unmodified seed, on every truncation of it
(lengths 0 to one less than its length), and on `mutations(seed, rounds)`. The cases run one after
another on one worker thread, and the test's thread fails the test when a case runs past its limit.
A case that finishes after its limit also fails the test. The test fails on the first case that
panics or does not finish, with a message that names the config's label, the case
(`the unmodified seed`, `truncation to N` or the edits), the input's length and the panic's message.
`parse` must be `Fn(&[u8]) + Send + Sync + 'static`, since a case that does not finish keeps running
on the worker thread after the test fails. `hammer` is not built for `wasm32`.

```rust
use std::time::Duration;
use craft_testkit::DeadlineLimit;
use craft_testkit::hostile::{HammerConfig, hammer};

// 2,000 rounds times CRAFT_HOSTILE_MULT; 5 s per case, or CRAFT_TEST_TIMEOUT_SECS.
hammer(&HammerConfig::new("png seed 3", 2_000), &seed, |data| {
    let _ = decode(data);
});

// The app's own variables instead.
let config = HammerConfig::new("mxf primer", 5_000)
    .with_multiplier_variable("CRAFT_MEDIA_HOSTILE_MULT")
    .with_timeout(DeadlineLimit::new(Duration::from_secs(5)).with_variable("FILMCRAFT_HOSTILE_TIMEOUT_SECS"));
hammer(&config, &primer, |data| {
    let _ = parse_primer(data);
});
```

### The rounds

The rounds are the config's iterations times the multiplier variable, `CRAFT_HOSTILE_MULT` unless
`with_multiplier_variable` names another. The limit for each case is 5 s, replaced by
`CRAFT_TEST_TIMEOUT_SECS` or the variable that `with_timeout` names, as in `deadline`. Both variables
are read once, before any case runs. `HammerConfig::rounds` returns the rounds or the error that `hammer` would fail with.

| Multiplier value | Result |
|---|---|
| Not set | The multiplier is 1. |
| Decimal digits above zero, such as `10` | The rounds are multiplied by that number. |
| Empty, signed, fractional, with spaces, zero, or larger than a `usize` | The test fails with the variable's name and value before `parse` is called. |
| Not Unicode | The test fails with the variable's name before `parse` is called. |

A product of the iterations and the multiplier that does not fit in a `usize`, a call that would run
more than `MAX_CASES` (10,000,000) cases in all, and a limit variable that `deadline` would reject
also fail the test before `parse` is called.

### Quick runs

`CRAFT_HOSTILE_DIVISOR=N` runs a fraction of every sweep: the unmodified seed, every N-th truncation
(lengths 0, N, 2N, ...) and the rounds divided by N, rounded up. It takes the same values as the
multiplier. The Workspace gate sets it on every run and the nightly run leaves it unset, so every
case still runs once a day. A sweep written without `hammer` reads it with `hostile::divisor()` and
runs the cases whose index plus an offset is a multiple of it, with a different offset for each input
so that the inputs together still reach every index. A failing case is found again with the same
value.

Changes from `craft-media-testkit`'s `hammer`:

- A case that does not finish fails the test after its limit with the case's label. Before, a case
  was timed after it returned and failed only when it took longer than 5 s.
- A panic's message is part of the failure.
- Every truncation runs. Before, a seed of 8,192 bytes or more ran every `len / 4096`th truncation.
- The caller names the multiplier variable. Before, it was always `CRAFT_MEDIA_HOSTILE_MULT`, and a
  value that was not a number counted as 1.
- An empty seed runs once. Before, it ran once more for each round.
- A mutation's label joins its edits with single spaces. Before, every edit ended with a space, so
  each label ended with one.
- `parse` is owned and runs on a worker thread. A closure that borrows from the test needs its data
  moved or shared with an `Arc`.

## oracle

`craft_testkit::oracle` finds the external programs that tests compare against, such as ffmpeg, ffprobe, qpdf, `pdftoppm` and `pdftotext`. It is not built for `wasm32`.

`find(program, version_arg)` takes a bare file name: a name that is empty, `.` or `..`, or contains `/`, `\`, `:` or NUL returns `OracleError::InvalidName`. It looks in this order:

1. The file that `CRAFT_<PROGRAM>` names, for example `CRAFT_FFMPEG`. `find_with` takes the variable name from the caller, such as `FILMCRAFT_FFMPEG` or `CRAFT_MEDIA_FFMPEG`. An empty value counts as unset. When the value is not a file the OS can start, the lookup returns `OracleError::InvalidExplicitPath` and does not search further.
2. Each directory on `PATH`, in order. Empty entries are skipped, and a relative entry is resolved against the current directory at the time of the lookup. On Windows the search tries `<name>.exe`, then each `.com`, `.bat` or `.cmd` suffix that `PATHEXT` lists. Elsewhere it tries the bare name, then `<name>.exe`. A file the OS cannot start is skipped: on Windows a file whose extension is not `.com`, `.exe`, `.bat` or `.cmd` (in any case), and on Unix a file the current process does not have permission to execute.
3. The install folders `known_dirs()` returns: `/opt/homebrew/bin`, `/usr/local/bin`, `/usr/bin`, `/opt/local/bin` and `/snap/bin`, or on Windows `C:\ffmpeg\bin`, `C:\Program Files\ffmpeg\bin` and `C:\ProgramData\chocolatey\bin`.

The program found is run once with `version_arg` and standard input closed. `Oracle::version()` is the first non-blank line of its standard output, or of its standard error when standard output is blank. `ProbeLimits` bounds the run: each output stream may hold at most `max_output_bytes`, and the child is killed and reaped when it passes `timeout` or the byte limit. On Unix the child runs in its own process group, and the whole group is killed. On Windows the child and its descendants run in a Job Object and are stopped together through `process-wrap`. `ProbeLimits::VERSION` (the default) allows 64 KiB per stream and 5 seconds, and `ProbeLimits::LISTING` allows 1 MiB per stream and 5 seconds. These values are this crate's own defaults. A timeout too long to set a deadline, such as `Duration::MAX`, returns `OracleError::InvalidTimeout` without starting the program. A successful lookup is cached for the process under every input it read; a failed lookup is not cached, so the next call searches again.

On Windows, drive-relative paths such as `C:tools\qpdf.exe` use the captured current directory on the same drive, or that drive's current directory otherwise. Returned executable paths are absolute. The resolved executable path is part of the cache key.

After one version probe succeeds, concurrent and later lookups with the same cache key reuse its
result. A failed probe can be retried by a waiting or later lookup.
If a Unix permission query is unavailable, the lookup attempts to start the candidate. An execution
permission denial continues the search, or rejects an explicit path. Other failures to start return
`OracleError::Start`. Unix process-group
cleanup sends a signal directly. An exit observed after the probe deadline returns a timeout.

`find_in` takes the explicit path, the `PATH` list and the install folders as arguments, so a test can search folders it made without changing the process environment.

`ffmpeg_supports_format(ff, name, limits)` runs `ffmpeg -hide_banner -formats` and looks for `name` in the name column only. A run that fails to start, passes `limits` or exits with a failure status returns an error. `ffmpeg_version(ff)` reads the version from ffmpeg's banner line.

No function panics. `require_oracle!`, `require_ffmpeg!` and `require_ffprobe!` expand in the calling test:

```rust
let qpdf = craft_testkit::require_oracle!(craft_testkit::oracle::find("qpdf", "--version"), "PDFCRAFT_ORACLES_REQUIRED");
let ff = craft_testkit::require_ffmpeg!("FILMCRAFT_FFMPEG", "FILMCRAFT_REQUIRE_ORACLES");
```

A missing program goes to `skip::skip` with the required variable the caller names (`skip::REQUIRED_VAR`, `CRAFT_TEST_REQUIRED`, by default). The test prints a `SKIPPED:` line and returns, or fails, under the rule in the `skip` section below. `oracle::outcome` sorts a lookup result into `Outcome::Missing` and `Outcome::Fail` without reading the required variable's value. A required variable name that is empty or holds `=` or a NUL character fails the test even when the program is found. Every other error, such as a wrong explicit path, a timeout or output over the limit, fails the test.

## skip

The `skip` module is not built for `wasm32`. A test that needs something a test environment may lack
(a program, a corpus file, the craft-fonts faces) skips or fails under one rule:

The variable is read only when something is missing.

| Value | Result |
|---|---|
| Not set or empty | The helper prints `SKIPPED: <reason>` and one line feed to stderr, and the test returns and passes. Carriage returns and line feeds in the reason are printed as `\r` and `\n`. |
| `1` | The test fails. No skip line is printed. |
| Any other value, such as `0`, `true`, `yes`, ` 1` or `1` followed by a line feed | The test fails with the reason and the variable's name and value. No skip line is printed. |
| Not Unicode | The test fails with the reason and the variable's name. No skip line is printed. |

A variable name that is empty or holds `=` or a NUL character also fails the test, before the
environment is read.

`skip::required(var)` returns the same decision without failing the test: `Ok(false)`, `Ok(true)`,
or a `RequiredError` that names the variable. Every module of this crate that reads a required
variable reads it with `skip::required`, so one rule applies to all of them.

The caller passes the name its repository uses today, such as `CRAFT_MEDIA_REQUIRE_ORACLES`,
`CRAFT_PDF_ORACLES_REQUIRED`, `PDFCRAFT_ORACLES_REQUIRED` or `FILMCRAFT_REQUIRE_ORACLES`.
`skip::REQUIRED_VAR` (`CRAFT_TEST_REQUIRED`) is the default name.

The example assumes `find_qpdf()` reports only an absent program. Use `require_oracle!` with
`oracle::find` so other lookup errors fail the test. Optional skips ignore stderr write failures.

```rust
use craft_testkit::skip::{REQUIRED_VAR, craft_fonts, skip};

#[test]
fn renders_with_qpdf() {
    let qpdf = craft_testkit::or_skip!(find_qpdf(), REQUIRED_VAR);
    // ...
}

#[test]
fn draws_japanese() {
    if !craft_fonts(HAS_CRAFT_FONTS, CRAFT_FONTS_REQUIRED) {
        return;
    }
    // ...
}
```

`or_skip!` evaluates a `Result` once, binds the `Ok` value, and on `Err` skips with the error's
`Display` text as the reason. The two-argument form returns `()`; a test that returns a `Result`
passes the value to return as the third argument, for example `Ok(())`.

`craft_fonts(present, required)` takes both values from the caller's build: a build constant, an
`option_env!` value, or the table `craft-fonts-build` generates. A required failure names
`CRAFT_FONTS_REQUIRED=1`.

`craft_fonts_cover(text, faces)` needs the `font-coverage` feature, which adds skrifa. It returns
`true` when one face maps every character of `text` that is not whitespace. Each face is font data
with its index in a collection (`0` for a single font). A face that does not parse covers nothing,
and coverage is not combined across faces.

The module does not change the process environment. Its tests that read a real variable run the
test binary again as a child process with that variable set.

## serial

`serial` is native only.

- `TestLock::new()` is `const`, so a lock can be a `static`:
  `static L: TestLock = TestLock::new(); let _g = L.hold();`.
- The thread that holds a lock can call `hold()` again. The lock is released when the last of that
  thread's guards is dropped, in any order.
- A test that panics while it holds a lock does not block the next one, and a poisoned internal
  mutex is recovered.
- `gpu_lock()` is the one lock of the process for tests that build a GPU harness or device.
  `hold_gpu_lock()` takes it and keeps it until the calling thread ends; a second call on the same
  thread does nothing more.
- A guard is not `Send`. The lock covers one process: under nextest each test runs in its own
  process.
- The module reads no environment variables and prints nothing.

The tests use channel handshakes and contain no sleeps or time limits.

## Origin

### serial

| Module | Source repository | Path | SHA |
|---|---|---|---|
| `serial` | storytold/photocraft | `crates/testkit/src/gpu.rs` | `0d3b1e8523` |
| `serial` | storytold/deckcraft | `crates/ui-egui/src/gpu_lock.rs` | `b0c77ebd6a` |
| `serial` | storytold/effectcraft | `crates/ui-egui/tests/it/gpu_test.rs`, `crates/gpu/src/tests.rs` | `f029bbadb2` |
| `serial` | storytold/pdfcraft | `apps/pdfcraft/src/tests.rs` | `346d803080` |

`a_panicked_holder_does_not_block_the_next_test` and `holders_take_turns` keep their names from
PhotoCraft and DeckCraft. `holders_take_turns` waits on channels where the copies slept 2 ms.

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

### hostile

| Code | Repository | Path | SHA |
|---|---|---|---|
| `Rng`, the edits of `mutations` and `hammer` | storytold/craft-libs | `crates/media-testkit/src/hostile.rs` | `75a654f4` |

### oracle

| Code | Repository | Path | SHA |
|---|---|---|---|
| Search order, `find_in`, format and version parsing, `require_ffmpeg!`, `require_ffprobe!` | storytold/filmcraft | `crates/testkit/src/oracle.rs`, `crates/testkit/src/lib.rs` | `e4ab3c6a` |
| `Oracle`, the version probe, install hints, `or_skip!` | storytold/pdfcraft | `crates/oracle/src/lib.rs` | `eead4541` |
| The same probe, with `CRAFT_PDF_ORACLES_REQUIRED` | storytold/craft-libs | `crates/pdf/tests/cos/oracle.rs` | `75a654f4` |
| Search order and install folders | storytold/craft-libs | `crates/media-testkit/src/lib.rs` | `75a654f4` |

### skip

| Code | Repository | Path | Commit |
|---|---|---|---|
| `skip!`, `record` | storytold/lightcraft | `crates/testkit/src/lib.rs` | `7ebbb17a82` |
| `or_skip!`, `REQUIRED_VAR` | storytold/pdfcraft | `crates/oracle/src/lib.rs` | `ee9bf28126` |
| `require_*!`, `skip` | storytold/filmcraft | `crates/testkit/src/lib.rs`, `crates/testkit/src/oracle.rs` | `cff7e1442e` |
| `skip`, `skip_with` | storytold/craft-libs | `crates/media-testkit/src/lib.rs` | `75a654f4f8` |
| `craft_fonts_cover` | storytold/gridcraft | `crates/ui-egui/src/theme.rs` | `06c2b1440b` |

### rng

| Module | Source | Path | Commit |
|---|---|---|---|
| `rng` (`XorShift64`) | storytold/filmcraft | `crates/dnx/tests/it/fuzz_hostile.rs:9-20` | `0a881266` |
| `rng` (`XorShift64`) | storytold/craft-libs | `crates/video/tests/dnx/fuzz_hostile.rs:9-20` | `9f10d434` |
| `rng` (`XorShift64Star`) | storytold/craft-libs | `crates/jpeg/tests/mutation.rs:28-46` | `97c83dc9` |

### bin

| Module | Source | Path | Commit |
|---|---|---|---|
| `bin` (`bin_path!`, both nextest spellings) | storytold/soundcraft | `apps/soundcraft-cli/tests/common.rs` | `2688ccac88` |
| `bin` (`bin_path!`, runtime `CARGO_BIN_EXE_`) | storytold/vectorcraft | `apps/vectorcraft-cli/tests/it/main.rs` | `bada8e1710` |
| `bin` (lookup tests `cli_binary_prefers_relocated_runner_path`, `cli_binary_defaults_to_cargo_path`) | storytold/texturecraft | `apps/texturecraft-cli/tests/cli.rs` | `386630ee85` |
| `bin` (test `cli_executable_is_resolved`) | storytold/filmcraft | `apps/filmcraft-cli/tests/it/main.rs` | `e4ab3c6a99` |
| `bin` (`run`: null stdin, environment on top of the inherited one) | storytold/soundcraft | `apps/soundcraft-cli/tests/it/behaviour.rs` | `2688ccac88` |

## Migration

### guards

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |

### rng

| Consumer | Code it replaces | State |
|---|---|---|
| craft-libs `video` | the generator in `tests/dnx/fuzz_hostile.rs` and the other `fuzz_hostile.rs` suites (`rng::XorShift64`) | Not started |
| craft-libs `jpeg` | the generator in `tests/mutation.rs` (`rng::XorShift64Star`) | Not started |
| FilmCraft | the generator in `crates/dnx/tests/it/fuzz_hostile.rs` (`rng::XorShift64`) | Not started |

### deadline

| Consumer | Code it replaces | State |
|---|---|---|
| FilmCraft | `filmcraft-testkit`'s `with_deadline` (`FILMCRAFT_HOSTILE_TIMEOUT_SECS`) and the export UI tests' `wait_until` | Pending |
| LightCraft | `within` in the demo tests and `wait_until` in the availability tests | Pending |
| GridCraft | `within` in the tests of its `crates/math` | Pending |
| PhotoCraft | `wait_until` in the paint thread tests | Pending |
| EffectCraft | `wait_until` in the footage reload tests | Pending |
| `craft-media-testkit` | `with_deadline` (`CRAFT_TEST_TIMEOUT_SECS`) | Done |

An adoption keeps the variable name the app uses today.

### hostile

| Consumer | Code it replaces | State |
|---|---|---|
| `craft-media-testkit` | `hostile::hammer` (`CRAFT_MEDIA_HOSTILE_MULT`), called by the ISO BMFF, AVI, Matroska, MXF and MPEG TS hostile tests | Done |
| `craft-cfb` | its `hostile_input_never_panics` test, through `craft-media-testkit` | Done |
| CADCraft | the mutation loop of `hostile_files_never_panic` in `crates/fonts/src/shx/tests.rs` | Pending |
| TextureCraft | the `hammer` of `malformed_meshes_are_errors` in `crates/formats/src/mesh_import/tests.rs` | Pending |

CADCraft's and TextureCraft's loops make other edits than `mutations`. Their adoptions say which
cases change, and keep the test names.

### bin

| Consumer | Replaces | Source commit | State |
|---|---|---|---|
| SoundCraft | `apps/soundcraft-cli/tests/common.rs` lookup, `tests/it/behaviour.rs` runner | `2688ccac88` | Planned |
| VectorCraft | `apps/vectorcraft-cli/tests/it/main.rs` lookup | `bada8e1710` | Planned |
| CADCraft | `apps/cadcraft-cli/tests/it/cli.rs` runner | `8e69890223` | Planned |
| DeckCraft | `apps/deckcraft-cli/tests/it/main.rs` lookup, `cli.rs` runner | `b0c77ebd6a` | Planned |
| FilmCraft | `apps/filmcraft-cli/tests/it/main.rs` lookup, `cli.rs` runner | `e4ab3c6a99` | Planned |
| GridCraft | `apps/gridcraft-cli/tests/it/cli.rs` runner | `f2044337c7` | Planned |
| PdfCraft | `apps/pdfcraft-cli/tests/it/help.rs` runner | `eead454105` | Planned |
| TextureCraft | `apps/texturecraft-cli/tests/cli.rs` lookup | `386630ee85` | Planned |
| WordCraft | `apps/wordcraft-cli/tests/it/help.rs` runner | `f8c66d0692` | Planned |

### oracle

| Consumer | Code it replaces | State |
|---|---|---|
| `craft-media-testkit` | `tool`, `ffmpeg`, `ffprobe`, `oracles_required` | Pending |
| `craft-pdf` | `tests/cos/oracle.rs` | Pending |
| FilmCraft | `filmcraft-testkit`'s `oracle` | Pending |
| PdfCraft | `pdfcraft-oracle` | Pending |
| EffectCraft | `crates/opusenc/tests/it/ffmpeg.rs` and the tests that call ffmpeg or ffprobe | Pending |
| SoundCraft | `crates/video/src/tests.rs` | Pending |

Each adoption keeps the variable names the repository uses today. Its required variable then follows the `skip` rule, and the adoption says which values change, as the `skip` table below describes.

### skip

| Consumer | Code it replaces | State |
|---|---|---|
| LightCraft | `lightcraft-testkit`'s `skip!`, which keeps its JSON skip log on top of `skip` | Not started |
| PdfCraft | `pdfcraft-oracle`'s `or_skip!`, which reads any value of `PDFCRAFT_ORACLES_REQUIRED` | Not started |
| FilmCraft | `filmcraft-testkit`'s `require_*!` and `skip`, which accept `1`, `true`, `yes` and `on` | Not started |
| GridCraft | `craft_fonts_cover`, which reads GridCraft's embedded face table | Not started |
| craft-libs `craft-media-testkit` | `skip` and `skip_with`, which accept `1`, `true`, `yes` and `on` | Not started |
| craft-libs `pdf` tests | `or_skip!` in `crates/pdf/tests/cos/oracle.rs`, which reads any value of `CRAFT_PDF_ORACLES_REQUIRED` | Not started |

After an adoption, an app that reads any value as required skips on an empty value, and fails as
a configuration error on every other value except `1`, `0` included. An app that accepts `true`, `yes`
and `on` fails on those values as a configuration error.
Each adoption says which values change and keeps the variable name.

### serial

| Consumer | Module | Switched |
|---|---|---|
| PhotoCraft | `serial` | No |
| DeckCraft | `serial` | No |
| EffectCraft | `serial` | No |
| PdfCraft | `serial` | No |
