//! Dialog chrome: the modal body scope, full-width rows, the footer row and the title band's
//! close box and drag band.
//!
//! Mirror each surface exactly once: the direction flag, a [`body_layout`](super::scope) scope
//! and an explicit layout are three mechanisms over disjoint widgets — never compose two on one
//! row. Footer slot order stays logical (the caller's add order); mirroring comes from the row
//! direction, so Tab order is unchanged.

use egui::{Align, InnerResponse, Layout, Rect, Ui, pos2, vec2};

use super::{
    Direction, HasDirection, is_rtl,
    scope::{body_layout, direction_scope},
};

/// A row hugging the reading end with top-aligned items: the footer shape for dialog and window
/// bodies ([`super::trailing`] centres its items). A `Center` main align fills the height left
/// over from a larger previous frame, so a shrinking dialog would never shrink back — keep
/// `Align::Min`.
pub fn trailing_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::left_to_right(Align::Min) } else { Layout::right_to_left(Align::Min) }
}

/// The body of a modal dialog, bounded to the dialog's content rect: in a right-to-left
/// interface the body re-scopes with the mirrored body layout so its blocks right-align and its
/// nested rows mirror, but the child is sized from the cursor — never seeded with the parent's
/// full `max_rect`, which under RTL re-anchors the title and the content onto one baseline. The
/// sizing pass runs in an LTR scope so the modal measures its rows' width, not the full default
/// area width. Left to right the caller's own layout runs unchanged.
pub fn dialog_content<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> R {
    if is_rtl(ui) {
        if ui.is_sizing_pass() {
            let _ltr = direction_scope(ui.ctx(), Direction::Ltr);
            add(ui)
        } else {
            ui.with_layout(body_layout(ui), add).inner
        }
    } else {
        add(ui)
    }
}

/// A full-width reading-order row with bounded height, for bodies that stay
/// `top_down(Align::Min)`: [`super::horizontal`] without the vertical centring. The row
/// allocates one interact height, like `ui.horizontal` — a full-remaining-height frame would let
/// a nested center-aligned scope fill the window's whole remaining height and a non-resizable
/// window would then size itself to the screen height.
pub fn dialog_row<R>(ui: &mut Ui, add: impl FnOnce(&mut Ui) -> R) -> InnerResponse<R> {
    let size = vec2(row_width(ui), ui.spacing().interact_size.y);
    ui.allocate_ui_with_layout(size, row_min(ui), add)
}

/// [`super::row`] with top-aligned items, for rows inside a vertical body.
fn row_min(d: &impl HasDirection) -> Layout {
    if is_rtl(d) { Layout::right_to_left(Align::Min) } else { Layout::left_to_right(Align::Min) }
}

/// The usable row width: in RTL read `ui.max_rect().width()` (the available-width read balloons
/// to the offered width); LTR keeps egui's `available_size_before_wrap().x`.
fn row_width(ui: &Ui) -> f32 {
    if is_rtl(ui) { ui.max_rect().width() } else { ui.available_size_before_wrap().x }
}

/// The side of the title band's close box, in points.
pub const CLOSE_BOX_SIZE: f32 = 20.0;

/// The close box's square: flush with the title band's far end — the band's left end in a
/// right-to-left interface, its right end otherwise — centred on the title row at `centre_y`.
pub fn close_box_rect(d: &impl HasDirection, band_end: f32, centre_y: f32) -> Rect {
    let x = if is_rtl(d) { band_end } else { band_end - CLOSE_BOX_SIZE };
    Rect::from_min_size(pos2(x, centre_y - CLOSE_BOX_SIZE / 2.0), vec2(CLOSE_BOX_SIZE, CLOSE_BOX_SIZE))
}

/// The title band's drag area: the band minus the close box on the box's inner side, extended 7
/// px past the title on the other side, so a drag starting on the × neither closes nor moves
/// the dialog. `close` is the close box, or `None` on platforms without one (macOS traffic
/// lights).
pub fn title_drag_band(d: &impl HasDirection, band: Rect, title: Rect, close: Option<Rect>) -> Rect {
    if is_rtl(d) {
        let bar_left = close.map_or(band.left() - 7.0, |r| r.right());
        Rect::from_min_max(pos2(bar_left, title.min.y - 7.0), pos2(title.max.x + 7.0, title.bottom() + 4.0))
    } else {
        let bar_right = close.map_or(band.right() + 7.0, |r| r.left());
        Rect::from_min_max(title.min - vec2(7.0, 7.0), pos2(bar_right, title.bottom() + 4.0))
    }
}
