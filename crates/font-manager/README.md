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
| `store` | always | `Installer`, what the interface needs from the font store: the download consent, install, remove, downloads for a preview, the indexes (`indexes`, `fetch_indexes`), the sources the user added (`user_sources`, `add_user_source`, `remove_user_source`), and a family's copies in the OS user font folder (`available_to_all_supported`, `is_available_to_all`, `not_yet_available`, `make_available`, `withdraw`) with its recorded choice (`available_to_all_choice`, `set_available_to_all_choice`, the default `craft_apps_only`, and `shares` for the two together). `SuiteStore` is craft-font-store's store (native only), `FakeStore` a stand-in for tests, `NoStore` one for when the store folder is unknown. What the preview pool (craft-font-store's `preview`) reads is the `PreviewStore` part |
| `FontsPanel` | `ui` | The whole interface as one egui panel |
| `glyphs` | `ui` | `GlyphsPanel`: the character grid of a face with categories, zoom and Recently Used; the app supplies the glyphs through `GlyphSource`, which also reports when it is ready and when its faces change. Sizes are the `glyphs.*` tokens |
| `missing` | `ui` | `document_fonts`, `missing_font_warnings` and `MissingFontsDialog`: which fonts a document names are not installed, and a replacement for each |
| `axes` | `ui` | `read_axes`, `read_named_instances` and `AxisSliders`: a variable face's axes as sliders with a Style list of its named instances |
| `picker` | `ui` | `FontPicker`: the family field with its list (stars, recents, class and feature filters, similar fonts, previews, style rows, the served families and the banner) |

The stories (`patterns/font-picker/*`, `patterns/glyphs-panel/panel`, `patterns/missing-fonts-dialog/dialog`, `composites/axis-sliders/sliders`) use the two faces egui ships.

## The panel

The layout follows Adobe Fonts' and Google Fonts' browse pages.

- **Across the top**: Browse and Installed, with their counts, and one search field.
- **Filters** at the reading start, each under its label: category, language (searchable by its
  English name or its own, "Japanese · 日本語"), script, source, and installed or not; each option
  with its count.
- **Family cards**, one per row: the name, style count, category and source on one line with the
  card's state at its end (Install, whose menu installs for all apps or for Craft apps only, and
  Preview, Installed, a progress bar while downloading,
  Couldn't install with Retry, "Not curated" for a source the user added), and a large sample in
  the face below. The list is virtual: only the cards on screen are laid out, and their faces load
  as they come into view. With a preview service set, families that aren't installed draw from
  small subsets of their face: the sample for the cards on screen, then one screen above and two
  below, then ASCII for the Latin cards on screen; typed text outside those asks for its own
  characters once typing pauses. A card the service draws offers no Preview; one whose subset
  failed does, and so does its details sheet. The arrow keys step through the cards and Enter
  opens one.
- **Sample toolbar**: a preset (a sentence in the language filtered by, else the family's own
  sample from the index, by its `sample_script`; the alphabet; numerals), the sample text, and the
  size.
- **Details sheet** over the list: every style in its own face with Install or Remove for each,
  Install all (with the same menu) and Remove all, whether the family is available to all apps
  with the switch that changes it, the source, designer, licence, scripts and languages, and what a
  preview downloads. With a preview service, styles not installed draw with its subsets: a static
  file's sample, and one subset of each variable file with every axis kept, drawn at each named
  instance and at the values of its axis sliders. The copyright and the upstream link, and
  an Apache family's NOTICE and, on request, the licence text, read through the store.
- **Installed**: system and font store fonts as the same cards, filtered by source. Only font
  store families can be removed; system fonts are removed with the system's own tools.
- **Sources** (`sources_ui`, for a host's settings page): the built-in sources, the index URLs the
  user added, marked not curated and flagged when the file at their address changed since it was
  added, adding one, and Update sources.

`FontsPanel` fills the `Ui` it is given and has no window, menu or interface fonts of its own; the
craft-ui's theme gives it the colours and interface fonts. The faces it draws samples in come from
craft-ui's document-face registry, which loads them on the host's jobs, keeps the ones drawn lately
up to a budget of font file bytes, and never touches the host's fonts. Right-to-left interfaces mirror through the layout; sample text
keeps its own direction.

The panel follows the `craft_host::Host` installed in the egui context (`Host::builder(..).install(ctx)`):
its strings and numbers are in `host.lang()` each frame (English without a Host, or when the panel
has no catalog for the language), its settings section is `fonts_panel`, and installs, removals and
previews run through `host.jobs()`, one at a time in request order (without a Host, on craft-host's default job service: a thread per runner, inline on wasm32).

Native code (`SuiteStore` is not built for wasm32):

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
| `use_default_filters(language)` | Starts the catalogue on sans-serif families in the host's interface language (English when the index has none for it). Called once, after building the panel |
| `with_previews(previews)` | The preview service's client (craft-font-store's `preview::Previews`), for a host that builds its own (native only) |
| `reload_preview_settings()` | Reads the preview service and the previews setting again, after a host's settings page changed them. The panel also reads them when it first shows and when its window regains focus |
| `loading_previews()` | Whether previews are being fetched |
| `show_downloads_switch(false)` | For a host with the switch in its own settings |
| `sources_ui(ui)` | The font sources, for a host's settings page |
| `set_index(index)` | The indexes, after the host fetched them |
| `install_interface_fonts(ctx, craft_faces)` | For a host without interface fonts of its own (the standalone app): craft-ui's font stack for the language of the Host installed in `ctx`, craft-fonts' faces the app embeds with craft-fonts-build, and the interface direction |

## Network use

No font is installed or downloaded whole until the user allows it. Downloads are off by default; the
first Install or Preview asks first. With a preview service set and previews on (a user setting, on by
default), the panel fetches the small preview subsets of the fonts it shows without asking; with
previews off it uses only those in the disk cache. Files come from each source's repositories at the commit or tag its index
records, through jsDelivr (raw GitHub as the fallback), and each is checked against the index's
size and sha256 before use. Previews stay in memory and are never installed. A source the user
adds is an index at an https URL; its families are never curated.

## Strings

Every interface string is a word of the Host's `I18n` (`craft_ui::words!`), so the panel is in the Host's
language when the app's catalogs have its rows and in English where they have none. `CATALOGS` holds the
Japanese and Arabic rows, until they are part of craft-i18n's shared catalog, as data the app passes to
its Host's `I18n`:

```rust
// An app with no shared catalog of its own; one with a shared catalog joins the rows to it for
// `I18n::from_owned`.
let (i18n, problems) = craft_i18n::I18n::new(craft_font_manager::CATALOGS, &["ja", "ar"]);
let lang = i18n.match_tag(language).unwrap_or(craft_i18n::Lang::EN);
Host::builder(app).i18n(Arc::new(i18n)).lang(lang).install(ctx);
```

Style and weight names, licence names and product names stay in English.

## Testing

```sh
cargo test -p craft-font-manager --features screen-testing --test it
```

`tests/it` runs the panel's trials against a small index, a font folder made per test and `FakeStore`. They install egui's Hack and Ubuntu Light faces into temporary folders.
