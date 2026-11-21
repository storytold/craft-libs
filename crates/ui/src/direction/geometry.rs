//! Hand-painted geometry: every x expressed from a rect's reading-start or reading-end edge,
//! never from its left or right. The egui direction flag never mirrors painted shapes, so these
//! helpers do it by hand.

use egui::{Align2, Pos2, Rect, Vec2, pos2, vec2};

use super::{HasDirection, forward, is_rtl};

/// The reading-start x for content offset `off` from `rect`'s start edge: `rect.left() + off`
/// left-to-right, `rect.right() - off` right-to-left.
pub fn start_x(d: &impl HasDirection, rect: Rect, off: f32) -> f32 {
    if is_rtl(d) { rect.right() - off } else { rect.left() + off }
}

/// The min-x of a `w`-wide element hugging `rect`'s reading-start edge with margin `off` (an eye
/// icon or thumbnail column built with [`Rect::from_min_size`]): `rect.left() + off` left-to-right,
/// `rect.right() - off - w` right-to-left, so the element grows inward from the start edge.
pub fn start_x_w(d: &impl HasDirection, rect: Rect, off: f32, w: f32) -> f32 {
    if is_rtl(d) { rect.right() - off - w } else { rect.left() + off }
}

/// The reading-end x for content offset `off` from `rect`'s end edge: `rect.right() - off`
/// left-to-right, `rect.left() + off` right-to-left (the scrollbar side of a mirrored row).
pub fn end_x(d: &impl HasDirection, rect: Rect, off: f32) -> f32 {
    if is_rtl(d) { rect.left() + off } else { rect.right() - off }
}

/// The x for a `w`-wide painted element placed `off` beyond `rect` in reading order (a label after
/// a thumbnail): `rect.right() + off` left-to-right, `rect.left() - off - w` right-to-left. With an
/// alignment anchor instead of a galley, pass `w = 0.0` and [`start_align2`].
pub fn after_x(d: &impl HasDirection, rect: Rect, off: f32, w: f32) -> f32 {
    if is_rtl(d) { rect.left() - off - w } else { rect.right() + off }
}

/// A `size` rect whose reading-start-top corner sits `off` from `rect`'s reading-start-top corner:
/// LTR min = `rect.left_top() + off`; RTL min = `(rect.right() - off.x - size.x, rect.top() + off.y)`.
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

/// The walking-cursor step of a hand-painted row: `x + forward(d) * w`.
pub fn advance(d: &impl HasDirection, x: f32, w: f32) -> f32 {
    x + forward(d) * w
}

/// A rect of width `w` whose reading-start edge sits at `x` (it grows against [`forward`]).
pub fn start_span(d: &impl HasDirection, x: f32, w: f32, top: f32, bottom: f32) -> Rect {
    if is_rtl(d) { Rect::from_min_max(pos2(x - w, top), pos2(x, bottom)) } else { Rect::from_min_max(pos2(x, top), pos2(x + w, bottom)) }
}

/// A rect of width `w` whose reading-end edge sits at `x` (it grows against [`forward`]):
/// `[x, x + w]` left-to-right, `[x - w, x]` right-to-left.
pub fn end_span(d: &impl HasDirection, x: f32, w: f32, top: f32, bottom: f32) -> Rect {
    if is_rtl(d) { Rect::from_min_max(pos2(x - w, top), pos2(x, bottom)) } else { Rect::from_min_max(pos2(x, top), pos2(x + w, bottom)) }
}

/// The (anchor, limit) span a name is painted in, after `prev` inside `container`:
/// LTR `(prev.right() + pad, container.right() - edge_pad)`; RTL `(prev.left() - pad, container.left() + edge_pad)`.
pub fn name_span(d: &impl HasDirection, prev: Rect, container: Rect, pad: f32, edge_pad: f32) -> (f32, f32) {
    if is_rtl(d) { (prev.left() - pad, container.left() + edge_pad) } else { (prev.right() + pad, container.right() - edge_pad) }
}

/// The reading-start edge of `rect` (its right edge in a right-to-left UI, else its left).
pub fn start_edge(d: &impl HasDirection, rect: Rect) -> f32 {
    if is_rtl(d) { rect.right() } else { rect.left() }
}

/// Fold `rects` into the running reading-end edge of a row: RTL folds the min of the lefts, LTR the
/// max of the rights, starting from `start`.
pub fn row_end_edge(d: &impl HasDirection, rects: impl IntoIterator<Item = Rect>, start: f32) -> f32 {
    if is_rtl(d) { rects.into_iter().map(|r| r.left()).fold(start, f32::min) } else { rects.into_iter().map(|r| r.right()).fold(start, f32::max) }
}

/// Split `bar` into `(rest, end_strip)`: a `w`-wide strip at its reading-end edge (right
/// left-to-right, left right-to-left), clamped into `bar`.
pub fn split_end(d: &impl HasDirection, bar: Rect, w: f32) -> (Rect, Rect) {
    if is_rtl(d) {
        let split = (bar.left() + w).min(bar.right());
        (Rect::from_min_max(pos2(split, bar.top()), bar.max), Rect::from_min_max(bar.min, pos2(split, bar.bottom())))
    } else {
        let split = (bar.right() - w).max(bar.left());
        (Rect::from_min_max(bar.min, pos2(split, bar.bottom())), Rect::from_min_max(pos2(split, bar.top()), bar.max))
    }
}

/// [`Align2`] for [`egui::Painter::text`] hugging the reading-start edge of a row: `LEFT_CENTER`
/// left-to-right, `RIGHT_CENTER` right-to-left.
pub fn start_align2(d: &impl HasDirection) -> Align2 {
    if is_rtl(d) { Align2::RIGHT_CENTER } else { Align2::LEFT_CENTER }
}

/// [`Align2`] for [`egui::Painter::text`] hugging the reading-end edge of a row (a shortcut hint):
/// `RIGHT_CENTER` left-to-right, `LEFT_CENTER` right-to-left.
pub fn end_align2(d: &impl HasDirection) -> Align2 {
    if is_rtl(d) { Align2::LEFT_CENTER } else { Align2::RIGHT_CENTER }
}

/// Is `p` on the reading-start side of the vertical edge `edge` (a mirrored hit test)?
pub fn start_side(d: &impl HasDirection, p: f32, edge: f32) -> bool {
    if is_rtl(d) { p > edge } else { p < edge }
}

/// A directional glyph's x mirrored around `cx` in a right-to-left UI (disclosure triangles),
/// unchanged left-to-right.
pub fn glyph_x(d: &impl HasDirection, x: f32, cx: f32) -> f32 {
    if is_rtl(d) { 2.0 * cx - x } else { x }
}

/// (pos, align) for [`egui::Painter::text`] hugging `rect`'s reading-start vertical centre, inset `off`.
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

/// The (anchor, offset) pair for an [`egui::Area`] pinned to the screen's reading-end corner:
/// bottom-right left-to-right, bottom-left right-to-left (`bottom = false` picks the top pair);
/// a positive `x_off` moves inward from that edge.
pub fn end_corner_anchor(d: &impl HasDirection, bottom: bool, x_off: f32, y_off: f32) -> (Align2, Vec2) {
    if is_rtl(d) {
        let anchor = if bottom { Align2::LEFT_BOTTOM } else { Align2::LEFT_TOP };
        (anchor, vec2(x_off, y_off))
    } else {
        let anchor = if bottom { Align2::RIGHT_BOTTOM } else { Align2::RIGHT_TOP };
        (anchor, vec2(-x_off, y_off))
    }
}

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
/// fixed: under RTL the window rect is anchored at its right edge (resize grip on the left), so
/// the clamped rect grows leftward from a fixed right edge; in LTR it keeps the fixed left edge.
/// The top edge is fixed in both directions. The returned rect's size is exactly `min` — it never
/// preserves the input rect's current size, so shrink drags are honoured.
pub fn clamp_window_min_size(d: &impl HasDirection, rect: Rect, min: Vec2) -> Rect {
    if is_rtl(d) { Rect::from_min_size(pos2(rect.right() - min.x, rect.top()), min) } else { Rect::from_min_size(rect.min, min) }
}
