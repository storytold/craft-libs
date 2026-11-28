//! The coordinates argument (30) of each object type read as geometry: curves (R16), rectangles
//! (R15), ellipses and arcs (R17) and polygons (R18).

use std::f64::consts::PI;

use crate::Warning;
use crate::bytes::Rd;
use crate::layout::Profile;
use crate::limits::Budget;
use crate::model::{Geometry, Path, Pt, Segment, SubPath};
use crate::records::{angle, coord};
use crate::riff::push;

/// The most sides a polygon record may give (VectorCraft's own bound).
const MAX_POLYGON_SIDES: u32 = 1000;
/// The base curve of a polygon lies on a circle of 1000 mm (R18), in points.
const POLYGON_UNIT_RADIUS: f64 = 1000.0 / 25.4 * 72.0;

/// What a point type byte asks for (R16): bits 7 and 6.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Op {
    Move,
    Line,
    Curve,
    Control,
}

fn op(t: u8) -> Op {
    match t >> 6 {
        0 => Op::Move,
        1 => Op::Line,
        2 => Op::Curve,
        _ => Op::Control,
    }
}

/// Bit 3 closes the subpath (R16).
const CLOSES: u8 = 0x08;

/// A point list (R16): the count, the points, then one type byte each. The count is a u32 from
/// version 6 and a u16 followed by another u16 before it (R39). It is held to what the bytes hold;
/// `None` when the points are past the budget.
pub(crate) fn point_list(b: &[u8], p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Path> {
    let mut r = Rd::new(b);
    let count = if p.short {
        let count = usize::from(r.u16()?);
        r.skip(2)?;
        count
    } else {
        r.u32()? as usize
    };
    points(b, r, count, p, budget, warn)
}

/// A path record (object type 0x25, R30): 4 bytes, two u16 counts whose sum is the point count, 16
/// bytes, then the points and their type bytes as in a point list.
pub(crate) fn path_record(b: &[u8], p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Path> {
    let mut r = Rd::new(b);
    r.skip(4)?;
    let count = usize::from(r.u16()?) + usize::from(r.u16()?);
    r.skip(16)?;
    points(b, r, count, p, budget, warn)
}

/// `count` points from `r` over `b`, then their type bytes. A list cut short is read two ways: as
/// all `count` points with the type bytes after them, and as the points and type bytes the bytes
/// hold split evenly. The reading that keeps more points with a type byte wins, the first on a tie.
fn points(b: &[u8], mut r: Rd, count: usize, p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Path> {
    let each = 2 * p.word();
    let left = r.left();
    let split = (left / (each + 1)).min(count);
    let whole = count.checked_mul(each).filter(|len| *len <= left).map(|len| (left - len).min(count));
    let stored = match whole {
        Some(typed) if typed >= split => count,
        _ => split,
    };
    let types = b.get(r.pos() + stored * each..)?;
    let n = stored.min(types.len());
    if count > n {
        push(warn, Warning::Damaged);
    }
    if !budget.take_coordinates(n) {
        push(warn, Warning::TooDeep);
        return None;
    }
    let mut points = Vec::with_capacity(n);
    for _ in 0..n {
        points.push(Pt { x: coord(&mut r, p)?, y: coord(&mut r, p)? });
    }
    Some(path_of(&points, types))
}

/// The subpaths that points and their type bytes draw.
pub(crate) fn path_of(points: &[Pt], types: &[u8]) -> Path {
    let mut subpaths = Vec::new();
    let mut cur: Option<SubPath> = None;
    // The last two control points: a curve end uses only those.
    let mut controls: Vec<Pt> = Vec::with_capacity(2);
    for (&pt, &t) in points.iter().zip(types) {
        let o = op(t);
        if o == Op::Control {
            if controls.len() == 2 {
                controls.remove(0);
            }
            controls.push(pt);
            continue;
        }
        if o == Op::Move || cur.is_none() {
            subpaths.extend(cur.take());
            cur = Some(SubPath { start: pt, segments: Vec::new(), closed: t & CLOSES != 0 && o == Op::Move });
            controls.clear();
            if o == Op::Move {
                continue;
            }
        }
        let Some(sp) = cur.as_mut() else { continue };
        let seg = match (o, controls.as_slice()) {
            (Op::Curve, [c1, c2]) => Segment::Cubic(*c1, *c2, pt),
            _ => Segment::Line(pt),
        };
        controls.clear();
        sp.segments.push(seg);
        if t & CLOSES != 0 {
            sp.closed = true;
        }
    }
    subpaths.extend(cur);
    // A closed subpath with no segments, or an open one with none, draws nothing.
    subpaths.retain(|s| !s.segments.is_empty());
    Path { subpaths }
}

/// A rectangle record (R15): width, height, then one corner radius for every corner, four from
/// version 9.
pub(crate) fn rect(b: &[u8], p: &Profile) -> Option<Geometry> {
    if p.f64_rect {
        return rect_f64(b, p);
    }
    let mut r = Rd::new(b);
    let (w, h) = (coord(&mut r, p)?, coord(&mut r, p)?);
    let first = coord(&mut r, p)?.abs();
    let mut radii = [first; 4];
    if p.four_radii {
        for slot in radii.iter_mut().skip(1) {
            *slot = coord(&mut r, p)?.abs();
        }
    }
    Some(Geometry::Rect { w, h, radii })
}

/// A rectangle record from version 15 (R29): width and height as f64 coordinate units, two scale
/// factors that repeat the transform's (not applied), the scale-with flag and 7 bytes, then the
/// four radii as f64 16 bytes apart. With the flag clear each radius is a share of half the shorter
/// side; with it set the radius is in inches. A corner is held to half the shorter side, and
/// sizes to the range of the 32-bit coordinates of earlier versions.
fn rect_f64(b: &[u8], p: &Profile) -> Option<Geometry> {
    let mut r = Rd::new(b);
    let size = |v: f64| (v.abs() <= f64::from(i32::MAX)).then(|| v * p.unit());
    let (w, h) = (size(r.f64()?)?, size(r.f64()?)?);
    r.skip(8 + 8)?;
    let inches = r.u8()? != 0;
    r.skip(7)?;
    let short = w.abs().min(h.abs());
    let mut radii = [0.0; 4];
    for (k, slot) in radii.iter_mut().enumerate() {
        if k > 0 {
            r.skip(16)?;
        }
        let v = r.f64()?.abs();
        let radius = if inches { v * 72.0 } else { v * short / 2.0 };
        *slot = radius.min(short / 2.0);
    }
    Some(Geometry::Rect { w, h, radii })
}

/// An ellipse record (R17): the box's width and height, the start and end angles and whether
/// the arc closes through the centre. Equal angles make a whole ellipse.
pub(crate) fn ellipse(b: &[u8], p: &Profile) -> Option<Geometry> {
    let mut r = Rd::new(b);
    let (w, h) = (coord(&mut r, p)?, coord(&mut r, p)?);
    let (a1, a2) = (angle(&mut r, p)?, angle(&mut r, p)?);
    let pie = if p.short { r.u16()? != 0 } else { r.u32()? != 0 };
    let sweep = (a2 - a1).rem_euclid(360.0);
    if sweep.abs() < 1e-9 {
        return Some(Geometry::Ellipse { w, h });
    }
    Some(Geometry::Path(arc(w, h, a1, sweep, pie)))
}

/// The arc of the ellipse in the box from the origin to (`w`, `h`), from `start` degrees through
/// `sweep` degrees, in the ellipse's own axes; a pie runs from the centre and closes there. The
/// radii are half the size, and a negative width or height points its axis the other way (R17), so
/// with a negative height 0 to 180 degrees is the half below the centre.
fn arc(w: f64, h: f64, start: f64, sweep: f64, pie: bool) -> Path {
    let (cx, cy, rx, ry) = (w / 2.0, h / 2.0, w / 2.0, h / 2.0);
    let at = |t: f64| Pt { x: cx + rx * t.cos(), y: cy + ry * t.sin() };
    let tangent = |t: f64| Pt { x: -rx * t.sin(), y: ry * t.cos() };
    let pieces = (sweep / 90.0).ceil().max(1.0) as usize;
    let step = sweep.to_radians() / pieces as f64;
    let k = 4.0 / 3.0 * (step / 4.0).tan();
    let t0 = start.to_radians();
    let mut segments = Vec::with_capacity(pieces + 2);
    if pie {
        segments.push(Segment::Line(at(t0)));
    }
    for i in 0..pieces {
        let (a, b) = (t0 + step * i as f64, t0 + step * (i + 1) as f64);
        let (p0, p1, d0, d1) = (at(a), at(b), tangent(a), tangent(b));
        segments.push(Segment::Cubic(Pt { x: p0.x + k * d0.x, y: p0.y + k * d0.y }, Pt { x: p1.x - k * d1.x, y: p1.y - k * d1.y }, p1));
    }
    let start = if pie { Pt { x: cx, y: cy } } else { at(t0) };
    Path { subpaths: vec![SubPath { start, segments, closed: pie }] }
}

/// A polygon (R18): its base curve, one side of the outline on a circle of 1000 mm with its top
/// vertex first, copied once per side and turned clockwise each time by the vertices the side
/// spans, then scaled by (`rx`, `ry`) and moved to the centre the polygon argument (11000) gives.
/// A star whose sides skip vertices draws as one outline, or as several when the side count and
/// the step share a factor.
pub(crate) fn polygon(base: &Path, b: &[u8], p: &Profile, budget: &mut Budget, warn: &mut Vec<Warning>) -> Option<Geometry> {
    let mut r = Rd::new(b);
    if !p.tagged {
        r.skip(4)?;
    }
    let sides = r.u32()?;
    // The next point: the first field, or the second when the first is 0 or 1.
    let (first, second) = (r.u32()?, r.u32()?);
    let next = if first > 1 { first } else { second };
    if p.tagged {
        r.skip(4)?;
    }
    let (rx, ry) = (r.f64()?, r.f64()?);
    let (cx, cy) = (coord(&mut r, p)?, coord(&mut r, p)?);
    let side = base.subpaths.first()?;
    if !(1..=MAX_POLYGON_SIDES).contains(&sides) {
        push(warn, Warning::Damaged);
        return None;
    }
    if !budget.take_coordinates(side.segments.len().saturating_mul(3).saturating_mul(sides as usize)) {
        push(warn, Warning::TooDeep);
        return None;
    }
    // The base curve's own unit radius, measured from its first point, keeps the scale right
    // whatever unit the file's coordinates are in.
    let unit = side.start.x.hypot(side.start.y);
    let unit = if unit > 0.0 { unit } else { POLYGON_UNIT_RADIUS };
    let place = |q: Pt, turn: f64| {
        let (s, c) = turn.sin_cos();
        let (x, y) = (q.x * c - q.y * s, q.x * s + q.y * c);
        Pt { x: x / unit * POLYGON_UNIT_RADIUS * rx + cx, y: y / unit * POLYGON_UNIT_RADIUS * ry + cy }
    };
    let step = side_step(side, sides, next);
    let outlines = gcd(sides, step);
    let each = sides / outlines;
    let angle = |k: u32| -2.0 * PI * f64::from(k % sides) / f64::from(sides);
    let mut subpaths = Vec::with_capacity(outlines as usize);
    for o in 0..outlines {
        let mut segments = Vec::with_capacity(side.segments.len() * each as usize);
        for j in 0..each {
            let turn = angle(o + j * step);
            segments.extend(side.segments.iter().map(|s| match *s {
                Segment::Line(a) => Segment::Line(place(a, turn)),
                Segment::Cubic(a, b, c) => Segment::Cubic(place(a, turn), place(b, turn), place(c, turn)),
            }));
        }
        subpaths.push(SubPath { start: place(side.start, angle(o)), segments, closed: true });
    }
    Some(Geometry::Path(Path { subpaths }))
}

/// How many vertices the base side moves on from its first point (R18): the vertex its last
/// point lies at, clockwise; else the next-point field when it names one; else 1.
fn side_step(side: &SubPath, sides: u32, next: u32) -> u32 {
    let field = if (1..sides).contains(&next) { next } else { 1 };
    let Some(end) = side.segments.last().map(|s| match *s {
        Segment::Line(e) | Segment::Cubic(_, _, e) => e,
    }) else {
        return field;
    };
    let (r0, r1) = (side.start.x.hypot(side.start.y), end.x.hypot(end.y));
    if !(r0 > 0.0 && (r1 - r0).abs() <= r0 * 1e-3) {
        return field;
    }
    let turned = (side.start.y.atan2(side.start.x) - end.y.atan2(end.x)) / (2.0 * PI / f64::from(sides));
    let k = turned.round();
    if !k.is_finite() || (turned - k).abs() > 1e-3 {
        return field;
    }
    match k.rem_euclid(f64::from(sides)) as u32 {
        0 => field,
        k => k,
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    let (mut a, mut b) = (a, b);
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a.max(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pt(x: f64, y: f64) -> Pt {
        Pt { x, y }
    }

    #[test]
    fn type_bytes_make_moves_lines_curves_and_closes() {
        let points = [pt(0.0, 0.0), pt(10.0, 0.0), pt(10.0, 5.0), pt(5.0, 10.0), pt(0.0, 10.0), pt(20.0, 20.0), pt(30.0, 20.0)];
        // move (closed), line, control, control, curve end closing; move, line.
        let types = [0x08, 0x40, 0xC0, 0xC0, 0x88, 0x00, 0x40];
        let path = path_of(&points, &types);
        assert_eq!(path.subpaths.len(), 2);
        let first = &path.subpaths[0];
        assert!(first.closed);
        assert_eq!(first.segments, vec![Segment::Line(pt(10.0, 0.0)), Segment::Cubic(pt(10.0, 5.0), pt(5.0, 10.0), pt(0.0, 10.0))]);
        assert!(!path.subpaths[1].closed);
    }

    #[test]
    fn a_point_list_short_of_type_bytes_reads_them_after_all_its_points() {
        let p = crate::layout::for_version(1300).unwrap_or_else(|| panic!("profile"));
        // Three points, (0, 0), (1 inch, 0) and (0, 0), and type bytes for the first two only.
        let mut b = 3u32.to_le_bytes().to_vec();
        for v in [0i32, 0, 254_000, 0, 0, 0] {
            b.extend(v.to_le_bytes());
        }
        b.extend([0x00, 0x40]);
        let mut warn = vec![];
        let path = point_list(&b, &p, &mut Budget::new(crate::Limits::default()), &mut warn).unwrap_or_else(|| panic!("path"));
        assert_eq!(path.subpaths.len(), 1);
        assert_eq!(path.subpaths[0].segments, vec![Segment::Line(pt(72.0, 0.0))]);
        assert_eq!(warn, vec![Warning::Damaged]);
        // A count past the points the bytes hold keeps the points that have a type byte.
        // Eight points and their eight type bytes take the bytes of nine points.
        let mut b = 9u32.to_le_bytes().to_vec();
        for i in 0..8 {
            b.extend((i * 254_000i32).to_le_bytes());
            b.extend(0i32.to_le_bytes());
        }
        b.extend([0x00, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40, 0x40]);
        let mut warn = vec![];
        let path = point_list(&b, &p, &mut Budget::new(crate::Limits::default()), &mut warn).unwrap_or_else(|| panic!("path"));
        assert_eq!(path.subpaths[0].segments.len(), 7);
        assert!(matches!(path.subpaths[0].segments[6], Segment::Line(end) if (end.x - 504.0).abs() < 1e-9 && end.y == 0.0));
        assert_eq!(warn, vec![Warning::Damaged]);
    }

    #[test]
    fn a_run_of_control_points_keeps_the_last_two() {
        let points: Vec<Pt> = (0..7).map(|i| pt(f64::from(i), 0.0)).collect();
        let path = path_of(&points, &[0x00, 0xC0, 0xC0, 0xC0, 0xC0, 0xC0, 0x80]);
        assert_eq!(path.subpaths[0].segments, vec![Segment::Cubic(pt(4.0, 0.0), pt(5.0, 0.0), pt(6.0, 0.0))]);
    }

    #[test]
    fn a_curve_end_without_two_controls_is_a_line() {
        let path = path_of(&[pt(0.0, 0.0), pt(1.0, 1.0)], &[0x00, 0x80]);
        assert_eq!(path.subpaths[0].segments, vec![Segment::Line(pt(1.0, 1.0))]);
    }

    #[test]
    fn a_quarter_arc_ends_on_the_ellipse() {
        let p = arc(20.0, 10.0, 0.0, 90.0, false);
        let sp = &p.subpaths[0];
        assert_eq!(sp.start, pt(20.0, 5.0));
        let Some(Segment::Cubic(_, _, end)) = sp.segments.last() else { panic!("no curve") };
        assert!((end.x - 10.0).abs() < 1e-9 && (end.y - 10.0).abs() < 1e-9);
        let pie = arc(20.0, 10.0, 0.0, 180.0, true);
        assert!(pie.subpaths[0].closed && pie.subpaths[0].start == pt(10.0, 5.0));
    }

    #[test]
    fn a_negative_height_turns_the_arc_below_the_centre() {
        let p = arc(20.0, -10.0, 0.0, 180.0, false);
        let sp = &p.subpaths[0];
        assert_eq!(sp.start, pt(20.0, -5.0));
        let Some(Segment::Cubic(_, _, mid)) = sp.segments.first() else { panic!("no curve") };
        assert!((mid.x - 10.0).abs() < 1e-9 && (mid.y + 10.0).abs() < 1e-9, "{mid:?}");
    }

    /// A version 13 polygon argument: `sides`, the two next-point fields, then rx = ry = 1 at the
    /// origin.
    fn polygon_arg(sides: u32, first: u32, second: u32) -> Vec<u8> {
        let mut b = Vec::new();
        for v in [sides, first, second, 0] {
            b.extend(v.to_le_bytes());
        }
        b.extend(1.0f64.to_le_bytes());
        b.extend(1.0f64.to_le_bytes());
        b.extend([0; 8]);
        b
    }

    /// Vertex `k` of `sides`, counted clockwise from the top, on the base circle.
    fn vertex(k: u32, sides: u32) -> Pt {
        let a = PI / 2.0 - 2.0 * PI * f64::from(k) / f64::from(sides);
        pt(POLYGON_UNIT_RADIUS * a.cos(), POLYGON_UNIT_RADIUS * a.sin())
    }

    fn ends(sp: &SubPath) -> Vec<Pt> {
        sp.segments
            .iter()
            .map(|s| match *s {
                Segment::Line(e) | Segment::Cubic(_, _, e) => e,
            })
            .collect()
    }

    fn close(a: &[Pt], b: &[Pt]) -> bool {
        a.len() == b.len() && a.iter().zip(b).all(|(p, q)| (p.x - q.x).abs() < 1e-6 && (p.y - q.y).abs() < 1e-6)
    }

    fn star(sides: u32, step: u32, first: u32, second: u32) -> Path {
        let p = crate::layout::for_version(1300).unwrap();
        let base = Path { subpaths: vec![SubPath { start: vertex(0, sides), segments: vec![Segment::Line(vertex(step, sides))], closed: false }] };
        let mut budget = Budget::new(crate::Limits::default());
        let Some(Geometry::Path(path)) = polygon(&base, &polygon_arg(sides, first, second), &p, &mut budget, &mut vec![]) else {
            panic!("no polygon")
        };
        path
    }

    #[test]
    fn a_star_whose_sides_skip_a_vertex_chains_its_points() {
        // {5/2}: each side runs two vertices on, so the outline visits 2, 4, 1, 3 and 0.
        let path = star(5, 2, 2, 0);
        assert_eq!(path.subpaths.len(), 1);
        let want: Vec<Pt> = [2, 4, 1, 3, 0].iter().map(|&k| vertex(k, 5)).collect();
        assert!(close(&ends(&path.subpaths[0]), &want), "{:?}", ends(&path.subpaths[0]));
        // The base side alone tells the step when the field doesn't.
        assert!(close(&ends(&star(5, 2, 0, 0).subpaths[0]), &want));
    }

    #[test]
    fn a_star_sharing_a_factor_with_its_sides_draws_one_outline_for_each() {
        // {6/2}: two triangles, 0-2-4 and 1-3-5.
        let path = star(6, 2, 0, 2);
        assert_eq!(path.subpaths.len(), 2);
        assert!(close(&ends(&path.subpaths[0]), &[vertex(2, 6), vertex(4, 6), vertex(0, 6)]));
        assert!(close(&[path.subpaths[1].start], &[vertex(1, 6)]));
        assert!(close(&ends(&path.subpaths[1]), &[vertex(3, 6), vertex(5, 6), vertex(1, 6)]));
    }

    #[test]
    fn a_plain_polygon_turns_one_vertex_for_each_side() {
        let path = star(4, 1, 0, 1);
        assert_eq!(path.subpaths.len(), 1);
        assert!(close(&ends(&path.subpaths[0]), &[1, 2, 3, 0].map(|k| vertex(k, 4))));
    }
}
