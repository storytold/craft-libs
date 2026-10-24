# Craft UI

`craft-ui` provides small egui components with application-supplied styles. `icons` prepares
embedded SVG catalogs; `buttons` paints accessible icon buttons; `tabs` handles tab fitting and
interaction; `layout` provides toolbar fitting, split geometry and resize handles; `theme` shares widget-state styling.
`docking` supplies transactional panel moves and an optional docking surface.
`printing` supplies printer/job controls and asynchronous feedback used by WordCraft and GridCraft.
`file_nav` is the navigation column of the Office-style apps' File page.
`places` is the panel an app opens documents from and saves them to, with or without remote storage.
The current delivery covers nine creative apps (DesignCraft, VectorCraft, PhotoCraft, FilmCraft, EffectCraft, CADCraft, PDFCraft, LightCraft and SoundCraft); prior basic and office adapters are separate. The [suite audit](../../docs/ui-components-audit.md#adoption-and-acceptance)
records their scope and acceptance limits. Assets, complete themes, localization and command handling
remain in each application. No icons, fonts or application engines are bundled.

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

`tabs::Tab` registers a tab at a supplied rectangle and invokes the application's painter:

```rust
fn tab(ui: &mut egui::Ui, rect: egui::Rect, selected: bool) -> egui::Response {
    craft_ui::tabs::Tab::new(ui.id().with("layers-tab"), "Layers", selected)
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

This is a mechanism for coloring widgets, not a shared theme schema. Fonts, complete palettes,
selection colors, shadows, spacing, context storage and preference serialization remain in each
application. See the [eight-app comparison](../../docs/theme-infrastructure.md) for the exact
common operation and the differences preserved by the adapters.

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

## File page navigation

`file_nav::FileNav` is the column down the left of the File page in the Office-style apps: a
labelled Back, the pages in groups with a rule between them, and a foot for Options and About.
WordCraft, GridCraft and DeckCraft show the same column, each in its own colour. The `office`
renders of `places_preview` show it beside the places panel.

```rust
use craft_ui::file_nav::{FileNav, FileNavAction, FileNavItem, FileNavStyle};

fn column(ui: &mut egui::Ui, showing: &'static str, accent: egui::Color32, title_bar: f32) {
    let style = FileNavStyle::on_accent(accent);
    let item = |key, label| FileNavItem { key, label };
    let top = [item("home", "Home"), item("new", "New"), item("open", "Open")];
    let rest = [item("info", "Info"), item("save", "Save"), item("print", "Print"), item("export", "Export")];
    let foot = [item("options", "Options"), item("about", "About")];
    let icons = |_key: Option<&str>, _painter: &egui::Painter, _rect: egui::Rect, _colour: egui::Color32| { /* the app's own icon; `None` is Back */ };
    match FileNav::new("Back", &[&top, &rest], Some(showing), &style, &icons).foot(&foot).title_band(title_bar).show(ui).action {
        Some(FileNavAction::Back) => { /* leave the File page */ }
        Some(FileNavAction::Choose(page)) => { /* show that page, or run Save */ }
        Some(FileNavAction::DragWindow) => ui.ctx().send_viewport_cmd(egui::ViewportCommand::StartDrag),
        None => {}
    }
}
```

- **Back is a row.** It has a label and an icon like every other entry, and it comes first.
- **The title band.** Where the File page covers the title bar (macOS), `title_band` leaves a band
  of that height above Back for the window buttons. It holds nothing else and reports
  `FileNavAction::DragWindow`. Elsewhere pass 0 and the column starts with Back.
- **Keyboard.** Tab reaches every entry, Up and Down walk them and wrap at the ends, Home and
  End reach the ends, and Enter or Space chooses the entry in focus. The title band is not a stop.
- **Keys.** `K` is whatever the app names its pages with (`&str`, an enum); it comes back in
  `FileNavAction::Choose`. The page that is showing is marked, and announced as chosen.
- **Disabled entries.** `.disabled(&keys)` marks the entries that cannot be chosen now (Info,
  Save, Print and Close with no document open). They keep their place and are drawn in
  `FileNavStyle::disabled` with no hover, are announced as unavailable, never come back in
  `FileNavAction::Choose`, and Tab and the arrow keys pass over them.
- **Too little room.** A label longer than the column ends in an ellipsis and is whole in its
  tip. A column too short for its entries puts the foot after the pages and scrolls.
- **Colour.** `FileNavStyle::on_accent` fills the column with the app's colour;
  `FileNavStyle::from_visuals` is a neutral column with the accent only on the chosen page.
  Fonts are `FontId`s in the style, so the app's own faces apply.

## Places panel

`places::PlacesPanel` is where an app opens a document from and saves it to: places on the left
(Recent, one per connected account, the synced cloud folders, This computer), the chosen place's
files and folders on the right. One panel serves three hosts: the File › Open and Save As pages of the Office-style apps,
the home screen of the graphics apps, and, through `places::menu`, the "Open from" items of a
classic File menu. Preview all of them with
`cargo run -p craft-ui --example places_preview -- OUT_DIR` (set `CRAFT_PREVIEW_FONTS` to a folder
with `Inter-Regular.ttf` to render in Inter).

```rust
use craft_ui::places::{Place, PlaceEntry, PlacesAction, PlacesContent, PlacesLabels, PlacesPanel, PlacesStyle};

fn open_page(ui: &mut egui::Ui, places: &[Place<'_>], selected: usize, entries: &[PlaceEntry<'_>], crumbs: &[&str], remote: bool) {
    let style = PlacesStyle::from_visuals(ui.visuals());
    let labels = PlacesLabels { places: "Places", add_place: "Add a place", browse: "Browse…", loading: "Loading…" };
    let panel = PlacesPanel::new(places, Some(selected), PlacesContent::Entries(entries), &style, labels).crumbs(crumbs).add_place(remote);
    match panel.show(ui).action {
        Some(PlacesAction::Select(place)) => { /* list that place */ }
        Some(PlacesAction::Activate(entry)) => { /* open the file, or list the folder */ }
        Some(PlacesAction::Crumb(step)) => { /* go back up to that folder */ }
        Some(PlacesAction::AddPlace) => { /* start a sign-in */ }
        Some(PlacesAction::Browse) => { /* show the system's own file dialog */ }
        None => {}
    }
}
```

- **No IO.** The panel opens no file and no socket and knows no storage provider. The app passes
  the places and entries it has and acts on the `PlacesAction` it gets back; indices point into
  the slices passed that frame. `PlacesContent::Loading` and `PlacesContent::Message` stand in
  while the app is still asking, or has nothing to list.
- **Nothing to list.** `PlacesContent::Notice` says why properly: an icon, a short title, a
  sentence or two and, where the user can act, a button that reports `PlacesAction::Notice`. Use
  it for an empty folder, a folder that is gone, or one the system will not let the app read.
  Running text keeps to a readable width however wide the page is.
- **With or without remote storage.** An app without it passes no account places and leaves
  `add_place` off, and nothing else changes. "Add a place" is off by default: a control that
  cannot work is worse than none.
- **The system dialog stays.** Browse reports `PlacesAction::Browse` and the app shows the native
  dialog, which already knows the system's own cloud folders and network drives. A host with its
  own Open button above the places turns Browse off.
- **Hosts.** The panel fills the rect it is given. `header` puts the host's own controls above
  the places (a home screen's New and Open buttons); `view(PlacesView::Thumbnails)` with
  `thumbnails` suits pictures, the default list suits documents.
- **Text.** A long name is cut with an ellipsis and never pushes the detail beside it ("Anna is
  editing", "shared by Tom · view only") out of sight. The app supplies every string, and its own
  glyphs through `icons` when the built-in line icons do not match its set.
- **Accent.** The app's accent marks state only: the chosen place, the focus ring and the hovered
  thumbnail. Icons keep one colour, the same in every app. `PlacesStyle::from_visuals` takes the
  accent from `selection.bg_fill`.

### This computer

`places::local::LocalFolder` is what an app adds when its "This computer" place should list
folders in the page instead of sending the user to the system's dialog. It reads a folder once
per move, never per frame, and only lists names.

```rust
use craft_ui::places::local::{LocalFolder, LocalLabels, LocalOutcome};
use craft_ui::places::{Place, PlacesLabels, PlacesPanel, PlacesStyle};

fn this_computer(ui: &mut egui::Ui, folder: &mut LocalFolder, places: &[Place<'_>], style: &PlacesStyle, labels: PlacesLabels<'_>, words: LocalLabels<'_>) {
    let entries = folder.entries();
    let crumbs = folder.crumbs("This computer");
    let crumbs: Vec<&str> = crumbs.iter().map(String::as_str).collect();
    let action = PlacesPanel::new(places, Some(1), folder.content(&entries, words), style, labels).crumbs(&crumbs).show(ui).action;
    match action.map(|action| folder.act(action)) {
        Some(LocalOutcome::Open(path)) => { /* open the document at `path` */ }
        Some(LocalOutcome::Browse) => { /* show the system's own file dialog */ }
        Some(LocalOutcome::Stay) | None => {}
    }
}
```

- **Opening.** `LocalFolder::open(dir, &["docx", "rtf"])` lists the subfolders and the files of
  those kinds (`*` for every file), folders first, each by name; hidden entries are left out and a
  folder of more than `local::MAX_LISTED` entries is cut. `LocalFolder::home()` is where the place
  usually starts.
- **Moving.** `act` moves the listing when a subfolder or a breadcrumb step is chosen, and returns
  the file to open or the request for the system's dialog. Choosing another place is the app's.
- **Nothing to show.** `content` turns an empty folder, a folder that is gone and a folder closed
  to the app into the right `PlacesNotice`, with the app's words from `LocalLabels`. The closed
  folder gets a Browse button: a document picked in the system's dialog still opens.
- **macOS.** The system asks the user once before an app reads Documents, Desktop or Downloads,
  and refuses silently after a No. Give its question a reason in the bundle's `Info.plist`:

  ```xml
  <key>NSDocumentsFolderUsageDescription</key>
  <string>WordCraft lists the documents in your Documents folder on its Open page and opens the ones you choose.</string>
  <key>NSDesktopFolderUsageDescription</key>
  <string>WordCraft lists the documents on your Desktop on its Open page and opens the ones you choose.</string>
  <key>NSDownloadsFolderUsageDescription</key>
  <string>WordCraft lists the documents in your Downloads folder on its Open page and opens the ones you choose.</string>
  ```

  A build started from a terminal has no identity of its own, so the system answers for the
  terminal instead and may refuse without asking.
- **The web** has no folders to list: offer no such place there.

### Synced cloud folders

`places::synced` finds the folders that a cloud service's own app keeps in step on this computer
(OneDrive, Google Drive, iCloud Drive, Dropbox) and makes each a place of its own. There is no
account, no network and no provider API: the service's app moves the files, and the panel lists
the folder with `LocalFolder`, so it works offline.

```rust
use craft_ui::places::synced::{self, SyncedFolder};

fn start(ctx: egui::Context, found: std::sync::Arc<std::sync::Mutex<Vec<SyncedFolder>>>) {
    // It touches the disk, and a cloud drive still starting can keep it waiting: off the UI thread.
    synced::find_in_background(move |folders| {
        if let Ok(mut found) = found.lock() {
            *found = folders;
        }
        ctx.request_repaint();
    });
}
```

- **Places.** `SyncedFolder::place()` is the row: the service's name (not translated) and, where
  there are two of one service, the organisation, account or team under it. They go after the
  accounts and before This computer.
- **Listing.** `SyncedFolder::open(&["docx"])` is a `LocalFolder::rooted` at the synced folder: the
  breadcrumb starts at the place, and its first step leads back to it, not to the home folder.
- **Where it looks.** Windows: `%OneDriveConsumer%`, `%OneDriveCommercial%`, `%OneDrive%` and the
  `OneDrive`/`OneDrive - <organisation>` folders in the home folder; `%USERPROFILE%\iCloudDrive`;
  and, only when Google Drive for desktop is installed, the drive letter holding `My Drive` or a
  mirrored `%USERPROFILE%\My Drive`. macOS: `~/Library/CloudStorage` (`OneDrive-*`,
  `GoogleDrive-*`, `Dropbox`) and iCloud Drive's `~/Library/Mobile Documents/com~apple~CloudDocs`.
  Everywhere: Dropbox's `info.json`, else `~/Dropbox`. Linux has no OneDrive, Google Drive or
  iCloud app of the services' own, so only Dropbox is looked for there.
- **Icons.** `PlacesIcon::Synced` is a folder with a cloud on it, drawn like the others: never a
  service's logo.
- **The web** finds nothing; `find_in_background` returns false where no thread can start.

## Compatibility and checks

The crate uses egui 0.36 and Rust 1.95 or later. Its production dependency disables egui's default
features so fonts and rendering features remain the application's choice. See the
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
