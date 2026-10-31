# Craft UI

`craft-ui` provides small egui components with application-supplied styles. `icons` prepares
embedded SVG catalogs; `buttons` paints accessible icon buttons; `tabs` handles tab fitting and
interaction; `layout` provides toolbar fitting, split geometry and resize handles; `theme` shares widget-state styling.
`docking` supplies transactional panel moves and an optional docking surface.
`printing` supplies printer/job controls and asynchronous feedback used by WordCraft and GridCraft, and a `printing::network` destination for printers reached by address.
`fonts` installs the UI font stack, the same fallback order in every app.
`localized` supplies number fields that write and read the user's region's numbers.
The current delivery covers nine creative apps (DesignCraft, VectorCraft, PhotoCraft, FilmCraft, EffectCraft, CADCraft, PDFCraft, LightCraft and SoundCraft); prior basic and office adapters are separate. The [suite audit](../../docs/ui-components-audit.md#adoption-and-acceptance)
records their scope and acceptance limits. Assets (font files included), complete themes, localization
and command handling remain in each application. No icons, fonts or application engines are bundled.

## SVG catalogs

```rust
use std::sync::OnceLock;
use craft_ui::icons::{SvgIconSet, SvgStyle};

// Supply the application's own attributed assets here.
static ICONS: &[(&str, &[u8])] = &[];

fn icons() -> &'static SvgIconSet {
    static ICONS_SET: OnceLock<SvgIconSet> = OnceLock::new();
    ICONS_SET.get_or_init(|| SvgIconSet::new(
        ICONS,
        SvgStyle { current_color: "#ffffff", stroke_width: Some("1.75") },
        "bytes://my-app/icons/",
        "square",
    ))
}

fn icon(name: &str) -> egui::Image<'static> {
    egui::Image::new(icons().source(name))
}
```

The application installs `egui_extras::install_image_loaders` with its `svg` feature before drawing.
It sets tint and size on the returned image.
The library does not install loaders or an egui theme.

The catalog is immutable. SVGs are prepared once and held in `Arc<[u8]>`; lookups share those bytes.
The fallback uses the requested name in its URI, matching PhotoCraft and VectorCraft. Missing both
the requested name and fallback produces empty bytes for the loader to report as an error, without
indexed lookups or a panic. `contains` and `canonical_name` inspect only actual catalog entries. PdfCraft resolves an unknown
name to `square` before calling `source`, preserving its fallback cache key.

Use a different URI prefix for every catalog/style combination used in the same egui context.
egui caches by URI; supplying different bytes under one URI would reuse stale content. Apps with
their own contexts can retain their current prefixes. Unknown names do not add entries to this
catalog, though the egui loader may cache a requested URI.

This is an adapter for trusted, compiled-in monochrome assets, not an SVG sanitizer. It performs
literal `currentColor` and optional `stroke-width="2"` substitutions, keeps other widths and explicit
colors untouched, and uses the existing lossy UTF-8 behavior. General SVG parsing and rasterization
remain the responsibility of the existing loader.

## Icon buttons

`buttons::IconButton` accepts a label, size and `IconButtonStyle`, and returns `egui::Response`.
`ButtonVisuals` describes fill, border and icon tint for normal, hovered, pressed and selected
states. Selection wins over press and hover. `pressed: None` keeps ordinary hover/normal styling;
`Some` supplies a separate press treatment. egui owns pointer capture and click cancellation.

```rust
use craft_ui::buttons::{ButtonVisuals, IconButton, IconButtonStyle};
use egui::{Color32, CornerRadius, Stroke, Vec2};

fn layer_button(ui: &mut egui::Ui, selected: bool) -> egui::Response {
    let normal = ButtonVisuals {
        fill: Color32::TRANSPARENT, stroke: Stroke::NONE, icon: Color32::WHITE,
    };
    let style = IconButtonStyle {
        normal,
        hovered: ButtonVisuals { fill: Color32::DARK_GRAY, ..normal },
        pressed: None,
        selected: ButtonVisuals { fill: Color32::BLUE, ..normal },
        corner_radius: CornerRadius::same(4),
        focus_stroke: Stroke::new(1.0, Color32::LIGHT_BLUE),
    };
    IconButton::new("Layers", Vec2::splat(28.0), &style).selected(selected)
        .show(ui, |ui, rect, tint| {
            // Use the application's SVG catalog or its existing painter icon here.
            ui.painter().circle_filled(rect.center(), 5.0, tint);
        })
}
```

Use actual theme colors in the style adapter and localized text for the label. The painter receives
the full button rectangle and icon tint; the app chooses icon padding/size. Parent disabled state
applies to painting, interaction and accessibility. The library reports label and selection to
AccessKit and draws the supplied inset focus outline. Mouse, Enter and Space activation come from
egui. The caller reads `clicked()`, runs its command and updates selection; the component neither
dispatches nor toggles state. Attach tooltips to the returned response.

Controls previously painted with one `egui::Painter::rect` can opt into `.single_shape_frame()`.
This keeps their fill/border edge tessellation, especially for translucent disabled borders.
The default paints fill and border separately, preserving existing consumers' rendering.

IDs come from the surrounding UI scope/position, independent of the translated label. Use
`ui.push_id(stable_id, ...)` when items reorder. Dimensions must be finite, positive and no larger
than 4096 points; other sizes produce an inert zero-size response. Invalid stroke widths are
ignored and large strokes are bounded to half the smaller button dimension.

## Tabs

`tabs::fit(natural_widths, selected, available, minimum, overflow_width)` returns visible tab
indices and widths, overflow indices, and the actual reserved overflow width. It shrinks wide
tabs before hiding them, keeps input order and retains the selected tab. Tiny strips may reduce
its width to zero. Negative/non-finite dimensions become zero and large dimensions are bounded.
The work is linear in tab count with at most 32 fitting passes; the caller caps its own tab count.
Docking tab strips use this policy, with `DockStyle::min_tab_width` as the minimum.

`tabs::fit_whole(natural_widths, selected, available, overflow_width)` keeps every shown tab at its
natural width and sends the rest to overflow. Tabs are shown in order up to the first that does not
fit; the selected tab is always shown in its logical place, taking the place of the last tabs that
fit when it lies past them. Only a selected tab wider than the strip minus the overflow button is
narrowed.

`tabs::Tab` registers a tab at a supplied rectangle and invokes the application's painter:

```rust
fn tab(ui: &mut egui::Ui, rect: egui::Rect, selected: bool) -> egui::Response {
    craft_ui::tabs::Tab::new(ui.scope_id().with("layers-tab"), "Layers", selected)
        .sense(egui::Sense::click_and_drag())
        .show_at(ui, rect, |ui, response| {
            let fill = if response.hovered() { egui::Color32::GRAY } else { egui::Color32::DARK_GRAY };
            ui.painter().rect_filled(response.rect, 0.0, fill);
        })
}
```

Supply a stable ID independent of the translated label. The component reports the accessible
Tab role, label and selection, handles egui keyboard activation, and paints a focus outline from
the caller's theme. The caller keeps selection, labels, overflow menus, painting and drag actions;
close buttons and tab context menus stay separate. Dock and floating-panel adapters use the same
component without sharing window or workspace state.

## Splits and resize handles

`layout::split_rect` divides a rectangle along `SplitAxis::Horizontal` (left/right children) or
`Vertical` (top/bottom). `SplitSize` accepts a ratio, fixed first pane, or fixed second pane in
logical points. The result contains `first`, `divider` and `second` rectangles. Gap and minimum
are supplied by the application. Tiny areas relax the minimum to half the space available;
non-finite or inverted input returns `None` instead of panicking.

```rust
use craft_ui::layout::{split_rect, SplitAxis, SplitSize};

fn two_panes(rect: egui::Rect) -> Option<(egui::Rect, egui::Rect)> {
    let split = split_rect(rect, SplitAxis::Horizontal, SplitSize::Ratio(0.5), 4.0, 20.0)?;
    Some((split.first, split.second))
}
```

`layout::ResizeHandle` receives a stable ID, axis, divider rectangle, localized label and
`ResizeHandleStyle` colors. `show` returns an egui response and a movement delta for that frame.
Pointer drags, focused axis-arrow keys and AccessKit increment/decrement actions all produce
that delta. `value(current, range)` adds accessible numeric bounds and SetValue support. Tab
continues normal focus traversal. Invalid geometry or numeric parameters make the handle inert.

Apply the delta through `SplitLayout::resized(size, delta, minimum)` to retain fixed/ratio sizing,
or use an application-specific adapter. The library does not persist geometry, dispatch commands
or create panes. Apps keep their existing dock trees, stacked panels, layout files, pane contents,
cameras, document focus and native windows. Shared controls do not constitute a dock-tree migration.

## Panel docking

`docking::Layout<P>` owns panel placement independently of application content: a tree of split,
tab and accordion nodes plus floating tab groups. `apply(Action)` validates the complete layout,
applies a proposed change to a copy and commits only a valid result. Moves, tab insertion/reorder,
side splits, detach, activate, close, reveal and resizing use this one transaction path. Panel IDs
must be unique, stable values; labels and document state are supplied by the app.

`Layout::location` captures a panel's placement before hiding it; `restore` returns it there if
the remembered destination still exists. Apps choose an explicit fallback when it does not.
Enable the optional `serde` feature to persist the shared layout, or convert to/from an existing
workspace schema. Validate loaded input before rendering or recursively converting it.

`DockArea` is an optional renderer. It takes a layout snapshot, styles, labels, panel permissions
and body callbacks, then returns actions for the app's command handler. Its floating groups stay
inside the current egui viewport. Protected document canvases have no tab header. Per-panel size
limits constrain geometry without overwriting saved preferred widths when the window shrinks.
Applications with established chrome can use the shared transaction model and `resolve_drop`
without changing their renderer. See [the docking guide](../../docs/ui-docking.md) for details.

## Toolbar overflow

`layout::fit_toolbar(natural_widths, available, spacing, overflow_width)` keeps an ordered
prefix of indivisible groups at their natural widths. When groups do not fit, it reserves the
full overflow-control width and the preceding gap. `ToolbarFit` contains visible `(index, width)`
pairs, overflow indices and the reserved width. It does not skip wide groups or compress controls.

The application paints an enabled, focusable overflow control and makes every overflow index
available in its menu through existing commands. Group identity, labels, selection and command
dispatch remain local. An empty input produces an empty fit. Invalid dimensions, more than 4096
groups, or an available width too small for the overflow control return `None`; the caller must
then provide a scrollable row, wrapping or an existing application menu. Never turn that result
into silently missing commands. See the API documentation for the accepted numeric bounds.

## Theme mechanisms

`theme::WidgetPalette` applies background colors to egui's five widget states, with a supplied
corner radius and foreground stroke. VectorCraft, DesignCraft and LightCraft construct it from their existing
local tokens. It updates each state's strong/weak background fill, radius and foreground; borders,
expansion and all other visual settings are retained.

```rust
use craft_ui::theme::WidgetPalette;
use egui::{Color32, CornerRadius, Stroke};

fn style_widgets(visuals: &mut egui::Visuals, palette: WidgetPalette, text: Color32) {
    palette.apply_to(&mut visuals.widgets, CornerRadius::same(4), Stroke::new(1.0, text));
    // Optional app-specific exceptions belong after the shared operation.
    visuals.widgets.inactive.weak_bg_fill = Color32::TRANSPARENT;
}
```

This is a mechanism for coloring widgets, not a shared theme schema. Complete palettes,
selection colors, shadows, spacing, context storage and preference serialization remain in each
application; fonts are installed by `fonts`, below. See the
[eight-app comparison](../../docs/theme-infrastructure.md) for the exact common operation and the
differences preserved by the adapters.

## UI fonts

egui draws a character only if one of its installed faces has it, so an app's labels, layer names
and file names show boxes for any script its faces miss. `fonts::UiFonts` builds every app's UI
font stack in the same order:

1. the app's own faces for the family (UI face, semibold, mono), or egui's default faces for a
   family the app has none for;
2. craft-fonts faces for the UI language;
3. craft-fonts faces for every other script the app ships, so a Japanese interface still draws
   Arabic file names;
4. installed system fonts (see below), set to regular weight so variable fonts don't draw Thin;
5. emoji faces (system emoji first, then the colour, web and monochrome faces the app embeds) and,
   behind the app's own faces, egui's default faces.

When the faces of step 1 don't cover the UI language's main script (Arabic, Thai, Georgian), that
script's face goes first. egui takes a row's height from a family's first face, so the language's
own face sets the row height and its marks aren't clipped. Han, kana and Hangul sit inside the em
square and fit a Latin row, so Chinese, Japanese and Korean UIs keep the Latin face first: Latin
text draws in the app's face in every language, and CJK characters fall back to the language's
face. Rows and widgets keep the same height in every one of these languages as in a Latin UI:
CJK faces sit on the first face's baseline inside its row, scaled down only if a glyph would
otherwise leave the row (Noto Sans CJK and BIZ UDPGothic at the same size don't need it), with
crates.io egui and the suite's patched egui alike. `UiFonts::cjk_rows(CjkRows::Grow)` opts into
the CJK face's taller line instead. The app's theme sets the minimum widget height for rows of its
own face; when the UI language's script face leads the stack and has taller rows, `install` adds
the difference to the theme's height, and takes it off again when a later install needs less. The
theme's height and padding stay the app's: a theme applied after `install` is fitted at the start
of the next pass, so the result doesn't depend on which comes first. On the suite's patched egui
(`FontTweak::set_baseline_fallback`, which the other craft faces and installed text faces turn
on), other fallback glyphs sit on the row's baseline and rows grow to fit them by themselves; emoji
and symbol faces (colour glyphs, or "Emoji" or "Symbol" in the name) stay centred. With crates.io
egui those fallbacks are centred. Monospace keeps the app's or egui's mono face first. Han characters take the regional face the language needs: `zh-TW` gets the
Traditional (TC) face, then Hong Kong, then Simplified; `zh-HK` the Hong Kong face first; `ja` the
Japanese face; `ko` the Korean one.

The app keeps its font files: its `build.rs` embeds the craft-fonts faces it wants from
`fonts/manifest.txt` with `craft-fonts-build`, and craft-ui never depends on craft-fonts. A crate
below the UI (an app's engine or font crate, without egui) holds the table as
`craft_fonts_build::EmbeddedFace` values (storytold/craft-libs#213), and the UI converts each into
a `fonts::CraftFace` with `From`. Apps embed with `Embed::subset_to(catalogs)`, which takes the
CJK faces as craft-fonts' interface faces (`Noto Sans TC UI`, BIZ UDPGothic Regular) and cuts them
at build time to the characters the translation catalogs use (about 3 MB for all five with
FilmCraft's catalogs). A character a cut face lacks, such as one in a file name, comes from
another region's interface face that has it, then from the installed system fonts; the build
warns when a catalog has characters no embedded face covers. Preferences name the full family
(`Noto Sans TC`) and also apply to its interface face (`Noto Sans TC UI`).

The app configures its own faces, a preferred face per script, extra named families (a semibold
family, a family led by Arabic for the runs it shapes itself), emoji faces, and whether system
fonts come before or after the craft faces for other scripts. `UiFonts::stack` returns a `Stack`
whose chains the app can still edit (`insert`, `replace`, `remove`) before `Stack::install`.
`Stack::after_system` (a `FaceList`) holds the faces asked after the system fonts, readable with
`get` and `faces`; `Stack::flat_definitions` appends them to each chain, for a context that takes
its fonts only through `set_fonts`.

`fonts::I18nScripts` reads each language's scripts and Han forms from craft-i18n; an app with its
own table implements `fonts::LanguageScripts` instead.

```rust
use craft_fonts_build::EmbeddedFace;
use craft_ui::fonts::{CraftFace, Family, I18nScripts, UiFonts};
use egui::{FontData, FontFamily};

pub static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

fn install_fonts(ctx: &egui::Context, ui_language: &str) {
    let craft_faces: Vec<CraftFace> = CRAFT_FONTS.iter().copied().map(CraftFace::from).collect();
    UiFonts::new(&I18nScripts)
        .app_face("ui", FontData::from_static(UI_FONT), &[FontFamily::Proportional])
        .app_face("ui-semibold", FontData::from_static(UI_SEMIBOLD), &[FontFamily::Name("semibold".into())])
        .app_face("mono", FontData::from_static(MONO_FONT), &[FontFamily::Monospace])
        .family(Family::named("semibold").weight(600))
        .craft_faces(&craft_faces)
        .languages(OFFERED_LANGUAGES)
        .ui_language(ui_language)
        .install(ctx);
}
```

Call `install` again when the UI language changes.

### Web builds

A web build serves the craft-fonts faces beside its wasm instead of embedding them, so the wasm
stays small and an English interface never downloads Chinese. `craft-fonts-build`'s
`Embed::serve` copies the faces into a directory and writes a table of `fonts::ServedFace`s (URL,
size, Subresource Integrity hash and the characters the face maps, no bytes);
`UiFonts::served_faces` takes it in place of `craft_faces`. Each face's characters are its
`cmap`'s code point ranges, packed as LEB128 gaps and lengths: about 3 KB for a CJK interface face
cut to an app's catalogs, a few hundred bytes for Noto Sans, the symbol faces or an emoji face. The
stack orders served faces exactly as embedded ones, and on the web `Stack::install` fetches them:

- the UI language's faces at once (`Stack::ui_downloads`: the best face for each of its scripts
  the app's faces lack, per family, so a semibold family brings the bold face);
- for a character no installed face has, the one face that draws it (`Stack::downloads_for`):
  the first face in the family's order whose file maps the character and whose scripts fit it.
  Noto Sans draws Latin, Greek and Cyrillic letters the app's faces lack (Vietnamese ẫ), the
  symbol faces (Noto Sans Symbols, Symbols 2, Math) keyboard keys, arrows and other text symbols
  (⌘ ⌥ ⇧ ⎋ ⏎), the emoji faces emoji. A Han character takes the interface face of the UI
  language's region first (Simplified, Traditional, Hong Kong, Japanese or Korean), then the
  other offered languages' and the rest in manifest order;
- a served emoji face (`Emoji::served`) the first time text needs an emoji.

A character no served face maps, such as a rare Han character none of the cut interface faces
has, fetches nothing: egui's web glyph rasterizer draws it with the browser's fonts, or it shows
as a missing glyph. A file that
fails to download leaves its characters to the next face that draws them and is fetched again
after 2 seconds, then after twice as long with each further failure, up to 10 minutes. Each face is
added when its file arrives, and the context repaints; text falls back to the faces after it until
then. Fetched files stay in memory for later installs, such as a language switch, and the
browser's HTTP cache keeps them across page loads. `fonts::loading(ctx)` is true while the UI
language's files are on their way, for an app that would rather draw nothing than Latin fallbacks
in its first frames.

`build.rs`:

```rust
fn main() {
    println!("cargo::rerun-if-changed=build.rs");
    // CJK faces as craft-fonts' interface faces, cut to the app's catalogs (`catalogs` as in
    // craft-fonts-build's README; `subset` feature on the build-dependency).
    let fonts = craft_fonts_build::Embed::new().subset_to(catalogs("myapp"));
    let written = if craft_fonts_build::target_is_wasm() {
        // Served by the page as `fonts/`; see Trunk.toml.
        fonts.serve("web/fonts", "fonts/").write("craft_fonts.rs")
    } else {
        fonts.write("craft_fonts.rs")
    };
    if let Err(e) = written {
        println!("cargo::error={e}");
    }
}
```

The app:

```rust
// The embedded table is written as `EmbeddedFace` literals, the served one as `ServedFace`s.
use craft_ui::fonts::{CraftFace as EmbeddedFace, I18nScripts, ServedFace, UiFonts};

#[cfg(not(target_arch = "wasm32"))]
static CRAFT_FONTS: &[EmbeddedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));
#[cfg(target_arch = "wasm32")]
static CRAFT_FONTS: &[ServedFace] = include!(concat!(env!("OUT_DIR"), "/craft_fonts.rs"));

fn install_fonts(ctx: &egui::Context, ui_language: &str) {
    let fonts = UiFonts::new(&I18nScripts).app_face("ui", FontData::from_static(UI_FONT), &[FontFamily::Proportional]);
    #[cfg(not(target_arch = "wasm32"))]
    let fonts = fonts.craft_faces(CRAFT_FONTS);
    #[cfg(target_arch = "wasm32")]
    let fonts = fonts.served_faces(CRAFT_FONTS);
    fonts.languages(OFFERED_LANGUAGES).ui_language(ui_language).install(ctx);
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if craft_ui::fonts::loading(ui.ctx()) {
            return; // The fetch repaints when it arrives.
        }
        // …
    }
}
```

Call `install_fonts` from the `eframe::App` creator, so the fetches start before the first frame.
The served directory has to reach the site next to the wasm; with Trunk, copy it in a hook
(`Trunk.toml`):

```toml
[[hooks]]
stage = "post_build"
command = "sh"
command_arguments = ["-c", "cp -R web/fonts \"$TRUNK_STAGING_DIR/fonts\""]
```

A relative `serve` directory is under the package whose `build.rs` runs it; a crate deeper in the
workspace can pass `concat!(env!("CARGO_MANIFEST_DIR"), "/../../web/fonts")` instead. Keep the
directory out of version control. A build filters faces for the web as for the desktop (`scripts`,
`filter`), e.g. to leave out bold faces.

Two features, both off by default, change what every craft-ui user draws, so the app turns them on:

- `system_fonts` (desktop): the stack asks the installed fonts, at regular weight, and `install`
  replaces the context's font providers, including the one eframe adds. One index of the
  installed fonts serves every UI language, so a language switch doesn't scan them again.
  `install` starts building it on a background thread; until it is ready, text draws with the
  stack's own faces, and the context repaints when it arrives. Without it,
  `install` keeps the providers the context has (eframe's `system_fonts`, for one) and asks the
  stack's own faces after them. `UiFonts::system_provider` supplies a provider either way, and
  `SystemFonts::Off` drops system fonts.
- `color_fonts`: colour glyphs (COLR, sbix, CBDT), for colour emoji.

egui 0.37's default faces have only a few monochrome emoji and lack symbols such as ⊞ ☰ 🔍. On
the web there are no system fonts, so emoji draw as boxes unless the app sets an `Emoji` face.
`UiFonts::egui_face` puts one of egui's own faces into a family, e.g. Hack in the proportional
family for ⇧ and ▸.

Each app tests its coverage with `fonts::assert_covers`: it checks every shipped translation and a
sample of content scripts against the stack without system fonts, and fails naming the language,
message key and character that has no glyph. Build the stack with `.system_fonts(SystemFonts::Off)`
in that test, and run it with `CRAFT_FONTS_DIR` set so it sees the faces a release embeds. For
languages the app leaves to installed fonts, `fonts::assert_covers_with_system` asks the stack's
system provider too; `fonts::system_font_provider(locale)` gives egui's provider (with
`system_fonts`), shared per locale, for `UiFonts::system_provider` or such a check. At run time,
`Stack::covers_language(language, with_system)` says whether the stack draws a language's scripts,
for leaving a language out of the app's menu on a machine that can't show it.

## Testing screens in each language

The `screen-testing` feature adds `testing::Screens`, for an app's screenshot tests of its key screens in
Arabic, Japanese, Traditional Chinese (Taiwan), Korean, Thai and Vietnamese (`testing::LANGUAGES`).
Each language gets an egui_kittest harness with the app's font stack installed for that UI language
and system fonts off, so snapshots match on every machine and a character no face draws fails the
test. Right-to-left languages lay the screen out from the right. The app's UI closure receives the
`Locale` (language tag and direction) and picks its strings from it.

```rust,ignore
use craft_ui::testing::{KOREAN, Screens, compose};

let screens = Screens::new("rename").size([320.0, 150.0]).fonts(|fonts| fonts.craft_faces(FACES));
// Compares tests/snapshots/rename-ar.png, rename-ja.png, …; UPDATE_SNAPSHOTS=1 writes them.
screens.snapshots(|ui, locale| rename_dialog(ui, locale));

// IME smoke test: a scripted 2-Set Korean composition into the focused field.
let mut harness = screens.harness_state("ko", |ui, locale, state: &mut Dialog| state.ui(ui, locale), Dialog::default());
harness.get_by_role(egui::accesskit::Role::TextInput).focus();
compose(&mut harness, &KOREAN);
assert_eq!(harness.state().name, "한");
```

`compose` sends the preedit and commit events an input method sends (`KOREAN`, `JAPANESE`,
`PINYIN`, `TELEX`), which egui's `TextEdit` and canvases reading `Event::Ime` both take. The module
documentation has the recipe for a shortcut smoke test on a non-Latin layout through craft-keymap.
`tests/screens.rs` is a complete example. egui's default proportional face has no Vietnamese
letters such as ổ, so an app relying on it fails the `vi` screens until its own face covers them. The per-release manual checks are in
[language-testing.md](../../docs/language-testing.md).

## Print settings and feedback

`printing::State` holds the transient printer selection, copies, duplex and grayscale settings,
plus discovery and submission receivers. Hosts supply `Discover` and `Submit` workers; neither
the state nor the controls invokes an OS spooler or generates PDF content. Apps map `Settings`
to `craft-print::Job` and own the Print action, page selection, preview and export behavior.

`State::show` takes a stable `egui::Id` and app-localized `Labels`. It draws the settings only;
apps render typed `Feedback` using their own translations. `poll` requests occasional repaints
while work is pending. Refresh disables submission until the new printer list arrives, preserves
a still-present selection, and otherwise chooses the system default or first available printer.
An empty or failed discovery cannot leave a stale printer ready to submit.

`ready` is false while a job is pending. `begin` refuses replacing an active job receiver;
dropping a receiver does not cancel a native job. Apps must test readiness before invoking the
host submit callback and keep active work alive when closing a window. Queue-default duplex is
represented by `None`, independently of explicit one-sided, long-edge or short-edge choices.
Receipts and backend errors remain unmodified data for the app to display. These controls work
on WASM too; browser PDF printing remains an app-owned service.

## Localized numbers

`localized::LocaleNumbers` writes numbers in the region's digits, decimal mark and grouping and
reads back any digits. Build it from the app's `I18n` with `.with_catalog(&i18n, lang)`, so the
error tooltips come from the catalog rows keyed by `localized::MESSAGES` (English where a row is
missing). `LocalizedDragValue` puts it on an `egui::DragValue`.

`NumberField` is the panel and dialog field. It reads text on commit and keeps text it can't
read, outlined, with the reason in a tooltip.

```rust
let label = ui.label(i18n.t(lang, "Width"));
let frame = Recessed { fill: t.input, stroke: Stroke::new(1.0, t.input_border), ..Recessed::from_style(ui.style()) };
let r = ui.add(NumberField::new(&mut width, &numbers).range(0.0..=1e4).step(1.0).unit(Unit::translated(&i18n, lang, "{n} px"))
    .recessed(frame).scrub_label(&label).mixed(widths_differ));
```

- `.recessed(Recessed)` draws the sunken box: fill, border, focus border, radius, height, margin.
- `.scrub_label(&label)` names the field and scrubs it when the label is dragged: a step every two
  points. A click on the label focuses the field, and Escape puts the value back. The response
  carries the label's drag flags, so the app can make the drag one undo step.
- Arrow keys, the wheel over the focused field and scrubbing step to the next multiple of `step`
  (5.1 up to 6, down to 5); Shift steps ten, Ctrl/Cmd a tenth where the decimals show it.
- `.mixed(true)` with `.placeholder(text)` shows a blank field for values that differ between
  objects. The value stays until the user enters a number, which is `changed()` even when equal.
- `localized::focus_first_field(ctx, true)` when a dialog opens: the first enabled field drawn
  takes the focus with its text selected.

## Screenshot completion

`capture::CaptureSlot` is a one-result slot for egui 0.37's one-shot screenshot callback. It is not a
capture scheduler: request IDs, output paths, deadlines, settling frames, geometry epochs and the
aggregate admission reservation stay in the application. The slot owns strong `Context` and
`ViewportId` handles only in the live request; the callback holds a weak reference, so closing the
request cannot leave a callback-owned context cycle. Keep the slot in the application's request
record, outside egui-owned storage: a slot stored in `egui::Memory::data` or a registered plugin
would form a `Context` → slot → `Context` cycle.

```rust
use craft_ui::capture::{CaptureError, CaptureSlot};
use egui::{Context, ViewportCommand, ViewportId};

fn issue_capture(ctx: &Context, viewport: ViewportId) -> CaptureSlot {
    // A per-image bound, not the application's aggregate admission budget.
    let (slot, callback) = CaptureSlot::new(ctx, viewport, 128 * 1024 * 1024);
    // A pending screenshot command requests the paint that hidden windows need.
    ctx.send_viewport_cmd_to(viewport, ViewportCommand::Screenshot(callback));
    slot
}

fn report_capture(slot: &CaptureSlot) {
    if let Some(result) = slot.take() {
        match result {
            Ok(image) => {
                // Validate the app-owned request/geometry epoch, then encode and reply.
                let _ = image;
            }
            Err(error) => {
                // Report the completion error through the app's request record.
                let _: CaptureError = error;
            }
        }
    }
}
```

`take` returns the image or completion error at most once; `None` means pending, cancelled, already
taken or dropped at that moment. The slot cannot distinguish those closed states, so the
application's request record owns the pending/closed distinction and the associated timeout or
error reply.

Completion checks a non-zero width and height, `width * height * size_of::<Color32>()` with checked
arithmetic, a pixel buffer that matches the reported dimensions, and the per-image byte bound. A
zero width or height is `MalformedImage`; an overflowing pixel or byte count is
`DimensionsOverflow`. `CaptureError` is `#[non_exhaustive]`, so match the known variants with a
wildcard arm. The slot releases its mutex before requesting a repaint, and `cancel`/`Drop` replace
the state and drop any retained image only after unlocking, so a large image is not freed while
another thread is completing.
Both accepted images and rejection errors wake the owning viewport so the application can take
the result and reply promptly.

Every retry needs a fresh `CaptureSlot` and a fresh callback from `CaptureSlot::new`. A callback is
one-shot and a slot never accepts a second completion, even after `take` removed the first result.
Issue an explicit screenshot command only once per slot, after the application has settled the
frame it wants. The first delivered frame wins; later frames delivered to the same callback are
discarded. A completion is published before the wake: a completion racing `cancel` or `Drop`
may already have published its result, so it can still deliver one harmless extra repaint after
cancellation. If the cancellation won, the image is released and a later `take` returns `None`; if
`take` won first, the caller already owns the result. A completion that observes the cancelled or
taken state stores no image and does not wake. Cancellation and wakeup are therefore not one atomic
step, and the helper never blocks a completing thread under its lock.

`complete` calls `Context::request_repaint_of`, which takes egui's context lock. Completing from an
ordinary `Ui` closure, an egui 0.37 plugin hook or a renderer/worker thread is fine: these run outside
the context lock. Repaint observers and request-repaint callbacks run under that lock, so completing
there would deadlock. An observer can take or cancel a slot because those operations only use the
slot lock, but further `Context` access must happen outside the observer.

Correlation is based on callback ownership. The helper cannot detect a callback sent to the wrong
viewport if the returned image otherwise fits its bound. The application must route each callback
to its recorded viewport and validate its request and geometry; native multi-window tests must
verify the actual pixels as well as callback routing.

## Text reading areas

`text::align_galley_rows` positions already shaped rows inside a fixed-width reading area without changing glyphs or logical text. Supply a per-row alignment, for example the paragraph direction from the application shaper. The helper uses stock egui APIs and keeps cached glyph meshes shared. Its logical rectangle is the requested width; ink may overflow, so clip the painter when drawing cards. Use it for read-only painting, keeping the original galley for editable hit testing.

## Scanner selection and settings

`scanning::State` holds the selected scanner and capture `Settings` (scanner id, source, colour,
resolution, page count, hardware duplex, last preset) plus the scanner list and the selected
scanner's details. It uses its own light types (`Scanner`, `Connection`, `Source`, `Color`,
`Choices`) and depends on no scanning backend: apps map their engine's types to these.

Every option in `scanning::Choices` is a three-state `capability::Capability<T>` (see "Device
capabilities"): the feeder, the glass, colour modes, resolutions, the page-count range, "until
empty" and the duplex unit. The dialog treats the states differently: **supported** (the device
published a list or range) is enabled and restricted to it, device default first; **unsupported**
(a published list without it, or the unit is absent: no feeder, no duplex unit) is shown disabled
with an app-worded reason, never hidden, and nothing is sent for it; **unknown** (not published, or
the details read failed) is enabled with a quiet "not confirmed" line and sent only as a hint.
`Auto` source falls to the glass when the feeder is unsupported. `Choices::default()` is all unknown
(what applies when the details cannot be read). Capability is static; the device's status (feeder
empty, cover open, "feeder reported ready but empty") is separate: `State::set_warnings` takes
`Warning`s that are drawn as warnings and never disable a control. `Choices::request(&settings)`
returns what to send as `Sent<T>` per option (`Omitted`, `Hint`, `Value`).

Host mapping (the names match the printing dialog's `unsupported_reason` / `unconfirmed_note`):

| Backend knows | Set | Result |
|---|---|---|
| feeder / glass / duplex present, absent or not reported | `presence(Some(true / false) / None)` | supported / unsupported / unknown |
| list of resolutions or colours, empty list, no list | `Capability::from_option_list(Some(list) / Some(&[]) / None)`, or `list_default_first(list, default)` | supported / unsupported / unknown |
| page limit | `Capability::from_option_range(Some((1, max)))` | range `1..=max` |
| details read failed | `Choices::default()` (the library does this itself) | all unknown |
| status | `State::set_warnings(vec![Warning::..])` | warnings only |
| wording | `Labels::unsupported_reason`, `unconfirmed_note`, `reasons` (per option, optional), `warning_*` | the app's language |

`Choices`, `Request`, `Sent` and `Warning` are `#[non_exhaustive]`: start from `Choices::default()`
and assign fields.

**Page length.** On the feeder, `State::show` also draws a "Page length" row with four options
(`scanning::PageLength`): *Driver default*, *Match page size* (enabled when the app sets
`Settings::page_size_chosen`, i.e. it has a page size that names a length), *Detect (scanner)* and
*Detect from image (approximate)*. "Detect" follows the three states through `Choices::detect_length`:
published is enabled, a published "no" is disabled with `LengthLabels::no_detection`, not published (the
default) is enabled with the quiet unconfirmed note and sent only as a hint. The image guess is always
enabled and always shows `LengthLabels::infer_note`, because it is a heuristic. A choice the scanner or the
page size rules out falls back to the driver default (`Choices::length_or_default`, also applied by
`Settings::clamp`); the row is absent on the bed. `Choices::request` carries it as `Request::page_length`
(omitted for the bed and for an unavailable option). The app maps the four values to its backend's request.

Hosts supply two workers that return `mpsc` receivers. `Discover` lists scanners and must open
nothing; `LoadChoices` reads what ONE scanner supports and is called only for the selected scanner.
Results are matched by generation and scanner id, so a slow answer for a scanner the user has left
is dropped. If the details cannot be read (busy, driver error, worker stopped) the scanner stays
listed and selected, `State::choices` returns the conservative `Choices::default()` and
`Feedback::DetailsFailed` carries the backend sentence, so scanning stays possible; `retry` reads
it again. A refresh keeps a still-listed selection and forgets the old details. With nothing
remembered, the USB entry of a device that is also listed over the network is selected (names are
compared without a trailing ` [..]` and ignoring case), otherwise the first.

`State::show` takes a stable `egui::Id`, app-translated `Labels` and both workers. It draws only
the device row (with a connection tag and a hover note from the pure `same_printer_note`) and the
settings with preset buttons, and returns `Response { changed, refresh, retry }`. Apps render
`State::feedback()` in their own words and own the Scan action, preview, interruptions, manual
duplex flows, document handling, commands, theme and any pill styling. Wrap `show` in
`add_enabled_ui` while a scan runs. `Settings::apply_preset` and `Settings::clamp` are pure: a
colour mode the scanner lacks falls back to the first one it lists (the device default when
published). Unknown colour modes take the preset's colour. An unpublished resolution list or an
unsupported option leaves the matching field alone.

## Device capabilities

`capability::Capability<T>` is plain data for what a device says about one option, in three states
that a dialog must treat differently: `Supported(Advertised)` (the device published a list or an
inclusive range, `Choices::List` / `Choices::Range`, and optionally its own default): enable the
control and offer only that; `Unsupported` (the device published its answer and the option is not in
it): show the control disabled with a reason and send nothing; `Unknown` (not published, or the query
failed): enable the control, say it is not confirmed, and send the value as a hint. "Unknown" is
not "unsupported": options are hints a backend may ignore. `Capability::from_option_list` maps `None`
to unknown, an empty list to unsupported and a non-empty list to supported; `from_option_range` and
`list_default_first` (device default first) build the other forms. `allows`, `clamp_to_supported`,
`is_known`, `is_enabled`, `choices`, `list` and `default_value` read it. The type carries no wording
and no egui types; the application supplies the reason and the note. It describes capability only:
a device's status (out of paper, cover open) is a separate, dynamic matter and should only warn.

## Network printer destination

`printing::network` adds a destination for a printer reached by address, beside the spooler-queue
controls above; nothing in `printing::{State, Settings, Labels}` changes. It has its own light types
(`PrinterSummary`, `Caps`, `Request`, `Report`, `Failure`) and depends on no print client: the host
maps its client's answers into them. `Draft` is the pure model, `State` adds the workers.

The host supplies `Host { validate, query, sheets_ok, formats }` and app-localised `network::Labels`
(wording of the per-job confirmation included), and `State::show(ui, id, &host, &labels)` returns a
`Response { block, changed, query_started, cancel_requested }`. `query` is a `QueryPrinter` closure
returning an `mpsc` receiver; the job is a `SendJob` closure returning a `JobHandle` (a result
receiver, optionally an updates receiver, and a cancel closure). `poll` runs each frame inside `show`
and may be called directly.

A job has two phases. The worker sends the document (`JobView::Running`), reports `JobUpdate::Accepted`
when the printer has it (`JobView::Accepted`: "accepted", with a `Stage` of Accepted, Waiting, Held,
Printing or Stopped and optional sheet counters), sends `JobUpdate::Status` as it learns more, and
finishes with the result on the result receiver. For a printer that cannot report, the worker sends
`Accepted` and then `Ok(Report { outcome: Outcome::AcceptedUnknown, .. })`: "sent, outcome unknown",
which is neither a failure nor a claim that it printed. A worker that simply stops after `Accepted`
is shown the same way. A `Failure` can carry `outcome_unknown` (the request may have reached the
printer) and the UI then says to check the printer before sending again. Cancel works in both
phases and the host is told once. Everything arrives over `mpsc` receivers drained once per frame, so
the UI thread never blocks; a worker built on `craft-print` maps `Submitted` to `Accepted`, each
`JobStatus` to `Status` (`JobState::name()` feeds `Stage::from_name`) and its `WatchOutcome` to the
`Report`. A worker that sends no updates shows only its result.

**Sheet totals are optional.** Many printers report sheets done and never a total, so
`Progress::sheets_total` is an `Option`: while it is unknown the accepted phase shows the count
("N sheets", `Labels::sheets`) and no bar that could never fill; once a total is known it shows a
determinate bar (`Labels::sheets_progress`). The app can supply one: set `Draft::expected_pages`
(the document's page count) before `State::start_job`. It travels to the host in
`Request::expected_pages` (a `craft-print` worker passes it as `JobOptions::expected_pages`), and the
model turns it into an estimated sheet total (`Draft::expected_sheets`: pages halved and rounded up
for a two-sided job, times the copies, fixed when the job starts) that fills a missing or zero total
of every `Accepted` and `Status` update. A total the printer reports always wins. A host that builds a
`Request` itself sets `expected_pages` or uses `..Default::default()`; `Progress::fraction()` and
`Progress::with_expected_total()` are the helpers the model uses.

**Options are three-state** (`capability::Capability`, see [Device capabilities](#device-capabilities) above). For copies, sides, orientation,
quality, colour mode and resolution (and the media size the app names in `Draft::request`), `Caps`
holds a `Capability`: *supported* (the printer published a list or range: the control is enabled and
offers only that), *unsupported* (it published its answer and the option is not in it: the control is
shown **disabled**, not hidden, with the app's reason from `Labels::unsupported_*` as visible text and
as the control's accessible description; the request never carries a value for it) or *unknown* (not
published, or the details read failed: the control is enabled with the quiet `Labels::unconfirmed_note`
and the value is sent as a hint, because a backend may ignore it or honour it). Unknown offers every
value the dialog knows (`COMMON_RESOLUTIONS` for resolution, up to `UNKNOWN_MAX_COPIES` copies). The
model also reads a published list that offers nothing to choose as unsupported: sides without a
two-sided value (so the flip-on-long/short-edge choices are disabled with it), colour without
`ColorMode::Color`, copies with a maximum of 1, a list with no value the dialog knows. Capability is
static; the printer's status (state and reasons) is only displayed and never disables a control.
`Draft::{sides,orientation,quality,color,resolution,copies,media}_state()` return the effective state.
A new query or a failed one replaces the states (a failed read leaves everything unknown and clears
the choices); a choice the new answer rules out is cleared.

`Caps` holds its options as `Capability` values (`sides`, `orientations`, `qualities`, `media_sizes`,
`color_modes`: `Capability<String>`; `copies: Capability<u32>`, a range; `resolutions: Capability<Resolution>`); how a host
reads "not said" from `craft-print` is in the table. `Request` and `Draft` carry `color` and `resolution`.
`Labels` has `color`, `color_color`, `color_monochrome`, `resolution`, `resolution_dpi` (`{dpi}`),
`resolution_dpi_xy` (`{x}`, `{y}`), six `unsupported_*` reasons and `unconfirmed_note` (empty omits it).
`ColorMode`, `Resolution`, `COMMON_RESOLUTIONS` and `UNKNOWN_MAX_COPIES` are public, and `Sides`,
`Orientation` and `Quality` derive `Ord`.

How a host fills `Caps` from `craft-print`'s `PrinterDetails`:

| Option (`Caps`) | `PrinterDetails` field | Unknown | Supported | Unsupported |
|---|---|---|---|---|
| `sides` | `two_sided: Option<bool>` | `None` | `Some(true)`: `OneSided` and the two-sided tokens (the field cannot say which edges) | `Some(false)` (or `Supported(["OneSided"])`) |
| `color_modes` | `colour: Option<bool>` | `None` | `Some(true)`: `Color`, `Monochrome` | `Some(false)`: `Supported(["Monochrome"])` (read as unsupported) |
| `copies` | `max_copies: Option<u32>` | `None` | `Some(n)`: range `1..=n` | `Some(0 or 1)` (read as unsupported) |
| `media_sizes` | `paper: Vec<String>` | empty (not distinguishable) | non-empty | not reachable: see below |
| `resolutions` | `resolutions: Vec<Resolution>` | empty (not distinguishable) | non-empty | not reachable: see below |
| `orientations`, `qualities` | no field | always, unless `honoured` is `Some(list)` without `JobHint::Orientation` / `JobHint::Quality` (the backend ignores the hint: map to `Unsupported`) | a client-specific list, if the backend has one | as for `honoured` |
| `formats` | `formats: Vec<DocumentFormat>` | empty: the printer is left to answer for itself (unchanged) | non-empty | - |

Findings for `craft-print` (read only, not changed here): `paper`, `resolutions`, `formats` and
`options` are plain `Vec`s documented as "empty when not published", so a printer that published an
empty list cannot be told from one that said nothing, and a host can only map them to `Unknown`.
`two_sided`, `colour` and `max_copies` are `Option`s and map cleanly. Orientation and quality have no
field at all; `honoured` (`Option<Vec<JobHint>>`, `None` = unknown) says only whether the backend
applies the hint, not which values the device takes. `honoured` is device-independent evidence that
a hint is useless, so it is the one case where a host may map an option to `Unsupported` without a
list from the device.

Other rules kept by the model: the confirmation is
never on by default and is cleared when the address changes, the printer is queried again, a job
starts and the dialog opens (`dialog_opened`); answers for an older query or another address are
dropped; Print is blocked (`Block`) until the typed address was queried, the printer takes one of the
host's `formats`, there is something to print and the box is ticked. The Print button, document
generation, the attestation flow and all translations stay with the app: enable Print when
`Response::block` is `None` and call `State::start_job`, which re-checks every block and spends the
confirmation. No network work happens while drawing, and dropping a result receiver cancels nothing.

## Compatibility and checks

The crate uses egui 0.37 and Rust 1.95 or later. Its production dependency disables egui's default
features, so egui's bundled fonts and rendering features remain the application's choice. It has
no default features; see [UI fonts](#ui-fonts) for `color_fonts` and `system_fonts`, and
[Testing screens in each language](#testing-screens-in-each-language) for `screen-testing`;
`testing` adds the headless docking driver `docking::testing`. See the
[suite audit](../../docs/ui-components-audit.md) for candidates, policies and adoption order.

Validation:

```sh
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --target wasm32-unknown-unknown -p craft-ui
```

The [visual regression suite](../../docs/ui-visual-regressions.md) compares the actual controls
against independent reference painters, including exact mesh and RGBA checks, and proves that
color, spacing and stroke changes fail those comparisons. To retain reviewable PNG pairs:

```sh
CRAFT_UI_VISUAL_DIR=/absolute/path/to/visuals cargo test -p craft-ui --test visual_regression
```
