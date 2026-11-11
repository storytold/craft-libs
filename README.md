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

Apps depend on a crate by git:

```toml
craft-<name> = { git = "https://github.com/storytold/craft-libs", rev = "<commit>" }
```

## Libraries

- [`craft-platform`](crates/platform/README.md): optional native dialog lifecycle, Linux/BSD desktop appearance observation and browser download ownership; application policy stays in the apps.

- [`craft-ai-core`](crates/ai-core/README.md): the data model and contract for AI in the Craft Apps (connections, models, tools, requests, jobs, consent, history, route spec). Pure, no I/O, wasm-safe. Contract in [docs/ai-shared-layer.md](docs/ai-shared-layer.md).
- [`craft-pdf`](crates/pdf/README.md): independently optional PDF parsing, interpretation, rasterization and guarded authoring. Apps retain layout, fonts, editing and export policy; writer-only consumers do not enable the shared reader.

- [`craft-atomic`](crates/atomic/README.md): Crash-safe atomic file writing and replacement with same-directory staging, directory fsync, Apple `F_FULLFSYNC` fallback, transient lock backoff, and fault injection for the Craft suite.
- [`craft-cfb`](crates/cfb/README.md): clean-room Compound File Binary ([MS-CFB], OLE structured
  storage) reader and writer: the container of legacy Office files, Outlook messages, encrypted
  OOXML packages and AAF. Bounded on hostile input; what the streams hold stays with each format.
- [`craft-cms`](crates/cms/README.md): pure-Rust ICC v2/v4 colour management: profile parsing
  and writing, transforms, rendering intents, black point compensation, soft proofing, gamut
  checks, 3D LUTs and built-in profiles (sRGB, Display P3, synthetic coated CMYK and others).
- [`craft-crw`](crates/crw/README.md): clean-room Canon CRW (CIFF) raw decoder, from LightCraft:
  the heap container, the entropy coding derived by black-box analysis (frozen code tables), sensor
  black and white levels and as-shot white balance. The complete derivation record (plan, sources
  log, scripts, verification) is in `crates/crw/clean-room/`. Colour stays in each app.
- [`craft-dialogs`](crates/dialogs/README.md): native file and message dialogs that never block
  the event loop: queued requests shown at the end of the frame and answered on a later frame,
  free-standing Open and Folder panels and Save sheets on macOS, case-insensitive filters. What
  to do with the answer stays in each app.
- [`craft-doc`](crates/doc/README.md): reads PhotoCraft `.pcraft`, VectorCraft `.vectorcraft`
  and EffectCraft `.ecproj` documents without the apps: the kind and version, the previews and
  PDF they carry, and their pages and layers. Rendering and editing stay in each app.
- [`craft-docpath`](crates/docpath/src/lib.rs): decides whether a path or `file:` URL stored in a
  document may be opened. Paths stored in documents are followed only when they are local or on the
  document's own network share; other network paths, device paths and non-file URLs are refused.
- [`craft-dxf`](crates/dxf/src/lib.rs): DXF tag reader and writer, from CADCraft: ASCII and
  binary DXF as (group code, value) tags, split into sections and entities, with pre-2007 code
  pages and version detection, and written back. Bounded on hostile input. Mapping tags to a
  drawing stays in each app.
- [`craft-ecproj-format`](crates/ecproj-format/README.md): the envelope of EffectCraft `.ecproj`
  project files: `savedBy`, the schema version and its check, and the file object with the
  project and its saved view as type parameters. The project model stays in EffectCraft.
- [`craft-font-catalog`](crates/font-catalog/README.md): the installed fonts on every platform
  (font folders, fontconfig and Flatpak host folders, CoreText, DirectWrite, the font store),
  lookup by family, style, localized, legacy, full and PostScript names, family loads shared
  between threads, and rescans when fonts are installed or removed.
- [`craft-font-manager`](crates/font-manager/README.md): Craft Font Manager as a library: search
  and filters over the Google Fonts index, installed fonts by family, the font store interface,
  and (`egui` feature) `FontsPanel`, the whole interface as one panel for the launcher's Fonts tab
  and the standalone app.
- [`craft-font-store`](crates/font-store/README.md): the suite's font store: the pinned Google
  Fonts and Noto indexes plus indexes the user adds (never curated), exact lookup by family and
  style or PostScript name, and installs from each source's URLs (jsDelivr, then raw GitHub)
  checked by size, sha256 and parsing, with the licence, a manifest, and removal. Off until the
  user turns downloading on; agents can't.
- [`craft-fonts-build`](crates/fonts-build/README.md): embeds the optional craft-fonts faces
  from an app's `build.rs` (`CRAFT_FONTS_DIR`, `CRAFT_FONTS_REQUIRED`, script filters) as a table
  of `EmbeddedFace`s. No dependencies; the app picks the faces.
- [`craft-gcode`](crates/gcode/src/lib.rs): Grbl-style G-code writer: polylines in
  millimetres to `G0`/`G1` moves with laser power and feed, for cutting lasers and plotters.
- [`craft-heif`](crates/heif/README.md): optional HEIF/HEIC still-image decoder, from LightCraft:
  heic-rs reads the container and HEVC pictures, and this crate converts to 16-bit RGB to match
  libheif (VUI range and matrix, chroma upsampling, `imir`, ICC on grid tiles). Also orientation,
  alpha, EXIF/XMP and thumbnails. It never panics, and apps put it behind a non-default feature.
- [`craft-i18n`](crates/i18n/README.md): translations over the embedded craft-i18n catalogs:
  lookup with fallback (`pt-br` → `pt` → English), plurals, language names, the system locale and
  a pseudo locale. The catalogs stay a fully optional build input; apps keep `tl!` and their
  language setting.
- [`craft-tiff`](crates/tiff/README.md): TIFF and BigTIFF IFD reader and writer, from LightCraft:
  every field type, the IFD chain, SubIFDs and Exif/GPS/Interop pointer IFDs, maker-note IFDs
  (Canon, Nikon, Sony, Fujifilm, Olympus, Panasonic, Pentax), strip and tile layout. Bounded and
  panic-free on hostile input. Pixel decompression stays with the callers.
- [`craft-metafile`](crates/metafile/README.md): Windows metafiles, from DeckCraft, WordCraft and
  VectorCraft: a WMF, EMF and EMF+ reader into a neutral picture (paths with line styles, pattern
  fills, bitmaps, text, clip regions and clip paths) and, behind `write`, EMF and WMF writers.
  Bounded and panic-free on hostile input. Mapping the picture onto a document stays in each app.
- [`craft-jpeg`](crates/jpeg/README.md): clean-room decoder for 12-bit and arithmetic-coded
  JPEG (T.81 SOF0/SOF1 at 12 bits, SOF9, SOF10), from PhotoCraft, and a frame probe for routing.
  Metadata stays in each app.
- [`craft-keymap`](crates/keymap/README.md): keyboard bindings: chords and their text forms,
  scopes, conflicts, user overrides stored through craft-settings and import of other
  applications' shortcut sets. Command catalogs, file readers and the shortcut editor UI stay
  in each app.
- [`craft-audio`](crates/audio/README.md): audio file I/O and codecs (WAV, AIFF, FLAC, AAC, Opus,
  AC-3/E-AC-3, MP1/MP2), streaming and seeking, and packet decoding for any demuxer. See its
  README for formats and features.
- [`craft-media`](crates/media/README.md): bit-level I/O (`bitstream`), clean-room MP4/MOV
  (`isobmff`) and Matroska/WebM (`matroska`) demuxers and muxers, an Ogg demuxer (`ogg`), from
  FilmCraft, and an ISO BMFF box reader. Containers only; codecs are in `craft-audio`, and
  video codecs will go in a separate crate.
- [`craft-menu`](crates/menu/README.md): an app's menus as a macOS menu bar: the Mac layout
  (app menu, Window, Help, system-key clashes), a platform backend trait, and key equivalents
  fed back to egui as the input egui-winit would have made. Commands and their enabled state
  stay in each app.
- [`craft-mesh`](crates/mesh/README.md): 3D mesh file formats behind one neutral model (meshes
  of polygons with per-corner indices, materials, a node hierarchy): STL, OBJ with MTL, PLY and
  glTF 2.0 / GLB, one feature each. Readers are bounded by `Limits`, never open a file themselves
  and never panic. Carved from EffectCraft, SolidCraft, TextureCraft and MaterialCraft.
- [`craft-mime`](crates/mime/README.md): Internet messages and MIME, read and written: RFC 5322
  header fields, MIME parts and multiparts, encoded-words, RFC 2231 parameters, base64 and
  quoted-printable, character sets, address lists, dates, mbox files and the `multipart/related`
  lookups of MHT web archives. What a message means (body, attachments, flags) stays in each app.
- [`craft-odf`](crates/odf/README.md): OpenDocument packages (ODF 1.3 Part 2), read and written: the ZIP container with its `mimetype` and manifest within limits, password protection (AES-256-CBC with PBKDF2, as LibreOffice writes it) and flat single-file documents. The container only: what the XML inside means belongs to the readers of each document type.
- [`craft-pcraft-format`](crates/pcraft-format/README.md): the envelope of PhotoCraft's `.pcraft`
  bundle: `Manifest<D>` with the document as a type parameter, `TileRef`, the entry paths, the
  manifest decode limits and the migration runner. The document model stays in PhotoCraft.
- [`craft-print`](crates/print/README.md): PDF submission to CUPS and Windows, printer discovery,
  duplex/copies/colour and driver preferences. PDF generation stays in each app.
- [`craft-proof`](crates/proof/README.md): proofing per language (spelling, suggestions, grammar,
  hyphenation, a Hunspell reader, user words, Language packs). The word lists and patterns are
  craft-i18n's `language-data/`, embedded at build time from `CRAFT_I18N_DIR`: English and the
  carry-over locales by default, the set `CRAFT_PROOF_EMBED` names otherwise (`en` for English
  only); other languages come from installed Language packs.

- [`craft-quit`](crates/quit/src/lib.rs): native macOS quit, egui close handling and an optional
  app menu. Unsaved-document decisions and save prompts stay in each app.

- [`craft-settings`](crates/settings/README.md): `config.toml` settings storage for every app: platform folders, portable mode and overrides, atomic saves while running, backup, damaged-file recovery and schema migrations. Policy in [docs/settings-storage.md](docs/settings-storage.md).
- [`craft-silhouette-studio`](crates/silhouette-studio/src/lib.rs): bounded, fail-closed reader for Silhouette Studio `.studio` and `.studio3` documents: outlines, line colours and page size in millimetres. Protected designs are refused.
- [`craft-svg-paths`](crates/svg-paths/src/lib.rs): SVG geometry reader: shapes,
  transforms, `<use>`/`<symbol>` instances and the CSS cascade as Bézier paths in millimetres, each
  tagged by the caller; a CSS colour parser (`color`); and a small SVG writer (`write`: elements with
  escaping, path data, numbers). Not the shared SVG model.
- [`craft-text`](crates/text/README.md): text processing below the composer. Its `hyphen` module finds
  hyphenation points, with a bundled US English dictionary and patterns and optional hypher patterns
  for 47 more languages (one cargo feature each).
- [`craft-ui`](crates/ui/README.md): SVG catalogs, icon buttons, tabs, pane controls, transactional docking, print settings and theme helpers for egui frontends;
  assets, themes, localization and command handling stay in each app. Requires Rust 1.95 or later.
- [`craft-vectorcraft-format`](crates/vectorcraft-format/README.md): the envelope of VectorCraft's
  `.vectorcraft` file: its members, versions, embedded images and the data after the JSON, with the
  document as a type parameter. The document and its migrations stay in VectorCraft.
- [`craft-web-api`](crates/web-api/README.md): the page command API every Craft web build offers its page — one
  `window.<app>` shape (`request`, `execute`, `commands`, `docInspect`, `uiInspect`, `screenshot`) over the app's
  JSON control channel.
- [`craft-web-host`](crates/web-host/README.md): the host messages between a Craft app's web build and the page that hosts it (`?host=parent` or `data-host="page"`): one contract and one bridge for every app.
- [`craft-xml`](crates/xml/README.md): bounded XML for hostile input: an owned element tree with limits on size, depth, nodes and attributes, streaming of repeated elements, namespace handling (as written, by local name, or canonical prefixes by URI), escaping and a writer. No DTDs and no entity expansion.
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
