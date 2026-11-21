//! Shape generators matching Illustrator's anchor layouts (so later edits behave the same).

use std::f64::consts::{PI, TAU};

use kurbo::{Point, Rect, Vec2};
use serde::{Deserialize, Serialize};

use crate::path::{Anchor, AnchorKind, PathData, SubPath};

/// Circle-approximation handle ratio for a quarter arc.
pub const KAPPA: f64 = 0.552_284_749_830_793_6;

/// Rectangle with 4 corner anchors, clockwise from the top-left.
pub fn rectangle(r: Rect) -> PathData {
    let r = r.abs();
    PathData::single(SubPath::polyline(&[Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1)], true))
}

/// Rounded rectangle (8 anchors). The radius is clamped to half the shorter side.
pub fn rounded_rectangle(r: Rect, radius: f64) -> PathData {
    rectangle_with_corners(r, [radius; 4], [CornerKind::Round; 4])
}

/// How a rectangle's corner is cut (Live Corners).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CornerKind {
    /// A quarter circle, bulging outwards.
    #[default]
    Round,
    /// A quarter circle cut into the shape, centred on the corner.
    InvertedRound,
    /// A straight bevel.
    Chamfer,
}

impl CornerKind {
    pub const ALL: [CornerKind; 3] = [CornerKind::Round, CornerKind::InvertedRound, CornerKind::Chamfer];

    /// The kind after this one (Alt-clicking a corner widget cycles through them).
    pub fn next(self) -> Self {
        match self {
            CornerKind::Round => CornerKind::InvertedRound,
            CornerKind::InvertedRound => CornerKind::Chamfer,
            CornerKind::Chamfer => CornerKind::Round,
        }
    }

    /// The kind before this one (the Down arrow while dragging a corner widget).
    pub fn previous(self) -> Self {
        match self {
            CornerKind::Round => CornerKind::Chamfer,
            CornerKind::InvertedRound => CornerKind::Round,
            CornerKind::Chamfer => CornerKind::InvertedRound,
        }
    }
}

/// The largest corner radius a `w` × `h` rectangle draws: half its shorter side.
pub fn max_corner_radius(w: f64, h: f64) -> f64 {
    w.abs().min(h.abs()) / 2.0
}

/// The radii the corners of a `w` × `h` rectangle (top-left, top-right, bottom-right,
/// bottom-left) are drawn with: each its own, no larger than leaves room for its neighbours'
/// cuts ([`crate::corners::fitted_radii`]): half the shorter side when they are all as large, up
/// to a whole side when its neighbours are square (#2332); 0 when negative or not a number.
pub fn fitted_corner_radii(w: f64, h: f64, radii: [f64; 4]) -> [f64; 4] {
    let corners = crate::corners::path_corners(&rectangle(Rect::new(0.0, 0.0, w, h)));
    let mut out = [0.0; 4];
    for (c, r) in corners.iter().zip(crate::corners::fitted_radii(&corners, &radii)) {
        if let Some(o) = out.get_mut(c.index) {
            *o = r;
        }
    }
    out
}

/// Rectangle whose corners (top-left, top-right, bottom-right, bottom-left, y down) each have
/// their own radius and kind, every radius no larger than [`fitted_corner_radii`] leaves. A corner without a
/// radius is one anchor, the others two (where the cut meets each side). Clockwise from the
/// top-left corner, its anchor on the top side first. Cut as any path's corners are
/// ([`crate::corners::cut_corners`]).
pub fn rectangle_with_corners(r: Rect, radii: [f64; 4], kinds: [CornerKind; 4]) -> PathData {
    crate::corners::cut_corners(&rectangle(r), &radii, &kinds).0
}

/// Ellipse inscribed in `r` with 4 smooth anchors (left, top, right, bottom).
pub fn ellipse(r: Rect) -> PathData {
    let r = r.abs();
    let c = r.center();
    let rx = r.width() / 2.0;
    let ry = r.height() / 2.0;
    let kx = rx * KAPPA;
    let ky = ry * KAPPA;
    let s = |p: Point, hin: Vec2| Anchor { p, h_in: p + hin, h_out: p - hin, kind: AnchorKind::Smooth };
    let anchors = vec![
        s(Point::new(c.x - rx, c.y), Vec2::new(0.0, ky)),
        s(Point::new(c.x, c.y - ry), Vec2::new(-kx, 0.0)),
        s(Point::new(c.x + rx, c.y), Vec2::new(0.0, -ky)),
        s(Point::new(c.x, c.y + ry), Vec2::new(kx, 0.0)),
    ];
    PathData::single(SubPath::new(anchors, true))
}

/// Ellipse inscribed in `r` with `n` smooth anchors (at least 2) spaced evenly by angle, clockwise
/// on screen from the left one, as [`ellipse`] runs. Each segment is a cubic whose handles are
/// 4/3·tan(θ/4) of the tangent at its ends (θ = 360°/`n`), scaled along each axis; the path is then
/// fitted to `r`, so its bounds are `r` whatever the count. 4 anchors give [`ellipse`].
pub fn ellipse_with_anchors(r: Rect, n: u32) -> PathData {
    let n = n.max(2);
    let r = r.abs();
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    let step = TAU / f64::from(n);
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let anchors = (0..n)
        .map(|i| {
            // From the left anchor (angle π). With y down, sin is negative just past π, so the next
            // anchor is above the centre: left, then top, then right, as `ellipse` runs.
            let a = PI + step * f64::from(i);
            let p = Point::new(c.x + rx * a.cos(), c.y + ry * a.sin());
            let d = Vec2::new(-rx * a.sin(), ry * a.cos()) * k;
            Anchor { p, h_in: p - d, h_out: p + d, kind: AnchorKind::Smooth }
        })
        .collect();
    let path = PathData::single(SubPath::new(anchors, true));
    // Between anchors a curve can reach a little past the ellipse or stop short of it (0.15% of
    // the height with 3 anchors): fit the bounds to `r` along each axis that has a size.
    let Some(b) = path.bounds() else { return path };
    let fit = |want: f64, got: f64| if want > 0.0 && got > 0.0 { want / got } else { 1.0 };
    let (sx, sy) = (fit(r.width(), b.width()), fit(r.height(), b.height()));
    path.transformed(
        kurbo::Affine::translate(r.origin().to_vec2()) * kurbo::Affine::scale_non_uniform(sx, sy) * kurbo::Affine::translate(-b.origin().to_vec2()),
    )
}

/// How an ellipse or shape arc is closed or left open when cut between start and end angles.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum EllipseMode {
    /// Closed sector through the centre (pie slice).
    #[default]
    Slice,
    /// Open arc curve without closing segments.
    Arc,
    /// Closed arc with a straight chord connecting the end back to the start.
    Chord,
    /// Whole closed shape (ignores start/end cut).
    Whole,
}

/// The ellipse inscribed in `r` cut as a pie: from `start` to `end` degrees, measured
/// counterclockwise on screen from 3 o'clock (as Illustrator's Pie Start and End Angle are), then
/// back through the centre. A pie spanning the whole turn (or nothing) is the whole [`ellipse`].
/// The arc is one cubic per quarter turn at most: the centre, then the arc's anchors.
pub fn ellipse_pie(r: Rect, start: f64, end: f64) -> PathData {
    ellipse_arc(r, start, end, EllipseMode::Slice)
}

/// The ellipse inscribed in `r` cut from `start` to `end` degrees according to `mode`:
/// - [`EllipseMode::Slice`]: closed through the centre (pie chart).
/// - [`EllipseMode::Chord`]: closed with a straight chord between the endpoints.
/// - [`EllipseMode::Arc`]: open elliptical arc (no centre lines, `closed = false`).
/// - [`EllipseMode::Whole`]: full closed [`ellipse`].
pub fn ellipse_arc(r: Rect, start: f64, end: f64, mode: EllipseMode) -> PathData {
    if mode == EllipseMode::Whole || !(start.is_finite() && end.is_finite()) {
        return ellipse(r);
    }
    let raw_span = (end - start).rem_euclid(360.0);
    let full = raw_span < 1e-9 || 360.0 - raw_span < 1e-9;
    if full && mode != EllipseMode::Arc {
        return ellipse(r);
    }
    let span = if full { 360.0 } else { raw_span };
    let r = r.abs();
    let c = r.center();
    let (rx, ry) = (r.width() / 2.0, r.height() / 2.0);
    // Screen y runs down: counterclockwise on screen is a falling y.
    let at = |deg: f64| {
        let t = deg.to_radians();
        Point::new(c.x + rx * t.cos(), c.y - ry * t.sin())
    };
    let tangent = |deg: f64| {
        let t = deg.to_radians();
        Vec2::new(-rx * t.sin(), -ry * t.cos())
    };
    let pieces = if span <= 90.0 {
        1
    } else if span <= 180.0 {
        2
    } else if span <= 270.0 {
        3
    } else {
        4
    };
    let step = span / f64::from(pieces);
    let k = 4.0 / 3.0 * (step.to_radians() / 4.0).tan();
    let mut anchors = Vec::with_capacity(pieces as usize + 2);
    if mode == EllipseMode::Slice {
        anchors.push(Anchor::corner(c));
    }
    for i in 0..=pieces {
        let a = start + step * f64::from(i);
        let (p, d) = (at(a), tangent(a) * k);
        let h_in = if i == 0 { p } else { p - d };
        let h_out = if i == pieces { p } else { p + d };
        let kind = if i == 0 || i == pieces { AnchorKind::Corner } else { AnchorKind::Smooth };
        anchors.push(Anchor { p, h_in, h_out, kind });
    }
    let closed = mode != EllipseMode::Arc;
    PathData::single(SubPath::new(anchors, closed))
}

/// Cut or open any closed single-subpath `path` around `center` from `start_deg` to `end_deg`
/// according to `mode` ([`EllipseMode::Slice`], [`EllipseMode::Arc`], [`EllipseMode::Chord`],
/// [`EllipseMode::Whole`]).
pub fn apply_shape_arc(path: PathData, center: Point, start_deg: f64, end_deg: f64, mode: EllipseMode) -> PathData {
    use kurbo::{BezPath, ParamCurve};
    if mode == EllipseMode::Whole || !(start_deg.is_finite() && end_deg.is_finite()) {
        return path;
    }
    let Some(sp) = path.subpaths.first() else {
        return path;
    };
    let n = sp.segment_count();
    if !sp.closed || n == 0 {
        return path;
    }
    let raw_span = (end_deg - start_deg).rem_euclid(360.0);
    let full = raw_span < 1e-9 || 360.0 - raw_span < 1e-9;
    if full {
        if mode == EllipseMode::Arc {
            let mut open_sp = sp.clone();
            open_sp.closed = false;
            return PathData::single(open_sp);
        }
        return path;
    }
    let u0 = start_deg.rem_euclid(360.0) / 360.0;
    let du = (raw_span / 360.0).clamp(1e-6, 1.0 - 1e-6);
    let s0 = u0 * (n as f64);
    let s1 = (u0 + du) * (n as f64);
    let k0 = (s0.floor() as usize).min(n.saturating_sub(1));
    let t0 = (s0 - k0 as f64).clamp(0.0, 1.0);
    let k1 = s1.floor() as usize;
    let t1 = (s1 - k1 as f64).clamp(0.0, 1.0);
    let mut cubics = Vec::new();
    if k0 == k1 {
        if t1 > t0 + 1e-9 {
            cubics.push(sp.segment(k0 % n).subsegment(t0..t1));
        }
    } else {
        if t0 < 1.0 - 1e-9 {
            cubics.push(sp.segment(k0 % n).subsegment(t0..1.0));
        }
        for k in (k0 + 1)..k1 {
            cubics.push(sp.segment(k % n));
        }
        if t1 > 1e-9 {
            cubics.push(sp.segment(k1 % n).subsegment(0.0..t1));
        }
    }
    let Some(first) = cubics.first() else {
        return path;
    };
    let mut bp = BezPath::new();
    if mode == EllipseMode::Slice {
        bp.move_to(center);
        bp.line_to(first.p0);
    } else {
        bp.move_to(first.p0);
    }
    for c in &cubics {
        if c.p0.distance(c.p1) <= 1e-9 && c.p3.distance(c.p2) <= 1e-9 {
            bp.line_to(c.p3);
        } else {
            bp.curve_to(c.p1, c.p2, c.p3);
        }
    }
    if mode != EllipseMode::Arc {
        bp.close_path();
    }
    PathData::from_bezpath(&bp)
}

/// Deterministic vertex jitter for `randomized` polygons and stars (`(dr, da)` in `[-1, 1]`).
fn vertex_jitter(index: u32, total: u32) -> (f64, f64) {
    let mut s = 0x9e37_79b9_7f4a_7c15_u64
        .wrapping_add((u64::from(index) + 1).wrapping_mul(0xbf58_476d_1ce4_e5b9))
        .wrapping_add(u64::from(total).wrapping_mul(0x94d0_49bb_1331_11eb));
    let mut next = || {
        s ^= s >> 12;
        s ^= s << 25;
        s ^= s >> 27;
        let v = s.wrapping_mul(0x2545_f491_4f6c_dd1d);
        ((v >> 11) as f64 / ((1u64 << 53) as f64)) * 2.0 - 1.0
    };
    (next(), next())
}

/// Build a closed subpath from polygon/star vertices with optional smooth bezier `rounded`
/// tangents (`-10..=10`).
fn contour_with_rounding(pts: &[Point], rounded: f64) -> PathData {
    let n = pts.len();
    if n == 0 || !rounded.is_finite() || rounded.abs() < 1e-9 {
        return PathData::single(SubPath::polyline(pts, true));
    }
    let r_factor = rounded.clamp(-10.0, 10.0) * 0.4;
    let mut anchors = Vec::with_capacity(n);
    for i in 0..n {
        let p_prev = pts[(i + n - 1) % n];
        let p = pts[i];
        let p_next = pts[(i + 1) % n];
        let v_in = p - p_prev;
        let v_out = p_next - p;
        let len_in = v_in.hypot();
        let len_out = v_out.hypot();
        let u_in = if len_in > 1e-9 { v_in / len_in } else { Vec2::ZERO };
        let u_out = if len_out > 1e-9 { v_out / len_out } else { Vec2::ZERO };
        let sum = u_in + u_out;
        let sum_len = sum.hypot();
        let tangent = if sum_len > 1e-9 { sum / sum_len } else { Vec2::new(-u_in.y, u_in.x) };
        let h_in = p - tangent * (len_in * r_factor);
        let h_out = p + tangent * (len_out * r_factor);
        anchors.push(Anchor { p, h_in, h_out, kind: AnchorKind::Smooth });
    }
    PathData::single(SubPath::new(anchors, true))
}

/// The circle through the points `a`, `b` and `c`: its center and radius. `None` when two of the
/// points are the same or the three lie on one line, which this takes to be when twice the
/// triangle's area is at most 10⁻¹⁰ of its longest side squared.
pub fn circle_through(a: Point, b: Point, c: Point) -> Option<(Point, f64)> {
    // The corner opposite the longest side is the origin, so the two shorter sides are used.
    let (ab, bc, ca) = ((b - a).hypot2(), (c - b).hypot2(), (a - c).hypot2());
    let (o, p, q, longest) = if bc >= ab && bc >= ca {
        (a, b, c, bc)
    } else if ca >= ab {
        (b, c, a, ca)
    } else {
        (c, a, b, ab)
    };
    let (u, v) = (p - o, q - o);
    let cross = u.cross(v);
    // Not a number, or nearly zero: no circle (a not-a-number side fails the finite check below).
    if cross.is_nan() || cross.abs() <= 1e-10 * longest {
        return None;
    }
    let (uu, vv) = (u.hypot2(), v.hypot2());
    let d = 2.0 * cross;
    let offset = Vec2::new((v.y * uu - u.y * vv) / d, (u.x * vv - v.x * uu) / d);
    let center = o + offset;
    let radius = offset.hypot();
    (center.is_finite() && radius.is_finite()).then_some((center, radius))
}

/// The circle through two or three points: two are the ends of a diameter, and three lie on its
/// edge ([`circle_through`]). Its center and radius; `None` for any other number of points, for
/// points that are the same, for three on one line, and for a center or radius that is not finite.
pub fn circle_through_points(points: &[Point]) -> Option<(Point, f64)> {
    let circle = match *points {
        [a, b] if a != b => Some((a.midpoint(b), a.distance(b) / 2.0)),
        [a, b, c] => circle_through(a, b, c),
        _ => None,
    };
    circle.filter(|(c, r)| c.is_finite() && r.is_finite() && *r > 0.0)
}

/// Regular polygon with `sides` (≥ 3), first vertex straight up from the centre.
pub fn polygon(center: Point, radius: f64, sides: u32, rotation_deg: f64) -> PathData {
    let n = sides.max(3);
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + radius * a.cos(), center.y + radius * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Regular polygon with `sides` (≥ 3), bezier tangent `rounded` factor and deterministic
/// `randomized` vertex jitter.
pub fn polygon_configured(center: Point, radius: f64, sides: u32, rotation_deg: f64, rounded: f64, randomized: f64) -> PathData {
    if (!rounded.is_finite() || rounded.abs() < 1e-9) && (!randomized.is_finite() || randomized.abs() < 1e-9) {
        return polygon(center, radius, sides.clamp(3, 1000), rotation_deg);
    }
    let n = sides.clamp(3, 1000);
    let rot = if rotation_deg.is_finite() { rotation_deg.to_radians() } else { 0.0 };
    let rand_amt = if randomized.is_finite() { randomized.clamp(0.0, 10.0) } else { 0.0 };
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let (dr, da) = if rand_amt > 1e-9 { vertex_jitter(i, n) } else { (0.0, 0.0) };
            let r = (radius * (1.0 + rand_amt * 0.45 * dr)).max(0.01);
            let a = -PI / 2.0 + rot + TAU * (i as f64 / n as f64) + rand_amt * (PI / n as f64) * da;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    contour_with_rounding(&pts, rounded)
}

/// Star with `points` tips (≥ 2), outer radius `r1`, inner radius `r2`, first tip up.
pub fn star(center: Point, r1: f64, r2: f64, points: u32, rotation_deg: f64) -> PathData {
    let n = points.max(2) * 2;
    let rot = rotation_deg.to_radians();
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let r = if i % 2 == 0 { r1 } else { r2 };
            let a = -PI / 2.0 + rot + TAU * i as f64 / n as f64;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    PathData::single(SubPath::polyline(&pts, true))
}

/// Star with `points` tips (≥ 2), outer radius `r1`, inner radius `r2`, bezier tangent `rounded`
/// factor and deterministic `randomized` vertex jitter.
pub fn star_configured(center: Point, r1: f64, r2: f64, points: u32, rotation_deg: f64, rounded: f64, randomized: f64) -> PathData {
    if (!rounded.is_finite() || rounded.abs() < 1e-9) && (!randomized.is_finite() || randomized.abs() < 1e-9) {
        return star(center, r1, r2, points.clamp(2, 1000), rotation_deg);
    }
    let n = points.clamp(2, 1000) * 2;
    let rot = if rotation_deg.is_finite() { rotation_deg.to_radians() } else { 0.0 };
    let rand_amt = if randomized.is_finite() { randomized.clamp(0.0, 10.0) } else { 0.0 };
    let pts: Vec<Point> = (0..n)
        .map(|i| {
            let base_r = if i % 2 == 0 { r1 } else { r2 };
            let (dr, da) = if rand_amt > 1e-9 { vertex_jitter(i, n) } else { (0.0, 0.0) };
            let r = (base_r * (1.0 + rand_amt * 0.45 * dr)).max(0.01);
            let a = -PI / 2.0 + rot + TAU * (i as f64 / n as f64) + rand_amt * (PI / n as f64) * da;
            Point::new(center.x + r * a.cos(), center.y + r * a.sin())
        })
        .collect();
    contour_with_rounding(&pts, rounded)
}

/// Inner radius of a "straight-armed" star (Illustrator's Alt-drag behaviour): shoulders aligned.
pub fn straight_star_inner_radius(r1: f64, points: u32) -> f64 {
    let n = points.max(3) as f64;
    r1 * (PI / n * 2.0).cos() / (PI / n).cos()
}

/// Open straight line.
pub fn line(a: Point, b: Point) -> PathData {
    PathData::single(SubPath::polyline(&[a, b], false))
}

/// Bounding box of an arc segment when reference point `reference` (0…8) is placed on `anchor`.
pub fn arc_bbox(anchor: Point, reference: usize, length_x: f64, length_y: f64) -> Rect {
    let w = length_x.abs().max(1e-9);
    let h = length_y.abs().max(1e-9);
    let rp = crate::reference_point(Rect::new(0.0, 0.0, w, h), reference.min(8));
    Rect::new(anchor.x - rp.x, anchor.y - rp.y, anchor.x - rp.x + w, anchor.y - rp.y + h)
}

/// Arc segment from the Arc tool dialog: lengths, reference point, base axis, slope and open/closed.
pub fn arc_segment(anchor: Point, reference: usize, length_x: f64, length_y: f64, base_along_x: bool, slope: f64, closed: bool) -> PathData {
    let r = arc_bbox(anchor, reference, length_x, length_y).abs();
    let (a, b) = if base_along_x { (Point::new(r.x0, r.y0), Point::new(r.x1, r.y1)) } else { (Point::new(r.x0, r.y1), Point::new(r.x1, r.y0)) };
    arc(a, b, slope, closed, base_along_x)
}

/// Quarter-ellipse arc from `a` to `b` (Arc tool default: open). `slope` is −100…100
/// (concave…convex); 0 is a straight chord. A convex arc bulges toward the corner `(b.x, a.y)`,
/// and a closed arc closes through the corner `(a.x, b.y)`, so a closed arc at slope 100 is the
/// quarter-ellipse sector. The shape does not depend on `_base_along_x`: for a Y-base arc,
/// [`arc_segment`] passes ends on the box mirrored top to bottom.
pub fn arc(a: Point, b: Point, slope: f64, closed: bool, _base_along_x: bool) -> PathData {
    let t = slope.clamp(-100.0, 100.0) / 100.0;
    let corner = if t >= 0.0 { Point::new(b.x, a.y) } else { Point::new(a.x, b.y) };
    let k = KAPPA * t.abs();
    let h1 = if k == 0.0 { a } else { a + (corner - a) * k };
    let h2 = if k == 0.0 { b } else { b + (corner - b) * k };
    let mut anchors =
        vec![Anchor { p: a, h_in: a, h_out: h1, kind: AnchorKind::Corner }, Anchor { p: b, h_in: h2, h_out: b, kind: AnchorKind::Corner }];
    if closed {
        anchors.push(Anchor::corner(Point::new(a.x, b.y)));
    }
    PathData::single(SubPath::new(anchors, closed))
}

/// Logarithmic spiral with decay per full wind (`segments` quarter turns).
pub fn spiral(center: Point, radius: f64, decay_percent: f64, segments: u32, clockwise: bool) -> PathData {
    let decay = (decay_percent / 100.0).clamp(0.05, 0.9999);
    let step_decay = decay.powf(0.25);
    let dir = if clockwise { 1.0 } else { -1.0 };
    // The radius shrinks by `decay` each full wind: r(φ) = radius · e^(kφ).
    let k = decay.ln() / TAU;
    // Handle lengths per unit of the derivative below, at a segment's outer and inner end: √r
    // changes evenly with the curve's parameter along each quarter turn. Both are KAPPA when the
    // radius does not shrink, and the curve stays within 0.8% of the spiral's radius at 20% decay.
    let outer = 2.0 * KAPPA * (step_decay.sqrt() - 1.0) / step_decay.ln();
    let inner = outer / step_decay.sqrt();
    let mut anchors = Vec::new();
    let mut r = radius;
    for i in 0..=segments {
        let a = dir * (i as f64) * PI / 2.0;
        let p = Point::new(center.x + r * a.cos(), center.y + r * a.sin());
        // The spiral's derivative by the angle turned: the radius changes along the radial
        // direction while the point turns along the circle's tangent.
        let d = Vec2::new(a.cos(), a.sin()) * (r * k) + Vec2::new(-a.sin(), a.cos()) * (r * dir);
        let h_in = p - d * inner;
        let h_out = p + d * outer;
        anchors.push(Anchor { p, h_in: if i == 0 { p } else { h_in }, h_out: if i == segments { p } else { h_out }, kind: AnchorKind::Smooth });
        r *= step_decay;
    }
    // Illustrator spirals wind inward from the outer point; reverse so the path starts at the centre end.
    let mut sp = SubPath::new(anchors, false);
    sp.reverse();
    PathData::single(sp)
}

/// Configurable spiral with `turns` revolutions (`0.25..=50.0`), `divergence` radial growth power
/// (`0.05..=20.0`, `1.0` = uniform Archimedean spacing) and `inner_radius` fraction (`0.0..=0.99`).
pub fn spiral_configured(center: Point, radius: f64, turns: f64, divergence: f64, inner_radius: f64, clockwise: bool) -> PathData {
    let turns = if turns.is_finite() { turns.clamp(0.25, 50.0) } else { 3.0 };
    let div = if divergence.is_finite() { divergence.clamp(0.05, 20.0) } else { 1.0 };
    let inner_frac = if inner_radius.is_finite() { inner_radius.clamp(0.0, 0.99) } else { 0.0 };
    let r_outer = if radius.is_finite() { radius.abs().max(0.01) } else { 50.0 };
    let r_inner = r_outer * inner_frac;
    let steps = (turns * 4.0).round().clamp(1.0, 200.0) as usize;
    let dir = if clockwise { 1.0 } else { -1.0 };
    let total_angle = dir * turns * TAU;
    let dtheta = (turns * TAU) / (steps as f64);
    let k = if dtheta > 1e-6 { KAPPA / dtheta } else { KAPPA };
    let mut anchors = Vec::with_capacity(steps + 1);
    for i in 0..=steps {
        let u = (i as f64) / (steps as f64);
        let theta = u * total_angle;
        let f = if u <= 0.0 { 0.0 } else { u.powf(div) };
        let r = r_inner + (r_outer - r_inner) * f;
        let p = Point::new(center.x + r * theta.cos(), center.y + r * theta.sin());
        let dr_du = if u > 1e-9 {
            (r_outer - r_inner) * div * u.powf(div - 1.0)
        } else if (div - 1.0).abs() < 1e-9 {
            r_outer - r_inner
        } else {
            0.0
        };
        let dx_du = dr_du * theta.cos() - r * theta.sin() * total_angle;
        let dy_du = dr_du * theta.sin() + r * theta.cos() * total_angle;
        let d = Vec2::new(dx_du, dy_du) * (k / (steps as f64));
        let h_in = if i == 0 { p } else { p - d };
        let h_out = if i == steps { p } else { p + d };
        anchors.push(Anchor { p, h_in, h_out, kind: AnchorKind::Smooth });
    }
    PathData::single(SubPath::new(anchors, false))
}

/// Rotatable 3D box inscribed in `r`, rotated by `(angle_x, angle_y, angle_z)` degrees and using
/// orthographic parallel projection per axis when `parallel_*` is true or perspective foreshortening
/// when false. Emits the visible front-facing quad faces with clockwise winding.
pub fn box3d(r: Rect, angle_x: f64, angle_y: f64, angle_z: f64, parallel_x: bool, parallel_y: bool, parallel_z: bool) -> PathData {
    let r = r.abs();
    if r.width() <= 1e-9 || r.height() <= 1e-9 {
        return rectangle(r);
    }
    let rx = if angle_x.is_finite() { angle_x.to_radians() } else { 30.0_f64.to_radians() };
    let ry = if angle_y.is_finite() { angle_y.to_radians() } else { 35.0_f64.to_radians() };
    let rz = if angle_z.is_finite() { angle_z.to_radians() } else { 0.0 };
    let (sx, cx) = rx.sin_cos();
    let (sy, cy) = ry.sin_cos();
    let (sz, cz) = rz.sin_cos();

    let transform_pt = |mut x: f64, mut y: f64, mut z: f64| -> (f64, f64, f64) {
        if !parallel_x {
            let k = 1.0 - 0.22 * x;
            y *= k;
            z *= k;
        }
        if !parallel_y {
            let k = 1.0 - 0.22 * y;
            x *= k;
            z *= k;
        }
        // Rotate around X
        let (y1, z1) = (y * cx - z * sx, y * sx + z * cx);
        // Rotate around Y
        let (x2, z2) = (x * cy + z1 * sy, -x * sy + z1 * cy);
        // Rotate around Z
        let (x3, y3) = (x2 * cz - y1 * sz, x2 * sz + y1 * cz);
        let persp = if parallel_z { 1.0 } else { 1.0 / (1.0 - 0.25 * z2).max(0.2) };
        (x3 * persp, y3 * persp, z2)
    };

    let raw_verts: [(f64, f64, f64); 8] = [
        transform_pt(-1.0, -1.0, -1.0),
        transform_pt(1.0, -1.0, -1.0),
        transform_pt(1.0, 1.0, -1.0),
        transform_pt(-1.0, 1.0, -1.0),
        transform_pt(-1.0, -1.0, 1.0),
        transform_pt(1.0, -1.0, 1.0),
        transform_pt(1.0, 1.0, 1.0),
        transform_pt(-1.0, 1.0, 1.0),
    ];

    let mut min_x = f64::INFINITY;
    let mut max_x = f64::NEG_INFINITY;
    let mut min_y = f64::INFINITY;
    let mut max_y = f64::NEG_INFINITY;
    for &(vx, vy, _) in &raw_verts {
        min_x = min_x.min(vx);
        max_x = max_x.max(vx);
        min_y = min_y.min(vy);
        max_y = max_y.max(vy);
    }
    let span_x = (max_x - min_x).max(1e-6);
    let span_y = (max_y - min_y).max(1e-6);
    let proj = |i: usize| -> Point {
        let (vx, vy, _) = raw_verts[i];
        Point::new(r.x0 + (vx - min_x) / span_x * r.width(), r.y0 + (vy - min_y) / span_y * r.height())
    };

    // 6 cube faces with outward-pointing vertex order in 3D.
    const FACES: [[usize; 4]; 6] = [
        [4, 5, 6, 7], // +Z front
        [1, 0, 3, 2], // -Z back
        [0, 4, 7, 3], // -X left
        [5, 1, 2, 6], // +X right
        [0, 1, 5, 4], // -Y top
        [7, 6, 2, 3], // +Y bottom
    ];

    let mut faces_with_depth: Vec<(f64, SubPath)> = Vec::new();
    for idx in FACES {
        let z_avg = (raw_verts[idx[0]].2 + raw_verts[idx[1]].2 + raw_verts[idx[2]].2 + raw_verts[idx[3]].2) * 0.25;
        let mut pts = [proj(idx[0]), proj(idx[1]), proj(idx[2]), proj(idx[3])];
        let mut sp = SubPath::polyline(&pts, true);
        let area = sp.signed_area();
        // Outward face in camera space has positive signed area (clockwise on screen, y down).
        if area > 1e-4 {
            faces_with_depth.push((z_avg, sp));
        } else if faces_with_depth.is_empty() && area.abs() > 1e-4 {
            pts.reverse();
            sp = SubPath::polyline(&pts, true);
            faces_with_depth.push((z_avg, sp));
        }
    }
    faces_with_depth.sort_by(|a, b| a.0.total_cmp(&b.0));
    let subpaths: Vec<SubPath> = faces_with_depth.into_iter().map(|(_, sp)| sp).collect();
    if subpaths.is_empty() { rectangle(r) } else { PathData::new(subpaths) }
}

/// The largest grid divider skew, in percent either way.
pub const GRID_SKEW_MAX: f64 = 500.0;

/// Where a divider at fraction `t` (0..1) of a grid's extent lands under `skew` percent: evenly
/// spaced at 0, crowded toward 1 by a positive skew and toward 0 by a negative one (`t` raised to
/// 1 + |skew| / 100 from the far end).
fn skewed(t: f64, skew: f64) -> f64 {
    // NaN reads as no skew.
    let k = 1.0 + skew.abs().min(GRID_SKEW_MAX) / 100.0;
    if skew > 0.0 {
        1.0 - (1.0 - t).powf(k)
    } else if skew < 0.0 {
        t.powf(k)
    } else {
        t
    }
}

/// Rectangular grid, bottom object first: the outside as one rectangle (`frame`) or as its four
/// sides (top, bottom, left, right), then the horizontal and the vertical dividers as open lines.
/// A positive `h_skew` crowds the horizontal dividers toward the top and a positive `v_skew` the
/// vertical ones toward the right (percent, see [`GRID_SKEW_MAX`]).
pub fn rectangular_grid(r: Rect, (h_dividers, h_skew): (u32, f64), (v_dividers, v_skew): (u32, f64), frame: bool) -> Vec<PathData> {
    let r = r.abs();
    let (tl, tr, br, bl) = (Point::new(r.x0, r.y0), Point::new(r.x1, r.y0), Point::new(r.x1, r.y1), Point::new(r.x0, r.y1));
    let mut out = if frame { vec![rectangle(r)] } else { vec![line(tl, tr), line(bl, br), line(tl, bl), line(tr, br)] };
    for i in 1..=h_dividers {
        // From the top down: toward the top is toward 0.
        let y = r.y0 + r.height() * skewed(i as f64 / (h_dividers + 1) as f64, -h_skew);
        out.push(line(Point::new(r.x0, y), Point::new(r.x1, y)));
    }
    for i in 1..=v_dividers {
        let x = r.x0 + r.width() * skewed(i as f64 / (v_dividers + 1) as f64, v_skew);
        out.push(line(Point::new(x, r.y0), Point::new(x, r.y1)));
    }
    out
}

/// Polar grid, bottom object first: the concentric ellipses from the outside in, then the radial
/// dividers from the top one clockwise. A positive `c_skew` crowds the ellipses toward the outside
/// and a positive `r_skew` the radial dividers clockwise of the top one (percent, see
/// [`GRID_SKEW_MAX`]). With `compound`, the ellipses pair up from the outside in into rings (two
/// subpaths, the inner one reversed so that it is a hole under either fill rule); an odd one out
/// is the innermost ellipse alone.
pub fn polar_grid(r: Rect, (concentric, c_skew): (u32, f64), (radial, r_skew): (u32, f64), compound: bool) -> Vec<PathData> {
    let r = r.abs();
    let c = r.center();
    let mut out: Vec<PathData> = Vec::new();
    for i in (1..=concentric + 1).rev() {
        let f = skewed(i as f64 / (concentric + 1) as f64, c_skew);
        let mut e = ellipse(Rect::from_center_size(c, (r.width() * f, r.height() * f)));
        // Every second ellipse from the outside is the hole of the one before it.
        match out.last_mut().filter(|_| compound && (concentric + 1 - i) % 2 == 1) {
            Some(ring) => {
                e.reverse();
                ring.subpaths.append(&mut e.subpaths);
            }
            None => out.push(e),
        }
    }
    // No dividers at 0 (the loop never divides by it).
    for i in 0..radial {
        // The turn runs clockwise on screen: clockwise of the top is toward its start.
        let a = -PI / 2.0 + TAU * skewed(i as f64 / radial as f64, -r_skew);
        out.push(line(c, Point::new(c.x + r.width() / 2.0 * a.cos(), c.y + r.height() / 2.0 * a.sin())));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The corner (index into `radii`) of each anchor of [`rectangle_with_corners`].
    fn corner_sources(r: Rect, radii: [f64; 4]) -> Vec<usize> {
        crate::corners::cut_corners(&rectangle(r), &radii, &[]).1.concat()
    }

    /// A pie is the arc from its start to its end angle, counterclockwise on screen, closed through
    /// the centre; a whole turn (or none) is the ellipse.
    #[test]
    fn a_pie_is_its_arc_closed_through_the_centre() {
        let r = Rect::new(0.0, 0.0, 200.0, 100.0);
        assert_eq!(ellipse_pie(r, 0.0, 360.0), ellipse(r));
        assert_eq!(ellipse_pie(r, 90.0, 90.0), ellipse(r));
        // A quarter, from 3 o'clock up to 12: the centre, then the arc's two ends.
        let q = ellipse_pie(r, 0.0, 90.0);
        let a: Vec<Point> = q.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(a.len(), 3);
        assert!(a[0].distance(Point::new(100.0, 50.0)) < 1e-9 && a[1].distance(Point::new(200.0, 50.0)) < 1e-9);
        assert!(a[2].distance(Point::new(100.0, 0.0)) < 1e-9, "counterclockwise on screen: up");
        assert_eq!(q.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((100.0, 0.0, 200.0, 50.0)));
        // Three quarters, from 12 round to 3 o'clock: four pieces at most, the whole box covered.
        let big = ellipse_pie(r, 90.0, 0.0);
        assert!(big.subpaths[0].anchors.len() <= 6 && big.subpaths[0].closed);
        assert_eq!(big.bounds().map(|b| (b.x0.round(), b.y0.round(), b.x1.round(), b.y1.round())), Some((0.0, 0.0, 200.0, 100.0)));
        assert!(ellipse_pie(r, f64::NAN, 10.0).anchor_count() == 4, "not a number: the whole ellipse");
    }

    /// The circle through three points against the closed form: (0, 0), (8, 0) and (0, 6) lie on
    /// the circle with the hypotenuse (8, 0)–(0, 6) as its diameter, center (4, 3) and radius 5.
    #[test]
    fn the_circle_through_three_points_has_the_closed_form_center_and_radius() {
        let (o, a, b) = (Point::ORIGIN, Point::new(8.0, 0.0), Point::new(0.0, 6.0));
        for (p, q, r) in [(o, a, b), (a, b, o), (b, o, a), (b, a, o)] {
            let (c, radius) = circle_through(p, q, r).unwrap();
            assert!(c.distance(Point::new(4.0, 3.0)) < 1e-12 && (radius - 5.0).abs() < 1e-12, "{c:?} {radius}");
        }
        // Moved far from the origin, the answer moves with the points.
        let far = Vec2::new(3.0e6, -2.0e6);
        let (c, radius) = circle_through(o + far, a + far, b + far).unwrap();
        assert!(c.distance(Point::new(4.0, 3.0) + far) < 1e-6 && (radius - 5.0).abs() < 1e-6, "{c:?} {radius}");
    }

    /// Two points that are the same, or three on one line, give no circle.
    #[test]
    fn coincident_or_collinear_points_give_no_circle() {
        let (a, b) = (Point::new(1.0, 2.0), Point::new(5.0, 7.0));
        assert_eq!(circle_through(a, a, b), None);
        assert_eq!(circle_through(a, b, b), None);
        assert_eq!(circle_through(a, a, a), None);
        assert_eq!(circle_through(a, b, a.midpoint(b)), None);
        assert_eq!(circle_through(a, b, a + (b - a) * 3.0), None);
        assert_eq!(circle_through(a, b, Point::new(f64::NAN, 0.0)), None);
    }

    /// Two points are the ends of a diameter, three are on the edge; one, four, two that are the
    /// same and a point that is not finite give no circle.
    #[test]
    fn the_circle_through_two_or_three_points() {
        let (a, b, c) = (Point::new(8.0, 0.0), Point::new(0.0, 6.0), Point::ORIGIN);
        assert_eq!(circle_through_points(&[a, b]), Some((Point::new(4.0, 3.0), 5.0)));
        let (center, r) = circle_through_points(&[a, b, c]).unwrap();
        assert!(center.distance(Point::new(4.0, 3.0)) < 1e-12 && (r - 5.0).abs() < 1e-12, "{center:?} {r}");
        for points in [&[a][..], &[a, b, c, a], &[a, a], &[a, Point::new(f64::INFINITY, 0.0)], &[], &[a, b, a.midpoint(b)]] {
            assert_eq!(circle_through_points(points), None, "{points:?}");
        }
    }

    /// Nearly on one line: a point 10⁻⁴ off a chord 2000 long still gives the circle (its sagitta
    /// formula `r = (h² + (L/2)²) / 2h`), and its center sits on the chord's perpendicular
    /// bisector; a point 10⁻¹² off it does not.
    #[test]
    fn nearly_collinear_points_give_a_stable_circle() {
        let (a, b) = (Point::new(-1000.0, 0.0), Point::new(1000.0, 0.0));
        let h = 1e-4;
        let (c, radius) = circle_through(a, b, Point::new(0.0, h)).unwrap();
        let want = (h * h + 1000.0 * 1000.0) / (2.0 * h);
        assert!((radius - want).abs() / want < 1e-9, "{radius} vs {want}");
        assert!(c.x.abs() < 1e-6 && (c.y - (h - want)).abs() / want < 1e-9, "{c:?}");
        assert_eq!(circle_through(a, b, Point::new(0.0, 1e-12)), None);
    }

    #[test]
    fn rect_is_4_corners_clockwise() {
        let p = rectangle(Rect::new(0.0, 0.0, 100.0, 50.0));
        assert_eq!(p.anchor_count(), 4);
        assert!(p.subpaths[0].area() > 0.0);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 100.0, 50.0)));
    }

    #[test]
    fn rect_normalizes_negative() {
        let p = rectangle(Rect::new(10.0, 10.0, 0.0, 0.0));
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 10.0, 10.0)));
    }

    #[test]
    fn ellipse_bounds_and_area() {
        let p = ellipse(Rect::new(0.0, 0.0, 100.0, 60.0));
        let b = p.bounds().unwrap();
        assert!((b.width() - 100.0).abs() < 1e-6 && (b.height() - 60.0).abs() < 1e-6);
        let area = p.subpaths[0].area().abs();
        assert!((area - PI * 50.0 * 30.0).abs() / area < 0.001);
    }

    #[test]
    fn ellipse_with_four_anchors_is_the_ellipse_and_others_fit_the_box() {
        let r = Rect::new(10.0, 20.0, 110.0, 80.0);
        let (four, plain) = (ellipse_with_anchors(r, 4), ellipse(r));
        for (a, b) in four.subpaths[0].anchors.iter().zip(&plain.subpaths[0].anchors) {
            assert!(a.p.distance(b.p) < 1e-9 && a.h_in.distance(b.h_in) < 1e-9 && a.h_out.distance(b.h_out) < 1e-9, "{a:?} {b:?}");
        }
        for n in [2, 3, 5, 7, 9, 1000] {
            let p = ellipse_with_anchors(r, n);
            assert_eq!(p.anchor_count(), n as usize);
            let b = p.bounds().unwrap();
            assert!(
                (b.x0 - r.x0).abs() < 1e-9 && (b.y0 - r.y0).abs() < 1e-9 && (b.x1 - r.x1).abs() < 1e-9 && (b.y1 - r.y1).abs() < 1e-9,
                "{n}: {b:?}"
            );
            let area = p.subpaths[0].area().abs();
            assert!((area - PI * 50.0 * 30.0).abs() / area < if n == 2 { 0.1 } else { 0.01 }, "{n}: {area}");
        }
        // A box with no height leaves the path flat rather than dividing by zero.
        let flat = ellipse_with_anchors(Rect::new(0.0, 5.0, 40.0, 5.0), 6);
        assert!(flat.subpaths[0].anchors.iter().all(|a| a.p.y == 5.0 && a.p.x.is_finite()));
    }

    #[test]
    fn rounded_rect_clamps() {
        let p = rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 100.0);
        assert_eq!(p.anchor_count(), 8);
        assert_eq!(p.bounds(), Some(Rect::new(0.0, 0.0, 20.0, 10.0)));
        assert_eq!(rounded_rectangle(Rect::new(0.0, 0.0, 20.0, 10.0), 0.0).anchor_count(), 4);
    }

    #[test]
    fn uniform_corners_keep_the_rounded_rectangle_layout() {
        // The 8 anchors a rounded rectangle always had, from the top side's first one.
        let (r, rad) = (Rect::new(10.0, 20.0, 110.0, 70.0), 7.5);
        let (x0, y0, x1, y1, k) = (10.0, 20.0, 110.0, 70.0, rad * KAPPA);
        let a =
            |p: (f64, f64), hin: (f64, f64), hout: (f64, f64)| Anchor { p: p.into(), h_in: hin.into(), h_out: hout.into(), kind: AnchorKind::Corner };
        let want = vec![
            a((x0 + rad, y0), (x0 + rad - k, y0), (x0 + rad, y0)),
            a((x1 - rad, y0), (x1 - rad, y0), (x1 - rad + k, y0)),
            a((x1, y0 + rad), (x1, y0 + rad - k), (x1, y0 + rad)),
            a((x1, y1 - rad), (x1, y1 - rad), (x1, y1 - rad + k)),
            a((x1 - rad, y1), (x1 - rad + k, y1), (x1 - rad, y1)),
            a((x0 + rad, y1), (x0 + rad, y1), (x0 + rad - k, y1)),
            a((x0, y1 - rad), (x0, y1 - rad + k), (x0, y1 - rad)),
            a((x0, y0 + rad), (x0, y0 + rad), (x0, y0 + rad - k)),
        ];
        assert_eq!(rounded_rectangle(r, rad).subpaths[0].anchors, want);
        assert_eq!(corner_sources(r, [rad; 4]), [0, 1, 1, 2, 2, 3, 3, 0]);
        assert_eq!(rounded_rectangle(r, 0.0), rectangle(r));
    }

    #[test]
    fn each_corner_takes_its_own_radius_and_kind() {
        let r = Rect::new(0.0, 0.0, 100.0, 60.0);
        // Only the top-right corner rounded: 5 anchors, the corner's two on its sides.
        let radii = [0.0, 10.0, 0.0, 0.0];
        let p = rectangle_with_corners(r, radii, [CornerKind::Round; 4]);
        let pts: Vec<Point> = p.subpaths[0].anchors.iter().map(|a| a.p).collect();
        assert_eq!(pts, [Point::new(0.0, 0.0), Point::new(90.0, 0.0), Point::new(100.0, 10.0), Point::new(100.0, 60.0), Point::new(0.0, 60.0)]);
        assert_eq!(corner_sources(r, radii), [0, 1, 1, 2, 3]);
        let quarter = PI * 25.0;
        assert!((p.subpaths[0].area() - (6000.0 - 100.0 + quarter)).abs() < 0.1, "a quarter circle off one corner");
        // The top-left corner rounded: its top-side anchor first, its left-side one last.
        assert_eq!(corner_sources(r, [10.0, 0.0, 0.0, 200.0]), [0, 1, 2, 3, 3, 0]);
        // Inverted round cuts a quarter circle out; a chamfer cuts a triangle.
        let inverted = rectangle_with_corners(r, radii, [CornerKind::InvertedRound; 4]);
        assert!((inverted.subpaths[0].area() - (6000.0 - quarter)).abs() < 0.1);
        let chamfer = rectangle_with_corners(r, radii, [CornerKind::Chamfer; 4]);
        assert!((chamfer.subpaths[0].area() - (6000.0 - 50.0)).abs() < 1e-9);
        assert!(chamfer.subpaths[0].anchors.iter().all(|a| !a.has_in() && !a.has_out()), "straight");
        // Radii clamp to what their neighbours leave, the whole shorter side when they are
        // square (#2332): the cut reaches the bottom-left corner. Negative and NaN radii are none.
        let p = rectangle_with_corners(r, [500.0, -5.0, f64::NAN, 0.0], [CornerKind::Round; 4]);
        assert_eq!(p.subpaths[0].anchors[0].p, Point::new(60.0, 0.0));
        assert_eq!(p.subpaths[0].anchors.last().map(|a| a.p), Some(Point::new(0.0, 60.0)));
        assert_eq!(p.anchor_count(), 5);
        assert_eq!(CornerKind::ALL.map(CornerKind::next), [CornerKind::InvertedRound, CornerKind::Chamfer, CornerKind::Round]);
    }

    /// #442: on a rectangle that isn't square, every corner is a circular arc of the same radius,
    /// and radii past the limit stop at half the shorter side, whichever side a corner is on.
    #[test]
    fn corners_of_a_non_square_rectangle_are_alike() {
        let r = Rect::new(0.0, 0.0, 200.0, 80.0);
        assert_eq!(max_corner_radius(-200.0, 80.0), 40.0);
        for (radius, drawn) in [(30.0, 30.0), (60.0, 40.0)] {
            for kinds in [[CornerKind::Round; 4], [CornerKind::Chamfer, CornerKind::InvertedRound, CornerKind::Round, CornerKind::Chamfer]] {
                let a: Vec<Point> = rectangle_with_corners(r, [radius; 4], kinds).subpaths[0].anchors.iter().map(|a| a.p).collect();
                for (i, j) in [(7, 0), (1, 2), (3, 4), (5, 6)] {
                    let (x, y) = ((a[j].x - a[i].x).abs(), (a[j].y - a[i].y).abs());
                    assert!((x - drawn).abs() < 1e-9 && (y - drawn).abs() < 1e-9, "radius {radius}: a corner spans {x} × {y}");
                }
            }
        }
    }

    #[test]
    fn polygon_first_vertex_up() {
        let p = polygon(Point::new(0.0, 0.0), 10.0, 6, 0.0);
        assert_eq!(p.anchor_count(), 6);
        let a = p.subpaths[0].anchors[0].p;
        assert!(a.x.abs() < 1e-9 && (a.y + 10.0).abs() < 1e-9);
    }

    #[test]
    fn star_alternates_radii() {
        let p = star(Point::ZERO, 10.0, 5.0, 5, 0.0);
        assert_eq!(p.anchor_count(), 10);
        let d: Vec<f64> = p.subpaths[0].anchors.iter().map(|a| a.p.to_vec2().hypot()).collect();
        assert!((d[0] - 10.0).abs() < 1e-9 && (d[1] - 5.0).abs() < 1e-9);
    }

    #[test]
    fn straight_star_five_points() {
        let r2 = straight_star_inner_radius(100.0, 5);
        assert!((r2 - 38.196_601).abs() < 1e-3);
    }

    #[test]
    fn spiral_has_segments() {
        let p = spiral(Point::ZERO, 100.0, 80.0, 10, true);
        assert_eq!(p.anchor_count(), 11);
        assert!(!p.subpaths[0].closed);
    }

    #[test]
    fn spiral_decay_is_per_full_wind() {
        let p = spiral(Point::ZERO, 100.0, 80.0, 8, true);
        let radii: Vec<_> = p.subpaths[0].anchors.iter().map(|a| (a.p - Point::ZERO).hypot()).collect();
        assert!((radii[8] - 100.0).abs() < 1e-9);
        assert!((radii[4] - 80.0).abs() < 1e-9);
        assert!((radii[0] - 64.0).abs() < 1e-9);
    }

    /// #3437: every segment stays on the spiral through its anchors, whose radius shrinks by
    /// `decay` each full wind, for strong and weak decay and both directions.
    #[test]
    fn spiral_segments_follow_the_spiral() {
        let center = Point::new(40.0, -25.0);
        for decay in [20.0, 30.0, 50.0, 80.0, 95.0] {
            for clockwise in [true, false] {
                let p = spiral(center, 100.0, decay, 8, clockwise);
                let anchors = &p.subpaths[0].anchors.to_vec();
                let k = (decay / 100.0_f64).ln() / TAU;
                let mut worst = 0.0_f64;
                for pair in anchors.windows(2) {
                    let c = kurbo::CubicBez::new(pair[0].p, pair[0].h_out, pair[1].h_in, pair[1].p);
                    let r0 = (pair[0].p - center).hypot();
                    let a0 = (pair[0].p - center).atan2();
                    for step in 1..20 {
                        let q = kurbo::ParamCurve::eval(&c, f64::from(step) / 20.0) - center;
                        // The angle turned from the segment's start, toward the outer end.
                        let turned = (q.atan2() - a0 + 3.0 * PI).rem_euclid(TAU) - PI;
                        let want = r0 * (-k * turned.abs()).exp();
                        worst = worst.max((q.hypot() - want).abs() / want);
                    }
                }
                assert!(worst < 0.008, "decay {decay} clockwise {clockwise}: radius off by {:.2}%", worst * 100.0);
            }
        }
    }

    #[test]
    fn grids() {
        let r = Rect::new(0.0, 0.0, 10.0, 10.0);
        assert_eq!(rectangular_grid(r, (5, 0.0), (5, 0.0), true).len(), 11);
        assert_eq!(polar_grid(r, (5, 0.0), (5, 0.0), false).len(), 11);
        // #1008: no radial dividers is none, only the outer ellipse without concentric ones.
        for (radial, len) in [(0, 1), (1, 2), (2, 3)] {
            assert_eq!(polar_grid(r, (0, 0.0), (radial, 0.0), false).len(), len, "{radial}");
        }
    }

    fn close(got: &[f64], want: &[f64]) -> bool {
        got.len() == want.len() && got.iter().zip(want).all(|(a, b)| (a - b).abs() < 1e-9)
    }

    /// The first anchor of each of `paths`.
    fn starts(paths: &[PathData]) -> Vec<Point> {
        paths.iter().map(|p| p.subpaths[0].anchors[0].p).collect()
    }

    /// #1377: the frame is one rectangle or its four sides, at the bottom; a skew moves the
    /// dividers toward the top / right (positive) or the bottom / left, in order and inside the
    /// grid, and none leaves them evenly spaced.
    #[test]
    fn a_rectangular_grid_frames_and_skews_its_dividers() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let framed = rectangular_grid(r, (3, 0.0), (1, 0.0), true);
        assert!(framed[0].is_closed() && framed[1..].iter().all(|p| !p.is_closed()));
        assert_eq!(starts(&framed[1..]), [Point::new(0.0, 25.0), Point::new(0.0, 50.0), Point::new(0.0, 75.0), Point::new(50.0, 0.0)]);
        let sides = rectangular_grid(r, (3, 0.0), (1, 0.0), false);
        assert_eq!(sides.len(), 8);
        assert!(sides.iter().all(|p| !p.is_closed() && p.anchor_count() == 2));
        assert_eq!(sides[4..], framed[1..]);
        for (skew, toward_start) in [(100.0, true), (-100.0, false), (GRID_SKEW_MAX, true), (-1e9, false), (f64::INFINITY, true)] {
            let g = rectangular_grid(r, (3, skew), (3, -skew), true);
            let ys: Vec<f64> = starts(&g[1..4]).iter().map(|p| p.y).collect();
            let xs: Vec<f64> = starts(&g[4..]).iter().map(|p| p.x).collect();
            for v in [&ys, &xs] {
                assert!(v.windows(2).all(|w| w[0] < w[1]) && v[0] > 0.0 && v[2] < 100.0, "{skew}: {v:?}");
                // Toward the top (y) and the left (x) is toward 0: the middle divider passes the middle.
                assert_eq!(v[1] < 50.0, toward_start, "{skew}: {v:?}");
            }
            assert_eq!(ys, xs, "one curve for both axes");
        }
        // 100 % squares the fraction from the side the dividers crowd to: 1/4, 1/2 and 3/4 from the top.
        let g = rectangular_grid(r, (3, 100.0), (0, 0.0), true);
        assert!(close(&starts(&g[1..]).iter().map(|p| p.y).collect::<Vec<_>>(), &[6.25, 25.0, 56.25]), "{g:?}");
        assert_eq!(rectangular_grid(r, (3, f64::NAN), (1, f64::NAN), true), framed);
    }

    /// #1377: the ellipses come from the outside in and skew toward the outside (positive) or the
    /// centre; the radial dividers keep the top one and skew clockwise (positive) of it or
    /// counterclockwise; as compound paths the ellipses pair up into rings.
    #[test]
    fn a_polar_grid_skews_its_dividers_and_pairs_ellipses_into_rings() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let widths = |g: &[PathData], n: usize| g[..n].iter().map(|p| p.bounds().unwrap().width()).collect::<Vec<_>>();
        for (skew, want) in [(0.0, [100.0, 75.0, 50.0, 25.0]), (100.0, [100.0, 93.75, 75.0, 43.75]), (-100.0, [100.0, 56.25, 25.0, 6.25])] {
            let got = widths(&polar_grid(r, (3, skew), (0, 0.0), false), 4);
            assert!(close(&got, &want), "{skew}: {got:?}");
        }
        // Radial: four dividers point up, right, down and left; skewed, the first stays up and the
        // other three all point to the right (clockwise of it) or to the left.
        let ends = |skew: f64| polar_grid(r, (0, 0.0), (4, skew), false)[1..].iter().map(|p| p.subpaths[0].anchors[1].p).collect::<Vec<_>>();
        let even = ends(0.0);
        for (p, want) in even.iter().zip([(50.0, 0.0), (100.0, 50.0), (50.0, 100.0), (0.0, 50.0)]) {
            assert!(p.distance(Point::new(want.0, want.1)) < 1e-9, "{even:?}");
        }
        for (skew, right) in [(GRID_SKEW_MAX, true), (-GRID_SKEW_MAX, false)] {
            let e = ends(skew);
            assert!(e[0].distance(even[0]) < 1e-9);
            assert!(e[1..].iter().all(|p| (p.x > 50.0) == right && (p.x - 50.0).abs() > 1e-6), "{skew}: {e:?}");
        }
        // Compound: 4 ellipses are 2 rings, 5 are 2 rings and the innermost ellipse; each ring's
        // hole runs against its outside.
        for (concentric, shapes) in [(3, vec![2, 2]), (4, vec![2, 2, 1]), (0, vec![1])] {
            let g = polar_grid(r, (concentric, 0.0), (2, 0.0), true);
            assert_eq!(g.len(), shapes.len() + 2);
            assert_eq!(g[..shapes.len()].iter().map(|p| p.subpaths.len()).collect::<Vec<_>>(), shapes);
            for ring in g.iter().filter(|p| p.subpaths.len() == 2) {
                assert!(ring.is_closed());
                assert!(ring.subpaths[0].signed_area() * ring.subpaths[1].signed_area() < 0.0);
                assert!(ring.subpaths[0].signed_area().abs() > ring.subpaths[1].signed_area().abs());
            }
        }
    }

    /// An arc with Base Along Y, open or closed, convex or concave, is the arc with Base Along X in
    /// the same box mirrored top to bottom, anchor for anchor and handle for handle.
    #[test]
    fn a_y_base_arc_is_the_x_base_arc_mirrored_top_to_bottom() {
        let mirror = |p: Point| Point::new(p.x, 20.0 + 70.0 - p.y);
        for closed in [false, true] {
            for slope in [100.0, 50.0, -50.0, -100.0] {
                let x = arc_segment(Point::new(10.0, 20.0), 0, 100.0, 50.0, true, slope, closed);
                let y = arc_segment(Point::new(10.0, 20.0), 0, 100.0, 50.0, false, slope, closed);
                let (xs, ys) = (&x.subpaths[0], &y.subpaths[0]);
                assert_eq!(ys.closed, closed);
                assert_eq!(xs.anchors.len(), ys.anchors.len(), "slope {slope}, closed {closed}");
                for (i, (p, q)) in xs.anchors.iter().zip(&ys.anchors).enumerate() {
                    for (u, v) in [(p.p, q.p), (p.h_in, q.h_in), (p.h_out, q.h_out)] {
                        assert!(mirror(u).distance(v) < 1e-9, "slope {slope}, closed {closed}, anchor {i}: {u:?} mirrored is not {v:?}");
                    }
                }
            }
        }
    }

    /// A closed convex arc is the quarter-ellipse sector of its box with either base: its area is
    /// a quarter of the ellipse's, π/4 of the box's.
    #[test]
    fn a_closed_convex_arc_is_a_quarter_ellipse_sector_with_either_base() {
        for base_along_x in [true, false] {
            let a = arc_segment(Point::ZERO, 0, 100.0, 50.0, base_along_x, 100.0, true).subpaths[0].signed_area().abs();
            assert!((a / (100.0 * 50.0) - PI / 4.0).abs() < 0.01, "base along x {base_along_x}: area {a}");
        }
    }

    #[test]
    fn arc_open_and_closed() {
        assert_eq!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, false, true).anchor_count(), 2);
        assert!(arc(Point::ZERO, Point::new(10.0, 10.0), 0.0, true, true).subpaths[0].closed);
        let seg = arc_segment(Point::ZERO, 0, 100.0, 50.0, true, 50.0, false);
        assert_eq!(seg.anchor_count(), 2);
    }

    #[test]
    fn ellipse_arc_modes_and_configured_shapes() {
        let r = Rect::new(0.0, 0.0, 100.0, 100.0);
        let slice = ellipse_arc(r, 0.0, 90.0, EllipseMode::Slice);
        assert!(slice.is_closed() && slice.anchor_count() == 3);
        assert_eq!(slice, ellipse_pie(r, 0.0, 90.0), "ellipse_pie matches Slice mode");
        let chord = ellipse_arc(r, 0.0, 90.0, EllipseMode::Chord);
        assert!(chord.is_closed() && chord.anchor_count() == 2);
        let open_arc = ellipse_arc(r, 0.0, 90.0, EllipseMode::Arc);
        assert!(!open_arc.is_closed() && open_arc.anchor_count() == 2);
        let whole = ellipse_arc(r, 0.0, 90.0, EllipseMode::Whole);
        assert_eq!(whole, ellipse(r));

        // Rectangle opened/cut with apply_shape_arc
        let rect_open = apply_shape_arc(rectangle(r), r.center(), 30.0, 300.0, EllipseMode::Arc);
        assert!(!rect_open.is_closed());
        let rect_slice = apply_shape_arc(rectangle(r), r.center(), 30.0, 300.0, EllipseMode::Slice);
        assert!(rect_slice.is_closed() && rect_slice.subpaths[0].anchors[0].p == r.center());

        // Polygon and Star with rounded and randomized, and unconfigured parity
        assert_eq!(polygon_configured(Point::ZERO, 50.0, 6, 0.0, 0.0, 0.0), polygon(Point::ZERO, 50.0, 6, 0.0));
        assert_eq!(star_configured(Point::ZERO, 50.0, 25.0, 5, 0.0, 0.0, 0.0), star(Point::ZERO, 50.0, 25.0, 5, 0.0));
        let poly_round = polygon_configured(Point::ZERO, 50.0, 6, 0.0, 0.5, 0.1);
        assert_eq!(poly_round.anchor_count(), 6);
        assert!(poly_round.subpaths[0].anchors.iter().all(|a| a.has_in() && a.has_out()));
        let star_round = star_configured(Point::ZERO, 50.0, 25.0, 5, 0.0, 0.5, 0.1);
        assert_eq!(star_round.anchor_count(), 10);

        // Spiral configured
        let sp = spiral_configured(Point::ZERO, 100.0, 3.0, 1.5, 0.25, true);
        assert!(!sp.is_closed() && sp.anchor_count() == 13);

        // 3D Box
        let b3d = box3d(r, 30.0, 35.0, 0.0, true, true, true);
        assert!(!b3d.subpaths.is_empty() && b3d.subpaths.iter().all(|s| s.closed && s.anchors.len() == 4));
    }
}
