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
| `store` | always | `Installer`, what the interface needs from the font store: the download consent, install, remove, downloads for a preview, the indexes (`indexes`, `fetch_indexes`) and the sources the user added (`user_sources`, `add_user_source`, `remove_user_source`) and the optional preview service address (`preview_service`, `set_preview_service`) and the previews setting (`previews_enabled`, `set_previews_enabled`). `SuiteStore` is craft-font-store's store, `FakeStore` a stand-in for tests, `NoStore` one for when the store folder is unknown |
| `remote` | always | The preview service client: subset URLs (`PreviewKey`, `Codepoints`), a fetch pool (`Previews`: 8 at a time, highest priority first, cancel by key, results by `poll`) that decodes WOFF2, and the disk cache in the store's `previews/` folder (`DiskCache`, least recently used out past 50 MB). It requests nothing unless a preview service is set and previews are enabled (`Installer::previews_enabled`, a user setting, on by default). There is no prompt, and the install consent (`downloads_allowed`) is not consulted. With previews off nothing goes to the network, and cached previews are still used |
| `FontsPanel` | `egui` | The whole interface as one egui panel |

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
  as they come into view. With a preview service set, families that aren't installed draw from
  small subsets of their face: the sample for the cards on screen, then one screen above and two
  below, then ASCII for the Latin cards on screen; typed text outside those asks for its own
  characters once typing pauses. The arrow keys step through the cards and Enter opens one.
- **Sample toolbar**: a preset (a sentence in the language filtered by, else the family's own
  sample from the index, by its `sample_script`; the alphabet; numerals), the sample text, and the
  size.
- **Details sheet** over the list: every style in its own face with Install or Remove for each,
  Install all and Remove all, the source, scripts and languages, and what a preview downloads.
  With a preview service, styles not installed draw with its subsets: a static file's sample, and
  one subset of each variable file with every axis kept, drawn at each named instance and at the
  values of its axis sliders. The licence block shows the licence, copyright, designer and
  upstream link, an Apache family's NOTICE, and the licence text on request, read through the
  store.
- **Installed**: system and font store fonts as the same cards, filtered by source. Only font
  store families can be removed; system fonts are removed with the system's own tools.
- **Sources** (`sources_ui`, for a host's settings page): the built-in sources, the index URLs the
  user added, marked not curated and flagged when the file at their address changed since it was
  added, adding one, and Update sources.

`FontsPanel` fills the `Ui` it is given and has no window, menu or interface fonts of its own; the
host's `Look` gives it the host's colours and font families. The faces it draws samples in are egui
font families of its own, added a few per frame and taken out again past 64, without touching the
host's fonts. Right-to-left interfaces mirror through `craft_ui::direction`; sample text keeps its
own direction.

```rust
let catalog = Arc::clone(Catalog::global());
catalog.scan_in_background();
let installer: Arc<dyn Installer> = match SuiteStore::for_user(Arc::clone(&catalog)) {
    Some(store) => Arc::new(store),
    None => Arc::new(NoStore),
};
let index = installer.indexes().map_err(|e| e.to_string());
let options = Options { catalog, installer, index, runner: Runner::Thread };
let mut panel = FontsPanel::new(options, Settings::default());
// each frame
panel.ui(ui);
```

| Call | Use |
|---|---|
| `set_look(look)` | The host's palettes (dark and light) and font families, and whether labels are set in capitals |
| `settings()`, `settings_mut()` | What the host saves: the sample text, preset and size, and whether the downloads note was closed (`serde`) |
| `downloads_allowed()`, `set_downloads_allowed(on)` | The download consent, for a host's settings page. It is the font store's own setting, shared by every Craft app |
| `show_downloads_switch(false)` | For a host with the switch in its own settings |
| `sources_ui(ui)` | The font sources, for a host's settings page |
| `set_index(index)` | The indexes, after the host fetched them |
| `with_previews(previews)`, `loading_previews()` | The list's preview service pool (tests pass one with a stand-in `Fetch`), and whether it is fetching |
| `reload_preview_settings()` | Reads the preview service and previews settings again after a host's settings page changed them (also every few seconds) |
| `set_language(tag)` | The interface language, the host's own; English when its catalog isn't embedded |
| `install_interface_fonts(ctx, craft_faces)` | For a host without interface fonts of its own (the standalone app): craft-ui's font stack for the panel's language, craft-fonts' faces the app embeds with craft-fonts-build, and the interface direction |

## Network use

Nothing is installed or downloaded whole until the user allows it. Downloads are off by default;
the first Install or Preview asks first. Files come from each source's repositories at the commit or tag its index
records, through jsDelivr (raw GitHub as the fallback), and each is checked against the index's
size and sha256 before use. Previews stay in memory and are never installed. List previews from a
preview service don't ask: they follow the previews setting (on by default), not the download
consent; with the setting off, only subsets already in the disk cache are shown. A source the user
adds is an index at an https URL; its families are never curated.

## Strings

Every interface string goes through craft-i18n with the English text as the key. The panel embeds
its own Japanese and Arabic catalogs (`src/panel/catalogs/`) until its rows are part of
craft-i18n; style and weight names, licence names and product names stay in English.

## Testing

```sh
cargo test -p craft-font-manager --all-features
CRAFT_FONT_MANAGER_SNAPSHOTS=<dir> cargo test -p craft-font-manager --features egui --test flows screenshots -- --test-threads=1
```

`tests/flows.rs` drives the panel with `egui_kittest` against a small index, a font folder made per
test and `FakeStore`. They install egui's Hack and Ubuntu Light faces into temporary folders. With
`CRAFT_FONT_MANAGER_GOOGLE_FONTS=<google/fonts checkout>`, the fake store also "downloads" the
index's files from the checkout, so screenshots show real previews.
