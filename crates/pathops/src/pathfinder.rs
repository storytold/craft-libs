// Carved from VectorCraft's crates/pathops (src/pathfinder.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! The Pathfinder operations and planar regions, over an ordered stack of shapes (index 0 =
//! back-most, last = front-most).

use std::collections::HashMap;

use kurbo::{BezPath, ParamCurve, ParamCurveNearest, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours};

use crate::FillRule;
use crate::boolean::{Arrangement, DEFAULT_PRECISION, Multi, all_contours_to_path, contours_to_path, unite_all, unite_all_with};
use crate::path::{Seg, control_bounds, fill_bezpath, guarded, segs_to_bezpath, subs};

/// A filled shape in a Pathfinder stack. `key` identifies its paint (e.g. a hashed fill colour, or
/// the index of the caller's object); results carry the key of the object whose paint they keep.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub path: BezPath,
    pub rule: FillRule,
    pub key: u64,
}

impl Shape {
    pub fn new(path: BezPath, rule: FillRule, key: u64) -> Self {
        Self { path, rule, key }
    }
}

/// Pathfinder operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PathfinderOp {
    /// Shape mode: union of everything; result keeps the front-most key.
    Unite,
    /// Shape mode: back-most minus everything in front; keeps the back-most key.
    MinusFront,
    /// Shape mode: area covered by *all* shapes; keeps the front-most key.
    Intersect,
    /// Shape mode: area covered by an odd number of shapes; keeps the front-most key.
    Exclude,
    /// Split into every non-overlapping face; each face keeps the key of the front-most shape
    /// covering it.
    Divide,
    /// Remove hidden parts; one result per shape that remains visible (no merging).
    Trim,
    /// Trim, then merge touching/overlapping visible parts with the same key.
    Merge,
    /// Keep the visible parts of lower shapes that lie inside the front-most shape (which is removed).
    Crop,
    /// Split every edge at intersections; returns *open* paths keyed by the shape they bound.
    Outline,
    /// Front-most minus everything behind; keeps the front-most key.
    MinusBack,
}

/// One face of the planar arrangement of a set of shapes.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Face outline (outer contour plus holes, consistently oriented).
    pub path: BezPath,
    /// Indices of the input shapes covering this face, ascending.
    pub sources: Vec<usize>,
}

impl Region {
    /// Front-most covering shape: `None` for a face nothing fills, which [`regions`] never gives but
    /// [`live_paint`](crate::live_paint) and [`shape_builder`](crate::shape_builder) do (the area
    /// enclosed by paths, with an empty `sources`).
    pub fn top(&self) -> Option<usize> {
        self.sources.last().copied()
    }
    /// Does the face contain `p`?
    pub fn contains(&self, p: Point) -> bool {
        self.path.winding(p) != 0
    }
}

/// The arrangement of `shapes`, its faces refit to `precision` and its slivers thinner than it dropped.
fn arrangement(shapes: &[Shape], precision: f64) -> Option<Arrangement> {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return None;
    }
    let bps: Vec<(BezPath, FillRule)> = shapes.iter().map(|s| (fill_bezpath(&s.path), s.rule)).collect();
    let mut arr = Arrangement::new(bps).ok()?;
    arr.tidy.precision = precision;
    Some(arr)
}

/// The one item of `it`, if it has exactly one.
fn only(mut it: impl Iterator<Item = usize>) -> Option<usize> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

fn one(path: BezPath, key: u64) -> Vec<Shape> {
    if path.elements().is_empty() { Vec::new() } else { vec![Shape::new(path, FillRule::NonZero, key)] }
}

/// Run a Pathfinder operation over `shapes` (back → front). Shapes that cannot be swept (their
/// coordinates are not finite or are too large, or they hold more than [`crate::MAX_ELEMENTS`]
/// elements) give no result.
pub fn pathfinder(op: PathfinderOp, shapes: &[Shape]) -> Vec<Shape> {
    pathfinder_with(op, shapes, DEFAULT_PRECISION)
}

/// [`pathfinder`] with a refit `precision`: results are refit to within it (in path units) and
/// pieces thinner than it are dropped. A smaller value follows the inputs more closely. A value
/// that isn't positive and finite is [`DEFAULT_PRECISION`].
pub fn pathfinder_with(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Vec<Shape> {
    // A panic in the sweep, which can happen on rare near-degenerate input, is no result.
    guarded(|| pathfinder_unguarded(op, shapes, precision)).unwrap_or_default()
}

fn pathfinder_unguarded(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Vec<Shape> {
    let precision = if precision.is_finite() && precision > 0.0 { precision } else { DEFAULT_PRECISION };
    let n = shapes.len();
    // No shapes, or more segments than are read: no result.
    if n == 0 || !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return Vec::new();
    }
    let front_key = shapes[n - 1].key;
    let unite = || {
        let v: Vec<(&BezPath, FillRule)> = shapes.iter().map(|s| (&s.path, s.rule)).collect();
        one(unite_all_with(&v, precision), front_key)
    };
    // Unite doesn't need the arrangement.
    if op == PathfinderOp::Unite {
        return unite();
    }
    // Open paths are cutting edges for Divide; the filled-only operations keep their
    // implicit closure. Reuse the planar map of `shape_builder` and its bound and fallback.
    if op == PathfinderOp::Divide && crate::has_open_path(shapes) {
        return crate::planar::builder(shapes, false, precision)
            .regions
            .into_iter()
            .filter_map(|r| {
                let key = shapes.get(r.top()?)?.key;
                Some(Shape::new(r.path, FillRule::NonZero, key))
            })
            .collect();
    }
    let Some(arr) = arrangement(shapes, precision) else { return Vec::new() };
    let p = &arr.tidy;
    match op {
        PathfinderOp::Unite => unite(),
        // The tests read the covering inputs straight off the windings ([`Arrangement::covered`]);
        // each is the coverage-mask test in its comment.
        // m[0] && !m[1..].any()
        PathfinderOp::MinusFront => one(all_contours_to_path(&arr.contours_where(|w| only(arr.covered(w)) == Some(0)), p), shapes[0].key),
        // m[n - 1] && !m[..n - 1].any()
        PathfinderOp::MinusBack => one(all_contours_to_path(&arr.contours_where(|w| only(arr.covered(w)) == Some(n - 1)), p), front_key),
        // m.all()
        PathfinderOp::Intersect => one(all_contours_to_path(&arr.contours_where(|w| arr.covered(w).count() == n), p), front_key),
        // m.count() is odd
        PathfinderOp::Exclude => one(all_contours_to_path(&arr.contours_where(|w| arr.covered(w).count() % 2 == 1), p), front_key),
        PathfinderOp::Divide => regions_of(&arr)
            .into_iter()
            .filter_map(|r| {
                let key = shapes.get(r.top()?)?.key;
                Some(Shape::new(r.path, FillRule::NonZero, key))
            })
            .collect(),
        // topmost(m) == Some(i)
        PathfinderOp::Trim => {
            let mut paths = vec![BezPath::new(); n];
            each_classified_face(
                &arr,
                n,
                |w| arr.covered(w).next_back(),
                |i, c, group| {
                    if let Some(sp) = paths.get_mut(i) {
                        sp.extend(contours_to_path(c, group.iter().copied(), &arr.tidy).iter());
                    }
                },
            );
            paths.into_iter().zip(shapes).flat_map(|(sp, s)| one(sp, s.key)).collect()
        }
        PathfinderOp::Merge => {
            // The distinct keys, in the order they first appear, each with a class number.
            let mut class_of_key: HashMap<u64, usize> = HashMap::new();
            let mut keys: Vec<u64> = Vec::new();
            for s in shapes {
                class_of_key.entry(s.key).or_insert_with(|| {
                    keys.push(s.key);
                    keys.len() - 1
                });
            }
            let class_of_shape: Vec<usize> = shapes.iter().filter_map(|s| class_of_key.get(&s.key).copied()).collect();
            // topmost(m) is a shape with key k: one contour pass for every batch of keys whose
            // faces share no vertex, not one for each key.
            let mut parts: Vec<(usize, BezPath)> = Vec::new();
            each_classified_face(
                &arr,
                keys.len(),
                |w| arr.covered(w).next_back().and_then(|t| class_of_shape.get(t).copied()),
                |k, c, group| parts.push((k, contours_to_path(c, group.iter().copied(), p))),
            );
            // By key, as each key's own pass gave them (a stable sort keeps a key's faces in order).
            parts.sort_by_key(|(k, _)| *k);
            parts.into_iter().flat_map(|(k, path)| one(path, keys[k])).collect()
        }
        PathfinderOp::Crop => {
            if n < 2 {
                return Vec::new();
            }
            let mut paths = vec![BezPath::new(); n - 1];
            each_classified_face(
                &arr,
                n - 1,
                |w| {
                    let mut c = arr.covered(w).rev();
                    if c.next() == Some(n - 1) { c.next() } else { None }
                },
                |i, c, group| {
                    if let Some(sp) = paths.get_mut(i) {
                        sp.extend(contours_to_path(c, group.iter().copied(), &arr.tidy).iter());
                    }
                },
            );
            paths.into_iter().zip(shapes).flat_map(|(sp, s)| one(sp, s.key)).collect()
        }
        PathfinderOp::Outline => {
            // Open paths (but for a ring that comes back to where it started) are edges that cross
            // others without a trace in the filled areas: the junctions come from the planar map.
            let open = shapes.iter().any(|s| subs(&s.path).into_iter().map(crate::path::Sub::closed_if_ends_meet).any(|sp| !sp.closed));
            let from_map = open.then(|| crate::planar::map_junctions(shapes, precision)).flatten();
            outline_edges(from_map.unwrap_or_else(|| arr.junctions()), shapes)
        }
    }
}

/// The faces of `arr`, each distinct coverage mask's in turn (sorted), each face an outer contour
/// and the holes of its mask directly inside it ([`face_groups`]).
fn regions_of(arr: &Arrangement) -> Vec<Region> {
    let masks = arr.distinct_masks();
    let refs: Vec<&[bool]> = masks.iter().map(Vec::as_slice).collect();
    let mut faces: Vec<Vec<BezPath>> = vec![Vec::new(); masks.len()];
    each_face(arr, &refs, |m, c, group| {
        if let Some(f) = faces.get_mut(m) {
            f.push(contours_to_path(c, group.iter().copied(), &arr.tidy));
        }
    });
    let mut out = Vec::new();
    for (mask, paths) in masks.iter().zip(faces) {
        let sources: Vec<usize> = mask.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
        out.extend(paths.into_iter().filter(|p| !p.elements().is_empty()).map(|path| Region { path, sources: sources.clone() }));
    }
    out
}

/// Calls `f` with each face of `arr` classified by `classify: impl Fn(&Multi) -> Option<usize>`
/// into `0..num_classes`: the index of its class, the contour pass it is in, and its contours,
/// an outer contour and the holes of its class directly inside it ([`face_groups`]).
///
/// A contour pass walks the whole arrangement, so one pass per class would cost classes × edges.
/// Instead the classes go into batches whose faces share no vertex (so no edge either): one pass
/// per batch walks each face's contours exactly as its own class's pass would, and the class a
/// contour bounds is the one of its batch at its first point.
fn each_classified_face(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    mut f: impl FnMut(usize, &Contours, &[ContourIdx]),
) {
    if num_classes == 0 {
        return;
    }
    let top = &arr.top;
    // The classes of the faces around each vertex.
    let mut around: HashMap<(u64, u64), Vec<usize>> = HashMap::new();
    for s in top.segment_indices() {
        let h = s.first_half();
        let sides = [classify(top.winding_clockwise(h)), classify(top.winding_counter_clockwise(h))];
        for end in [h, s.second_half()] {
            let at = around.entry(vertex_key(top.point(end).to_kurbo())).or_default();
            for m in sides.into_iter().flatten() {
                if !at.contains(&m) {
                    at.push(m);
                }
            }
        }
    }
    let mut near = vec![Vec::new(); num_classes];
    for at in around.values() {
        for &a in at {
            near[a].extend(at.iter().copied().filter(|&b| b != a));
        }
    }
    // Greedy colouring: each class takes the first batch none of its neighbours is in.
    let mut batch = vec![usize::MAX; num_classes];
    let mut batches = 0;
    for m in 0..num_classes {
        let mut taken = vec![false; batches];
        for &n in &near[m] {
            if let Some(t) = taken.get_mut(batch[n]) {
                *t = true;
            }
        }
        batch[m] = taken.iter().position(|t| !t).unwrap_or(batches);
        batches = batches.max(batch[m] + 1);
    }
    for b in 0..batches {
        let c = arr.contours_where(|w| classify(w).is_some_and(|m| batch[m] == b));
        let owner: Option<Vec<usize>> = c
            .contours()
            .map(|ct| {
                let Some(kurbo::PathEl::MoveTo(p)) = ct.path.elements().first() else { return None };
                around.get(&vertex_key(*p))?.iter().copied().find(|&m| batch[m] == b)
            })
            .collect();
        let Some(owner) = owner else {
            // Not expected: every contour starts at a vertex of the arrangement. Fall back to a
            // pass per class for this batch.
            for m in (0..num_classes).filter(|m| batch[*m] == b) {
                let c = arr.contours_where(|w| classify(w) == Some(m));
                let owner = vec![m; c.contours().count()];
                for (_, group) in face_groups(&c, &owner) {
                    f(m, &c, &group);
                }
            }
            continue;
        };
        for (m, group) in face_groups(&c, &owner) {
            f(m, &c, &group);
        }
    }
}

/// Calls `f` with each face of `arr` whose coverage is one of `masks` (distinct): the index of
/// its mask in `masks`, the contour pass it is in, and its contours, an outer contour and the
/// holes of its mask directly inside it ([`face_groups`]). Each mask's faces come in the order
/// of that mask's own contour pass.
///
/// A contour pass walks the whole arrangement, so one pass per mask would cost masks × edges
/// (two blends crossing make thousands of each). Instead the masks go into batches whose faces
/// share no vertex (so no edge either): one pass per batch walks each face's contours exactly as
/// its own mask's pass would, and the mask a contour bounds is the one of its batch at its first
/// point.
fn each_face(arr: &Arrangement, masks: &[&[bool]], f: impl FnMut(usize, &Contours, &[ContourIdx])) {
    if masks.is_empty() {
        return;
    }
    // Each mask is found by the inputs it covers.
    let ids = CoverIds::new(masks.iter().map(|m| m.iter().enumerate().filter(|(_, b)| **b).map(|(k, _)| k).collect()).collect());
    each_classified_face(arr, masks.len(), |w| ids.of(arr, w), f);
}

/// Lists of inputs, each ascending as [`Arrangement::covered`] gives them, found by the inputs
/// that cover a winding.
struct CoverIds {
    /// The indices in `lists` of the lists with each hash ([`CoverIds::hash`]).
    by_hash: HashMap<u64, Vec<usize>>,
    lists: Vec<Vec<usize>>,
}

impl CoverIds {
    fn new(lists: Vec<Vec<usize>>) -> Self {
        let mut by_hash: HashMap<u64, Vec<usize>> = HashMap::new();
        for (i, l) in lists.iter().enumerate() {
            by_hash.entry(Self::hash(l.iter().copied())).or_default().push(i);
        }
        Self { by_hash, lists }
    }

    fn hash(inputs: impl Iterator<Item = usize>) -> u64 {
        inputs.fold(0xcbf2_9ce4_8422_2325, |h, i| (h ^ i as u64).wrapping_mul(0x0000_0100_0000_01b3).rotate_left(29))
    }

    /// The index of the list of the inputs that cover winding `w`, if it is one of them.
    fn of(&self, arr: &Arrangement, w: &Multi) -> Option<usize> {
        let same = |&&i: &&usize| self.lists.get(i).is_some_and(|l| arr.covered(w).eq(l.iter().copied()));
        self.by_hash.get(&Self::hash(arr.covered(w)))?.iter().find(same).copied()
    }
}

/// Splits a contour pass into faces: each face is an outer contour plus the holes of the same mask
/// directly inside it (`owner` gives each contour's mask). A hole belongs under the nearest
/// enclosing contour of its mask, as in that mask's own pass; an outer contour always starts a face
/// of its own, even inside a hole of its mask (an island left in a cutout is a separate face, not
/// part of the frame around it).
///
/// The sweep's tree of contours is wrong where it cut a loop off a contour at a point the contour
/// visits twice and the loop isn't nested in it: two holes touching at a point (the second comes
/// out an island inside the first), or an outer contour touching its own hole (it comes out an
/// island inside the hole). Such a loop winds the same way as the contour it was cut from, against
/// its flag ([`Contour::outer`](linesweeper::topology::Contour::outer)), and the pass is then
/// grouped by where the contours lie instead ([`face_groups_by_winding`]).
fn face_groups(c: &Contours, owner: &[usize]) -> Vec<(usize, Vec<ContourIdx>)> {
    let areas: Vec<f64> = c.contours().map(|ct| ct.path.area()).collect();
    if c.contours().zip(&areas).any(|(ct, &a)| ct.outer != (a > 0.0)) {
        return face_groups_by_winding(c, owner, &areas);
    }
    let mut children = vec![Vec::new(); owner.len()];
    let mut roots = Vec::new();
    for (i, &m) in owner.iter().enumerate() {
        let ct = &c[ContourIdx(i)];
        let mut up = if ct.outer { None } else { ct.parent };
        while let Some(p) = up.filter(|p| owner.get(p.0) != Some(&m)) {
            up = c[p].parent;
        }
        match up {
            Some(p) => children[p.0].push(i),
            None => roots.push(i),
        }
    }
    roots
        .into_iter()
        .map(|r| {
            let mut group = Vec::new();
            let mut stack = vec![r];
            while let Some(i) = stack.pop() {
                group.push(ContourIdx(i));
                stack.extend(children[i].iter().rev());
            }
            (owner[r], group)
        })
        .collect()
}

/// [`face_groups`] without the sweep's tree: each hole (a contour whose area in `areas` isn't
/// positive) goes with the smallest outer contour of its mask around it, and each outer contour,
/// or hole with none around it, starts a face. Faces come in the order of their first contour,
/// each with its contours in pass order.
fn face_groups_by_winding(c: &Contours, owner: &[usize], areas: &[f64]) -> Vec<(usize, Vec<ContourIdx>)> {
    let boxes: Vec<kurbo::Rect> = c.contours().map(|ct| ct.path.bounding_box()).collect();
    let outer = |i: usize| areas.get(i).is_some_and(|&a| a > 0.0);
    let mut root: Vec<usize> = (0..owner.len()).collect();
    for (i, ct) in c.contours().enumerate() {
        // Contours of one mask share no edge, so the middle of a hole's first piece is strictly
        // inside or outside each outer contour of its mask.
        let (false, Some(p), Some(b)) = (outer(i), ct.path.segments().next().map(|s| s.eval(0.5)), boxes.get(i)) else {
            continue;
        };
        let around = c
            .contours()
            .enumerate()
            .filter(|&(j, o)| {
                outer(j) && owner.get(j) == owner.get(i) && boxes.get(j).is_some_and(|bj| bj.union(*b) == *bj) && o.path.winding(p) != 0
            })
            .min_by(|(j, _), (k, _)| areas.get(*j).unwrap_or(&f64::INFINITY).total_cmp(areas.get(*k).unwrap_or(&f64::INFINITY)))
            .map(|(j, _)| j);
        if let (Some(j), Some(r)) = (around, root.get_mut(i)) {
            *r = j;
        }
    }
    let mut slot: Vec<Option<usize>> = vec![None; owner.len()];
    let mut groups: Vec<(usize, Vec<ContourIdx>)> = Vec::new();
    for (i, &r) in root.iter().enumerate() {
        let (Some(s), Some(&m)) = (slot.get_mut(r), owner.get(r)) else { continue };
        let k = *s.get_or_insert_with(|| {
            groups.push((m, Vec::new()));
            groups.len() - 1
        });
        if let Some((_, g)) = groups.get_mut(k) {
            g.push(ContourIdx(i));
        }
    }
    groups
}

/// A point as a map key (the arrangement's vertices are exact).
fn vertex_key(p: Point) -> (u64, u64) {
    (p.x.to_bits(), p.y.to_bits())
}

/// All faces of the planar arrangement of `shapes` (every area covered by at least one shape,
/// split wherever coverage changes).
pub fn regions(shapes: &[Shape]) -> Vec<Region> {
    regions_with(shapes, DEFAULT_PRECISION)
}

/// [`regions`] refit to `precision`.
pub(crate) fn regions_with(shapes: &[Shape], precision: f64) -> Vec<Region> {
    guarded(|| arrangement(shapes, precision).map(|a| regions_of(&a))).flatten().unwrap_or_default()
}

/// The face under `point`, if any.
pub fn region_at(shapes: &[Shape], point: Point) -> Option<Region> {
    regions(shapes).into_iter().find(|r| r.contains(point))
}

/// Merge regions into one path (their union).
pub fn merge_regions(regions: &[&Region]) -> BezPath {
    let v: Vec<(&BezPath, FillRule)> = regions.iter().map(|r| (&r.path, FillRule::NonZero)).collect();
    unite_all(&v)
}

/// Pathfinder Outline: split every shape's boundary at `junctions`, the points where three or more
/// edges meet (sorted by x, then y, so that a segment needs only the junctions inside its control
/// box). A closed subpath comes back as the edges of its outline, an open one as the edges of the
/// path itself, with no closing edge.
fn outline_edges(junctions: Vec<Point>, shapes: &[Shape]) -> Vec<Shape> {
    let scale = shapes.iter().filter_map(|s| control_bounds(&s.path)).map(|r| r.width().max(r.height())).fold(1.0, f64::max);
    let tol = scale * 1e-7;
    let reach = tol.max(1e-6);
    // (chain, key, source index)
    let mut chains: Vec<(Vec<Seg>, u64)> = Vec::new();
    for s in shapes {
        for sp in subs(&s.path).into_iter().map(crate::path::Sub::closed_if_ends_meet) {
            let open = !sp.closed;
            let outline = sp.chain();
            // A closed subpath of one segment is no outline; an open one is the path.
            if outline.len() < usize::from(!open) + 1 {
                continue;
            }
            // Split each segment at junctions; record which piece starts are breaks.
            let mut pieces: Vec<(Seg, bool)> = Vec::new();
            for Seg { c, line } in outline {
                // A point within `reach` of the curve is within `reach` of its control box (the
                // curve stays inside its control points' hull), so the other junctions can't match.
                let b = kurbo::Shape::bounding_box(&c).union(kurbo::Rect::from_points(c.p1, c.p2)).inflate(reach, reach);
                let lo = junctions.partition_point(|j| j.x < b.x0);
                let mut ts: Vec<f64> = junctions
                    .get(lo..)
                    .unwrap_or_default()
                    .iter()
                    .take_while(|j| j.x <= b.x1)
                    .filter(|j| j.y >= b.y0 && j.y <= b.y1)
                    .filter_map(|&j| {
                        let nr = c.nearest(j, 1e-9);
                        (nr.distance_sq.sqrt() <= reach).then_some(nr.t)
                    })
                    .collect();
                ts.sort_by(f64::total_cmp);
                // Junctions are compared where they are, not by parameter: a shape's own anchor and
                // a crossing computed a hair from it are one junction, and one within `reach` of an
                // end of the segment is that end. Else a sliver piece is left between them (VectorCraft issue 3870).
                let at = |t: f64| kurbo::ParamCurve::eval(&c, t);
                let start_is_junction = ts.first().is_some_and(|&t| at(t).distance(c.p0) <= reach);
                let mut cuts: Vec<f64> = vec![0.0];
                for t in ts {
                    let p = at(t);
                    let last = at(cuts.last().copied().unwrap_or(0.0));
                    if p.distance(last) > reach && p.distance(c.p3) > reach {
                        cuts.push(t);
                    }
                }
                cuts.push(1.0);
                for (k, w) in cuts.windows(2).enumerate() {
                    let sub = c.subsegment(w[0]..w[1]);
                    let seg = if line { Seg::line(sub.p0, sub.p3) } else { Seg { c: sub, line: false } };
                    pieces.push((seg, if k == 0 { start_is_junction } else { true }));
                }
            }
            // An open path is chained from its start to its end, cut at the junctions.
            if open {
                let mut cur: Vec<Seg> = Vec::new();
                for (seg, brk) in pieces {
                    if brk && !cur.is_empty() {
                        chains.push((std::mem::take(&mut cur), s.key));
                    }
                    cur.push(seg);
                }
                if !cur.is_empty() {
                    chains.push((cur, s.key));
                }
                continue;
            }
            // Chain pieces between breaks.
            let first_break = pieces.iter().position(|(_, b)| *b);
            match first_break {
                None => {
                    let segs: Vec<Seg> = pieces.into_iter().map(|(s, _)| s).collect();
                    chains.push((segs, s.key));
                }
                Some(k) => {
                    pieces.rotate_left(k);
                    let mut cur: Vec<Seg> = Vec::new();
                    for (seg, brk) in pieces {
                        if brk && !cur.is_empty() {
                            chains.push((std::mem::take(&mut cur), s.key));
                        }
                        cur.push(seg);
                    }
                    if !cur.is_empty() {
                        chains.push((cur, s.key));
                    }
                }
            }
        }
    }
    // De-duplicate coincident chains, keeping the front-most key (later shapes overwrite). A
    // match has its midpoint within `near` of this one's, so only the chains in the grid cells
    // around it are candidates; the first of them in `out` order is the one a scan would find.
    let mut out: Vec<(Vec<Seg>, u64)> = Vec::new();
    let mut sigs: Vec<(Point, Point, Point)> = Vec::new();
    let sig = |c: &[Seg]| -> (Point, Point, Point) {
        let a = c[0].c.p0;
        let b = c[c.len() - 1].c.p3;
        let mid = mid_point(c);
        (a, b, mid)
    };
    let d = reach * 10.0;
    let near = |p: Point, q: Point| p.distance(q) <= d;
    let cell = |p: Point| ((p.x / d).floor() as i64, (p.y / d).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<usize>> = HashMap::new();
    for (chain, key) in chains {
        let (a, b, m) = sig(&chain);
        let (cx, cy) = cell(m);
        let found = (cx - 1..=cx + 1)
            .flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y)))
            .filter_map(|k| grid.get(&k))
            .flatten()
            .copied()
            .filter(|&i| {
                let (a2, b2, m2) = sigs[i];
                near(m, m2) && ((near(a, a2) && near(b, b2)) || (near(a, b2) && near(b, a2)))
            })
            .min();
        match found {
            Some(i) => out[i].1 = key,
            None => {
                grid.entry((cx, cy)).or_default().push(out.len());
                sigs.push((a, b, m));
                out.push((chain, key));
            }
        }
    }
    out.into_iter()
        .filter_map(|(chain, key)| {
            // Chain joints are the shapes' own anchors, so there is nothing to re-merge.
            let sp = segs_to_bezpath(&chain, false)?;
            Some(Shape::new(sp, FillRule::NonZero, key))
        })
        .collect()
}

fn mid_point(c: &[Seg]) -> Point {
    let lens: Vec<f64> = c.iter().map(|s| kurbo::ParamCurveArclen::arclen(&s.c, 1e-6)).collect();
    let total: f64 = lens.iter().sum();
    let mut acc = 0.0;
    for (s, l) in c.iter().zip(&lens) {
        if acc + l >= total / 2.0 && *l > 0.0 {
            let t = kurbo::ParamCurveArclen::inv_arclen(&s.c, total / 2.0 - acc, 1e-6);
            return s.c.eval(t);
        }
        acc += l;
    }
    c[0].c.p0
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::CONTOUR_PASSES;
    use crate::testutil::{ellipse, length, line, polyline, polyline_pts, rect, subpath_count};
    use kurbo::Rect;
    use proptest::prelude::*;

    /// The faces one contour pass per mask finds: what [`regions_of`] must match exactly when no
    /// face lies inside a hole of its own mask (`grouped` puts such an island in the frame's face).
    fn regions_one_mask_at_a_time(arr: &Arrangement) -> Vec<Region> {
        let mut out = Vec::new();
        for mask in arr.distinct_masks() {
            let c = arr.contours(|m| m == mask.as_slice());
            let sources: Vec<usize> = mask.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
            for g in c.grouped() {
                let path = contours_to_path(&c, g, &arr.tidy);
                if !path.elements().is_empty() {
                    out.push(Region { path, sources: sources.clone() });
                }
            }
        }
        out
    }

    fn same_as_one_mask_at_a_time(shapes: &[Shape]) {
        let arr = arrangement(shapes, DEFAULT_PRECISION).expect("arrangement");
        assert_eq!(regions_of(&arr), regions_one_mask_at_a_time(&arr));
    }

    /// The refit precision: pieces thinner than the precision are dropped, so a 0.5 unit strip that
    /// the default keeps goes at a precision of 1, whether the Pathfinder works on the arrangement
    /// (Divide), the planar map (Divide with an open path) or the union (Unite).
    #[test]
    fn precision_drops_pieces_thinner_than_it() {
        let overlapping =
            [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(rect(0.0, 99.5, 100.0, 200.0), FillRule::NonZero, 2)];
        let line = polyline(&[(-10.0, 99.5), (110.0, 99.5)], false);
        let cut = [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(line, FillRule::NonZero, 2)];
        let apart =
            [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(rect(0.0, 150.0, 100.0, 150.5), FillRule::NonZero, 2)];
        for (what, op, shapes, kept) in [
            ("Divide", PathfinderOp::Divide, &overlapping, 3),
            ("Divide with an open path", PathfinderOp::Divide, &cut, 2),
            ("Unite", PathfinderOp::Unite, &apart, 2),
        ] {
            let count = |r: Vec<Shape>| r.iter().map(|s| subpath_count(&s.path)).sum::<usize>();
            assert_eq!(pathfinder_with(op, shapes, DEFAULT_PRECISION), pathfinder(op, shapes), "{what}: the default is the plain Pathfinder");
            assert_eq!(count(pathfinder(op, shapes)), kept, "{what}: the default keeps the strip");
            assert_eq!(count(pathfinder_with(op, shapes, 1.0)), kept - 1, "{what}: 1 drops it");
            assert_eq!(pathfinder_with(op, shapes, f64::NAN), pathfinder(op, shapes), "{what}: an invalid precision is the default");
        }
    }

    /// The planar map that Divide with an open path uses merges two edges between the same points
    /// only when they are closer than the precision: a lens 0.005 thick between two curves is a
    /// piece at a precision of 0.001, and not at the default.
    #[test]
    fn open_path_divide_keeps_a_lens_thicker_than_the_precision() {
        // Curves monotonic in x and y, so the sweep keeps each one whole between the shared ends;
        // their middles are 0.005 apart.
        let shift = 0.005 / 0.75 / std::f64::consts::SQRT_2;
        let curve = |d: f64| [Point::new(0.0, 0.0), Point::new(d, 50.0 - d), Point::new(50.0 + d, 100.0 - d), Point::new(100.0, 100.0)];
        let (top, bottom) = (curve(0.0), curve(shift));
        let mut bp = kurbo::BezPath::new();
        bp.move_to(top[0]);
        bp.curve_to(top[1], top[2], top[3]);
        bp.curve_to(bottom[2], bottom[1], bottom[0]);
        bp.close_path();
        let lens = Shape::new(bp, FillRule::NonZero, 1);
        let line = polyline(&[(200.0, 0.0), (300.0, 0.0)], false);
        let shapes = [lens, Shape::new(line, FillRule::NonZero, 2)];
        let area = |r: &[Shape]| r.iter().map(|s| crate::area(&s.path, FillRule::NonZero)).sum::<f64>();
        let fine = pathfinder_with(PathfinderOp::Divide, &shapes, 0.001);
        assert!(area(&fine) > 0.1, "the lens is a piece: {} square units", area(&fine));
        assert_eq!(area(&pathfinder(PathfinderOp::Divide, &shapes)), 0.0, "thinner than the default precision");
        // A precision no float can flatten a curve to: VectorCraft's Divide panicked here ("capacity
        // overflow", in kurbo's flatten) as it looked for a point inside the thin face.
        for precision in [1e-300, f64::MIN_POSITIVE, 5e-324] {
            let _ = pathfinder_with(PathfinderOp::Divide, &shapes, precision);
        }
    }

    /// A precision so fine that flattening a face to find a point inside it would not end is
    /// taken as fine as the face allows.
    #[test]
    fn a_precision_finer_than_floats_allow_returns() {
        let shapes = [
            Shape::new(ellipse(Rect::new(0.0, 0.0, 100.0, 100.0)), FillRule::NonZero, 1),
            Shape::new(line(Point::new(-10.0, 50.0), Point::new(110.0, 50.0)), FillRule::NonZero, 2),
        ];
        for precision in [1e-300, 1e-100, 1e-30, f64::MIN_POSITIVE, 5e-324] {
            for op in [PathfinderOp::Divide, PathfinderOp::Unite, PathfinderOp::Trim, PathfinderOp::Outline] {
                let _ = pathfinder_with(op, &shapes, precision);
            }
        }
    }

    /// Shapes on a coarse grid, so that edges and corners coincide: rectangles, rectangles with
    /// a hole, ellipses and polygons that may cross themselves, under either fill rule.
    fn arb_shape() -> impl Strategy<Value = Shape> {
        let c = || (0..12i32).prop_map(|v| f64::from(v) * 10.0);
        let r = (c(), c(), 1..6i32, 1..6i32).prop_map(|(x, y, w, h)| (x, y, x + f64::from(w) * 10.0, y + f64::from(h) * 10.0));
        let path = prop_oneof![
            r.clone().prop_map(|(x0, y0, x1, y1)| rect(x0, y0, x1, y1)),
            r.clone().prop_map(|(x0, y0, x1, y1)| {
                let (cx, cy) = ((x0 + x1) / 2.0, (y0 + y1) / 2.0);
                let mut p = rect(x0 - 10.0, y0 - 10.0, x1 + 10.0, y1 + 10.0);
                p.extend(rect(x0, y0, cx.max(x0 + 5.0), cy.max(y0 + 5.0)).iter());
                p
            }),
            r.prop_map(|(x0, y0, x1, y1)| ellipse(Rect::new(x0, y0, x1, y1))),
            prop::collection::vec((c(), c()), 3..7).prop_map(|pts| polyline(&pts, true)),
        ];
        (path, any::<bool>(), any::<u64>())
            .prop_map(|(p, even_odd, key)| Shape::new(p, if even_odd { FillRule::EvenOdd } else { FillRule::NonZero }, key))
    }

    /// Contours that touch at a point, which the sweep's tree of contours gets wrong
    /// ([`face_groups`]): each face keeps its holes, and no hole is a face of its own.
    #[test]
    fn contours_touching_at_a_point_keep_their_faces() {
        let sorted = |shapes: &[Shape]| {
            let mut v: Vec<(Vec<usize>, f64)> =
                regions(shapes).iter().map(|r| (r.sources.clone(), crate::area(&r.path, FillRule::NonZero))).collect();
            v.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.total_cmp(&b.1)));
            v
        };
        // A frame whose cutout corner (60,110) touches its hole: the sweep makes the frame an
        // island inside the hole.
        let mut frame = rect(50.0, 100.0, 80.0, 130.0);
        frame.extend(rect(60.0, 110.0, 65.0, 115.0).iter());
        let s = [Shape::new(rect(20.0, 70.0, 60.0, 110.0), FillRule::NonZero, 0), Shape::new(frame, FillRule::EvenOdd, 0)];
        assert_eq!(sorted(&s), [(vec![0], 1500.0), (vec![0, 1], 100.0), (vec![1], 775.0)]);
        // Two holes touching at (30,50): the sweep makes the second an island inside the first.
        let s = [
            Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 0),
            Shape::new(rect(20.0, 40.0, 30.0, 50.0), FillRule::NonZero, 0),
            Shape::new(rect(30.0, 50.0, 40.0, 60.0), FillRule::NonZero, 0),
        ];
        assert_eq!(sorted(&s), [(vec![0], 9800.0), (vec![0, 1], 100.0), (vec![0, 2], 100.0)]);
    }

    #[test]
    fn crossing_strips_match_one_mask_at_a_time() {
        let mut v = Vec::new();
        for i in 0..12 {
            let x = 10.0 + 20.0 * f64::from(i);
            v.push(Shape::new(rect(x, 0.0, x + 4.0, 250.0), FillRule::NonZero, 1));
            v.push(Shape::new(rect(0.0, x, 250.0, x + 4.0), FillRule::NonZero, 2));
        }
        same_as_one_mask_at_a_time(&v);
        // Every crossing, and each strip in 13 pieces.
        assert_eq!(regions(&v).len(), 12 * 12 + 2 * 12 * 13);
    }

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed_9a7f), ..ProptestConfig::default() })]

        /// Batching the masks changes nothing: the same regions, in the same order, with the same
        /// contours in each.
        #[test]
        fn batched_regions_match_one_mask_at_a_time(v in prop::collection::vec(arb_shape(), 1..8)) {
            same_as_one_mask_at_a_time(&v);
        }
    }

    /// A merge walks the arrangement once for each batch of the faces' coverages: 49 rectangles in
    /// a row, each overlapping the next, make 97 faces, each with a coverage of its own, and the
    /// regions of all of them take a handful of contour passes, not 97.
    #[test]
    fn many_faces_are_found_in_a_few_contour_passes() {
        let v: Vec<Shape> = (0..49)
            .map(|k| {
                let x = 10.0 * f64::from(k);
                Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, 1)
            })
            .collect();
        CONTOUR_PASSES.with(|n| n.set(0));
        let faces = regions(&v);
        let passes = CONTOUR_PASSES.with(std::cell::Cell::get);
        assert_eq!(faces.len(), 97);
        assert!(passes <= 8, "{passes} contour passes");
    }

    /// The lists of inputs a winding covers are found by those inputs, also among lists that share
    /// a hash: here every list is filed under every hash.
    #[test]
    fn cover_ids_tell_apart_lists_with_one_hash() {
        let v: Vec<Shape> = (0..3)
            .map(|k| {
                let x = 10.0 * f64::from(k);
                Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, 1)
            })
            .collect();
        let arr = arrangement(&v, DEFAULT_PRECISION).unwrap();
        let lists = vec![vec![0], vec![0, 1], vec![1], vec![1, 2], vec![2]];
        let mut ids = CoverIds::new(lists.clone());
        let all: Vec<usize> = (0..lists.len()).collect();
        for l in &lists {
            ids.by_hash.insert(CoverIds::hash(l.iter().copied()), all.clone());
        }
        let mut seen = vec![false; lists.len()];
        for s in arr.top.segment_indices() {
            let h = s.first_half();
            for w in [arr.top.winding_clockwise(h), arr.top.winding_counter_clockwise(h)] {
                let covered: Vec<usize> = arr.covered(w).collect();
                let want = lists.iter().position(|l| *l == covered);
                assert_eq!(ids.of(&arr, w), want, "{covered:?}");
                if let Some(i) = want {
                    seen[i] = true;
                }
            }
        }
        assert!(seen.iter().all(|s| *s), "every list is next to an edge: {seen:?}");
    }

    #[test]
    fn divide_open_paths_cut_faces_without_taking_the_cutters_paint() {
        let seg = |a: (f64, f64), b: (f64, f64)| line(Point::from(a), Point::from(b));
        let mut cases = vec![
            ("horizontal", vec![seg((-10.0, 50.0), (110.0, 50.0))], 2),
            ("reverse", vec![seg((110.0, 50.0), (-10.0, 50.0))], 2),
            ("vertical", vec![seg((50.0, -10.0), (50.0, 110.0))], 2),
            ("diagonal", vec![seg((-10.0, -10.0), (110.0, 110.0))], 2),
            ("crossed", vec![seg((-10.0, 50.0), (110.0, 50.0)), seg((50.0, -10.0), (50.0, 110.0))], 4),
            ("outside", vec![seg((-10.0, 150.0), (110.0, 150.0))], 1),
            ("dangling", vec![seg((-10.0, 50.0), (50.0, 50.0))], 1),
            ("boundary", vec![seg((-10.0, 0.0), (110.0, 0.0))], 1),
            ("tangent", vec![seg((-10.0, 10.0), (10.0, -10.0))], 1),
        ];
        let mut curve = kurbo::BezPath::new();
        curve.move_to((-10.0, 50.0));
        curve.curve_to((30.0, 15.0), (70.0, 85.0), (110.0, 50.0));
        cases.push(("curved", vec![curve], 2));
        let bent = polyline(&[(-10.0, 25.0), (50.0, 75.0), (110.0, 25.0)], false);
        cases.push(("bent", vec![bent], 2));
        for (label, cutters, count) in cases {
            let mut inputs = vec![Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 7)];
            inputs.extend(cutters.into_iter().map(|p| Shape::new(p, FillRule::NonZero, 99)));
            let divided = pathfinder(PathfinderOp::Divide, &inputs);
            assert_eq!(divided.len(), count, "{label}");
            assert!(divided.iter().all(|s| s.key == 7 && crate::path::subs(&s.path).iter().all(|sp| sp.closed)), "{label}");
            let total: f64 = divided.iter().map(|s| crate::area(&s.path, s.rule)).sum();
            assert!((total - 10000.0).abs() < 0.01, "{label}: area {total}");
            for p in [Point::new(20.0, 31.0), Point::new(79.0, 37.0), Point::new(81.0, 68.0), Point::new(17.0, 85.0)] {
                assert_eq!(divided.iter().filter(|s| s.path.winding(p) != 0).count(), 1, "{label}: {p:?}");
            }
        }
    }

    #[test]
    fn divide_open_paths_preserve_frontmost_fills_and_holes() {
        let cutter = || Shape::new(line(Point::new(-10.0, 50.0), Point::new(160.0, 50.0)), FillRule::NonZero, 99);
        let inputs = [
            Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 7),
            Shape::new(rect(50.0, 0.0, 150.0, 100.0), FillRule::NonZero, 8),
            cutter(),
        ];
        let faces = pathfinder(PathfinderOp::Divide, &inputs);
        assert_eq!(faces.len(), 6);
        assert_eq!(faces.iter().filter(|s| s.key == 7).count(), 2);
        assert_eq!(faces.iter().filter(|s| s.key == 8).count(), 4);
        let total: f64 = faces.iter().map(|s| crate::area(&s.path, s.rule)).sum();
        assert!((total - 15000.0).abs() < 1e-6);
        let mut ring = rect(0.0, 0.0, 100.0, 100.0);
        ring.extend(rect(30.0, 30.0, 70.0, 70.0).iter());
        let faces = pathfinder(PathfinderOp::Divide, &[Shape::new(ring, FillRule::EvenOdd, 7), cutter()]);
        assert_eq!(faces.len(), 2);
        assert!(faces.iter().all(|s| s.key == 7 && s.path.winding(Point::new(50.0, 40.0)) == 0));
        let total: f64 = faces.iter().map(|s| crate::area(&s.path, s.rule)).sum();
        assert!((total - 8400.0).abs() < 1e-6);
    }

    #[test]
    fn divide_open_paths_retains_the_existing_segment_cap() {
        let mut inputs = vec![Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 7)];
        // The square's four sides and one line past the cap.
        let n = crate::SHAPE_BUILDER_MAX_SEGMENTS - 3;
        inputs.extend((0..n).map(|_| Shape::new(line(Point::new(-10.0, 50.0), Point::new(110.0, 50.0)), FillRule::NonZero, 99)));
        let faces = pathfinder(PathfinderOp::Divide, &inputs);
        assert_eq!(faces.len(), 1, "a segment past the cap falls back to filled areas");
        assert_eq!(faces[0].key, 7);
        assert!((crate::area(&faces[0].path, faces[0].rule) - 10000.0).abs() < 1e-6);
    }

    fn old_trim(shapes: &[Shape], precision: f64) -> Vec<Shape> {
        let n = shapes.len();
        let Some(arr) = arrangement(shapes, precision) else { return Vec::new() };
        (0..n)
            .flat_map(|i| {
                let c = arr.contours_where(|w| arr.covered(w).next_back() == Some(i));
                one(all_contours_to_path(&c, &arr.tidy), shapes[i].key)
            })
            .collect()
    }

    fn old_crop(shapes: &[Shape], precision: f64) -> Vec<Shape> {
        let n = shapes.len();
        if n < 2 {
            return Vec::new();
        }
        let Some(arr) = arrangement(shapes, precision) else { return Vec::new() };
        (0..n - 1)
            .flat_map(|i| {
                let c = arr.contours_where(|w| {
                    let mut c = arr.covered(w).rev();
                    c.next() == Some(n - 1) && c.next() == Some(i)
                });
                one(all_contours_to_path(&c, &arr.tidy), shapes[i].key)
            })
            .collect()
    }

    #[test]
    fn trim_and_crop_differential_matches_reference_on_overlapping_shapes() {
        let shapes = vec![
            Shape::new(rect(0.0, 0.0, 60.0, 60.0), FillRule::NonZero, 1),
            Shape::new(rect(30.0, 0.0, 90.0, 60.0), FillRule::NonZero, 2),
            Shape::new(rect(15.0, 15.0, 75.0, 75.0), FillRule::NonZero, 3),
            Shape::new(rect(40.0, 40.0, 100.0, 100.0), FillRule::NonZero, 4),
        ];

        let trim_actual = pathfinder_with(PathfinderOp::Trim, &shapes, DEFAULT_PRECISION);
        let trim_expected = old_trim(&shapes, DEFAULT_PRECISION);
        assert_eq!(trim_actual.len(), trim_expected.len(), "trim shape count must match");
        for (actual, expected) in trim_actual.iter().zip(&trim_expected) {
            assert_eq!(actual.key, expected.key);
            assert_eq!(actual.path, expected.path, "subpath order and contours must match reference trim");
        }

        let crop_actual = pathfinder_with(PathfinderOp::Crop, &shapes, DEFAULT_PRECISION);
        let crop_expected = old_crop(&shapes, DEFAULT_PRECISION);
        assert_eq!(crop_actual.len(), crop_expected.len(), "crop shape count must match");
        for (actual, expected) in crop_actual.iter().zip(&crop_expected) {
            assert_eq!(actual.key, expected.key);
            assert_eq!(actual.path, expected.path, "subpath order and contours must match reference crop");
        }

        // Test with shape that has no face left (completely covered by front shape)
        let covered = vec![
            Shape::new(rect(10.0, 10.0, 20.0, 20.0), FillRule::NonZero, 10), // inside shape 20
            Shape::new(rect(0.0, 0.0, 50.0, 50.0), FillRule::NonZero, 20),
        ];
        let trim_cov_actual = pathfinder_with(PathfinderOp::Trim, &covered, DEFAULT_PRECISION);
        let trim_cov_expected = old_trim(&covered, DEFAULT_PRECISION);
        assert_eq!(trim_cov_actual.len(), 1);
        assert_eq!(trim_cov_actual, trim_cov_expected);
    }

    /// Merge walks the arrangement once for all the keys that share no vertex, not once for each:
    /// 300 rectangles apart, each with a key of its own, are one contour pass.
    #[test]
    fn merge_of_many_shapes_apart_is_a_few_contour_passes() {
        let shapes: Vec<Shape> = (0..300u32)
            .map(|i| Shape::new(rect(f64::from(i) * 20.0, 0.0, f64::from(i) * 20.0 + 10.0, 10.0), FillRule::NonZero, u64::from(i)))
            .collect();
        CONTOUR_PASSES.with(|n| n.set(0));
        let merged = pathfinder(PathfinderOp::Merge, &shapes);
        let passes = CONTOUR_PASSES.with(std::cell::Cell::get);
        assert_eq!(merged.len(), 300);
        assert!(merged.iter().enumerate().all(|(i, s)| s.key == i as u64 && (crate::area(&s.path, FillRule::NonZero) - 100.0).abs() < 1e-9));
        assert!(passes <= 3, "{passes} passes for 300 keys");
    }

    /// Merge by batches gives what a contour pass for each key gave: the same shapes, in the same order.
    #[test]
    fn merge_is_what_a_pass_for_each_key_gives() {
        let reference = |shapes: &[Shape]| -> Vec<Shape> {
            let arr = arrangement(shapes, DEFAULT_PRECISION).expect("arrangement");
            let mut keys: Vec<u64> = Vec::new();
            for s in shapes {
                if !keys.contains(&s.key) {
                    keys.push(s.key);
                }
            }
            let mut out = Vec::new();
            for k in keys {
                let c = arr.contours_where(|w| arr.covered(w).next_back().is_some_and(|t| shapes[t].key == k));
                for g in c.grouped() {
                    out.extend(one(contours_to_path(&c, g, &arr.tidy), k));
                }
            }
            out
        };
        let mut stacks: Vec<Vec<Shape>> = Vec::new();
        // Overlapping rectangles and circles with repeated keys, and some apart.
        for seed in 0..40u32 {
            let mut v = Vec::new();
            for i in 0..(3 + seed % 5) {
                let (x, y) = (f64::from((seed * 7 + i * 13) % 11) * 8.0, f64::from((seed * 5 + i * 3) % 7) * 8.0);
                let path = if (seed + i) % 3 == 0 { ellipse(Rect::new(x, y, x + 30.0, y + 24.0)) } else { rect(x, y, x + 25.0, y + 20.0) };
                v.push(Shape::new(path, FillRule::NonZero, u64::from((seed + i * 2) % 3)));
            }
            stacks.push(v);
        }
        for (n, shapes) in stacks.iter().enumerate() {
            assert_eq!(pathfinder(PathfinderOp::Merge, shapes), reference(shapes), "stack {n}");
        }
    }

    /// An island of a key inside the hole of its own frame is a component of its own, so a shape of its own.
    #[test]
    fn merge_makes_a_shape_of_each_component_also_an_island_in_a_hole() {
        let mut frame = rect(0.0, 0.0, 100.0, 100.0);
        frame.extend(rect(20.0, 20.0, 80.0, 80.0).reverse_subpaths().iter());
        let shapes = [Shape::new(frame, FillRule::NonZero, 7), Shape::new(rect(40.0, 40.0, 60.0, 60.0), FillRule::NonZero, 7)];
        let merged = pathfinder(PathfinderOp::Merge, &shapes);
        assert_eq!(merged.len(), 2, "{:?}", merged.iter().map(|s| subpath_count(&s.path)).collect::<Vec<_>>());
        assert!(merged.iter().all(|s| s.key == 7));
        let areas: Vec<f64> = merged.iter().map(|s| crate::area(&s.path, FillRule::NonZero)).collect();
        assert!(areas.iter().any(|a| (a - 6400.0).abs() < 1e-6) && areas.iter().any(|a| (a - 400.0).abs() < 1e-6), "{areas:?}");
    }

    #[test]
    fn open_paths_are_the_only_input_to_outline_and_survive_it() {
        // A polyline and a rectangle crossing it: the outline cuts both where they meet.
        let shapes = [
            Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1),
            Shape::new(polyline_pts(&[Point::new(-10.0, 50.0), Point::new(110.0, 50.0)], false), FillRule::NonZero, 2),
        ];
        let pieces = pathfinder(PathfinderOp::Outline, &shapes);
        let of = |key: u64| pieces.iter().filter(|s| s.key == key).collect::<Vec<_>>();
        // The rectangle is cut at the two crossings, and so is the line: the line survives.
        assert_eq!(of(1).len(), 2, "the rectangle is cut where the line crosses it");
        assert_eq!(of(2).len(), 3, "the line is cut at the same two points");
        assert_eq!(pieces.len(), 5);
        assert!((of(1).iter().map(|s| length(&s.path)).sum::<f64>() - 400.0).abs() < 1e-6);
        assert!((of(2).iter().map(|s| length(&s.path)).sum::<f64>() - 120.0).abs() < 1e-6);
        // The line's pieces end at the crossings, which are the rectangle's cuts.
        let crossing = |p: &BezPath, at: Point| {
            let first = p.elements().first().and_then(kurbo::PathEl::end_point);
            let last = p.elements().last().and_then(kurbo::PathEl::end_point);
            first.is_some_and(|a| a.distance(at) < 1e-6) || last.is_some_and(|b| b.distance(at) < 1e-6)
        };
        for at in [Point::new(0.0, 50.0), Point::new(100.0, 50.0)] {
            assert_eq!(of(1).iter().filter(|s| crossing(&s.path, at)).count(), 2, "{at:?}");
            assert_eq!(of(2).iter().filter(|s| crossing(&s.path, at)).count(), 2, "{at:?}");
        }
    }

    /// A path that comes back to where it started without closing (how a drawing program writes an
    /// ellipse) is a ring: its outline is that of the closed path, not two open chains cut at its start.
    #[test]
    fn outline_of_an_open_ring_is_that_of_the_closed_one() {
        let ring = |close: bool| {
            let pts: Vec<Point> = (0..=8)
                .map(|i| {
                    let a = std::f64::consts::TAU * f64::from(i % 8) / 8.0;
                    Point::new(50.0 + 30.0 * a.cos(), 50.0 + 30.0 * a.sin())
                })
                .collect();
            let mut p = polyline_pts(&pts, false);
            if close {
                p.close_path();
            }
            p
        };
        let cutter = rect(40.0, 30.0, 60.0, 90.0);
        let outline = |close: bool| {
            pathfinder(PathfinderOp::Outline, &[Shape::new(ring(close), FillRule::NonZero, 1), Shape::new(cutter.clone(), FillRule::NonZero, 2)])
        };
        let (open, closed) = (outline(false), outline(true));
        assert_eq!(open.len(), closed.len(), "{} pieces of the open ring, {} of the closed", open.len(), closed.len());
        let total = |v: &[Shape], key: u64| v.iter().filter(|s| s.key == key).map(|s| length(&s.path)).sum::<f64>();
        assert!((total(&open, 1) - total(&closed, 1)).abs() < 1e-6 && (total(&open, 2) - total(&closed, 2)).abs() < 1e-6);
        // Alone, the ring is one piece, as a closed path is (not cut at the point it starts from).
        let alone = pathfinder(PathfinderOp::Outline, &[Shape::new(ring(false), FillRule::NonZero, 1)]);
        assert_eq!(alone.len(), 1);
    }

    /// An open path comes back as its own edges: no closing edge, cut where another path meets it.
    #[test]
    fn outline_of_an_open_path_has_no_closing_edge() {
        let v = polyline_pts(&[Point::new(0.0, 0.0), Point::new(10.0, 20.0), Point::new(20.0, 0.0)], false);
        let alone = pathfinder(PathfinderOp::Outline, &[Shape::new(v.clone(), FillRule::NonZero, 1)]);
        assert_eq!(alone.len(), 1);
        assert!((length(&alone[0].path) - 2.0 * 500f64.sqrt()).abs() < 1e-9, "{}", length(&alone[0].path));
        // A second path through its corner cuts it there in two.
        let cutter = polyline_pts(&[Point::new(10.0, 30.0), Point::new(10.0, 20.0), Point::new(10.0, 5.0)], false);
        let cut = pathfinder(PathfinderOp::Outline, &[Shape::new(v, FillRule::NonZero, 1), Shape::new(cutter, FillRule::NonZero, 2)]);
        assert_eq!(cut.iter().filter(|s| s.key == 1).count(), 2);
        assert!((cut.iter().filter(|s| s.key == 1).map(|s| length(&s.path)).sum::<f64>() - 2.0 * 500f64.sqrt()).abs() < 1e-9);
    }
}
