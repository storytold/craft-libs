# Development workflow

How to build, test and look at this workspace without waiting for it. Porting an app onto the framework is described in [porting.md](porting.md). The rules here exist because a cargo workspace of 157 crates, with a framework that every domain crate links, rebuilds far more than the change touched whenever two commands disagree about features or profiles.

## The commands

`tools/cl` is the entry point for every local cargo command (put it on your PATH, or run `tools/cl` from the worktree):

| What | Command | Builds |
|---|---|---|
| check a crate you changed but don't test | `cl check <crate>` | `cargo check` of the crate's lib, bins and tests |
| build its tests | `cl test <crate> --no-run` | `cargo test -p <crate> --tests --no-run`; prints each test binary's path |
| run some trials | `cl test <crate> -- <filter> --test-threads 4` | nothing new when warm; then runs each test binary with the filter |
| the debug gallery | `cl build gallery` | the `craft-ui-gallery` binary only |
| one component's stories as PNGs | `cl render <crate\|ui> <Component> [--story <name>] [--out <dir>]` | the `story-render` binary with that crate's feature |
| anything else | `cl cargo <args>` | what you asked for, through the job pool, nothing added |
| the features `cl` adds for a crate | `cl features <crate>` | nothing |

For a crate whose tests you build next, skip `cl check`: a check is a separate cargo build (metadata only) of the same crates, so after a framework edit the check and the test build each compile the edited crates' front end, and the test build reports the same errors at about the same point.

`<crate>` is the package name with or without `craft-` (`color`, `craft-ui-widgets`). `cl` uses `<worktree>-target` as the target directory when that directory exists beside the worktree, else `CARGO_TARGET_DIR`, else `target/`.

### The one-minute component loop

A fix to one gallery component, from the edit to its old/new picture on a page: edit the component's crate, then `cl render <crate> <Component>` (about 10–25 s warm: the changed crate, the renderer's link, the render) and compare the PNGs with the previous ones; put the old and the new image side by side in whatever image viewer or comparison page you use. `story-render` links the framework and only the domain crates its features name, so a change in craft-color rebuilds craft-color and the renderer, not the gallery and its 17 crates. `cl render ui <Component>` renders the framework's own components.

## One feature resolution

cargo compiles a crate once per distinct feature set, and every crate above it once more, and it unifies features only over the packages a command selects. A framework crate's tests have no eframe in their graph; a domain crate's tests do; the gallery adds every domain crate, whose dependencies turn on more features of egui, wgpu, craft-window, image and others below the framework. Each kind of command would build its own copy of the framework, and wgpu with it.

`cl` selects the `feature-root` package beside the package of every command (`-p <crate> -p feature-root`). Its dependencies are the gallery's, with every domain crate's `screen-testing`, so every local build resolves features as the gallery does, and a target directory holds one build of each crate. Nothing of `feature-root` is compiled: its one target is a bin no `cl` command selects (`cl` names the crate's own targets), and only the targets a command names are built, so a render still links one domain crate and a framework crate's tests still link no app. On top, `cl` adds the crate's own test feature (`screen-testing`, else `testing`). The Mac's and CI's commands never select `feature-root` and are unaffected.

Four manifest facts keep this true, and `cl doctor` checks them: `feature-root` lists every optional domain crate of the gallery with the gallery's features, and depends on the gallery without its default features (so the gallery lib's own features stay each command's: `apps` for the gallery, one crate for a render); the story renderer asks its shared dependencies for no feature the gallery's lines lack; dev-dependency lines ask egui for no features (a feature there differs between a crate's tests and everything else); the proc-macro crate pins syn's and proc-macro2's features to everything other derive crates may add, so the one syn every proc macro shares is the same with or without design-lint in the graph. `cl` names any new build of a framework crate in the target directory after each command; after a manifest change one is expected, otherwise run `cl doctor`.

## PhotoCraft

`tools/cl` finds its workspace with `git rev-parse --show-toplevel` from the current directory, so `/path/to/worktree/tools/cl` run inside a PhotoCraft tree builds and tests that tree. A workspace that has `photocraft-ui-egui` gets one fixed selection: `-p photocraft-ui-egui -p photocraft-engine -p photocraft -p photocraft-algo -p photocraft-automation -p photocraft-text -p photocraft-tablet -p photocraft-cli -p photocraft-paint -p photocraft-io -p photocraft-gpu`, in test mode, without `--features` (the test features come from the dev-dependencies). Each package brings features the others lack, so a command that drops one builds a second copy of egui, photocraft-io or craft-affinity and everything above them. The craft-libs workspace is unchanged.

| to | run | what it does |
| --- | --- | --- |
| test one crate | `cl test <crate> [-- <filters>]` | builds `--lib --tests` over the 11 packages, then runs only that crate's test binaries |
| test a ui-egui change fast | `cl test ui-egui --unit [-- <filters>]` | builds and runs the lib binaries only |
| the integration round | `cl test all [-- <filters>]` | builds all, runs every binary of the 11 packages three at a time, longest first |
| check | `cl check <crate>` | `cargo check --lib --tests` over the same 11 packages (the selection is fixed, so every package is checked) |
| the snapshot example | `cl build snapshot` | `cargo test --no-run --example snapshot` over the same selection |
| the packages and builds | `cl doctor` | the 11 packages exist; the target holds one build of each pinned crate with the expected features |

`<crate>` is the package name with or without `photocraft-` (`ui-egui`, `engine`, `photocraft` for the app). `--no-run` builds only.

The environment is the same for every command: `TMPDIR=/tmp` (PhotoCraft's tests keep a scratch reserve that a larger default TMPDIR can trip); `RUST_MIN_STACK`, `CARGO_INCREMENTAL` and `CRAFT_FONTS_DIR` unset (the text crate's build script tracks the fonts directory, so any change rebuilds it and everything above). In a PhotoCraft workspace every cargo command runs through the command in `CL_PC_LAUNCHER` when it is set, and each test binary through the command in `CL_PC_RUNNER` when it is set, with `--test-threads 4`; they play the roles `CL_LAUNCHER` and `CL_RUNNER` play in craft-libs, which do not apply there. Without them cargo and the binaries run directly.

`cl test all` orders the binaries by the times of the previous run (`<target>/cl-test-times.json`), else ui-egui's lib, ui-egui's `it`, then engine first. Without filters, the photocraft-ui-egui lib binary runs as three shards chosen by top-level module from the binary's `--list` (a module over a third of the tests is split by its submodules); each test runs in exactly one shard. The last line is `N passed, M failed, B binaries, K overflowed stacks, T s`; failures follow as `binary test_name`.

Refused in a PhotoCraft workspace, with the reason: one `-p <crate>` (or none) through `cl cargo`, `--features`/`-F`/`--no-default-features`, `--all-features`, `--workspace`, and a `cargo test` without `--lib`, `--tests` or `--example`. After every command, a new build of egui, eframe, photocraft-ui-egui, photocraft-io, craft-affinity or a framework crate is named with its feature set. `CL_DRY_RUN=1` prints the composed cargo command and the environment and runs nothing.

## What not to run locally

- `--all-features`: enables `snapshot`, `system_fonts`, `color_fonts`, text's `all-languages` and more, a set no loop uses; it rebuilt 206 dependency crates and every framework crate once. CI runs it.
- `--bins` on the gallery: three 1 GB executables nobody opens locally.
- Workspace-wide builds and tests (`--workspace`): 43 minutes for the test build alone. Locally, build the crate you changed and run its trials; `workspace.yml` runs the workspace-wide checks in CI.

What CI covers (.github/workflows). Pushes to `main` and `next` and every pull request run the workflows whose paths changed; on `next` the Linux jobs run:

- `workspace.yml` runs `cargo fmt --check`, clippy and tests for the whole workspace, with and without `--all-features`, on every push and pull request (Linux only on `next`; Linux, Windows and macOS elsewhere). This is the full gate.
- `ui.yml` is the framework's fast gate: clippy and tests of the seven ui crates (`craft-ui` and the six it re-exports) with and without `--all-features`, their docs, and a wasm check of the facade. It runs on changes under those crates, `craft-ui-macros` and `craft-i18n`.
- `theme.yml` covers craft-theme and craft-theme-store, and checks that craft-ui's library still builds.
- `docs.yml` builds the documentation of the workspace.
- `formats.yml` covers the shared format crates; `video.yml` covers craft-video.
- Leaf crates with their own path-filtered workflow: ai-core, atomic, audio, cms, crw, dialogs, doc, docpath, ecproj-format, font-store, i18n, jpeg, keymap, media, menu, pcraft-format, pdf, print, proof, update, vectorcraft-format, web-api, web-host, window.
- `house-rules.yml` runs the organisation's shared house-rules check.

A crate without its own workflow (craft-behavior, craft-host, the domain crates that use the framework, the gallery, the story renderer and the benchmark) is tested only by `workspace.yml` in CI. Locally, check it with its own `cl test`.

## Profiles

- Dependencies build at `opt-level = 2` in dev and test builds, once per target directory, so screen tests and the debug gallery run at a usable speed.
- `lto = "off"` in dev: with cargo's default (`lto = false`) rustc runs thin-local LTO across every optimised crate's codegen units on each rebuild; off, a one-line edit re-optimises only the changed units (widgets' one-line rebuild went from 41 s to 4 s).
- craft-theme, craft-behavior and craft-proof build at `opt-level = 1`: their code runs in every screen-test frame (colour maths, behaviour cores) or in dictionary-sized tests, and they are rarely edited. Every other workspace crate, the runtime (craft-ui-contract) included, builds at 0, where rustc shares generic instantiations across crates and an edit rebuilds in seconds (the runtime's 65k lines cost 35–90 s of codegen per edit at opt-level 1).
- The `gallery` profile (the native gallery): release optimisation, incremental, 256 codegen units, and the workspace's own crates at `opt-level = 1`, which keeps rustc sharing generics across crates so a framework edit does not re-generate every dependant's copies; dependencies keep `opt-level = 3`.

## Measuring frame time

Frame-time numbers come from the gallery profile, the build a developer runs on the target machine: `cl cargo build --profile gallery -p craft-ui-gallery --bin craft-ui-gallery` (the same command as `cargo run --profile gallery -p craft-ui-gallery`, with nothing added). Build it in a worktree kept for measuring only, so its gallery profile stays warm and a rebuild after a change compiles only the crates the change reaches. Build the old gallery with the same settings (for a tree without the gallery profile, release with the gallery profile's values passed as `--config profile.release...` overrides), so a ratio compares the code and not the optimisation. Measure the old and the new gallery at the same time, on two virtual displays, in every round: load on a shared machine moves both by more than one fix does, and the same load cancels in the comparison. What a frame runs (passes per frame, measuring runs, runs per interaction) is asserted by count trials, not counted in the measured build.

## The job pool

cargo gives each build a jobserver with one token per core, so a build alone uses the whole machine, but several builds at once (worktrees, agents) each take every core and slow each other down. To share one job pool between them, set `CL_LAUNCHER` to a command that runs its arguments with a shared jobserver in `MAKEFLAGS` (cargo joins a jobserver it finds there). `cl` runs every cargo command through that command; without `CL_LAUNCHER` it runs cargo directly.

Test binaries take no tokens: each starts its own test threads, and screen tests a software renderer's threads as well. `cl test` therefore builds with `cargo test --no-run` through `CL_LAUNCHER` and then runs each test binary itself, as cargo would (from its package's directory, the lib's unit tests first, stopping at the first that fails), through `CL_RUNNER` when it is set: a command that limits how many test binaries run at once. `cl render` runs the story renderer through `CL_RUNNER` too. Keep the two apart: a limit held for a whole build only queues the builds behind it, and the job pool already shares the cores between builds.

## Build records

Every build `cl` composes passes cargo's `--timings`: a report per build in `<target>/cargo-timings/` (each unit's start and duration, whether it was fresh, and how many units waited for a job token; the newest 200 are kept). Every `cl` command also appends one line to `<target>/cl.log`: when it started, the command, the wall time of its build and of its test runs, the exit code and the build's report. A slow loop is read from these, not timed again: a build much longer than its report's units points at a wait before cargo started, a run much longer than the tests' own `finished in` at a wait for `CL_RUNNER`.

## A new worktree's first build

A new target directory builds the dependencies once (about 240 crates, 2–3 minutes with 16 cores) and then the framework for the first crate you work on (contract, widgets, panels, canvas, patterns, shell: another 1–2 minutes). After that, a one-line edit in a framework crate rebuilds that crate (seconds) and its test binary; a domain crate's test binary after a framework edit takes about 25 s (the chain above the edit at opt-level 0).
