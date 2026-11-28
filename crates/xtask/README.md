# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `assets` | always | `Policy`, `RowMode`, `check`, `run`, `list_files`, `read_document`, `is_asset`, `ASSET_EXT`: the asset attribution check against the document, folders and extensions the app supplies |
| `names` | always | `Policy`, `Input`, `scan`, `scan_source`, `literals`, `test_path`: the display-name scanner, which looks for the words an app lists in its string literals, `Cargo.toml` descriptions and text files |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |
| `pins` | `pins` | `run`, `check`, `run_target_features`, `host_target`: the version, source and features of pinned dependencies, and packages the app forbids, read from `cargo metadata` within size limits |
| `stats` | `stats` | `run`, `collect` and `write_table`: files, lines and tests of Rust per workspace crate (`cargo xtask stats`) |
| `version` | `version` | `run`, `run_with`, `Limits`, `lock_dirs`, `read`, `replace`, `validate`: `cargo xtask version [set X.Y.Z[-pre]]` on the root `Cargo.toml` |

No feature is on by default. `report` adds `serde_json` and `craft-atomic`, and `pins` and `stats`
each add `serde_json`.
`version` adds `craft-atomic` and `toml_edit` and turns on `lock`.

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
- On Unix a child started by `run_cancelable` is not in the terminal's foreground process group. A
  child that reads from the terminal is stopped by `SIGTTIN`, and the run waits until the `Cancel` flag
  is set. Run a command that reads from the terminal with `run`.

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
  `*_tests.rs` and files under a `tests` folder). Links to folders are not followed, also when the
  link is an input. A file input is read whatever its name.
- `Input::manifest` reads the values at the key paths `package.description` and
  `workspace.package.description` of a `Cargo.toml`: the table header (`[package]`,
  `[workspace.package]`) joined with the key, which may be bare, quoted or dotted. Headers and keys
  inside strings and arrays do not count, and a key inside an inline table is not read. Before the
  first header, the dotted keys `package.description` and `workspace.package.description` are read,
  and a bare `description` is not. A `description` under any other table is not read. A TOML string,
  single-line or multi-line, is decoded, and a comment after it is not read; any other value is
  read as the rest of its line. `Input::text` reads every line of a text file, and
  `Input::text_mark_above` also lets a `#` line with the marker exempt the line below it. Inputs
  must not include binary files.
- In Rust, the scanner reads `"…"` and `r"…"` literals with any number of `#`, decodes every escape
  (`\n`, `\x7A`, `\u{…}`, a backslash before a line break) and reads CRLF as LF. `b"…"`, `br"…"`,
  `c"…"` and `cr"…"` are read when `Policy::literals` asks for them; their `\x` escapes are decoded
  together as UTF-8, and bytes that are not UTF-8 become U+FFFD. A shebang line, comments,
  character, byte and lifetime tokens, and the rest of a module after `#![cfg(test)]` are skipped.
  So is the code under `#[cfg(test)]`, comments in the attribute included: a `const`, `static` or
  `let` up to its `;`, another item up to the `}` that closes it or the `;` that ends it, and a
  field, argument, arm or statement up to its `,` or `;`. A `,` in an expression's generic
  arguments (`f::<A, B>`), in the type of a field, a parameter (after its name or pattern) or a
  cast (`mut p: Pair<A, B>`, `ptr as *const Pair<A, B>`), or in the parameters of a closure that
  starts the expression does not end it. An item keyword counts only when nothing
  but qualifiers (`pub`, `unsafe`, `extern "C"` and the like) comes before it, so the `fn` in
  `f::<fn()>(…)` or in a field's type does not start an item, and the braces of a macro in an
  item's header (`-> ty! { u8 }`) are not its body. A block expression (a block, a macro's braces, a labeled block, or
  `if`, `match`, `loop`, `while`, `for`, `unsafe`, `async` or `const`), as the whole statement or as
  an arm's expression after `=>`, ends at its `}`, or at the last `}` of an `if` with `else`
  branches. Braces in the condition of an `if`, `while`, `match` or `for` (a macro's, or a block
  operand such as `if { ready } { … }`) are not its block. When a method call, field or `?` follows
  that `}`, it runs up to its `,` or `;`
  instead. Other `cfg` expressions and macros are not evaluated. A source the lexer cannot read
  (an unterminated literal or comment, an unknown escape, unbalanced delimiters) is an error.
- A word matches as a whole word: the characters next to it are not letters, digits or `_`, in any
  script. `MatchCase::Exact` compares characters exactly, and `MatchCase::AsciiInsensitive` ignores
  the case of ASCII letters. `allowed` and `exceptions` exempt a whole decoded literal or
  description or trimmed text line, never part of one.
- In Rust, the marker counts only inside a comment on a line the literal spans. In a manifest it
  counts anywhere on a line the description spans, and in a text file anywhere on the line.
- `scan` identifies a file by its input path joined to the base folder, and joins a relative base
  folder to the current folder before it lists any input (on Windows, a drive-relative base such as
  `C:src` to the current folder of its drive), so a relative and an absolute input for one file are
  read once; `..` and links are not resolved. A drive-relative input is joined to the current
  folder of its drive once, before it is listed, and its files are opened under that path.
- `scan` reads each file once, in path order, and returns the findings in path order, then source
  order, then the order of `banned`. A `Finding` has the path, the first line of the literal or
  description, the word, the decoded text or text line and the kind; its `Display` text is
  `path:line: "word" in "text"`. `scan_source` scans one source already in memory, and
  `literals` returns a source's literals.
- `names::Error` is `Policy` (no banned words, an empty word or an empty marker), `Io` (with the
  path), `InvalidUtf8`, `FileTooLarge`, `TotalTooLarge`, `TooDeep`, `Conflict` (a file given as two
  kinds of input) or `Syntax` (with the path and line). `DEFAULT_LIMITS` is 8 MiB per file, 512 MiB
  per scan and folders 64 deep; a file larger than the per-file limit, or than what is left of the
  scan's limit, is rejected before it is read.

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

## Assets

The app owns its `cargo xtask assets` command, the printing and the gate step. The library lists the
repository's files, reads the attribution document and reports each asset without a row:

```rust,no_run
use craft_xtask::assets::{self, FolderExtensions, Policy};

const POLICY: Policy<'static> = Policy {
    all_files_under: &["assets", "docs/images"],
    ext_in_folders: &[FolderExtensions { folder: "examples", nested: true, extensions: &["csv", "json"] }],
    ..Policy::new("ATTRIBUTION.md")
};

fn assets(root: &std::path::Path) -> Result<(), String> {
    let report = assets::run(root, &POLICY).map_err(|e| e.to_string())?;
    if report.is_ok() {
        println!("assets: all {} asset files attributed in ATTRIBUTION.md", report.assets);
        return Ok(());
    }
    let lines: Vec<String> = report.violations.iter().map(ToString::to_string).collect();
    Err(format!("{} asset rule violations:\n  {}", lines.len(), lines.join("\n  ")))
}
```

- A file is an asset when it is below a folder in `all_files_under`, when its extension is in
  `ASSET_EXT` or `extra_ext`, or when its extension is in an `ext_in_folders` entry and it is below
  that folder (at the repository root, or at any depth with `nested: true`). `ASSET_EXT` holds 69
  extensions: the lists of the apps in the Origin table, and `ttc` and `otc`. Extensions compare
  without regard to ASCII case; folders compare by whole path components and with case.
- `attribution_file` is the document's repository path (`ASSETS.md`, `ATTRIBUTION.md`,
  `assets/ATTRIBUTION.md`). Only backticked spans in the first cell of a Markdown table row count.
  A table row is a line that starts with `|`, or a line of a table whose second line is a delimiter
  row (`---|---`), up to the next blank line. Lines in fenced code blocks and lines indented 4
  columns or more are not read. A span opened by a run of backticks ends at the next run of the
  same length, so ``` `` a`b.png `` ``` names `` a`b.png ``. A `|` inside a span or after `\` does
  not end a cell.
- `RowMode::Exact` (the default) needs a span that spells the path byte for byte.
  `RowMode::Patterns` reads `*` as any run of characters except `/` and a trailing `/` as every
  file below that folder. A file pattern costs its length times the path's length to match, however
  many `*` it holds. A folder pattern is matched against each folder prefix of the path, so its cost
  is its length times the sum of those prefixes' lengths.
- An entry with `\`, a leading `/` or drive letter, or an empty, `.` or `..` component is reported
  as a `BadEntry`. An unattributed asset whose path a row spells with different letter case is
  reported with that row.
- `forbidden_ext` files are reported whatever their folder, and are not checked further.
  `forbidden_name_fragments` are matched against an asset's file name without regard to ASCII case.
  With `license_column: Some(n)`, every backticked span containing `/` in column `n` must name a
  regular file; neither the file nor a folder on the way to it may be a symbolic link.
  `skip_folders` files are checked only against `forbidden_ext`; `Report::skipped_by_folder` counts
  them.
- `list_files` runs `git ls-files -z --cached --others --exclude-standard`: tracked files, ignored
  or not, and untracked files that `.gitignore`, `.git/info/exclude` and the global exclude file do
  not ignore. It reads at most `max_git_output_bytes` (32 MiB by default) and ends the child when it
  writes more. A failure exit status, a name that is not UTF-8 and a name that a row cannot name
  (one that is not relative, or has a `\`, a drive prefix such as `C:`, or an empty, `.` or `..`
  component, which Git can list on Unix) are errors. A file listed more than once (an unmerged file) is checked once. Names that no
  longer exist are left out. Symbolic links, submodule folders, other entries that are not regular
  files, and files reached through a linked folder are not followed or checked;
  `Report::skipped_entries` counts them.
- `read_document` fails before reading a document larger than `max_document_bytes` (8 MiB by
  default), and fails on a document that is not UTF-8.
- `check` takes the file list and the document text, and a function that answers whether a license
  file exists, so an app's tests run without Git or a file system. `Error` is `Policy`, `Git`,
  `GitName`, `Document` or `Metadata`.

The source copies differ from this module in a few rules. An app that switches lists the results
that change, and the files that the 69 extensions newly select:

- Every source copy but LightCraft accepts a backticked path anywhere in the document, in prose or
  in any column. DeckCraft also takes every backticked span in the document as a pattern. No source
  copy skips code blocks or reads multi-backtick spans.
- No source copy reports bad entries or case-only near rows, or checks an unmerged file once. Each
  one follows symbolic links to the file and counts it as an asset, and none has a size limit on
  the document.
- VectorCraft (`repo_files` in `xtask/src/main.rs`), DesignCraft, CADCraft, GridCraft and
  LightCraft list files without `-z`, so Git quotes a name with a non-ASCII letter, a `"`, a `\` or
  a control character such as a newline, and the check passes over it. DesignCraft, CADCraft and
  DeckCraft do not check Git's exit status.
- DeckCraft and WordCraft replace bytes of a name that is not UTF-8; this module reports an error.
- DeckCraft's patterns have no trailing-`/` folder form. LightCraft's checker skips table rows
  with fewer than five cells and first-cell spans without a `/`.

## Lock

`RunOptions::with_lock(LockConfig::new(dir, slots, owner_label))` takes one of `slots` lock files in
`dir` before the first step and releases it when `run_steps` returns. The OS releases a lock when its
process ends. A slot count of 0 or more than 64, or a poll interval (`with_poll`, 2 s by default) of zero, is an
error before any file is opened. Next to each
held lock file, an owner file holds the owner label, which waiting runs print; `holders` reads at
most 4096 bytes of each and leaves out an owner file whose slot is free, such as one a killed run left
behind. An owner entry that is a link, or on Unix a file with more than one name, is also left
out, and a link is not followed. On Unix, a lock or owner open does not wait on a FIFO: a FIFO
lock file counts as a free slot to `holders`, and taking it is a lock error. Where the lock files live, the environment variables that set them, and
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

## Pins

`cargo xtask pdf-deps` checks that pinned dependencies resolve to the pinned version and source, with
the allowed features, and that packages the app forbids are not in the graph. The app enables the
`pins` feature, keeps its own table and passes it with a function that builds its `cargo` command:

```rust
use craft_xtask::pins::{FeatureRule, Forbidden, Name, Pin, Policy, Scope};

const REV: &str = "9f75d3dc19b4b514c4f3d4847ed1ec1b0e86b4ef";

Some("pdf-deps") => {
    let source = craft_xtask::pins::git_source("https://github.com/storytold/craft-libs", REV);
    let pins = [Pin { name: "craft-pdf", version: "0.1.0", source: &source, features: FeatureRule::Exactly(&["read"]) }];
    let forbidden = [Forbidden { name: Name::Exact("krilla"), scope: Scope::Active }];
    craft_xtask::pins::run(cargo, &Policy { pins: &pins, forbidden: &forbidden }, &Default::default())
        .map_err(|e| e.to_string())
}
```

- `run` runs `cargo metadata --locked --format-version 1` and passes the JSON to `check`.
- Each `Pin` name has exactly one package, with the pinned `version` and `source`. `git_source(url, rev)`
  gives the source Cargo writes for a git dependency pinned to a full commit hash, and `CRATES_IO` the
  source for crates.io.
- `FeatureRule::Contains` requires the listed features and allows others. `FeatureRule::Exactly`
  requires every listed feature and allows no others. Both read the package's node in `resolve.nodes`.
- `Scope::Active` rejects a forbidden package that has a resolved node, development dependencies
  included. `Scope::ProductionFrom(root)` rejects a forbidden package reached from `root` over normal
  or build edges; one reached only through development edges is allowed. `Name::Family("hayro")`
  matches `hayro` and every `hayro-*` name.
- Packages the policy does not name are allowed, and an empty policy accepts any metadata.
- JSON that does not parse, or lacks a field a rule needs (`resolve.nodes`, a package `id`, a node's
  `features` or `deps`, an edge's `dep_kinds`), is an error.
- Cargo merges every target's features into one metadata result. `run_target_features` runs
  `cargo tree --target <target>` from a root package and checks that each line for one package has
  exactly the given features; `host_target` reads the host target from `rustc -vV`.
- `Limits` bounds each command's output. More standard output than the limit stops the command and
  returns an error. Standard error is read to its end, and its first `stderr_bytes` are kept for the
  error message:

  | Limit | Default |
  |---|---|
  | `metadata_bytes`: the `cargo metadata` JSON, also for JSON passed to `check` | 64 MiB |
  | `tree_bytes`: one `cargo tree` output | 64 MiB |
  | `version_bytes`: the `rustc -vV` output | 64 KiB |
  | `stderr_bytes`: standard error kept for a message | 64 KiB |

- `pins::Error` tells apart a command that could not start, could not be read, exited without
  success or printed too much; output that is not valid; and a `Violation` of the policy.

## Stats

`cargo xtask stats [--exact]` prints the `.rs` files, lines, `#[test]` attributes and `proptest!` blocks
of each workspace crate. The app passes the `cargo metadata --format-version 1 --no-deps` JSON, a
function that builds its `cargo` command, and a writer:

```rust,ignore
Some("stats") => {
    let options = craft_xtask::stats::Options { exact: rest.contains(&"--exact"), ..Default::default() };
    craft_xtask::stats::run(&metadata_bytes()?, &options, cargo, &mut std::io::stdout().lock())
        .map(drop)
        .map_err(|e| e.to_string())
}
```

- Each workspace member's folder is walked. Folders named `target` or `fuzz` and folders whose names
  start with `.` are skipped, and a member inside another member's folder is counted in its own row
  only. Symbolic links and Windows junctions are not followed, to folders or to files.
- `#[test]` counts the text `#[test]` on lines that do not start with `//`. Tests that macros generate
  count once, and `#[tokio::test]` is not counted. `--exact` runs
  `cargo test -q -p <crate> -- --list` for each crate and counts the lines that end in `: test`. A
  crate whose listing fails shows `error`, and a line under the table names it.
- `Limits` bounds every input. A larger input is an error naming the path and the limit:

  | Limit | Default |
  |---|---|
  | `metadata_bytes`: the `cargo metadata` JSON | 64 MiB |
  | `source_bytes`: one `.rs` file | 16 MiB |
  | `total_source_bytes`: all `.rs` files of a run | 1 GiB |
  | `entries`: folder entries of a run | 1,000,000 |
  | `depth`: folder levels below a crate's folder | 256 |
  | `harness_output_bytes`: one crate's `--list` output | 64 MiB |

  A folder or file that cannot be read and a `.rs` file that is not UTF-8 are errors too.
- `collect` returns the `Report` without writing it, and `write_table` writes a `Report`.

## Version

```rust,no_run
use std::path::Path;
use craft_xtask::runner;

/// `rest` holds the words after `version`.
fn version(rest: &[&str]) -> Result<(), String> {
    let root = runner::root(Path::new(env!("CARGO_MANIFEST_DIR")))?;
    let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
    craft_xtask::version::run(&root, &cargo, rest)
}
```

- `cargo xtask version` prints the `version` of `[workspace.package]`.
- `cargo xtask version set X.Y.Z[-pre]` checks the new version, writes it into `Cargo.toml`, runs
  `cargo update --workspace --offline` in the root and, when that fails, `cargo update --workspace`,
  and prints `version: OLD -> NEW`. It also moves the `version` of each path dependency in
  `[workspace.dependencies]` whose requirement is `"OLD"` or `"=OLD"`; registry dependencies and path
  dependencies at other versions keep theirs. Trailing ASCII spaces in a matching requirement are retained.
- A version is `MAJOR.MINOR.PATCH` with an optional `-pre.release`: no leading zeros, numbers up to
  `u64::MAX`, and no build metadata (`+...`).
- When both updates fail, `Cargo.toml` and `Cargo.lock` are put back as they were (a lock file that did
  not exist is removed) and the error says so. Both files are replaced atomically through
  `craft-atomic`.
- Setting the current version prints `version: OLD (unchanged)` and writes only its lock file.
- One `set` runs at a time in a workspace. Before it reads `Cargo.toml`, it takes its locks and
  holds them until it returns. On Unix it locks the workspace directory itself; on Windows it locks
  `workspace.lock` in the first directory below, opened so that the file cannot be deleted while it
  is held. Every process that writes the workspace shares that lock, and `cargo clean` cannot remove
  it. It also takes the locks in `lock_dirs(root)`:
  `target/xtask-version/version-<hash of the canonical root>/` (required; its owner file names the
  run) and `~/.cache/craft-xtask/version-<hash of the canonical root>/` (when its file can be
  opened). A `set` that finds a lock held returns an error naming the other run and writes
  nothing.
- The root manifest is read and written with `toml_edit`, keeping comments, quotes and line endings
  (LF and CRLF), and the version may be `[workspace.package]` `version`, a dotted key or an inline
  table. A manifest that is not valid TOML (a duplicate key included), a version that is not a
  one-line string, or a current version that is not valid is an error before anything is written. `Cargo.toml` is read up to 1 MiB
  and `Cargo.lock` up to 64 MiB by default; `run_with` takes other `Limits`.

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
| `stats`: the folder walk that skips directory links, and the table | DesignCraft `xtask/src/stats.rs` at `dc9eea2695` |
| `stats`: regular files only, and `a_walk_does_not_follow_directory_links` | WordCraft `xtask/src/stats.rs` at `6b163b33dc` |
| `stats`: the entry budget that returns an error, and its two tests | CADCraft `xtask/src/stats.rs` at `ca87680fb0` |
| `stats`: the per-file size limit, and its two tests | GridCraft `xtask/src/stats.rs` at `9e9f13114f` |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report; in `names`, the full escape decoder, byte and C strings as an option, every `#[cfg(test)]` item form and `#![cfg(test)]`, the syntax errors, the marker only in comments, word boundaries in every script, the limits, the sorted findings and the skipped folder links; in `stats`, the metadata, total-size, depth and `--list` output limits, the metadata checks and the line naming failed listings | written here |
| `version`: `read`, `validate` with the overflow and prerelease checks, the restore after a failed update, and the tests `reads_the_workspace_version`, `replaces_only_the_workspace_version`, `keeps_crlf_line_endings`, `validates_versions`, `rejects_what_cargo_rejects`, `a_failed_update_restores_the_manifest` | SoundCraft `xtask/src/version.rs` at [6c84416](https://github.com/storytold/soundcraft/blob/6c84416c3cffb72d8d350ddfefc4fc9ee954ca1a/xtask/src/version.rs) |
| `version`: the path dependency pins and the test `moves_the_workspace_crates_pins_with_it` | PdfCraft `xtask/src/version.rs` at [3e30645](https://github.com/storytold/pdfcraft/blob/3e3064532f04fd6ec769c6bd7b2c97464757970e/xtask/src/version.rs) |
| `version`: the `toml_edit` edit that keeps quotes and line endings, the `=OLD` pins, the `Cargo.lock` restore, the size limits, the unchanged-version case and the one-`set`-at-a-time lock | written here |
| `assets`: `is_asset`, the exact-row check, `run`, the three asset folders and the test `detects_unattributed_assets` | VectorCraft `xtask/src/assets.rs` at [bada8e1](https://github.com/storytold/vectorcraft/blob/bada8e17103db9917de44e40a1cead5a7fc71c3c/xtask/src/assets.rs) and DesignCraft `xtask/src/assets.rs` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/assets.rs) |
| `assets`: the CADCraft and WordCraft extension lists, the `-z` listing, the exit-status check and the tests in `assets::tests::wordcraft` | CADCraft `xtask/src/assets.rs` at [8e69890](https://github.com/storytold/cadcraft/blob/8e6989022398ff5f79adaab77d19fdd7137591f3/xtask/src/assets.rs), WordCraft `xtask/src/assets.rs` at [42a5df5](https://github.com/storytold/wordcraft/blob/42a5df5f94e03288037f731ffd86dee7c7cf1332/xtask/src/assets.rs) |
| `assets`: the DeckCraft extension list, the pattern table and the tests in `assets::tests::deckcraft` | DeckCraft `xtask/src/assets.rs` at [b0c77eb](https://github.com/storytold/deckcraft/blob/b0c77ebd6a6fe6c9037cfbe1c8a7f397304b2205/xtask/src/assets.rs) |
| `assets`: folder-only extensions and the tests in `assets::tests::gridcraft` | GridCraft `xtask/src/assets.rs` at [f204433](https://github.com/storytold/gridcraft/blob/f2044337c7e195bdf6bec1b3914937a3dd386749/xtask/src/assets.rs) |
| `assets`: first-cell rows, folder patterns, forbidden extensions and names, the license column and the tests in `assets::tests::lightcraft` | LightCraft `xtask/src/assets.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/assets.rs) |
| `assets`: the policy checks, the cell parser, bad-entry and near-row reports, the size limits, UTF-8 names, skipped folders and the regular-file test | written here |
| `pins`: exact feature sets, the production walk over normal and build edges, the per-target `cargo tree` feature check, and their five tests | DeckCraft `xtask/src/pdf_deps.rs` at `aa3a1baa01` |
| `pins`: one package per name with an exact version and source, and its three tests | VectorCraft `xtask/src/pdf_deps.rs` at `34627134fa` |
| `pins`: required features that allow others, and its five tests | DesignCraft `xtask/src/pdf_deps.rs` at `18246dddd2` |
| `pins`: packages forbidden when resolved, and its five tests | EffectCraft `xtask/src/pdf_deps.rs` at `f31a6b0aef` |
| `pins`: the output size limits, an empty feature list in `cargo tree`, and the errors for missing fields, features that are not strings, edges without a kind and packages without a name | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| AniCraft, CADCraft, DeckCraft, FigCraft, GridCraft, LightCraft, PhotoCraft, WordCraft | Planned: the lexer and matcher of `display_names.rs` move to `names`, with `MatchCase::Exact`. `BANNED`, `ALLOWED`, `ROOTS`, the prefix exceptions (LightCraft, WordCraft), PhotoCraft's catalog test and the tests named after them stay in the app. |
| VectorCraft | Planned: `cargo xtask brands` calls `names::scan` with `MatchCase::AsciiInsensitive`, byte and C strings, `brand-ok` and its desktop entry as `Input::text_mark_above`. The word list, the file selection from `git ls-files` and the message stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| DeckCraft, DesignCraft, EffectCraft, VectorCraft | Planned: `xtask/src/pdf_deps.rs` moves to `pins`. Each app keeps its `pdf-deps` command and its table, and runs `cargo xtask pdf-deps` before and after the switch. |
| CADCraft, DeckCraft, DesignCraft, GridCraft, LightCraft, PhotoCraft, VectorCraft, WordCraft | Planned: `xtask/src/stats.rs` moves to `stats`. Each app's switch lists its per-crate counts before and after in its pull request. |
| SoundCraft, PdfCraft | Planned: `xtask/src/version.rs` moves to `version`, with the app's root and `CARGO` passed to `run`. `the_real_manifest_has_a_valid_version`, and PdfCraft's `workspace_rust_version_covers_the_ui_crate`, stay in the app. |
| VectorCraft, DesignCraft, CADCraft, DeckCraft, WordCraft, GridCraft, LightCraft | Planned: `xtask/src/assets.rs` moves to `assets`, each with its own document, folders and row mode; LightCraft with `RowMode::Patterns`, its forbidden extensions and names, and `license_column: Some(4)`. The `cargo xtask assets` output stays in the app. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
