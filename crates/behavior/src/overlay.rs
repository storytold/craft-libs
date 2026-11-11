//! `place`: where an overlay of a given content size goes around its anchor.
//!
//! The function is pure. The overlay host resolves the anchor to a screen-space rect and reads the content size
//! from layout memory before it calls.

use emath::{Pos2, Rect, Vec2};

/// The side of the anchor an overlay prefers. `Start` and `End` follow the reading direction.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Side {
    /// Under the anchor.
    Below,
    /// Over the anchor.
    Above,
    /// Before the anchor in reading order: left under LTR, right under RTL.
    Start,
    /// After the anchor in reading order.
    End,
}

impl Side {
    const fn flipped(self) -> Side {
        match self {
            Side::Below => Side::Above,
            Side::Above => Side::Below,
            Side::Start => Side::End,
            Side::End => Side::Start,
        }
    }

    const fn vertical(self) -> bool {
        matches!(self, Side::Below | Side::Above)
    }
}

/// How the overlay lines up with the anchor along the edge it hangs from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Align {
    /// Reading start (top for a side placement).
    Start,
    /// Centred on the anchor.
    Center,
    /// Reading end (bottom for a side placement).
    End,
}

/// The narrowest an overlay may be.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub enum MinWidth {
    /// The content decides.
    #[default]
    None,
    /// At least as wide as the anchor.
    Anchor,
    /// At least this many points.
    Points(f32),
}

impl MinWidth {
    /// The width in points, given the anchor's width.
    pub fn points(self, anchor_width: f32) -> f32 {
        match self {
            MinWidth::None => 0.0,
            MinWidth::Anchor => anchor_width,
            MinWidth::Points(points) => finite(points).max(0.0),
        }
    }
}

/// What `place` reads.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PlaceRequest {
    /// The anchor in screen space.
    pub anchor: Rect,
    /// The content size.
    pub size: Vec2,
    /// The preferred side.
    pub side: Side,
    /// The alignment along the anchor's edge.
    pub align: Align,
    /// The reading direction.
    pub rtl: bool,
    /// The space between anchor and overlay.
    pub gap: f32,
    /// The narrowest the overlay may be.
    pub min_width: MinWidth,
    /// The room the overlay must stay inside: the viewport's safe rect.
    pub bounds: Rect,
    /// The side the overlay had when it opened; it keeps that side while open instead of flipping.
    pub kept: Option<Side>,
}

impl PlaceRequest {
    /// A request for `size` around `anchor` inside `bounds`: below, aligned to the start, no gap, free width.
    pub const fn new(anchor: Rect, size: Vec2, bounds: Rect) -> Self {
        PlaceRequest { anchor, size, side: Side::Below, align: Align::Start, rtl: false, gap: 0.0, min_width: MinWidth::None, bounds, kept: None }
    }

    /// Sets the content size.
    pub const fn size(mut self, size: Vec2) -> Self {
        self.size = size;
        self
    }

    /// Sets the preferred side.
    pub const fn side(mut self, side: Side) -> Self {
        self.side = side;
        self
    }

    /// Sets the alignment.
    pub const fn align(mut self, align: Align) -> Self {
        self.align = align;
        self
    }

    /// Sets the reading direction.
    pub const fn rtl(mut self, rtl: bool) -> Self {
        self.rtl = rtl;
        self
    }

    /// Sets the gap.
    pub const fn gap(mut self, gap: f32) -> Self {
        self.gap = gap;
        self
    }

    /// Sets the narrowest width.
    pub const fn min_width(mut self, min_width: MinWidth) -> Self {
        self.min_width = min_width;
        self
    }

    /// Keeps `side` instead of flipping.
    pub const fn kept(mut self, side: Option<Side>) -> Self {
        self.kept = side;
        self
    }
}

/// Where `place` put the overlay.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Placement {
    /// The physical rect, left-top first.
    pub rect: Rect,
    /// The side it took: the preferred one, or the other after a flip.
    pub side: Side,
}

fn finite(value: f32) -> f32 {
    if value.is_finite() { value } else { 0.0 }
}

fn clean(rect: Rect) -> Rect {
    let (min, max) = (Pos2::new(finite(rect.min.x), finite(rect.min.y)), Pos2::new(finite(rect.max.x), finite(rect.max.y)));
    Rect::from_min_max(min, max.max(min))
}

/// The room on `side` of `anchor` inside `bounds`, after the gap.
fn room(anchor: Rect, bounds: Rect, side: Side, rtl: bool, gap: f32) -> f32 {
    let left = match (side, rtl) {
        (Side::Start, false) | (Side::End, true) => Some(true),
        (Side::Start, true) | (Side::End, false) => Some(false),
        _ => None,
    };
    match (side, left) {
        (Side::Below, _) => bounds.bottom() - anchor.bottom() - gap,
        (Side::Above, _) => anchor.top() - bounds.top() - gap,
        (_, Some(true)) => anchor.left() - bounds.left() - gap,
        (_, _) => bounds.right() - anchor.right() - gap,
    }
}

/// Places an overlay: the preferred side unless the other has more room and the content does not fit, shifted to
/// stay inside the bounds, its height (or width, beside the anchor) capped to the room. The rect is always finite
/// and inside the bounds.
pub fn place(request: &PlaceRequest) -> Placement {
    let anchor = clean(request.anchor);
    let bounds = clean(request.bounds);
    let gap = finite(request.gap).max(0.0);
    let min_width = request.min_width.points(anchor.width());
    let wanted = Vec2::new(finite(request.size.x).max(min_width).max(0.0), finite(request.size.y).max(0.0));
    let rtl = request.rtl;
    let need = if request.side.vertical() { wanted.y } else { wanted.x };
    let preferred = room(anchor, bounds, request.side, rtl, gap);
    let other = room(anchor, bounds, request.side.flipped(), rtl, gap);
    let side = match request.kept {
        Some(kept) => kept,
        None if preferred >= need || preferred >= other => request.side,
        None => request.side.flipped(),
    };
    let available = room(anchor, bounds, side, rtl, gap).max(0.0);
    let size = if side.vertical() {
        Vec2::new(wanted.x.min(bounds.width()), wanted.y.min(available).min(bounds.height()))
    } else {
        Vec2::new(wanted.x.min(available).min(bounds.width()), wanted.y.min(bounds.height()))
    };
    let across_x = |align: Align| match (align, rtl) {
        (Align::Start, false) | (Align::End, true) => anchor.left(),
        (Align::Start, true) | (Align::End, false) => anchor.right() - size.x,
        (Align::Center, _) => anchor.center().x - size.x / 2.0,
    };
    let across_y = match request.align {
        Align::Start => anchor.top(),
        Align::Center => anchor.center().y - size.y / 2.0,
        Align::End => anchor.bottom() - size.y,
    };
    let min = match (side, rtl) {
        (Side::Below, _) => Pos2::new(across_x(request.align), anchor.bottom() + gap),
        (Side::Above, _) => Pos2::new(across_x(request.align), anchor.top() - gap - size.y),
        (Side::Start, false) | (Side::End, true) => Pos2::new(anchor.left() - gap - size.x, across_y),
        (Side::Start, true) | (Side::End, false) => Pos2::new(anchor.right() + gap, across_y),
    };
    let max_min = Pos2::new((bounds.right() - size.x).max(bounds.left()), (bounds.bottom() - size.y).max(bounds.top()));
    let min = Pos2::new(min.x.clamp(bounds.left(), max_min.x), min.y.clamp(bounds.top(), max_min.y));
    Placement { rect: Rect::from_min_size(min, size), side }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect::from_min_size(Pos2::new(x, y), Vec2::new(w, h))
    }

    const BOUNDS: Rect = Rect { min: Pos2::new(0.0, 0.0), max: Pos2::new(800.0, 600.0) };

    #[test]
    fn rect_is_finite_and_inside_the_bounds_for_any_request() {
        let sides = [Side::Below, Side::Above, Side::Start, Side::End];
        let aligns = [Align::Start, Align::Center, Align::End];
        let values = [-50.0, 0.0, 10.0, 399.0, 790.0, 900.0, f32::NAN, f32::INFINITY];
        for &x in &values {
            for &y in &values {
                for &w in &values {
                    for &h in &values {
                        for (i, &side) in sides.iter().enumerate() {
                            for &align in &aligns {
                                let request = PlaceRequest::new(rect(x, y, w.abs(), h.abs()), Vec2::new(w, h), BOUNDS)
                                    .side(side)
                                    .align(align)
                                    .rtl(i % 2 == 0)
                                    .gap(if i == 1 { 4.0 } else { 0.0 });
                                let placed = place(&request).rect;
                                assert!(placed.min.is_finite() && placed.max.is_finite(), "{request:?}");
                                assert!(BOUNDS.contains_rect(placed), "{request:?} gave {placed:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}
