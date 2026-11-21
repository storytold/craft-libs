//! Closed 2D contours grouped into outlines, and extruded, beveled meshes of outlines.
//!
//! Input contours are closed polylines (last point not repeated) in a plane with x right and y
//! down, as in a 2D layer. The mesh is in the same space: the front cap lies on the plane
//! (z = 0) and faces -z, the back cap lies at z = depth and faces +z. Each contour gets a ring of
//! vertices per profile step, inset from the contour by the bevel at a depth along the profile;
//! consecutive rings are joined by quads, and the caps are triangulated from the innermost front
//! and back rings with [`crate::triangulate::triangulate`]. Caps and walls share positions
//! exactly, so the mesh is closed when its vertices are welded by position. Positions are `f32`
//! in the outline's own coordinates.

use crate::triangulate::{
    MAX_OUTLINE_EXTENT, Work, checked_polygon, cross, on_edge, point_in, segments_cross, signed_area, triangulate_counted, try_collect,
};
use crate::{Limits, MeshOpsError, Result, check_limit, try_filled, try_vec};

type P = [f64; 2];

/// The profile of the bevel between a cap and the walls.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub enum BevelStyle {
    /// No bevel: the walls meet the caps at a right angle.
    #[default]
    None,
    /// A straight slope, in one step.
    Angular,
    /// A quarter circle scooped in, in six steps.
    Concave,
    /// A quarter circle bulging out towards the outline's corner, in six steps.
    Convex,
}

impl BevelStyle {
    pub const ALL: [BevelStyle; 4] = [BevelStyle::None, BevelStyle::Angular, BevelStyle::Concave, BevelStyle::Convex];

    /// (inset fraction 1 → 0, depth fraction 0 → 1) along the profile at `t` in 0..1.
    fn profile(self, t: f64) -> (f64, f64) {
        let th = t * std::f64::consts::FRAC_PI_2;
        match self {
            BevelStyle::None | BevelStyle::Angular => (1.0 - t, t),
            BevelStyle::Convex => (1.0 - th.sin(), 1.0 - th.cos()),
            BevelStyle::Concave => (th.cos(), th.sin()),
        }
    }

    fn steps(self) -> usize {
        match self {
            BevelStyle::None => 0,
            BevelStyle::Angular => 1,
            _ => 6,
        }
    }
}

/// How [`extrude`] shapes the outlines.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ExtrudeParams {
    pub bevel: BevelStyle,
    /// Inset and depth of the bevel, in the outline's units: finite and 0 or more. A bevel deeper
    /// than half the extrusion is built at half the extrusion.
    pub bevel_depth: f64,
    /// Bevels of holes as a fraction of `bevel_depth`, from 0 to 1.
    pub hole_bevel: f64,
    /// Extrusion depth, in the outline's units: finite and greater than 0.
    pub depth: f64,
}

impl Default for ExtrudeParams {
    fn default() -> Self {
        ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 2.0, hole_bevel: 1.0, depth: 50.0 }
    }
}

impl ExtrudeParams {
    fn check(&self) -> Result<()> {
        let bad = |what: &str, v: f64, rule: &str| Err(MeshOpsError::Invalid(format!("extrusion {what} {v} is not {rule}")));
        if !(self.depth.is_finite() && self.depth > 0.0) {
            return bad("depth", self.depth, "a finite positive number");
        }
        if !(self.bevel_depth.is_finite() && self.bevel_depth >= 0.0) {
            return bad("bevel depth", self.bevel_depth, "finite and 0 or more");
        }
        if !(0.0..=1.0).contains(&self.hole_bevel) {
            return bad("hole bevel", self.hole_bevel, "from 0 to 1");
        }
        Ok(())
    }
}

/// An outer contour with its holes.
#[derive(Clone, Debug, PartialEq)]
pub struct Outline {
    pub outer: Vec<P>,
    pub holes: Vec<Vec<P>>,
}

/// Groups closed contours into outlines by nesting depth: a contour inside an even number of
/// larger contours is an outer contour, and one inside an odd number is a hole of the innermost
/// containing outer contour. A larger contour counts as a container only when the whole contour
/// lies inside it or on its edges; overlapping contours that do not nest each count as a
/// container of what lies wholly inside them. Repeated consecutive points and a last point equal to the first are
/// removed, and contours left with fewer than three points or an area of 1e-9 or less are dropped.
///
/// A contour that touches a larger contour, at a point or along an edge, still counts as inside it,
/// so a hole can touch its outer contour. [`extrude`] and [`crate::triangulate::triangulate`]
/// reject contours that touch: such a hole has to be moved off its outer contour, or merged into
/// it, before the outline is extruded.
///
/// Errors: `Invalid` for a non-finite point and for contours that together span more than 1e150
/// in x or y; `TooLarge` past [`Limits::max_outline_vertices`]
/// (the points of every contour), past [`Limits::max_outline_contours`] (the contours left after
/// dropping) or past [`Limits::max_outline_work`]. [`group_largest_contours`] keeps the largest
/// contours instead of returning an error for too many.
pub fn group_contours<C: AsRef<[P]>>(contours: &[C], limits: &Limits) -> Result<Vec<Outline>> {
    group(contours, limits, false)
}

/// [`group_contours`], keeping the [`Limits::max_outline_contours`] largest contours (by area) in
/// their order when more are left after dropping. A hole that is kept always keeps its larger
/// container.
pub fn group_largest_contours<C: AsRef<[P]>>(contours: &[C], limits: &Limits) -> Result<Vec<Outline>> {
    group(contours, limits, true)
}

/// The contours worth grouping: finite, without repeated points, with three points or more and
/// area above 1e-9.
fn cleaned<C: AsRef<[P]>>(contours: &[C], limits: &Limits) -> Result<Vec<Vec<P>>> {
    let total = contours.iter().try_fold(0usize, |n, c| n.checked_add(c.as_ref().len())).unwrap_or(usize::MAX);
    check_limit("outline vertex count", total, limits.max_outline_vertices)?;
    let mut cs = try_vec(contours.len(), "contours")?;
    for c in contours {
        let c = c.as_ref();
        if !c.iter().flatten().all(|v| v.is_finite()) {
            return Err(MeshOpsError::Invalid("a contour point is not finite".into()));
        }
        let mut c = try_collect(c.iter().copied(), "contour")?;
        c.dedup();
        while c.len() > 1 && c.first() == c.last() {
            c.pop();
        }
        if c.len() < 3 {
            continue;
        }
        // The extent of each contour is checked before its area, whose products overflow to NaN
        // for a contour wider than the limit.
        let (lo, hi) = bounds_of(&c);
        let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
        if extent > MAX_OUTLINE_EXTENT {
            return Err(MeshOpsError::Invalid(format!("the contours span {extent:e}, over {MAX_OUTLINE_EXTENT:e}")));
        }
        if signed_area(&c).abs() > 1e-9 {
            cs.push(c);
        }
    }
    let (lo, hi) = bounds_of(cs.iter().flatten());
    let extent = (hi[0] - lo[0]).max(hi[1] - lo[1]);
    if extent > MAX_OUTLINE_EXTENT {
        return Err(MeshOpsError::Invalid(format!("the contours span {extent:e}, over {MAX_OUTLINE_EXTENT:e}")));
    }
    Ok(cs)
}

/// The lowest and highest x and y of `points` (infinite for none).
fn bounds_of<'a>(points: impl IntoIterator<Item = &'a P>) -> (P, P) {
    points
        .into_iter()
        .fold(([f64::INFINITY; 2], [f64::NEG_INFINITY; 2]), |(lo, hi), p| ([lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]))
}

/// The `keep` largest of `cs`, in their order.
fn largest(cs: Vec<Vec<P>>, keep: usize) -> Result<Vec<Vec<P>>> {
    let area = try_collect(cs.iter().map(|c| signed_area(c).abs()), "contour areas")?;
    let mut order = try_collect(0..cs.len(), "contour order")?;
    // Largest first, equal areas in input order; an unstable sort allocates nothing.
    order.sort_unstable_by(|&a, &b| area[b].total_cmp(&area[a]).then(a.cmp(&b)));
    order.truncate(keep);
    order.sort_unstable();
    let mut cs = try_collect(cs.into_iter().map(Some), "contours")?;
    let mut kept = try_vec(order.len(), "contours")?;
    kept.extend(order.into_iter().filter_map(|i| cs[i].take()));
    Ok(kept)
}

/// Contours with their areas and bounding boxes, for containment tests.
struct Nest {
    cs: Vec<Vec<P>>,
    area: Vec<f64>,
    bounds: Vec<[P; 2]>,
}

impl Nest {
    /// The larger contours that hold all of contour `i` and that `only` accepts: every point of
    /// `i` lies inside or on the larger contour, no edge of `i` crosses one of its edges, and
    /// each edge of `i`, cut at the larger contour's points that lie on it, has every piece inside
    /// the larger contour or along one of its edges. None of these tests depends on where `i`
    /// starts. The larger contour's bounding box has to hold `i`'s bounding box first.
    fn containers(&self, i: usize, only: impl Fn(usize) -> bool, work: &mut Work) -> Result<Vec<usize>> {
        let c = &self.cs[i];
        let [ilo, ihi] = self.bounds[i];
        work.add(self.cs.len() as u64)?;
        let mut out = Vec::new();
        let mut cuts = Vec::new();
        for j in 0..self.cs.len() {
            let [lo, hi] = self.bounds[j];
            if j == i || self.area[j] <= self.area[i] || !only(j) || ilo[0] < lo[0] || ilo[1] < lo[1] || ihi[0] > hi[0] || ihi[1] > hi[1] {
                continue;
            }
            let d = &self.cs[j];
            // Each point of `i` against every edge of `d`, each edge of `i` against every edge
            // and every point of `d`; `holds` adds the pieces it tests.
            work.add((c.len() as u64).saturating_mul(d.len() as u64).saturating_mul(3))?;
            if holds(d, c, &mut cuts, work)? {
                out.try_reserve(1).map_err(|_| MeshOpsError::OutOfMemory("contour containers"))?;
                out.push(j);
            }
        }
        Ok(out)
    }
}

/// Whether `p` lies inside the closed contour `c` or on one of its edges.
fn inside_or_on(p: P, c: &[P]) -> bool {
    let n = c.len();
    point_in(p, c) || (0..n).any(|k| on_edge(c[k], c[(k + 1) % n], p))
}

/// Whether the points of segment `p`-`q` just past `p` lie inside the closed contour `c`, by the
/// rule of [`point_in`] applied to `p + t (q - p)` for every `t` above 0 small enough. No point is
/// rounded: a vertex at `p`'s height counts as above when `q` is lower, and an edge whose line
/// passes through `p` takes the side of `q`. An edge whose line holds both `p` and `q` is not
/// counted.
fn leaves_inside(p: P, q: P, c: &[P]) -> bool {
    let n = c.len();
    let down = q[1] < p[1];
    let above = |v: P| if v[1] != p[1] { v[1] > p[1] } else { down };
    let mut inside = false;
    let mut j = n.wrapping_sub(1);
    for i in 0..n {
        let (a, b) = (c[i], c[j]);
        if above(a) != above(b) {
            let (lo, hi) = if a[1] < b[1] { (a, b) } else { (b, a) };
            let s = cross(lo, hi, p);
            if (if s == 0.0 { cross(lo, hi, q) } else { s }) > 0.0 {
                inside = !inside;
            }
        }
        j = i;
    }
    inside
}

/// Whether contour `outer` holds all of contour `inner`, as [`Nest::containers`] tests it. An edge
/// of `inner` may meet `outer` at its vertices and run along its edges. Between two such contacts
/// the edge lies wholly inside `outer`, wholly outside it or along one of its edges. A piece whose
/// ends lie on one edge of `outer` runs along that edge; any other piece is tested just past its
/// first end ([`leaves_inside`]) without rounding a point onto it. `cuts` is scratch space for the
/// contacts along one edge.
fn holds(outer: &[P], inner: &[P], cuts: &mut Vec<(f64, P)>, work: &mut Work) -> Result<bool> {
    let (n, m) = (inner.len(), outer.len());
    let on_one_edge = |p: P, q: P| (0..m).any(|e| on_edge(outer[e], outer[(e + 1) % m], p) && on_edge(outer[e], outer[(e + 1) % m], q));
    for k in 0..n {
        let (a, b) = (inner[k], inner[(k + 1) % n]);
        if !inside_or_on(a, outer) || (0..m).any(|e| segments_cross(a, b, outer[e], outer[(e + 1) % m])) {
            return Ok(false);
        }
        // The points of `outer` on a-b, by where they lie along it, between a and b.
        let d = [b[0] - a[0], b[1] - a[1]];
        let along = |p: P| (p[0] - a[0]) * d[0] + (p[1] - a[1]) * d[1];
        cuts.clear();
        cuts.try_reserve(m + 2).map_err(|_| MeshOpsError::OutOfMemory("contour containment"))?;
        cuts.push((f64::NEG_INFINITY, a));
        cuts.extend(outer.iter().filter(|&&v| v != a && v != b && on_edge(a, b, v)).map(|&v| (along(v), v)));
        cuts.push((f64::INFINITY, b));
        cuts.sort_unstable_by(|x, y| x.0.total_cmp(&y.0));
        work.add((cuts.len() as u64).saturating_mul(m as u64))?;
        for w in cuts.windows(2) {
            let (p, q) = (w[0].1, w[1].1);
            if p == q {
                continue;
            }
            if !on_one_edge(p, q) && !leaves_inside(p, q, outer) {
                return Ok(false);
            }
        }
    }
    Ok(true)
}

fn group<C: AsRef<[P]>>(contours: &[C], limits: &Limits, keep_largest: bool) -> Result<Vec<Outline>> {
    let mut cs = cleaned(contours, limits)?;
    if cs.len() > limits.max_outline_contours {
        if !keep_largest {
            check_limit("outline contour count", cs.len(), limits.max_outline_contours)?;
        }
        cs = largest(cs, limits.max_outline_contours)?;
    }
    let mut work = Work::new(limits);
    let n = cs.len();
    let area = try_collect(cs.iter().map(|c| signed_area(c).abs()), "contour areas")?;
    let bounds = try_collect(
        cs.iter().map(|c| {
            c.iter().fold([[f64::MAX; 2], [f64::MIN; 2]], |[lo, hi], p| [[lo[0].min(p[0]), lo[1].min(p[1])], [hi[0].max(p[0]), hi[1].max(p[1])]])
        }),
        "contour bounds",
    )?;
    let nest = Nest { cs, area, bounds };
    // Depth is the count of containers, so overlapping (non-nested) paths keep every container
    // they sit inside.
    let mut depth = try_vec(n, "contour depths")?;
    for i in 0..n {
        depth.push(nest.containers(i, |_| true, &mut work)?.len());
    }
    let even = |i: usize| depth[i] % 2 == 0;
    // A hole belongs to the innermost containing contour that is itself an outer contour: with
    // overlapping paths the smallest container can be a hole, which has no outline to take it.
    let mut parent = try_vec(n, "contour parents")?;
    for i in 0..n {
        parent.push(if even(i) {
            None
        } else {
            nest.containers(i, even, &mut work)?.into_iter().min_by(|&a, &b| nest.area[a].total_cmp(&nest.area[b]))
        });
    }
    let mut outlines: Vec<Outline> = try_vec(n, "outlines")?;
    let mut slot = try_filled(n, usize::MAX, "outline slots")?;
    let mut order = try_collect(0..n, "contour order")?;
    // By depth, equal depths in input order; an unstable sort allocates nothing.
    order.sort_unstable_by_key(|&i| (depth[i], i));
    let mut cs = try_collect(nest.cs.into_iter().map(Some), "contours")?;
    // Every outline gets its slot before any hole is attached.
    for &i in order.iter().filter(|&&i| even(i)) {
        slot[i] = outlines.len();
        outlines.push(Outline { outer: cs[i].take().unwrap_or_default(), holes: vec![] });
    }
    for &i in order.iter().filter(|&&i| !even(i)) {
        if let (Some(o), Some(h)) = (parent[i].and_then(|p| outlines.get_mut(slot[p])), cs[i].take()) {
            o.holes.try_reserve(1).map_err(|_| MeshOpsError::OutOfMemory("outline holes"))?;
            o.holes.push(h);
        }
    }
    Ok(outlines)
}

struct Contour {
    pts: Vec<P>,
    /// Outward unit normal of edge i (pts[i] → pts[i+1]).
    edge_n: Vec<P>,
    /// Miter offset direction per vertex (outward, length ≥ 1).
    miter: Vec<P>,
    /// Smooth-shaded vertex (shallow corner).
    smooth: Vec<bool>,
    bevel: f64,
}

fn contour(src: &[P], outer: bool, bevel: f64) -> Result<Contour> {
    let mut pts = try_collect(src.iter().copied(), "contour points")?;
    // Outer counterclockwise (in x/y), holes clockwise: the outward normal is (dy, -dx).
    let ccw = signed_area(&pts) > 0.0;
    if ccw != outer {
        pts.reverse();
    }
    let n = pts.len();
    let edge_n = try_collect(
        (0..n).map(|i| {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let (dx, dy) = (b[0] - a[0], b[1] - a[1]);
            let l = dx.hypot(dy).max(f64::MIN_POSITIVE);
            [dy / l, -dx / l]
        }),
        "contour normals",
    )?;
    let mut miter = try_filled(n, [0.0; 2], "contour miters")?;
    let mut smooth = try_filled(n, false, "contour shading")?;
    for i in 0..n {
        let (n0, n1) = (edge_n[(i + n - 1) % n], edge_n[i]);
        let d = n0[0] * n1[0] + n0[1] * n1[1];
        let s = [n0[0] + n1[0], n0[1] + n1[1]];
        let k = 1.0 + d;
        miter[i] = if k < 0.25 {
            // Very sharp corner: limit the miter (offset along the bisector, length 2.8).
            let l = s[0].hypot(s[1]).max(f64::MIN_POSITIVE);
            [s[0] / l * 2.8, s[1] / l * 2.8]
        } else {
            [s[0] / k, s[1] / k]
        };
        smooth[i] = d > 0.82; // < ~35° turn
    }
    Ok(Contour { pts, edge_n, miter, smooth, bevel })
}

/// Rings of a contour: (inset, z) from the front cap to the back cap.
fn rings(style: BevelStyle, bevel: f64, depth: f64) -> Vec<(f64, f64)> {
    let b = bevel.clamp(0.0, depth * 0.5);
    let steps = if b > 0.0 { style.steps() } else { 0 };
    if steps == 0 {
        return vec![(0.0, 0.0), (0.0, depth)];
    }
    let front: Vec<(f64, f64)> = (0..=steps)
        .map(|k| {
            let (f, g) = style.profile(k as f64 / steps as f64);
            (b * f, b * g)
        })
        .collect();
    let mut out = front.clone();
    let back: Vec<(f64, f64)> = front.iter().rev().map(|&(i, z)| (i, depth - z)).collect();
    // Skip a duplicate ring where the walls have zero height (bevel = depth / 2).
    let start = if (back[0].1 - out[out.len() - 1].1).abs() <= 1e-12 * depth { 1 } else { 0 };
    out.extend_from_slice(&back[start..]);
    out
}

/// Whether the profile segment from ring `k` to ring `k + 1` is a straight wall (no inset change).
fn is_wall(rs: &[(f64, f64)], k: usize) -> bool {
    rs[k].0 == rs[k + 1].0
}

/// A surface of an extruded mesh: the parts an app colors and shades separately.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum OutlineSurface {
    /// The front cap (on the plane, facing -z).
    Front,
    /// The beveled rings between the caps and the walls (front and back).
    Bevel,
    /// The straight walls.
    Side,
    /// The back cap.
    Back,
}

impl OutlineSurface {
    pub const ALL: [OutlineSurface; 4] = [OutlineSurface::Front, OutlineSurface::Bevel, OutlineSurface::Side, OutlineSurface::Back];

    /// Index into [`OutlineSurface::ALL`] (and into the meshes of [`extrude_surfaces`]).
    pub fn index(self) -> usize {
        self as usize
    }
}

/// The triangles of an extruded outline. Normals are unit vectors; every UV is `(0, 0)`.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct OutlineMesh {
    pub positions: Vec<[f32; 3]>,
    pub normals: Vec<[f32; 3]>,
    pub uvs: Vec<[f32; 2]>,
    pub indices: Vec<u32>,
}

impl OutlineMesh {
    pub fn triangle_count(&self) -> usize {
        self.indices.len() / 3
    }

    /// Reserves room for `vertices` and `triangles` more.
    fn reserve(&mut self, vertices: usize, triangles: usize) -> Result<()> {
        let ok = self.positions.try_reserve(vertices).is_ok()
            && self.normals.try_reserve(vertices).is_ok()
            && self.uvs.try_reserve(vertices).is_ok()
            && triangles.checked_mul(3).is_some_and(|i| self.indices.try_reserve(i).is_ok());
        if ok { Ok(()) } else { Err(MeshOpsError::OutOfMemory("extruded mesh")) }
    }

    /// Adds a vertex; a position outside the `f32` range is an error.
    fn push(&mut self, pos: [f64; 3], n: [f64; 3]) -> Result<u32> {
        let p = pos.map(|v| v as f32);
        if !p.iter().all(|v| v.is_finite()) {
            return Err(MeshOpsError::Invalid(format!("the extruded point {pos:?} is outside the f32 range")));
        }
        let l = n[0].hypot(n[1]).hypot(n[2]).max(f64::MIN_POSITIVE);
        self.positions.push(p);
        self.normals.push(n.map(|v| (v / l) as f32));
        self.uvs.push([0.0, 0.0]);
        // The vertex count was checked against `u32` before any vertex was added.
        Ok((self.positions.len() - 1) as u32)
    }
}

/// Builds the extruded mesh of `outlines`, as the module documentation describes.
///
/// Errors: `Invalid` for a parameter outside the range [`ExtrudeParams`] gives, an outline that
/// [`crate::triangulate::triangulate`] rejects, a bevel so wide that it turns a cap's contour
/// inside out or makes it cross itself or a hole, a point outside the `f32` range, neighboring
/// points that round to one `f32` position, contours that meet once rounded to `f32`, a depth
/// that rounds to 0 in `f32`, and a bevel step that rounds to no width in `f32`;
/// `TooLarge` past [`Limits::max_outline_vertices`] (the points of every outline),
/// [`Limits::max_outline_contours`] (in one outline), [`Limits::max_outline_work`] (for the whole
/// call), or [`Limits::max_vertices`] or [`Limits::max_triangles`] (the output, counted before
/// it is built).
pub fn extrude(outlines: &[Outline], p: &ExtrudeParams, limits: &Limits) -> Result<OutlineMesh> {
    Ok(build(outlines, p, limits, false, 0.0)?.into_iter().next().unwrap_or_default())
}

/// [`extrude`] split by surface: one mesh per [`OutlineSurface`], in [`OutlineSurface::ALL`]
/// order. Together they hold the same triangles as [`extrude`].
pub fn extrude_surfaces(outlines: &[Outline], p: &ExtrudeParams, limits: &Limits) -> Result<[OutlineMesh; 4]> {
    let mut v = build(outlines, p, limits, true, 0.0)?.into_iter();
    Ok(std::array::from_fn(|_| v.next().unwrap_or_default()))
}

/// Gap between stacked extrusions of one layer, in the outline's units: paint drawn later (a
/// stroke over its fill, an upper shape group) sits this much nearer the viewer per level, so
/// coplanar front caps do not fight in the depth buffer.
pub const STACK_GAP: f64 = 0.02;

/// [`extrude`] at stacking level `level` (0 = the bottom paint): the mesh moves
/// `level × STACK_GAP` towards -z. The largest level, `usize::MAX`, gives an offset near 3.7e17,
/// inside the `f32` range. The offset is added before the conversion to `f32`, and caps that it
/// rounds to one `f32` depth are an `Invalid` error.
pub fn extrude_stacked(outlines: &[Outline], p: &ExtrudeParams, level: usize, limits: &Limits) -> Result<OutlineMesh> {
    Ok(build(outlines, p, limits, false, level as f64 * STACK_GAP)?.into_iter().next().unwrap_or_default())
}

/// [`extrude_surfaces`] at stacking level `level` (see [`extrude_stacked`]).
pub fn extrude_surfaces_stacked(outlines: &[Outline], p: &ExtrudeParams, level: usize, limits: &Limits) -> Result<[OutlineMesh; 4]> {
    let mut v = build(outlines, p, limits, true, level as f64 * STACK_GAP)?.into_iter();
    Ok(std::array::from_fn(|_| v.next().unwrap_or_default()))
}

/// One outline's contours (outer first) with their rings.
struct Plan {
    cs: Vec<Contour>,
    rings: Vec<Vec<(f64, f64)>>,
}

impl Plan {
    /// Vertices and triangles per [`OutlineSurface`].
    fn counts(&self) -> Option<[(usize, usize); 4]> {
        let mut out = [(0usize, 0usize); 4];
        let add = |slot: &mut (usize, usize), v: usize, t: usize| -> Option<()> {
            *slot = (slot.0.checked_add(v)?, slot.1.checked_add(t)?);
            Some(())
        };
        for (c, rs) in self.cs.iter().zip(&self.rings) {
            let n = c.pts.len();
            for k in 0..rs.len() - 1 {
                let s = if is_wall(rs, k) { OutlineSurface::Side } else { OutlineSurface::Bevel };
                add(&mut out[s.index()], n.checked_mul(4)?, n.checked_mul(2)?)?;
            }
        }
        // The caps' ring joins each hole through two extra points; ear clipping gives two
        // triangles fewer than its points.
        let points = self.cs.iter().try_fold(0usize, |s, c| s.checked_add(c.pts.len()))?;
        let tris = points.checked_add(2 * (self.cs.len() - 1))? - 2;
        add(&mut out[OutlineSurface::Front.index()], points, tris)?;
        add(&mut out[OutlineSurface::Back.index()], points, tris)?;
        Some(out)
    }
}

/// Checks `o` and works out its contours and rings.
fn plan(o: &Outline, p: &ExtrudeParams, limits: &Limits, work: &mut Work) -> Result<Plan> {
    let mut holes = try_vec(o.holes.len(), "outline holes")?;
    holes.extend(o.holes.iter().map(Vec::as_slice));
    checked_polygon(&o.outer, &holes, limits, work)?;
    let mut cs = try_vec(o.holes.len() + 1, "outline contours")?;
    cs.push(contour(&o.outer, true, p.bevel_depth)?);
    for h in &o.holes {
        cs.push(contour(h, false, p.bevel_depth * p.hole_bevel)?);
    }
    let rings = try_collect(cs.iter().map(|c| rings(p.bevel, c.bevel, p.depth)), "contour rings")?;
    Ok(Plan { cs, rings })
}

/// The extruded mesh in one piece (`split` false) or one per [`OutlineSurface`], moved `lift`
/// towards -z.
fn build(outlines: &[Outline], p: &ExtrudeParams, limits: &Limits, split: bool, lift: f64) -> Result<Vec<OutlineMesh>> {
    p.check()?;
    let total =
        outlines.iter().flat_map(|o| std::iter::once(&o.outer).chain(&o.holes)).try_fold(0usize, |n, c| n.checked_add(c.len())).unwrap_or(usize::MAX);
    check_limit("outline vertex count", total, limits.max_outline_vertices)?;
    let mut work = Work::new(limits);
    let mut plans = try_vec(outlines.len(), "outlines")?;
    for o in outlines {
        plans.push(plan(o, p, limits, &mut work)?);
    }
    let slot = |s: OutlineSurface| if split { s.index() } else { 0 };
    let mut counts = [(0usize, 0usize); 4];
    for plan in &plans {
        let c = plan.counts().ok_or(MeshOpsError::TooLarge { what: "vertex count", got: u64::MAX, limit: limits.max_vertices as u64 })?;
        for s in OutlineSurface::ALL {
            let (v, t) = (&mut counts[slot(s)], c[s.index()]);
            *v = (v.0.saturating_add(t.0), v.1.saturating_add(t.1));
        }
    }
    let vertices = counts.iter().fold(0usize, |s, c| s.saturating_add(c.0));
    check_limit("vertex count", vertices, limits.max_vertices.min(u32::MAX as usize))?;
    check_limit("triangle count", counts.iter().fold(0usize, |s, c| s.saturating_add(c.1)), limits.max_triangles)?;
    let mut out = try_vec(if split { 4 } else { 1 }, "extruded meshes")?;
    for &(v, t) in counts.iter().take(if split { 4 } else { 1 }) {
        let mut m = OutlineMesh::default();
        m.reserve(v, t)?;
        out.push(m);
    }
    for plan in &plans {
        emit(&mut out, split, plan, p, limits, &mut work, lift)?;
    }
    Ok(out)
}

/// Adds one outline's walls, bevels and caps to `out`, moved `lift` towards -z.
fn emit(out: &mut [OutlineMesh], split: bool, plan: &Plan, p: &ExtrudeParams, limits: &Limits, work: &mut Work, lift: f64) -> Result<()> {
    let slot = |s: OutlineSurface| if split { s.index() } else { 0 };
    // Positions per contour per ring.
    let mut ring_pos: Vec<Vec<Vec<[f64; 3]>>> = try_vec(plan.cs.len(), "rings")?;
    for (c, rs) in plan.cs.iter().zip(&plan.rings) {
        let mut rings = try_vec(rs.len(), "rings")?;
        for &(inset, z) in rs {
            rings.push(try_collect(c.pts.iter().zip(&c.miter).map(|(q, m)| [q[0] - m[0] * inset, q[1] - m[1] * inset, z - lift]), "ring")?);
        }
        ring_pos.push(rings);
    }
    let beveled = plan.rings.iter().any(|rs| rs[0].0 > 0.0);
    check_caps(&plan.cs, &ring_pos, beveled, limits, work).map_err(|e| match e {
        MeshOpsError::Invalid(why) if beveled => MeshOpsError::Invalid(format!("the bevel is too wide for the outline: {why}")),
        e => e,
    })?;
    check_f32_edges(&plan.rings, &ring_pos)?;
    let tris = cap_triangles(&ring_pos, limits, work)?;
    check_f32_walls(&plan.rings, &ring_pos, limits, work)?;
    let curved = matches!(p.bevel, BevelStyle::Concave | BevelStyle::Convex);
    for ((c, rs), pos) in plan.cs.iter().zip(&plan.rings).zip(&ring_pos) {
        for k in 0..rs.len() - 1 {
            let s = if is_wall(rs, k) { OutlineSurface::Side } else { OutlineSurface::Bevel };
            if let Some(m) = out.get_mut(slot(s)) {
                band(m, c, rs, pos, k, curved)?;
            }
        }
    }
    for (front, nz) in [(true, -1.0), (false, 1.0)] {
        let Some(m) = out.get_mut(slot(if front { OutlineSurface::Front } else { OutlineSurface::Back })) else { continue };
        let base = m.positions.len() as u32;
        for pos in &ring_pos {
            let ring = if front { pos.first() } else { pos.last() };
            for &q in ring.into_iter().flatten() {
                m.push(q, [0.0, 0.0, nz])?;
            }
        }
        for t in &tris {
            let (a, b, c) = (base + t[0], base + t[1], base + t[2]);
            m.indices.extend(if front { [a, c, b] } else { [a, b, c] });
        }
    }
    Ok(())
}

/// Rejects rings whose neighboring points round to one `f32` position, where the mesh would lose
/// an edge of the outline, caps whose planes round to one `f32` depth, two consecutive rings
/// whose points round to the same `f32` positions, and bevel steps (`rings` changes the inset)
/// in which a point does not move in x or y once rounded to `f32`, whether or not it moved
/// before rounding. A wall step keeps its x and y.
fn check_f32_edges(rings: &[Vec<(f64, f64)>], ring_pos: &[Vec<Vec<[f64; 3]>>]) -> Result<()> {
    let f = |v: f64| v as f32;
    for (rs, pos) in rings.iter().zip(ring_pos) {
        let z = |r: Option<&Vec<[f64; 3]>>| r.and_then(|r| r.first()).map(|q| f(q[2]));
        if z(pos.first()) == z(pos.last()) {
            return Err(MeshOpsError::Invalid("the extrusion depth rounds to 0 in f32".into()));
        }
        for (k, pair) in pos.windows(2).enumerate() {
            let flat = |a: &[f64; 3], b: &[f64; 3]| f(a[0]) == f(b[0]) && f(a[1]) == f(b[1]);
            if pair[0].iter().zip(&pair[1]).all(|(a, b)| flat(a, b) && f(a[2]) == f(b[2])) {
                return Err(MeshOpsError::Invalid("two rings of the profile round to one f32 position".into()));
            }
            if k + 1 < rs.len() && !is_wall(rs, k) && pair[0].iter().zip(&pair[1]).any(|(a, b)| flat(a, b)) {
                return Err(MeshOpsError::Invalid("the bevel rounds to no width in f32".into()));
            }
        }
    }
    for ring in ring_pos.iter().flatten() {
        let n = ring.len();
        if (0..n).any(|i| {
            let (a, b) = (ring[i], ring[(i + 1) % n]);
            a[0] as f32 == b[0] as f32 && a[1] as f32 == b[1] as f32
        }) {
            return Err(MeshOpsError::Invalid("two neighboring outline points round to one f32 position".into()));
        }
    }
    Ok(())
}

/// The x and y of ring `k` of every contour (the last ring for a contour with fewer), rounded to
/// `f32` when `round` is set.
fn ring_contours(ring_pos: &[Vec<Vec<[f64; 3]>>], k: impl Fn(usize) -> usize, round: bool) -> Result<Vec<Vec<P>>> {
    let r = |v: f64| if round { f64::from(v as f32) } else { v };
    let mut contours: Vec<Vec<P>> = try_vec(ring_pos.len(), "contour rings")?;
    for (c, pos) in ring_pos.iter().enumerate() {
        let ring = pos.get(k(c)).or(pos.last()).map_or(&[][..], Vec::as_slice);
        contours.push(try_collect(ring.iter().map(|q| [r(q[0]), r(q[1])]), "contour ring")?);
    }
    Ok(contours)
}

/// An `Invalid` error from checking or triangulating the rounded contours, worded to say that
/// they meet once rounded to `f32`.
fn rounded_error(e: MeshOpsError) -> MeshOpsError {
    match e {
        MeshOpsError::Invalid(why) => MeshOpsError::Invalid(format!("the outline's contours meet once rounded to f32: {why}")),
        e => e,
    }
}

/// Rejects an outline whose wall contours (the ring with the least inset) cross or touch once
/// rounded to `f32`. When that ring is the front ring of every contour, [`cap_triangles`] has
/// checked it.
fn check_f32_walls(rings: &[Vec<(f64, f64)>], ring_pos: &[Vec<Vec<[f64; 3]>>], limits: &Limits, work: &mut Work) -> Result<()> {
    let wall = |c: usize| rings.get(c).and_then(|rs| rs.iter().enumerate().min_by(|a, b| a.1.0.total_cmp(&b.1.0))).map_or(0, |(k, _)| k);
    if (0..rings.len()).all(|c| wall(c) == 0) {
        return Ok(());
    }
    let contours = ring_contours(ring_pos, wall, true)?;
    let Some((outer, holes)) = contours.split_first() else { return Ok(()) };
    let mut refs = try_vec(holes.len(), "rounded holes")?;
    refs.extend(holes.iter().map(Vec::as_slice));
    checked_polygon(outer, &refs, limits, work).map_err(rounded_error)?;
    Ok(())
}

/// The triangles of the caps, from the front ring of every contour rounded to `f32`, the
/// positions the mesh stores, so every cap triangle faces the way its cap does once stored.
fn cap_triangles(ring_pos: &[Vec<Vec<[f64; 3]>>], limits: &Limits, work: &mut Work) -> Result<Vec<[u32; 3]>> {
    let contours = ring_contours(ring_pos, |_| 0, true)?;
    let Some((outer, holes)) = contours.split_first() else { return Ok(Vec::new()) };
    let mut refs = try_vec(holes.len(), "cap holes")?;
    refs.extend(holes.iter().map(Vec::as_slice));
    triangulate_counted(outer, &refs, limits, work).map_err(rounded_error)
}

/// Checks the caps at the outline's own precision: an inset edge that is reversed or has no
/// length (the bevel turns the contour inside out or collapses it) is an error, and with a bevel
/// the inset contours must not cross or touch.
fn check_caps(cs: &[Contour], ring_pos: &[Vec<Vec<[f64; 3]>>], beveled: bool, limits: &Limits, work: &mut Work) -> Result<()> {
    let contours = ring_contours(ring_pos, |_| 0, false)?;
    for (c, ring) in cs.iter().zip(&contours) {
        let n = c.pts.len();
        let turned = (0..n).any(|i| {
            let (a, b, p, q) = (c.pts[i], c.pts[(i + 1) % n], ring[i], ring[(i + 1) % n]);
            (b[0] - a[0]) * (q[0] - p[0]) + (b[1] - a[1]) * (q[1] - p[1]) <= 0.0
        });
        if turned {
            return Err(MeshOpsError::Invalid("a cap contour turns inside out".into()));
        }
    }
    let Some((outer, holes)) = contours.split_first() else { return Ok(()) };
    if signed_area(outer) <= 0.0 || holes.iter().any(|h| signed_area(h) >= 0.0) {
        return Err(MeshOpsError::Invalid("a cap contour turns inside out".into()));
    }
    if beveled {
        let mut refs = try_vec(holes.len(), "cap holes")?;
        refs.extend(holes.iter().map(Vec::as_slice));
        checked_polygon(outer, &refs, limits, work)?;
    }
    Ok(())
}

/// The quads of one profile segment (ring `k` to ring `k + 1`) around contour `c`.
fn band(m: &mut OutlineMesh, c: &Contour, rs: &[(f64, f64)], pos: &[Vec<[f64; 3]>], k: usize, curved: bool) -> Result<()> {
    let wall = is_wall(rs, k);
    // Profile normal (outward weight, z weight) of segment k.
    let seg_n = |k: usize| -> (f64, f64) {
        let (i0, z0) = rs[k];
        let (i1, z1) = rs[k + 1];
        let (di, dz) = (i1 - i0, z1 - z0);
        let l = di.hypot(dz).max(f64::MIN_POSITIVE);
        (dz / l, di / l)
    };
    let base = seg_n(k);
    // Curved bevels: average with the neighboring bevel segments at shared rings.
    let blend = |k2: Option<usize>| -> (f64, f64) {
        match k2 {
            Some(k2) if curved && !wall && k2 < rs.len() - 1 && !is_wall(rs, k2) => {
                let o = seg_n(k2);
                (base.0 + o.0, base.1 + o.1)
            }
            _ => base,
        }
    };
    let (pa, pb) = (blend(k.checked_sub(1)), blend(Some(k + 1)));
    let n = c.pts.len();
    let vn = |v: usize, en: P| -> P {
        if c.smooth[v] {
            let m = c.miter[v];
            let l = m[0].hypot(m[1]).max(f64::MIN_POSITIVE);
            [m[0] / l, m[1] / l]
        } else {
            en
        }
    };
    let nrm = |xy: P, pr: (f64, f64)| [xy[0] * pr.0, xy[1] * pr.0, pr.1];
    for i in 0..n {
        let j = (i + 1) % n;
        let en = c.edge_n[i];
        let (ni, nj) = (vn(i, en), vn(j, en));
        let ai = m.push(pos[k][i], nrm(ni, pa))?;
        let aj = m.push(pos[k][j], nrm(nj, pa))?;
        let bj = m.push(pos[k + 1][j], nrm(nj, pb))?;
        let bi = m.push(pos[k + 1][i], nrm(ni, pb))?;
        m.indices.extend([ai, aj, bj, ai, bj, bi]);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use super::*;

    fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
    }

    fn cross3(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
        [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
    }

    fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
        a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
    }

    type EdgeUses = HashMap<(u32, u32), usize>;

    /// Edges of a mesh welded by position: (undirected edge → uses, directed edge → uses).
    fn edge_stats(m: &OutlineMesh) -> (EdgeUses, EdgeUses) {
        let mut weld: HashMap<[u32; 3], u32> = HashMap::new();
        let ids: Vec<u32> = m
            .positions
            .iter()
            .map(|q| {
                let k = q.map(|v| (v + 0.0).to_bits());
                let n = weld.len() as u32;
                *weld.entry(k).or_insert(n)
            })
            .collect();
        let mut und = HashMap::new();
        let mut dir = HashMap::new();
        for t in m.indices.chunks(3) {
            let v = [ids[t[0] as usize], ids[t[1] as usize], ids[t[2] as usize]];
            if v[0] == v[1] || v[1] == v[2] || v[0] == v[2] {
                continue;
            }
            for k in 0..3 {
                let (a, b) = (v[k], v[(k + 1) % 3]);
                *und.entry((a.min(b), a.max(b))).or_insert(0) += 1;
                *dir.entry((a, b)).or_insert(0) += 1;
            }
        }
        (und, dir)
    }

    fn assert_watertight(m: &OutlineMesh) {
        let (und, dir) = edge_stats(m);
        for (e, n) in &und {
            assert_eq!(*n, 2, "edge {e:?} used {n} times");
        }
        for (e, n) in &dir {
            assert_eq!(*n, 1, "directed edge {e:?} used {n} times (inconsistent winding)");
        }
    }

    /// Signed volume (positive when faces wind outward).
    fn volume(m: &OutlineMesh) -> f64 {
        m.indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| m.positions[t[k] as usize].map(|v| v as f64));
                (a[0] * (b[1] * c[2] - b[2] * c[1]) - a[1] * (b[0] * c[2] - b[2] * c[0]) + a[2] * (b[0] * c[1] - b[1] * c[0])) / 6.0
            })
            .sum()
    }

    /// Vertex normals agree with the faces they belong to.
    fn assert_normals_outward(m: &OutlineMesh) {
        for t in m.indices.chunks(3) {
            let (a, b, c) = (m.positions[t[0] as usize], m.positions[t[1] as usize], m.positions[t[2] as usize]);
            let f = cross3(sub(b, a), sub(c, a));
            if dot(f, f) < 1e-10 {
                continue;
            }
            for &i in t {
                let n = m.normals[i as usize];
                assert!((dot(n, n) - 1.0).abs() < 1e-3, "normal not unit");
                assert!(dot(f, n) > 0.0, "normal {n:?} faces away from triangle {f:?}");
            }
        }
    }

    fn z_range(m: &OutlineMesh) -> (f32, f32) {
        m.positions.iter().fold((f32::INFINITY, f32::NEG_INFINITY), |(lo, hi), p| (lo.min(p[2]), hi.max(p[2])))
    }

    fn square(s: f64) -> Outline {
        Outline { outer: vec![[0.0, 0.0], [s, 0.0], [s, s], [0.0, s]], holes: vec![] }
    }

    fn ring_outline() -> Outline {
        let circle = |r: f64, n: usize| -> Vec<[f64; 2]> {
            (0..n).map(|i| (i as f64 / n as f64) * std::f64::consts::TAU).map(|a| [r * a.cos(), r * a.sin()]).collect()
        };
        Outline { outer: circle(100.0, 48), holes: vec![circle(50.0, 32)] }
    }

    #[test]
    fn extrusion_is_watertight_with_outward_normals() {
        let l = Limits::default();
        for bevel in BevelStyle::ALL {
            for o in [square(100.0), ring_outline()] {
                let p = ExtrudeParams { bevel, bevel_depth: 8.0, hole_bevel: 0.5, depth: 40.0 };
                let m = extrude(std::slice::from_ref(&o), &p, &l).unwrap();
                assert_watertight(&m);
                assert_normals_outward(&m);
                assert!(volume(&m) > 0.0, "{bevel:?}: inward winding");
                let (lo, hi) = z_range(&m);
                assert!(lo.abs() < 1e-6 && (hi - 40.0).abs() < 1e-4);
            }
        }
        // Without a bevel the square block's volume is exact.
        let m = extrude(&[square(100.0)], &ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 0.0, hole_bevel: 1.0, depth: 10.0 }, &l).unwrap();
        assert!((volume(&m) - 100_000.0).abs() < 1e-2);
        // Front cap faces -z, back cap +z.
        assert!(m.normals.iter().any(|n| n[2] < -0.99) && m.normals.iter().any(|n| n[2] > 0.99));
    }

    #[test]
    fn bevels_shrink_the_caps() {
        let l = Limits::default();
        let sq = square(100.0);
        let m =
            extrude(std::slice::from_ref(&sq), &ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 10.0, hole_bevel: 1.0, depth: 50.0 }, &l)
                .unwrap();
        // Front cap vertices (z = 0) are inset by the bevel.
        let front: Vec<_> = m.positions.iter().filter(|p| p[2] == 0.0).collect();
        assert!(front.iter().all(|p| p[0] >= 9.999 && p[0] <= 90.001));
        // The widest ring (the walls) still reaches the outline.
        assert!(m.positions.iter().any(|p| p[0] == 0.0 && p[2] == 10.0));
        // Convex bevels have more profile steps than angular ones.
        let c = extrude(std::slice::from_ref(&sq), &ExtrudeParams { bevel: BevelStyle::Convex, bevel_depth: 10.0, hole_bevel: 1.0, depth: 50.0 }, &l)
            .unwrap();
        assert!(c.positions.len() > m.positions.len());
    }

    #[test]
    fn extrusions_split_into_front_bevel_side_and_back() {
        let l = Limits::default();
        let sq = square(100.0);
        let p = ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 10.0, hole_bevel: 1.0, depth: 50.0 };
        let whole = extrude(std::slice::from_ref(&sq), &p, &l).unwrap();
        let parts = extrude_surfaces(std::slice::from_ref(&sq), &p, &l).unwrap();
        // The parts hold exactly the triangles of the whole mesh.
        assert_eq!(parts.iter().map(|m| m.triangle_count()).sum::<usize>(), whole.triangle_count());
        let [front, bevel, side, back] = &parts;
        // Caps: flat, at the plane (front, facing -z) and at the extrusion depth (back, +z).
        assert!(front.positions.iter().all(|q| q[2] == 0.0) && front.normals.iter().all(|n| n[2] < -0.99));
        assert!(back.positions.iter().all(|q| q[2] == 50.0) && back.normals.iter().all(|n| n[2] > 0.99));
        // Walls: the full outline, between the bevels (z 10..40), with sideways normals.
        assert!(side.positions.iter().all(|q| (10.0..=40.0).contains(&q[2])) && side.normals.iter().all(|n| n[2].abs() < 1e-6));
        assert!(side.positions.iter().any(|q| q[0] == 0.0));
        // Bevels: slanted between the caps and the walls, front and back.
        assert!(bevel.triangle_count() > 0 && bevel.normals.iter().all(|n| n[2].abs() > 0.1 && n[2].abs() < 0.99));
        assert!(bevel.positions.iter().any(|q| q[2] == 0.0) && bevel.positions.iter().any(|q| q[2] == 50.0));
        assert_eq!(OutlineSurface::ALL.map(OutlineSurface::index), [0, 1, 2, 3]);
        // Without a bevel there is nothing on the Bevel surface.
        let flat = extrude_surfaces(std::slice::from_ref(&sq), &ExtrudeParams { bevel: BevelStyle::None, ..p }, &l).unwrap();
        assert_eq!(flat[OutlineSurface::Bevel.index()].triangle_count(), 0);
        assert!(flat[OutlineSurface::Side.index()].triangle_count() > 0);
        // Stacked like `extrude_stacked`.
        let up = extrude_surfaces_stacked(std::slice::from_ref(&sq), &p, 2, &l).unwrap();
        assert!(up[0].positions.iter().all(|q| (q[2] + 2.0 * STACK_GAP as f32).abs() < 1e-6));
    }

    #[test]
    fn stacked_extrusions_move_towards_the_camera() {
        let l = Limits::default();
        let sq = square(10.0);
        let p = ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 0.0, hole_bevel: 1.0, depth: 10.0 };
        let base = extrude(std::slice::from_ref(&sq), &p, &l).unwrap();
        let up = extrude_stacked(std::slice::from_ref(&sq), &p, 3, &l).unwrap();
        assert_eq!(base.indices, up.indices);
        for (a, b) in base.positions.iter().zip(&up.positions) {
            assert!((a[2] - b[2] - 3.0 * STACK_GAP as f32).abs() < 1e-6);
            assert_eq!((a[0], a[1]), (b[0], b[1]));
        }
        // The largest level stays inside the f32 range.
        let deep = ExtrudeParams { depth: 1e12, ..p };
        let far = extrude_stacked(std::slice::from_ref(&sq), &deep, usize::MAX, &l).unwrap();
        assert!(far.positions.iter().all(|q| q[2].is_finite() && q[2] < 0.0));
        // Caps that the offset rounds onto one f32 depth are an error, not a flat mesh.
        // Near z = -20 at level 1000, f32 steps by about 1.9e-6.
        let thin = ExtrudeParams { depth: 1e-7, ..p };
        assert!(extrude_stacked(std::slice::from_ref(&sq), &thin, 0, &l).is_ok());
        assert!(matches!(extrude_stacked(std::slice::from_ref(&sq), &thin, 1000, &l), Err(MeshOpsError::Invalid(_))));
        assert!(matches!(extrude_surfaces_stacked(std::slice::from_ref(&sq), &thin, 1000, &l), Err(MeshOpsError::Invalid(_))));
        assert!(matches!(extrude_stacked(std::slice::from_ref(&sq), &p, usize::MAX, &l), Err(MeshOpsError::Invalid(_))));
    }

    #[test]
    fn extrusion_counts_its_output_before_building() {
        let l = Limits::default();
        let p = ExtrudeParams::default();
        let m = extrude(&[ring_outline()], &p, &l).unwrap();
        let parts = extrude_surfaces(&[ring_outline()], &p, &l).unwrap();
        let (v, t) = (m.positions.len(), m.triangle_count());
        assert_eq!(parts.iter().map(|m| m.positions.len()).sum::<usize>(), v);
        assert_eq!(m.normals.len(), v);
        assert_eq!(m.uvs.len(), v);
        assert!(extrude(&[ring_outline()], &p, &Limits { max_vertices: v, max_triangles: t, ..l }).is_ok());
        let too_large = |r: Result<OutlineMesh>, what: &str| match r {
            Err(MeshOpsError::TooLarge { what: w, .. }) => assert_eq!(w, what),
            other => panic!("{what}: {other:?}"),
        };
        too_large(extrude(&[ring_outline()], &p, &Limits { max_vertices: v - 1, ..l }), "vertex count");
        too_large(extrude(&[ring_outline()], &p, &Limits { max_triangles: t - 1, ..l }), "triangle count");
        too_large(extrude(&[ring_outline()], &p, &Limits { max_outline_vertices: 79, ..l }), "outline vertex count");
        too_large(extrude(&[ring_outline()], &p, &Limits { max_outline_work: 100, ..l }), "outline work");
        // Nothing to extrude gives an empty mesh.
        assert_eq!(extrude(&[], &p, &l).unwrap(), OutlineMesh::default());
    }

    #[test]
    fn extrusion_rejects_invalid_parameters() {
        let l = Limits::default();
        let sq = [square(10.0)];
        let ok = ExtrudeParams::default();
        for p in [
            ExtrudeParams { depth: f64::NAN, ..ok },
            ExtrudeParams { depth: f64::INFINITY, ..ok },
            ExtrudeParams { depth: 0.0, ..ok },
            ExtrudeParams { depth: -1.0, ..ok },
            ExtrudeParams { bevel_depth: f64::NAN, ..ok },
            ExtrudeParams { bevel_depth: f64::INFINITY, ..ok },
            ExtrudeParams { bevel_depth: -1.0, ..ok },
            ExtrudeParams { hole_bevel: f64::NAN, ..ok },
            ExtrudeParams { hole_bevel: 1.5, ..ok },
            ExtrudeParams { hole_bevel: -0.1, ..ok },
        ] {
            assert!(matches!(extrude(&sq, &p, &l), Err(MeshOpsError::Invalid(_))), "{p:?}");
        }
        // A depth that leaves the f32 range, and an outline outside it.
        assert!(matches!(extrude(&sq, &ExtrudeParams { depth: 1e39, ..ok }, &l), Err(MeshOpsError::Invalid(_))));
        let far = Outline { outer: vec![[1e39, 0.0], [2e39, 0.0], [2e39, 1e39], [1e39, 1e39]], holes: vec![] };
        assert!(matches!(extrude(&[far], &ok, &l), Err(MeshOpsError::Invalid(_))));
        // f32 steps by 8 near 1e8: a square of side 1 there loses its edges, one of side 16 keeps them.
        let at = |x: f64, s: f64| Outline { outer: vec![[x, 0.0], [x + s, 0.0], [x + s, s], [x, s]], holes: vec![] };
        assert!(matches!(extrude(&[at(1e8, 1.0)], &ok, &l), Err(MeshOpsError::Invalid(_))));
        assert!(extrude(&[at(1e8, 16.0)], &ExtrudeParams { bevel_depth: 0.0, ..ok }, &l).is_ok());
        // A depth that rounds to 0 in f32 would put both caps on one plane.
        assert!(matches!(extrude(&sq, &ExtrudeParams { depth: 1e-50, ..ok }, &l), Err(MeshOpsError::Invalid(_))));
        // A bevel deeper than half the extrusion is built at half the extrusion.
        let m =
            extrude(&[square(100.0)], &ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 30.0, hole_bevel: 1.0, depth: 20.0 }, &l).unwrap();
        assert!(m.positions.iter().filter(|q| q[2] == 0.0).all(|q| q[0] >= 9.999 && q[0] <= 90.001));
    }

    #[test]
    fn bevels_are_built_at_any_scale() {
        let l = Limits::default();
        for s in [1e-8, 1.0, 1e6] {
            let p = ExtrudeParams { bevel: BevelStyle::Concave, bevel_depth: s * 0.01, hole_bevel: 1.0, depth: s * 0.5 };
            let [front, bevel, side, back] = extrude_surfaces(&[square(s)], &p, &l).unwrap();
            assert_eq!(bevel.triangle_count(), 2 * 6 * 4 * 2, "{s}: six steps of four quads at each end");
            assert!(side.triangle_count() == 8 && front.triangle_count() == 2 && back.triangle_count() == 2, "{s}");
            assert!(side.normals.iter().all(|n| n[2] == 0.0), "{s}: walls stay walls");
            let whole = extrude(&[square(s)], &p, &l).unwrap();
            assert_watertight(&whole);
        }
        // A bevel under 1e-12 units is still a bevel when f32 can hold it at the outline's size.
        let tiny = ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 1e-13, hole_bevel: 1.0, depth: 1e-8 };
        assert_eq!(extrude_surfaces(&[square(1e-8)], &tiny, &l).unwrap()[1].triangle_count(), 2 * 4 * 2);
        // Normals stay unit length on a very thin extrusion.
        let thin = extrude(&[square(1.0)], &ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 0.0, hole_bevel: 1.0, depth: 1e-30 }, &l).unwrap();
        assert!(thin.normals.iter().all(|n| (n[0].hypot(n[1]).hypot(n[2]) - 1.0).abs() < 1e-6), "{:?}", thin.normals);
        // A bevel narrower than the f32 step near 1e8 would leave no bevel in the mesh.
        let far = Outline { outer: vec![[1e8, 1e8], [1e8 + 64.0, 1e8], [1e8 + 64.0, 1e8 + 64.0], [1e8, 1e8 + 64.0]], holes: vec![] };
        let narrow = ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 1.0, hole_bevel: 1.0, depth: 10.0 };
        assert!(matches!(extrude(&[far], &narrow, &l), Err(MeshOpsError::Invalid(_))));
        // A hole 1 unit inside the outer wall near 1e8 meets that wall once rounded to f32.
        let sq = |x: f64, y: f64, s: f64| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        let near = Outline { outer: sq(1e8, 1e8, 64.0), holes: vec![sq(1e8 + 1.0, 1e8 + 8.0, 16.0)] };
        let flat = ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 0.0, hole_bevel: 1.0, depth: 10.0 };
        assert!(matches!(extrude(std::slice::from_ref(&near), &flat, &l), Err(MeshOpsError::Invalid(_))));
        let apart = Outline { outer: sq(1e8, 1e8, 64.0), holes: vec![sq(1e8 + 16.0, 1e8 + 8.0, 16.0)] };
        assert!(extrude(&[apart], &flat, &l).is_ok());
    }

    #[test]
    fn malformed_outlines_and_wide_bevels_are_errors() {
        let l = Limits::default();
        let angular = |b: f64| ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: b, hole_bevel: 1.0, depth: 50.0 };
        let bow_tie = Outline { outer: vec![[0.0, 0.0], [2.0, 2.0], [0.0, 2.0], [2.0, 0.0]], holes: vec![] };
        assert!(matches!(extrude(&[bow_tie], &ExtrudeParams::default(), &l), Err(MeshOpsError::Invalid(_))));
        let crossing = Outline { outer: square(10.0).outer, holes: vec![vec![[5.0, 5.0], [15.0, 5.0], [15.0, 6.0], [5.0, 6.0]]] };
        assert!(matches!(extrude(&[crossing], &ExtrudeParams::default(), &l), Err(MeshOpsError::Invalid(_))));
        // An inset of 4 leaves a 2 × 2 cap; 5 collapses it and 6 turns it inside out.
        assert!(extrude(&[square(10.0)], &angular(4.0), &l).is_ok());
        for b in [5.0, 6.0, 10.0] {
            match extrude(&[square(10.0)], &angular(b), &l) {
                Err(MeshOpsError::Invalid(why)) => assert!(why.contains("bevel"), "{why}"),
                other => panic!("bevel {b}: {other:?}"),
            }
        }
        // A hole's bevel that grows it into the outer contour's cap.
        let near = Outline { outer: square(10.0).outer, holes: vec![vec![[2.0, 2.0], [2.0, 8.0], [8.0, 8.0], [8.0, 2.0]]] };
        assert!(extrude(std::slice::from_ref(&near), &angular(0.5), &l).is_ok());
        assert!(matches!(extrude(&[near], &angular(1.5), &l), Err(MeshOpsError::Invalid(_))));
    }

    #[test]
    fn contours_group_into_outlines_by_nesting() {
        let sq = |x: f64, y: f64, s: f64| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        // "O" (outer + hole) next to an island inside another hole.
        let g =
            group_contours(&[sq(0.0, 0.0, 10.0), sq(2.0, 2.0, 6.0), sq(20.0, 0.0, 10.0), sq(22.0, 2.0, 6.0), sq(24.0, 4.0, 2.0)], &Limits::default())
                .unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g.iter().map(|o| o.holes.len()).sum::<usize>(), 2);
    }

    #[test]
    fn overlapping_contours_keep_every_container_in_their_depth() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        // A and B overlap without nesting; C crosses A and lies wholly inside B, so only B holds
        // it: C is a hole of B, wherever C starts.
        let (a, b) = (rect(0.0, 0.0, 10.0, 10.0), rect(5.0, -2.0, 13.0, 8.0));
        let c = rect(9.0, 1.0, 12.0, 2.0);
        for start in 0..c.len() {
            let mut rotated = c.clone();
            rotated.rotate_left(start);
            let g = group_contours(&[a.clone(), b.clone(), rotated.clone()], &Limits::default()).unwrap();
            assert_eq!(g.len(), 2, "start {start}");
            let o = g.iter().find(|o| o.outer == b).unwrap();
            assert_eq!(o.holes, vec![rotated], "start {start}");
        }
        // A square whose first point lies inside an L but which crosses its notch, inside the
        // L's bounding box: the L does not hold it, so both are outer contours.
        let l_shape = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 4.0], [4.0, 4.0], [4.0, 10.0], [0.0, 10.0]];
        let g = group_contours(&[l_shape, rect(3.0, 3.0, 6.0, 6.0)], &Limits::default()).unwrap();
        assert_eq!(g.len(), 2);
        assert!(g.iter().all(|o| o.holes.is_empty()));
        // A contour that lies inside both of two overlapping contours is two deep: an outer
        // contour, not a hole.
        let d = rect(6.0, 1.0, 8.0, 2.0);
        let g = group_contours(&[a, b, d], &Limits::default()).unwrap();
        assert_eq!(g.len(), 3);
        assert_eq!(g.iter().map(|o| o.holes.len()).sum::<usize>(), 0);
    }

    #[test]
    fn an_edge_that_leaves_through_two_outer_vertices_is_not_held() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        // Each candidate's top edge runs through two vertices of the outer contour and leaves it
        // between them, while its ends and middle stay inside.
        let notch =
            vec![[0.0, 0.0], [12.0, 0.0], [12.0, 10.0], [4.0, 10.0], [4.0, 6.0], [4.0, 4.0], [2.0, 4.0], [2.0, 6.0], [2.0, 10.0], [0.0, 10.0]];
        let vee = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [9.0, 10.0], [9.0, 5.0], [8.0, 4.0], [7.0, 5.0], [7.0, 10.0], [0.0, 10.0]];
        for (outer, inner) in [(notch, rect(1.0, 3.0, 11.0, 6.0)), (vee, rect(1.0, 3.0, 9.5, 5.0))] {
            for start in 0..inner.len() {
                let mut rotated = inner.clone();
                rotated.rotate_left(start);
                let g = group_contours(&[outer.clone(), rotated], &Limits::default()).unwrap();
                assert_eq!(g.len(), 2, "start {start}");
                assert!(g.iter().all(|o| o.holes.is_empty()), "start {start}");
            }
        }
        // A hole whose edge runs along part of the outer contour's edge is still held.
        let g = group_contours(&[rect(0.0, 0.0, 10.0, 10.0), rect(0.0, 2.0, 4.0, 4.0)], &Limits::default()).unwrap();
        assert_eq!(g.iter().map(|o| o.holes.len()).collect::<Vec<_>>(), [1]);
        // Extrusion rejects a hole that touches its outer contour, as the grouping docs say.
        assert!(matches!(extrude(&g, &ExtrudeParams::default(), &Limits::default()), Err(MeshOpsError::Invalid(_))));
    }

    #[test]
    fn a_piece_between_adjacent_coordinates_is_tested_without_its_rounded_middle() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        // A notch one coordinate step wide: the middle of the candidate's edge across it rounds onto
        // a vertex of the outer contour, at 2^53 and near 1.
        let big = 2f64.powi(53);
        let near_one = 1.0 + f64::EPSILON;
        let cases = [(1.0, near_one, [0.0, 0.0, 3.0, 3.0], [1.0, 2.0]), (big, big + 2.0, [big - 4.0, 0.0, big + 12.0, 10.0], [4.0, 6.0])];
        for (x0, x1, [left, top, right, bottom], [y0, y1]) in cases {
            let outer =
                vec![[left, top], [right, top], [right, bottom], [x1, bottom], [x1, y1], [x1, y0], [x0, y0], [x0, y1], [x0, bottom], [left, bottom]];
            assert_eq!([x0 * 0.5 + x1 * 0.5, y1], [x0, y1], "the middle rounds onto a vertex");
            let (in_left, in_right) = ((left + x0) * 0.5, (x1 + right) * 0.5);
            // The edge at y1 crosses the notch: not held, wherever the candidate starts.
            let crossing = rect(in_left, top + (y0 - top) * 0.5, in_right, y1);
            // The edge at y0 runs along the notch's bottom and touches its two corners: held.
            let touching = rect(in_left, top + (y0 - top) * 0.5, in_right, y0);
            for start in 0..4 {
                let mut rotated = crossing.clone();
                rotated.rotate_left(start);
                let g = group_contours(&[outer.clone(), rotated], &Limits::default()).unwrap();
                assert_eq!(g.len(), 2, "x0 {x0}, start {start}");
                assert!(g.iter().all(|o| o.holes.is_empty()), "x0 {x0}, start {start}");
                let mut rotated = touching.clone();
                rotated.rotate_left(start);
                let g = group_contours(&[outer.clone(), rotated.clone()], &Limits::default()).unwrap();
                assert_eq!(g, [Outline { outer: outer.clone(), holes: vec![rotated] }], "x0 {x0}, start {start}");
            }
        }
    }

    #[test]
    fn a_contour_is_kept_wherever_it_starts() {
        // Relative to (-1e20, -1e20), the other two points round to one difference.
        let mut tri = vec![[-1e20, -1e20], [0.0, 0.0], [1.0, 0.0]];
        for start in 0..3 {
            let g = group_contours(&[tri.clone()], &Limits::default()).unwrap();
            assert_eq!(g.len(), 1, "start {start}");
            tri.rotate_left(1);
        }
    }

    #[test]
    fn small_features_at_large_coordinates_and_large_features_near_the_origin_group_and_extrude() {
        let sq = |x: f64, y: f64, s: f64| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        let l = Limits::default();
        // A unit square near 1e6 holding a square 1e-3 wide, and a square 2e6 wide around the
        // origin holding two squares 1e-3 wide near the origin.
        let near = group_contours(&[sq(1e6, 1e6, 1.0), sq(1e6 + 0.5, 1e6 + 0.5, 1e-3)], &l).unwrap();
        assert_eq!(near.iter().map(|o| o.holes.len()).collect::<Vec<_>>(), [1]);
        let far = group_contours(&[sq(-1e6, -1e6, 2e6), sq(1e-3, 1e-3, 1e-3), sq(-2e-3, -2e-3, 1e-3)], &l).unwrap();
        assert_eq!(far.iter().map(|o| o.holes.len()).collect::<Vec<_>>(), [2]);
        // f32 steps by 0.0625 near 1e6, so the small hole there rounds away: an error, not a mesh.
        let flat = ExtrudeParams { bevel: BevelStyle::None, bevel_depth: 0.0, hole_bevel: 1.0, depth: 10.0 };
        assert!(matches!(extrude(&near, &flat, &l), Err(MeshOpsError::Invalid(_))));
        // Near the origin f32 keeps the holes: the mesh is closed and faces outward.
        let m = extrude(&far, &flat, &l).unwrap();
        assert_watertight(&m);
        assert_normals_outward(&m);
        assert!(volume(&m) > 0.0);
    }

    #[test]
    fn a_contour_wider_than_the_limit_is_invalid_before_its_area_is_taken() {
        // The products of one shoelace term overflow, so the area is NaN.
        let r = group_contours(&[vec![[0.0, 0.0], [1e160, 1e160], [1e160, 2e160]]], &Limits::default());
        assert!(matches!(r, Err(MeshOpsError::Invalid(ref e)) if e.contains("span")), "{r:?}");
    }

    #[test]
    fn caps_face_their_way_once_rounded_to_f32() {
        // The first ear turns left in f64 and right once its x near 1e8 is rounded to f32.
        let outer = vec![[100_000_011.0, 1.0], [100_000_021.0, 2.0], [100_000_000.0, 4.0], [100_000_000.0, 0.0]];
        let m = extrude(&[Outline { outer, holes: vec![] }], &ExtrudeParams::default(), &Limits::default()).unwrap();
        assert_normals_outward(&m);
    }

    #[test]
    fn rings_that_round_to_one_position_are_invalid() {
        // At stacking level 1000 the two wall rings, at depths 1 and 1.0000001, both round to
        // -19.
        let sq = vec![[0.0, 0.0], [10.0, 0.0], [10.0, 10.0], [0.0, 10.0]];
        let p = ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 1.0, depth: 2.000_000_1, ..ExtrudeParams::default() };
        let o = [Outline { outer: sq.clone(), holes: vec![] }];
        let r = extrude_stacked(&o, &p, 1000, &Limits::default());
        assert!(matches!(r, Err(MeshOpsError::Invalid(ref e)) if e.contains("round to one f32 position")), "{r:?}");
        // At level 0 the depths stay apart.
        extrude_stacked(&o, &p, 0, &Limits::default()).unwrap();
    }

    #[test]
    fn a_bevel_lost_before_rounding_is_invalid() {
        // At 1e8 an inset of 1e-10 is lost in f64, while the bevel's depth step survives in f32.
        let (x, s) = (1e8, 64.0);
        let sq = vec![[x, x], [x + s, x], [x + s, x + s], [x, x + s]];
        let p = ExtrudeParams { bevel: BevelStyle::Angular, bevel_depth: 1e-10, depth: 1e-8, ..ExtrudeParams::default() };
        let r = extrude(&[Outline { outer: sq, holes: vec![] }], &p, &Limits::default());
        assert!(matches!(r, Err(MeshOpsError::Invalid(ref e)) if e.contains("no width")), "{r:?}");
    }

    #[test]
    fn a_contour_inside_a_hole_takes_the_innermost_outline_not_the_hole() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        // A and B overlap; C lies in A only (a hole of A); D lies in A, B and C (three deep, a hole).
        // The smallest container of D is the hole C, which has no outline of its own: D goes to B.
        let g = group_contours(
            &[rect(0.0, 0.0, 10.0, 10.0), rect(5.0, -2.0, 13.0, 8.0), rect(1.0, 1.0, 9.0, 9.0), rect(6.0, 2.0, 7.0, 3.0)],
            &Limits::default(),
        )
        .unwrap();
        assert_eq!(g.len(), 2);
        assert_eq!(g.iter().map(|o| o.holes.len()).collect::<Vec<_>>(), [1, 1], "C is a hole of A, D a hole of B (not dropped)");
    }

    #[test]
    fn a_hole_whose_outline_is_deeper_than_it_is_still_attached() {
        let rect = |x0: f64, y0: f64, x1: f64, y1: f64| vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]];
        // D is three deep and goes to its innermost outline A. The large square and the long strip
        // cross A without holding it, so A is two deep.
        let a = rect(0.0, 0.0, 10.0, 10.0);
        let g = group_contours(
            &[
                a.clone(),
                rect(-1.0, -1.0, 20.0, 20.0),
                rect(-2.0, -2.0, 25.0, 25.0),
                rect(5.0, 5.0, 6.0, 6.0),
                rect(-100.0, -100.0, 3.0, 3.0),
                rect(-0.5, -1000.0, 0.5, 1000.0),
            ],
            &Limits::default(),
        )
        .unwrap();
        let o = g.iter().find(|o| o.outer == a).expect("A is an outline");
        assert_eq!(o.holes.len(), 1, "D is a hole of A");
    }

    #[test]
    fn contour_grouping_is_bounded() {
        let sq = |x: f64, y: f64, s: f64| vec![[x, y], [x + s, y], [x + s, y + s], [x, y + s]];
        let l = Limits::default();
        assert_eq!(l.max_outline_contours, 10_000);
        let n = l.max_outline_contours + 1;
        let cs: Vec<Vec<[f64; 2]>> = (0..n).map(|i| sq(i as f64 * 2.0, 0.0, 1.0)).collect();
        let g = group_largest_contours(&cs, &l).unwrap();
        assert_eq!(g.len(), l.max_outline_contours);
        assert!(g.iter().all(|o| o.holes.is_empty()));
        assert!(matches!(group_contours(&cs, &l), Err(MeshOpsError::TooLarge { what: "outline contour count", .. })));
        // The largest are kept, in their order.
        let few = Limits { max_outline_contours: 2, ..l };
        let g = group_largest_contours(&[sq(0.0, 0.0, 1.0), sq(10.0, 0.0, 3.0), sq(20.0, 0.0, 2.0)], &few).unwrap();
        assert_eq!(g.iter().map(|o| o.outer[0][0]).collect::<Vec<_>>(), [10.0, 20.0]);
        // Points and work are capped as well.
        assert!(matches!(group_contours(&cs, &Limits { max_outline_vertices: 4 * n - 1, ..l }), Err(MeshOpsError::TooLarge { .. })));
        assert!(matches!(
            group_contours(&cs[..100], &Limits { max_outline_work: 1_000, ..l }),
            Err(MeshOpsError::TooLarge { what: "outline work", .. })
        ));
    }

    #[test]
    fn grouping_cleans_and_checks_contours() {
        let l = Limits::default();
        // Repeated points and a closing point are removed; contours without area are dropped.
        let g = group_contours(
            &[
                vec![[0.0, 0.0], [0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0], [0.0, 0.0]],
                vec![[0.0, 0.0], [1.0, 1.0], [2.0, 2.0]],
                vec![[1.0, 1.0]],
            ],
            &l,
        )
        .unwrap();
        assert_eq!(g, [Outline { outer: vec![[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]], holes: vec![] }]);
        assert!(group_contours::<Vec<P>>(&[], &l).unwrap().is_empty());
        assert!(matches!(group_contours(&[vec![[0.0, 0.0], [f64::NAN, 0.0], [1.0, 1.0]]], &l), Err(MeshOpsError::Invalid(_))));
        // Contours spanning more than 1e150 together are refused, as triangulate refuses them,
        // so containment tests do not overflow.
        let big = vec![[0.0, 0.0], [1e160, 0.0], [0.0, 1e160]];
        let small = vec![[8e159, 8e159], [8e159 + 1e146, 8e159], [8e159, 8e159 + 1e146]];
        assert!(matches!(group_contours(&[big, small.clone()], &l), Err(MeshOpsError::Invalid(_))));
        // Far from the origin, a small extent is accepted.
        assert_eq!(group_contours(&[small], &l).unwrap().len(), 1);
        assert!(!point_in([0.0, 0.0], &[]));
    }
}
