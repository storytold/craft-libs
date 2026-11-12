//! Utilities for generating examples, benchmarks, and test cases.

use std::f64::consts::PI;

use kurbo::{Affine, BezPath};

use crate::Point;

type Contours = Vec<Vec<Point>>;

/// Generate a bunch of squares, arranged in a grid.
///
/// The top-left of the first square is at (x0, y0). Each square has size `size
/// x size`, and the distance between squares (both horizontally and vertically)
/// is `offset`.
///
/// If `slant` is non-zero, generates parallelograms instead of squares: the
/// right-hand side of each square gets translated down by `slant`.
fn squares((x0, y0): (f64, f64), size: f64, offset: f64, slant: f64, count: usize) -> Contours {
    let mut ret = Vec::new();
    for i in 0..count {
        let x = x0 + i as f64 * offset;
        for j in 0..count {
            let y = y0 + j as f64 * offset;
            ret.push(vec![
                Point::new(x, y),
                Point::new(x, y + size),
                Point::new(x + size, y + size + slant),
                Point::new(x + size, y + slant),
            ]);
        }
    }

    ret
}

/// Generate an `n` by `n` checkerboard-like pattern with overlapping squares.
/// For `n = 3`, it looks like:
///
/// ```text
/// ┌────┐ ┌────┐ ┌────┐
/// │    │ │    │ │    │
/// │  ┌─┼─┼─┐┌─┼─┼─┐  │
/// └──┼─┘ └─┼┼─┘ └─┼──┘
/// ┌──┼─┐ ┌─┼┼─┐ ┌─┼──┐
/// │  └─┼─┼─┘└─┼─┼─┘  │
/// │  ┌─┼─┼─┐┌─┼─┼─┐  │
/// └──┼─┘ └─┼┼─┘ └─┼──┘
/// ┌──┼─┐ ┌─┼┼─┐ ┌─┼──┐
/// │  └─┼─┼─┘└─┼─┼─┘  │
/// │    │ │    │ │    │
/// └────┘ └────┘ └────┘
/// ```
///
/// We return the pattern in two parts: the outer collection of `n x n`
/// non-overlapping squares, and the inner collection of `(n - 1) x (n - 1)`
/// non-overlapping squares.
pub fn checkerboard(n: usize) -> (Contours, Contours) {
    (
        squares((0.0, 0.0), 30.0, 40.0, 0.0, n),
        squares((20.0, 20.0), 30.0, 40.0, 0.0, n - 1),
    )
}

/// Like `checkerboard`, but with no exactly-horizontal lines.
///
/// Horizontal lines have special handling in the sweep-line algorithm, so
/// their presence or absence can affect performance.
pub fn slanted_checkerboard(n: usize) -> (Contours, Contours) {
    (
        squares((0.0, 0.0), 30.0, 40.0, 1.0, n),
        squares((20.0, 20.0), 30.0, 40.0, 1.0, n - 1),
    )
}

/// The "evens" are a bunch of long, skinny parallelograms going from top-left
/// to bottom-right. The "odds" go from top-right to bottom-left.
pub fn slanties(n: usize) -> (Contours, Contours) {
    let h = 20.0 * n as f64;

    let mut even = Vec::new();
    let mut odd = Vec::new();
    for i in 0..n {
        let x_off = 20.0 * i as f64;
        even.push(vec![
            Point::new(x_off, 0.0),
            Point::new(x_off + h, h),
            Point::new(x_off + h + 10.0, h),
            Point::new(x_off + 10.0, 0.0),
        ]);

        odd.push(vec![
            Point::new(x_off + h, 0.0),
            Point::new(x_off, h),
            Point::new(x_off + 10.0, h),
            Point::new(x_off + h + 10.0, 0.0),
        ]);
    }

    (even, odd)
}

/// A bunch of things that all intersect at the origin, some tangentially.
pub fn star(n: usize) -> (BezPath, BezPath) {
    fn mul(p: kurbo::Point, f: f64) -> kurbo::Point {
        (p.x * f, p.y * f).into()
    }

    let angle_increment = PI / (n as f64 * 2.0);
    let mut angle = 0.0;

    let mut straight = BezPath::new();
    let mut bent = BezPath::new();
    let p = kurbo::Point::new(1.0, 0.0);
    for i in 0..n {
        let p0 = Affine::rotate(angle) * p;
        let p1 = Affine::rotate(angle + angle_increment) * p;
        let p2 = Affine::rotate(angle + angle_increment + PI) * p;
        let p3 = Affine::rotate(angle + PI) * p;

        if i.is_multiple_of(2) {
            straight.move_to(p0);
            straight.line_to(p1);
            straight.line_to(p2);
            straight.line_to(p3);
            straight.close_path();
        } else {
            let zero_ish = mul(p0, 1e-12);
            let neg_zero_ish = mul(p0, -1e-12);
            bent.move_to(p0);
            bent.line_to(p1);
            bent.curve_to(mul(p1, 2. / 3.), mul(p0.midpoint(p1), 1. / 3.), zero_ish);
            bent.curve_to(mul(p2.midpoint(p3), 1. / 3.), mul(p2, 2. / 3.), p2);
            bent.line_to(p3);
            bent.curve_to(
                mul(p3, 2. / 3.),
                mul(p2.midpoint(p3), 1. / 3.),
                neg_zero_ish,
            );
            bent.curve_to(mul(p0.midpoint(p1), 1. / 3.), mul(p0, 2. / 3.), p0);
        }

        angle += 2.0 * angle_increment;
    }

    (straight, bent)
}
