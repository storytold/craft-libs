//! The canvas outside a crop frame, as the convex pieces the shield fills.

use craft_behavior::geometry::crop::turn;
use craft_behavior::geometry::outside_pieces;
use kurbo::Point;

const CANVAS: [Point; 4] = [Point::new(0.0, 0.0), Point::new(400.0, 0.0), Point::new(400.0, 300.0), Point::new(0.0, 300.0)];

fn quad(x0: f64, y0: f64, x1: f64, y1: f64) -> Vec<Point> {
    vec![Point::new(x0, y0), Point::new(x1, y0), Point::new(x1, y1), Point::new(x0, y1)]
}

fn area(poly: &[Point]) -> f64 {
    poly.iter().zip(poly.iter().cycle().skip(1)).map(|(p, q)| p.x * q.y - q.x * p.y).sum::<f64>().abs() / 2.0
}

/// Whether `p` lies strictly inside the convex `poly`, either way round; repeated points are skipped.
fn contains(poly: &[Point], p: Point) -> bool {
    let sides: Vec<f64> =
        poly.iter().zip(poly.iter().cycle().skip(1)).filter(|(a, b)| a.distance(**b) > 1e-12).map(|(a, b)| (*b - *a).cross(p - *a)).collect();
    sides.iter().all(|s| *s > 0.0) || sides.iter().all(|s| *s < 0.0)
}

/// How far `p` is from the outline of `poly`.
fn to_outline(poly: &[Point], p: Point) -> f64 {
    poly.iter()
        .zip(poly.iter().cycle().skip(1))
        .map(|(a, b)| {
            let d = *b - *a;
            let t = ((p - *a).dot(d) / d.hypot2()).clamp(0.0, 1.0);
            p.distance(*a + d * t)
        })
        .fold(f64::INFINITY, f64::min)
}

/// Every sample point of the canvas outside `hole` lies in exactly one piece, every one inside it in none, and the
/// pieces' areas add up to the canvas's outside the hole (`hole_on_canvas`, the hole's area on the canvas).
fn covers_once(hole: &[Point], hole_on_canvas: f64) {
    let pieces = outside_pieces(CANVAS, hole);
    for i in 0..40 {
        for j in 0..30 {
            let p = Point::new(f64::from(i) * 10.0 + 3.7, f64::from(j) * 10.0 + 4.3);
            if to_outline(hole, p) < 1e-6 {
                continue;
            }
            let count = pieces.iter().filter(|piece| contains(piece, p)).count();
            let want = usize::from(!contains(hole, p));
            assert_eq!(count, want, "{p:?} with the hole {hole:?}: in {count} pieces");
        }
    }
    let total: f64 = pieces.iter().map(|piece| area(piece)).sum();
    assert!((total - (area(&CANVAS) - hole_on_canvas)).abs() < 1e-6, "{total} for the hole {hole:?}");
}

#[test]
fn outside_pieces_cover_the_canvas_outside_the_hole_once() {
    // An upright frame, the same frame given the other way round, and turned frames.
    let upright = quad(100.0, 80.0, 300.0, 220.0);
    covers_once(&upright, 200.0 * 140.0);
    covers_once(&upright.iter().rev().copied().collect::<Vec<_>>(), 200.0 * 140.0);
    for angle in [15.0, -40.0, 90.0] {
        let turned: Vec<Point> = upright.iter().map(|p| turn(*p, Point::new(200.0, 150.0), angle)).collect();
        covers_once(&turned, 200.0 * 140.0);
    }
    // A hole past the canvas edge, one off the canvas, and one that covers it all.
    covers_once(&quad(300.0, 200.0, 500.0, 400.0), 100.0 * 100.0);
    covers_once(&quad(500.0, 0.0, 600.0, 100.0), 0.0);
    assert!(outside_pieces(CANVAS, &quad(-10.0, -10.0, 410.0, 310.0)).is_empty(), "nothing of the canvas is outside");
    // Holes with no area, too few points, or points that are not finite give nothing.
    let point = Point::new(50.0, 50.0);
    for hole in [
        vec![],
        vec![point, Point::new(80.0, 80.0)],
        vec![point, Point::new(80.0, 80.0), Point::new(110.0, 110.0)],
        vec![point; 4],
        vec![point, Point::new(f64::NAN, 80.0), Point::new(110.0, 50.0), Point::new(80.0, 20.0)],
        vec![point, Point::new(f64::INFINITY, 80.0), Point::new(110.0, 50.0)],
    ] {
        assert!(outside_pieces(CANVAS, &hole).is_empty(), "{hole:?}");
    }
}
