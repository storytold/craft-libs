//! Scroll bar parts: their declarations, thumb geometry and the moves their results ask for.

use crate::primitives::sensing::PartResults;
use craft_behavior::{Axis, PartKey, PressPhase, ScrollSpan};
use egui::{Pos2, Rect, Vec2};
use std::ops::Range;

/// Whether the thumb covers the track's centre, the point an AccessKit click on the track pages toward: a click there
/// pages nowhere.
pub(super) fn thumb_covers_centre(track: Rect, thumb: Rect, axis: Axis) -> bool {
    let middle = track.center();
    if axis == Axis::X { thumb.x_range().contains(middle.x) } else { thumb.y_range().contains(middle.y) }
}

/// The index of a scroll bar's thumb part.
#[doc(hidden)]
pub const THUMB: u64 = 0;

/// One axis's scroll bar, declared by `PartsCx::scroll`.
pub(crate) struct ScrollDecl {
    pub key: PartKey,
    pub axis: Axis,
    pub rtl: bool,
    pub track: Rect,
    pub thumb: Rect,
    pub span: ScrollSpan,
    pub min_thumb: f32,
}

pub(super) fn track_length(rect: Rect, axis: Axis) -> f32 {
    match axis {
        Axis::X => rect.width(),
        _ => rect.height(),
    }
}

/// The thumb `at` points from the start edge of `track`, `len` long; the start edge of the x axis is the right
/// edge under right-to-left.
pub(crate) fn thumb_rect(track: Rect, axis: Axis, rtl: bool, at: f32, len: f32) -> Rect {
    match axis {
        Axis::X if rtl => Rect::from_min_size(egui::pos2(track.max.x - at - len, track.min.y), Vec2::new(len, track.height())),
        Axis::X => Rect::from_min_size(egui::pos2(track.min.x + at, track.min.y), Vec2::new(len, track.height())),
        _ => Rect::from_min_size(egui::pos2(track.min.x, track.min.y + at), Vec2::new(track.width(), len)),
    }
}

/// What one scroll bar can move the visible range to this pass.
pub(crate) struct ScrollMove {
    pub(super) key: PartKey,
    /// Where this pass's thumb drag or track press moved it.
    pub(super) pressed: Option<Range<f64>>,
    /// Where an AccessKit click on the track part moves it: one page toward the track's centre, which is where a
    /// pointer click on the node would land. `None` when the thumb covers that point.
    pub(super) clicked: Option<Range<f64>>,
}

/// The ranges this pass's scroll-part presses moved, and the ranges an AccessKit click on each track would move
/// to, from step 8's results.
pub(crate) fn scroll_moves(decls: &[ScrollDecl], results: &PartResults) -> Vec<ScrollMove> {
    let mut moves = Vec::new();
    for decl in decls {
        let sign = if decl.axis == Axis::X && decl.rtl { -1.0 } else { 1.0 };
        let along = |delta: Vec2| sign * if decl.axis == Axis::X { delta.x } else { delta.y };
        let axis_pos = |pos: Pos2| if decl.axis == Axis::X { pos.x } else { pos.y };
        let page_toward = |at: f32| craft_behavior::scroll::page(&decl.span, sign * (at - axis_pos(decl.thumb.center())) > 0.0);
        let length = track_length(decl.track, decl.axis);
        let thumb = results.get(&(decl.key, Some(THUMB))).map(|result| result.press.phase);
        let track = results.get(&(decl.key, None)).map(|result| result.press.phase);
        let pressed = match (thumb, track) {
            (Some(PressPhase::Drag { delta, .. }), _) => Some(craft_behavior::scroll::drag(&decl.span, length, decl.min_thumb, along(delta))),
            (_, Some(PressPhase::Down { start, .. })) => Some(page_toward(axis_pos(start.origin))),
            _ => None,
        };
        let middle = decl.track.center();
        let clicked = (!thumb_covers_centre(decl.track, decl.thumb, decl.axis)).then(|| page_toward(axis_pos(middle)));
        moves.push(ScrollMove { key: decl.key, pressed, clicked });
    }
    moves
}
