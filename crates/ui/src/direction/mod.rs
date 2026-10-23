//! Interface direction for right-to-left languages (Arabic, Hebrew, Persian, Urdu, …).
//!
//! In a right-to-left interface the chrome mirrors, as in right-to-left macOS and Windows apps:
//! the panel that sits at the left moves to the right, rows and menu bars start at the right,
//! menus open under their button's right edge, and back arrows point right. Content keeps its
//! geometry: [`Unmirrored`] is the suite-wide list of what never mirrors.
//!
//! egui has no direction for a whole context. A `Ui` runs left to right unless its layout says
//! otherwise, and panels, popups and hand-painted positions name a side. Call sites name the
//! reading start or end through these helpers instead of left or right. Rows nested in a [`row`]
//! follow it on their own: egui starts a `ui.horizontal*` row from the right when its parent
//! layout runs right to left.
//!
//! # Setting the direction
//!
//! [`resolve`] picks the direction for the UI language: the `CRAFT_UI_DIRECTION` environment
//! variable first (`rtl` or `ltr`, for checking a mirrored layout before a right-to-left catalog
//! exists), then the app's [`Setting`], then the language's script ([`craft_i18n::direction`]).
//! [`apply`] stores it on the context; call it at startup and whenever the language changes.
//! Every helper reads it from a [`HasDirection`]: a `Context`, a `Ui`, or a [`Direction`] itself.
//! Tests force a direction by calling [`apply`] on their own context.
//!
//! # egui's own chrome
//!
//! The suite's egui fork adds `Style::right_to_left`, which mirrors submenus, scroll bars,
//! collapsing-header triangles and window title bars. When the app patches that fork in, [`apply`]
//! sets it on every theme's style; with crates.io egui, egui's own chrome stays left to right.
//! Installing a whole new [`egui::Style`] resets the field, so call [`apply`] after changing theme.

use egui::{Align, Align2, AsIdSalt, Context, Id, InnerResponse, Key, Layout, Panel, Rect, RectAlign, Ui, pos2, vec2};

pub use craft_i18n::Direction;

/// The environment variable that forces a direction whatever the language and setting.
pub const ENV_VAR: &str = "CRAFT_UI_DIRECTION";

/// Where the interface direction comes from: an app setting, usually hidden in a debug section.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize), serde(rename_all = "lowercase"))]
pub enum Setting {
    /// Follow the UI language.
    #[default]
    Language,
    /// Left to right whatever the language.
    Ltr,
    /// Right to left whatever the language.
    Rtl,
}

impl Setting {
    /// The direction this setting gives for a UI language tag.
    pub fn direction(self, ui_language: &str) -> Direction {
        match self {
            Setting::Language => craft_i18n::direction(ui_language),
            Setting::Ltr => Direction::Ltr,
            Setting::Rtl => Direction::Rtl,
        }
    }

    /// `language`, `ltr` or `rtl`, any case, as [`ENV_VAR`] and settings files spell them.
    pub fn parse(value: &str) -> Option<Setting> {
        match value.trim().to_ascii_lowercase().as_str() {
            "language" => Some(Setting::Language),
            "ltr" => Some(Setting::Ltr),
            "rtl" => Some(Setting::Rtl),
            _ => None,
        }
    }
}

/// The interface direction for a UI language tag: [`ENV_VAR`] when set to `rtl` or `ltr`, then
/// `setting`, then the language's script.
pub fn resolve(ui_language: &str, setting: Setting) -> Direction {
    env_setting().filter(|s| *s != Setting::Language).unwrap_or(setting).direction(ui_language)
}

fn env_setting() -> Option<Setting> {
    static ENV: std::sync::OnceLock<Option<Setting>> = std::sync::OnceLock::new();
    *ENV.get_or_init(|| Setting::parse(&std::env::var(ENV_VAR).ok()?))
}

fn key() -> Id {
    Id::unique("craft_ui::direction")
}

/// Draw the interface on `ctx` in `dir` from the next layout on.
pub fn apply(ctx: &Context, dir: Direction) {
    ctx.data_mut(|d| d.insert_temp(key(), dir));
    ctx.all_styles_mut(|style| style.set_right_to_left(dir == Direction::Rtl));
}

/// The suite's egui fork has an inherent `Style::set_right_to_left`; crates.io egui has neither
/// the method nor the field. A method call picks an inherent method over a trait method of the
/// same name, so [`apply`] reaches egui's method on the fork and this no-op everywhere else. The
/// same source thus builds against both without a feature.
#[allow(dead_code, reason = "unused when egui has the inherent method")]
trait SetRightToLeft {
    fn set_right_to_left(&mut self, _right_to_left: bool) {}
}

impl SetRightToLeft for egui::Style {}

/// Anything the interface direction can be read from.
pub trait HasDirection {
    fn direction(&self) -> Direction;
}

impl HasDirection for Direction {
    fn direction(&self) -> Direction {
        *self
    }
}

impl HasDirection for Context {
    /// The direction last [`apply`]-ed to this context; left to right before that.
    fn direction(&self) -> Direction {
        self.data(|d| d.get_temp(key())).unwrap_or(Direction::Ltr)
    }
}

impl HasDirection for Ui {
    fn direction(&self) -> Direction {
        self.ctx().direction()
    }
}

/// Is the interface drawn right to left?
pub fn is_rtl(d: &impl HasDirection) -> bool {
    d.direction() == Direction::Rtl
}

// Panels and layouts.

/// A side panel at the reading start: left, or right in a right-to-left interface.
pub fn start_panel(d: &impl HasDirection, id_salt: impl AsIdSalt) -> Panel {
    if is_rtl(d) { Panel::right(id_salt) } else { Panel::left(id_salt) }
}

/// A side panel at the reading end: right, or left in a right-to-left interface.
pub fn end_panel(d: &impl HasDirection, id_salt: impl AsIdSalt) -> Panel {
    if is_rtl(d) { Panel::left(id_salt) } else { Panel::right(id_salt) }
}

/// A vertically centred row in reading order.
pub fn row(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::right_to_left(Align::Center) } else { Layout::left_to_right(Align::Center) }
}

/// A vertically centred row that starts from the reading end, for groups pinned to the far end
/// of a bar: a dialog's buttons, a title bar's controls, a status bar's zoom field.
pub fn trailing(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::left_to_right(Align::Center) } else { Layout::right_to_left(Align::Center) }
}

/// A top-down column with its items against the reading start.
pub fn column(d: &impl HasDirection) -> Layout {
    Layout::top_down(start_align(d))
}

/// [`Ui::horizontal`] in reading order. Use it where the parent runs top down (a panel, a
/// column); inside a [`row`], `ui.horizontal` already follows the parent.
pub fn horizontal<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    let layout = row(ui);
    ui.allocate_ui_with_layout(size, layout, add)
}

/// A left-to-right row whatever the interface direction, for content on the [`Unmirrored`] list
/// placed inside a mirrored row: a transport bar, a slider with its value, a timecode.
pub fn unmirrored_row<R>(ui: &mut Ui, _what: Unmirrored, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, Layout::left_to_right(Align::Center), add)
}

// Hand-painted positions.

/// The horizontal alignment of the reading start: [`Align::Min`], or [`Align::Max`] right to left.
pub fn start_align(d: &impl HasDirection) -> Align {
    if is_rtl(d) { Align::Max } else { Align::Min }
}

/// The x of `rect`'s reading-start edge.
pub fn start_x(d: &impl HasDirection, rect: Rect) -> f32 {
    if is_rtl(d) { rect.right() } else { rect.left() }
}

/// The x of `rect`'s reading-end edge.
pub fn end_x(d: &impl HasDirection, rect: Rect) -> f32 {
    if is_rtl(d) { rect.left() } else { rect.right() }
}

/// `1.0`, or `-1.0` right to left: multiply an x offset written for left to right by it, as in
/// `start_x(ui, rect) + forward(ui) * padding`.
pub fn forward(d: &impl HasDirection) -> f32 {
    if is_rtl(d) { -1.0 } else { 1.0 }
}

/// An anchor written for left to right, mirrored in a right-to-left interface.
pub fn align2(d: &impl HasDirection, ltr: Align2) -> Align2 {
    if is_rtl(d) { ltr.flipped_x() } else { ltr }
}

// Popups.

/// A popup alignment written for left to right, mirrored in a right-to-left interface.
pub fn popup(d: &impl HasDirection, ltr: RectAlign) -> RectAlign {
    if is_rtl(d) { ltr.flipped_x() } else { ltr }
}

/// A menu below its button, under the button's reading-start edge.
pub fn below(d: &impl HasDirection) -> RectAlign {
    popup(d, RectAlign::BOTTOM_START)
}

/// A menu above its button, over the button's reading-start edge.
pub fn above(d: &impl HasDirection) -> RectAlign {
    popup(d, RectAlign::TOP_START)
}

/// A flyout beside its item, towards the reading end, top edges level: a tool's flyout, a
/// hand-made submenu.
pub fn beside(d: &impl HasDirection) -> RectAlign {
    popup(d, RectAlign::RIGHT_START)
}

// Arrow keys.

/// A move along a row or list in reading order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Step {
    Previous,
    Next,
}

/// What the left and right arrow keys mean in a mirrored row (tabs, a toolbar, a segmented
/// control, a breadcrumb): `ArrowRight` moves to the previous item right to left. `None` for any
/// other key. Over [`Unmirrored`] content (nudging on a canvas, stepping a timeline or a slider)
/// read the keys as they are.
pub fn step(d: &impl HasDirection, key: Key) -> Option<Step> {
    let (previous, next) = if is_rtl(d) { (Key::ArrowRight, Key::ArrowLeft) } else { (Key::ArrowLeft, Key::ArrowRight) };
    if key == previous {
        Some(Step::Previous)
    } else if key == next {
        Some(Step::Next)
    } else {
        None
    }
}

/// The arrow key that makes `step`, as a shortcut label shows it.
pub fn step_key(d: &impl HasDirection, step: Step) -> Key {
    match (step, is_rtl(d)) {
        (Step::Previous, false) | (Step::Next, true) => Key::ArrowLeft,
        (Step::Next, false) | (Step::Previous, true) => Key::ArrowRight,
    }
}

/// The step pressed this frame on the arrow keys, if any; the keys stay in the input.
pub fn pressed_step(ctx: &Context) -> Option<Step> {
    ctx.input(|i| [Key::ArrowLeft, Key::ArrowRight].into_iter().find(|k| i.key_pressed(*k))).and_then(|k| step(ctx, k))
}

// Icons.

/// Icon pairs that are each other's mirror image: right to left, each name draws as the other.
/// Names follow Lucide, which the suite's icon sets use; an app with other names maps them first.
///
/// They point along the reading direction: back and forward, previous and next, collapse and
/// expand a side panel, a reply and a forward.
pub const SWAPPED_ICONS: &[(&str, &str)] = &[
    ("arrow-left", "arrow-right"),
    ("arrow-big-left", "arrow-big-right"),
    ("arrow-left-to-line", "arrow-right-to-line"),
    ("arrow-left-from-line", "arrow-right-from-line"),
    ("chevron-left", "chevron-right"),
    ("chevrons-left", "chevrons-right"),
    ("chevron-first", "chevron-last"),
    ("circle-arrow-left", "circle-arrow-right"),
    ("circle-chevron-left", "circle-chevron-right"),
    ("square-chevron-left", "square-chevron-right"),
    ("move-left", "move-right"),
    ("panel-left", "panel-right"),
    ("panel-left-close", "panel-right-close"),
    ("panel-left-open", "panel-right-open"),
    ("reply", "forward"),
];

/// Icons with no mirrored counterpart that are drawn flipped right to left: what they show starts
/// at the reading start. Indent and outdent (nesting in an outline, a list or a layer tree),
/// bulleted lists, signing in and out, sending.
pub const FLIPPED_ICONS: &[&str] =
    &["indent-increase", "indent-decrease", "indent", "outdent", "list", "list-checks", "list-tree", "log-in", "log-out", "send", "send-horizontal"];

/// Icons that look directional but keep their drawing right to left. [`icon`] returns every name
/// not in [`SWAPPED_ICONS`] or [`FLIPPED_ICONS`] unchanged; this list records the decisions.
///
/// - Undo and redo: history runs in time, which the suite draws left to right (timelines, media
///   transport). The buttons still sit in reading order, since their row mirrors.
/// - Rotate, refresh and flip: they act on the canvas, which doesn't mirror, so counterclockwise
///   stays counterclockwise.
/// - Media transport: see [`Unmirrored::MediaTransport`].
/// - Paragraph alignment (`align-left`, `text-align-start`, …): document text has its own
///   direction. Pass the paragraph's [`Direction`] to [`icon`] where an icon means its start or end.
/// - Object alignment and distribution: they name edges of the canvas.
/// - Numbered lists: the digits would mirror.
pub const KEPT_ICONS: &[&str] = &[
    "undo",
    "redo",
    "undo-2",
    "redo-2",
    "undo-dot",
    "redo-dot",
    "rotate-ccw",
    "rotate-cw",
    "refresh-ccw",
    "refresh-cw",
    "flip-horizontal",
    "flip-horizontal-2",
    "flip-vertical",
    "flip-vertical-2",
    "play",
    "pause",
    "skip-back",
    "skip-forward",
    "step-back",
    "step-forward",
    "rewind",
    "fast-forward",
    "align-left",
    "align-right",
    "text-align-start",
    "text-align-end",
    "align-start-vertical",
    "align-end-vertical",
    "align-horizontal-justify-start",
    "align-horizontal-justify-end",
    "align-horizontal-distribute-start",
    "align-horizontal-distribute-end",
    "list-ordered",
];

/// An icon to draw for the interface direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Icon<'a> {
    /// The icon to look up in the app's set.
    pub name: &'a str,
    /// Draw it mirrored, through [`Icon::uv`].
    pub flip: bool,
}

impl Icon<'_> {
    /// The texture coordinates to draw the icon with, for [`egui::Image::uv`] or
    /// [`egui::Painter::image`]: the whole texture, mirrored when [`Icon::flip`] is set.
    pub fn uv(&self) -> Rect {
        if self.flip { Rect::from_min_max(pos2(1.0, 0.0), pos2(0.0, 1.0)) } else { Rect::from_min_max(pos2(0.0, 0.0), pos2(1.0, 1.0)) }
    }
}

/// The icon to draw for `name`: its mirror-image counterpart or a flipped drawing right to left
/// ([`SWAPPED_ICONS`], [`FLIPPED_ICONS`]), and `name` as it is otherwise.
pub fn icon<'a>(d: &impl HasDirection, name: &'a str) -> Icon<'a> {
    let plain = Icon { name, flip: false };
    if !is_rtl(d) {
        return plain;
    }
    for &(a, b) in SWAPPED_ICONS {
        if name == a {
            return Icon { name: b, flip: false };
        }
        if name == b {
            return Icon { name: a, flip: false };
        }
    }
    Icon { name, flip: FLIPPED_ICONS.contains(&name) }
}

// What never mirrors.

/// The suite-wide list of what keeps its left-to-right geometry in a right-to-left interface.
/// These show the content itself, or follow a convention that holds in right-to-left locales too.
/// Their surroundings (the panel, the toolbar, the labels) still mirror. Lay such content out
/// with [`unmirrored_row`] or a fixed [`Layout::left_to_right`], never with [`row`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Unmirrored {
    /// Document canvases and viewports, with everything drawn over them: selections, crop and
    /// mask overlays, guides, handles and transform gizmos. Arrow keys nudge in screen space.
    Canvas,
    /// Photos, previews and thumbnail contents, with marks pinned to their corners (flags,
    /// ratings, badges, Before and After labels). A grid of thumbnails follows reading order.
    Image,
    /// Rulers: units count from the document's origin.
    Ruler,
    /// Timelines, tracks, waveforms, keyframe lanes and playheads: time runs left to right.
    Timeline,
    /// Tone curves, histograms, graphs and plots: their axes are mathematical.
    Graph,
    /// Gradients, colour ramps, hue strips, colour wheels and two-dimensional colour pickers.
    ColourRamp,
    /// Sliders: the value increases to the right, and the value field sits on the same side.
    SliderValue,
    /// Play, pause, step, skip and rewind: their order and arrows, which match the timeline.
    MediaTransport,
    /// Code, expressions, file paths and URLs: left-to-right text, aligned left.
    Code,
    /// Number fields with their units, coordinates and sizes: digits run left to right, and x
    /// comes before y.
    Number,
    /// Timecodes, durations and frame counters.
    Timecode,
}

impl Unmirrored {
    pub const ALL: [Unmirrored; 11] = [
        Unmirrored::Canvas,
        Unmirrored::Image,
        Unmirrored::Ruler,
        Unmirrored::Timeline,
        Unmirrored::Graph,
        Unmirrored::ColourRamp,
        Unmirrored::SliderValue,
        Unmirrored::MediaTransport,
        Unmirrored::Code,
        Unmirrored::Number,
        Unmirrored::Timecode,
    ];
}

#[cfg(test)]
mod tests;
