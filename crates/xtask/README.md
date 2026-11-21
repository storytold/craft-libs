# craft-xtask

A library for the Craft apps' `cargo xtask` binaries. Each app keeps its own `xtask` binary, command
dispatch, step list, test targets and app-only commands, and calls this crate with its own policy.
The crate is native only and is a dependency of an app's `xtask` package only.

| Module | Feature | Contents |
|---|---|---|
| `runner` | always | `Step`, `run_steps` (steps in order, stop at the first failure, the `ok`, `FAIL` and `skip` summary), `root`, `cargo`, `run`, `run_cancelable`, `nextest_or_test`, `doctest`, `Cancel` |
| `assets` | always | `Policy`, `RowMode`, `check`, `run`, `list_files`, `read_document`, `is_asset`, `ASSET_EXT`: the asset attribution check against the document, folders and extensions the app supplies |
| `layers` | always | `Policy`, `Class`, `check`, `wasm_set`, `from_metadata`, `cargo_metadata`, `read_json`: the crate-layer check against a table the app supplies |
| `lock` | `lock` | `LockConfig`, `try_acquire`, `acquire`, `holders`: 1 to 64 slots held across processes through file locks in a directory the app chooses |
| `report` | `report` | `write` and `to_json`: the per-step JSON report (schema 1), replaced atomically through `craft-atomic` |

No feature is on by default. The crate depends on `serde_json`; `report` adds `craft-atomic`.

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

## Layers

The app owns its table, its exceptions, the `cargo xtask layers` command, the printing and the
`cargo check --target` call of its wasm step. The library reads the workspace, checks it and picks
the wasm packages:

```rust,no_run
use craft_xtask::layers::{self, Class, Policy};

const TABLE: &[(&str, Class)] = &[
    ("app-geom", Class::Layer(0)),
    ("app-color", Class::Layer(0)),
    ("app-doc", Class::Layer(1)),
    ("app-engine", Class::Layer(5)),
    ("app-ui", Class::Layer(6)),
    ("app-testkit", Class::Testkit),
    ("app", Class::Exempt),
    ("xtask", Class::BuildTool),
];
const POLICY: Policy<'static> = Policy {
    ordered_chains: &[&["app-geom", "app-color"]],
    ui_crates: &["egui*", "eframe", "winit", "rfd"],
    ui_min_layer: 6,
    testkit_layer: 6,
    testkit_crates: &["craft-testkit"],
    testkit_normal_users: &["xtask"],
    build_tool_crates: &["craft-xtask"],
    wasm_max_layer: Some(5),
    wasm_extra: &["app-ui"],
    ..Policy::new(TABLE)
};

fn layers(root: &std::path::Path) -> Result<(), String> {
    let meta = layers::cargo_metadata(root, layers::DEFAULT_METADATA_LIMIT).map_err(|e| e.to_string())?;
    let crates = layers::from_metadata(&meta).map_err(|e| e.to_string())?;
    let violations = layers::check(&POLICY, &crates).map_err(|e| e.to_string())?;
    for v in &violations {
        eprintln!("{v}");
    }
    if violations.is_empty() { Ok(()) } else { Err(format!("{} layer violations", violations.len())) }
}
```

- Table names are exact package names. A name may appear once. A table entry that matches no
  workspace package is an error unless `optional` lists it, and an empty table or an empty workspace
  is an error. Every name in `optional`, `ordered_chains`, `sideways`, `dev_exceptions`,
  `standalone_deps`, `format_allowed`, `testkit_normal_users` and `wasm_extra` must be in the table;
  the names in `ui_crates`, `ui_dev_allowed`, `testkit_crates` and `build_tool_crates` are not
  checked against the table.
- `Class::Layer(n)` crates depend on lower layers. `Class::Standalone(n)` crates depend only on the
  standalone crates `standalone_deps` lists for them, at layer `n` or lower, and on test tooling as a
  dev-dependency. `Class::Testkit` crates sit at `testkit_layer` for their own dependencies, and other
  crates use them, and the external `testkit_crates`, only as dev-dependencies, except the packages in
  `testkit_normal_users`. `Class::BuildTool` packages (the app's `xtask`) are exempt from the layers
  and are the only packages that may depend on another build tool or on `build_tool_crates`.
  `Class::Exempt` packages (apps, CLIs, web shells) are exempt from the layers and the UI rule only.
- Same-layer dependencies: `SameLayer::Ordered` allows a later member of an `ordered_chains` chain to
  use an earlier one, and each `sideways` pair `(from, to)`. `SameLayer::Allowed` allows every
  same-layer dependency. Either way, same-layer dependencies that form a cycle are reported.
- Normal, build and dev dependencies are all checked. `dev_exceptions` pairs pass as
  dev-dependencies only, and `ui_dev_allowed` UI crates pass as dev-dependencies at any layer. A
  same-layer dev exception still counts toward cycles. A crate's workspace dependency on itself is
  skipped. `format_allowed` limits a crate's normal and build
  dependencies on workspace crates to a list, on top of the layer rule.
- `ui_crates` entries match a name exactly, or, ending in `*`, by prefix. UI crates are allowed from
  `ui_min_layer`.
- `from_metadata` reads `cargo metadata --format-version 1 --no-deps`. A dependency is a workspace
  member when its `path` is a member's manifest directory (`\` and `/` compare equal), so a registry
  crate or an outside path crate with a member's name stays external, and a renamed member is matched
  by its package `name`. A missing `packages`, `workspace_members`, `dependencies`, `name`, `id`,
  `manifest_path` or `kind`, an unknown `kind`, a package missing from `workspace_members` and a
  repeated package are errors.
- `cargo_metadata` and `read_json` read at most the given number of bytes (`DEFAULT_METADATA_LIMIT`
  is 64 MiB) from the child's stdout, end the child when it writes more, and return an error for a
  spawn failure, a failure exit status or output that is not JSON. Stderr is inherited, also when the
  caller set it to a pipe.
- `check` returns the violations sorted by kind and fields, without duplicates; their `Display` text
  names the packages, layers and dependency kind. `wasm_set` returns the layered and standalone
  packages at or below `wasm_max_layer` and the `wasm_extra` packages the workspace has, sorted, with
  no test or build tooling. An empty set is an error. It does not check that the packages build for
  wasm.

The source copies differ in a few rules. An app that switches lists the results that change:

- DesignCraft and its copies skip an unregistered workspace dependency, take a path dependency as a
  workspace member whatever its directory, read a missing `dependencies` as empty and an unknown kind
  as normal, and take the first of two table rows with one name. Each of these is an error or a
  violation here.
- No source copy reports a same-layer cycle, or a test-tooling or build-tool dependency of an exempt
  package.
- DesignCraft reports a standalone crate's dev-dependency on test tooling; this crate allows it, as
  PdfCraft does. PhotoCraft's `STANDALONE_EXCEPTIONS` pairs become `standalone_deps` entries.
- EffectCraft keeps a flat table with its own same-layer pairs and does not check dev-dependencies.

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

## Origin

| Part | Source |
|---|---|
| `runner`: the step loop and summary, `root`, `cargo`, `run`, the nextest probe and fallback, doctests | DesignCraft `xtask/src/main.rs:68-82`, `:160-201`, `:204-249` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/main.rs) |
| `lock` and its tests `one_slot_is_exclusive_until_dropped`, `more_slots_admit_more_runs` | LightCraft `xtask/src/ci_lock.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci_lock.rs) |
| `report`: the per-step fields | LightCraft `xtask/src/ci.rs:347-414` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/ci.rs#L347-L414) |
| `layers`: `Class`, `Dep`, `Crate`, `Violation`, `check`, `from_metadata`, the ordered chains and UI prefixes, and the tests `clean_downward_graph_passes`, `upward_dependency_flagged`, `sideways_dependency_flagged`, `l0_foundation_chain_allowed_one_way`, `self_dev_dependency_ignored`, `upward_dev_dependency_flagged`, `ui_crates_below_l6_flagged`, `unregistered_crate_is_error`, `testkit_only_as_dev_dependency`, `apps_and_xtask_exempt`, `metadata_parsing` | DesignCraft `xtask/src/layers.rs` at [4bc6f8a](https://github.com/storytold/designcraft/blob/4bc6f8a730f1f12321f5a442b94f30513d7a3f93/xtask/src/layers.rs) |
| `layers`: numbered standalone crates and their lists, sideways pairs, UI dev-dependencies, and the tests in `layers::tests::pdfcraft` | PdfCraft `xtask/src/layers.rs` at [ee9bf28](https://github.com/storytold/pdfcraft/blob/ee9bf2812671f9f7b53e631f4f9f8b6543bf7995/xtask/src/layers.rs) |
| `layers`: standalone exceptions and the tests in `layers::tests::photocraft` | PhotoCraft `xtask/src/layers.rs` at [58d11d9](https://github.com/storytold/photocraft/blob/58d11d91ea46041650058284163008d534431329/xtask/src/layers.rs) |
| `layers`: format allowlists and the tests in `layers::tests::gridcraft` | GridCraft `xtask/src/layers.rs` at [06c2b14](https://github.com/storytold/gridcraft/blob/06c2b1440b0134ec184561dcc2275fdb5f18a5af/xtask/src/layers.rs) |
| `layers`: dev-only exceptions | DeckCraft `xtask/src/layers.rs` at [4b93429](https://github.com/storytold/deckcraft/blob/4b93429c9ec3921042f5d4ff27efa5ae999336fb/xtask/src/layers.rs) |
| `layers`: `SameLayer::Allowed` and `sideways` for a flat table (EffectCraft allows every same-layer dependency in L0 and listed pairs above it) | EffectCraft `xtask/src/main.rs` at [d4e33b2](https://github.com/storytold/effectcraft/blob/d4e33b2bda4515d456408b9bd8afb7e536770a40/xtask/src/main.rs) |
| `assets`: `is_asset`, the exact-row check, `run`, the three asset folders and the test `detects_unattributed_assets` | VectorCraft `xtask/src/assets.rs` at [bada8e1](https://github.com/storytold/vectorcraft/blob/bada8e17103db9917de44e40a1cead5a7fc71c3c/xtask/src/assets.rs) and DesignCraft `xtask/src/assets.rs` at [8b169f8](https://github.com/storytold/designcraft/blob/8b169f8ee7832e60ed1ac4c9f5dd34f954d3a992/xtask/src/assets.rs) |
| `assets`: the CADCraft and WordCraft extension lists, the `-z` listing, the exit-status check and the tests in `assets::tests::wordcraft` | CADCraft `xtask/src/assets.rs` at [8e69890](https://github.com/storytold/cadcraft/blob/8e6989022398ff5f79adaab77d19fdd7137591f3/xtask/src/assets.rs), WordCraft `xtask/src/assets.rs` at [42a5df5](https://github.com/storytold/wordcraft/blob/42a5df5f94e03288037f731ffd86dee7c7cf1332/xtask/src/assets.rs) |
| `assets`: the DeckCraft extension list, the pattern table and the tests in `assets::tests::deckcraft` | DeckCraft `xtask/src/assets.rs` at [b0c77eb](https://github.com/storytold/deckcraft/blob/b0c77ebd6a6fe6c9037cfbe1c8a7f397304b2205/xtask/src/assets.rs) |
| `assets`: folder-only extensions and the tests in `assets::tests::gridcraft` | GridCraft `xtask/src/assets.rs` at [f204433](https://github.com/storytold/gridcraft/blob/f2044337c7e195bdf6bec1b3914937a3dd386749/xtask/src/assets.rs) |
| `assets`: first-cell rows, folder patterns, forbidden extensions and names, the license column and the tests in `assets::tests::lightcraft` | LightCraft `xtask/src/assets.rs` at [25ffe3e](https://github.com/storytold/lightcraft/blob/25ffe3e5d4ac754829b99dd7368d94185adbf8d5/xtask/src/assets.rs) |
| `assets`: the policy checks, the cell parser, bad-entry and near-row reports, the size limits, UTF-8 names, skipped folders and the regular-file test | written here |
| The root and slot-count errors, the `--exclude` of the xtask package, cancellation, the bounded owner read and the atomic report; in `layers`, the policy checks, membership by manifest directory, the strict metadata reader, the bounded metadata command, cycle detection, the build-tool class and `wasm_set` | written here |

## Migration

No app uses this crate yet.

| App | Status |
|---|---|
| DesignCraft | Planned: `cmd_ci`, `root`, `cargo`, `run`, `cmd_test` and `cmd_doctest` move to `runner`, with `TestMode::PreferNextest`. The step list, `TEST_TARGETS` and the i18n report stay in the app. |
| DesignCraft, CADCraft, VectorCraft, DeckCraft, WordCraft, PdfCraft, PhotoCraft, GridCraft, EffectCraft | Planned: `xtask/src/layers.rs` (EffectCraft: the check in `xtask/src/main.rs`) moves to `layers`. The table, chains, exceptions, UI layer and the tests of the app's own policy stay in the app. |
| LightCraft | Planned: `ci_lock.rs` moves to `lock`, with `LIGHTCRAFT_CI_LOCK` and `LIGHTCRAFT_CI_SLOTS` read by the app. The machine, GPU, corpus and skip report stays in the app. |
| VectorCraft, DesignCraft, CADCraft, DeckCraft, WordCraft, GridCraft, LightCraft | Planned: `xtask/src/assets.rs` moves to `assets`, each with its own document, folders and row mode; LightCraft with `RowMode::Patterns`, its forbidden extensions and names, and `license_column: Some(4)`. The `cargo xtask assets` output stays in the app. |

Maintainers: the craft-libs maintainers, until a responsible maintainer or team is agreed in the
review of this crate's first pull request.
