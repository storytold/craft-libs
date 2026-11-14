# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
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
| LightCraft, VectorCraft, PhotoCraft, DesignCraft, WordCraft | Not switched: each app's own pinned-download code for its test corpora can move to `fetch`. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
