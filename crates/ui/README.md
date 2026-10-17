# Craft UI

`craft-ui` provides small egui components with application-supplied styles. `icons` prepares
embedded SVG catalogs; `buttons` paints accessible icon buttons; `tabs` handles tab fitting and
interaction; `layout` provides toolbar fitting, split geometry and resize handles; `theme` shares widget-state styling.
`docking` supplies transactional panel moves and an optional docking surface.
`printing` supplies printer/job controls and asynchronous feedback used by WordCraft and GridCraft, and a `printing::network` destination for printers reached by address.
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
published), and unknown or unsupported options leave the matching field alone.

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
`Report`. Workers that send no updates behave exactly as before.

**Sheet totals are optional.** Many printers report sheets done and never a total, so
`Progress::sheets_total` is an `Option`: while it is unknown the accepted phase shows the count
("N sheets", `Labels::sheets`) and no bar that could never fill; once a total is known it shows a
determinate bar (`Labels::sheets_progress`). The app can supply one: set `Draft::expected_pages`
(the document's page count) before `State::start_job`. It travels to the host in
`Request::expected_pages` (a `craft-print` worker passes it as `JobOptions::expected_pages`), and the
model turns it into an estimated sheet total (`Draft::expected_sheets`: pages halved and rounded up
for a two-sided job, times the copies, fixed when the job starts) that fills a missing or zero total
of every `Accepted` and `Status` update. A total the printer reports always wins. Additive changes
for hosts: `Request` gained `expected_pages` (build one with `..Default::default()` if you
construct it yourself), `Draft` gained `expected_pages` and `expected_sheets()`, and `Progress` gained
`fraction()` and `with_expected_total()`; `JobHandle`, `JobUpdate`, `State::start_job` and `Labels` are
unchanged.

**Options are three-state** (`capability::Capability`, see below). For copies, sides, orientation,
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

Breaking changes to `Caps` (this module is unmerged): its option fields are `Capability` values
instead of `Vec<String>` and `Option<(u32, u32)>` (`sides`, `orientations`, `qualities`,
`media_sizes`: `Capability<String>`; `copies: Capability<u32>`, a range), and it gained `color_modes`
and `resolutions`. An empty list used to mean "not said"; now `None`/empty is the host's call, see the
table. `Request` gained `color` and `resolution`, `Draft` gained `color` and `resolution`, and `Labels`
gained `color`, `color_color`, `color_monochrome`, `resolution`, `resolution_dpi` (`{dpi}`),
`resolution_dpi_xy` (`{x}`, `{y}`), six `unsupported_*` reasons and `unconfirmed_note` (empty omits
it); `ColorMode`, `Resolution`, `COMMON_RESOLUTIONS` and `UNKNOWN_MAX_COPIES` are new, and `Sides`,
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
