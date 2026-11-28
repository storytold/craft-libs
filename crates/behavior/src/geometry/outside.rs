//! The canvas outside a convex hole, as convex pieces.

use kurbo::Point;

/// Share of the canvas's area under which a piece or a hole counts as having none: rounding at the hole's edges.
const TINY_AREA: f64 = 1e-12;

/// Convex pieces that cover the canvas quad outside the convex `hole` exactly once: the canvas clipped by each
/// hole edge's outer half-plane in turn. Empty for a hole with fewer than three finite points or no area; the whole
/// canvas for a hole outside it.
pub fn outside_pieces(canvas: [Point; 4], hole: &[Point]) -> Vec<Vec<Point>> {
    let finite = |p: &Point| p.x.is_finite() && p.y.is_finite();
    if hole.len() < 3 || !hole.iter().all(finite) || !canvas.iter().all(finite) {
        return Vec::new();
    }
    let canvas_area = area(&canvas).abs();
    let tiny = TINY_AREA * canvas_area.max(1.0);
    let hole_area = area(hole);
    if !(canvas_area > tiny && hole_area.abs() > tiny) {
        return Vec::new();
    }
    // The hole's inside lies left of each edge (positive cross product) when its points run counter-clockwise in the
    // plane's own orientation; a hole given the other way round has it on the right.
    let inside_left = hole_area > 0.0;
    let mut rest = canvas.to_vec();
    let mut pieces = Vec::new();
    for (&a, &b) in hole.iter().zip(hole.iter().cycle().skip(1)) {
        // A repeated point has no edge to cut along.
        if (b - a).hypot2() <= f64::EPSILON * canvas_area.max(1.0) {
            continue;
        }
        let outside = clipped(&rest, a, b, !inside_left);
        if area(&outside).abs() > tiny {
            pieces.push(outside);
        }
        rest = clipped(&rest, a, b, inside_left);
        if area(&rest).abs() <= tiny {
            break;
        }
    }
    pieces
}

/// `poly` cut to the side of the line from `a` to `b` that is to the left (`left`) or the right of it, points on the
/// line kept (Sutherland–Hodgman against one half-plane).
fn clipped(poly: &[Point], a: Point, b: Point, left: bool) -> Vec<Point> {
    let side = |p: Point| {
        let c = (b - a).cross(p - a);
        if left { c } else { -c }
    };
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (&p, &q) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        let (sp, sq) = (side(p), side(q));
        if sp >= 0.0 {
            out.push(p);
        }
        // The signs differ, so `sp - sq` is not zero.
        if (sp >= 0.0) != (sq >= 0.0) {
            out.push(p.lerp(q, sp / (sp - sq)));
        }
    }
    out
}

/// The signed area of `poly`: positive when its points run counter-clockwise with y up (clockwise on a y-down screen).
fn area(poly: &[Point]) -> f64 {
    poly.iter().zip(poly.iter().cycle().skip(1)).map(|(p, q)| p.x * q.y - q.x * p.y).sum::<f64>() / 2.0
}
