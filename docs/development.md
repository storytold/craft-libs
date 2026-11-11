# Development workflow

How to build, test and look at this workspace without waiting for it. The rules here exist because a cargo workspace of 62 crates, with a framework that every domain crate links, rebuilds far more than the change touched whenever two commands disagree about features or profiles.

## The commands

`tools/cl` is the entry point for every local cargo command (put it on your PATH, or run `tools/cl` from the worktree):

| What | Command | Builds |
|---|---|---|
| check the crate you changed | `cl check <crate>` | `cargo check -p <crate> --tests` with the fixed feature set |
| build its tests | `cl test <crate> --no-run` | `cargo test -p <crate> --tests` with the fixed feature set |
| run some trials | `cl test <crate> -- <filter> --test-threads 4` | nothing new when warm |
| the debug gallery | `cl build gallery` | the `craft-ui-gallery` binary only |
| one component's stories as PNGs | `cl render <crate\|ui> <Component> [--story <name>] [--out <dir>]` | the `story-render` binary with that crate's feature |
| anything else | `cl cargo <args>` | what you asked for, through the job pool |
| the fixed feature set of a crate | `cl features <crate>` | nothing |

`<crate>` is the package name with or without `craft-` (`color`, `craft-ui-widgets`). `cl` uses `<worktree>-target` as the target directory when that directory exists beside the worktree, else `CARGO_TARGET_DIR`, else `target/`.

### The one-minute component loop

A fix to one gallery component, from the edit to its old/new picture on a page: edit the component's crate, then `cl render <crate> <Component>` (about 10–25 s warm: the changed crate, the renderer's link, the render) and compare the PNGs with the previous ones; the box's comparison page does the compare step (`gal-sheet/pair.py`, box only). `story-render` links the framework and only the domain crates its features name, so a change in craft-color rebuilds craft-color and the renderer, not the gallery and its 17 crates. `cl render ui <Component>` renders the framework's own components.

## One fixed feature set

cargo compiles a crate once per distinct feature set it is asked for, and every crate above it once more. Before `cl`, local commands differed in craft-ui's `app` (the gallery), `snapshot` (the sheet), `testing` (the tests) and the domain crates' `screen-testing`, so a target directory held two to five copies of the framework and rebuilt it between one agent's test, gallery and sheet commands. `cl` adds the same features to every command: the crate's test feature (`screen-testing`, else `testing`) plus craft-ui's `app`, `testing` and `snapshot` (for the framework crates themselves: craft-ui-contract's `testing` and `snapshot`, craft-ui-shell's `app`). The proc-macro crate pins syn's and proc-macro2's features to everything other derive crates may add, so the one syn every proc macro shares is the same in every build; craft-ui-contract depends on craft-window with `ui`, so the shell's `app` changes nothing below the shell; and dev-dependency lines ask for no egui features the test kit does not need.

What still differs, by design: a framework crate's own tests have no eframe in their graph, domain crates do; two domain crates can differ in a few of their own dependencies' features; the gallery with every app enables a few more features below the framework. Each of those is one extra framework build in a target directory, once; `cl` names the crate and the feature set whenever it happens.

## What not to run locally

- `--all-features`: enables `snapshot`, `system_fonts`, `color_fonts`, text's `all-languages` and more, a set no loop uses; it rebuilt 206 dependency crates and every framework crate once. CI runs it.
- `--bins` on the gallery: three 1 GB executables nobody opens locally.
- Workspace-wide builds and tests (`--workspace`, `cargo xtask ci`): 43 minutes for the test build alone. The GitHub workflows run the full matrix on every push; locally, build the crate you changed and run its trials.

## Profiles

- Dependencies build at `opt-level = 2` in dev and test builds, once per target directory, so screen tests and the debug gallery run at a usable speed.
- `lto = "off"` in dev: with cargo's default (`lto = false`) rustc runs thin-local LTO across every optimised crate's codegen units on each rebuild; off, a one-line edit re-optimises only the changed units (widgets' one-line rebuild went from 41 s to 4 s).
- craft-ui-contract, craft-theme, craft-behavior and craft-proof build at `opt-level = 1`: the runtime is where a screen-test frame spends its time. Everything above the runtime (widgets, panels, canvas, patterns, shell, the domain crates) builds at 0, where rustc shares generic instantiations across crates and an edit rebuilds in seconds.
- The `gallery` profile (the owner's native gallery): release optimisation, incremental, 256 codegen units, and the workspace's own crates at `opt-level = 1`, which keeps rustc sharing generics across crates so a framework edit does not re-generate every dependant's copies; dependencies keep `opt-level = 3`.

## The job pool

On the shared build box every cargo runs through `job-pool` (`~/.local/bin`), which gives all builds one jobserver of 16 tokens through `MAKEFLAGS`: a build alone gets every core, builds together share them and never exceed the count. Elsewhere, plain cargo does the same for one build at a time.

## A new worktree's first build

A new target directory builds the dependencies once (about 240 crates, 2–3 minutes with 16 cores) and then the framework for the first crate you work on (contract, widgets, panels, canvas, patterns, shell: another 1–2 minutes). `seed-target` on the box hardlinks the dependencies from a warm target directory, so only the framework is paid. After that, a one-line edit in a framework crate rebuilds that crate (seconds) and its test binary; a domain crate's test binary after a framework edit takes about 25 s (the chain above the edit at opt-level 0).
