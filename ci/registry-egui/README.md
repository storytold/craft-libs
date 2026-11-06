# Registry-egui smoke workspace

Isolated workspace (its own `[workspace]`, not a member of the root one). Nothing here
patches crates.io, so egui resolves to **stock registry 0.37.0** — the baseline the suite
fork is derived from (`docs/egui-037-migration-plan.md` §5.2, CL-4). Consumers that want
the suite fork patch it in their own `[patch.crates-io]`; the published `craft-ui`
manifest must never carry a machine-local path, a branch pin or a hidden winit source.

## Running the gates

```sh
./ci/registry-egui/check.sh
```

1. `craft-ui` with its default features, added by path without any patch, passes clippy
   and tests against stock egui 0.37.0, and `cargo tree -i egui` shows exactly one egui
   source in the graph, the registry one (no `git+`). The fork-only
   `Style::set_right_to_left` call in `craft_ui::direction::apply` resolves to the no-op
   trait shim here (`crates/ui/src/direction/mod.rs`).
2. `cargo package -p craft-ui` smoke: the internal crates are unpublished and `cargo
   package`'s upload-prepare resolve ignores `[patch]`, so the script stages a minimal
   workspace, vendors the graph into a directory source (adding `craft-i18n` and
   `craft-fonts-build` by hand) and packs `craft-ui` there. It fails if the packaged
   manifest contains a dependency `path =` outside the package, a `git +`, a `branch =`,
   an absolute home path, or a `winit` dependency.
3. `cargo check -p craft-ui --all-features --locked --target wasm32-unknown-unknown` in the
   root workspace (the same command CI's wasm job in `.github/workflows/ui.yml` runs).
   Compile-only: browser IME and input stay on the manual release list
   (`docs/language-testing.md`).

`Cargo.lock` here is committed and used with `--locked`.

## Fork-only boundary

`crates/ui/src/ime` (the canvas IME `Composer`) is built on the fork-only
`egui::ime_context` module and `IMEOutput` fields, so it sits behind craft-ui's
non-default `ime` feature; `native-windows` implies it. A consumer on stock egui simply
does not enable it. Everything else in the crate, including all of `direction` and
`combo`, compiles against stock 0.37.0 — the inherent-vs-trait shim in
`crates/ui/src/direction/mod.rs` covers the fork-only setter, and direction reads go
through `craft_ui::direction` (the `Context` temp set by `direction::apply`), never
through the fork-only style field.
