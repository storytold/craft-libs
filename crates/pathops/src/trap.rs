//! The geometry of a trap (Pathfinder › Trap): the thin strip where one colour is spread into the
//! area of an adjoining colour, so a slight misregistration on press leaves no paper gap between
//! them. See `vectorcraft_effects::trap` for which colour spreads and how the strip is painted.

use craft_geom::{FillRule, PathData};
use kurbo::Affine;

use crate::boolean::{BoolOp, try_boolean, try_normalize};
use crate::offset::{Cap, Join, outline_stroke};

/// Largest trap width, in points, as the public Trap options allow (0.01–5000 pt).
pub const TRAP_MAX_WIDTH: f64 = 5000.0;
/// Smallest trap width, in points.
pub const TRAP_MIN_WIDTH: f64 = 0.01;
/// Height/Width range (horizontal edges' trap as a fraction of vertical edges').
pub const TRAP_HEIGHT_WIDTH_RANGE: (f64, f64) = (0.01, 10.0);

/// Does every anchor and handle of `p` have finite coordinates?
fn finite(p: &PathData) -> bool {
    p.subpaths.iter().flat_map(|sp| sp.anchors.iter()).all(|a| [a.p, a.h_in, a.h_out].iter().all(|q| q.x.is_finite() && q.y.is_finite()))
}

/// The strip of `into` (with `into_rule`) within `width` of the edge of `spread` (with
/// `spread_rule`): where `spread`'s colour reaches when it grows by `width` across vertical edges
/// and by `width × height_width` across horizontal edges. Where the shapes overlap, the strip
/// also covers the overlap, so callers pass areas that don't overlap (the visible parts of two
/// objects). `precision` is the refit tolerance of the boolean that cuts the strip, in points.
///
/// Empty when the shapes don't come within `width` of each other, when either is empty or has a
/// coordinate that isn't finite, or when `width` or `height_width` isn't a finite positive number.
/// `width` is capped at [`TRAP_MAX_WIDTH`] and `height_width` at [`TRAP_HEIGHT_WIDTH_RANGE`].
pub fn trap_band(
    spread: &PathData,
    spread_rule: FillRule,
    into: &PathData,
    into_rule: FillRule,
    width: f64,
    height_width: f64,
    precision: f64,
) -> PathData {
    if !(width.is_finite() && width > 0.0 && height_width.is_finite() && height_width > 0.0) || spread.is_empty() || into.is_empty() {
        return PathData::default();
    }
    if !finite(spread) || !finite(into) {
        return PathData::default();
    }
    let w = width.clamp(TRAP_MIN_WIDTH, TRAP_MAX_WIDTH);
    let r = height_width.clamp(TRAP_HEIGHT_WIDTH_RANGE.0, TRAP_HEIGHT_WIDTH_RANGE.1);
    let precision = if precision.is_finite() && precision > 0.0 { precision.min(10.0) } else { crate::boolean::DEFAULT_PRECISION };
    // Quick reject: bounds farther apart than the trap reaches.
    let (Some(sb), Some(ib)) = (spread.bounds(), into.bounds()) else { return PathData::default() };
    let reach = sb.inflate(w, w * r);
    if reach.x1 < ib.x0 || ib.x1 < reach.x0 || reach.y1 < ib.y0 || ib.y1 < reach.y0 {
        return PathData::default();
    }
    // Grow uniformly in a space squashed vertically by `r`, then stretch back: horizontal edges'
    // strip comes out `r` times as tall as vertical edges' is wide.
    let Ok(outline) = try_normalize(spread, spread_rule) else { return PathData::default() };
    let squash = Affine::scale_non_uniform(1.0, 1.0 / r);
    let ring = outline_stroke(&outline.transformed(squash), 2.0 * w, Cap::Butt, Join::Miter, 4.0);
    if ring.is_empty() {
        return PathData::default();
    }
    let ring = ring.transformed(Affine::scale_non_uniform(1.0, r));
    try_boolean(&ring, FillRule::NonZero, into, into_rule, BoolOp::Intersect, precision).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use craft_geom::Rect;
    use craft_geom::shapes::rectangle;

    use super::*;

    fn bounds(p: &PathData) -> Rect {
        p.bounds().unwrap_or_default()
    }

    fn near(a: f64, b: f64) -> bool {
        (a - b).abs() < 0.02
    }

    #[test]
    fn abutting_rectangles_trap_along_their_shared_edge() {
        let light = rectangle(Rect::new(0.0, 0.0, 50.0, 40.0));
        let dark = rectangle(Rect::new(50.0, 0.0, 100.0, 40.0));
        let band = trap_band(&light, FillRule::NonZero, &dark, FillRule::NonZero, 2.0, 1.0, 0.01);
        let b = bounds(&band);
        assert!(near(b.x0, 50.0) && near(b.x1, 52.0) && near(b.y0, 0.0) && near(b.y1, 40.0), "{b:?}");
        assert!(near(crate::area(&band, FillRule::NonZero), 80.0));
    }

    #[test]
    fn height_width_scales_the_trap_on_horizontal_edges() {
        let light = rectangle(Rect::new(0.0, 0.0, 40.0, 50.0));
        let dark = rectangle(Rect::new(0.0, 50.0, 40.0, 100.0));
        let band = trap_band(&light, FillRule::NonZero, &dark, FillRule::NonZero, 2.0, 2.0, 0.01);
        let b = bounds(&band);
        assert!(near(b.y0, 50.0) && near(b.y1, 54.0), "a horizontal edge's trap is twice the width: {b:?}");
    }

    #[test]
    fn far_apart_shapes_and_bad_numbers_give_nothing() {
        let a = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let b = rectangle(Rect::new(30.0, 0.0, 40.0, 10.0));
        assert!(trap_band(&a, FillRule::NonZero, &b, FillRule::NonZero, 2.0, 1.0, 0.01).is_empty());
        let c = rectangle(Rect::new(10.0, 0.0, 20.0, 10.0));
        for (w, r) in [(f64::NAN, 1.0), (f64::INFINITY, 1.0), (-1.0, 1.0), (0.0, 1.0), (1.0, f64::NAN), (1.0, 0.0)] {
            assert!(trap_band(&a, FillRule::NonZero, &c, FillRule::NonZero, w, r, 0.01).is_empty(), "{w} {r}");
        }
        let mut bad = a.clone();
        bad.subpaths[0].anchors[0].p.x = f64::NAN;
        assert!(trap_band(&bad, FillRule::NonZero, &c, FillRule::NonZero, 1.0, 1.0, 0.01).is_empty());
    }

    #[test]
    fn a_huge_width_is_capped_and_stays_inside_the_darker_shape() {
        let a = rectangle(Rect::new(0.0, 0.0, 10.0, 10.0));
        let c = rectangle(Rect::new(10.0, 0.0, 20.0, 10.0));
        let band = trap_band(&a, FillRule::NonZero, &c, FillRule::NonZero, 1e300, 1e300, f64::NAN);
        let b = bounds(&band);
        assert!(near(b.x0, 10.0) && near(b.x1, 20.0), "the whole darker square at most: {b:?}");
    }
}
