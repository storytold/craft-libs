//! Conservative checks for transversal intersections.
//!
//! We go to some effort to output curves with the correct horizontal
//! order. When curves are close to one another, our general solution
//! involves approximation with axis-aligned quadratics. This works,
//! but can lead to lots of subdivision. This module is about
//! improving a common case: when two curves intersect each other
//! transversally, they are close to one another near the intersection
//! point, but we can still tell that they are ordered. Hence, we
//! can avoid subdividing.

use kurbo::{CubicBez, ParamCurve, ParamCurveDeriv, QuadBez};

use crate::curve::solve_t_for_y;

// Finds the minimum of `a t^2 + 2 b t (1 - t) + c (1 - t)^2` in the given range of t.
fn quad_max_between(a: f64, b: f64, c: f64, range: std::ops::RangeInclusive<f64>) -> f64 {
    // We don't check for inf or nan here, but our subsequent comparisons will filter
    // those out. (The order of the comparisons is important, because nan always compares
    // false.)
    let critical_t = (c - b) / (a - b + (c - b));

    let t0 = *range.start();
    let t1 = *range.end();

    let eval = |t: f64| a * t * t + 2.0 * b * t * (1.0 - t) + c * (1.0 - t) * (1.0 - t);
    let mut max = eval(t0).max(eval(t1));

    if critical_t > t0 && critical_t < t1 {
        max = max.max(eval(critical_t));
    }
    max
}

fn quad_min_between(a: f64, b: f64, c: f64, range: std::ops::RangeInclusive<f64>) -> f64 {
    -quad_max_between(-a, -b, -c, range)
}

pub fn transversal_after(left: CubicBez, right: CubicBez, y1: f64) -> bool {
    debug_assert_eq!(left.start().y, right.start().y);
    debug_assert!(left.start().x <= right.start().x);
    debug_assert!(left.start().y < y1);

    // First, check if they're transversal at the initial point. We exclude
    // curves whose tangents are close to degenerate here, just to make things
    // nicer numerically.
    let left_tangent = left.p1 - left.p0;
    let right_tangent = right.p1 - right.p0;
    if left_tangent.hypot2() < 1e-20 || right_tangent.hypot2() < 1e-20 {
        return false;
    }
    let cross = left_tangent.cross(right_tangent);
    if left_tangent.dot(right_tangent) > 0.0
        && cross * cross < 1e-2 * left_tangent.hypot2() * right_tangent.hypot2()
    {
        return false;
    }

    let t1_left = solve_t_for_y(left, y1);
    let QuadBez {
        p0: l0,
        p1: l1,
        p2: l2,
    } = left.deriv();
    let left_max_dx_dt = quad_max_between(l2.x, l1.x, l0.x, 0.0..=t1_left);

    let t1_right = solve_t_for_y(right, y1);
    let QuadBez {
        p0: r0,
        p1: r1,
        p2: r2,
    } = right.deriv();
    let right_min_dx_dt = quad_min_between(r2.x, r1.x, r0.x, 0.0..=t1_right);

    if left_max_dx_dt < 0.0 && right_min_dx_dt > 0.0 {
        return true;
    }

    let left_dy_dt = if left_max_dx_dt < 0.0 {
        quad_max_between(l2.y, l1.y, l0.y, 0.0..=t1_left)
    } else {
        quad_min_between(l2.y, l1.y, l0.y, 0.0..=t1_left)
    };

    let right_dy_dt = if right_min_dx_dt < 0.0 {
        quad_min_between(r2.y, r1.y, r0.y, 0.0..=t1_right)
    } else {
        quad_max_between(r2.y, r1.y, r0.y, 0.0..=t1_right)
    };

    left_max_dx_dt * right_dy_dt < right_min_dx_dt * left_dy_dt
}

pub fn transversal_before(left: CubicBez, right: CubicBez, y0: f64) -> bool {
    // Reflect everything in y.
    let p = |q: kurbo::Point| kurbo::Point { x: q.x, y: -q.y };
    let c = |d: CubicBez| CubicBez {
        p0: p(d.p3),
        p1: p(d.p2),
        p2: p(d.p1),
        p3: p(d.p0),
    };
    transversal_after(c(left), c(right), -y0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_transversal() {
        let left = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 1.).into(),
            p2: (-1., 1.).into(),
            p3: (0., 2.).into(),
        };
        let right = CubicBez {
            p0: (0., 0.).into(),
            p1: (1., 1.).into(),
            p2: (1., 1.).into(),
            p3: (0., 2.).into(),
        };

        assert!(transversal_after(left, right, 0.5));

        // It isn't really necessary for the implementation to pass
        // these two tests; they just validate that our particular
        // implementation does what we think. Our implementation looks
        // only at derivatives, and the derivatives change direction
        // at y = 1.0.
        assert!(transversal_after(left, right, 0.9));
        assert!(!transversal_after(left, right, 1.1));
    }

    #[test]
    fn parallel() {
        let left = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 1.).into(),
            p2: (-1., 1.).into(),
            p3: (0., 2.).into(),
        };
        let right = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 1.).into(),
            p2: (1., 1.).into(),
            p3: (0., 2.).into(),
        };

        assert!(!transversal_after(left, right, 0.01));
    }

    #[test]
    fn parallel_and_horizontal() {
        let left = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 0.).into(),
            p2: (-1., 1.).into(),
            p3: (0., 2.).into(),
        };
        let right = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 0.).into(),
            p2: (1., 1.).into(),
            p3: (0., 2.).into(),
        };

        assert!(!transversal_after(left, right, 0.01));
    }

    #[test]
    fn antiparallel_and_horizontal() {
        let left = CubicBez {
            p0: (0., 0.).into(),
            p1: (-1., 0.).into(),
            p2: (-1., 1.).into(),
            p3: (0., 2.).into(),
        };
        let right = CubicBez {
            p0: (0., 0.).into(),
            p1: (1., 0.).into(),
            p2: (1., 1.).into(),
            p3: (0., 2.).into(),
        };

        assert!(transversal_after(left, right, 0.01));
    }
}
