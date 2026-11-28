//! Plane geometry the canvas overlays share, in document or screen units: segment clipping. Pure maths beside
//! [`quad`](super::quad), on kurbo types; curve flattening is kurbo's.

use kurbo::{Point, Rect};

/// The part of segment `a`–`b` inside `rect` (Liang–Barsky), or `None` when none is.
pub fn clip_segment(a: Point, b: Point, rect: Rect) -> Option<(Point, Point)> {
    let d = b - a;
    let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
    // Each edge as (the run toward it, the room before it).
    for (run, room) in [(-d.x, a.x - rect.x0), (d.x, rect.x1 - a.x), (-d.y, a.y - rect.y0), (d.y, rect.y1 - a.y)] {
        if run == 0.0 {
            if room < 0.0 {
                return None;
            }
        } else if run < 0.0 {
            t0 = t0.max(room / run);
        } else {
            t1 = t1.min(room / run);
        }
    }
    (t0 <= t1).then(|| (a + d * t0, a + d * t1))
}
