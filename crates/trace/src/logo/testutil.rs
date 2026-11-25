//! Shared synthetic shapes for tests (no brand artwork: everything is made here from scratch).

use craft_geom::Point;

/// A square from (10, 10) to (30, 30) with corners rounded to radius `rad`, as a dense loop.
pub(crate) fn rounded_square(rad: f64) -> Vec<Point> {
    let mut pts = Vec::new();
    let corners = [(30.0 - rad, 10.0 + rad, -90.0), (30.0 - rad, 30.0 - rad, 0.0), (10.0 + rad, 30.0 - rad, 90.0), (10.0 + rad, 10.0 + rad, 180.0)];
    for (cx, cy, start) in corners {
        for k in 0..=24 {
            let a = (start + 90.0 * k as f64 / 24.0).to_radians();
            pts.push(Point::new(cx + rad * a.cos(), cy + rad * a.sin()));
        }
    }
    pts
}

/// A circle of radius `r` about (`cx`, `cy`) with `n` points.
pub(crate) fn circle(cx: f64, cy: f64, r: f64, n: usize) -> Vec<Point> {
    (0..n)
        .map(|i| {
            Point::new(cx + r * (i as f64 / n as f64 * std::f64::consts::TAU).cos(), cy + r * (i as f64 / n as f64 * std::f64::consts::TAU).sin())
        })
        .collect()
}
