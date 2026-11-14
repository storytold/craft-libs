//! Direction-aware layout helpers: bodies, forms, grids and hand-painted geometry.
//!
//! These mirror what egui's own `Style::right_to_left` flag does not: row and column order of
//! explicit layouts, dialog and window body scopes, settings grids, and painted positions. The
//! [`row`]/[`trailing`] family lives in [`super`]; this module adds the body, form and geometry
//! layers on top. Every helper reads the direction from a [`HasDirection`] (a [`Context`], a
//! [`Ui`], or a [`Direction`] itself); left to right every helper is a byte-identical passthrough
//! to plain egui, and right to left the spacing a row shows is exactly the left-to-right spacing.
//!
//! egui's `Grid` is direction-blind, so a grid must never sit directly in a right-to-left row
//! (its first cell anchors at the raw cursor): wrap it in [`ltr`]/[`ltr_vertical`]/[`ltr_grid`]
//! and mirror the cell order with [`grid_cells`], or use [`settings_grid`]/[`form_grid`], which
//! never put a grid in a mirrored row.

use std::cell::Cell;
use std::sync::Arc;

use egui::{
    Align, Align2, Area, AsIdSalt, Color32, Context, FontId, FontSelection, Galley, Grid, Id, InnerResponse, Layout, Painter, PopupAnchor, Pos2,
    Rect, Response, Ui, UiBuilder, Vec2, WidgetText, pos2, vec2,
};

use super::{Direction, HasDirection, apply, is_rtl, row, trailing};
// Method resolution picks egui's inherent `set_right_to_left` on the fork; the trait import keeps
// `sizing_pass_ltr` building against crates.io egui.
#[allow(unused_imports, reason = "unused when egui has the inherent method")]
use super::SetRightToLeft as _;

/// Run `add` with `ctx` drawn left to right, restoring the previous direction after. The sizing
/// pass of a mirrored host measures the tight left-to-right width, so the reading-end padding of
/// the first visible frame equals its start padding.
fn with_ltr<R>(ctx: &Context, add: impl FnOnce() -> R) -> R {
    let prev = ctx.direction();
    if prev == Direction::Ltr {
        return add();
    }
    apply(ctx, Direction::Ltr);
    let r = add();
    apply(ctx, prev);
    r
}

// Bodies and rows.

/// The body layout for dialogs and window bodies: right to left, blocks align to the reading
/// start and nested `ui.horizontal` rows mirror; left to right it is egui's default.
///
/// egui mirrors a nested row only when the parent layout runs right to left, or is vertical with
/// `Align::Max` cross alignment; a `top_down(Align::Min)` body keeps every nested row left to
/// right even with the direction flag set, so mirrored bodies scope themselves with this.
pub fn body_layout(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::top_down(Align::Max) } else { Layout::top_down(Align::Min) }
}

/// A row hugging the reading end with top-aligned items: the footer shape for window and dialog
/// bodies ([`trailing`] centres its items).
pub fn trailing_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::left_to_right(Align::Min) } else { Layout::right_to_left(Align::Min) }
}

/// [`row`] with top-aligned items, for rows inside a vertical body.
fn row_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) }
}

/// [`egui::Ui::horizontal_centered`] in reading order: a row filling the space left, starting
/// from the right in a right-to-left interface.
pub fn horizontal_centered<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = ui.available_size_before_wrap();
    ui.allocate_ui_with_layout(size, row(ui), add)
}

/// A full-width row in reading order for bodies that stay `top_down(Align::Min)`:
/// [`horizontal_centered`] without the vertical centring. The row allocates one interact height,
/// like `ui.horizontal`: a full-remaining-height frame would let a nested center-aligned scope
/// fill the window's whole remaining height, and a non-resizable window then sizes itself to the
/// screen height.
pub fn dialog_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(row_width(ui), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row_min(ui), add)
}

/// A full-width, interact-height row in reading order ([`row`]): the pinned-height reading-row
/// shape.
pub fn full_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row(ui), add)
}

/// [`full_row`] starting from the reading end ([`trailing`]).
pub fn full_trailing<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(ui.available_size_before_wrap().x, ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, trailing(ui), add)
}

/// Wrap a floating window's (or dialog's) content: right to left, the body's blocks align to the
/// reading start and its nested rows mirror; left to right the content draws unchanged. Call once
/// at the top of each window body closure. As in [`popup_body`], the sizing pass runs in a
/// left-to-right scope: under the mirrored [`body_layout`] the fork right-anchors every child
/// frame and grows the measured `min_rect` leftward, so width-reading rows balloon the window and
/// the slack lands on the reading-end side.
pub fn window_body<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let ctx = ui.ctx().clone();
            let inner = with_ltr(&ctx, || add(ui));
            InnerResponse::new(inner, ui.response())
        } else {
            ui.with_layout(body_layout(ui), add)
        }
    } else {
        let inner = add(ui);
        InnerResponse::new(inner, ui.response())
    }
}

/// The body of a modal dialog, bounded to the dialog's content rect: right to left the body
/// re-scopes with [`body_layout`], but the child is sized from the cursor — never seeded with the
/// parent's full `max_rect`, which under right-to-left re-anchors the title and the content onto
/// one baseline. As in [`popup_body`], the sizing pass runs in a left-to-right scope. Left to
/// right the caller's own layout runs unchanged.
pub fn dialog_content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let ctx = ui.ctx().clone();
            with_ltr(&ctx, || add(ui))
        } else {
            ui.with_layout(body_layout(ui), add).inner
        }
    } else {
        add(ui)
    }
}

/// The body of an open menu popup. egui pins popup bodies left to right, so right to left every
/// popup body scopes itself through this helper. The cross align stays `Min`: rows fill the
/// width (justified), and a `Max` align would seed the scope's `min_rect` at the far edge of the
/// pinned `max_rect`, so the sizing pass would measure the popup at the full default area width
/// instead of the rows' width. The sizing pass itself runs in a left-to-right scope. Left to
/// right the popup's own layout is used unchanged, sizing pass included.
pub fn popup_body<R>(ui: &mut Ui, add_contents: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let ctx = ui.ctx().clone();
            with_ltr(&ctx, || add_contents(ui))
        } else {
            ui.scope_builder(UiBuilder::new().max_rect(ui.max_rect()).layout(Layout::top_down_justified(Align::Min)), add_contents).inner
        }
    } else {
        add_contents(ui)
    }
}

/// A resizable host's contents (a modal dialog body): right to left, its sizing pass runs left to
/// right — the context's direction, the fork's direction flag on this `Ui` and the body layout —
/// so the fork measures the content's natural minimum left-anchored exactly as left to right.
/// Non-sizing frames and left-to-right pass through untouched.
pub fn sizing_pass_ltr<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) && ui.is_sizing_pass() {
        let ctx = ui.ctx().clone();
        with_ltr(&ctx, || {
            ui.style_mut().set_right_to_left(false);
            let r = ui.with_layout(Layout::top_down(Align::Min), add).inner;
            ui.style_mut().set_right_to_left(true);
            r
        })
    } else {
        add(ui)
    }
}

/// A label inside a mirrored body ([`body_layout`]/[`window_body`]): right to left, the paragraph
/// base direction is pinned right to left and the galley aligns to the reading-start edge, so
/// mixed Arabic/Latin lines keep their phrase order and hug the reading-start margin. Left to
/// right this is egui's plain label, unchanged.
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

// Left-to-right scopes.

/// Run a block inside a left-to-right scope even in a right-to-left interface. `egui::Grid`
/// places its first cell at the raw cursor, which is `x = -inf` at the start of a right-to-left
/// row, so a grid must never sit directly in a mirrored row; wrap it in this and mirror the cell
/// order with [`grid_cells`].
pub fn ltr<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.with_layout(Layout::left_to_right(Align::Min), add)
}

/// [`ltr`] for a block in a vertical flow: keeps the top-down direction (and so the parent's
/// cursor advance) but keeps nested rows left to right, which a mirrored body
/// (`top_down(Align::Max)`) would not. Wrap direction-blind widgets such as `egui::Grid` in this.
pub fn ltr_vertical<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ui.with_layout(Layout::top_down(Align::Min), add)
}

/// An `egui::Grid` in an [`ltr_vertical`] scope so the direction-blind placer never sees a
/// right-to-left cursor; cell order inside a row stays the caller's (mirror with [`grid_cells`]).
pub fn ltr_grid<R>(ui: &mut Ui, id: impl AsIdSalt, cols: usize, spacing: Vec2, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    ltr_vertical(ui, |ui| Grid::new(id).num_columns(cols).spacing(spacing).show(ui, add)).inner
}

/// Two `egui::Grid` cells in reading order — label then editor left to right, editor then label
/// right to left. `egui::Grid` ignores the direction flag, so the column order is emitted
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

// Forms and grids.

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

/// The shared body of [`form_grid`] and [`settings_grid`]; see [`settings_grid`] for the
/// right-to-left geometry. `rows` calls `row(caption, editor)` once per row.
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
            let mut row_fn = |caption: WidgetText, editor: &mut dyn FnMut(&mut Ui, Id)| {
                if caption.text().is_empty() {
                    dialog_row(ui, |ui| editor(ui, Id::NULL));
                } else {
                    let caption_id = Cell::new(Id::NULL);
                    settings_row(ui, label_w, |ui| caption_id.set(ui.label(caption).id), |ui| editor(ui, caption_id.get()));
                }
            };
            rows(&mut row_fn);
        });
    } else {
        let mut grid = Grid::new(id).num_columns(2).spacing(spacing);
        if let Some(w) = min_col_width {
            grid = grid.min_col_width(w);
        }
        grid.show(ui, |ui| {
            let mut row_fn = |caption: WidgetText, editor: &mut dyn FnMut(&mut Ui, Id)| {
                let caption_id = ui.label(caption).id;
                editor(ui, caption_id);
                ui.end_row();
            };
            rows(&mut row_fn);
        });
    }
}

/// A two-column caption/editor form. Left to right: `egui::Grid::new(id).num_columns(2).spacing(spacing)`.
/// Right to left: hand rows via [`settings_row`] under [`body_layout`] (the fork's `Grid` cannot
/// run under a mirrored body). `rows` calls `row(caption, editor)` once per row; the editor
/// closure receives the caption label's Id for accessibility labeling. An empty caption draws a
/// bare editor hugging the reading start.
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

/// A two-column settings grid of label/editor rows, direction-aware. Left to right: a plain
/// `egui::Grid` (byte-identical layout). Right to left: hand rows via [`settings_row`] under
/// [`body_layout`]: the fork's `Grid` gives its last column the remaining width, which detaches a
/// right-aligned label from its editor in a mirrored grid, while the hand rows keep the label
/// column fixed at `label_w` hugging the reading start, so the label-to-editor gap equals the
/// left-to-right formula (`label_w - label_width + spacing.x`) in both directions. Pass `label_w`
/// as the left-to-right label column width (the widest label, or `min_col_width` where the grid
/// sets one). `rows` calls `row(label, editor)` once per row; the editor closure receives the
/// label's Id for accessibility labeling. An empty label draws a bare editor hugging the reading
/// start.
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

/// A grid inside a scroll area, in an [`ltr_vertical`] child sized from the cursor: the
/// direction-aware replacement for `ui.new_child(UiBuilder::new().max_rect(ui.max_rect()))` +
/// `egui::Grid` + `advance_cursor_after_rect`. Seeding the child with the parent's full `max_rect`
/// re-anchors the grid's rows against the whole scroll content under right-to-left (cells land on
/// wrong baselines and past the window edge); sizing the child from the cursor keeps every row at
/// the cursor, and the parent cursor advances from the child's actual [`Ui::min_rect`] in both
/// directions. Cell order inside a row stays the caller's — mirror it with [`grid_cells`]. Left
/// to right the grid runs exactly as the raw pattern.
pub fn scroll_grid<R>(ui: &mut Ui, id: impl AsIdSalt, cols: usize, spacing: Vec2, add: impl FnOnce(&mut Ui) -> R) -> R {
    let max_rect = Rect::from_min_size(ui.cursor().min, vec2(row_width(ui), ui.available_height()));
    let mut child = ui.new_child(UiBuilder::new().max_rect(max_rect).layout(Layout::top_down(Align::Min)));
    let inner = Grid::new(id).num_columns(cols).spacing(spacing).show(&mut child, add).inner;
    ui.advance_cursor_after_rect(child.min_rect());
    inner
}

/// The usable row width: right to left the fork's available-width read balloons, so read
/// `ui.max_rect().width()`; left to right keep `ui.available_size_before_wrap().x`.
pub fn row_width(ui: &Ui) -> f32 {
    if is_rtl(ui) { ui.max_rect().width() } else { ui.available_size_before_wrap().x }
}

/// A full-width centred row inside a horizontal scroll area. Right to left: the fork anchors
/// scroll content at the viewport's left edge and clamps the offset at zero, which would leave a
/// mirrored row's trailing end cropped off-screen, so the row's region is mirrored about the
/// viewport's left edge and the strip then anchors at the reading-start edge. Left to right: plain
/// [`horizontal_centered`].
pub fn hscroll_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    if is_rtl(ui) {
        let offset = (ui.clip_rect().min.x - ui.max_rect().min.x).max(0.0);
        let rect = ui.max_rect().translate(vec2(2.0 * offset, 0.0));
        ui.scope_builder(UiBuilder::new().max_rect(rect), |ui| horizontal_centered(ui, add)).inner
    } else {
        horizontal_centered(ui, add)
    }
}

// Hand-painted geometry.

/// `1.0`, or `-1.0` right to left: the signed x-step for hand-painted row geometry (eye column,
/// thumbnails, indicators), which the egui flag never mirrors.
pub fn step(d: &impl HasDirection) -> f32 {
    if is_rtl(d) { -1.0 } else { 1.0 }
}

/// The reading-start x for content offset `off` from a rect's start edge: `rect.left() + off`
/// left to right, `rect.right() - off` right to left.
pub fn start_x(d: &impl HasDirection, rect: Rect, off: f32) -> f32 {
    if is_rtl(d) { rect.right() - off } else { rect.left() + off }
}

/// The min-x of a `w`-wide element hugging `rect`'s reading-start edge with margin `off` (an eye
/// icon or thumbnail column built with `Rect::from_min_size`): `rect.left() + off` left to right,
/// `rect.right() - off - w` right to left, so the element grows inward from the start edge.
pub fn start_x_w(d: &impl HasDirection, rect: Rect, off: f32, w: f32) -> f32 {
    if is_rtl(d) { rect.right() - off - w } else { rect.left() + off }
}

/// The reading-end x for content offset `off` from a rect's end edge: `rect.right() - off` left to
/// right, `rect.left() + off` right to left (the scrollbar side of a mirrored row).
pub fn end_x(d: &impl HasDirection, rect: Rect, off: f32) -> f32 {
    if is_rtl(d) { rect.left() + off } else { rect.right() - off }
}

/// The x for a `w`-wide painted element placed `off` beyond `rect` in reading order (a label after
/// a thumbnail): `rect.right() + off` left to right, `rect.left() - off - w` right to left. With an
/// alignment anchor instead of a galley, pass `w = 0.0` and [`start_align`].
pub fn after_x(d: &impl HasDirection, rect: Rect, off: f32, w: f32) -> f32 {
    if is_rtl(d) { rect.left() - off - w } else { rect.right() + off }
}

/// A `size` rect whose reading-start-top corner sits `off` from `rect`'s reading-start-top corner.
pub fn start_rect(d: &impl HasDirection, rect: Rect, off: Vec2, size: Vec2) -> Rect {
    if is_rtl(d) {
        Rect::from_min_size(pos2(rect.right() - off.x - size.x, rect.top() + off.y), size)
    } else {
        Rect::from_min_size(rect.left_top() + off, size)
    }
}

/// A full-height, `w`-wide cell at `rect`'s reading-start edge, clamped to `rect`'s width.
pub fn start_cell(d: &impl HasDirection, rect: Rect, w: f32) -> Rect {
    let w = w.min(rect.width());
    Rect::from_min_size(pos2(start_x_w(d, rect, 0.0, w), rect.top()), vec2(w, rect.height()))
}

/// The walking-cursor step of a hand-painted row: `x + step() * w`.
pub fn advance(d: &impl HasDirection, x: f32, w: f32) -> f32 {
    x + step(d) * w
}

/// A rect of width `w` whose reading-start edge sits at `x` (it grows against [`step`]).
pub fn start_span(d: &impl HasDirection, x: f32, w: f32, top: f32, bottom: f32) -> Rect {
    if is_rtl(d) { Rect::from_min_max(pos2(x - w, top), pos2(x, bottom)) } else { Rect::from_min_max(pos2(x, top), pos2(x + w, bottom)) }
}

/// The (anchor, limit) span a name is painted in, after `prev` inside `container`:
/// left to right `(prev.right() + pad, container.right() - edge_pad)`, mirrored right to left.
pub fn name_span(d: &impl HasDirection, prev: Rect, container: Rect, pad: f32, edge_pad: f32) -> (f32, f32) {
    if is_rtl(d) { (prev.left() - pad, container.left() + edge_pad) } else { (prev.right() + pad, container.right() - edge_pad) }
}

/// The reading-start edge of `rect` (its right edge right to left, else its left).
pub fn start_edge(d: &impl HasDirection, rect: Rect) -> f32 {
    if is_rtl(d) { rect.right() } else { rect.left() }
}

/// Fold `rects` into the running reading-end edge of a row: right to left folds the min of the
/// lefts, left to right the max of the rights, starting from `start`.
pub fn row_end_edge(d: &impl HasDirection, rects: impl IntoIterator<Item = Rect>, start: f32) -> f32 {
    if is_rtl(d) { rects.into_iter().map(|r| r.left()).fold(start, f32::min) } else { rects.into_iter().map(|r| r.right()).fold(start, f32::max) }
}

/// `Align2` for [`Painter::text`] hugging the reading-start edge of a row: `LEFT_CENTER` left to
/// right, `RIGHT_CENTER` right to left.
pub fn start_align(d: &impl HasDirection) -> Align2 {
    if is_rtl(d) { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }
}

/// `Align2` for [`Painter::text`] hugging the reading-end edge of a row (a shortcut hint):
/// `RIGHT_CENTER` left to right, `LEFT_CENTER` right to left.
pub fn end_align(d: &impl HasDirection) -> Align2 {
    if is_rtl(d) { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER }
}

/// Is `p` on the reading-start side of the vertical edge `edge` (a mirrored hit test)?
pub fn start_side(d: &impl HasDirection, p: f32, edge: f32) -> bool {
    if is_rtl(d) { p > edge } else { p < edge }
}

/// A directional glyph's x mirrored around `cx` in a right-to-left interface (disclosure
/// triangles), unchanged left to right.
pub fn glyph_x(d: &impl HasDirection, x: f32, cx: f32) -> f32 {
    if is_rtl(d) { 2.0 * cx - x } else { x }
}

/// (pos, align) for [`Painter::text`] hugging `rect`'s reading-start vertical centre, inset `off`.
pub fn start_text_anchor(d: &impl HasDirection, rect: Rect, off: f32) -> (Pos2, Align2) {
    if is_rtl(d) {
        (pos2(rect.right() - off, rect.center().y), Align2::RIGHT_CENTER)
    } else {
        (pos2(rect.left() + off, rect.center().y), Align2::LEFT_CENTER)
    }
}

/// Same at the reading-end edge.
pub fn end_text_anchor(d: &impl HasDirection, rect: Rect, off: f32) -> (Pos2, Align2) {
    if is_rtl(d) {
        (pos2(rect.left() + off, rect.center().y), Align2::LEFT_CENTER)
    } else {
        (pos2(rect.right() - off, rect.center().y), Align2::RIGHT_CENTER)
    }
}

/// Split `bar` into `(rest, end_strip)`: a `w`-wide strip at its reading-end edge, clamped into
/// `bar`.
pub fn split_end(d: &impl HasDirection, bar: Rect, w: f32) -> (Rect, Rect) {
    if is_rtl(d) {
        let split = (bar.left() + w).min(bar.right());
        (Rect::from_min_max(pos2(split, bar.top()), bar.max), Rect::from_min_max(bar.min, pos2(split, bar.bottom())))
    } else {
        let split = (bar.right() - w).max(bar.left());
        (Rect::from_min_max(bar.min, pos2(split, bar.bottom())), Rect::from_min_max(pos2(split, bar.top()), bar.max))
    }
}

/// A rect of width `w` whose reading-end edge sits at `x` (it grows against [`step`]).
pub fn end_span(d: &impl HasDirection, x: f32, w: f32, top: f32, bottom: f32) -> Rect {
    if is_rtl(d) { Rect::from_min_max(pos2(x - w, top), pos2(x, bottom)) } else { Rect::from_min_max(pos2(x, top), pos2(x + w, bottom)) }
}

/// The (anchor, offset) pair for an [`Area`] pinned to the screen's reading-end corner:
/// bottom-right left to right, bottom-left right to left (`bottom = false` picks the top pair); a
/// positive `x_off` moves inward from that edge.
pub fn end_corner_anchor(d: &impl HasDirection, bottom: bool, x_off: f32, y_off: f32) -> (Align2, Vec2) {
    if is_rtl(d) {
        let anchor = if bottom { Align2::LEFT_BOTTOM } else { Align2::LEFT_TOP };
        (anchor, vec2(x_off, y_off))
    } else {
        let anchor = if bottom { Align2::RIGHT_BOTTOM } else { Align2::RIGHT_TOP };
        (anchor, vec2(-x_off, y_off))
    }
}

/// The base direction of a text line in the interface's direction: right to left in a
/// right-to-left interface, left to right otherwise.
pub fn text_direction(d: &impl HasDirection) -> egui::epaint::text::TextDirection {
    if is_rtl(d) { egui::epaint::text::TextDirection::RightToLeft } else { egui::epaint::text::TextDirection::LeftToRight }
}

// Painted text.

/// A right-to-left `text` cut at its logical end with "…" where it doesn't fit (`fits` measures a
/// candidate). The start of the name stays, and egui puts the "…" at the left, the end of a
/// right-to-left line. `None`: `text` fits as it is (or a left-to-right one, which the caller
/// elides at its right end as usual).
fn elide_rtl(text: &str, fits: impl Fn(&str) -> bool) -> Option<String> {
    if craft_i18n::base_direction(text) != Some(Direction::Rtl) || fits(text) {
        return None;
    }
    // The longest logical prefix (whole characters) that fits with "…" after it.
    let ends: Vec<usize> = text.char_indices().map(|(i, _)| i).skip(1).chain(std::iter::once(text.len())).collect();
    let cut = |n: usize| -> String {
        let end = if n == 0 { 0 } else { ends.get(n - 1).copied().unwrap_or(text.len()) };
        format!("{}…", text.get(..end).unwrap_or_default().trim_end())
    };
    let (mut lo, mut hi) = (0usize, ends.len());
    while lo < hi {
        let mid = (lo + hi).div_ceil(2);
        if fits(&cut(mid)) { lo = mid } else { hi = mid - 1 }
    }
    Some(cut(lo))
}

/// Single-line galley elided with "…" to `max_width` (nothing when there's no room).
pub fn elided_galley(
    painter: &Painter,
    d: &impl HasDirection,
    text: &str,
    font: FontId,
    color: Color32,
    italics: bool,
    max_width: f32,
) -> Option<Arc<Galley>> {
    if max_width < 8.0 {
        return None;
    }
    let format = egui::text::TextFormat { font_id: font, color, italics, ..Default::default() };
    let layout = |s: &str, wrap: bool| {
        let mut job = egui::text::LayoutJob::default();
        job.append(s, 0.0, format.clone());
        // Base the paragraph on the interface's direction: a right-to-left name in a left-to-right
        // interface then elides with "…" at the visual right, not at its own reading end.
        job.direction = text_direction(d);
        if wrap {
            job.wrap = egui::text::TextWrapping { max_width, max_rows: 1, break_anywhere: true, overflow_character: Some('…') };
        }
        painter.layout_job(job)
    };
    // A right-to-left name too long for the row is cut at its logical end, so its start stays and
    // "…" is at the left.
    if let Some(shown) = elide_rtl(text, |s| layout(s, false).size().x <= max_width) {
        return Some(layout(&shown, false));
    }
    // egui puts a right-to-left name into display order itself; left-to-right names are cut at
    // their right end by egui.
    Some(layout(text, true))
}

/// Paint `text` in the span anchor→limit at height `cy`, elided to fit, aligned into the reading
/// direction (RIGHT_CENTER at the anchor right to left, LEFT_CENTER left to right). A span whose
/// anchor sits past its limit left to right (an unconverted caller) keeps the left-to-right
/// contract. Returns the painted rect.
#[allow(clippy::too_many_arguments)]
pub fn paint_label_span(
    painter: &Painter,
    d: &impl HasDirection,
    anchor: f32,
    cy: f32,
    limit: f32,
    text: &str,
    font: FontId,
    color: Color32,
) -> Option<Rect> {
    if is_rtl(d) && anchor > limit {
        let g = elided_galley(painter, d, text, font, color, false, anchor - limit)?;
        let r = Align2::RIGHT_CENTER.anchor_size(pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    } else {
        let g = elided_galley(painter, d, text, font, color, false, limit - anchor)?;
        let r = Align2::LEFT_CENTER.anchor_size(pos2(anchor, cy), g.size());
        painter.galley(r.min, g, color);
        Some(r)
    }
}

/// Paint a single-line elided label at the x-anchor `x` (a [`start_x`] reading-start edge),
/// vertically centred on `cy`; returns (painted rect, elided). Right to left, paints right-aligned
/// from the reading-start edge.
#[allow(clippy::too_many_arguments, reason = "a paint call takes its full geometry")]
pub fn paint_elided(
    painter: &Painter,
    d: &impl HasDirection,
    x: f32,
    cy: f32,
    max_w: f32,
    text: &str,
    font: FontId,
    color: Color32,
) -> Option<(Rect, bool)> {
    let mut job = egui::text::LayoutJob::default();
    job.append(text, 0.0, egui::text::TextFormat { font_id: font.clone(), color, ..Default::default() });
    let elided = painter.layout_job(job).size().x > max_w;
    let g = elided_galley(painter, d, text, font, color, false, max_w)?;
    let align = if is_rtl(d) { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER };
    let r = align.anchor_size(pos2(x, cy), g.size());
    painter.galley(r.min, g, color);
    Some((r, elided))
}

// Windows and popups.

/// First-show x of a floating panel window `width` wide, `inset` px inside the canvas's
/// reading-end edge, shifted `cascade` px further toward the reading end (cascading panels;
/// negative cascades run back toward the canvas centre). Clamped to keep 8 px of the window on
/// the canvas.
pub fn float_window_x(d: &impl HasDirection, canvas: Rect, width: f32, inset: f32, cascade: f32) -> f32 {
    if is_rtl(d) {
        (canvas.left() + inset - cascade).min(canvas.right() - 8.0 - width).max(canvas.left() + 8.0)
    } else {
        (canvas.right() - width - inset + cascade).max(canvas.left() + 8.0)
    }
}

/// Clamp a resizable window's rect to exactly `min`, keeping the edge the window is anchored on
/// fixed: right to left the fork anchors the window rect at its right edge (resize grip on the
/// left), so the clamped rect grows leftward from a fixed right edge; left to right it keeps the
/// fixed left edge. The top edge is fixed in both directions. The returned rect's size is exactly
/// `min` — never the input rect's current size, so shrink drags are honoured.
pub fn clamp_window_min_size(d: &impl HasDirection, rect: Rect, min: Vec2) -> Rect {
    if is_rtl(d) { Rect::from_min_size(pos2(rect.right() - min.x, rect.top()), min) } else { Rect::from_min_size(rect.min, min) }
}

/// An [`Area`] hanging off `anchor`'s reading-end edge, `side_offset` px toward the canvas.
pub fn popup_area_near(ui: &Ui, id: impl AsIdSalt, anchor: Rect, side_offset: f32) -> Area {
    let id = ui.scope_id().with(id);
    if is_rtl(ui) {
        Area::new(id).pivot(Align2::RIGHT_TOP).fixed_pos(anchor.left_top() - vec2(side_offset, 0.0))
    } else {
        Area::new(id).pivot(Align2::LEFT_TOP).fixed_pos(anchor.right_top() + vec2(side_offset, 0.0))
    }
}

/// The [`PopupAnchor`] of `resp` translated `offset` px past its reading-end edge
/// (`ParentRect` moved by `(step() * offset, 0)`; the other variants unchanged).
pub fn popup_anchor_reading_end(d: &impl HasDirection, resp: &Response, offset: f32) -> PopupAnchor {
    match PopupAnchor::from(resp) {
        PopupAnchor::ParentRect(rect) => PopupAnchor::ParentRect(rect.translate(vec2(step(d) * offset, 0.0))),
        other => other,
    }
}
