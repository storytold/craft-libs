//! Outlines of curves (`PCvD` node lists), parametric shapes (`Shpe` objects) and compounds, in
//! node space.

use crate::model::{Affine, Path, Point, Reader, SubPath};
use crate::stream::{ObjId, Tag, Value};

/// Nodes per curve object.
const MAX_NODES: usize = 4_000_000;
/// A shared-object stream is bounded, but a compound graph can expand exponentially. Count
/// each visit and each curve record / generated contour / transformed segment across the whole
/// outline, including repeated references. Match the document reader's nesting limit.
const MAX_OUTLINE_DEPTH: usize = 128;
const MAX_OUTLINE_VISITS: usize = 100_000;

struct OutlineBudget {
    active: Vec<ObjId>,
    visits: usize,
    work: usize,
    failed: bool,
}

impl OutlineBudget {
    fn fail(&mut self, r: &mut Reader, warning: &'static str) -> Option<()> {
        if !self.failed {
            r.warn(warning);
        }
        self.failed = true;
        None
    }

    fn spend(&mut self, r: &mut Reader, work: usize) -> Option<()> {
        if work > self.work {
            return self.fail(r, "compound outlines exceeding the geometry work limit");
        }
        self.work -= work;
        Some(())
    }

    fn path(&mut self, r: &mut Reader, path: &Path) -> Option<()> {
        for subpath in &path.subpaths {
            self.spend(r, 1)?;
            self.spend(r, subpath.segments.len())?;
        }
        Some(())
    }
}

/// The node-space outline of a curve, shape or compound node, and whether it fills even-odd.
/// Instances (master-page or symbol copies) take the geometry of their master.
pub(crate) fn outline(r: &mut Reader, id: ObjId, class: Tag, world: Affine) -> Option<(Path, bool)> {
    let mut budget = OutlineBudget { active: Vec::new(), visits: MAX_OUTLINE_VISITS, work: MAX_NODES, failed: false };
    bounded_outline(r, id, class, world, &mut budget)
}

fn bounded_outline(r: &mut Reader, id: ObjId, class: Tag, world: Affine, budget: &mut OutlineBudget) -> Option<(Path, bool)> {
    if budget.active.contains(&id) {
        budget.fail(r, "a compound outline that contains itself")?;
    }
    if budget.active.len() >= MAX_OUTLINE_DEPTH {
        budget.fail(r, "compound outlines nested deeper than 128 levels")?;
    }
    if budget.visits == 0 {
        budget.fail(r, "compound outlines exceeding the operand visit limit")?;
    }
    budget.visits -= 1;
    budget.active.push(id);
    let result = outline_inner(r, id, class, world, budget);
    // The popped value is our own stack marker, and is no longer needed.
    let _ = budget.active.pop();
    result
}

fn outline_inner(r: &mut Reader, id: ObjId, class: Tag, world: Affine, budget: &mut OutlineBudget) -> Option<(Path, bool)> {
    let s = r.s;
    match &class.0.to_be_bytes() {
        b"PCrv" | b"TxtC" => {
            let crvs = s.obj(id, b"Crvs").or_else(|| s.obj(id, b"MCrM").and_then(|m| s.obj(m, b"Crvs")))?;
            let path = curves(r, crvs, budget)?;
            budget.path(r, &path)?;
            let multi = path.subpaths.len() > 1;
            Some((path, multi))
        }
        b"Comp" => {
            // One path of all the operands' contours in the compound's space, filled even-odd: this
            // matches add and subtract operands in every public sample; a true union of overlapping
            // added operands would differ.
            let mut path = Path::default();
            for c in s.objs(id, b"Chld") {
                let local = s.floats::<6>(c, b"Xfrm").map(Affine::from_xfrm).unwrap_or(Affine::IDENTITY);
                let class = s.class(c)?;
                if s.enumeration(c, b"ComO").is_some_and(|(op, _)| op != 0 && op != 2) {
                    r.warn("compound shapes that intersect or exclude (imported as add)");
                }
                let child_world = local.then(world);
                if !child_world.is_finite() {
                    budget.fail(r, "a compound operand with an invalid transform")?;
                }
                if let Some((mut p, _)) = bounded_outline(r, c, class, child_world, budget) {
                    // Charge before transforming or extending. Transform the owned path in place
                    // so nested compounds do not make an additional copy of every contour.
                    budget.path(r, &p)?;
                    for subpath in &mut p.subpaths {
                        subpath.start = local.apply(subpath.start);
                        for segment in &mut subpath.segments {
                            *segment = segment.map(|point| local.apply(point));
                        }
                    }
                    path.subpaths.extend(p.subpaths);
                } else if budget.failed {
                    // Never present a partial compound as complete after a cycle or budget limit.
                    return None;
                }
            }
            Some((path, true))
        }
        _ => {
            let (shape, box_owner) = match s.obj(id, b"Shpe") {
                Some(sh) => (sh, id),
                None => {
                    let master = s.obj(id, b"ShpM")?;
                    (s.obj(master, b"Shpe")?, master)
                }
            };
            let b = s.floats::<4>(box_owner, b"ShpB")?;
            let path = crate::shapes::shape(r, shape, b, world)?;
            budget.path(r, &path)?;
            Some((path, false))
        }
    }
}

/// `PCvD.Data`: positional `[u8, u32 count, then per subpath: bool closed, 18-byte records]`.
/// A record is (f64 x, f64 y, u8 kind, u8 role): role 0 is an anchor, 1 the out-handle of the
/// anchor before it, 2 the in-handle of the anchor after it. A closed subpath repeats its first
/// anchor at the end. `CnrD` live corners are rounded on the stored polyline.
fn curves(r: &mut Reader, crvs: ObjId, budget: &mut OutlineBudget) -> Option<Path> {
    let s = r.s;
    let data = s.obj(crvs, b"Data")?;
    let mut fields = s.positional(data);
    fields.next()?;
    let count = match fields.next()? {
        Value::UInt(n) => usize::try_from(*n).ok()?,
        _ => return None,
    };
    budget.spend(r, count)?;
    let corners = s.obj(crvs, b"CnrD").and_then(|c| corner_radii(s.positional(c).collect()));
    let mut path = Path::default();
    let mut seen = 0usize;
    for index in 0..count {
        let closed = match fields.next()? {
            Value::Bool(b) => *b,
            _ => return None,
        };
        let Value::Array(records) = fields.next()? else { return None };
        seen = seen.checked_add(records.len())?;
        if seen > MAX_NODES {
            budget.fail(r, "curves with more than four million nodes")?;
        }
        budget.spend(r, records.len())?;
        let anchors = anchors(records)?;
        let radii = if index == 0 { corners.as_ref() } else { None };
        if let Some(sp) = subpath(&anchors, closed, radii) {
            path.subpaths.push(sp);
        }
    }
    if corners.is_some() && count > 1 {
        r.warn("rounded corners on curves with several subpaths (only the first is rounded)");
    }
    Some(path)
}

/// Most segments a stroke's pressure curve may have; the curves in real documents have a handful.
const MAX_PRESSURE_SEGMENTS: usize = 256;

/// Most regular samples of a pressure curve: each segment gets an equal share, at least 2 (its
/// middle and its end) and 8 at most. The points where a segment's width stops rising or falling
/// come on top (at most two per segment), so a curve has at most 1 + 4 × [`MAX_PRESSURE_SEGMENTS`]
/// points; the document's limit counts all of them.
const PRESSURE_POINTS: usize = 512;

/// Most points one pressure curve can have (see [`PRESSURE_POINTS`]). A curve that can't be read
/// counts this many against the document's limit.
pub(crate) const MAX_PRESSURE_CURVE_POINTS: usize = 1 + 4 * MAX_PRESSURE_SEGMENTS;

/// A stroke's pressure curve (`LDsc.LDeP`, a `CrvD` whose positional `Data` holds an array of
/// 18-byte records as in [`curves`]): x is the position along the path (0 to 1) and y the width as
/// a fraction of the stroke's weight. Each segment is sampled at 2 to 8 points, at most
/// [`PRESSURE_POINTS`] in all, and also where its width stops rising or falling (at most two more
/// points), so a dip between samples is kept → (position, width) in order along the path,
/// positions clamped into 0..1 and never going back, widths clamped into 0..1. `None` when the
/// curve can't be read or has more than [`MAX_PRESSURE_SEGMENTS`] segments.
pub(crate) fn pressure(r: &Reader, crvd: ObjId) -> Option<Vec<(f64, f64)>> {
    let s = r.s;
    let data = s.obj(crvd, b"Data")?;
    let records = s.positional(data).find_map(|v| if let Value::Array(a) = v { Some(a) } else { None })?;
    // Every segment takes at least an anchor record.
    if records.len() > MAX_PRESSURE_SEGMENTS * 3 + 1 {
        return None;
    }
    let curve = subpath(&anchors(records)?, false, None)?;
    if curve.segments.len() > MAX_PRESSURE_SEGMENTS {
        return None;
    }
    let steps = u32::try_from((PRESSURE_POINTS / curve.segments.len().max(1)).clamp(2, 8)).unwrap_or(2);
    let mut samples = vec![(curve.start.x, curve.start.y)];
    let mut from = curve.start;
    for [c1, c2, to] in &curve.segments {
        let mut ts: Vec<f64> = (1..=steps).map(|i| f64::from(i) / f64::from(steps)).collect();
        ts.extend(turning_points(from.y, c1.y, c2.y, to.y).into_iter().flatten());
        ts.sort_by(f64::total_cmp);
        ts.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
        for t in ts {
            let u = 1.0 - t;
            let at = |a: f64, b: f64, c: f64, d: f64| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
            samples.push((at(from.x, c1.x, c2.x, to.x), at(from.y, c1.y, c2.y, to.y)));
        }
        from = *to;
    }
    let mut last = 0.0f64;
    Some(
        samples
            .into_iter()
            .map(|(x, y)| {
                last = x.clamp(last, 1.0);
                (last, y.clamp(0.0, 1.0))
            })
            .collect(),
    )
}

/// The parameters strictly between 0 and 1 where the cubic Bézier with control values `a`, `b`,
/// `c` and `d` stops rising or falling (where its derivative is 0). The values are scaled into
/// -1..1 first, so huge ones can't overflow the discriminant.
fn turning_points(a: f64, b: f64, c: f64, d: f64) -> [Option<f64>; 2] {
    let scale = a.abs().max(b.abs()).max(c.abs()).max(d.abs());
    if !(scale.is_finite() && scale > 0.0) {
        return [None, None];
    }
    let (a, b, c, d) = (a / scale, b / scale, c / scale, d / scale);
    let (d0, d1, d2) = (b - a, c - b, d - c);
    let (qa, qb, qc) = (d0 - 2.0 * d1 + d2, 2.0 * (d1 - d0), d0);
    let inside = |t: f64| (t.is_finite() && t > 0.0 && t < 1.0).then_some(t);
    if qa.abs() < 1e-12 {
        return [if qb.abs() < 1e-12 { None } else { inside(-qc / qb) }, None];
    }
    let disc = qb * qb - 4.0 * qa * qc;
    if disc < 0.0 {
        return [None, None];
    }
    let root = disc.sqrt();
    [inside((-qb - root) / (2.0 * qa)), inside((-qb + root) / (2.0 * qa))]
}

struct Anchor {
    p: Point,
    inh: Option<Point>,
    out: Option<Point>,
    /// Index of the anchor's record (live corners refer to records).
    record: usize,
}

fn record(v: &Value) -> Option<(Point, u8)> {
    let Value::Bytes(b) = v else { return None };
    if b.len() != 18 {
        return None;
    }
    let x = f64::from_le_bytes(b.get(0..8)?.try_into().ok()?);
    let y = f64::from_le_bytes(b.get(8..16)?.try_into().ok()?);
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    Some((Point { x, y }, *b.get(17)?))
}

fn anchors(records: &[Value]) -> Option<Vec<Anchor>> {
    let mut out: Vec<Anchor> = Vec::with_capacity(records.len() / 3 + 1);
    let mut pending_in = None;
    for (i, v) in records.iter().enumerate() {
        let (p, role) = record(v)?;
        match role {
            0 => {
                out.push(Anchor { p, inh: pending_in.take(), out: None, record: i });
            }
            1 => {
                // A leading out-handle without an anchor is edit state only.
                if let Some(a) = out.last_mut() {
                    a.out = Some(p);
                }
            }
            2 => pending_in = Some(p),
            _ => return None,
        }
    }
    Some(out)
}

fn subpath(anchors: &[Anchor], closed: bool, radii: Option<&Vec<(usize, f64)>>) -> Option<SubPath> {
    let first = anchors.first()?;
    if let Some(radii) = radii.filter(|r| !r.is_empty()) {
        // The largest radius of each record, once (not a scan of all of them for each anchor).
        let mut largest: std::collections::HashMap<usize, f64> = std::collections::HashMap::new();
        for (rec, r) in radii {
            let e = largest.entry(*rec).or_insert(0.0);
            *e = e.max(*r);
        }
        let corner = |a: &Anchor| largest.get(&a.record).copied().unwrap_or(0.0);
        let polyline = anchors.iter().all(|a| a.inh.is_none_or(|h| h == a.p) && a.out.is_none_or(|h| h == a.p));
        if polyline {
            let mut pts: Vec<(Point, f64)> = anchors.iter().map(|a| (a.p, corner(a))).collect();
            // A closed subpath repeats its first anchor; its radius may be on either copy.
            if closed
                && pts.len() > 1
                && pts.first().map(|p| p.0) == pts.last().map(|p| p.0)
                && let Some((_, r)) = pts.pop()
                && let Some(f) = pts.first_mut()
            {
                f.1 = f.1.max(r);
            }
            return Some(crate::shapes::fillet(&pts, closed));
        }
    }
    let segments = anchors
        .windows(2)
        .filter_map(|w| {
            let [a, b] = w else { return None };
            Some([a.out.unwrap_or(a.p), b.inh.unwrap_or(b.p), b.p])
        })
        .collect();
    Some(SubPath { start: first.p, segments, closed })
}

/// `CnrD`: positional `[u8, i32[] record indices, f64[] radii, u8[] types]`.
fn corner_radii(fields: Vec<&Value>) -> Option<Vec<(usize, f64)>> {
    let (Some(Value::Array(idx)), Some(Value::Array(rad))) = (fields.get(1), fields.get(2)) else { return None };
    Some(
        idx.iter()
            .zip(rad)
            .filter_map(|(i, r)| match (i, r) {
                (Value::Int(i), Value::Float(r)) if r.is_finite() && *r > 0.0 => Some((usize::try_from(*i).ok()?, *r)),
                _ => None,
            })
            .collect(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn turning_points_of_huge_control_values_are_found() {
        let small = turning_points(1.0, -3.0, 3.0, 1.0);
        let huge = turning_points(1e200, -3e200, 3e200, 1e200);
        assert!(small.iter().all(Option::is_some), "{small:?}");
        for (s, h) in small.iter().zip(&huge) {
            let (Some(s), Some(h)) = (s, h) else { panic!("{huge:?}") };
            assert!((s - h).abs() < 1e-12, "{small:?} {huge:?}");
        }
    }
}
