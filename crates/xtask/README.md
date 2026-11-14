# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |
| `icon` | always | `ico::pack`, `icns::pack`, `png_size`, `png_info`, `read_pngs`, `Limits`, and `ico::read_file` and `icns::read_file` for an app's test of its committed icons |

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

## Icons

```rust,no_run
use std::path::Path;
use craft_xtask::icon::{self, Limits, ico};

fn cmd_ico(out: &Path, inputs: &[&Path]) -> Result<(), icon::Error> {
    let limits = Limits::default();
    let pngs = icon::read_pngs(inputs, &limits)?;
    let file = ico::pack(&pngs, &limits)?;
    std::fs::write(out, &file).map_err(|e| icon::Error::io(out, &e))?;
    Ok(())
}
```

The app renders the PNGs and chooses the sizes, the paths and the command name. Both formats store
each PNG as is; no image is decoded.

- Every PNG is checked whole: the signature, each chunk's length and CRC, a first `IHDR` of 13 bytes
  with valid fields, at most one `PLTE` before the first `IDAT`, at least one `IDAT` with no other
  chunk between two of them, no other critical chunk, and a final empty `IEND` with nothing after
  it. Images must be square and at least 1 px.
- `ico::pack` takes 1 to 256 px images in 8-bit RGBA, the form a PNG entry of an `.ico` holds
  ([ICO format, part 4](https://devblogs.microsoft.com/oldnewthing/20101022-00/?p=12473/)). A 256 px
  side is written as 0.
- `icns::pack` takes 16, 32, 64, 128, 256, 512 and 1024 px images and writes each into every slot of
  its size: 16 `icp4`; 32 `icp5`, `ic11`; 64 `icp6`, `ic12`; 128 `ic07`; 256 `ic08`, `ic13`; 512
  `ic09`, `ic14`; 1024 `ic10` ([ICNS format](https://hexbase.dev/formats/icns/)).
- Both write the images in increasing size, whatever the input order. Two images of one size are an
  error.
- `Limits` bounds the image count (default 16), each PNG (16 MiB) and the packed file (128 MiB). The
  formats' 32-bit offsets and lengths also cap a file at 4 GiB - 1 byte. Sizes are checked and the
  file's memory reserved before any byte is written. `read_pngs` stops reading a file one byte past
  the smaller of the PNG limit and what the total limit has left.
- `ico::read_file` and `icns::read_file` read a file within the same limits and check its structure.
  `ico::read` accepts only the entries `ico::pack` writes. Both readers hold each image to the PNG
  limit. `icns::read` returns at most four elements for each image `Limits` allows, and returns
  elements that are not PNGs as stored. An app's `committed_icon_is_valid` test passes its own icon
  path, for example `assets/app-icon/<app>.ico`.

Errors are `icon::Error`: `Empty`, `TooManyImages`, `InvalidPng`, `NotSquare`, `UnsupportedSize`,
`DuplicateSize`, `TooLarge`, `Allocation`, `InvalidIcon` or `Io`.

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| `icon::ico`: `pack`, `png_size` and the tests `packs_directory_and_payloads`, `rejects_bad_input`, `committed_icon_is_valid` | DeckCraft `xtask/src/ico.rs` at [ae9d525](https://github.com/storytold/deckcraft/blob/ae9d525bce365083bb62b4754bdd346feb2a79b0/xtask/src/ico.rs); CADCraft's copy differs only in its icon path |
| `icon::icns`: `pack`, the slot table and the tests `fills_every_slot_of_each_size`, `rejects_bad_input`, `committed_icon_is_valid`, `packs_icns_elements` | craft-launcher `xtask/src/icns.rs` at [019330f](https://github.com/storytold/craft-launcher/blob/019330f99aa90a3d6501e134fc48df8d719e15a2/xtask/src/icns.rs); MaterialCraft `xtask/src/ico.rs` at [6e952a9](https://github.com/storytold/materialcraft/blob/6e952a9a10f9bce77f330e430a828c066be6ad9b/xtask/src/ico.rs) |
| `icon`: the test `rejects_non_png` | VectorCraft `xtask/src/ico.rs` at [4373205](https://github.com/storytold/vectorcraft/blob/437320509da102f5d9aca44d411ffb254f071932/xtask/src/ico.rs) |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report | written here |
| The whole-PNG check, the RGBA check for `.ico`, the size order and duplicate check, `Limits`, the checked lengths and offsets, `read_pngs` and the readers | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| DeckCraft | Planned: `xtask/src/ico.rs` moves to `icon::ico`. The `ico` command and `packaging/icons.sh` stay in the app. |
| CADCraft | Planned: `xtask/src/ico.rs` moves to `icon::ico`. The `ico` command and `packaging/icons.sh` stay in the app. |
| craft-launcher | Planned: `xtask/src/ico.rs` and `icns.rs` move to `icon`. The `ico` and `icns` commands stay in the app. |
| MaterialCraft | Planned: `pack_ico` and `pack_icns` move to `icon`. The `ico` and `icns` commands stay in the app. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
