Craft App Libraries
===================

<a href="https://sonarcloud.io/project/overview?id=storytold_craft-libs"><img alt="Scanned by SonarCloud" src="https://img.shields.io/badge/Scanned_by-SonarCloud-4b9fd5?style=flat-square&logo=sonarqubecloud&logoColor=white"></a>

This repo contains libraries used by the various Craft Apps. It's a Rust monorepo workspace, and the libraries can be included by any Craft Apps that need them.

In the future this may grow to include non-Rust library code.

If any libraries grow beyond Craft App usage, eg. RAW format support, they may be promoted out and into their own libraries for the broader ecosystem to take advantage of.


## Layout

A Cargo workspace (edition 2024, resolver 3). Each library is a crate under `crates/<name>`, named
`craft-<name>`, with `version`, `edition`, `license` and `repository` inherited from the workspace
(`license.workspace = true`, and so on) and `[lints] workspace = true`. Register each crate in
`[workspace.dependencies]` in the root `Cargo.toml`.

## Use a released version

The first coordinated release is **[v0.1.0](https://github.com/storytold/craft-libs/releases/tag/v0.1.0)**.
It gives consumers a fixed baseline while development continues on `main`. All five crates
share this version. They require Rust 1.95 or later; the UI integrations use egui 0.36.

Add only the crates you need, using the same tag for every craft-libs dependency:

```toml
[dependencies]
craft-ui = { git = "https://github.com/storytold/craft-libs", tag = "v0.1.0" }
craft-settings = { git = "https://github.com/storytold/craft-libs", tag = "v0.1.0" }
# Optional, for egui close handling and the macOS application menu:
craft-quit = { git = "https://github.com/storytold/craft-libs", tag = "v0.1.0", features = ["menu"] }
```

`craft-atomic` and `craft-print` use the same dependency syntax. Enable `craft-ui`'s
`serde` feature if you need to serialize its docking layout. These are Git releases;
crates.io publication is deferred.

Release tags are never moved or reused. For a commit pin, resolve the tag with
`git ls-remote https://github.com/storytold/craft-libs 'refs/tags/v0.1.0^{}'` and replace
`tag = "v0.1.0"` with `rev = "<full commit SHA>"` on every craft-libs dependency.
Use either `tag` or `rev`, not both. An existing commit pin keeps working; switching to this
release may require adapting to changes since that commit.

Commit your application's `Cargo.lock` and build with `cargo build --locked` for reproducible
Rust dependency resolution. A Git pin fixes craft-libs source, not every transitive dependency.
A library's own lockfile does not constrain its downstream consumers. Upgrade deliberately:
read the release notes, update all craft-libs pins together, refresh the lockfile with
`cargo check`, and run your application's tests and affected workflows before committing.

See the [release notes](CHANGELOG.md) for included functionality and known limits, and the
[versioning policy](docs/versioning.md) for compatibility guarantees and release checks.

## Libraries

- [`craft-platform`](crates/platform/README.md): optional native dialog lifecycle, Linux/BSD desktop appearance observation and browser download ownership; application policy stays in the apps.

- [`craft-ai-core`](crates/ai-core/README.md): the data model and contract for AI in the Craft Apps (connections, models, tools, requests, jobs, consent, history, route spec). Pure, no I/O, wasm-safe. Contract in [docs/ai-shared-layer.md](docs/ai-shared-layer.md).
- [`craft-pdf`](crates/pdf/README.md): independently optional PDF parsing, interpretation, rasterization and guarded authoring. Apps retain layout, fonts, editing and export policy; writer-only consumers do not enable the shared reader.

- [`craft-atomic`](crates/atomic/README.md): Crash-safe atomic file writing and replacement with same-directory staging, directory fsync, Apple `F_FULLFSYNC` fallback, transient lock backoff, and fault injection for the Craft suite.
- [`craft-cms`](crates/cms/README.md): pure-Rust ICC v2/v4 colour management: profile parsing
  and writing, transforms, rendering intents, black point compensation, soft proofing, gamut
  checks, 3D LUTs and built-in profiles (sRGB, Display P3, synthetic coated CMYK and others).
- [`craft-docpath`](crates/docpath/src/lib.rs): decides whether a path or `file:` URL stored in a
  document may be opened. Paths stored in documents are followed only when they are local or on the
  document's own network share; other network paths, device paths and non-file URLs are refused.
- [`craft-fonts-build`](crates/fonts-build/README.md): embeds the optional craft-fonts faces
  from an app's `build.rs` (`CRAFT_FONTS_DIR`, `CRAFT_FONTS_REQUIRED`, script filters) as a table
  of `EmbeddedFace`s. No dependencies; the app picks the faces.
- [`craft-i18n`](crates/i18n/README.md): translations over the embedded craft-i18n catalogs:
  lookup with fallback (`pt-br` → `pt` → English), plurals, language names, the system locale and
  a pseudo locale. The catalogs stay a fully optional build input; apps keep `tl!` and their
  language setting.
- [`craft-keymap`](crates/keymap/README.md): keyboard bindings: chords and their text forms,
  scopes, conflicts, user overrides stored through craft-settings and import of other
  applications' shortcut sets. Command catalogs, file readers and the shortcut editor UI stay
  in each app.
- [`craft-media`](crates/media/README.md): bit-level I/O (`bitstream`), clean-room MP4/MOV
  (`isobmff`) and Matroska/WebM (`matroska`) demuxers and muxers, from FilmCraft, and an ISO BMFF
  box reader. Containers only; codecs stay in each app.
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

## License and credits

craft-libs is dual-licensed under [MIT](LICENSE-MIT) or [Apache-2.0](LICENSE-APACHE), at your option.
Copyright (c) 2026 ArtCraft Team and the craft-libs contributors. Required notices are in [NOTICE](NOTICE).

Bundled fonts, icons, images and other assets keep their own open licenses; each one is listed
with its author, source and license in [ATTRIBUTION.md](ATTRIBUTION.md).
