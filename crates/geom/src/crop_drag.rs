//! Interactive crop editing: what dragging a crop handle does.
//!
//! The crop rectangle lives in the straightened (rotated) frame and must stay inside the rotated
//! image. A drag is a *domain operation* on the crop, not a UI detail: the handle being dragged
//! decides which edges are anchored, an optional aspect lock decides the shape, and the image
//! bounds decide how far the drag may go. [`drag_crop`] answers all three in one place so the app,
//! CLI, control channel and MCP agree.

use crate::{Affine, CropGeometry, Point, Rect, Vec2};

/// Smallest crop side, as a fraction of the image's shorter side.
pub const MIN_CROP_FRACTION: f64 = 0.02;

/// Most elongated locked crop shape (width:height or its inverse) the drag supports.
pub const MAX_RATIO: f64 = 20.0;

/// A grab point on the crop frame.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CropHandle {
    TopLeft,
    TopRight,
    BottomRight,
    BottomLeft,
    Top,
    Right,
    Bottom,
    Left,
    /// Drag the whole frame.
    Move,
}

impl CropHandle {
    /// The overlay's numbering: 0..3 corners (clockwise from top-left), 4..7 edges (top, right,
    /// bottom, left), 8 = move.
    pub fn from_index(i: u8) -> Option<CropHandle> {
        Some(match i {
            0 => CropHandle::TopLeft,
            1 => CropHandle::TopRight,
            2 => CropHandle::BottomRight,
            3 => CropHandle::BottomLeft,
            4 => CropHandle::Top,
            5 => CropHandle::Right,
            6 => CropHandle::Bottom,
            7 => CropHandle::Left,
            8 => CropHandle::Move,
            _ => return None,
        })
    }

    /// Parse a command-parameter name (`"topLeft"`, `"top"`, `"move"`, …).
    pub fn from_name(n: &str) -> Option<CropHandle> {
        Some(match n {
            "topLeft" => CropHandle::TopLeft,
            "topRight" => CropHandle::TopRight,
            "bottomRight" => CropHandle::BottomRight,
            "bottomLeft" => CropHandle::BottomLeft,
            "top" => CropHandle::Top,
            "right" => CropHandle::Right,
            "bottom" => CropHandle::Bottom,
            "left" => CropHandle::Left,
            "move" => CropHandle::Move,
            _ => return None,
        })
    }
}

/// Drag `handle` of `start` from `grab` to `pointer` (both normalized, in the straightened frame)
/// on a `w × h` image. `aspect` is the locked width/height ratio in pixels, if any.
///
/// Guarantees (for any input, including hostile numbers):
/// - the result lies inside the rotated image;
/// - the edges opposite the handle stay where they were (a move keeps the size);
/// - with `aspect`, the frame keeps that shape on every handle (when `start` had it);
/// - the drag stops at the image edge instead of pushing the frame around, and slides along it.
pub fn drag_crop(start: CropGeometry, handle: CropHandle, grab: Point, pointer: Point, w: f64, h: f64, aspect: Option<f64>) -> CropGeometry {
    let sane = |v: f64| v.is_finite();
    let r0 = start.rect;
    if ![w, h, grab.x, grab.y, pointer.x, pointer.y, start.angle, r0.x0, r0.y0, r0.x1, r0.y1].iter().copied().all(sane)
        || w <= 0.0
        || h <= 0.0
        || r0.is_empty()
        || aspect.is_some_and(|a| !(a.is_finite() && a > 0.0))
    {
        return start;
    }
    // Start from a valid frame, then work in pixels of the straightened frame.
    let mut base = start.constrained(w, h);
    if base.angle == 0.0 {
        // absorb the 1e-6 tolerance of `is_within_image` so the exact test below accepts the start
        let r = base.rect;
        base.rect = Rect::new(r.x0.clamp(0.0, 1.0), r.y0.clamp(0.0, 1.0), r.x1.clamp(0.0, 1.0), r.y1.clamp(0.0, 1.0));
    }
    let sr = base.rect_px(w, h);
    // Moving further than twice the image never matters; clamping keeps the searches well-conditioned.
    let (dx, dy) = (((pointer.x - grab.x) * w).clamp(-2.0 * w, 2.0 * w), ((pointer.y - grab.y) * h).clamp(-2.0 * h, 2.0 * h));
    // Ratios beyond 20:1 make even the minimum frame larger than the image; treat them as 20:1.
    let aspect = aspect.map(|a| a.clamp(1.0 / MAX_RATIO, MAX_RATIO));
    // Unrotated, the frame must be exactly inside the image (`is_within_image` tolerates 1e-6, which
    // would leak out as a sliver past the edge and, after any clamp, a shifted ratio).
    let feasible = |r: Rect| {
        if base.angle == 0.0 {
            r.x0 >= 0.0 && r.y0 >= 0.0 && r.x1 <= w && r.y1 <= h
        } else {
            CropGeometry { rect: r.scale(1.0 / w, 1.0 / h), angle: base.angle }.is_within_image(w, h)
        }
    };
    let min_side = MIN_CROP_FRACTION * w.min(h);

    let out = match (handle, aspect) {
        (CropHandle::Move, _) => {
            let slide = |cur: Rect, ax: f64, ay: f64| {
                let t = bisect(0.0, 1.0, |t| feasible(cur.translate(Vec2::new(ax * t, ay * t))));
                cur.translate(Vec2::new(ax * t, ay * t))
            };
            // straightened, go to the nearest place that fits, so the frame slides along tilted edges (storytold/lightcraft#2299)
            let nearest = if base.angle != 0.0 {
                let from = |r: Rect| {
                    let corners = r.corners().map(|c| (c, Vec2::new(1.0, 1.0)));
                    nearest_offset(&corners, &[], base.angle, w, h, Vec2::new(dx, dy)).map(|v| r.translate(v)).filter(|r| feasible(*r))
                };
                // a frame at the tolerance's edge on opposite sides moves only once it is inside it
                from(sr).or_else(|| shrunk_inside(sr, base.angle, w, h).and_then(from))
            } else {
                None
            };
            match nearest {
                Some(r) => r,
                None => slide(slide(sr, dx, 0.0), 0.0, dy),
            }
        }
        (hd, None) => {
            let (mx0, mx1, my0, my1) = sides(hd);
            let grow = |cur: Rect, tx: f64, ty: f64| {
                // `tx`/`ty`: how far the moving edges would go; interpolate towards it
                let target = Rect::new(
                    if mx0 { (cur.x0 + tx).min(cur.x1 - min_side) } else { cur.x0 },
                    if my0 { (cur.y0 + ty).min(cur.y1 - min_side) } else { cur.y0 },
                    if mx1 { (cur.x1 + tx).max(cur.x0 + min_side) } else { cur.x1 },
                    if my1 { (cur.y1 + ty).max(cur.y0 + min_side) } else { cur.y1 },
                );
                let at = |t: f64| lerp_rect(cur, target, t);
                at(bisect(0.0, 1.0, |t| feasible(at(t))))
            };
            // straightened, a free corner goes to the nearest place that fits, growing both sides (storytold/lightcraft#2308)
            let corner = (mx0 || mx1) && (my0 || my1);
            let nearest =
                if base.angle != 0.0 && corner { nearest_corner(sr, mx0, my0, base.angle, (w, h), min_side, Vec2::new(dx, dy)) } else { None };
            match nearest.filter(|r| feasible(*r)) {
                Some(r) => r,
                None => grow(grow(sr, dx, 0.0), 0.0, dy),
            }
        }
        (hd, Some(a)) => {
            let (mx0, _, my0, _) = sides(hd);
            let (min_w, min_h) = (min_side.max(min_side * a), min_side.max(min_side / a));
            let c = sr.center();
            // Anchor: the opposite corner / edge midpoint. `build(n, o)` is the frame of width `n` that
            // keeps it, shifted by `o` along the anchored edge (edge handles only).
            let (target_w, build): (f64, Box<dyn Fn(f64, f64) -> Rect>) = match hd {
                CropHandle::Top | CropHandle::Bottom => {
                    let (ay, sy, edge) = if my0 { (sr.y1, -1.0, sr.y0) } else { (sr.y0, 1.0, sr.y1) };
                    let nh = (sy * (edge + dy - ay)).max(min_h);
                    (nh * a, Box::new(move |nw, o| vband(c.x + o, ay, sy, nw, a)))
                }
                CropHandle::Left | CropHandle::Right => {
                    let (ax, sx, edge) = if mx0 { (sr.x1, -1.0, sr.x0) } else { (sr.x0, 1.0, sr.x1) };
                    let nw = (sx * (edge + dx - ax)).max(min_w);
                    (nw, Box::new(move |nw, o| hband(ax, sx, c.y + o, nw, a)))
                }
                _ => {
                    let (ax, sx, ex) = if mx0 { (sr.x1, -1.0, sr.x0) } else { (sr.x0, 1.0, sr.x1) };
                    let (ay, sy, ey) = if my0 { (sr.y1, -1.0, sr.y0) } else { (sr.y0, 1.0, sr.y1) };
                    let (wx, wy) = (sx * (ex + dx - ax), sy * (ey + dy - ay));
                    let nw = ((wx + wy * a) / 2.0).max(min_w);
                    (nw, Box::new(move |nw, _| corner_rect(ax, sx, ay, sy, nw, a)))
                }
            };
            let target_w = target_w.min(2.0 * w.max(h * a));
            let slides = matches!(hd, CropHandle::Top | CropHandle::Bottom | CropHandle::Left | CropHandle::Right);
            let extent = if matches!(hd, CropHandle::Top | CropHandle::Bottom) { w } else { h };
            // Frame of width `n`, centred on the anchored edge if it fits, else slid the least distance.
            let place = |n: f64| -> Option<Rect> {
                let at = |o: f64| build(n, o);
                if feasible(at(0.0)) {
                    return Some(at(0.0));
                }
                if !slides {
                    return None;
                }
                const STEPS: u32 = 128;
                let step = extent / STEPS as f64;
                for k in 1..=STEPS {
                    for sign in [1.0, -1.0] {
                        let hi = sign * k as f64 * step;
                        if feasible(at(hi)) {
                            // closest feasible offset: bisect between the last infeasible and this one
                            let (mut bad, mut good) = (sign * (k - 1) as f64 * step, hi);
                            for _ in 0..40 {
                                let mid = (bad + good) / 2.0;
                                if feasible(at(mid)) {
                                    good = mid;
                                } else {
                                    bad = mid;
                                }
                            }
                            return Some(at(good));
                        }
                    }
                }
                None
            };
            if place(min_w).is_some() {
                let n = bisect_range(min_w, target_w.max(min_w), |n| place(n).is_some());
                place(n).unwrap_or(sr)
            } else {
                sr
            }
        }
    };
    let rect = out.scale(1.0 / w, 1.0 / h);
    if [rect.x0, rect.y0, rect.x1, rect.y1].iter().all(|v| v.is_finite()) { CropGeometry { rect, angle: base.angle } } else { start }
}

/// The offset `v` (pixels) nearest `want` that keeps each `p + m ⊙ v` inside a `w × h` image turned by `angle`
/// degrees and meets the `extra` half-planes `n · v <= b`.
fn nearest_offset(points: &[(Point, Vec2)], extra: &[(Vec2, f64)], angle: f64, w: f64, h: f64, want: Vec2) -> Option<Vec2> {
    let back = Affine::rotate_about(-angle.to_radians(), Point::new(w / 2.0, h / 2.0));
    let (ex, ey) = (back.apply_vec(Vec2::new(1.0, 0.0)), back.apply_vec(Vec2::new(0.0, 1.0)));
    // never tighter than where the frame is, plus a slack so a zero-width strip survives rounding
    let (sx, sy) = (1e-9 * w, 1e-9 * h);
    let mut planes = Vec::with_capacity(4 * points.len() + extra.len());
    for (p, m) in points {
        let q = back.apply(*p);
        let (nx, ny) = (Vec2::new(ex.x * m.x, ey.x * m.y), Vec2::new(ex.y * m.x, ey.y * m.y));
        for (n, b, s) in [(nx, w - q.x, sx), (-nx, q.x, sx), (ny, h - q.y, sy), (-ny, q.y, sy)] {
            planes.push((n, b.max(0.0) + s));
        }
    }
    planes.extend_from_slice(extra);
    if planes.iter().all(|(n, b)| n.dot(want) <= *b) {
        return Some(want);
    }
    let lim = 4.0 * w.max(h);
    let mut poly = vec![Vec2::new(-lim, -lim), Vec2::new(lim, -lim), Vec2::new(lim, lim), Vec2::new(-lim, lim)];
    for (n, b) in &planes {
        poly = clip_half_plane(&poly, *n, *b);
    }
    poly.iter()
        .zip(poly.iter().cycle().skip(1))
        .map(|(a, b)| {
            let ab = *b - *a;
            let l2 = ab.dot(ab);
            let t = if l2 > 0.0 { ((want - *a).dot(ab) / l2).clamp(0.0, 1.0) } else { 0.0 };
            *a + ab * t
        })
        .filter(|v| v.x.is_finite() && v.y.is_finite())
        .min_by(|a, b| (*a - want).len().total_cmp(&(*b - want).len()))
}

/// `r` scaled about its centre by the largest factor down to `1 - 1e-5` that puts it exactly inside a `w × h` image
/// turned by `angle` degrees.
fn shrunk_inside(r: Rect, angle: f64, w: f64, h: f64) -> Option<Rect> {
    let back = Affine::rotate_about(-angle.to_radians(), Point::new(w / 2.0, h / 2.0));
    let scaled = |k: f64| Rect::from_center(r.center(), r.width() * k, r.height() * k);
    let inside = |k: f64| {
        scaled(k).corners().iter().all(|c| {
            let q = back.apply(*c);
            q.x >= 0.0 && q.y >= 0.0 && q.x <= w && q.y <= h
        })
    };
    let lo = 1.0 - 1e-5;
    inside(lo).then(|| scaled(bisect_range(lo, 1.0, inside)))
}

/// The frame `r` (pixels) with a corner dragged by `want`, the opposite one held, at the nearest place that fits.
fn nearest_corner(r: Rect, left: bool, top: bool, angle: f64, (w, h): (f64, f64), min_side: f64, want: Vec2) -> Option<Rect> {
    let anchor = Point::new(if left { r.x1 } else { r.x0 }, if top { r.y1 } else { r.y0 });
    let corner = Point::new(if left { r.x0 } else { r.x1 }, if top { r.y0 } else { r.y1 });
    let points =
        [(corner, Vec2::new(1.0, 1.0)), (Point::new(corner.x, anchor.y), Vec2::new(1.0, 0.0)), (Point::new(anchor.x, corner.y), Vec2::new(0.0, 1.0))];
    let (sx, sy) = (if left { -1.0 } else { 1.0 }, if top { -1.0 } else { 1.0 });
    // at least `min_side`, and per axis only between where it is and where the pointer asks
    let extra = [
        (Vec2::new(-sx, 0.0), sx * (corner.x - anchor.x) - min_side),
        (Vec2::new(0.0, -sy), sy * (corner.y - anchor.y) - min_side),
        (Vec2::new(1.0, 0.0), want.x.max(0.0)),
        (Vec2::new(-1.0, 0.0), (-want.x).max(0.0)),
        (Vec2::new(0.0, 1.0), want.y.max(0.0)),
        (Vec2::new(0.0, -1.0), (-want.y).max(0.0)),
    ];
    let v = nearest_offset(&points, &extra, angle, w, h, want)?;
    Some(Rect::from_points(anchor, corner + v))
}

/// The part of the convex polygon `poly` where `n · v <= b`.
fn clip_half_plane(poly: &[Vec2], n: Vec2, b: f64) -> Vec<Vec2> {
    let mut out = Vec::with_capacity(poly.len() + 1);
    for (p, q) in poly.iter().zip(poly.iter().cycle().skip(1)) {
        let (dp, dq) = (n.dot(*p) - b, n.dot(*q) - b);
        if dp <= 0.0 {
            out.push(*p);
        }
        if (dp <= 0.0) != (dq <= 0.0) {
            out.push(*p + (*q - *p) * (dp / (dp - dq)));
        }
    }
    out
}

/// Which sides a handle moves: (left, right, top, bottom).
fn sides(h: CropHandle) -> (bool, bool, bool, bool) {
    use CropHandle::*;
    (
        matches!(h, TopLeft | BottomLeft | Left),
        matches!(h, TopRight | BottomRight | Right),
        matches!(h, TopLeft | TopRight | Top),
        matches!(h, BottomLeft | BottomRight | Bottom),
    )
}

fn lerp_rect(a: Rect, b: Rect, t: f64) -> Rect {
    let l = |p: f64, q: f64| p + (q - p) * t;
    Rect::new(l(a.x0, b.x0), l(a.y0, b.y0), l(a.x1, b.x1), l(a.y1, b.y1))
}

/// Frame of width `nw` and ratio `a` extending from corner (`ax`, `ay`) in directions `sx`, `sy`.
fn corner_rect(ax: f64, sx: f64, ay: f64, sy: f64, nw: f64, a: f64) -> Rect {
    let nh = nw / a;
    Rect::from_points(Point::new(ax, ay), Point::new(ax + sx * nw, ay + sy * nh))
}

/// Frame whose top/bottom edge (at `ay`, growing in direction `sy`) is anchored, centred on `cx`.
fn vband(cx: f64, ay: f64, sy: f64, nw: f64, a: f64) -> Rect {
    let nh = nw / a;
    Rect::from_points(Point::new(cx - nw / 2.0, ay), Point::new(cx + nw / 2.0, ay + sy * nh))
}

/// Frame whose left/right edge (at `ax`, growing in direction `sx`) is anchored, centred on `cy`.
fn hband(ax: f64, sx: f64, cy: f64, nw: f64, a: f64) -> Rect {
    let nh = nw / a;
    Rect::from_points(Point::new(ax, cy - nh / 2.0), Point::new(ax + sx * nw, cy + nh / 2.0))
}

/// Largest `t` in `[lo, hi]` for which `ok` holds, given that it holds at `lo` and the set of good
/// `t` is an interval (the feasible region is convex).
fn bisect_range(lo: f64, hi: f64, ok: impl Fn(f64) -> bool) -> f64 {
    if ok(hi) {
        return hi;
    }
    let (mut good, mut bad) = (lo, hi);
    for _ in 0..48 {
        let mid = (good + bad) / 2.0;
        if ok(mid) {
            good = mid;
        } else {
            bad = mid;
        }
    }
    good
}

fn bisect(lo: f64, hi: f64, ok: impl Fn(f64) -> bool) -> f64 {
    bisect_range(lo, hi, ok)
}

#[cfg(test)]
mod tests;
