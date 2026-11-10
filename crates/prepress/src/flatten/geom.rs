//! Planar regions for the transparency flattener: a page's filled shapes cut into the faces of
//! their arrangement, each tagged with the shapes that cover it. The same shapes cover every
//! point of a region, so one composited color is exact for all of it.
//!
//! Ported from VectorCraft (MIT OR Apache-2.0) at commit c2b15982:
//! `crates/pathops/src/pathfinder.rs` (`regions`, `Region`, `Shape`),
//! `crates/pathops/src/boolean.rs` (the arrangement, per-input winding numbers, horizontal
//! snapping, contour repair and cleanup) and `overlapping` in
//! `crates/engine/src/cmd/flatten.rs`. The sweep is `linesweeper`'s, on `kurbo` curves.
//!
//! Clips go into the same sweep as the shapes instead of being applied first: one sweep replaces
//! a boolean per clipped shape, and a clip edge is then the same edge in every region it bounds.

use std::collections::HashMap;

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveNearest, PathEl, PathSeg, Point, Rect, Shape as _, Vec2};
use linesweeper::topology::{ContourIdx, Contours, Topology, WindingNumber};

/// Contours narrower than this on average (2 × area / perimeter, in input units) are dropped as
/// numerical debris, such as the lens between two nearly coincident curves.
pub const DEFAULT_SLIVER: f64 = 0.01;

/// How a path's winding number decides what is inside it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum FillRule {
    #[default]
    NonZero,
    EvenOdd,
}

impl FillRule {
    /// Whether a point of winding number `winding` is inside.
    pub fn inside(self, winding: i32) -> bool {
        match self {
            FillRule::NonZero => winding != 0,
            FillRule::EvenOdd => winding % 2 != 0,
        }
    }
}

/// A filled shape. Open subpaths are closed, as a fill closes them.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub path: BezPath,
    pub rule: FillRule,
    /// Indices into the clip list: the shape covers only what all of them cover.
    pub clips: Vec<usize>,
}

impl Shape {
    pub fn new(path: BezPath, rule: FillRule) -> Self {
        Self { path, rule, clips: Vec::new() }
    }

    pub fn clipped(mut self, clips: impl IntoIterator<Item = usize>) -> Self {
        self.clips.extend(clips);
        self
    }
}

/// A clipping path.
#[derive(Clone, Debug, PartialEq)]
pub struct ClipPath {
    pub path: BezPath,
    pub rule: FillRule,
}

/// One face of the arrangement: an area where the covering shapes don't change.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Outer contours and holes, oriented so that the nonzero rule fills the region.
    pub path: BezPath,
    /// Unsigned area.
    pub area: f64,
    /// Indices of the shapes covering the region, in input (paint) order.
    pub sources: Vec<usize>,
}

impl Region {
    /// The front-most shape covering the region.
    pub fn top(&self) -> Option<usize> {
        self.sources.last().copied()
    }

    pub fn contains(&self, p: Point) -> bool {
        self.path.winding(p) != 0
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum GeomError {
    #[error("input contains NaN or infinite coordinates")]
    NonFinite,
    #[error("shape {shape} names clip {clip}, which does not exist")]
    UnknownClip { shape: usize, clip: usize },
    /// The sweep failed on near-degenerate input; the caller rasterizes instead.
    #[error("the shapes are too degenerate to sweep")]
    Degenerate,
}

/// The regions of `shapes` (back to front), each clipped by the `clips` it names. They tile the
/// union of the clipped shapes without overlaps; regions narrower than [`DEFAULT_SLIVER`] are
/// left out.
pub fn regions(shapes: &[Shape], clips: &[ClipPath]) -> Result<Vec<Region>, GeomError> {
    regions_with(shapes, clips, DEFAULT_SLIVER)
}

/// [`regions`] with contours narrower than `sliver` dropped.
pub fn regions_with(shapes: &[Shape], clips: &[ClipPath], sliver: f64) -> Result<Vec<Region>, GeomError> {
    if shapes.is_empty() {
        return Ok(Vec::new());
    }
    Ok(Arrangement::new(shapes, clips, sliver)?.regions())
}

/// Indices of `bounds` grouped by overlap, transitively, each group in index order. Touching boxes
/// overlap; NaN boxes overlap nothing.
pub fn overlap_groups(bounds: &[Rect]) -> Vec<Vec<usize>> {
    fn find(p: &mut [usize], mut i: usize) -> usize {
        while p[i] != i {
            p[i] = p[p[i]];
            i = p[i];
        }
        i
    }
    let mut parent: Vec<usize> = (0..bounds.len()).collect();
    let mut order: Vec<(usize, Rect)> = bounds.iter().enumerate().filter(|(_, b)| !b.is_nan()).map(|(i, b)| (i, b.abs())).collect();
    order.sort_by(|a, b| a.1.x0.total_cmp(&b.1.x0));
    // Sweep left to right, comparing each box with those still open at its left edge.
    let mut open: Vec<(usize, Rect)> = Vec::new();
    for (i, b) in order {
        open.retain(|(_, o)| o.x1 >= b.x0);
        for (j, o) in &open {
            if o.y0 <= b.y1 && b.y0 <= o.y1 {
                let (a, c) = (find(&mut parent, i), find(&mut parent, *j));
                parent[a.max(c)] = a.min(c);
            }
        }
        open.push((i, b));
    }
    let mut groups: Vec<Vec<usize>> = Vec::new();
    let mut slot: Vec<Option<usize>> = vec![None; bounds.len()];
    for i in 0..bounds.len() {
        let r = find(&mut parent, i);
        match slot[r] {
            Some(g) => groups[g].push(i),
            None => {
                slot[r] = Some(groups.len());
                groups.push(vec![i]);
            }
        }
    }
    groups
}

/// The [`overlap_groups`] that hold a transparent item. Only their items need [`regions`]; every
/// other item paints as it is. Groups are transitive so that an opaque item keeps its place in
/// paint order above or below everything it overlaps.
pub fn transparent_groups(bounds: &[Rect], transparent: &[bool]) -> Vec<Vec<usize>> {
    let mut groups = overlap_groups(bounds);
    groups.retain(|g| g.iter().any(|&i| transparent.get(i).copied().unwrap_or(false)));
    groups
}

/// Winding numbers for any number of inputs: the non-zero ones as (tag, winding), sorted by tag.
/// A face is covered by a few inputs, so this stays short where a vector of every input's winding
/// would cost one step per input.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
struct Multi(Vec<(usize, i32)>);

impl Multi {
    fn get(&self, tag: usize) -> i32 {
        self.0.binary_search_by_key(&tag, |e| e.0).ok().and_then(|k| self.0.get(k)).map_or(0, |e| e.1)
    }
}

impl std::ops::Add for Multi {
    type Output = Multi;
    fn add(mut self, rhs: Self) -> Multi {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Multi {
    fn add_assign(&mut self, rhs: Self) {
        // The sweep mostly adds one input's ±1 to a short list: add in place, insert in tag order,
        // drop a winding that reaches zero.
        for (t, k) in rhs.0 {
            match self.0.binary_search_by_key(&t, |e| e.0) {
                Ok(i) => {
                    if let Some(e) = self.0.get_mut(i) {
                        e.1 += k;
                        if e.1 == 0 {
                            self.0.remove(i);
                        }
                    }
                }
                Err(i) if k != 0 => self.0.insert(i, (t, k)),
                Err(_) => {}
            }
        }
    }
}

impl WindingNumber for Multi {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        Multi(vec![(tag, if positive { 1 } else { -1 })])
    }
    fn of_tag(&self, tag: usize) -> Self {
        let w = self.get(tag);
        Multi(if w == 0 { vec![] } else { vec![(tag, w)] })
    }
}

/// The sweep of shapes and clips. Tags `0..n` are shapes, `n + c` is clip `c`.
struct Arrangement {
    top: Topology<Multi>,
    rules: Vec<FillRule>,
    n: usize,
    /// Each shape's clip tags.
    clips: Vec<Vec<usize>>,
    tidy: Tidy,
}

impl Arrangement {
    fn new(shapes: &[Shape], clips: &[ClipPath], sliver: f64) -> Result<Self, GeomError> {
        let n = shapes.len();
        let mut used = vec![false; clips.len()];
        let mut shape_clips = Vec::with_capacity(n);
        for (shape, s) in shapes.iter().enumerate() {
            let mut tags = Vec::with_capacity(s.clips.len());
            for &clip in &s.clips {
                *used.get_mut(clip).ok_or(GeomError::UnknownClip { shape, clip })? = true;
                tags.push(n + clip);
            }
            tags.sort_unstable();
            tags.dedup();
            shape_clips.push(tags);
        }
        let mut paths: Vec<(BezPath, usize)> = shapes
            .iter()
            .enumerate()
            .map(|(i, s)| (closed(&s.path), i))
            .chain(clips.iter().enumerate().filter(|(c, _)| used[*c]).map(|(c, p)| (closed(&p.path), n + c)))
            .collect();
        let eps = eps_for(paths.iter().map(|p| &p.0))?;
        snap_horizontals(&mut paths.iter_mut().map(|p| &mut p.0).collect::<Vec<_>>(), eps);
        // The sweep can panic on rare near-degenerate input; the flattener then rasterizes.
        let sweep = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| Topology::<Multi>::from_paths(paths.iter().map(|(p, t)| (p, *t)), eps)));
        // Every subpath is closed, so a path error is a degenerate input too.
        let top = sweep.map_err(|_| GeomError::Degenerate)?.map_err(|_| GeomError::Degenerate)?;
        let rules = shapes.iter().map(|s| s.rule).chain(clips.iter().map(|c| c.rule)).collect();
        let tidy = Tidy::keeping(sliver, paths.iter().map(|p| &p.0));
        Ok(Self { top, rules, n, clips: shape_clips, tidy })
    }

    /// The shapes covering a face of winding `w`, inside all their clips.
    fn covering(&self, w: &Multi) -> Vec<usize> {
        let inside: Vec<usize> = w.0.iter().filter(|&&(t, k)| self.rules.get(t).is_some_and(|r| r.inside(k))).map(|&(t, _)| t).collect();
        inside.iter().copied().take_while(|&t| t < self.n).filter(|&s| self.clips[s].iter().all(|c| inside.binary_search(c).is_ok())).collect()
    }

    /// Every face, with the faces of one coverage set merged where they touch.
    ///
    /// A contour pass walks the whole arrangement, so one pass per coverage set would cost
    /// sets × edges. Instead the sets go into batches whose faces share no vertex (so no edge
    /// either): one pass per batch walks each face's contours as its own set's pass would, and the
    /// set a contour bounds is the one of its batch at its first point.
    fn regions(&self) -> Vec<Region> {
        let top = &self.top;
        // The distinct non-empty coverage sets, numbered in the order met, and the set of each
        // side of each segment. Faces have many sides, so a set is worked out once per winding.
        let mut ids: HashMap<Vec<usize>, usize> = HashMap::new();
        let mut sets: Vec<Vec<usize>> = Vec::new();
        let mut by_winding: HashMap<&Multi, Option<usize>> = HashMap::new();
        // The contour passes hand their predicate references into the topology's windings, so
        // the winding's address finds its set without hashing the winding.
        let mut by_addr: HashMap<usize, Option<usize>> = HashMap::new();
        let mut sides = Vec::new();
        for s in top.segment_indices() {
            let h = s.first_half();
            let mut pair = [None; 2];
            for (side, w) in pair.iter_mut().zip([top.winding_clockwise(h), top.winding_counter_clockwise(h)]) {
                let m = *by_winding.entry(w).or_insert_with(|| {
                    let set = self.covering(w);
                    let next = sets.len();
                    (!set.is_empty()).then(|| {
                        *ids.entry(set.clone()).or_insert_with(|| {
                            sets.push(set);
                            next
                        })
                    })
                });
                by_addr.insert(std::ptr::from_ref(w).addr(), m);
                *side = m;
            }
            sides.push((s, pair));
        }
        let id = |w: &Multi| match by_addr.get(&std::ptr::from_ref(w).addr()) {
            Some(m) => *m,
            None => ids.get(&self.covering(w)).copied(),
        };
        // The sets of the faces around each vertex.
        let mut around: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
        for (s, pair) in sides {
            for end in [s.first_half(), s.second_half()] {
                let at = around.entry(vertex_key(top.point(end).to_kurbo())).or_default();
                for m in pair.into_iter().flatten() {
                    if !at.contains(&m) {
                        at.push(m);
                    }
                }
            }
        }
        let mut near = vec![Vec::new(); sets.len()];
        for at in around.values() {
            for &a in at {
                near[a].extend(at.iter().copied().filter(|&b| b != a));
            }
        }
        // Greedy coloring: each set takes the first batch none of its neighbors is in.
        let mut batch = vec![usize::MAX; sets.len()];
        let mut batches = 0;
        for m in 0..sets.len() {
            let mut taken = vec![false; batches];
            for &n in &near[m] {
                if let Some(t) = taken.get_mut(batch[n]) {
                    *t = true;
                }
            }
            batch[m] = taken.iter().position(|t| !t).unwrap_or(batches);
            batches = batches.max(batch[m] + 1);
        }
        let mut faces: Vec<Vec<BezPath>> = vec![Vec::new(); sets.len()];
        for b in 0..batches {
            let c = top.contours(|w| id(w).is_some_and(|m| batch[m] == b));
            let owner: Option<Vec<usize>> = c
                .contours()
                .map(|ct| {
                    let Some(PathEl::MoveTo(p)) = ct.path.elements().first() else { return None };
                    around.get(&vertex_key(*p))?.iter().copied().find(|&m| batch[m] == b)
                })
                .collect();
            let Some(owner) = owner else {
                // Not expected: every contour starts at a vertex of the arrangement. Fall back to
                // a pass per set for this batch.
                for m in (0..sets.len()).filter(|&m| batch[m] == b) {
                    let c = top.contours(|w| id(w) == Some(m));
                    faces[m].extend(c.grouped().into_iter().map(|g| self.outline(&c, g)));
                }
                continue;
            };
            // A contour belongs under the nearest enclosing contour of the same set, as in that
            // set's own pass.
            let mut children = vec![Vec::new(); owner.len()];
            let mut roots = Vec::new();
            for (i, &m) in owner.iter().enumerate() {
                let mut up = c[ContourIdx(i)].parent;
                while let Some(p) = up.filter(|p| owner.get(p.0) != Some(&m)) {
                    up = c[p].parent;
                }
                match up {
                    Some(p) => children[p.0].push(i),
                    None => roots.push(i),
                }
            }
            for r in roots {
                let mut group = Vec::new();
                let mut stack = vec![r];
                while let Some(i) = stack.pop() {
                    group.push(ContourIdx(i));
                    stack.extend(children[i].iter().rev());
                }
                faces[owner[r]].push(self.outline(&c, group));
            }
        }
        let mut out = Vec::new();
        for (sources, paths) in sets.into_iter().zip(faces) {
            for path in paths.into_iter().filter(|p| !p.elements().is_empty()) {
                let area = path.area().abs();
                out.push(Region { path, area, sources: sources.clone() });
            }
        }
        out
    }

    /// The contours `idx` of `c` as one path, repaired, without slivers, and with the pieces the
    /// sweep split from one input curve or line joined again.
    fn outline(&self, c: &Contours, idx: impl IntoIterator<Item = ContourIdx>) -> BezPath {
        let mut out = BezPath::new();
        for i in idx {
            let path = self.tidy.repair(&c[i].path);
            if is_sliver(&path, self.tidy.sliver) {
                continue;
            }
            let segs = tidy_segments(path.segments().map(to_seg).collect(), &self.tidy);
            let Some(first) = segs.first() else { continue };
            out.move_to(first.c.p0);
            for s in &segs {
                if s.line {
                    out.line_to(s.c.p3);
                } else {
                    out.curve_to(s.c.p1, s.c.p2, s.c.p3);
                }
            }
            out.close_path();
        }
        out
    }
}

/// A point as a map key (the arrangement's vertices are exact).
fn vertex_key(p: Point) -> (u64, u64) {
    (p.x.to_bits(), p.y.to_bits())
}

/// `bp` with every subpath closed.
fn closed(bp: &BezPath) -> BezPath {
    let mut out = BezPath::new();
    let (mut open, mut start) = (false, Point::ZERO);
    for el in bp.iter() {
        match el {
            PathEl::MoveTo(p) => {
                if open {
                    out.close_path();
                }
                out.move_to(p);
                (open, start) = (true, p);
            }
            PathEl::ClosePath => {
                if open {
                    out.close_path();
                }
                open = false;
            }
            _ => {
                // Drawing on after a close starts again at the subpath's start.
                if !open {
                    out.move_to(start);
                    open = true;
                }
                out.push(el);
            }
        }
    }
    if open {
        out.close_path();
    }
    out
}

/// The sweep tolerance for a set of paths (linesweeper's own `binary_op` scaling).
fn eps_for<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Result<f64, GeomError> {
    let mut m: f64 = 0.0;
    for p in paths {
        for el in p.elements() {
            for pt in el_points(el) {
                if !pt.x.is_finite() || !pt.y.is_finite() {
                    return Err(GeomError::NonFinite);
                }
                m = m.max(pt.x.abs()).max(pt.y.abs());
            }
        }
    }
    Ok((m * f64::EPSILON * 64.0).max(1e-6))
}

fn el_points(el: &PathEl) -> impl Iterator<Item = Point> {
    let v: [Option<Point>; 3] = match *el {
        PathEl::MoveTo(p) | PathEl::LineTo(p) => [Some(p), None, None],
        PathEl::QuadTo(a, b) => [Some(a), Some(b), None],
        PathEl::CurveTo(a, b, c) => [Some(a), Some(b), Some(c)],
        PathEl::ClosePath => [None, None, None],
    };
    v.into_iter().flatten()
}

/// Make edges that rise by less than the sweep tolerance `eps` exactly horizontal.
///
/// The sweep orders a curve against another segment over a y-span shorter than `eps` by bounding
/// boxes alone, so an almost-horizontal edge can be mis-ordered against a curve it passes and a
/// corner is lost; exact horizontals are handled exactly. So on-curve y coordinates within `eps`
/// of each other snap to one value (the lowest of each cluster, keeping distinct values at least
/// `eps` apart), handles move with their anchors, and a handle within `eps` of its anchor's
/// height is leveled with it. Anchors move by less than `eps` and handles by less than `2·eps`.
fn snap_horizontals(paths: &mut [&mut BezPath], eps: f64) {
    let mut ys: Vec<f64> = paths.iter().flat_map(|p| p.elements().iter().filter_map(PathEl::end_point)).map(|p| p.y).collect();
    ys.sort_by(f64::total_cmp);
    ys.dedup();
    let mut start = f64::NEG_INFINITY;
    let levels: Vec<f64> = ys
        .iter()
        .map(|&y| {
            if y - start >= eps {
                start = y;
            }
            start
        })
        .collect();
    let snap = |p: &mut Point| {
        let y = levels.get(ys.partition_point(|&v| v < p.y)).copied().unwrap_or(p.y);
        let moved = y - p.y;
        p.y = y;
        moved
    };
    // Shift a handle with its anchor, then level it with the anchor when within `eps`.
    let follow = |h: &mut Point, anchor: Point, moved: f64| {
        h.y += moved;
        if (h.y - anchor.y).abs() < eps {
            h.y = anchor.y;
        }
    };
    for p in paths.iter_mut() {
        // The current point (snapped), how far snapping moved it, and the subpath's start.
        let (mut cur, mut moved, mut first) = (Point::ZERO, 0.0, Point::ZERO);
        for el in p.elements_mut() {
            match el {
                PathEl::MoveTo(q) => {
                    moved = snap(q);
                    (cur, first) = (*q, *q);
                }
                PathEl::LineTo(q) => {
                    moved = snap(q);
                    cur = *q;
                }
                PathEl::QuadTo(c, q) => {
                    let m = snap(q);
                    follow(c, cur, moved);
                    follow(c, *q, 0.0);
                    (cur, moved) = (*q, m);
                }
                PathEl::CurveTo(c1, c2, q) => {
                    let m = snap(q);
                    follow(c1, cur, moved);
                    follow(c2, *q, m);
                    (cur, moved) = (*q, m);
                }
                PathEl::ClosePath => {
                    (cur, moved) = (first, 0.0);
                }
            }
        }
    }
}

/// Contours whose mean width (2 × area / perimeter) is below `sliver` are debris.
fn is_sliver(bp: &BezPath, sliver: f64) -> bool {
    let a = bp.area().abs();
    let per = bp.perimeter(1e-6);
    per <= 0.0 || 2.0 * a / per < sliver
}

/// One segment of a contour: a cubic, and whether it is a straight line.
#[derive(Clone, Copy, Debug)]
struct Seg {
    c: CubicBez,
    line: bool,
}

impl Seg {
    fn line(a: Point, b: Point) -> Self {
        Self { c: CubicBez::new(a, a, b, b), line: true }
    }
}

fn to_seg(s: PathSeg) -> Seg {
    match s {
        PathSeg::Line(l) => Seg::line(l.p0, l.p1),
        PathSeg::Quad(q) => {
            let c = q.raise();
            Seg { c, line: is_straight(&c, 1e-9) }
        }
        PathSeg::Cubic(c) => Seg { c, line: is_straight(&c, 1e-9 * (1.0 + c.p0.distance(c.p3))) },
    }
}

/// Whether the cubic is (within `tol`) a straight line with its handles inside the chord.
fn is_straight(c: &CubicBez, tol: f64) -> bool {
    let chord = c.p3 - c.p0;
    let len = chord.hypot();
    if len < 1e-12 {
        return c.p1.distance(c.p0) <= tol && c.p2.distance(c.p0) <= tol;
    }
    let dir = chord / len;
    [c.p1, c.p2].iter().all(|&h| {
        let v = h - c.p0;
        let along = v.dot(dir);
        v.cross(dir).abs() <= tol && along >= -tol && along <= len + tol
    })
}

fn unit(v: Vec2) -> Option<Vec2> {
    let l = v.hypot();
    (l > 1e-12 && l.is_finite()).then(|| v / l)
}

/// The direction the curve leaves `p0`.
fn start_tangent(c: &CubicBez) -> Vec2 {
    unit(c.p1 - c.p0).or_else(|| unit(c.p2 - c.p0)).or_else(|| unit(c.p3 - c.p0)).unwrap_or(Vec2::new(1.0, 0.0))
}

/// The direction the curve arrives at `p3`.
fn end_tangent(c: &CubicBez) -> Vec2 {
    unit(c.p3 - c.p2).or_else(|| unit(c.p3 - c.p1)).or_else(|| unit(c.p3 - c.p0)).unwrap_or(Vec2::new(1.0, 0.0))
}

/// The distance two points must be within to count as one, at `p`'s magnitude.
fn tol_at(p: Point) -> f64 {
    1e-6 * (1.0 + p.x.abs().max(p.y.abs()))
}

/// Output cleanup: the inputs' on-curve points (joints there stay), and their curves, to rebuild
/// split pieces exactly.
struct Tidy {
    sliver: f64,
    /// Sorted by x.
    keep: Vec<Point>,
    /// Input curves with slightly inflated bounds.
    curves: Vec<(Rect, CubicBez)>,
    /// `curves` indices by the left edge of their bounds, and the widest bounds: the curves whose
    /// bounds can hold a point, without testing them all.
    by_x: Vec<usize>,
    max_width: f64,
}

impl Tidy {
    fn keeping<'a>(sliver: f64, paths: impl IntoIterator<Item = &'a BezPath>) -> Self {
        let mut keep = Vec::new();
        let mut curves = Vec::new();
        for p in paths {
            for s in p.segments() {
                let seg = to_seg(s);
                keep.push(seg.c.p0);
                if !seg.line {
                    curves.push((seg.c.bounding_box().inflate(1e-6, 1e-6), seg.c));
                }
            }
        }
        keep.sort_by(|a, b| a.x.total_cmp(&b.x));
        let mut by_x: Vec<usize> = (0..curves.len()).collect();
        by_x.sort_by(|&i, &j| curves[i].0.x0.total_cmp(&curves[j].0.x0));
        let max_width = curves.iter().map(|(r, _)| r.width()).fold(0.0, f64::max);
        Self { sliver, keep, curves, by_x, max_width }
    }

    fn is_kept(&self, p: Point) -> bool {
        let tol = tol_at(p);
        let lo = self.keep.partition_point(|q| q.x < p.x - tol);
        self.keep[lo..].iter().take_while(|q| q.x <= p.x + tol).any(|q| (q.y - p.y).abs() <= tol)
    }

    /// The input curves whose bounds hold both `a` and `b`.
    fn curves_around(&self, a: Point, b: Point) -> impl Iterator<Item = &CubicBez> {
        let x = a.x.min(b.x);
        let hi = self.by_x.partition_point(|&i| self.curves[i].0.x0 <= x);
        let lo = self.by_x[..hi].partition_point(|&i| self.curves[i].0.x0 < x - self.max_width);
        self.by_x[lo..hi].iter().map(|&i| &self.curves[i]).filter(move |(r, _)| r.contains(a) && r.contains(b)).map(|(_, c)| c)
    }

    /// `bp` with each curve piece that strays from the one input curve through both its ends
    /// rebuilt as that curve's exact sub-curve. linesweeper 0.5 can emit a piece that doesn't
    /// follow its curve; out and back between two points of one curve, such pieces make a lens
    /// instead of nothing (VectorCraft issue #1028). A piece on a curve, or whose ends two
    /// different input curves pass through (a real lens between them), is left as it is.
    fn repair(&self, bp: &BezPath) -> BezPath {
        if self.curves.is_empty() {
            return bp.clone();
        }
        let mut out = BezPath::new();
        let (mut cur, mut start) = (Point::ZERO, Point::ZERO);
        for el in bp.iter() {
            let piece = match el {
                PathEl::QuadTo(c, p) => Some(kurbo::QuadBez::new(cur, c, p).raise()),
                PathEl::CurveTo(c1, c2, p) => Some(CubicBez::new(cur, c1, c2, p)),
                _ => None,
            };
            match (el, piece) {
                (_, Some(raw)) => {
                    let (a, b) = (raw.p0, raw.p3);
                    let tol = tol_at(Point::new(a.x.abs().max(b.x.abs()), a.y.abs().max(b.y.abs())));
                    let mut subs: Vec<CubicBez> = Vec::new();
                    let mut follows = false;
                    for c in self.curves_around(a, b) {
                        let (na, nb) = (c.nearest(a, 1e-12), c.nearest(b, 1e-12));
                        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol || (na.t - nb.t).abs() < 1e-12 {
                            continue;
                        }
                        if [0.25, 0.5, 0.75].iter().all(|&t| c.nearest(raw.eval(t), 1e-12).distance_sq.sqrt() <= tol * 10.0) {
                            follows = true;
                            break;
                        }
                        subs.push(sub_curve(c, na.t, nb.t, a, b));
                    }
                    let same = |p: &CubicBez, q: &CubicBez| p.p1.distance(q.p1) <= tol * 10.0 && p.p2.distance(q.p2) <= tol * 10.0;
                    match subs.split_first() {
                        Some((sub, rest)) if !follows && rest.iter().all(|q| same(sub, q)) => out.curve_to(sub.p1, sub.p2, b),
                        _ => out.push(el),
                    }
                    cur = b;
                }
                (PathEl::MoveTo(p), None) => {
                    out.push(el);
                    (cur, start) = (p, p);
                }
                (PathEl::LineTo(p), None) => {
                    out.push(el);
                    cur = p;
                }
                (PathEl::ClosePath, None) => {
                    out.push(el);
                    cur = start;
                }
                (_, None) => out.push(el),
            }
        }
        out
    }
}

/// The part of `c` from parameter `ta` to `tb` (either order), with its ends at `a` and `b`.
fn sub_curve(c: &CubicBez, ta: f64, tb: f64, a: Point, b: Point) -> CubicBez {
    let sub = if ta < tb {
        c.subsegment(ta..tb)
    } else {
        let r = c.subsegment(tb..ta);
        CubicBez::new(r.p3, r.p2, r.p1, r.p0)
    };
    CubicBez::new(a, sub.p1, sub.p2, b)
}

/// The closed contour `segs` with the pieces the sweep split from one input line or curve joined
/// again, and degenerate pieces dropped. Joints at input points stay. Only exact joins are made,
/// so neighboring regions keep the same edges.
fn tidy_segments(segs: Vec<Seg>, tidy: &Tidy) -> Vec<Seg> {
    let scale = segs.iter().map(|s| s.c.p0.distance(s.c.p3)).fold(0.0, f64::max);
    let tiny = (scale * 1e-9).max(1e-9);
    let mut segs: Vec<Seg> = segs
        .into_iter()
        .filter(|s| {
            let r = Rect::from_points(s.c.p0, s.c.p3).union_pt(s.c.p1).union_pt(s.c.p2);
            r.width() + r.height() > tiny
        })
        .collect();
    let n = segs.len();
    if n < 2 {
        return segs;
    }
    // Joint i sits at the start of segment i.
    let mergeable = |a: &Seg, b: &Seg| -> bool {
        if a.line != b.line || tidy.is_kept(b.c.p0) {
            return false;
        }
        let ta = end_tangent(&a.c);
        let tb = start_tangent(&b.c);
        ta.dot(tb) > 0.9998 && ta.cross(tb).abs() < 0.02
    };
    // Start at a real corner, if there is one.
    if let Some(k) = (0..n).find(|&i| !mergeable(&segs[(i + n - 1) % n], &segs[i])) {
        segs.rotate_left(k);
    }
    let mut out: Vec<Seg> = Vec::with_capacity(n);
    let mut i = 0;
    while i < n {
        let mut cur = segs[i];
        let mut j = i + 1;
        while j < n && mergeable(&segs[j - 1], &segs[j]) {
            match join(&segs[i..=j], tidy) {
                Some(m) => {
                    cur = m;
                    j += 1;
                }
                None => break,
            }
        }
        out.push(cur);
        i = j;
    }
    out
}

/// `run` as one segment, when its pieces lie on one line or one input curve.
fn join(run: &[Seg], tidy: &Tidy) -> Option<Seg> {
    let (first, last) = (run.first()?.c, run.last()?.c);
    let (a, b) = (first.p0, last.p3);
    let tol = tol_at(Point::new(a.x.abs().max(b.x.abs()), a.y.abs().max(b.y.abs())));
    if run[0].line {
        let chord = b - a;
        let len = chord.hypot();
        if len < 1e-12 {
            return None;
        }
        let ok = run.iter().all(|s| ((s.c.p3 - a).cross(chord) / len).abs() <= tol);
        return ok.then(|| Seg::line(a, b));
    }
    for c in tidy.curves_around(a, b) {
        let (na, nb) = (c.nearest(a, 1e-12), c.nearest(b, 1e-12));
        if na.distance_sq.sqrt() > tol || nb.distance_sq.sqrt() > tol || (na.t - nb.t).abs() < 1e-12 {
            continue;
        }
        if run.iter().all(|s| c.nearest(s.c.eval(0.5), 1e-12).distance_sq.sqrt() <= tol * 10.0) {
            return Some(Seg { c: sub_curve(c, na.t, nb.t, a, b), line: false });
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use kurbo::Circle;

    use super::*;

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> BezPath {
        Rect::new(x0, y0, x1, y1).to_path(0.1)
    }

    fn fill(path: BezPath) -> Shape {
        Shape::new(path, FillRule::NonZero)
    }

    /// The regions with their sources and areas rounded to 1e-6, sorted.
    fn summary(regions: &[Region]) -> Vec<(Vec<usize>, f64)> {
        let mut v: Vec<(Vec<usize>, f64)> = regions.iter().map(|r| (r.sources.clone(), (r.area * 1e6).round() / 1e6)).collect();
        v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
        v
    }

    /// Each grid point inside the union is in exactly one region, whose sources are the shapes
    /// holding the point.
    fn assert_tiles(shapes: &[Shape], regions: &[Region], area: Rect, step: f64) {
        let mut y = area.y0 + step * 0.37;
        while y < area.y1 {
            let mut x = area.x0 + step * 0.41;
            while x < area.x1 {
                let p = Point::new(x, y);
                let covering: Vec<usize> = shapes.iter().enumerate().filter(|(_, s)| s.rule.inside(s.path.winding(p))).map(|(i, _)| i).collect();
                let hits: Vec<&Region> = regions.iter().filter(|r| r.contains(p)).collect();
                if covering.is_empty() {
                    assert!(hits.is_empty(), "{p:?} is in no shape but in {} regions", hits.len());
                } else {
                    assert_eq!(hits.len(), 1, "{p:?} is in {} regions", hits.len());
                    assert_eq!(hits[0].sources, covering, "at {p:?}");
                }
                x += step;
            }
            y += step;
        }
    }

    #[test]
    fn two_overlapping_rectangles_make_three_regions() {
        let shapes = [fill(rect(0.0, 0.0, 10.0, 10.0)), fill(rect(5.0, 5.0, 15.0, 15.0))];
        let r = regions(&shapes, &[]).unwrap();
        assert_eq!(summary(&r), vec![(vec![0], 75.0), (vec![0, 1], 25.0), (vec![1], 75.0)]);
        assert_tiles(&shapes, &r, Rect::new(-1.0, -1.0, 16.0, 16.0), 0.5);
        // The pieces the sweep split rectangle edges into are joined again.
        let overlap = r.iter().find(|r| r.sources == [0, 1]).unwrap();
        assert_eq!(overlap.path.segments().count(), 4);
        assert_eq!(overlap.top(), Some(1));
    }

    #[test]
    fn coincident_edges_make_no_slivers() {
        // Side by side, sharing an edge.
        let shapes = [fill(rect(0.0, 0.0, 10.0, 10.0)), fill(rect(10.0, 0.0, 20.0, 10.0))];
        assert_eq!(summary(&regions(&shapes, &[]).unwrap()), vec![(vec![0], 100.0), (vec![1], 100.0)]);
        // Sharing three edges.
        let shapes = [fill(rect(0.0, 0.0, 10.0, 10.0)), fill(rect(0.0, 0.0, 10.0, 5.0))];
        assert_eq!(summary(&regions(&shapes, &[]).unwrap()), vec![(vec![0], 50.0), (vec![0, 1], 50.0)]);
        // The same circle twice, one reversed.
        let circle = Circle::new((5.0, 5.0), 5.0).to_path(0.01);
        let shapes = [fill(circle.clone()), fill(circle.reverse_subpaths())];
        let r = regions(&shapes, &[]).unwrap();
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].sources, [0, 1]);
        assert!((r[0].area - circle.area().abs()).abs() < 1e-9);
    }

    #[test]
    fn fill_rules_differ_on_self_overlap() {
        // Two squares in one path, wound the same way: the overlap has winding 2.
        let mut path = rect(0.0, 0.0, 10.0, 10.0);
        path.extend(rect(5.0, 5.0, 15.0, 15.0));
        let center = Point::new(7.5, 7.5);
        let nonzero = regions(&[Shape::new(path.clone(), FillRule::NonZero)], &[]).unwrap();
        assert_eq!(summary(&nonzero), vec![(vec![0], 175.0)]);
        assert!(nonzero[0].contains(center));
        let shapes = [Shape::new(path, FillRule::EvenOdd)];
        let evenodd = regions(&shapes, &[]).unwrap();
        assert!((evenodd.iter().map(|r| r.area).sum::<f64>() - 150.0).abs() < 1e-9);
        assert!(evenodd.iter().all(|r| r.sources == [0] && !r.contains(center)));
        assert_tiles(&shapes, &evenodd, Rect::new(-1.0, -1.0, 16.0, 16.0), 0.5);
    }

    #[test]
    fn a_hole_is_part_of_its_region_and_a_shape_in_it_is_its_own() {
        // Nonzero hole by reversal, even-odd hole by nesting.
        let mut nonzero = rect(0.0, 0.0, 10.0, 10.0);
        nonzero.extend(rect(3.0, 3.0, 7.0, 7.0).reverse_subpaths());
        let mut evenodd = rect(0.0, 0.0, 10.0, 10.0);
        evenodd.extend(rect(3.0, 3.0, 7.0, 7.0));
        for donut in [Shape::new(nonzero, FillRule::NonZero), Shape::new(evenodd, FillRule::EvenOdd)] {
            let shapes = [donut, fill(rect(4.0, 4.0, 6.0, 6.0))];
            let r = regions(&shapes, &[]).unwrap();
            assert_eq!(summary(&r), vec![(vec![0], 84.0), (vec![1], 4.0)]);
            let ring = r.iter().find(|r| r.sources == [0]).unwrap();
            assert_eq!(ring.path.elements().iter().filter(|e| matches!(e, PathEl::MoveTo(_))).count(), 2);
            assert!(!ring.contains(Point::new(3.5, 3.5)));
            assert_tiles(&shapes, &r, Rect::new(-1.0, -1.0, 11.0, 11.0), 0.25);
        }
    }

    #[test]
    fn slivers_below_the_threshold_are_dropped() {
        // Overlapping by 0.001.
        let shapes = [fill(rect(0.0, 0.0, 10.0, 10.0)), fill(rect(9.999, 0.0, 20.0, 10.0))];
        let r = regions(&shapes, &[]).unwrap();
        assert!(r.iter().all(|r| r.sources != [0, 1]));
        assert_eq!(r.len(), 2);
        let kept = regions_with(&shapes, &[], 0.0).unwrap();
        let sliver = kept.iter().find(|r| r.sources == [0, 1]).unwrap();
        assert!((sliver.area - 0.01).abs() < 1e-9, "{}", sliver.area);
    }

    #[test]
    fn region_areas_sum_to_the_union_area() {
        let shapes = [
            fill(rect(0.0, 0.0, 40.0, 30.0)),
            fill(Circle::new((35.0, 25.0), 12.0).to_path(0.01)),
            fill(rect(10.0, 10.0, 60.0, 20.0)),
            Shape::new(Circle::new((20.0, 35.0), 8.0).to_path(0.01), FillRule::EvenOdd),
            fill(rect(70.0, 0.0, 80.0, 10.0)),
        ];
        let r = regions(&shapes, &[]).unwrap();
        let sum: f64 = r.iter().map(|r| r.area).sum();
        let union = union_area(&shapes);
        assert!((sum - union).abs() < 1e-6 * union, "{sum} vs {union}");
        assert_tiles(&shapes, &r, Rect::new(-1.0, -1.0, 81.0, 44.0), 0.7);
    }

    /// The area of the union of `shapes` (all positively wound), from a sweep of them as one path.
    fn union_area(shapes: &[Shape]) -> f64 {
        let mut all = BezPath::new();
        for s in shapes {
            all.extend(s.path.iter());
        }
        let top = Topology::<i32>::from_path(&all, 1e-6).unwrap();
        top.contours(|w| *w != 0).contours().map(|c| c.path.area()).sum::<f64>().abs()
    }

    #[test]
    fn clips_limit_their_shapes_only() {
        let clips = [ClipPath { path: rect(5.0, 0.0, 25.0, 10.0), rule: FillRule::NonZero }];
        let shapes = [fill(rect(0.0, 0.0, 10.0, 10.0)).clipped([0]), fill(rect(20.0, 0.0, 30.0, 10.0))];
        let r = regions(&shapes, &clips).unwrap();
        // The clip's edge at x = 25 crosses only the unclipped shape, which stays one region.
        assert_eq!(summary(&r), vec![(vec![0], 50.0), (vec![1], 100.0)]);
        assert!(r.iter().any(|r| r.sources == [0] && r.contains(Point::new(7.0, 5.0))));
        assert_eq!(regions(&[fill(rect(0.0, 0.0, 1.0, 1.0)).clipped([3])], &clips), Err(GeomError::UnknownClip { shape: 0, clip: 3 }));
    }

    #[test]
    fn open_and_non_finite_input() {
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        open.line_to((10.0, 0.0));
        open.line_to((10.0, 10.0));
        assert_eq!(summary(&regions(&[fill(open)], &[]).unwrap()), vec![(vec![0], 50.0)]);
        assert_eq!(regions(&[fill(rect(0.0, 0.0, f64::NAN, 1.0))], &[]), Err(GeomError::NonFinite));
        assert_eq!(regions(&[], &[]), Ok(vec![]));
    }

    #[test]
    fn only_groups_with_transparency_need_regions() {
        let bounds = [
            Rect::new(0.0, 0.0, 10.0, 10.0),
            Rect::new(100.0, 100.0, 110.0, 110.0),
            Rect::new(10.0, 5.0, 20.0, 15.0), // touches 0
            Rect::new(18.0, 14.0, 30.0, 30.0),
            Rect::new(200.0, 0.0, 210.0, 10.0),
        ];
        assert_eq!(overlap_groups(&bounds), vec![vec![0, 2, 3], vec![1], vec![4]]);
        assert_eq!(transparent_groups(&bounds, &[false, false, false, true, false]), vec![vec![0, 2, 3]]);
        assert!(transparent_groups(&bounds, &[false; 5]).is_empty());
    }

    /// About 1,000 rectangles on a letter page. Run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn timing_on_a_thousand_rectangles() {
        let mut seed: u64 = 0x2545_f491_4f6c_dd1d;
        let mut next = || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            (seed >> 11) as f64 / (1u64 << 53) as f64
        };
        let shapes: Vec<Shape> = (0..1000)
            .map(|_| {
                let (x, y) = (next() * 560.0, next() * 740.0);
                let (w, h) = (2.0 + next() * 50.0, 2.0 + next() * 50.0);
                fill(rect(x, y, x + w, y + h))
            })
            .collect();
        let t = std::time::Instant::now();
        let r = regions(&shapes, &[]).unwrap();
        let elapsed = t.elapsed();
        eprintln!("{} shapes -> {} regions in {:?}", shapes.len(), r.len(), elapsed);
        let union = union_area(&shapes);
        let sum: f64 = r.iter().map(|r| r.area).sum();
        // Less the slivers left out.
        assert!(sum <= union * (1.0 + 1e-9) && sum > union * (1.0 - 1e-4), "{sum} vs {union}");
    }
}
