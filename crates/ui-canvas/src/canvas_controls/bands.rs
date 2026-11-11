//! `range_bands` and `rect_around`: the outline and fill of a range selection (GridCraft). Pure screen geometry.

use egui::{Rect, pos2};

/// The four sides of a `width` wide outline centred on `rect`, as filled rectangles cut to `clip` (expanded by
/// `2 × width`), all in screen points: left, right, top, bottom in that order, the ones wholly outside the clip left
/// out. A stroked rectangle that spans a 20-million-point column loses its precision in the tessellator and its sides
/// vanish; sides filled from rectangles that stay near the viewport do not. A `rect` or `clip` that is not finite
/// gives none.
pub fn range_bands(rect: Rect, width: f32, clip: Rect) -> Vec<Rect> {
    if !rect.is_finite() || !clip.is_finite() {
        return Vec::new();
    }
    let half = width / 2.0;
    let clip = clip.expand(width * 2.0);
    let sides = [
        Rect::from_min_max(pos2(rect.left() - half, rect.top() - half), pos2(rect.left() + half, rect.bottom() + half)),
        Rect::from_min_max(pos2(rect.right() - half, rect.top() - half), pos2(rect.right() + half, rect.bottom() + half)),
        Rect::from_min_max(pos2(rect.left() - half, rect.top() - half), pos2(rect.right() + half, rect.top() + half)),
        Rect::from_min_max(pos2(rect.left() - half, rect.bottom() - half), pos2(rect.right() + half, rect.bottom() + half)),
    ];
    sides.into_iter().map(|side| side.intersect(clip)).filter(|side| side.is_finite() && side.width() > 0.0 && side.height() > 0.0).collect()
}

/// `area` without `hole`, as up to four rectangles that never overlap (a translucent fill cannot be cleared by
/// painting over it): the strip above the hole, the strip below, then what is left on each side. A hole that misses
/// the area, or is not finite, leaves the area whole; pieces with no size or with numbers that are not finite are
/// dropped. This is the fill of a range selection with its active cell left clear.
pub fn rect_around(area: Rect, hole: Rect) -> Vec<Rect> {
    // `intersect` takes the sane side of a NaN, so a broken hole is dropped first.
    let hole = if hole.is_finite() { area.intersect(hole) } else { Rect::NOTHING };
    let parts = if hole.is_positive() {
        [
            Rect::from_min_max(area.min, pos2(area.max.x, hole.min.y)),
            Rect::from_min_max(pos2(area.min.x, hole.max.y), area.max),
            Rect::from_min_max(pos2(area.min.x, hole.min.y), pos2(hole.min.x, hole.max.y)),
            Rect::from_min_max(pos2(hole.max.x, hole.min.y), pos2(area.max.x, hole.max.y)),
        ]
    } else {
        [area, Rect::NOTHING, Rect::NOTHING, Rect::NOTHING]
    };
    parts.into_iter().filter(|part| part.is_positive() && part.is_finite()).collect()
}
