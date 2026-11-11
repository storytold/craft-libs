# craft-font-manager

Craft Font Manager as a library: every font the suite's apps can use, and the font collections to
add to them: Google Fonts, Noto, and index URLs the user adds. ArtCraft Launcher shows it as its
Fonts tab; the standalone app
([aiongg/craft-font-manager](https://github.com/aiongg/craft-font-manager)) shows it as its window.

| Part | Feature | What it does |
|---|---|---|
| `filter` | always | Search over the index: family names ignoring case and accents, category, language (English name, autonym or id) and script |
| `index` | always | The pinned built-in indexes (Google Fonts and Noto) compiled into [`craft-font-store`](../font-store/README.md) (`bundled`), each family's source (`source_label`: its name and whether it is curated), and the categories |
| `installed` | always | The installed fonts [`craft-font-catalog`](../font-catalog/README.md) sees, grouped by family and source |
| `store` | always | `Installer`, what the interface needs from the font store: the download consent, install, remove, downloads for a preview, the indexes (`indexes`, `fetch_indexes`) and the sources the user added (`user_sources`, `add_user_source`, `remove_user_source`). `SuiteStore` is craft-font-store's store, `FakeStore` a stand-in for tests, `NoStore` one for when the store folder is unknown |
| `FontsPanel` | `ui` | The whole interface as one egui panel |
| `glyphs` | `ui` | `GlyphsPanel`: the character grid of a face with categories, zoom and Recently Used; the app supplies the glyphs through `GlyphSource`. Sizes are the `glyphs.*` tokens |
| `missing` | `ui` | `document_fonts`, `missing_font_warnings` and `MissingFontsDialog`: which fonts a document names are not installed, and a replacement for each |
| `axes` | `ui` | `read_axes`, `read_named_instances` and `AxisSliders`: a variable face's axes as sliders with a Style list of its named instances |
| `picker` | `ui` | `FontPicker`: the family field with its list (stars, recents, class and feature filters, similar fonts, previews) |

The stories (`patterns/font-picker/*`, `patterns/glyphs-panel/panel`, `patterns/missing-fonts-dialog/dialog`, `composites/axis-sliders/sliders`) use the two faces egui ships.

## The panel

The layout follows Adobe Fonts' and Google Fonts' browse pages.

- **Across the top**: Browse and Installed, with their counts, and one search field.
- **Filters** at the reading start, each under its label: category, language (searchable by its
  English name or its own, "Japanese · 日本語"), script, source, and installed or not; each option
  with its count.
- **Family cards**, one per row: the name, style count, category and source on one line with the
  card's state at its end (Install and Preview, Installed, a progress bar while downloading,
  Couldn't install with Retry, "Not curated" for a source the user added), and a large sample in
  the face below. The list is virtual: only the cards on screen are laid out, and their faces load
  as they come into view. The arrow keys step through the cards and Enter opens one.
- **Sample toolbar**: a preset (a sentence in the language filtered by, or in the family's own
  script when it has no Latin; the alphabet; numerals), the sample text, and the size.
- **Details sheet** over the list: every style in its own face with Install or Remove for each,
  Install all and Remove all, the source, designer, licence, scripts and languages, and what a
  preview downloads.
- **Installed**: system and font store fonts as the same cards, filtered by source. Only font
  store families can be removed; system fonts are removed with the system's own tools.
- **Sources** (`sources_ui`, for a host's settings page): the built-in sources, the index URLs the
  user added, marked not curated and flagged when the file at their address changed since it was
  added, adding one, and Update sources.

`FontsPanel` fills the `Ui` it is given and has no window, menu or interface fonts of its own; the
craft-ui's theme gives it the colours and interface fonts. The faces it draws samples in come from
craft-ui's document-face registry, which loads them a few per pass and evicts the ones off screen,
without touching the host's fonts. Right-to-left interfaces mirror through the layout; sample text
keeps its own direction.

The panel follows the `craft_host::Host` installed in the egui context (`Host::builder(..).install(ctx)`):
its strings and numbers are in `host.lang()` each frame (English without a Host, or when the panel
has no catalog for the language), its settings section is `fonts_panel`, and installs, removals and
previews run through `host.jobs()`, one at a time in request order (without a Host, on craft-host's default job service: a thread per runner, inline on wasm32).

```rust
let catalog = Arc::clone(Catalog::global());
catalog.scan_in_background();
let installer: Arc<dyn Installer> = match SuiteStore::for_user(Arc::clone(&catalog)) {
    Some(store) => Arc::new(store),
    None => Arc::new(NoStore),
};
let index = installer.indexes().map_err(|e| e.to_string());
// once, at startup: settings, jobs and language of the app
Host::builder(AppInfo { id: "myapp", name: "My App", version: env!("CARGO_PKG_VERSION") })
    .settings(sections)
    .install(ctx);
let options = Options { catalog, installer, index };
let mut panel = FontsPanel::new(options);
// each frame
panel.ui(ui);
```

| Call | Use |
|---|---|
| `set_look(look)` | The host's palettes (dark and light) and font families, and whether labels are set in capitals |
| `settings()` | The panel's `FontsPanelSettings`: the sample text, preset and size, and whether the downloads note was closed. The panel reads them from the Host's `fonts_panel` section on its first frame and writes the section when they change; without a Host they stay in memory |
| `downloads_allowed()`, `set_downloads_allowed(on)` | The download consent, for a host's settings page. It is the font store's own setting, shared by every Craft app |
| `show_downloads_switch(false)` | For a host with the switch in its own settings |
| `sources_ui(ui)` | The font sources, for a host's settings page |
| `set_index(index)` | The indexes, after the host fetched them |
| `install_interface_fonts(ctx, craft_faces)` | For a host without interface fonts of its own (the standalone app): craft-ui's font stack for the language of the Host installed in `ctx`, craft-fonts' faces the app embeds with craft-fonts-build, and the interface direction |

## Network use

Nothing is downloaded until the user allows it. Downloads are off by default; the first Install or
Preview asks first. Files come from each source's repositories at the commit or tag its index
records, through jsDelivr (raw GitHub as the fallback), and each is checked against the index's
size and sha256 before use. Previews stay in memory and are never installed. A source the user
adds is an index at an https URL; its families are never curated.

## Strings

Every interface string goes through craft-i18n with the English text as the key. The panel embeds
its own Japanese and Arabic catalogs (`src/panel/catalogs/`) until its rows are part of
craft-i18n; style and weight names, licence names and product names stay in English.

## Testing

```sh
cargo test -p craft-font-manager --features screen-testing --test it
```

`tests/it` runs the panel's trials against a small index, a font folder made per test and `FakeStore`. They install egui's Hack and Ubuntu Light faces into temporary folders.
