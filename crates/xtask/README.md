# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |
| `version` | `version` | `run`, `run_with`, `Limits`, `lock_dirs`, `read`, `replace`, `validate`: `cargo xtask version [set X.Y.Z[-pre]]` on the root `Cargo.toml` |

No feature is on by default. `report` adds `serde_json` and `craft-atomic`; `version` adds `craft-atomic` and turns on `lock`.

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
  dependencies at other versions keep theirs.
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
- The root manifest is read with a scanner that keeps comments, quotes and line endings (LF, CRLF and
  CR) and accepts the version as `[workspace.package]` `version`, a dotted key or an inline table. A
  manifest it cannot read, a duplicate version, a version that is not a one-line string, or a current
  version that is not valid is an error before anything is written. `Cargo.toml` is read up to 1 MiB
  and `Cargo.lock` up to 64 MiB by default; `run_with` takes other `Limits`.

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| `version`: `read`, `validate` with the overflow and prerelease checks, the restore after a failed update, and the tests `reads_the_workspace_version`, `replaces_only_the_workspace_version`, `keeps_crlf_line_endings`, `validates_versions`, `rejects_what_cargo_rejects`, `a_failed_update_restores_the_manifest` | SoundCraft `xtask/src/version.rs` at [6c84416](https://github.com/storytold/soundcraft/blob/6c84416c3cffb72d8d350ddfefc4fc9ee954ca1a/xtask/src/version.rs) |
| `version`: the path dependency pins and the test `moves_the_workspace_crates_pins_with_it` | PdfCraft `xtask/src/version.rs` at [3e30645](https://github.com/storytold/pdfcraft/blob/3e3064532f04fd6ec769c6bd7b2c97464757970e/xtask/src/version.rs) |
| `version`: the TOML scanner, the `=OLD` pins, the `Cargo.lock` restore, the size limits, the unchanged-version case and the one-`set`-at-a-time lock | written here |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| SoundCraft, PdfCraft | Planned: `xtask/src/version.rs` moves to `version`, with the app's root and `CARGO` passed to `run`. `the_real_manifest_has_a_valid_version`, and PdfCraft's `workspace_rust_version_covers_the_ui_crate`, stay in the app. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
