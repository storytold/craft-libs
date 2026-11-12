//! Special-case curve-comparisons for straight lines.

use kurbo::Line;

use crate::curve::{CurveOrder, MetricsStatus, Order};

fn line_at_ys(line: Line, y0: f64, y1: f64) -> (f64, f64) {
    let dy_recip = (line.p1.y - line.p0.y).recip();
    let t0 = (y0 - line.p0.y) * dy_recip;
    let t1 = (y1 - line.p0.y) * dy_recip;
    (
        (1.0 - t0) * line.p0.x + t0 * line.p1.x,
        (1.0 - t1) * line.p0.x + t1 * line.p1.x,
    )
}

/// Checks the relative order of two line segments.
///
/// The two line segments must both be increasing in `y`, and their `y` ranges
/// must overlap.
pub fn intersect_lines(l0: Line, l1: Line, tolerance: f64) -> CurveOrder {
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
    let z0 = y0 + (y1 - y0) * t0;
    let z1 = y0 + (y1 - y0) * t1;

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
}
