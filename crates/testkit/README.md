# craft-testkit

Source-level platform regression guards and verification utilities for the Craft application suite.

Shared test support for the Craft apps. The crate is not published, and apps and craft-libs crates
take it under `[dev-dependencies]`.

## Overview

`craft-testkit` provides lightweight, non-panicking source guards that prevent applications in the Craft ecosystem from repeating platform defects that were previously diagnosed and resolved in peer applications:

1. **`predefined_quit`**: Flags direct calls to `muda`'s `PredefinedMenuItem::quit` outside comments, strings and character literals, also with whitespace or a line break around `::`, in the qualified form `<muda::PredefinedMenuItem>::quit`, and through a `use muda::PredefinedMenuItem as ...` rename. A type of the same name reached through another crate or module, such as `other::PredefinedMenuItem::quit`, is not flagged. On macOS, this predefined menu item routes directly to `NSApplication terminate:`, completely bypassing application dirty document checks and unsaved changes save prompts. Applications should instead wire Quit through `craft-quit` lifecycle interceptors.
2. **`wgsl_float_literals` / `wgsl_float_literals_in`**: Scans WGSL shader sources and flags decimal and hexadecimal float literals whose value, rounded to `f64`, exceeds `f32::MAX` (such as `3.40282347e38` or `0x1p+500f`), or 65504, the largest finite `f16`, for a literal with an `h` suffix. A literal that exceeds the maximum by less than `f64` precision rounds to the maximum and is not reported. Integer literals are not checked. Some native compilers round such literals down, but browser WebGPU compilers reject them with pipeline compilation errors, forcing applications into CPU fallback rendering. `f32::MAX` can be written as the hex float `0x1.fffffep+127f`, which equals it, or as the decimal `3.4028234663852886e38`, which rounds to it.
3. **`std_instant`**: Flags `std::time::Instant` outside comments and strings: the full path, `<m>::Instant` where a `use` binds `m` to `std::time`, and each `use` that imports `std::time::Instant` or `std::time::*`, under any name. The parts of a path may be split by whitespace, comments or line breaks. Other crates' `Instant` (such as `web_time::Instant` or `tokio::time::Instant`) is not flagged, and neither is an item whose `cfg` attribute is false on `wasm32-unknown-unknown`; code before that attribute or after its item on the same line is still checked. A `mod name;` inside such an item also skips the module's file (`name.rs` or `name/mod.rs`, or the file a `#[path]` attribute names) and the files of the modules declared in it, when they are among the files passed in and no declaration outside such items also reaches them. On that target, `std::time::Instant::now()` panics at runtime with `"time not implemented on this platform"`. Code targeting the web must use `web_time::Instant`.
4. **`rust_sources`**: Recursively discovers a crate's `.rs` source files while excluding integration tests (`tests/`), test modules (`tests.rs`, `tests_*.rs`, `*_tests.rs`, `*_test.rs`), build artifacts (`target/`, any folder holding a `CACHEDIR.TAG` file that starts with the line `Signature: 8a477f597d28d172789f06886806bc55`, `.git/` and other folders whose name starts with `.`) and symbolic links, which it does not follow. A file over 8 MiB, more than 256 MiB in all or more than 50,000 files returns an error; `rust_sources_with_limits` takes other limits. It is not built for `wasm32`.

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

## Test fonts

The `fonts` module makes test fonts from font bytes the caller supplies. It reads no file and no environment variable, prints nothing, and builds for `wasm32-unknown-unknown`.

- **`fonts::rename_family(bytes, from, to)`** returns a copy of one standalone face with the family name `from` replaced by `to`, which must have the same length in UTF-16 code units. It changes the family name records (name IDs 1, 16 and 21) whose value is `from`, and full names (ID 4) that are `from` or start with `from` and a space. At least one family record must hold `from`. It reads UTF-16BE and Macintosh Roman records, writes only ASCII names into Macintosh Roman records, leaves the PostScript name (ID 6), other names and other tables as they are, and writes the `name` table checksum and the `head` table's `checkSumAdjustment`.
- **`fonts::collection(faces)`** returns a TrueType collection (TTC 1.0) holding standalone faces. Each face gets its own copy of every table, tables start on four-byte boundaries, offsets count from the start of the collection, `checkSumAdjustment` is zero, and every table checksum matches the bytes in the collection.

Both return a `fonts::FontError` instead of panicking: names of different lengths, a name that no family record holds, a face shorter than its table directory, a table or name string out of range, a collection given as a face, or output past the 4 GiB a collection's offsets reach. The layouts come from the OpenType font file and `name` table chapters: <https://learn.microsoft.com/en-us/typography/opentype/spec/otff> and <https://learn.microsoft.com/en-us/typography/opentype/spec/name>.

The tests that call the copies in VectorCraft, DesignCraft and WordCraft need each app's font database and its bundled fonts, so they stay in the apps. `tests/it/fonts.rs` tests the module on faces it assembles from bytes.

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

## corpus

`craft_testkit::corpus` is behind the `corpus` feature and is native only. It parses a corpus manifest and checks a folder of files against it.

A manifest has one row per file, with tab-separated columns: the SHA-256 as 64 hexadecimal digits in either case, the path relative to the corpus folder with `/` between its parts, and optional license and URL columns. Blank lines and lines whose first character other than a space or a tab is `#` are skipped. LF and CRLF line endings are both read.

- `Manifest::parse(text)` returns a `ManifestError` naming the line for a row with fewer than 2 or more than 4 columns, a digest that is not 64 hexadecimal digits, a path that is empty, starts with `/`, has an empty, `.` or `..` part, or holds a `\`, a `:` or a control character (NUL included), and a path listed twice. An empty optional column is `None`. The parser does not check that a name is valid on a given file system: a name Windows reserves (`<`, `>`, `"`, `|`, `?`, `*`, a trailing `.` or space, a device name such as `NUL`) or two names that differ only in case are accepted, so a manifest must use names that every file system it is verified on supports.
- `Manifest::read(path, &limits)` reads a manifest file, stopping one byte past the size limit. A path that is not a regular file, such as a folder or a named pipe, returns `ManifestError::NotAFile` without being opened.
- `verify(dir, &manifest)` checks that each listed file resolves inside `dir` after symbolic links are followed, is a regular file within the size limit, and has the listed SHA-256. It returns a `VerifyError` naming the path. A manifest with no rows is an error.
- On Unix, `Manifest::read` and `verify` open each file with `O_NONBLOCK` and check the opened file's type again, so a path replaced by a named pipe after the first check fails at once. After the open, `verify` also checks that the opened file is inside `dir`: on Linux, Android and Apple platforms by the path the kernel reports for the open file, and on other Unix systems, or on Linux and Android when `/proc/self/fd` cannot be read, by resolving the listed path again and comparing its device and inode with the opened file's. On Apple platforms an error from the path query fails verification with `VerifyError::Read` instead of falling back to the second lookup. That second lookup goes through the path again, so a folder changed during it is not detected. On Windows the location of the opened file is not checked again, so a part of the path changed into a link out of `dir` between the check and the open is not detected. In every case the opened file must still have the listed SHA-256.
- `pinned(var, manifest)` is the gate a test calls. `var` names the folder variable, and `None` reads `CRAFT_CORPUS`. With the folder variable unset, it calls `skip::skip` with the reason `corpus not given: set <var> to its folder` and the required variable `<var>_REQUIRED` (by default `CRAFT_CORPUS_REQUIRED`), so the [skip](#skip) rule applies: unset or empty prints `SKIPPED: corpus not given: set <var> to its folder` to stderr and returns `None`, `1` fails the test, and any other value, `0` included, fails the test as a configuration error. With the folder variable set to an empty value, a manifest that does not parse or a folder that does not match, it fails the test. A `var` that is empty or holds `=` or a NUL character fails the test before the environment is read. Otherwise it returns the folder.

`Limits` sets the size limits: by default a manifest of at most 1 MiB and 10,000 rows, and files of at most 8 GiB each, read 64 KiB at a time. No function downloads a file or changes the process environment.

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

## prop

The `prop` feature adds the `prop` module and the `proptest` dependency. The module is not built for
`wasm32`.

`prop::config(cases)` returns a `proptest::test_runner::Config` with failure persistence off. It
reads proptest's own variables:

| Variable | Not set | Set |
|---|---|---|
| `PROPTEST_CASES` | `cases` | The value as a `u32`. Any other value makes `config` panic. |
| `PROPTEST_RNG_SEED` | `prop::SEED` (`0x5eed_9a7f`) | The value as a `u64`. Any other value makes `config` panic. |

A suite under `prop::config` repeats the same cases on every run. To run it with other cases, set
`PROPTEST_RNG_SEED` to another number. No file under `proptest-regressions/` is read
or written, so a minimal failure found with other seeds or counts is kept as a named regression test
in the suite.

```rust
use proptest::prelude::*;

proptest! {
    #![proptest_config(craft_testkit::prop::config(64))]

    #[test]
    fn parse_never_panics(bytes in proptest::collection::vec(any::<u8>(), 0..256)) {
        let _ = my_crate::parse(&bytes);
    }
}
```

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

### fonts

| Module | Source | Path | Commit |
|---|---|---|---|
| `fonts` | storytold/vectorcraft, `rename` and `collection` | `crates/text/src/tests/sysfonts.rs` | `bada8e1710` |
| `fonts` | storytold/vectorcraft, `renamed` | `crates/testkit/src/fonts.rs` | `bada8e1710` |
| `fonts` | storytold/designcraft, the family renaming and `collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` | `8b169f8ee7` |
| `fonts` | storytold/wordcraft, `renamed_to` and `collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` | `f8c66d0692` |
| `fonts` | storytold/wordcraft, `renamed` | `crates/fonts/src/fontdb/user/tests.rs` | `f8c66d0692` |

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

### corpus

| Code | Repository | Path | SHA |
|---|---|---|---|
| The manifest rows and the digest check of `pinned` | storytold/craft-libs | `crates/affinity/tests/support/corpus.rs:18-35` | `75a654f4` |

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

### prop

| Code | Repository | Path | SHA |
|---|---|---|---|
| `config` and `SEED` | storytold/vectorcraft | `crates/pathops/tests/common/mod.rs:3-20` | `34627134` |

## Migration

### guards

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| PhotoCraft | `wgsl_float_literals` | `crates/gpu/src/tests.rs` scanner | Not started |
| FilmCraft | `std_instant` | `crates/engine/src/wasm_clock_tests.rs` search | Not started |

### fonts

| Consumer | Guard | Replaces | State |
|---|---|---|---|
| VectorCraft | `fonts::rename_family`, `fonts::collection` | `crates/text/src/tests/sysfonts.rs` and `crates/testkit/src/fonts.rs` helpers | Not started |
| DesignCraft | `fonts::rename_family`, `fonts::collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` helpers | Not started |
| WordCraft | `fonts::rename_family`, `fonts::collection` | `crates/fonts/src/fontdb/tests/sysfonts.rs` and `crates/fonts/src/fontdb/user/tests.rs` helpers | Not started |

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

### corpus

| Consumer | Code it replaces | State |
|---|---|---|
| craft-libs `affinity` | `tests/support/corpus.rs`, which parses `corpus.tsv` (four tab-separated columns) and checks the folder `AFFINITY_CORPUS` names | Pending |
| craft-libs `raw` | `tests/it/raf_corpus_manifest.rs`, which checks the rows of the tab-separated `docs/raf-corpus.sha256` but not the files | Pending |
| LightCraft | `crates/raw/tests/it/corpus.rs` checks the RAF list | Pending |
| PhotoCraft | `crates/io/tests/it/corpus.rs` and `xtask/*-corpus.sha256` (two-space lists, to be converted to tabs) | Pending |
| CADCraft | `crates/io/src/roundtrip/tests.rs` scans `CADCRAFT_CORPUS_DIR` without a checksum list | Pending: needs a pinned list first |
| VectorCraft | `crates/affinity/tests/support/corpus.rs` reads a two-space list, to be converted to tabs | Pending |

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

### prop

| Consumer | Code it replaces | State |
|---|---|---|
| VectorCraft `pathops` | the `Config` of `tests/props.rs` and `tests/prop_pathops.rs` | Pending |
| craft-libs | the 34 `Config` values that set `cases` directly, and the seven committed `proptest-regressions` files outside `vendor/` | Pending |
| LightCraft, FilmCraft | the property suites that set their case count | Pending |

Each adoption names every suite whose seed changes from random to pinned, and turns committed
regression cases into named tests.
