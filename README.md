Craft App Libraries
===================

This repo contains libraries used by the various Craft Apps. It's a Rust monorepo workspace, and the libraries can be included by any Craft Apps that need them.

In the future this may grow to include non-Rust library code.

If any libraries grow beyond Craft App usage, eg. RAW format support, they may be promoted out and into their own libraries for the broader ecosystem to take advantage of.


## Layout

A Cargo workspace (edition 2024, resolver 3). Each library is a crate under `crates/<name>`, named
`craft-<name>`, with `version`, `edition`, `license` and `repository` inherited from the workspace
(`license.workspace = true`, and so on) and `[lints] workspace = true`. Register each crate in
`[workspace.dependencies]` in the root `Cargo.toml`.

Apps depend on a crate by git:

```toml
craft-<name> = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Libraries

- [`craft-ui`](crates/ui/README.md): SVG catalogs, icon buttons, tabs, pane controls, transactional docking and theme helpers for egui frontends;
  assets, themes, localization and command handling stay in each app. Requires Rust 1.95 or later.
- [Shared UI component audit](docs/ui-components-audit.md): source inventory, extraction priorities
  and compatibility boundaries across the Craft suite.
- [Theme infrastructure](docs/theme-infrastructure.md): eight-app comparison and the small shared
  widget-state styling boundary.
- [Panel docking](docs/ui-docking.md): validated panel moves, floating groups, workspace adapters
  and an optional egui docking surface.

## Architecture & Standards

- [Architectural RFC (`RFC-CRAFT-LIBS-001`)](docs/rfc-shared-capabilities-architecture.md): The intended role of `craft-libs`, capability acceptance protocol, differential testing, and evidence standards.
- [Shared Crate Contract (`CRAFT-CONTRACT-001`)](docs/shared-crate-contract.md): Mandatory invariants, failure semantics, post-failure state guarantees, and cross-platform behavior specifications.
- [Consumer Dependency Map](docs/consumer-dependency-map.md) ([JSON](docs/consumer-dependency-map.json)): Inventory of the 10 Craft applications, shared crate adoptions, pinned revisions, and selective release gating.
- [Platform Test Inventory](docs/platform-test-inventory.md) ([JSON](docs/platform-test-inventory.json)): Hardware profiles, OS versions, and verified behaviors (including Touch Bar AppKit lifecycle and APFS durability).

## License and credits

craft-libs is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the craft-libs contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ATTRIBUTION.md](ATTRIBUTION.md).
