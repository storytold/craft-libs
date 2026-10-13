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

- [`craft-atomic`](crates/atomic/README.md): Crash-safe atomic file writing and replacement with same-directory staging, directory fsync, Apple `F_FULLFSYNC` fallback, transient lock backoff, and fault injection for the Craft suite.
- [`craft-cms`](crates/cms/README.md): pure-Rust ICC v2/v4 colour management: profile parsing
  and writing, transforms, rendering intents, black point compensation, soft proofing, gamut
  checks, 3D LUTs and built-in profiles (sRGB, Display P3, synthetic coated CMYK and others).
- [`craft-print`](crates/print/README.md): PDF submission to CUPS and Windows, printer discovery,
  duplex/copies/colour and driver preferences. PDF generation stays in each app.

- [`craft-quit`](crates/quit/src/lib.rs): native macOS quit, egui close handling and an optional
  app menu. Unsaved-document decisions and save prompts stay in each app.

- [`craft-settings`](crates/settings/README.md): `config.toml` settings storage for every app: platform folders, portable mode and overrides, atomic saves while running, backup, damaged-file recovery and schema migrations. Policy in [docs/settings-storage.md](docs/settings-storage.md).
- [`craft-ui`](crates/ui/README.md): SVG catalogs, icon buttons, tabs, pane controls, transactional docking, print settings and theme helpers for egui frontends;
  assets, themes, localization and command handling stay in each app. Requires Rust 1.95 or later.
- [Shared UI component audit](docs/ui-components-audit.md): source inventory, extraction priorities
  and compatibility boundaries across the Craft suite.
- [Theme infrastructure](docs/theme-infrastructure.md): eight-app comparison and the small shared
  widget-state styling boundary.
- [Panel docking](docs/ui-docking.md): validated panel moves, floating groups, workspace adapters
  and an optional egui docking surface.
- [Ecosystem architecture blueprint](docs/ecosystem-architecture.md): 13-application architecture breakdown,
  universal four-tier topology, engine mechanics, and three-tier promotion model.
- [Shared infrastructure specification](docs/shared-infrastructure.md): 18-crate shared architecture specification,
  dependency topology, and concrete interface contracts.
- [Creative document compatibility reference](docs/document-compatibility.md): cross-application creative
  document compatibility, structural preservation, and placement boundaries across 13 applications.
- [Shared media infrastructure specification](docs/media-infrastructure.md): bitstream models, audio DSP
  filter cascades, and ISOBMFF faststart progressive container layout.
- [Crate inventory audit](docs/crate-audit.md): comprehensive 100+ crate audit across all 13 applications
  and extraction candidate catalog.
- [Migration roadmap](docs/migration-plan.md): step-by-step phased crate promotion and ecosystem dependency
  alignment schedule.
- [Downstream consumer adoption guide](docs/downstream-adoption.md): integration playbooks, media export,
  vector metafile sharing, and v0.1.0 release pin migration.
- [Engineering methodology](docs/engineering-methodology.md): clean-room reconstruction lifecycle, headless
  command introspection, and platform fallbacks.
- [Testing and evaluation specification](docs/testing-and-evaluation.md): three-level quality gate hierarchy,
  hostile fuzzing, and empirical verification.

## License and credits

craft-libs is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the craft-libs contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ATTRIBUTION.md](ATTRIBUTION.md).
