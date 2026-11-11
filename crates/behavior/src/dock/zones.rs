//! The side zones of a group's body: the one function that divides a body into drop zones.

use super::DockSide;
use emath::{Pos2, Rect};

/// Share of a body, from each edge, that means "beside" under [`ZoneSize::Fraction`]: of its width from the start
/// and end edges, of its height from the top and bottom edges.
pub const SIDE_ZONE_FRACTION: f32 = 1.0 / 3.0;

/// The most of a body's smaller side that a [`ZoneSize::Depth`] zone reaches, so a body always keeps a middle.
pub const SIDE_ZONE_DEPTH_SHARE: f32 = 0.25;

/// How deep the side zones of a body are.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ZoneSize {
    /// Zones of this depth in points, at most [`SIDE_ZONE_DEPTH_SHARE`] of the body's smaller side. The caller passes
    /// a theme size.
    Depth(f32),
    /// Zones [`SIDE_ZONE_FRACTION`] of the body's width (start, end) and height (top, bottom).
    Fraction,
}

/// The zones of `body`, the part of a group below its tab strip: the start and end zones along its full height,
/// the top and bottom zones between them, and the middle. `Start` is the physical left rect, or the right rect when
/// `rtl`. The rects do not overlap. A body too small for the zones gives them an empty rect.
pub fn zones(body: Rect, size: ZoneSize, rtl: bool) -> [(DockSide, Rect); 5] {
    let (across, down) = match size {
        ZoneSize::Depth(depth) => {
            let depth = depth.min(body.width().min(body.height()) * SIDE_ZONE_DEPTH_SHARE).max(0.0);
            (depth, depth)
        }
        ZoneSize::Fraction => (body.width() * SIDE_ZONE_FRACTION, body.height() * SIDE_ZONE_FRACTION),
    };
    let left = Rect::from_min_max(body.min, Pos2::new(body.left() + across, body.bottom()));
    let right = Rect::from_min_max(Pos2::new(body.right() - across, body.top()), body.max);
    let (inner_left, inner_right) = (left.right(), right.left());
    let top = Rect::from_min_max(Pos2::new(inner_left, body.top()), Pos2::new(inner_right, body.top() + down));
    let bottom = Rect::from_min_max(Pos2::new(inner_left, body.bottom() - down), Pos2::new(inner_right, body.bottom()));
    let middle = Rect::from_min_max(Pos2::new(inner_left, top.bottom()), Pos2::new(inner_right, bottom.top()));
    let (start, end) = if rtl { (right, left) } else { (left, right) };
    [(DockSide::Start, start), (DockSide::End, end), (DockSide::Top, top), (DockSide::Bottom, bottom), (DockSide::Center, middle)]
}

#[cfg(test)]
mod tests {
    use super::*;

    fn body() -> Rect {
        Rect::from_min_max(Pos2::new(10.0, 20.0), Pos2::new(310.0, 220.0))
    }

    #[test]
    fn zones_do_not_overlap_and_cover_the_body() {
        for size in [ZoneSize::Fraction, ZoneSize::Depth(30.0), ZoneSize::Depth(500.0)] {
            for rtl in [false, true] {
                let all = zones(body(), size, rtl);
                for (i, (_, a)) in all.iter().enumerate() {
                    for (_, b) in &all[i + 1..] {
                        let overlap = a.intersect(*b);
                        assert!(overlap.width() <= 0.0 || overlap.height() <= 0.0, "{size:?} rtl {rtl}: {a:?} and {b:?}");
                    }
                }
                let area: f32 = all.iter().map(|(_, r)| r.width() * r.height()).sum();
                assert!((area - body().width() * body().height()).abs() < 0.5, "{size:?} rtl {rtl}");
            }
        }
    }
}
