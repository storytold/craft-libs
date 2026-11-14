# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `names` | always | `Policy`, `Input`, `scan`, `scan_source`, `literals`, `test_path`: the display-name scanner, which looks for the words an app lists in its string literals, `Cargo.toml` descriptions and text files |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |

No feature is on by default. `report` adds `serde_json` and `craft-atomic`.

## Use

```rust,no_run
use std::path::Path;
use craft_xtask::runner::{self, RunOptions, Step, TestMode};

fn ci() -> Result<(), String> {
    let root = runner::root(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    let mut steps = vec![
        Step::new("fmt", |log| {
            let mut c = runner::cargo(&root);
            c.args(["fmt", "--all", "--", "--check"]);
            runner::run(c, log.command("cargo fmt --all -- --check"))
        }),
        Step::new("test", |log| {
            let (c, label) = runner::nextest_or_test(&root, "xtask", TestMode::CargoTest, &["--lib", "--bins", "--tests"])?;
            runner::run(c, log.command(&label))
        }),
    ];
    runner::run_steps(&mut steps, &RunOptions::default())?;
    Ok(())
}
```

- `root` takes the app's own `CARGO_MANIFEST_DIR` and returns its parent, which must hold a
  `Cargo.toml`. It returns an error instead of panicking.
- `nextest_or_test` and `doctest` build `cargo test --workspace --exclude <xtask package>` (or
  `cargo nextest run` with the same arguments). The exclusion keeps the test build from rebuilding the
  running `xtask` executable, which Windows does not allow while it runs
  ([storytold/craft-common-actions#185](https://github.com/storytold/craft-common-actions/issues/185)).
  `TestMode::CargoTest` is the default and probes nothing. `TestMode::PreferNextest` runs nextest when
  `cargo nextest --version` succeeds and otherwise runs `cargo test` with a note; the label names the
  command that runs.
- Child processes inherit stdin, stdout and stderr. Their output is not decoded or stored.
- With `RunOptions::with_cancel`, a set `Cancel` flag stops the run before the next step, and
  `run_cancelable` ends a running child with the processes it started and waits for it. On Unix the
  child runs in its own process group, which `kill -s KILL` ends, so a terminal's Ctrl-C reaches only
  the xtask and the app's Ctrl-C handler sets the flag. On Windows `taskkill /T /F` ends the tree.

## Errors

`runner::Error` is `Config` (a root without `Cargo.toml`, an invalid package name, a slot count
outside 1 to 64), `Step` (the step and the error it returned), `Canceled`, `Lock` (the lock directory
or file could not be opened or locked) or `Report` (the run passed but the report could not be
written). A report that cannot be written after a failed run is printed to stderr, and the run keeps
its `Step` error.

## Names

The app owns its word list, the paths it scans, the literals and exceptions that may keep a word,
the marker, the command or test that runs the scan, and the message it prints. The library reads the
files and returns the findings:

```rust,no_run
use std::path::Path;
use craft_xtask::names::{self, Input, Policy};

const BANNED: &[&str] = &["Amber Tool", "AzureKit"];
const ALLOWED: &[&str] = &["AzureKit Design"];
const POLICY: Policy<'static> = Policy { allowed: ALLOWED, ..Policy::new(BANNED) };

#[test]
fn production_display_names_stay_neutral() {
    let base = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inputs = [Input::rust(Path::new("src")), Input::rust(Path::new("../engine/src/cmd"))];
    let found = names::scan(base, &inputs, &POLICY).unwrap();
    let list: Vec<String> = found.iter().map(ToString::to_string).collect();
    assert!(list.is_empty(), "Banned display names:\n{}", list.join("\n"));
}
```

- `Input::rust` takes a Rust file or a folder, which is read recursively for `.rs` files that
  `Policy::skip` keeps, given their path below the folder (`test_path` by default: `tests.rs`,
  `*_tests.rs` and files under a `tests` folder). Links to folders are not followed. A file input
  is read whatever its name.
- `Input::manifest` reads the values at the key paths `package.description` and
  `workspace.package.description` of a `Cargo.toml`: the table header (`[package]`,
  `[workspace.package]`) joined with the key, which may be bare, quoted or dotted. Headers and keys
  inside strings and arrays do not count, and a key inside an inline table is not read. A
  `description` under any other table, or before the first header, is not read. A TOML string,
  single-line or multi-line, is decoded, and a comment after it is not read; any other value is
  read as the rest of its line. `Input::text` reads every line of a text file, and
  `Input::text_mark_above` also lets a `#` line with the marker exempt the line below it. Inputs
  must not include binary files.
- In Rust, the scanner reads `"…"` and `r"…"` literals with any number of `#`, decodes every escape
  (`\n`, `\x7A`, `\u{…}`, a backslash before a line break) and reads CRLF as LF. `b"…"`, `br"…"`,
  `c"…"` and `cr"…"` are read when `Policy::literals` asks for them. Comments, character, byte and
  lifetime tokens, and the rest of a module after `#![cfg(test)]` are skipped. So is the code under
  `#[cfg(test)]`, comments in the attribute included: a `const`, `static` or `let` up to its `;`,
  another item up to the `}` that closes it or the `;` that ends it, and a field, argument, arm or
  statement up to its `,` or `;`. A block expression (a block, a macro's braces, a labeled block, or
  `if`, `match`, `loop`, `while`, `for`, `unsafe`, `async` or `const`), as the whole statement or as
  an arm's expression after `=>`, ends at its `}`, or at the last `}` of an `if` with `else`
  branches. When a method call, field or `?` follows that `}`, it runs up to its `,` or `;`
  instead. Other `cfg` expressions and macros are not evaluated. A source the lexer cannot read
  (an unterminated literal or comment, an unknown escape, unbalanced delimiters) is an error.
- A word matches as a whole word: the characters next to it are not letters, digits or `_`, in any
  script. `MatchCase::Exact` compares characters exactly, and `MatchCase::AsciiInsensitive` ignores
  the case of ASCII letters. `allowed` and `exceptions` exempt a whole decoded literal or
  description or trimmed text line, never part of one.
- In Rust, the marker counts only inside a comment on a line the literal spans. In a manifest it
  counts anywhere on a line the description spans, and in a text file anywhere on the line.
- `scan` reads each file once, in path order, and returns the findings in path order, then source
  order, then the order of `banned`. A `Finding` has the path, the first line of the literal or
  description, the word, the decoded text or text line and the kind; its `Display` text is
  `path:line: "word" in "text"`. `scan_source` scans one source already in memory, and
  `literals` returns a source's literals.
- `names::Error` is `Policy` (no banned words, an empty word or an empty marker), `Io` (with the
  path), `InvalidUtf8`, `FileTooLarge`, `TotalTooLarge`, `TooDeep`, `Conflict` (a file given as two
  kinds of input) or `Syntax` (with the path and line). `DEFAULT_LIMITS` is 8 MiB per file, 512 MiB
  per scan and folders 64 deep; a file over the limit is rejected before it is read.

The source copies differ in a few rules. An app that switches lists the results that change:

- The eight `display_names.rs` tests compare exactly (`MatchCase::Exact`), read byte and C strings
  like other strings, skip only `#[cfg(test)] mod` (GridCraft skips any `#[cfg(test)]` item up to
  its `}` or `;`), pass `\x` and `\u{…}` through undecoded, can take the quote in `'\"'` as the
  start of a string (LightCraft, FigCraft, PhotoCraft), follow folder links, skip a file root named
  like a test file, and stop with a panic on a file they cannot read (FigCraft returns the error).
  Each finds more or less here accordingly.
- VectorCraft's `brands` check compares ASCII letters without case (`MatchCase::AsciiInsensitive`)
  and reads byte and C strings (`Literals`). It also takes a non-ASCII letter or `_` next to a word
  as a word boundary, takes its marker anywhere on a line, literals included, reads an escape as a
  space, reads manifest lines that start with `description` as they are (comments included, one
  line of a multi-line value), and skips a file it cannot read.
- PhotoCraft's check of its locale catalogs stays in the app.

## Lock

`RunOptions::with_lock(LockConfig::new(dir, slots, owner_label))` takes one of `slots` lock files in
`dir` before the first step and releases it when `run_steps` returns. The OS releases a lock when its
process ends. A slot count of 0 or more than 64, or a poll interval (`with_poll`, 2 s by default) of zero, is an
error before any file is opened. Next to each
held lock file, an owner file holds the owner label, which waiting runs print; `holders` reads at
most 4096 bytes of each and leaves out an owner file whose slot is free, such as one a killed run left
behind. Where the lock files live, the environment variables that set them, and
whether a run uses a lock at all are the app's choice.

## Report

`RunOptions::with_report(path)` writes the report after the run, passed or not, and before the run
releases its lock slot. A run with a lock that does not take it (a configuration or lock error, or a
cancel while waiting) runs no step and writes no report. The report looks like this:

```json
{
  "schema": 1,
  "steps": [
    { "step": "fmt", "command": "cargo fmt --all -- --check", "status": "ok", "duration_ms": 812 },
    { "step": "test", "command": null, "status": "fail", "duration_ms": 4031 },
    { "step": "wasm", "command": null, "status": "skip", "duration_ms": null }
  ]
}
```

`status` is `ok`, `fail`, `canceled` or `skip`. The file is staged beside `path` and renamed over it,
so a reader never sees a partial report, and a failed write leaves the previous report in place.
Machine, GPU, corpus and skip details stay in the app's own report.

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| `names`: the literal lexer, whole-word matching, allowed literals, the tree walk and the tests `production_display_names_stay_neutral`, `guard_distinguishes_labels_from_identifiers_and_comments`, `guard_scans_production_after_test_only_items`, `guard_skips_escaped_character_literals`, `guard_decodes_whitespace_escapes` | CADCraft `crates/ui-egui/tests/display_names.rs` at [8e69890](https://github.com/storytold/cadcraft/blob/8e6989022398ff5f79adaab77d19fdd7137591f3/crates/ui-egui/tests/display_names.rs) and DeckCraft at [b0c77eb](https://github.com/storytold/deckcraft/blob/b0c77ebd6a6fe6c9037cfbe1c8a7f397304b2205/crates/ui-egui/tests/display_names.rs) |
| `names`: the test `guard_scans_after_escaped_character_literals` | AniCraft `crates/engine/tests/display_names.rs` at [3435e98](https://github.com/storytold/anicraft/blob/3435e9868150312e54694c0a70082609b2fe8573/crates/engine/tests/display_names.rs) |
| `names`: read errors returned, and the test `source_read_failures_are_reported` | FigCraft `crates/mcp/tests/display_names.rs` at [bad07e4](https://github.com/storytold/figcraft/blob/bad07e4433240cad046589129c084bd2fc36f097/crates/mcp/tests/display_names.rs) |
| `names`: the tests `guard_scans_production_after_test_only_items_and_parses_raw_labels`, `guard_skips_escaped_character_literals_without_hiding_labels`, and `exceptions` for a group-label prefix | WordCraft `crates/ui-egui/tests/display_names.rs` at [323cf78](https://github.com/storytold/wordcraft/blob/323cf78232ff3c07f9dbb7989ed729d5851c9773/crates/ui-egui/tests/display_names.rs) |
| `names`: `exceptions` for a message prefix | LightCraft `crates/ui-egui/tests/display_names.rs` at [6b2b7e6](https://github.com/storytold/lightcraft/blob/6b2b7e6d1117e683a1701568e56f979d061e4699/crates/ui-egui/tests/display_names.rs) |
| `names`: a single file as a root | GridCraft `crates/ui-egui/tests/display_names.rs` at [f204433](https://github.com/storytold/gridcraft/blob/f2044337c7e195bdf6bec1b3914937a3dd386749/crates/ui-egui/tests/display_names.rs) |
| `names`: allowed literals | PhotoCraft `crates/ui-egui/tests/display_names.rs` at [d6557b9](https://github.com/storytold/photocraft/blob/d6557b9a8b40f0542a99e7382ec2b5250548e27b/crates/ui-egui/tests/display_names.rs) |
| `names`: `MatchCase::AsciiInsensitive`, manifest descriptions, text files, the marker and the line above it, the literal's first line, and the tests `rust_literals_skip_comments_identifiers_tests_and_marked_lines`, `whole_words_only`, `docs_packaging_and_manifests`, `scope`, `injected_strings_fail_the_gate`, `the_tree_is_clean` | VectorCraft `xtask/src/brands.rs` at [107078a](https://github.com/storytold/vectorcraft/blob/107078ad10870f8a8fdeade9bd5627a3eea34a67/xtask/src/brands.rs) |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report; in `names`, the full escape decoder, byte and C strings as an option, every `#[cfg(test)]` item form and `#![cfg(test)]`, the syntax errors, the marker only in comments, word boundaries in every script, the limits, the sorted findings and the skipped folder links | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| AniCraft, CADCraft, DeckCraft, FigCraft, GridCraft, LightCraft, PhotoCraft, WordCraft | Planned: the lexer and matcher of `display_names.rs` move to `names`, with `MatchCase::Exact`. `BANNED`, `ALLOWED`, `ROOTS`, the prefix exceptions (LightCraft, WordCraft), PhotoCraft's catalog test and the tests named after them stay in the app. |
| VectorCraft | Planned: `cargo xtask brands` calls `names::scan` with `MatchCase::AsciiInsensitive`, byte and C strings, `brand-ok` and its desktop entry as `Input::text_mark_above`. The word list, the file selection from `git ls-files` and the message stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
