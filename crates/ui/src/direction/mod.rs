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
//! # Bodies, popups and dialogs
//!
//! The style flag alone leaves every nested row of a plain `top_down(Align::Min)` body left to
//! right: egui mirrors a nested row only when the parent layout runs right to left, or is vertical
//! with `Align::Max` cross align. Mirrored bodies therefore scope themselves with [`body_layout`]
//! (vertical + `Max`), wrapped by [`window_body`] for floating windows and [`dialog_content`] for
//! modals; [`popup_body`] re-scopes menu popup bodies, which egui pins left to right. Resizable
//! hosts wrap their contents in [`sizing_pass_ltr`] so the sizing pass measures left-to-right and
//! the enforced minimum size is direction-independent.
//!
//! Mirror each surface exactly once: the flag, a [`body_layout`] scope and an explicit [`row`] or
//! [`trailing`] pin each flip a row once — never combine two on the same row, or it double-flips.
//!
//! # Hand-painted geometry and grids
//!
//! [`geometry`] expresses painted positions from reading edges, and [`paint`] elides and paints
//! labels into the reading direction. `egui::Grid` is direction-blind, so forms go through
//! [`settings_grid`]/[`form_grid`], grids through [`ltr_grid`]/[`scroll_grid`] with cell order from
//! [`grid_cells`].
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

use std::cell::Cell;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::Arc;

use egui::{
    Align, Align2, AsIdSalt, Context, FontSelection, Grid, Id, InnerResponse, Key, Layout, Panel, Rect, RectAlign, Response, Ui, UiBuilder,
    WidgetText, pos2, vec2,
};

pub use craft_i18n::Direction;

mod geometry;
mod paint;

pub use geometry::*;
pub use paint::*;

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

fn store(ctx: &Context, dir: Direction) {
    ctx.data_mut(|d| d.insert_temp(key(), dir));
}

/// Draw the interface on `ctx` in `dir` from the next layout on.
pub fn apply(ctx: &Context, dir: Direction) {
    store(ctx, dir);
    ctx.all_styles_mut(|style| style.set_right_to_left(dir == Direction::Rtl));
}

thread_local! {
    // Per thread, like the drawing language: test harnesses run side by side.
    static FORCED: Cell<Option<Direction>> = const { Cell::new(None) };
}

fn forced() -> Option<Direction> {
    FORCED.try_with(|c| c.get()).ok().flatten()
}

fn set_forced(dir: Option<Direction>) {
    let _ = FORCED.try_with(|c| c.set(dir));
}

/// Draw in `dir` on this thread whatever the context holds, until the guard drops. Reads beat the
/// stored context direction ([`HasDirection`]), so this forces one frame or one widget; nested
/// scopes restore correctly. `!Send` by construction.
#[must_use]
pub fn direction_scope(dir: Direction) -> impl Drop {
    struct Restore {
        previous: Option<Direction>,
        _not_send: PhantomData<Rc<()>>,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            set_forced(self.previous);
        }
    }
    let previous = forced();
    set_forced(Some(dir));
    Restore { previous, _not_send: PhantomData }
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
    /// A [`direction_scope`] force on this thread first, then the direction last [`apply`]-ed to
    /// this context; left to right before that.
    fn direction(&self) -> Direction {
        forced().unwrap_or_else(|| self.data(|d| d.get_temp(key())).unwrap_or(Direction::Ltr))
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

// Bodies, popups and dialogs.

/// The body layout for dialog and window bodies: in a right-to-left UI blocks right-align and their
/// nested `ui.horizontal` rows mirror; in a left-to-right UI it is egui's default.
///
/// egui mirrors a nested row only when the parent layout runs right-to-left, or is vertical with
/// `Align::Max` cross alignment; a `top_down(Align::Min)` body keeps every nested row left-to-right
/// even with the direction flag set, so mirrored bodies scope themselves with this. Never combine
/// it with the flag or an explicit [`row`]/[`trailing`] pin on the same row: each flips a row once,
/// two on one row double-flip.
pub fn body_layout(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::top_down(Align::Max) } else { Layout::top_down(Align::Min) }
}

/// A row hugging the reading end with top-aligned items: the footer shape for window and dialog
/// bodies ([`trailing`] centres its items).
pub fn trailing_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::left_to_right(Align::Min) } else { Layout::right_to_left(Align::Min) }
}

/// The body of an open menu popup. egui pins popup bodies left-to-right whatever the direction
/// flag, so in a right-to-left UI every popup body scopes itself through this helper. The cross
/// align stays `Min`: rows fill the width (justified), and a `Max` align would seed the scope's
/// `min_rect` at the far edge of the pinned `max_rect`, so the sizing pass would measure the popup
/// at the full default area width instead of the rows' width. The sizing pass itself runs in an LTR
/// scope ([`direction_scope`]): it measures against the wide default area, and rows anchored to
/// the reading end would measure — and lock in — that full width. Left-to-right the popup's own
/// layout is used unchanged, sizing pass included.
pub fn popup_body<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(Direction::Ltr);
            add_contents(ui)
        } else {
            ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect()).layout(Layout::top_down_justified(Align::Min)), add_contents).inner
        }
    } else {
        add_contents(ui)
    }
}

/// A full-width row in reading order with top-aligned items, for bodies that stay
/// `top_down(Align::Min)`. Rows inside a [`body_layout`] parent already mirror; opt in here only
/// where that scope does not reach.
///
/// The row allocates one interact height, like `ui.horizontal`: a full-remaining-height frame
/// would let a nested center-aligned scope fill the window's whole remaining height, and a
/// non-resizable window then sizes itself to the screen height. Taller content still grows the
/// row's `min_rect` past the frame.
pub fn dialog_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(row_width(ui), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row_min(ui), add)
}

/// [`row`] with top-aligned items, for rows inside a vertical body.
fn row_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) }
}

/// Wrap a floating window's (or dialog's) content. Call once at the top of each `egui::Window`
/// body closure: in a right-to-left UI the body's blocks right-align and its nested rows mirror via
/// [`body_layout`]; in a left-to-right UI the content draws unchanged. As in [`popup_body`], the
/// sizing pass runs in an LTR scope: under the mirrored body every child frame is right-anchored
/// and the measured `min_rect` grows leftward, so width-reading rows balloon the window.
pub fn window_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(Direction::Ltr);
            let inner = add(ui);
            InnerResponse::new(inner, ui.response())
        } else {
            ui.with_layout(body_layout(ui), add)
        }
    } else {
        let inner = add(ui);
        InnerResponse::new(inner, ui.response())
    }
}

/// The body of a modal dialog, bounded to the dialog's content rect: in a right-to-left UI the
/// body re-scopes with [`body_layout`] so its blocks right-align and its nested rows mirror, but
/// the child is sized from the cursor — never seeded with the parent's full `max_rect`, which
/// under RTL re-anchors the title and the content onto one baseline. As in [`popup_body`], the
/// sizing pass runs in an LTR scope so the modal measures its rows' width, not the full default
/// area width. Left-to-right the caller's own layout runs unchanged.
pub fn dialog_content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(Direction::Ltr);
            add(ui)
        } else {
            ui.with_layout(body_layout(ui), add).inner
        }
    } else {
        add(ui)
    }
}

/// A resizable host's contents (a modal dialog body): in a right-to-left UI its sizing pass runs
/// left-to-right — a [`direction_scope`] force, the direction flag cleared, and the body layout —
/// so the content's natural minimum is measured left-anchored exactly as in LTR. Under the
/// mirrored `top_down(Align::Max)` body the sizing-pass children right-anchor instead, which
/// ballooned the measured min width to the offered width and made a resizable window's enforced
/// minimum direction-dependent. Non-sizing frames and LTR run unchanged, and the flag and layout
/// are restored afterwards.
pub fn sizing_pass_ltr<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    let rtl = is_rtl(ui);
    if rtl && ui.is_sizing_pass() {
        let _ltr = direction_scope(Direction::Ltr);
        ui.style_mut().set_right_to_left(false);
        let r = ui.with_layout(Layout::top_down(Align::Min), add).inner;
        ui.style_mut().set_right_to_left(rtl);
        r
    } else {
        add(ui)
    }
}

// Direction-blind widgets and forms.

/// Run a block inside a left-to-right scope even in a right-to-left UI. `egui::Grid` places its
/// first cell at the raw cursor, which is `x = -inf` at the start of a right-to-left row, so a
/// grid must never sit directly in an RTL row; wrap it in this and mirror the cell order with
/// [`grid_cells`].
pub fn ltr<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.with_layout(Layout::left_to_right(Align::Min), add)
}

/// [`ltr`] for a block in a vertical flow: keeps the top-down direction (and so the parent's
/// cursor advance) but keeps nested rows left-to-right, which a mirrored body
/// (`top_down(Align::Max)`) would not. Wrap direction-blind widgets such as `egui::Grid` in this.
pub fn ltr_vertical<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.with_layout(Layout::top_down(Align::Min), add)
}

/// Two `egui::Grid` cells in reading order — label then editor left-to-right, editor then label
/// right-to-left. `egui::Grid` ignores the direction flag, so the column order is emitted
/// manually. Call once per row; end the row with `ui.end_row()` as usual.
pub fn grid_cells<L, E>(ui: &mut Ui, label: L, editor: E)
where
    L: FnOnce(&mut Ui),
    E: FnOnce(&mut Ui),
{
    if is_rtl(ui) {
        editor(ui);
        label(ui);
    } else {
        label(ui);
        editor(ui);
    }
}

/// One settings row without a `Grid`: the label occupies `label_w` at the reading start and the
/// editor follows toward the reading end — the mirrored shape of a two-column settings grid.
/// `label_w = 0.0` lets a bare checkbox hug the reading start.
pub fn settings_row<L, E>(ui: &mut Ui, label_w: f32, label: L, editor: E)
where
    L: FnOnce(&mut Ui),
    E: FnOnce(&mut Ui),
{
    let width = row_width(ui);
    ui.allocate_ui_with_layout(vec2(width, 0.0), row(ui), |ui| {
        ui.allocate_ui_with_layout(vec2(label_w, 0.0), row(ui), |ui| {
            label(ui);
            // A scope only advances the parent cursor past the child's used min_rect; claim the
            // whole label column so the editor keeps the gap `label_w - label_width + spacing.x`.
            let cell = ui.max_rect();
            ui.expand_to_include_rect(cell);
        });
        editor(ui);
        ui.add_space(7.0);
    });
}

/// A two-column caption/editor form. LTR: `egui::Grid::new(id).num_columns(2).spacing(spacing)`.
/// RTL: hand rows via [`settings_row`] under [`body_layout`] (the `Grid` cannot run under a
/// mirrored body). `rows` calls `row(caption, editor)` once per row; the editor closure receives
/// the caption label's Id for accessibility labeling. An empty caption draws a bare editor hugging
/// the reading start.
pub fn form_grid(
    ui: &mut Ui,
    id: impl AsIdSalt,
    spacing: [f32; 2],
    caption_w: f32,
    rows: impl FnOnce(&mut dyn FnMut(&str, &mut dyn FnMut(&mut Ui, Id))),
) {
    two_col_grid(ui, id, spacing, caption_w, None, |grid_rows| {
        let mut adapt = |caption: &str, editor: &mut dyn FnMut(&mut Ui, Id)| grid_rows(WidgetText::from(caption), editor);
        rows(&mut adapt);
    });
}

/// The shared body of [`form_grid`] and [`settings_grid`]; see [`settings_grid`] for the RTL
/// geometry. `rows` calls `row(caption, editor)` once per row.
fn two_col_grid(
    ui: &mut Ui,
    id: impl AsIdSalt,
    spacing: [f32; 2],
    caption_w: f32,
    min_col_width: Option<f32>,
    rows: impl FnOnce(&mut dyn FnMut(WidgetText, &mut dyn FnMut(&mut Ui, Id))),
) {
    if is_rtl(ui) {
        ui.with_layout(body_layout(ui), |ui| {
            ui.spacing_mut().item_spacing = vec2(spacing[0], spacing[1]);
            let label_w = caption_w.max(min_col_width.unwrap_or(0.0));
            let mut row = |caption: WidgetText, editor: &mut dyn FnMut(&mut Ui, Id)| {
                if caption.text().is_empty() {
                    dialog_row(ui, |ui| editor(ui, Id::NULL));
                } else {
                    let caption_id = Cell::new(Id::NULL);
                    settings_row(ui, label_w, |ui| caption_id.set(ui.label(caption).id), |ui| editor(ui, caption_id.get()));
                }
            };
            rows(&mut row);
        });
    } else {
        let mut grid = Grid::new(id).num_columns(2).spacing(spacing);
        if let Some(w) = min_col_width {
            grid = grid.min_col_width(w);
        }
        grid.show(ui, |ui| {
            let mut row = |caption: WidgetText, editor: &mut dyn FnMut(&mut Ui, Id)| {
                let caption_id = ui.label(caption).id;
                editor(ui, caption_id);
                ui.end_row();
            };
            rows(&mut row);
        });
    }
}

/// A two-column settings grid of label/editor rows, direction-aware. LTR: a plain `egui::Grid`
/// (byte-identical layout). RTL: hand rows via [`settings_row`] under [`body_layout`]: the `Grid`
/// gives its last column the remaining width, which detaches a right-aligned label from its
/// editor in a mirrored grid, while the hand rows keep the label column fixed at `label_w` hugging
/// the reading start, so the label↔editor gap equals the LTR formula
/// (`label_w - label_width + spacing.x`) in both directions. Pass `label_w` as the LTR label column
/// width (the widest label, or `min_col_width` where the grid sets one). `rows` calls
/// `row(label, editor)` once per row; the editor closure receives the label's Id for accessibility
/// labeling. An empty label draws a bare editor hugging the reading start.
pub fn settings_grid(
    ui: &mut Ui,
    id: impl AsIdSalt,
    spacing: [f32; 2],
    label_w: f32,
    min_col_width: Option<f32>,
    rows: impl FnOnce(&mut dyn FnMut(WidgetText, &mut dyn FnMut(&mut Ui, Id))),
) {
    two_col_grid(ui, id, spacing, label_w, min_col_width, rows)
}

/// An `egui::Grid` in an LTR scope ([`ltr_vertical`]) so the placer never sees an RTL cursor; cell
/// order inside a row stays the caller's (mirror with [`grid_cells`] or an explicit reversal).
pub fn ltr_grid<R>(ui: &mut Ui, id: impl AsIdSalt, cols: usize, spacing: egui::Vec2, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ltr_vertical(ui, |ui| Grid::new(id).num_columns(cols).spacing(spacing).show(ui, add)).inner
}

/// A grid inside a scroll area, in an [`ltr_vertical`] child sized from the cursor: the
/// direction-aware replacement for `ui.new_child(UiBuilder::new().max_rect(ui.max_rect()))` +
/// `egui::Grid` + `advance_cursor_after_rect`. Seeding the child with the parent's full `max_rect`
/// re-anchors the grid's rows against the whole scroll content under RTL (cells land on wrong
/// baselines and past the window edge); sizing the child from the cursor keeps every row at the
/// cursor, and the parent cursor advances from the child's actual [`Ui::min_rect`] in both
/// directions. Cell order inside a row stays the caller's — mirror it with [`grid_cells`].
/// Left-to-right the grid runs exactly as the raw pattern.
pub fn scroll_grid<R>(ui: &mut Ui, id: impl AsIdSalt, cols: usize, spacing: egui::Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    let max_rect = Rect::from_min_size(ui.cursor().min, vec2(row_width(ui), ui.available_height()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(max_rect).layout(Layout::top_down(Align::Min)));
    let inner = Grid::new(id).num_columns(cols).spacing(spacing).show(&mut child, add).inner;
    ui.advance_cursor_after_rect(child.min_rect());
    inner
}

/// The usable row width: in RTL the available-width read balloons, so read `ui.max_rect().width()`;
/// LTR keeps `ui.available_size_before_wrap().x`.
pub fn row_width(ui: &Ui) -> f32 {
    if is_rtl(ui) { ui.max_rect().width() } else { ui.available_size_before_wrap().x }
}

/// A full-width centred row inside a horizontal scroll area. RTL: the content is anchored at the
/// viewport's left edge and the offset clamped ≥ 0, which would leave a mirrored row's trailing
/// end cropped off-screen and unreachable, so the row's region is mirrored about the viewport's
/// left edge (`offset = (clip.min.x - max.min.x).max(0)`, region translated by `2 * offset`) and
/// the strip then anchors at the reading-start edge; LTR: a plain centred row in reading order.
pub fn hscroll_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let centered = |ui: &mut Ui| {
        let size = ui.available_size_before_wrap();
        ui.allocate_ui_with_layout(size, row(ui), add)
    };
    if is_rtl(ui) {
        let offset = (ui.clip_rect().min.x - ui.max_rect().min.x).max(0.0);
        let rect = ui.max_rect().translate(vec2(2.0 * offset, 0.0));
        ui.scope_builder(UiBuilder::new().max_rect(rect), centered).inner
    } else {
        centered(ui)
    }
}

/// A full-width, interact-height row in reading order ([`row`]): the pinned-height reading-row
/// shape. Same allocation as [`horizontal`].
pub fn full_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    horizontal(ui, add)
}

/// [`full_row`] starting from the reading end ([`trailing`]).
pub fn full_trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, trailing(ui), add)
}

/// A row pinned to the reading end only in a right-to-left UI; left-to-right the caller's own
/// layout runs unchanged.
pub fn trailing_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) { ui.with_layout(trailing(ui), add).inner } else { add(ui) }
}

/// The horizontal keys meaning "next on screen" and "previous on screen", as `(next, previous)`:
/// `(ArrowRight, ArrowLeft)` left-to-right, swapped right-to-left. One tuple for every key machine
/// that walks a bar (a menu bar's open-menu and bar-focus arms), so they cannot diverge.
pub fn horizontal_nav_keys(d: &impl HasDirection) -> (Key, Key) {
    (step_key(d, Step::Next), step_key(d, Step::Previous))
}

/// A label inside a mirrored body ([`body_layout`]/[`window_body`]): in a right-to-left UI the
/// paragraph base direction is pinned right-to-left and the galley aligns to the reading-start
/// (right) edge, so mixed Arabic/Latin lines keep their phrase order instead of scrambling and hug
/// the right margin. In a left-to-right UI this is egui's plain label, unchanged.
pub fn body_label(ui: &mut Ui, text: impl Into<WidgetText>) -> Response {
    let text = text.into();
    if !is_rtl(ui) {
        return ui.label(text);
    }
    let mut job = Arc::unwrap_or_clone(text.into_layout_job(ui.style(), FontSelection::Default, ui.text_valign()));
    job.direction = text_direction(ui);
    job.halign = Align::Max;
    ui.label(WidgetText::LayoutJob(Arc::new(job)))
}

/// The base direction of a text line in the UI's direction: right-to-left in a right-to-left
/// interface, left-to-right otherwise.
pub fn text_direction(d: &impl HasDirection) -> epaint::text::TextDirection {
    if is_rtl(d) { epaint::text::TextDirection::RightToLeft } else { epaint::text::TextDirection::LeftToRight }
}

// Hand-painted positions.

/// The horizontal alignment of the reading start: [`Align::Min`], or [`Align::Max`] right to left.
pub fn start_align(d: &impl HasDirection) -> Align {
    if is_rtl(d) { Align::Max } else { Align::Min }
}

/// `1.0`, or `-1.0` right to left: multiply an x offset written for left to right by it, as in
/// `start_x(ui, rect, 0.0) + forward(ui) * padding`.
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
