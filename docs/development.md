# Development workflow

How to build, test and look at this workspace without waiting for it. The rules here exist because a cargo workspace of 62 crates, with a framework that every domain crate links, rebuilds far more than the change touched whenever two commands disagree about features or profiles.

## The commands

`tools/cl` is the entry point for every local cargo command (put it on your PATH, or run `tools/cl` from the worktree):

| What | Command | Builds |
|---|---|---|
| check the crate you changed | `cl check <crate>` | `cargo check` of the crate's lib, bins and tests |
| build its tests | `cl test <crate> --no-run` | `cargo test -p <crate> --tests` |
| run some trials | `cl test <crate> -- <filter> --test-threads 4` | nothing new when warm |
| the debug gallery | `cl build gallery` | the `craft-ui-gallery` binary only |
| one component's stories as PNGs | `cl render <crate\|ui> <Component> [--story <name>] [--out <dir>]` | the `story-render` binary with that crate's feature |
| anything else | `cl cargo <args>` | what you asked for, through the job pool, nothing added |
| the features `cl` adds for a crate | `cl features <crate>` | nothing |

`<crate>` is the package name with or without `craft-` (`color`, `craft-ui-widgets`). `cl` uses `<worktree>-target` as the target directory when that directory exists beside the worktree, else `CARGO_TARGET_DIR`, else `target/`.

### The one-minute component loop

A fix to one gallery component, from the edit to its old/new picture on a page: edit the component's crate, then `cl render <crate> <Component>` (about 10–25 s warm: the changed crate, the renderer's link, the render) and compare the PNGs with the previous ones; the box's comparison page does the compare step (`gal-sheet/pair.py`, box only). `story-render` links the framework and only the domain crates its features name, so a change in craft-color rebuilds craft-color and the renderer, not the gallery and its 17 crates. `cl render ui <Component>` renders the framework's own components.

## One feature resolution

cargo compiles a crate once per distinct feature set, and every crate above it once more, and it unifies features only over the packages a command selects. A framework crate's tests have no eframe in their graph; a domain crate's tests do; the gallery adds every domain crate, whose dependencies turn on more features of egui, wgpu, craft-window, image and others below the framework. Each kind of command would build its own copy of the framework, and wgpu with it.

`cl` selects the `feature-root` package beside the package of every command (`-p <crate> -p feature-root`). Its dependencies are the gallery's, with every domain crate's `screen-testing`, so every local build resolves features as the gallery does, and a target directory holds one build of each crate. Nothing of `feature-root` is compiled: its one target is a bin no `cl` command selects (`cl` names the crate's own targets), and only the targets a command names are built, so a render still links one domain crate and a framework crate's tests still link no app. On top, `cl` adds the crate's own test feature (`screen-testing`, else `testing`). The Mac's and CI's commands never select `feature-root` and are unaffected.

Four manifest facts keep this true, and `cl doctor` checks them: `feature-root` lists every optional domain crate of the gallery with the gallery's features, and depends on the gallery without its default features (so the gallery lib's own features stay each command's: `apps` for the gallery, one crate for a render); the story renderer asks its shared dependencies for no feature the gallery's lines lack; dev-dependency lines ask egui for no features (a feature there differs between a crate's tests and everything else); the proc-macro crate pins syn's and proc-macro2's features to everything other derive crates may add, so the one syn every proc macro shares is the same with or without design-lint in the graph. `cl` names any new build of a framework crate in the target directory after each command; after a manifest change one is expected, otherwise run `cl doctor`.

## What not to run locally

- `--all-features`: enables `snapshot`, `system_fonts`, `color_fonts`, text's `all-languages` and more, a set no loop uses; it rebuilt 206 dependency crates and every framework crate once. CI runs it.
- `--bins` on the gallery: three 1 GB executables nobody opens locally.
- Workspace-wide builds and tests (`--workspace`, `cargo xtask ci`): 43 minutes for the test build alone. Locally, build the crate you changed and run its trials; the GitHub workflows cover the rest of what they cover.

What CI covers today (.github/workflows):

- `ui.yml` is the framework's gate: clippy and tests of the seven ui crates (`craft-ui` and the six it re-exports) with and without `--all-features`, their docs, and a wasm check of the facade. It runs on changes under those crates, `craft-ui-macros` and `craft-i18n`. A change to `craft-behavior` or `craft-host` alone does not run it.
- `theme.yml` covers craft-theme and craft-theme-store, and checks that craft-ui's library still builds.
- `docs.yml` builds the documentation.
- Some leaf crates have their own path-filtered workflow: ai-core, atomic, audio, crw, dialogs, doc, docpath, ecproj-format, font-store, i18n, jpeg, keymap, media, pcraft-format, pdf, print, proof, update, vectorcraft-format, web-api, web-host, window.

No workflow runs the tests of: craft-behavior, craft-host and craft-ui-macros; the domain crates that use the framework (automation, browse, color, draw, font-manager, imaging, layers, menu, pages, review, table, text, timeline); cms, font-catalog, fonts-build, platform and quit; the gallery, the story renderer, the benchmark, the test kit, craft-design-lint, craft-win32 and the `craft` facade. Nothing runs the full workspace build or test. A change in one of those crates is checked by its own `cl test` only.

## Profiles

- Dependencies build at `opt-level = 2` in dev and test builds, once per target directory, so screen tests and the debug gallery run at a usable speed.
- `lto = "off"` in dev: with cargo's default (`lto = false`) rustc runs thin-local LTO across every optimised crate's codegen units on each rebuild; off, a one-line edit re-optimises only the changed units (widgets' one-line rebuild went from 41 s to 4 s).
- craft-theme, craft-behavior and craft-proof build at `opt-level = 1`: their code runs in every screen-test frame (colour maths, behaviour cores) or in dictionary-sized tests, and they are rarely edited. Every other workspace crate, the runtime (craft-ui-contract) included, builds at 0, where rustc shares generic instantiations across crates and an edit rebuilds in seconds (the runtime's 65k lines cost 35–90 s of codegen per edit at opt-level 1).
- The `gallery` profile (the owner's native gallery): release optimisation, incremental, 256 codegen units, and the workspace's own crates at `opt-level = 1`, which keeps rustc sharing generics across crates so a framework edit does not re-generate every dependant's copies; dependencies keep `opt-level = 3`.

## Measuring frame time

Frame-time numbers come from the gallery profile, the build the owner runs: `cl cargo build --profile gallery -p craft-ui-gallery --bin craft-ui-gallery` (the same command as `cargo run --profile gallery -p craft-ui-gallery`, with nothing added). Build it in a worktree kept for measuring only, so its gallery profile stays warm and a rebuild after a change compiles only the crates the change reaches. Build the old gallery with the same settings (for a tree without the gallery profile, release with the gallery profile's values passed as `--config profile.release...` overrides), so a ratio compares the code and not the optimisation. Measure the old and the new gallery at the same time, on two virtual displays, in every round: load on a shared machine moves both by more than one fix does, and the same load cancels in the comparison. What a frame runs (passes per frame, measuring runs, runs per interaction) is asserted by count trials, not counted in the measured build.

## The job pool

cargo gives each build a jobserver with one token per core, so a build alone uses the whole machine, but several builds at once (worktrees, agents) each take every core and slow each other down. To share one job pool between them, set `CL_LAUNCHER` to a command that runs its arguments with a shared jobserver in `MAKEFLAGS` (cargo joins a jobserver it finds there). `cl` runs every cargo command through that command; without `CL_LAUNCHER` it runs cargo directly.

## A new worktree's first build

A new target directory builds the dependencies once (about 240 crates, 2–3 minutes with 16 cores) and then the framework for the first crate you work on (contract, widgets, panels, canvas, patterns, shell: another 1–2 minutes). After that, a one-line edit in a framework crate rebuilds that crate (seconds) and its test binary; a domain crate's test binary after a framework edit takes about 25 s (the chain above the edit at opt-level 0).
