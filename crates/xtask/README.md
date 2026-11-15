# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |
| `stats` | `stats` | `run`, `collect` and `write_table`: files, lines and tests of Rust per workspace crate (`cargo xtask stats`) |

No feature is on by default. `report` adds `serde_json` and `craft-atomic`, and `stats` adds
`serde_json`.

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

## Lock

`RunOptions::with_lock(LockConfig::new(dir, slots, owner_label))` takes one of `slots` lock files in
`dir` before the first step and releases it when `run_steps` returns. The OS releases a lock when its
process ends. A slot count of 0 or more than 64, or a poll interval (`with_poll`, 2 s by default) of zero, is an
error before any file is opened. Next to each
held lock file, an owner file holds the owner label, which waiting runs print; `holders` reads at
most 4096 bytes of each and leaves out an owner file whose slot is free, such as one a killed run left
behind. An owner entry that is a link, or on Unix a file with more than one name, is also left
out, and a link is not followed. Where the lock files live, the environment variables that set them, and
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

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| `stats`: the folder walk that skips directory links, and the table | DesignCraft `xtask/src/stats.rs` at `dc9eea2695` |
| `stats`: regular files only, and `a_walk_does_not_follow_directory_links` | WordCraft `xtask/src/stats.rs` at `6b163b33dc` |
| `stats`: the entry budget that returns an error, and its two tests | CADCraft `xtask/src/stats.rs` at `ca87680fb0` |
| `stats`: the per-file size limit, and its two tests | GridCraft `xtask/src/stats.rs` at `9e9f13114f` |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report; in `stats`, the metadata, total-size, depth and `--list` output limits, the metadata checks and the line naming failed listings | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| CADCraft, DeckCraft, DesignCraft, GridCraft, LightCraft, PhotoCraft, VectorCraft, WordCraft | Planned: `xtask/src/stats.rs` moves to `stats`. Each app's switch lists its per-crate counts before and after in its pull request. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
