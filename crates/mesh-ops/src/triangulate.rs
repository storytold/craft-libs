//! Triangulation of a polygon with holes by ear clipping, as Eberly's *Triangulation by Ear
//! Clipping* describes it (<https://www.geometrictools.com/Documentation/TriangulationByEarClipping.pdf>):
//! each hole is joined to the outer contour through a bridge from its rightmost vertex to a
//! visible contour vertex, then ears are clipped from the joined ring.
//!
//! Contours are closed polylines in a plane, the last point not repeated. [`triangulate`] checks
//! that the outer contour and every hole are simple (no edge crosses or touches another edge, and
//! no point repeats), that every hole lies inside the outer contour and outside every other hole,
//! and that the work stays within [`Limits::max_outline_work`]. Every geometric test works on
//! differences of the coordinates as given, so a contour far from the origin keeps its shape and
//! points far apart in scale stay apart. Every decision on which side of a line a point lies
//! (convex and reflex corners, straight corners, crossings and containment) takes the exact sign
//! of the cross product, computed again exactly when rounding could change it. The sign is exact
//! unless a product of coordinate differences, or its rounding error, falls below the normal
//! `f64` range.

use crate::mesh::{exact_sum, two_sum};
use crate::{Limits, MeshOpsError, Result, check_limit, try_vec};

type P = [f64; 2];

/// Largest width or height of the input's bounding box: the cross products of coordinate
/// differences up to this size stay finite.
pub(crate) const MAX_OUTLINE_EXTENT: f64 = 1e150;

/// Twice the signed area of a closed contour (last point not repeated): positive when the points
/// run counterclockwise with y up, as for the shoelace formula on MathWorld
/// (<https://mathworld.wolfram.com/PolygonArea.html>). The terms are the cross products of the
/// edges seen from the contour's lowest point in x, then y, so the same terms come out wherever the
/// contour starts and a contour far from the origin keeps its area. Each term has its sign exact
/// and is rounded about once, and the terms are added with their rounding errors carried along. A
/// contour with fewer than three points has area 0.
pub fn signed_area(c: &[P]) -> f64 {
    let n = c.len();
    let Some(o) = c.iter().copied().min_by(|p, q| p[0].total_cmp(&q[0]).then(p[1].total_cmp(&q[1]))).filter(|_| n >= 3) else { return 0.0 };
    let (mut sum, mut error) = (0.0, 0.0);
    for i in 0..n {
        let (s, e) = two_sum(sum, cross(o, c[i], c[(i + 1) % n]));
        sum = s;
        error += e;
    }
    sum + error
}

/// Counts the steps of one outline call against [`Limits::max_outline_work`].
pub(crate) struct Work {
    done: u64,
    limit: u64,
}

impl Work {
    pub(crate) fn new(limits: &Limits) -> Work {
        Work { done: 0, limit: limits.max_outline_work }
    }

    /// Adds `steps`, or returns `TooLarge` when the total passes the limit.
    pub(crate) fn add(&mut self, steps: u64) -> Result<()> {
        self.done = self.done.saturating_add(steps);
        if self.done > self.limit { Err(MeshOpsError::TooLarge { what: "outline work", got: self.done, limit: self.limit }) } else { Ok(()) }
    }
}

/// `(a - o) × (b - o)`, with its sign exact: positive when `o`, `a`, `b` run counterclockwise with
/// y up, negative when clockwise and 0 only when they lie on one line. A result larger than the
/// error bound of the plain formula (Shewchuk's `ccwerrboundA` for `orient2d`,
/// <https://people.eecs.berkeley.edu/~jrs/papers/robustr.pdf>) is returned as it is; any other is
/// computed again exactly ([`exact_cross`]). Products that fall below the normal `f64` range
/// (coordinate differences under about 1e-154) lose their rounding error.
#[inline]
pub(crate) fn cross(o: P, a: P, b: P) -> f64 {
    let (l, r) = ((a[0] - o[0]) * (b[1] - o[1]), (a[1] - o[1]) * (b[0] - o[0]));
    let det = l - r;
    const U: f64 = f64::EPSILON / 2.0;
    if det.abs() > (3.0 + 16.0 * U) * U * (l.abs() + r.abs()) { det } else { exact_cross(o, a, b) }
}

/// `(a - o) × (b - o)` from the exact differences of the coordinates: each difference is split
/// into its rounded value and its rounding error, each of the eight products of those parts into
/// its rounded value and its rounding error, and the sixteen parts are added exactly
/// ([`exact_sum`]).
fn exact_cross(o: P, a: P, b: P) -> f64 {
    let diff = |x: f64, y: f64| two_sum(x, -y);
    let ((ax, axe), (ay, aye)) = (diff(a[0], o[0]), diff(a[1], o[1]));
    let ((bx, bxe), (by, bye)) = (diff(b[0], o[0]), diff(b[1], o[1]));
    let products = [(ax, by), (ax, bye), (axe, by), (axe, bye), (-ay, bx), (-ay, bxe), (-aye, bx), (-aye, bxe)];
    let mut terms = [0.0f64; 16];
    for (k, (x, y)) in products.into_iter().enumerate() {
        let p = x * y;
        terms[2 * k] = p;
        terms[2 * k + 1] = x.mul_add(y, -p);
    }
    exact_sum(&terms)
}

fn in_triangle(p: P, a: P, b: P, c: P) -> bool {
    let d1 = cross(a, b, p);
    let d2 = cross(b, c, p);
    let d3 = cross(c, a, p);
    d1 >= 0.0 && d2 >= 0.0 && d3 >= 0.0
}

/// The items of `it` in a vector reserved with [`try_vec`], or `OutOfMemory`.
pub(crate) fn try_collect<T>(it: impl ExactSizeIterator<Item = T>, what: &'static str) -> Result<Vec<T>> {
    let mut v = try_vec(it.len(), what)?;
    v.extend(it);
    Ok(v)
}

/// Whether `p` is inside the closed contour `c` (even-odd rule). A contour with fewer than three
/// points holds nothing. An edge counts when one end is above `p` and the other is not (a ray
/// through a vertex counts one of its two edges) and `p` lies strictly on its left seen from its
/// lower end, by the sign of [`cross`].
pub(crate) fn point_in(p: P, c: &[P]) -> bool {
    let n = c.len();
    if n < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = n - 1;
    for i in 0..n {
        let (a, b) = (c[i], c[j]);
        if (a[1] > p[1]) != (b[1] > p[1]) {
            // `p` on the left of the edge seen upward: the edge crosses the ray +x from `p`.
            let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
            if cross(lo, hi, p) > 0.0 {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Whether `p`, on the line through `a` and `b`, lies on the segment between them.
fn on_segment(a: P, b: P, p: P) -> bool {
    a[0].min(b[0]) <= p[0] && p[0] <= a[0].max(b[0]) && a[1].min(b[1]) <= p[1] && p[1] <= a[1].max(b[1])
}

/// Whether `p` lies on the segment `a`-`b`.
pub(crate) fn on_edge(a: P, b: P, p: P) -> bool {
    cross(a, b, p) == 0.0 && on_segment(a, b, p)
}

/// Whether segments `a`-`b` and `c`-`d` cross at a point inside both: each one's endpoints lie
/// strictly on opposite sides of the other's line.
pub(crate) fn segments_cross(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0))
}

/// Whether segments `a`-`b` and `c`-`d` share at least one point.
fn segments_meet(a: P, b: P, c: P, d: P) -> bool {
    let (d1, d2, d3, d4) = (cross(c, d, a), cross(c, d, b), cross(a, b, c), cross(a, b, d));
    if ((d1 > 0.0 && d2 < 0.0) || (d1 < 0.0 && d2 > 0.0)) && ((d3 > 0.0 && d4 < 0.0) || (d3 < 0.0 && d4 > 0.0)) {
        return true;
    }
    (d1 == 0.0 && on_segment(c, d, a))
        || (d2 == 0.0 && on_segment(c, d, b))
        || (d3 == 0.0 && on_segment(a, b, c))
        || (d4 == 0.0 && on_segment(a, b, d))
}

/// The contours of one polygon, concatenated, with their coordinates as given.
pub(crate) struct Polygon {
    pub(crate) pts: Vec<P>,
    /// Start of each contour in `pts`, outer first, and `pts.len()` at the end.
    pub(crate) starts: Vec<usize>,
}

impl Polygon {
    fn contours(&self) -> usize {
        self.starts.len() - 1
    }

    fn contour(&self, c: usize) -> &[P] {
        &self.pts[self.starts[c]..self.starts[c + 1]]
    }

    /// Endpoints of edge `i` of contour `c`.
    fn edge(&self, c: usize, i: usize) -> (P, P) {
        let r = self.contour(c);
        (r[i], r[(i + 1) % r.len()])
    }
}

/// Checks the counts and values of an outer contour and its holes and copies them into one
/// [`Polygon`].
fn gather(outer: &[P], holes: &[&[P]], limits: &Limits) -> Result<Polygon> {
    check_limit("outline contour count", holes.len().saturating_add(1), limits.max_outline_contours)?;
    let total = holes.iter().try_fold(outer.len(), |n, h| n.checked_add(h.len())).unwrap_or(usize::MAX);
    check_limit("outline vertex count", total, limits.max_outline_vertices)?;
    if u32::try_from(total).is_err() {
        return Err(MeshOpsError::TooLarge { what: "outline vertex count", got: total as u64, limit: u64::from(u32::MAX) });
    }
    let mut pts = try_vec(total, "outline points")?;
    let mut starts = try_vec(holes.len() + 2, "outline contours")?;
    for (c, ring) in std::iter::once(outer).chain(holes.iter().copied()).enumerate() {
        if ring.len() < 3 {
            return Err(MeshOpsError::Invalid(format!("outline contour {c} has {} points; a contour needs at least 3", ring.len())));
        }
        starts.push(pts.len());
        pts.extend_from_slice(ring);
    }
    starts.push(pts.len());
    if !pts.iter().flatten().all(|v| v.is_finite()) {
        return Err(MeshOpsError::Invalid("an outline point is not finite".into()));
    }
    let lo = pts.iter().fold([f64::INFINITY; 2], |lo, p| [lo[0].min(p[0]), lo[1].min(p[1])]);
    let hi = pts.iter().fold([f64::NEG_INFINITY; 2], |hi, p| [hi[0].max(p[0]), hi[1].max(p[1])]);
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    if extent > MAX_OUTLINE_EXTENT {
        return Err(MeshOpsError::Invalid(format!("the outline spans {extent}, over {MAX_OUTLINE_EXTENT}")));
    }
    Ok(Polygon { pts, starts })
}

/// Rejects a contour with a repeated point or no area, and any edge that meets another edge
/// other than at the corner two neighboring edges share.
fn check_simple(poly: &Polygon, work: &mut Work) -> Result<()> {
    for c in 0..poly.contours() {
        let r = poly.contour(c);
        if (0..r.len()).any(|i| r[i] == r[(i + 1) % r.len()]) {
            return Err(MeshOpsError::Invalid(format!("outline contour {c} repeats a point")));
        }
        if signed_area(r) == 0.0 {
            return Err(MeshOpsError::Invalid(format!("outline contour {c} has no area")));
        }
    }
    let edges = poly.pts.len() as u64;
    work.add(edges.saturating_mul(edges) / 2)?;
    let mut list: Vec<(usize, usize)> = try_vec(poly.pts.len(), "outline edges")?;
    list.extend((0..poly.contours()).flat_map(|c| (0..poly.contour(c).len()).map(move |i| (c, i))));
    for (k, &(ca, ia)) in list.iter().enumerate() {
        let (a, b) = poly.edge(ca, ia);
        for &(cb, ib) in &list[k + 1..] {
            let (c, d) = poly.edge(cb, ib);
            if ca == cb && neighbors(ia, ib, poly.contour(ca).len()) {
                // Neighboring edges share one corner; they may not run back over each other.
                let (p, q, s) = if b == c { (a, b, d) } else { (c, d, b) };
                if cross(p, q, s) == 0.0 && (q[0] - p[0]) * (s[0] - q[0]) + (q[1] - p[1]) * (s[1] - q[1]) < 0.0 {
                    return Err(MeshOpsError::Invalid(format!("outline contour {ca} doubles back on itself")));
                }
            } else if segments_meet(a, b, c, d) {
                return Err(MeshOpsError::Invalid(if ca == cb {
                    format!("outline contour {ca} crosses itself")
                } else {
                    format!("outline contours {ca} and {cb} cross or touch")
                }));
            }
        }
    }
    Ok(())
}

/// Whether edges `i` and `j` of a contour of `n` edges share a corner.
fn neighbors(i: usize, j: usize, n: usize) -> bool {
    (i + 1) % n == j || (j + 1) % n == i
}

/// Rejects a hole outside the outer contour or inside another hole. The contours do not meet
/// ([`check_simple`]), so one point of each hole decides.
fn check_nesting(poly: &Polygon, work: &mut Work) -> Result<()> {
    let outer = poly.contour(0);
    for h in 1..poly.contours() {
        let p = poly.contour(h)[0];
        work.add(outer.len() as u64)?;
        if !point_in(p, outer) {
            return Err(MeshOpsError::Invalid(format!("outline hole {} is outside the outer contour", h - 1)));
        }
        for other in 1..poly.contours() {
            if other == h {
                continue;
            }
            work.add(poly.contour(other).len() as u64)?;
            if point_in(p, poly.contour(other)) {
                return Err(MeshOpsError::Invalid(format!("outline hole {} is inside hole {}", h - 1, other - 1)));
            }
        }
    }
    Ok(())
}

/// Checks an outer contour and its holes as [`triangulate`] does, and returns them as one
/// [`Polygon`].
pub(crate) fn checked_polygon(outer: &[P], holes: &[&[P]], limits: &Limits, work: &mut Work) -> Result<Polygon> {
    let poly = gather(outer, holes, limits)?;
    check_simple(&poly, work)?;
    check_nesting(&poly, work)?;
    Ok(poly)
}

/// Triangulates an outer contour with holes. Contours are closed polylines (last point not
/// repeated) in either direction. The triangles index the contours concatenated in order
/// (`outer`, then each hole) and run counterclockwise with y up. A vertex where the contour goes
/// straight on is clipped as a triangle without area, so every contour edge is an edge of
/// exactly one triangle.
///
/// Errors: `Invalid` for a contour with fewer than three points, a non-finite point, a repeated
/// point, a contour without area, edges that cross or touch, a hole outside the outer contour or
/// inside another hole, a bounding box wider or taller than 1e150, and a polygon in which no ear
/// can be found; `TooLarge` past [`Limits::max_outline_vertices`],
/// [`Limits::max_outline_contours`] or [`Limits::max_outline_work`].
pub fn triangulate(outer: &[P], holes: &[&[P]], limits: &Limits) -> Result<Vec<[u32; 3]>> {
    triangulate_counted(outer, holes, limits, &mut Work::new(limits))
}

/// [`triangulate`], adding its steps to `work`.
pub(crate) fn triangulate_counted(outer: &[P], holes: &[&[P]], limits: &Limits, work: &mut Work) -> Result<Vec<[u32; 3]>> {
    let poly = checked_polygon(outer, holes, limits, work)?;
    let pts = &poly.pts;
    // Ring of global indices, outer counterclockwise.
    let mut ring: Vec<usize> = try_vec(pts.len() + 2 * poly.contours(), "outline ring")?;
    ring.extend(0..poly.starts[1]);
    if signed_area(poly.contour(0)) < 0.0 {
        ring.reverse();
    }
    // Holes clockwise, rightmost first.
    let mut hs: Vec<Vec<usize>> = try_vec(poly.contours() - 1, "outline holes")?;
    for h in 1..poly.contours() {
        let mut r: Vec<usize> = try_collect(poly.starts[h]..poly.starts[h + 1], "outline hole")?;
        if signed_area(poly.contour(h)) > 0.0 {
            r.reverse();
        }
        hs.push(r);
    }
    let right = |r: &Vec<usize>| r.iter().map(|&i| pts[i][0]).fold(f64::NEG_INFINITY, f64::max);
    // Rightmost first, holes with the same rightmost x in input order (each hole's indices start
    // after the previous hole's); an unstable sort allocates nothing.
    hs.sort_unstable_by(|a, b| right(b).total_cmp(&right(a)).then_with(|| a.first().cmp(&b.first())));
    for h in hs {
        bridge(pts, &mut ring, &h, work)?;
    }
    ear_clip(pts, ring, work)
}

/// The nearest ring edge the ray +x from `mp` reaches: where it does, and the edge's index. Only
/// edges that run upward across the ray count: the ring is counterclockwise, so those face the
/// ray from inside. The two copies of a bridge run opposite ways, so only one of them counts.
fn ray_hit(pts: &[P], ring: &[usize], mp: P) -> Option<(f64, usize)> {
    let n = ring.len();
    let mut best: Option<(f64, usize)> = None;
    for i in 0..n {
        let (a, b) = (pts[ring[i]], pts[ring[(i + 1) % n]]);
        // The edge reaches the ray at or past `mp` when `mp` lies on it or on its left.
        if !(a[1] <= mp[1] && mp[1] <= b[1] && a[1] < b[1]) || cross(a, b, mp) < 0.0 {
            continue;
        }
        let t = (mp[1] - a[1]) / (b[1] - a[1]);
        let x = (a[0] + t * (b[0] - a[0])).max(mp[0]);
        if best.is_none_or(|(bx, _)| x < bx) {
            best = Some((x, i));
        }
    }
    best
}

/// Joins hole `h` into `ring` through a bridge from the hole's rightmost vertex.
fn bridge(pts: &[P], ring: &mut Vec<usize>, h: &[usize], work: &mut Work) -> Result<()> {
    let n = ring.len();
    work.add(2 * n as u64)?;
    let no_bridge = || MeshOpsError::Invalid("an outline hole cannot be joined to the outer contour".into());
    let Some((hk, &m)) = h.iter().enumerate().max_by(|a, b| pts[*a.1][0].total_cmp(&pts[*b.1][0])) else { return Err(no_bridge()) };
    let mp = pts[m];
    let Some((x, i)) = ray_hit(pts, ring, mp) else { return Err(no_bridge()) };
    let (a, b) = (ring[i], ring[(i + 1) % n]);
    // Candidate: the edge endpoint with the larger x.
    let mut cand = if pts[a][0] > pts[b][0] { (i, a) } else { ((i + 1) % n, b) };
    let ip = [x, mp[1]];
    // Reflex ring vertices inside triangle (M, I, P) block the view: take the one with the
    // smallest angle to the ray (closest on ties).
    let pp = pts[cand.1];
    let side = cross(mp, ip, pp);
    let (t0, t1, t2) = if side >= 0.0 { (mp, ip, pp) } else { (mp, pp, ip) };
    let mut best_ang = f64::INFINITY;
    // When the ray reaches P itself (P on the ray), the triangle collapses to a segment of the
    // ray and nothing between M and P blocks the view: every point of the line would pass the
    // three cross-product tests, those behind M included.
    for k in (0..n).filter(|_| side != 0.0) {
        let v = ring[k];
        if v == cand.1 {
            continue;
        }
        let q = pts[v];
        let prev = pts[ring[(k + n - 1) % n]];
        let next = pts[ring[(k + 1) % n]];
        let reflex = cross(prev, q, next) <= 0.0;
        if reflex && in_triangle(q, t0, t1, t2) {
            let ang = (q[1] - mp[1]).abs().atan2(q[0] - mp[0]);
            let d = (q[0] - mp[0]).hypot(q[1] - mp[1]);
            let cd = (pts[cand.1][0] - mp[0]).hypot(pts[cand.1][1] - mp[1]);
            if ang < best_ang || (ang == best_ang && d < cd) {
                best_ang = ang;
                cand = (k, v);
            }
        }
    }
    let (k, _) = cand;
    // ring[..=k], hole from M around back to M, ring[k] again, rest. The ring was reserved for
    // every hole and its two bridge points.
    let mut out = try_vec(ring.len() + h.len() + 2, "outline ring")?;
    out.extend_from_slice(&ring[..=k]);
    for j in 0..=h.len() {
        out.push(h[(hk + j) % h.len()]);
    }
    out.extend_from_slice(&ring[k..]);
    *ring = out;
    Ok(())
}

/// Whether the corner at `b` of `a`-`b`-`c` is an ear of `ring`: convex with no other ring vertex
/// inside, or a vertex where the ring goes straight on.
fn is_ear(pts: &[P], ring: &[usize], [a, b, c]: [usize; 3]) -> bool {
    let (pa, pb, pc) = (pts[a], pts[b], pts[c]);
    let cr = cross(pa, pb, pc);
    // A straight-through vertex (the cross product is exactly zero and the ring goes on rather
    // than back) is clipped as a triangle without area, which keeps every contour edge in the
    // triangulation, so caps meet walls without T-junctions. A turn to the right is reflex. Every
    // turn to the left, however shallow, needs the blocker test.
    if cr == 0.0 {
        return (pa[0] - pb[0]) * (pc[0] - pb[0]) + (pa[1] - pb[1]) * (pc[1] - pb[1]) <= 0.0;
    }
    if cr < 0.0 {
        return false;
    }
    !ring.iter().any(|&v| {
        let q = pts[v];
        // Bridged duplicates share positions with the ear's corners.
        v != a && v != b && v != c && q != pa && q != pb && q != pc && in_triangle(q, pa, pb, pc)
    })
}

fn ear_clip(pts: &[P], mut ring: Vec<usize>, work: &mut Work) -> Result<Vec<[u32; 3]>> {
    let mut tris = try_vec(ring.len().saturating_sub(2), "triangles")?;
    let mut guard = 0usize;
    let mut i = 0usize;
    // Every index is below the vertex count, which `gather` checked fits `u32`.
    let id = |v: usize| v as u32;
    while ring.len() > 3 {
        let n = ring.len();
        if guard > 2 * n {
            return Err(MeshOpsError::Invalid("no ear found in the outline".into()));
        }
        work.add(n as u64)?;
        let ii = i % n;
        let (a, b, c) = (ring[(ii + n - 1) % n], ring[ii], ring[(ii + 1) % n]);
        if is_ear(pts, &ring, [a, b, c]) {
            tris.push([id(a), id(b), id(c)]);
            ring.remove(ii);
            guard = 0;
            i = ii.saturating_sub(1);
        } else {
            i = ii + 1;
            guard += 1;
        }
    }
    if let [a, b, c] = ring[..] {
        tris.push([id(a), id(b), id(c)]);
    }
    Ok(tris)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area_of(pts: &[P], tris: &[[u32; 3]]) -> f64 {
        tris.iter().map(|t| signed_area(&[pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]) * 0.5).sum()
    }

    /// Every contour edge is an edge of exactly one triangle, running the same way as the
    /// counterclockwise outer contour (clockwise holes).
    fn assert_edges_covered(pts: &[P], contours: &[&[P]], tris: &[[u32; 3]]) {
        let mut edges = std::collections::HashMap::new();
        for t in tris {
            for k in 0..3 {
                *edges.entry((t[k], t[(k + 1) % 3])).or_insert(0) += 1;
            }
        }
        let mut start = 0u32;
        for (c, ring) in contours.iter().enumerate() {
            let n = ring.len() as u32;
            let forward = (signed_area(ring) > 0.0) == (c == 0);
            for i in 0..n {
                let (a, b) = (start + i, start + (i + 1) % n);
                let e = if forward { (a, b) } else { (b, a) };
                assert_eq!(edges.get(&e).copied().unwrap_or(0), 1, "edge {e:?} of contour {c}");
            }
            start += n;
        }
        assert_eq!(pts.len() as u32, start);
    }

    #[test]
    fn triangulation_covers_polygon_with_hole() {
        let l = Limits::default();
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [5.0, 12.0], [0.0, 10.0]];
        let hole = vec![[3.0, 3.0], [3.0, 7.0], [7.0, 7.0], [7.0, 3.0]];
        let pts: Vec<[f64; 2]> = outer.iter().chain(hole.iter()).copied().collect();
        let tris = triangulate(&outer, &[&hole], &l).unwrap();
        let area = area_of(&pts, &tris);
        let expect = signed_area(&outer).abs() * 0.5 - 16.0;
        assert!((area - expect).abs() < 1e-9, "{area} vs {expect}");
        assert!(tris.iter().all(|t| signed_area(&[pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]) >= 0.0));
        assert_edges_covered(&pts, &[&outer, &hole], &tris);
        // A concave "C".
        let c = vec![[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 4.0], [6.0, 4.0], [6.0, 6.0], [0.0, 6.0]];
        let tris = triangulate(&c, &[], &l).unwrap();
        assert!((area_of(&c, &tris) - 28.0).abs() < 1e-9);
        assert_edges_covered(&c, &[&c], &tris);
    }

    #[test]
    fn several_holes_in_a_concave_outline_are_covered_exactly() {
        // A "U" of area 64 - 24 = 40, clockwise, with three square holes of area 1 each. The top
        // hole's ray reaches the bridge of the right hole, whose two copies lie on one line.
        let u = vec![[0.0, 0.0], [0.0, 8.0], [8.0, 8.0], [8.0, 0.0], [6.0, 0.0], [6.0, 6.0], [2.0, 6.0], [2.0, 0.0]];
        let sq = |x: f64, y: f64| vec![[x, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]];
        let (h0, h1, h2) = (sq(0.5, 1.0), sq(6.5, 3.0), sq(3.5, 6.5));
        let tris = triangulate(&u, &[&h0, &h1, &h2], &Limits::default()).unwrap();
        let pts: Vec<P> = [&u, &h0, &h1, &h2].iter().flat_map(|c| c.iter().copied()).collect();
        assert!((area_of(&pts, &tris) - 37.0).abs() < 1e-9);
        assert!(tris.iter().all(|t| signed_area(&[pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]) >= 0.0));
        assert_edges_covered(&pts, &[&u, &h0, &h1, &h2], &tris);
    }

    #[test]
    fn a_small_notch_at_large_coordinates_is_not_a_straight_vertex() {
        // The notch's reflex corner, listed first so it is the first corner tried, turns by 90°
        // over edges of length √2; only the edges, not the coordinates near 2e6, decide whether a
        // corner goes straight on.
        let outer = vec![[1e6, 2e6 - 1.0], [1e6 - 1.0, 2e6], [0.0, 2e6], [0.0, 0.0], [2e6, 0.0], [2e6, 2e6], [1e6 + 1.0, 2e6]];
        let tris = triangulate(&outer, &[], &Limits::default()).unwrap();
        assert!(tris.iter().all(|t| signed_area(&[outer[t[0] as usize], outer[t[1] as usize], outer[t[2] as usize]]) >= 0.0), "{tris:?}");
        assert!((area_of(&outer, &tris) - (4e12 - 1.0)).abs() < 1e-3);
        assert_edges_covered(&outer, &[&outer], &tris);
        // A corner on a straight line still becomes a triangle without area.
        let straight = vec![[0.0, 0.0], [1e6, 0.0], [2e6, 0.0], [2e6, 1.0], [0.0, 1.0]];
        let tris = triangulate(&straight, &[], &Limits::default()).unwrap();
        assert_edges_covered(&straight, &[&straight], &tris);
        // A very shallow notch, tried first, is reflex and gives no clockwise triangle.
        let shallow = vec![[5.0, 10.0 - 1e-13], [0.0, 10.0], [0.0, 0.0], [10.0, 0.0], [10.0, 10.0]];
        let tris = triangulate(&shallow, &[], &Limits::default()).unwrap();
        let at = |i: u32| shallow[i as usize];
        assert!(tris.iter().all(|t| signed_area(&[at(t[0]), at(t[1]), at(t[2])]) >= 0.0), "{tris:?}");
        assert_edges_covered(&shallow, &[&shallow], &tris);
    }

    #[test]
    fn a_hole_whose_ray_reaches_a_ring_vertex_bridges_to_it() {
        // The ray from the hole's rightmost point (5, 5) reaches the vertex (10, 5); the reflex
        // vertex (1, 5) lies on the same line behind the hole.
        let outer = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 5.0], [10.0, 10.0], [0.0, 10.0], [0.0, 6.0], [1.0, 5.0], [0.0, 4.0]];
        let hole = vec![[3.0, 4.0], [3.0, 6.0], [5.0, 5.0]];
        let tris = triangulate(&outer, &[&hole], &Limits::default()).unwrap();
        let pts: Vec<P> = outer.iter().chain(&hole).copied().collect();
        assert!(tris.iter().all(|t| signed_area(&[pts[t[0] as usize], pts[t[1] as usize], pts[t[2] as usize]]) >= 0.0), "{tris:?}");
        assert!((area_of(&pts, &tris) - (signed_area(&outer).abs() - signed_area(&hole).abs()) * 0.5).abs() < 1e-9);
        assert_edges_covered(&pts, &[&outer, &hole], &tris);
    }

    #[test]
    fn a_shallow_convex_corner_with_a_vertex_inside_is_not_clipped() {
        // The first corner turns left by about 1e-13 and its triangle holds (1, -5e-14).
        let c = vec![[1.0, -1e-13], [2.0, 0.0], [1.0, -5e-14], [0.0, 0.0]];
        let tris = triangulate(&c, &[], &Limits::default()).unwrap();
        assert!(tris.iter().all(|t| signed_area(&[c[t[0] as usize], c[t[1] as usize], c[t[2] as usize]]) >= 0.0), "{tris:?}");
        assert_edges_covered(&c, &[&c], &tris);
    }

    #[test]
    fn points_far_apart_in_scale_stay_apart() {
        // Subtracting the smallest x, -1e20, would round (0, 1) and (1, 1) to one point.
        let c = vec![[-1e20, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [-1e20, 1.0]];
        let tris = triangulate(&c, &[], &Limits::default()).unwrap();
        assert_edges_covered(&c, &[&c], &tris);
    }

    /// The sign of `(b - a) × (c - a)` for points with integer coordinates up to 1e18, in `i128`.
    fn exact_turn(a: P, b: P, c: P) -> i128 {
        let i = |v: f64| v as i128;
        ((i(b[0]) - i(a[0])) * (i(c[1]) - i(a[1])) - (i(b[1]) - i(a[1])) * (i(c[0]) - i(a[0]))).signum()
    }

    #[test]
    fn a_reflex_corner_next_to_a_distant_point_is_not_clipped() {
        // The first corner, (0, 0), is reflex, but the differences from (-1e18, -1e18) to (0, 0)
        // and to (1, 0) round to one pair, so its cross product rounds to 0.
        let outer = vec![[0.0, 0.0], [1.0, 0.0], [0.0, 10.0], [-1e18, -1e18]];
        assert_eq!(exact_turn(outer[3], outer[0], outer[1]), -1);
        let tris = triangulate(&outer, &[], &Limits::default()).unwrap();
        let at = |i: u32| outer[i as usize];
        assert!(tris.iter().all(|t| exact_turn(at(t[0]), at(t[1]), at(t[2])) >= 0), "{tris:?}");
        assert_edges_covered(&outer, &[&outer], &tris);
    }

    #[test]
    fn a_hole_far_below_its_outer_edges_in_scale_is_inside() {
        // The ray from (0.1, 1e-20) crosses the edge from (0, 0) to (1e20, 2) at x = 0.5; both
        // sides of an interpolated comparison round to -1e20.
        let outer = vec![[0.0, 0.0], [1e20, 2.0], [-10.0, 2.0]];
        let hole = vec![[0.1, 1e-20], [0.2, 1e-20], [0.2, 2e-20], [0.1, 2e-20]];
        assert!(hole.iter().all(|&p| point_in(p, &outer)));
        let tris = triangulate(&outer, &[&hole], &Limits::default()).unwrap();
        let pts: Vec<P> = outer.iter().chain(&hole).copied().collect();
        assert_edges_covered(&pts, &[&outer, &hole], &tris);
    }

    #[test]
    fn small_features_at_large_coordinates_and_large_features_near_the_origin_triangulate() {
        let sq = |x: f64, y: f64, s: f64| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        let check = |outer: &[P], holes: &[&[P]], area: f64| {
            let tris = triangulate(outer, holes, &Limits::default()).unwrap();
            let pts: Vec<P> = std::iter::once(outer).chain(holes.iter().copied()).flatten().copied().collect();
            let at = |i: u32| pts[i as usize];
            assert!(tris.iter().all(|t| signed_area(&[at(t[0]), at(t[1]), at(t[2])]) >= 0.0), "{tris:?}");
            let contours: Vec<&[P]> = std::iter::once(outer).chain(holes.iter().copied()).collect();
            assert_edges_covered(&pts, &contours, &tris);
            assert!((area_of(&pts, &tris) - area).abs() <= area * 1e-9, "{} vs {area}", area_of(&pts, &tris));
        };
        // A unit square near 1e6 with a notch 1e-3 deep and a hole 1e-3 wide.
        let (x, y) = (1e6, 1e6);
        let notched =
            vec![[x, y], [x + 0.5, y], [x + 0.5, y + 1e-3], [x + 0.501, y + 1e-3], [x + 0.501, y], [x + 1.0, y], [x + 1.0, y + 1.0], [x, y + 1.0]];
        let hole = sq(x + 0.25, y + 0.25, 1e-3);
        check(&notched, &[&hole], 1.0 - 1e-6 - 1e-6);
        // A square 2e6 wide around the origin with holes 1e-3 wide near the origin.
        let big = sq(-1e6, -1e6, 2e6);
        let (h0, h1) = (sq(1e-3, 1e-3, 1e-3), sq(-2e-3, -2e-3, 1e-3));
        check(&big, &[&h0, &h1], 4e12 - 2e-6);
    }

    #[test]
    fn signed_area_keeps_a_contour_far_from_the_origin() {
        let sq = |o: f64| vec![[o, o], [o + 1.0, o], [o + 1.0, o + 1.0], [o, o + 1.0]];
        assert_eq!(signed_area(&sq(0.0)), 2.0);
        // Products of coordinates near 1e8 round at a step of 2, which the area would lose.
        assert_eq!(signed_area(&sq(123_456_789.0)), 2.0);
        assert_eq!(signed_area(&[[1.0, 2.0]]), 0.0);
        assert_eq!(signed_area(&[]), 0.0);
        let far = sq(123_456_789.0);
        let tris = triangulate(&far, &[], &Limits::default()).unwrap();
        assert!((area_of(&far, &tris) - 1.0).abs() < 1e-9);
        // Relative to (-1e20, -1e20), (0, 0) and (1, 0) round to one difference; the area is the
        // same wherever the contour starts.
        let mut tri = vec![[-1e20, -1e20], [0.0, 0.0], [1.0, 0.0]];
        for _ in 0..3 {
            assert_eq!(signed_area(&tri), -1e20, "{tri:?}");
            tri.rotate_left(1);
        }
    }

    #[test]
    fn malformed_contours_are_invalid() {
        let l = Limits::default();
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let invalid = |outer: &[P], holes: &[&[P]], why: &str| {
            assert!(matches!(triangulate(outer, holes, &l), Err(MeshOpsError::Invalid(_))), "{why}");
        };
        invalid(&[], &[], "empty outer contour");
        invalid(&[[1.0, 1.0]], &[], "one point");
        invalid(&[[0.0, 0.0], [1.0, 0.0]], &[], "two points");
        invalid(&sq, &[&[]], "empty hole");
        invalid(&[[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]], &[], "bow-tie");
        invalid(&[[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]], &[], "NaN point");
        invalid(&[[0.0, 0.0], [f64::INFINITY, 0.0], [1.0, 1.0]], &[], "infinite point");
        invalid(&[[0.0, 0.0], [1.0, 0.0], [1.0, 0.0], [1.0, 1.0]], &[], "repeated point");
        invalid(&[[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]], &[], "collinear contour without area");
        invalid(&[[0.0, 0.0], [4.0, 0.0], [2.0, 0.0], [2.0, 3.0]], &[], "edge running back over its neighbor");
        invalid(&sq, &[&[[20.0, 0.0], [21.0, 0.0], [21.0, 1.0], [20.0, 1.0]]], "hole outside the outer contour");
        invalid(&sq, &[&[[5.0, 5.0], [15.0, 5.0], [15.0, 6.0], [5.0, 6.0]]], "hole crossing the outer contour");
        invalid(&sq, &[&[[0.0, 5.0], [5.0, 4.0], [5.0, 6.0]]], "hole touching the outer contour");
        invalid(&sq, &[&[[2.0, 2.0], [3.0, 2.0], [2.5, 2.0]]], "hole without area");
        let (a, b) = ([[2.0, 2.0], [6.0, 2.0], [6.0, 6.0], [2.0, 6.0]], [[4.0, 4.0], [8.0, 4.0], [8.0, 8.0], [4.0, 8.0]]);
        invalid(&sq, &[&a, &b], "overlapping holes");
        invalid(&sq, &[&a, &[[3.0, 3.0], [4.0, 3.0], [4.0, 4.0], [3.0, 4.0]]], "hole inside another hole");
        invalid(&[[0.0, 0.0], [1e200, 0.0], [1e200, 1e200], [0.0, 1e200]], &[], "bounding box past 1e150");
        // A unit step at 1e20 is below the spacing of f64 there, so the corners repeat.
        invalid(&[[1e20, 0.0], [1e20 + 1.0, 0.0], [1e20 + 1.0, 1.0], [1e20, 1.0]], &[], "corners that round together");
    }

    #[test]
    fn counts_and_work_are_bounded() {
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let hole = vec![[2.0, 2.0], [2.0, 4.0], [4.0, 4.0], [4.0, 2.0]];
        let l = Limits::default();
        let too_large = |r: Result<Vec<[u32; 3]>>, what: &str| match r {
            Err(MeshOpsError::TooLarge { what: w, .. }) => assert_eq!(w, what),
            other => panic!("{what}: {other:?}"),
        };
        too_large(triangulate(&sq, &[&hole], &Limits { max_outline_vertices: 7, ..l }), "outline vertex count");
        assert!(triangulate(&sq, &[&hole], &Limits { max_outline_vertices: 8, ..l }).is_ok());
        too_large(triangulate(&sq, &[&hole], &Limits { max_outline_contours: 1, ..l }), "outline contour count");
        too_large(triangulate(&sq, &[&hole], &Limits { max_outline_work: 10, ..l }), "outline work");
        // A long zigzag needs work that grows faster than its length.
        let zig: Vec<P> = (0..2_000).map(|i| [i as f64, if i % 2 == 0 { 0.0 } else { 1.0 }]).chain([[1_999.0, -5.0], [0.0, -5.0]]).collect();
        too_large(triangulate(&zig, &[], &Limits { max_outline_work: 1_000_000, ..l }), "outline work");
        let tris = triangulate(&zig, &[], &l).unwrap();
        assert_eq!(tris.len(), zig.len() - 2);
    }

    #[test]
    fn helpers_handle_short_input() {
        assert!(!point_in([0.0, 0.0], &[]));
        assert!(!point_in([0.0, 0.0], &[[0.0, 0.0], [1.0, 1.0]]));
        let mut work = Work::new(&Limits::default());
        let mut ring = vec![0, 1, 2];
        assert!(matches!(bridge(&[[0.0, 0.0], [1.0, 0.0], [0.0, 1.0]], &mut ring, &[], &mut work), Err(MeshOpsError::Invalid(_))));
        assert!(ear_clip(&[], vec![], &mut work).unwrap().is_empty());
    }
}
