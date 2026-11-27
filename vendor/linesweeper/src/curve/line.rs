//! Special-case curve-comparisons for straight lines.

use kurbo::{Affine, Line};

use crate::curve::{CurveOrder, MetricsStatus, Order};

fn line_at_ys(line: Line, y0: f64, y1: f64) -> (f64, f64) {
    let dy = line.p1.y - line.p0.y;
    let (t0, t1) = if dy.is_finite() {
        ((y0 - line.p0.y) / dy, (y1 - line.p0.y) / dy)
    } else {
        // The line is more than f64::MAX tall, so work with half of each height.
        let half_dy = line.p1.y * 0.5 - line.p0.y * 0.5;
        (
            (y0 * 0.5 - line.p0.y * 0.5) / half_dy,
            (y1 * 0.5 - line.p0.y * 0.5) / half_dy,
        )
    };
    (x_at_t(line, t0), x_at_t(line, t1))
}

/// Returns `(1 - t) * line.p0.x + t * line.p1.x`.
fn x_at_t(line: Line, t: f64) -> f64 {
    if line.p0.x == line.p1.x {
        return line.p0.x;
    }
    // Craft patch (subnormal x): a product of t and a subnormal x coordinate
    // loses bits, so x = 5e-324 times 0.5 rounds to 0. When both x coordinates
    // have a magnitude below 2^-500, take the weighted sum of the x coordinates
    // times 2^600 and scale the sum back. Multiplying by a power of two is
    // exact for normal numbers, and multiplying by 1 changes nothing.
    const SMALL: f64 = f64::from_bits((1023 - 500) << 52);
    const UP: f64 = f64::from_bits((1023 + 600) << 52);
    const DOWN: f64 = f64::from_bits((1023 - 600) << 52);
    let (up, down) = if line.p0.x.abs().max(line.p1.x.abs()) < SMALL {
        (UP, DOWN)
    } else {
        (1.0, 1.0)
    };
    ((1.0 - t) * (line.p0.x * up) + t * (line.p1.x * up)) * down
}

/// Checks the relative order of two line segments.
///
/// The two line segments must both be increasing in `y`, and their `y` ranges
/// must overlap.
pub fn intersect_lines(l0: Line, l1: Line, tolerance: f64) -> CurveOrder {
    const SCALE_ABOVE: f64 = f64::from_bits((1023 + 1019) << 52);
    const SCALE: f64 = 1.0 / 16.0;
    let max_x = [l0.p0, l0.p1, l1.p0, l1.p1]
        .iter()
        .fold(tolerance.abs(), |m, p| m.max(p.x.abs()));
    if max_x > SCALE_ABOVE {
        let y0 = l0.p0.y.max(l1.p0.y);
        let y1 = l0.p1.y.min(l1.p1.y);
        let (l0x0, l0x1) = line_at_ys(l0, y0, y1);
        let (l1x0, l1x1) = line_at_ys(l1, y0, y1);
        let dx0 = l1x0 - l0x0;
        let dx1 = l1x1 - l0x1;
        if [dx0, dx1, dx0 - dx1, dx0 + tolerance, dx0 - tolerance]
            .iter()
            .all(|v| v.is_finite())
        {
            return intersect_lines_unscaled(l0, l1, tolerance);
        }
        let scale = Affine::scale_non_uniform(SCALE, 1.0);
        return intersect_lines_unscaled(scale * l0, scale * l1, tolerance * SCALE);
    }
    intersect_lines_unscaled(l0, l1, tolerance)
}

fn intersect_lines_unscaled(l0: Line, l1: Line, tolerance: f64) -> CurveOrder {
    let y0 = l0.p0.y.max(l1.p0.y);
    let y1 = l0.p1.y.min(l1.p1.y);
    let mut ret = CurveOrder::new(y0);
    let status = MetricsStatus::default();

    debug_assert!(y0 <= y1);

    let (l0x0, l0x1) = line_at_ys(l0, y0, y1);
    let (l1x0, l1x1) = line_at_ys(l1, y0, y1);
    let dx0 = l1x0 - l0x0;
    let dx1 = l1x1 - l0x1;

    if dx0.abs() <= tolerance && dx1.abs() <= tolerance {
        ret.push(y1, Order::Ish, status);
        return ret;
    } else if (dx0 - dx1).abs() <= tolerance / 2.0 {
        // The lines are almost parallel: we'll mark them as always having the
        // same order.
        let order = if dx0 <= -tolerance || dx1 <= -tolerance {
            Order::Right
        } else if dx0 >= tolerance || dx1 >= tolerance {
            Order::Left
        } else {
            Order::Ish
        };
        ret.push(y1, order, status);
        return ret;
    }

    // The t at which l0 crosses l1 - tolerance.
    let t0 = (dx0 + tolerance) / (dx0 - dx1);
    let t1 = (dx0 - tolerance) / (dx0 - dx1);
    let height_at = |t: f64| {
        let dy = y1 - y0;
        if dy.is_finite() {
            y0 + dy * t
        } else if (0.0..=1.0).contains(&t) {
            y0 * (1.0 - t) + y1 * t
        } else if t < 0.0 {
            f64::NEG_INFINITY
        } else if t > 1.0 {
            f64::INFINITY
        } else {
            f64::NAN
        }
    };
    let z0 = height_at(t0);
    let z1 = height_at(t1);

    // For a second, consider the whole lines and not the segments. If dx0 < dx1,
    // l0 starts off Left, then they become Ish, then l0 is Right. If dx1 > dx0,
    // they go Right, Ish, Left.
    //
    // Once we restrict to the line segments instead of the lines, the basic
    // orderings are the same but they might get truncated.
    if dx0 < dx1 {
        debug_assert!(z0 <= z1);
        if z0 > y0 {
            ret.push(z0.min(y1), Order::Right, status);
            if z0 >= y1 {
                return ret;
            }
        }
        if z1 > y0 && z1 > z0 {
            ret.push(z1.min(y1), Order::Ish, status);
            if z1 >= y1 {
                return ret;
            }
        }
        ret.push(y1, Order::Left, status);
    } else {
        debug_assert!(z0 >= z1);
        if z1 > y0 {
            ret.push(z1.min(y1), Order::Left, status);
            if z1 >= y1 {
                return ret;
            }
        }
        if z0 > y0 && z0 > z1 {
            ret.push(z0.min(y1), Order::Ish, status);
            if z0 >= y1 {
                return ret;
            }
        }
        ret.push(y1, Order::Right, status);
    }

    ret
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn affine_endpoints_crossing_preserves_an_uneven_overflowing_span() {
        let a = 2f64.powi(1023);
        let b = a.next_up();
        let fixed = Line::new((0.0, -a), (0.0, b));
        let moving = Line::new((-1.0, -a), (1.0, b));
        let crossing = a.midpoint(-b).abs();
        let order = intersect_lines(fixed, moving, 0.0);
        assert_eq!(order.order_at(crossing * 0.5), Order::Right);
        assert_eq!(order.order_at(crossing * 1.5), Order::Left);
    }

    #[test]
    fn simple_intersection() {
        let l0 = Line::new((0.0, 0.0), (0.0, 1.0));
        let l1 = Line::new((1.0, 0.0), (-1.0, 1.0));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 3);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[1].order, Order::Ish);
        assert_eq!(order.cmps[2].order, Order::Right);
        assert!((order.cmps[0].end - 0.5).abs() < 1e-5);

        let order = intersect_lines(l1, l0, 1e-6);
        assert_eq!(order.cmps.len(), 3);
        assert_eq!(order.cmps[0].order, Order::Right);
        assert_eq!(order.cmps[1].order, Order::Ish);
        assert_eq!(order.cmps[2].order, Order::Left);
        assert!((order.cmps[0].end - 0.5).abs() < 1e-5);
    }

    #[test]
    fn unchanging() {
        let l0 = Line::new((-1.0, 0.0), (-1.0, 1.0));
        let l1 = Line::new((1.0, 0.0), (1.0, 1.0));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);

        let l0 = Line::new((-2.0, 0.0), (0.0, 1.0));
        let l1 = Line::new((1.0, 0.0), (1.0, 1.0));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
    }

    #[test]
    fn coincident_vertical_lines_keep_their_exact_x() {
        let whole = Line::new((0.1, 0.0), (0.1, 10.0));
        let part = Line::new((0.1, 3.0), (0.1, 10.0));
        for (left, right) in [(whole, part), (part, whole)] {
            let order = intersect_lines(left, right, 1e-18);
            assert!(
                order.cmps.iter().all(|entry| entry.order == Order::Ish),
                "{order:?}"
            );
            assert_eq!(line_at_ys(left, 3.0, 10.0), (0.1, 0.1));
        }
    }

    #[test]
    fn unchanging_with_huge_coordinates() {
        for x in [1e300, 1e308, f64::MAX] {
            let l0 = Line::new((-x, 0.0), (-x, 1.0));
            let l1 = Line::new((x, 0.0), (x, 1.0));

            let order = intersect_lines(l0, l1, 1e-6);
            assert_eq!(order.cmps.len(), 1, "x = {x}");
            assert_eq!(order.cmps[0].order, Order::Left, "x = {x}");
            assert_eq!(order.cmps[0].end, 1.0, "x = {x}");
        }
    }

    #[test]
    fn overlap_preserves_a_subnormal_gap_beside_a_distant_endpoint() {
        let unit = f64::from_bits(1);
        let left = Line::new((1e308, 0.0), (0.0, 1.0));
        let right = Line::new((2.0 * unit, 1.0), (2.0 * unit, 2.0));
        for (a, b, expected) in [(left, right, Order::Left), (right, left, Order::Right)] {
            let order = intersect_lines(a, b, unit);
            assert_eq!(order.start, 1.0);
            assert_eq!(order.cmps.len(), 1);
            assert_eq!(order.cmps[0].end, 1.0);
            assert_eq!(order.cmps[0].order, expected);
        }
    }

    #[test]
    fn huge_coordinates_on_one_axis() {
        // Huge x values with a tiny height.
        let l0 = Line::new((1e308, 0.0), (1e308, 1e-307));
        let l1 = Line::new((1.1e308, 0.0), (1.1e308, 1e-307));
        let order = intersect_lines(l0, l1, 0.0);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, 1e-307);

        // Lines more than f64::MAX tall, close together in x.
        let l0 = Line::new((0.0, -1e308), (0.0, 1e308));
        let l1 = Line::new((1.0, -1e308), (1.0, 1e308));
        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.start, -1e308);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, 1e308);

        // A very short line beside a line more than f64::MAX tall.
        let l0 = Line::new((0.0, 1e-308), (0.0, 2e-308));
        let l1 = Line::new((1.0, -1e308), (1.0, 1e308));
        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, 2e-308);

        // Lines more than f64::MAX tall that cross at y = 0.
        let l0 = Line::new((-1.0, -1e308), (1.0, 1e308));
        let l1 = Line::new((1.0, -1e308), (-1.0, 1e308));
        let order = intersect_lines(l0, l1, 1e-6);
        let orders: Vec<_> = order.cmps.iter().map(|c| c.order).collect();
        assert_eq!(orders, [Order::Left, Order::Ish, Order::Right]);
        assert!((-1e303..0.0).contains(&order.cmps[0].end), "{order:?}");
        assert!((0.0..1e303).contains(&order.cmps[1].end), "{order:?}");
        assert_eq!(order.cmps[2].end, 1e308);
    }

    #[test]
    fn subnormal_height() {
        let h = 1e-310;
        let l0 = Line::new((0.0, 0.0), (0.0, h));
        let l1 = Line::new((1.0, 0.0), (1.0, h));
        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, h);
    }

    #[test]
    fn subnormal_sloped_lines_keep_interior_overlap_coordinates() {
        let h = 1e-310;
        let sloped = Line::new((0.0, 0.0), (4.0, h));
        let (x0, x1) = super::line_at_ys(sloped, h * 0.25, h * 0.75);
        assert!((x0 - 1.0).abs() < 1e-12, "{x0}");
        assert!((x1 - 3.0).abs() < 1e-12, "{x1}");
        let other = Line::new((5.0, h * 0.25), (6.0, h * 0.75));
        let order = intersect_lines(sloped, other, 1e-6);
        assert_eq!(order.start, h * 0.25);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, h * 0.75);
    }

    #[test]
    fn close_near_endpoint() {
        let l0 = Line::new((0.0, 0.0), (0.0, 1.0));
        let l1 = Line::new((1e-7, 0.0), (-1.0, 1.0));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 2);
        assert_eq!(order.cmps[0].order, Order::Ish);
        assert_eq!(order.cmps[1].order, Order::Right);
        assert!(order.cmps[0].end.abs() < 1e-5);

        let l0 = Line::new((0.0, 0.0), (0.0, 1.0));
        let l1 = Line::new((-1.0, 0.0), (1e-7, 1.0));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 2);
        assert_eq!(order.cmps[0].order, Order::Right);
        assert_eq!(order.cmps[1].order, Order::Ish);
        assert!((order.cmps[0].end - 1.0).abs() < 1e-5);
    }

    #[test]
    fn subnormal_span() {
        let l0 = Line::new((-1.0, 0.0), (-1.0, 1e-310));
        let l1 = Line::new((1.0, 0.0), (1.0, 1e-310));

        let order = intersect_lines(l0, l1, 1e-6);
        assert_eq!(order.cmps.len(), 1);
        assert_eq!(order.cmps[0].order, Order::Left);
        assert_eq!(order.cmps[0].end, 1e-310);
    }

    #[test]
    fn subnormal_x() {
        let x = f64::from_bits(1);
        let l = Line::new((x, 0.0), (x, 2.0));
        assert_eq!(line_at_ys(l, 0.5, 1.0), (x, x));

        let l = Line::new((-3.0 * x, 0.0), (3.0 * x, 2.0));
        assert_eq!(line_at_ys(l, 0.0, 2.0), (-3.0 * x, 3.0 * x));
        assert_eq!(line_at_ys(l, 1.0, 1.0), (0.0, 0.0));
    }
}
