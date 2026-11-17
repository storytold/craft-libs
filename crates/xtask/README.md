# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `assets` | always | `Policy`, `RowMode`, `check`, `run`, `list_files`, `read_document`, `is_asset`, `ASSET_EXT`: the asset attribution check against the document, folders and extensions the app supplies |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |
| `fetch` | `fetch` | `fetch_pinned`, `fetch_archive`, `extract`, `sha256_hex`: files of a pinned list downloaded over HTTPS and checked against SHA-256, and tar archives extracted under entry and size limits |

No feature is on by default. `report` adds `serde_json` and `craft-atomic`.
`fetch` adds `craft-atomic`, `sha2`, `tar` and `flate2`, and runs the system `curl`.

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

## Fetch

| Function | What it does |
|---|---|
| `fetch_pinned(dest, pins)` | puts each `Pin { name, url, sha256, max_bytes }` at `dest/name`: a regular file already there with the pinned SHA-256 is kept, any other file there is deleted, and the file is downloaded to a temporary file beside it, counted and hashed while it is written, and renamed into place only when its size stays within `max_bytes` and its SHA-256 matches |
| `fetch_archive(dest, pin, options)` | downloads one pinned tar archive, plain or gzip-compressed, checks that its size stays within `max_bytes` and that its SHA-256 matches, and writes its regular files and folders under `dest` |
| `extract(dest, name, bytes, options)` | the archive step of `fetch_archive`, for archive bytes the caller already holds |
| `fetch_pinned_with`, `fetch_archive_with` | the same, through a caller's `Transport` instead of `curl` |
| `sha256_hex(bytes)` | the lowercase hexadecimal SHA-256 |

Every pin is checked before anything is downloaded. A name is a relative path with `/` between
folders; it may not be empty or absolute, hold an empty, `.` or `..` part, a backslash, a control
character or a character Windows does not allow in names (`:`, `<`, `>`, `"`, `|`, `?`, `*`), end a
part with a dot or a space, or use a Windows device name (`CON`, `NUL`, `COM1` and the rest). Two
pins may not name the same file ignoring case, and a file name may not also be used as a folder. The
address starts with `https://`, the SHA-256 has 64 hexadecimal digits, and `max_bytes` is at
least 1.

`curl` runs with `-q` (no `.curlrc`), `--proto =https` and `--proto-redir =https`, `--fail`,
`--max-filesize` set to `max_bytes`, a 10-second connect timeout, and a stop when a transfer stays
under 1 KiB per second for 60 seconds. The crate counts the bytes itself as well, so a response of
unknown length stops at `max_bytes` with any version of `curl`.

Neither `dest` nor any path under it may be a link. A link, or a folder where a file goes, or a
file where a folder goes, stops the fetch with `FetchError::NotRegular`. The folders above `dest`
are followed like any other path, links included.

The archive is held in memory, so the pin's `max_bytes` also bounds its memory. Before anything is
written, every entry is checked: a symbolic or hard link, a device, a FIFO or any other special
entry, a path that is absolute, holds `..`, is not UTF-8 or fails the name rules above, and a file
name repeated ignoring case each stop the extraction. `ExtractOptions` holds the limits:

| Field | Default |
|---|---|
| `max_entries` | 1000, folders included |
| `max_entry_bytes` | 4 MiB |
| `max_expanded_bytes` | 16 MiB for all files together |
| `strip_components` | 0; leading folders removed from every path, as `tar --strip-components`, except that leading `.` parts are not counted and the removed names must pass the name rules |

The tar data, decompressed or plain, is bounded too: `max_expanded_bytes` plus 12 KiB for headers,
padding and long-name records for each entry `max_entries` allows, plus one more. A gzip archive is
read to its end, so a damaged checksum or a cut trailer stops the extraction. Each file is written through a temporary file and a rename, so an
existing file at its name is replaced whole. Archives are not inferred from a file name: the
caller chooses `fetch_archive` or `fetch_pinned`.

The pins come from the caller. Reading a manifest file into pins is left to the caller.

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report | written here |
| `assets`: `is_asset`, the exact-row check, `run`, the three asset folders and the test `detects_unattributed_assets` | VectorCraft `xtask/src/assets.rs` at [bada8e1](https://github.com/storytold/vectorcraft/blob/bada8e17103db9917de44e40a1cead5a7fc71c3c/xtask/src/assets.rs) and DesignCraft `xtask/src/assets.rs` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/assets.rs) |
| `assets`: the CADCraft and WordCraft extension lists, the `-z` listing, the exit-status check and the tests in `assets::tests::wordcraft` | CADCraft `xtask/src/assets.rs` at [8e69890](https://github.com/storytold/cadcraft/blob/8e6989022398ff5f79adaab77d19fdd7137591f3/xtask/src/assets.rs), WordCraft `xtask/src/assets.rs` at [42a5df5](https://github.com/storytold/wordcraft/blob/42a5df5f94e03288037f731ffd86dee7c7cf1332/xtask/src/assets.rs) |
| `assets`: the DeckCraft extension list, the pattern table and the tests in `assets::tests::deckcraft` | DeckCraft `xtask/src/assets.rs` at [b0c77eb](https://github.com/storytold/deckcraft/blob/b0c77ebd6a6fe6c9037cfbe1c8a7f397304b2205/xtask/src/assets.rs) |
| `assets`: folder-only extensions and the tests in `assets::tests::gridcraft` | GridCraft `xtask/src/assets.rs` at [f204433](https://github.com/storytold/gridcraft/blob/f2044337c7e195bdf6bec1b3914937a3dd386749/xtask/src/assets.rs) |
| `assets`: first-cell rows, folder patterns, forbidden extensions and names, the license column and the tests in `assets::tests::lightcraft` | LightCraft `xtask/src/assets.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/assets.rs) |
| `assets`: the policy checks, the cell parser, bad-entry and near-row reports, the size limits, UTF-8 names, skipped folders and the regular-file test | written here |
| `fetch`: a list of name, address and SHA-256; a file with another SHA-256 is deleted so the next run downloads it again | LightCraft `xtask/src/corpus.rs` (`fetch_pinned`) at [6b2b7e6](https://github.com/storytold/lightcraft/blob/6b2b7e6d1117e683a1701568e56f979d061e4699/xtask/src/corpus.rs) |
| `fetch`: HTTPS-only `curl` arguments, a regular-file check before reuse, a per-file byte limit, and a download staged beside its destination and renamed after its check | VectorCraft `xtask/src/pngsuite.rs` at [ed96a12](https://github.com/storytold/vectorcraft/blob/ed96a1219620b5b6ab13e25369f7b55cddd664f6/xtask/src/pngsuite.rs) |
| `fetch`: the archive limits (1 MiB archive, 1000 entries, 16 MiB extracted) | DesignCraft `xtask/src/main.rs` (PngSuite download) at [f5087ea](https://github.com/storytold/designcraft/blob/f5087eae963c6f8474abb45669ce2b7275761e6d/xtask/src/main.rs) |
| `fetch`: the pin and entry name rules, the link checks, the bounded decompression and the in-process `Transport` | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| VectorCraft, DesignCraft, CADCraft, DeckCraft, WordCraft, GridCraft, LightCraft | Planned: `xtask/src/assets.rs` moves to `assets`, each with its own document, folders and row mode; LightCraft with `RowMode::Patterns`, its forbidden extensions and names, and `license_column: Some(4)`. The `cargo xtask assets` output stays in the app. |
| LightCraft, VectorCraft, PhotoCraft, DesignCraft, WordCraft | Not switched: each app's own pinned-download code for its test corpora can move to `fetch`. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
