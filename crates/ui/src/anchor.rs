//! Where a floating bar sits over a selection. Pure geometry: no window needed to test it.

use egui::{pos2, Rect, Vec2};

/// The bar's size and how it keeps clear of the selection.
#[derive(Clone, Copy, Debug)]
pub struct BarPlacement {
    pub size: Vec2,
    /// Vertical gap between the bar and the selection's first line.
    pub gap: f32,
    /// Distance from the selection's left edge at which the bar is centred.
    pub inset: f32,
    /// Least height of a selection rectangle that counts as visible.
    pub min_visible_h: f32,
}

/// The topmost selection rectangle visible inside `clip`, cut to its visible part. `None`
/// when no usable part is on screen. Rectangles that only touch the clip's edge, or are
/// not finite, are ignored.
pub fn visible_anchor(selection_rects: impl IntoIterator<Item = Rect>, clip: Rect, min_visible_h: f32) -> Option<Rect> {
    if !clip.is_finite() || !clip.is_positive() || !min_visible_h.is_finite() || min_visible_h < 0.0 {
        return None;
    }
    let mut first: Option<Rect> = None;
    for sr in selection_rects {
        if !sr.is_finite() || !sr.intersects(clip) {
            continue;
        }
        let visible = sr.intersect(clip);
        if visible.width() <= 0.0 || visible.height() <= 0.0 || visible.height() < min_visible_h {
            continue;
        }
        if first.is_none_or(|f| visible.min.y < f.min.y) {
            first = Some(visible);
        }
    }
    first
}

/// Where the bar sits for a selection line and viewport: above the line, below when there
/// is no room above, always inside the viewport when it is large enough. `None` for a
/// non-finite input.
pub fn anchor_rect(selection: Rect, viewport: Rect, bar: BarPlacement) -> Option<Rect> {
    if !selection.is_finite()
        || !selection.is_positive()
        || !viewport.is_finite()
        || !viewport.is_positive()
        || !bar.size.x.is_finite()
        || !bar.size.y.is_finite()
        || bar.size.x <= 0.0
        || bar.size.y <= 0.0
        || !bar.gap.is_finite()
        || !bar.inset.is_finite()
        || !bar.min_visible_h.is_finite()
        || bar.min_visible_h < 0.0
    {
        return None;
    }
    let (w, h) = (bar.size.x, bar.size.y);
    let cx = (selection.left() + bar.inset).clamp(viewport.left(), viewport.right());
    let x = (cx - w / 2.0).clamp(viewport.left(), (viewport.right() - w).max(viewport.left()));
    // Both viewport edges count: a selection can be partly scrolled out of view.
    let above = selection.top() - bar.gap - h;
    let below = selection.bottom() + bar.gap;
    let fits = |y: f32| y >= viewport.top() && y + h <= viewport.bottom();
    let y = if fits(above) {
        above
    } else if fits(below) {
        below
    } else {
        let desired = if above < viewport.top() { below } else { above };
        desired.clamp(viewport.top(), (viewport.bottom() - h).max(viewport.top()))
    };
    Some(Rect::from_min_size(pos2(x, y), bar.size))
}

#[cfg(test)]
mod tests;
