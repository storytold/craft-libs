// Carved from VectorCraft's crates/pathops (src/pathfinder.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

//! The Pathfinder operations and planar regions, over an ordered stack of shapes (index 0 =
//! back-most, last = front-most).

use std::collections::{HashMap, HashSet};

use kurbo::{BezPath, CubicBez, ParamCurve, ParamCurveNearest, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours};

use crate::FillRule;
use crate::boolean::{Arrangement, DEFAULT_PRECISION, Multi, all_contours_to_path, contours_to_path, unite_all, unite_all_with};
use crate::budget::{NEAREST_STEPS, SearchBudget};
use crate::path::{Seg, control_bounds, fill_bezpath, guarded, segs_to_bezpath, subs};
use crate::work::{self, Work};

#[cfg(test)]
thread_local! {
    /// Makes [`each_classified_face`] take its fallback (a pass for each class) for every batch, which
    /// no input does, so that a test can see what it gives when its allowance runs out part way.
    static FORCE_FALLBACK: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

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
    /// Split every edge at intersections; returns *open* paths keyed by the shape they bound. Where
    /// an open path is among the shapes the intersections come from the planar map, and there are no
    /// shapes when it cannot be built (past [`crate::SHAPE_BUILDER_MAX_SEGMENTS`] segments, say).
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
        crate::winding::winding(&self.path, p) != 0
    }
}

/// Is some subpath of `shapes` an open one, that is not a ring (a path of three anchors or more that
/// comes back to where it started counts as closed, as does one cubic that loops round an area)?
fn has_open_edges(shapes: &[Shape]) -> bool {
    shapes.iter().any(|s| subs(&s.path).into_iter().map(crate::path::Sub::closed_if_ends_meet).any(|sp| !sp.closed))
}

/// The arrangement of `shapes`, its faces refit to `precision` and its slivers thinner than it dropped.
#[cfg(test)]
fn arrangement(shapes: &[Shape], precision: f64) -> Option<Arrangement> {
    try_arrangement(shapes, precision).ok()
}

/// [`arrangement`], and why there is none: [`crate::PathOpsError::TooComplex`] past the element limit,
/// or the error of the sweep ([`Arrangement::new`]).
fn try_arrangement(shapes: &[Shape], precision: f64) -> Result<Arrangement, crate::PathOpsError> {
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return Err(crate::PathOpsError::TooComplex);
    }
    let bps: Vec<(BezPath, FillRule)> = shapes.iter().map(|s| (fill_bezpath(&s.path), s.rule)).collect();
    let mut arr = Arrangement::new(bps)?;
    arr.tidy.precision = precision;
    Ok(arr)
}

/// The one item of `it`, if it has exactly one.
fn only(mut it: impl Iterator<Item = usize>) -> Option<usize> {
    let first = it.next()?;
    it.next().is_none().then_some(first)
}

/// `path` as the one shape of a result, keyed `key` and filled non-zero; none when it is empty.
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
    guarded(|| pathfinder_unguarded(op, shapes, precision, false)).and_then(Result::ok).unwrap_or_default()
}

/// [`pathfinder`] that reports what it cannot do instead of giving no shapes, or falling back to the
/// filled areas: [`crate::PathOpsError::TooComplex`] past the element limit, past a limit of the sweep or
/// of the work of a contour pass ([`crate::MAX_PASS_WORK`]), or past one of the planar map's (more than
/// [`crate::SHAPE_BUILDER_MAX_SEGMENTS`] segments, or [`crate::MAX_FILL_WORK`]) for Divide and Outline
/// with an open path, and past the limit on the faces of closed shapes ([`crate::Limits::mask_work`], or
/// the passes' [`crate::Limits::pass_work`]) for Divide; [`crate::PathOpsError::NonFinite`] for a point that
/// is not finite, and [`crate::PathOpsError::Degenerate`] for one so large that the sweep cannot take it,
/// in the shapes as they are given (a closed loop of one cubic whose handle is no number is left out when
/// they are filled, and is no excuse); and [`crate::PathOpsError::Degenerate`] when the sweep fails or no
/// frame of the map leaves the open paths unlevelled. Where it gives shapes they are those [`pathfinder`]
/// gives.
pub fn try_pathfinder(op: PathfinderOp, shapes: &[Shape]) -> Result<Vec<Shape>, crate::PathOpsError> {
    try_pathfinder_with(op, shapes, DEFAULT_PRECISION)
}

/// [`try_pathfinder`] with a refit `precision`, as [`pathfinder_with`].
pub fn try_pathfinder_with(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Result<Vec<Shape>, crate::PathOpsError> {
    guarded(|| pathfinder_unguarded(op, shapes, precision, true)).unwrap_or(Err(crate::PathOpsError::Degenerate))
}

/// [`pathfinder_with`] without the guard that turns a panic into no result. With `strict`, what cannot
/// be done is the error ([`try_pathfinder`]); without, it gives no shapes, or the filled areas where
/// the planar map of a Divide with an open path is not made.
fn pathfinder_unguarded(op: PathfinderOp, shapes: &[Shape], precision: f64, strict: bool) -> Result<Vec<Shape>, crate::PathOpsError> {
    use crate::PathOpsError;
    let precision = if precision.is_finite() && precision > 0.0 { precision } else { DEFAULT_PRECISION };
    let n = shapes.len();
    // No shapes, or more segments than are read: no result.
    if n == 0 {
        return Ok(Vec::new());
    }
    if !crate::budget::fits_elements(shapes.iter().map(|s| &s.path)) {
        return if strict { Err(PathOpsError::TooComplex) } else { Ok(Vec::new()) };
    }
    // Strictly, the shapes are checked as given: the fill leaves out a closed loop of one cubic with a
    // handle that is no number, or so large that it cannot be swept, and what is left would be a result
    // without it where it is an error ([`PathOpsError::NonFinite`], [`PathOpsError::Degenerate`]).
    if strict {
        crate::boolean::eps_for(shapes.iter().map(|s| &s.path))?;
    }
    // What cannot be done: the error, or no shapes.
    let none = |e: PathOpsError| if strict { Err(e) } else { Ok(Vec::new()) };
    let front_key = shapes[n - 1].key;
    let unite = || -> Result<Vec<Shape>, PathOpsError> {
        let v: Vec<(&BezPath, FillRule)> = shapes.iter().map(|s| (&s.path, s.rule)).collect();
        if strict {
            Ok(one(crate::boolean::try_unite_all_with(&v, precision)?, front_key))
        } else {
            Ok(one(unite_all_with(&v, precision), front_key))
        }
    };
    // Unite doesn't need the arrangement.
    if op == PathfinderOp::Unite {
        return unite();
    }
    // Open paths are cutting edges for Divide; the filled-only operations keep their
    // implicit closure. Reuse the planar map of `shape_builder` and its bound and fallback.
    if op == PathfinderOp::Divide && crate::has_open_path(shapes) {
        return Ok(crate::planar::try_builder(shapes, false, precision, strict)?
            .regions
            .into_iter()
            .filter_map(|r| {
                let key = shapes.get(r.top()?)?.key;
                Some(Shape::new(r.path, FillRule::NonZero, key))
            })
            .collect());
    }
    // Open paths (but for a ring that comes back to where it started) are edges that cross others
    // without a trace in the filled areas, where their way there and back cancel: the crossings, and so
    // the cuts, come from the planar map, and the filled arrangement is not made at all (its closing
    // lines can be over its limits while the map is not). Where the map cannot be built there are no
    // shapes: the arrangement's junctions would leave every open path uncut where it meets another.
    if op == PathfinderOp::Outline && has_open_edges(shapes) {
        return match crate::planar::try_map_junctions(shapes, precision) {
            Ok(Some(junctions)) => match outline_edges(junctions, shapes, true) {
                Some(edges) => Ok(edges),
                // The searches that cut the edges ran out of their budget ([`crate::Limits::search_work`]).
                None => none(PathOpsError::TooComplex),
            },
            Ok(None) => Ok(Vec::new()),
            Err(e) => none(e),
        };
    }
    let arr = match try_arrangement(shapes, precision) {
        Ok(arr) => arr,
        Err(e) => return none(e),
    };
    let p = &arr.tidy;
    Ok(match op {
        PathfinderOp::Unite => unite()?,
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
        // Past the limit on the entries or the passes there are no faces: no shapes, or the error.
        PathfinderOp::Divide => match regions_of(&arr) {
            Some(regions) => regions
                .into_iter()
                .filter_map(|r| {
                    let key = shapes.get(r.top()?)?.key;
                    Some(Shape::new(r.path, FillRule::NonZero, key))
                })
                .collect(),
            None => return none(PathOpsError::TooComplex),
        },
        // topmost(m) == Some(i)
        PathfinderOp::Trim => {
            let mut paths = vec![BezPath::new(); n];
            let whole = each_classified_face(
                &arr,
                n,
                |w| arr.covered(w).next_back(),
                |i, c, group| {
                    if let Some(sp) = paths.get_mut(i) {
                        sp.extend(contours_to_path(c, group.iter().copied(), &arr.tidy).iter());
                    }
                },
            );
            // Past the limit on the passes there are no shapes, not the faces made before it was reached.
            if !whole {
                return none(PathOpsError::TooComplex);
            }
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
            let whole = each_classified_face(
                &arr,
                keys.len(),
                |w| arr.covered(w).next_back().and_then(|t| class_of_shape.get(t).copied()),
                |k, c, group| parts.push((k, contours_to_path(c, group.iter().copied(), p))),
            );
            if !whole {
                return none(PathOpsError::TooComplex);
            }
            // By key, as each key's own pass gave them (a stable sort keeps a key's faces in order).
            parts.sort_by_key(|(k, _)| *k);
            parts.into_iter().flat_map(|(k, path)| one(path, keys[k])).collect()
        }
        PathfinderOp::Crop => {
            if n < 2 {
                return Ok(Vec::new());
            }
            let mut paths = vec![BezPath::new(); n - 1];
            let whole = each_classified_face(
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
            if !whole {
                return none(PathOpsError::TooComplex);
            }
            paths.into_iter().zip(shapes).flat_map(|(sp, s)| one(sp, s.key)).collect()
        }
        // Closed shapes only: the junctions are the vertices of the filled areas' arrangement.
        PathfinderOp::Outline => match outline_edges(arr.junctions(), shapes, false) {
            Some(edges) => edges,
            None => return none(PathOpsError::TooComplex),
        },
    })
}

/// The faces of `arr`, each distinct coverage mask's in turn (sorted), each face an outer contour
/// and the holes of its mask directly inside it ([`face_groups`]); `None` past the limit on the entries
/// of the masks ([`crate::Limits::mask_work`]) or on the work of the contour passes
/// ([`crate::Limits::pass_work`]), where the faces are not all of them.
fn regions_of(arr: &Arrangement) -> Option<Vec<Region>> {
    let masks = arr.distinct_masks()?;
    let mut faces: Vec<Vec<BezPath>> = vec![Vec::new(); masks.len()];
    let whole = each_face(arr, &masks, |m, c, group| {
        if let Some(f) = faces.get_mut(m) {
            f.push(contours_to_path(c, group.iter().copied(), &arr.tidy));
        }
    });
    if !whole {
        return None;
    }
    let mut out = Vec::new();
    for (sources, paths) in masks.into_iter().zip(faces) {
        out.extend(paths.into_iter().filter(|p| !p.elements().is_empty()).map(|path| Region { path, sources: sources.clone() }));
    }
    Some(out)
}

/// Calls `f` with each face of `arr` classified by `classify: impl Fn(&Multi) -> Option<usize>`
/// into `0..num_classes`: the index of its class, the contour pass it is in, and its contours,
/// an outer contour and the holes of its class directly inside it ([`face_groups`]).
///
/// A contour pass walks the whole arrangement, so one pass per class would cost classes × edges.
/// Instead the classes go into batches whose faces share no vertex (so no edge either): one pass
/// per batch walks each face's contours exactly as its own class's pass would, and the class a
/// contour bounds is the one of its batch at its first point.
///
/// A contour pass walks every segment, so the work is the batches times the segments, and a class
/// that meets every other (a fan of shapes at one point) makes a batch of its own: counted before the
/// passes and held to [`crate::Limits::pass_work`]. False, with nothing called, when it is more; it
/// is also false if a pass made by the fallback (below) finds the allowance gone, after faces were
/// given, so a caller keeps none of them when it is false.
#[must_use = "false: the faces given are not all of them, and a caller makes no shapes"]
fn each_classified_face(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    mut f: impl FnMut(usize, &Contours, &[ContourIdx]),
) -> bool {
    if num_classes == 0 {
        return true;
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
    let segments = top.segment_indices().count();
    let planned = batches.saturating_mul(segments);
    let Some(mut left) = crate::budget::limits().pass_work.checked_sub(planned) else { return false };
    crate::work::add(crate::work::Work::PassSegments, planned);
    for b in 0..batches {
        let c = arr.contours_where(|w| classify(w).is_some_and(|m| batch[m] == b));
        let owner: Option<Vec<usize>> = c
            .contours()
            .map(|ct| {
                let Some(kurbo::PathEl::MoveTo(p)) = ct.path.elements().first() else { return None };
                around.get(&vertex_key(*p))?.iter().copied().find(|&m| batch[m] == b)
            })
            .collect();
        #[cfg(test)]
        let owner = if FORCE_FALLBACK.with(std::cell::Cell::get) { None } else { owner };
        let Some(owner) = owner else {
            // Not expected: every contour starts at a vertex of the arrangement. Fall back to a
            // pass per class for this batch.
            for m in (0..num_classes).filter(|m| batch[*m] == b) {
                // A pass for each class, charged as it is made.
                let Some(rest) = left.checked_sub(segments) else { return false };
                left = rest;
                crate::work::add(crate::work::Work::PassSegments, segments);
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
    true
}

/// Calls `f` with each face of `arr` whose coverage is one of `masks` (distinct lists of covering inputs): the index of
/// its mask in `masks`, the contour pass it is in, and its contours, an outer contour and the
/// holes of its mask directly inside it ([`face_groups`]). Each mask's faces come in the order
/// of that mask's own contour pass.
///
/// A contour pass walks the whole arrangement, so one pass per mask would cost masks × edges
/// (two blends crossing make thousands of each). Instead the masks go into batches whose faces
/// share no vertex (so no edge either): one pass per batch walks each face's contours exactly as
/// its own mask's pass would, and the mask a contour bounds is the one of its batch at its first
/// point.
#[must_use = "false: the faces given are not all of them, and a caller makes no regions"]
fn each_face(arr: &Arrangement, masks: &[Vec<usize>], f: impl FnMut(usize, &Contours, &[ContourIdx])) -> bool {
    if masks.is_empty() {
        return true;
    }
    // Each mask is found by the inputs it covers.
    let ids = CoverIds::new(masks.to_vec());
    each_classified_face(arr, masks.len(), |w| ids.of(arr, w), f)
}

/// Lists of inputs, each ascending as [`Arrangement::covered`] gives them, found by the inputs
/// that cover a winding.
struct CoverIds {
    /// The indices in `lists` of the lists with each hash ([`CoverIds::hash`]).
    by_hash: HashMap<u64, Vec<usize>>,
    lists: Vec<Vec<usize>>,
}

impl CoverIds {
    /// The lists of covering inputs, filed by their hashes.
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
    let areas: Vec<f64> = c.contours().map(|ct| crate::path::area_about_starts(&ct.path)).collect();
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
                outer(j)
                    && owner.get(j) == owner.get(i)
                    && boxes.get(j).is_some_and(|bj| bj.union(*b) == *bj)
                    && crate::winding::winding(&o.path, p) != 0
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
    try_regions_with(shapes, precision).unwrap_or_default()
}

/// [`regions_with`], and why there are no faces where there are not: the error of the arrangement
/// (past the element limit, or of the sweep), [`crate::PathOpsError::TooComplex`] past the limit on the
/// faces ([`regions_of`]), and [`crate::PathOpsError::Degenerate`] where the sweep panics.
pub(crate) fn try_regions_with(shapes: &[Shape], precision: f64) -> Result<Vec<Region>, crate::PathOpsError> {
    guarded(|| regions_of(&try_arrangement(shapes, precision)?).ok_or(crate::PathOpsError::TooComplex))
        .unwrap_or(Err(crate::PathOpsError::Degenerate))
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

/// The junctions of an Outline, in order of x and then y, in blocks of [`JUNCTION_BLOCK`] with the heights
/// each block spans: the junctions inside a box are the points between its sides in x, which a block
/// whose heights miss the box's is stepped over whole (a long segment beside a grid of a million
/// junctions would otherwise look at every one that has its x).
struct Junctions {
    points: Vec<Point>,
    /// The lowest and highest y of each block.
    blocks: Vec<(f64, f64)>,
}

/// Junctions in a block of the [`Junctions`] index.
const JUNCTION_BLOCK: usize = 64;

impl Junctions {
    /// `points`, sorted by x and then y.
    fn new(points: Vec<Point>) -> Self {
        let blocks = points
            .chunks(JUNCTION_BLOCK)
            .map(|c| c.iter().fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| (lo.min(p.y), hi.max(p.y))))
            .collect();
        Self { points, blocks }
    }

    /// Calls `visit` with each junction inside the box `b`, edges included, in order, and its place in the index, and says whether it
    /// came to the end: `visit` can stop it (by answering false), and so does `budget`. A block looked at costs
    /// a step and so does a junction in a block that has the box's heights, each charged before it is looked at.
    fn in_box(&self, b: kurbo::Rect, budget: &SearchBudget, mut visit: impl FnMut(usize, Point) -> bool) -> bool {
        let lo = self.points.partition_point(|j| j.x < b.x0);
        let hi = self.points.partition_point(|j| j.x <= b.x1);
        let mut at = lo;
        while at < hi {
            let block = at / JUNCTION_BLOCK;
            let end = ((block + 1) * JUNCTION_BLOCK).min(hi);
            if !budget.charge(1) {
                return false;
            }
            work::add(Work::JunctionSteps, 1);
            let (low, high) = self.blocks.get(block).copied().unwrap_or((f64::INFINITY, f64::NEG_INFINITY));
            if high >= b.y0 && low <= b.y1 {
                for (place, &j) in self.points.get(at..end).unwrap_or_default().iter().enumerate() {
                    if !budget.charge(1) {
                        return false;
                    }
                    work::add(Work::JunctionSteps, 1);
                    if j.y >= b.y0 && j.y <= b.y1 && !visit(at + place, j) {
                        return false;
                    }
                }
            }
            at = end;
        }
        true
    }
}

/// One subpath as the Outline takes it, with the junctions that lie on it.
struct Walk {
    key: u64,
    open: bool,
    segs: Vec<Seg>,
    /// For each segment, the junctions within reach of it: the parameter of the point nearest to each, and the junction's place in the index.
    hits: Vec<Vec<(f64, usize)>>,
}

/// The length of the control polygon of `c`: what the curve is no longer than.
fn polygon_len(c: &CubicBez) -> f64 {
    c.p0.distance(c.p1) + c.p1.distance(c.p2) + c.p2.distance(c.p3)
}

/// The junctions within `reach` of the segment `seg`, as the parameter of the point of it nearest to each and the junction's place in
/// the index; both of the places a cubic that crosses itself passes a junction at there. `None` when the searches are past their budget.
fn junctions_on(seg: &Seg, junctions: &Junctions, budget: &SearchBudget, reach: f64) -> Option<Vec<(f64, usize)>> {
    let (c, line) = (seg.c, seg.line);
    // A point within `reach` of the curve is within `reach` of its bounds, with a few units in
    // the last place of its coordinates for the rounding of both, so the other junctions can't
    // match: the box of its control points, where they lie off the curve, held junctions the
    // curve is nowhere near.
    let extent = [c.p0, c.p1, c.p2, c.p3].iter().fold(0.0f64, |m, p| m.max(p.x.abs()).max(p.y.abs()));
    let slack = reach + 16.0 * f64::EPSILON * extent;
    let b = kurbo::Shape::bounding_box(&c).inflate(slack, slack);
    let at = |t: f64| kurbo::ParamCurve::eval(&c, t);
    // A cubic that crosses itself passes through the junction there twice, and `nearest`
    // gives one of the two: both are cuts.
    let twice = if line { None } else { double_point(&c).filter(|&(s, t)| at(s).distance(at(t)) <= reach) };
    let mut hits: Vec<(f64, usize)> = Vec::new();
    let done = junctions.in_box(b, budget, |place, j| {
        if !budget.charge(NEAREST_STEPS) {
            return false;
        }
        work::add(Work::Nearest, 1);
        let nr = c.nearest(j, 1e-9);
        if nr.distance_sq.sqrt() <= reach {
            hits.push((nr.t, place));
        }
        if let Some((s, t)) = twice.filter(|&(s, _)| at(s).distance(j) <= reach) {
            hits.extend([(s, place), (t, place)]);
        }
        true
    });
    done.then_some(hits)
}

/// The junctions of `walks` that are places where paths meet: those that lie on two subpaths, and those that lie on one at two places that are
/// further apart along it than a speck (it crosses itself, or ends on itself). The others lie on one subpath at one place, and cut it
/// nowhere (a junction the planar map made of a path's way out and its way back, which part a hair).
fn real_junctions(walks: &[Walk], reach: f64) -> HashSet<usize> {
    // How far along each walk each segment starts, as the lengths of the control polygons of those before it come to.
    let along: Vec<Vec<f64>> = walks
        .iter()
        .map(|w| {
            let mut total = 0.0;
            let mut starts = vec![0.0];
            for seg in &w.segs {
                total += polygon_len(&seg.c);
                starts.push(total);
            }
            starts
        })
        .collect();
    // Where along which walk each junction lies.
    let mut places: HashMap<usize, Vec<(usize, f64)>> = HashMap::new();
    for (w, (walk, starts)) in walks.iter().zip(&along).enumerate() {
        for (k, (seg, hits)) in walk.segs.iter().zip(&walk.hits).enumerate() {
            let start = starts.get(k).copied().unwrap_or(0.0);
            for &(t, place) in hits {
                places.entry(place).or_default().push((w, start + polygon_len(&seg.c.subsegment(0.0..t))));
            }
        }
    }
    places.into_iter().filter(|(_, at)| walks_meet(at, walks, &along, 2.0 * reach)).map(|(place, _)| place).collect()
}

/// Whether a junction that lies on the walks `at` (each with how far along it) is where paths meet: on two walks, or at two places of one
/// that are more than `apart` from one another, which a closed walk measures round as well as along.
fn walks_meet(at: &[(usize, f64)], walks: &[Walk], along: &[Vec<f64>], apart: f64) -> bool {
    let Some(&(first, _)) = at.first() else { return false };
    if at.iter().any(|&(w, _)| w != first) {
        return true;
    }
    let mut spots: Vec<f64> = at.iter().map(|&(_, x)| x).collect();
    spots.sort_by(f64::total_cmp);
    let gaps = spots.windows(2).filter(|p| matches!(p, [a, b] if b - a > apart)).count();
    let closed = walks.get(first).is_some_and(|w| !w.open);
    if !closed {
        return gaps > 0;
    }
    let total = along.get(first).and_then(|a| a.last()).copied().unwrap_or(0.0);
    let round = total - (spots.last().copied().unwrap_or(0.0) - spots.first().copied().unwrap_or(0.0));
    gaps + usize::from(round > apart) >= 2
}

/// The chains of `walk` cut at its junctions (those in `real`, or all of them), each with the key of its shape.
fn cut_walk(walk: &Walk, real: Option<&HashSet<usize>>, reach: f64) -> Vec<(Vec<Seg>, u64)> {
    let mut chains: Vec<(Vec<Seg>, u64)> = Vec::new();
    // Split each segment at junctions; record which piece starts are breaks.
    let mut pieces: Vec<(Seg, bool)> = Vec::new();
    for (seg, hits) in walk.segs.iter().zip(&walk.hits) {
        let (c, line) = (seg.c, seg.line);
        let at = |t: f64| kurbo::ParamCurve::eval(&c, t);
        let mut ts: Vec<f64> = hits.iter().filter(|(_, place)| real.is_none_or(|r| r.contains(place))).map(|&(t, _)| t).collect();
        ts.sort_by(f64::total_cmp);
        // Junctions are compared where they are, not by parameter alone: a shape's own anchor and
        // a crossing computed a hair from it are one junction, and one within `reach` of an end of
        // the segment is that end. Else a sliver piece is left between them (VectorCraft issue
        // 3870). But two cuts at one point with the curve's loop between them are two.
        let speck = |a: f64, z: f64| polygon_len(&c.subsegment(a..z)) <= 2.0 * reach;
        let start_is_junction = ts.first().is_some_and(|&t| at(t).distance(c.p0) <= reach);
        let mut cuts: Vec<f64> = vec![0.0];
        for t in ts {
            let p = at(t);
            let last = cuts.last().copied().unwrap_or(0.0);
            let at_last = p.distance(at(last)) <= reach && speck(last, t);
            let at_end = p.distance(c.p3) <= reach && speck(t, 1.0);
            if !at_last && !at_end {
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
    if walk.open {
        let mut cur: Vec<Seg> = Vec::new();
        for (seg, brk) in pieces {
            if brk && !cur.is_empty() {
                chains.push((std::mem::take(&mut cur), walk.key));
            }
            cur.push(seg);
        }
        if !cur.is_empty() {
            chains.push((cur, walk.key));
        }
        return chains;
    }
    // Chain pieces between breaks.
    let first_break = pieces.iter().position(|(_, b)| *b);
    match first_break {
        None => {
            let segs: Vec<Seg> = pieces.into_iter().map(|(s, _)| s).collect();
            chains.push((segs, walk.key));
        }
        Some(k) => {
            pieces.rotate_left(k);
            let mut cur: Vec<Seg> = Vec::new();
            for (seg, brk) in pieces {
                if brk && !cur.is_empty() {
                    chains.push((std::mem::take(&mut cur), walk.key));
                }
                cur.push(seg);
            }
            if !cur.is_empty() {
                chains.push((cur, walk.key));
            }
        }
    }
    chains
}

/// Pathfinder Outline: split every shape's boundary at `junctions`, the points where three or more
/// edges meet (sorted by x, then y, so that a segment needs only the junctions inside its bounds). A
/// closed subpath comes back as the edges of its outline, an open one as the edges of the path itself,
/// with no closing edge. `None` when the searches that find the junctions on each segment, and the
/// edges that coincide, are past their budget ([`crate::Limits::search_work`], charged before each step). With `validate`, a junction cuts only
/// where paths meet (see [`real_junctions`]): the junctions of a planar map of open paths are not all of them places where paths meet.
fn outline_edges(junctions: Vec<Point>, shapes: &[Shape], validate: bool) -> Option<Vec<Shape>> {
    let budget = SearchBudget::new();
    let junctions = Junctions::new(junctions);
    let scale = shapes.iter().filter_map(|s| control_bounds(&s.path)).map(|r| r.width().max(r.height())).fold(1.0, f64::max);
    let tol = scale * 1e-7;
    let reach = tol.max(1e-6);
    // Each subpath of the shapes, with the junctions that lie on its segments.
    let mut walks: Vec<Walk> = Vec::new();
    for s in shapes {
        for sp in subs(&s.path).into_iter().map(crate::path::Sub::closed_if_ends_meet) {
            let open = !sp.closed;
            let outline = sp.chain();
            // A closed subpath of one segment is a point, or a line out and back, and no outline, unless
            // it is a cubic that loops round an area; an open one is the path.
            if outline.is_empty() || (outline.len() == 1 && !open && !sp.draws_something()) {
                continue;
            }
            let mut hits = Vec::with_capacity(outline.len());
            for seg in &outline {
                hits.push(junctions_on(seg, &junctions, &budget, reach)?);
            }
            walks.push(Walk { key: s.key, open, segs: outline, hits });
        }
    }
    // The planar map of open paths, which trace their way out and back, finds vertices where the two ways part a hair, along a curve that
    // runs level for a stretch: a path makes a junction only where it meets another, or itself.
    let real = validate.then(|| real_junctions(&walks, reach));
    let mut chains: Vec<(Vec<Seg>, u64)> = Vec::new();
    for walk in &walks {
        chains.extend(cut_walk(walk, real.as_ref(), reach));
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
        // The chains in the cells round it, each compared with this one: charged before it is, as chains
        // with distinct ends and one midpoint (a fan of long ones) all lie in one cell.
        let candidates: Vec<usize> =
            (cx - 1..=cx + 1).flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y))).filter_map(|k| grid.get(&k)).flatten().copied().collect();
        if !budget.charge(candidates.len().saturating_mul(CHAIN_COMPARE_STEPS)) {
            return None;
        }
        work::add(Work::ChainCompares, candidates.len());
        let found = candidates
            .into_iter()
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
    Some(
        out.into_iter()
            .filter_map(|(chain, key)| {
                // Chain joints are the shapes' own anchors, so there is nothing to re-merge.
                let sp = segs_to_bezpath(&chain, false)?;
                Some(Shape::new(sp, FillRule::NonZero, key))
            })
            .collect(),
    )
}

/// What comparing a chain of an Outline with one found beside it costs, in the steps of
/// [`crate::MAX_SEARCH_WORK`]: three distances and a few comparisons.
const CHAIN_COMPARE_STEPS: usize = 4;

/// The point halfway along the chain `c`, by length.
///
/// Each segment is measured to a millionth, or to a billionth of its control polygon where that is
/// coarser. kurbo's solver for where a length runs out steps by the accuracy over the length, and
/// takes as many halvings as `log2(length / accuracy)`: a fixed 1e-6 asks it for 2^64 of them or
/// more on a curve longer than about 9e12 (a shift past the width of its integer, a panic where
/// arithmetic is checked), and a relative accuracy keeps them to about thirty at any size, and the
/// measuring of each segment to a few levels (the same 1e-6 for a segment up to a thousand long).
fn mid_point(c: &[Seg]) -> Point {
    let accuracy = |s: &CubicBez| (1e-9 * (s.p0.distance(s.p1) + s.p1.distance(s.p2) + s.p2.distance(s.p3))).max(1e-6);
    let lens: Vec<f64> = c.iter().map(|s| kurbo::ParamCurveArclen::arclen(&s.c, accuracy(&s.c))).collect();
    let total: f64 = lens.iter().sum();
    let mut acc = 0.0;
    for (s, l) in c.iter().zip(&lens) {
        if acc + l >= total / 2.0 && *l > 0.0 {
            let t = kurbo::ParamCurveArclen::inv_arclen(&s.c, total / 2.0 - acc, accuracy(&s.c));
            return s.c.eval(t);
        }
        acc += l;
    }
    c.first().map_or(Point::ZERO, |s| s.c.p0)
}

/// The two parameters, `s < t` inside `c`, at which the cubic `c` passes through one point (where it
/// crosses itself), if it does.
///
/// In the power basis `B(t) = a t³ + b t² + c₁ t + d`, `B(s) = B(t)` for `s ≠ t` is, divided by
/// `s - t`, `a (s² + s t + t²) + b (s + t) + c₁ = 0`: with `σ = s + t` and `π = s t`, `a (σ² - π) +
/// b σ + c₁ = 0`. Crossed with `a` that is `(a × b) σ + a × c₁ = 0`, and along `a` it gives `π`; `s` and
/// `t` are the roots of `x² - σ x + π`.
fn double_point(c: &CubicBez) -> Option<(f64, f64)> {
    let (p0, p1, p2, p3) = (c.p0.to_vec2(), c.p1.to_vec2(), c.p2.to_vec2(), c.p3.to_vec2());
    let a = p3 - p2 * 3.0 + p1 * 3.0 - p0;
    let b = (p2 - p1 * 2.0 + p0) * 3.0;
    let c1 = (p1 - p0) * 3.0;
    let axb = a.cross(b);
    let aa = a.hypot2();
    // `a` and `b` along one line (or a quadratic, `a` nothing): no single point it passes twice.
    if !(axb.is_finite() && aa.is_finite()) || axb.abs() <= 1e-12 * aa.sqrt() * b.hypot() || aa <= 0.0 {
        return None;
    }
    let sigma = -a.cross(c1) / axb;
    let pi = sigma * sigma + (a.dot(b) * sigma + a.dot(c1)) / aa;
    let disc = sigma * sigma - 4.0 * pi;
    if disc.is_nan() || disc <= 0.0 {
        return None;
    }
    let r = disc.sqrt();
    let (s, t) = ((sigma - r) / 2.0, (sigma + r) / 2.0);
    (s > 0.0 && t < 1.0).then_some((s, t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::boolean::CONTOUR_PASSES;
    use crate::testutil::{ellipse, length, line, polyline, polyline_pts, rect, subpath_count};
    use crate::work::{self, Work};
    use kurbo::Rect;
    use proptest::prelude::*;

    /// The faces one contour pass per mask finds: what [`regions_of`] must match exactly when no
    /// face lies inside a hole of its own mask (`grouped` puts such an island in the frame's face).
    fn regions_one_mask_at_a_time(arr: &Arrangement) -> Vec<Region> {
        let mut out = Vec::new();
        for sources in arr.distinct_masks().expect("within the limit") {
            let c = arr.contours(|m| m.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).eq(sources.iter().copied()));
            for g in c.grouped() {
                let path = contours_to_path(&c, g, &arr.tidy);
                if !path.elements().is_empty() {
                    out.push(Region { path, sources: sources.clone() });
                }
            }
        }
        out
    }

    /// The distinct non-empty coverages next to some edge as a flag per input, sorted.
    fn flag_masks(arr: &Arrangement) -> Vec<Vec<bool>> {
        let mut set = std::collections::BTreeSet::new();
        for s in arr.top.segment_indices() {
            let h = s.first_half();
            for w in [arr.top.winding_clockwise(h), arr.top.winding_counter_clockwise(h)] {
                let m = arr.mask(w);
                if m.iter().any(|&b| b) {
                    set.insert(m);
                }
            }
        }
        set.into_iter().collect()
    }

    /// [`Arrangement::distinct_masks`] lists the inputs of each of [`flag_masks`] in its order, and
    /// [`regions_of`] finds the faces one contour pass per flag mask finds, in the same order.
    fn same_as_one_mask_at_a_time(shapes: &[Shape]) {
        let arr = arrangement(shapes, DEFAULT_PRECISION).expect("arrangement");
        let listed: Vec<Vec<usize>> = flag_masks(&arr).iter().map(|m| m.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect()).collect();
        assert_eq!(arr.distinct_masks(), Some(listed));
        assert_eq!(regions_of(&arr), Some(regions_one_mask_at_a_time(&arr)));
    }

    /// The coverages come in the order of their flag masks, with the same regions, for shapes that
    /// overlap, nest, touch or lie apart, and for more than 64 shapes stacked out of their order
    /// across the page (VectorCraft 3fcd157's test, on the same sets).
    #[test]
    fn coverages_keep_the_order_of_their_flag_masks() {
        let r = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::new(rect(x0, y0, x1, y1), FillRule::NonZero, 1);
        let overlapping = vec![r(0.0, 0.0, 100.0, 100.0), r(50.0, 50.0, 150.0, 150.0), r(25.0, 75.0, 125.0, 175.0)];
        let nested = vec![r(0.0, 0.0, 300.0, 300.0), r(50.0, 50.0, 250.0, 250.0), r(100.0, 100.0, 200.0, 200.0), r(140.0, 140.0, 160.0, 160.0)];
        let touching = vec![r(0.0, 0.0, 100.0, 100.0), r(100.0, 0.0, 200.0, 100.0), r(0.0, 100.0, 100.0, 200.0), r(100.0, 100.0, 200.0, 200.0)];
        let apart = vec![r(0.0, 0.0, 10.0, 10.0), r(50.0, 0.0, 60.0, 10.0), r(0.0, 50.0, 10.0, 60.0)];
        // 70 strips, each overlapping its neighbors, stacked in an order other than across the page, and two bars across them.
        let mut many: Vec<Shape> = (0..70)
            .map(|i| {
                let x = 10.0 * f64::from((i * 37) % 70);
                r(x, 0.0, x + 15.0, 300.0)
            })
            .collect();
        many.insert(20, r(-10.0, 100.0, 730.0, 110.0));
        many.push(r(-10.0, 200.0, 730.0, 205.0));
        for (what, shapes) in [("overlapping", overlapping), ("nested", nested), ("touching", touching), ("apart", apart), ("many", many)] {
            assert!(!regions(&shapes).is_empty(), "{what}");
            same_as_one_mask_at_a_time(&shapes);
        }
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

    /// Divide lists the inputs that cover each side of each edge, not a flag for every input: for shapes
    /// that lie apart the work is in proportion to their number (it was four times the square: 80,000
    /// flags for 100 shapes, 1,280,000 for 400).
    #[test]
    fn divide_of_shapes_apart_walks_entries_in_proportion_to_their_number() {
        let walked = |n: u32| {
            let shapes: Vec<Shape> = (0..n)
                .map(|i| Shape::new(rect(f64::from(i) * 20.0, 0.0, f64::from(i) * 20.0 + 10.0, 10.0), FillRule::NonZero, u64::from(i)))
                .collect();
            work::reset();
            let divided = pathfinder(PathfinderOp::Divide, &shapes);
            assert_eq!(divided.len(), n as usize);
            // In the order of the flags of every input, which puts the later shape's face first.
            assert!(
                divided.iter().rev().enumerate().all(|(i, s)| s.key == i as u64 && (crate::area(&s.path, FillRule::NonZero) - 100.0).abs() < 1e-9)
            );
            let regions = crate::regions(&shapes);
            assert_eq!(regions.len(), n as usize);
            assert!(regions.iter().rev().enumerate().all(|(i, r)| r.sources == [i]));
            (work::take(Work::MaskEntries), work::take(Work::CoverEntries))
        };
        let ((small, small_lists), (large, large_lists)) = (walked(100), walked(400));
        assert_eq!((small_lists, large_lists), (0, 0), "no list of the inputs covering a winding is made for a predicate");
        // Four times the shapes, four times the entries (not sixteen).
        assert!(small > 0 && large == 4 * small, "{small} entries for 100 shapes, {large} for 400");
    }

    /// Past `mask_work` entries Divide and `regions` give nothing, and stop walking at the limit: with
    /// exactly the entries it needs it divides, with one less it walks no further than that.
    #[test]
    fn divide_stops_at_the_mask_limit_before_walking_past_it() {
        // Overlapping circles: faces are covered by several shapes, the entries are many.
        let shapes: Vec<Shape> = (0..14u32)
            .map(|i| Shape::new(ellipse(Rect::new(f64::from(i) * 7.0, 0.0, f64::from(i) * 7.0 + 40.0, 40.0)), FillRule::NonZero, u64::from(i)))
            .collect();
        work::reset();
        let free = pathfinder(PathfinderOp::Divide, &shapes);
        let needed = work::take(Work::MaskEntries);
        assert!(free.len() > 14 && needed > 14 * 8, "{} shapes from {needed} entries", free.len());
        let at = crate::Limits { mask_work: needed, ..crate::Limits::default() };
        assert_eq!(crate::with_limits(at, || pathfinder(PathfinderOp::Divide, &shapes)), free);
        let under = crate::Limits { mask_work: needed - 1, ..crate::Limits::default() };
        work::reset();
        assert!(crate::with_limits(under, || pathfinder(PathfinderOp::Divide, &shapes)).is_empty());
        let walked = work::take(Work::MaskEntries);
        assert!(walked < needed, "{walked} entries walked with a limit of {}", needed - 1);
        assert!(crate::with_limits(under, || crate::regions(&shapes)).is_empty());
        // Lowest: nothing at all is walked.
        let none = crate::Limits { mask_work: 0, ..crate::Limits::default() };
        work::reset();
        assert!(crate::with_limits(none, || crate::regions(&shapes)).is_empty());
        assert_eq!(work::take(Work::MaskEntries), 0);
    }

    /// Lists of covering inputs sort as the flags of every input did.
    #[test]
    fn lists_of_covering_inputs_sort_as_flags_did() {
        use crate::boolean::cmp_masks;
        let n = 6;
        let flags = |ids: &[usize]| -> Vec<bool> { (0..n).map(|i| ids.contains(&i)).collect() };
        let subsets: Vec<Vec<usize>> = (1u32..(1 << n)).map(|bits| (0..n).filter(|i| bits >> i & 1 == 1).collect()).collect();
        for a in &subsets {
            for b in &subsets {
                assert_eq!(cmp_masks(a, b), flags(a).cmp(&flags(b)), "{a:?} against {b:?}");
            }
        }
    }

    /// `k` thin triangles that meet at the origin, each its own key, and `squares` squares in a column
    /// apart from them.
    fn fan_and_squares(k: usize, squares: usize) -> Vec<Shape> {
        let mut shapes: Vec<Shape> = (0..k)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / k as f64;
                let b = a + std::f64::consts::TAU / (3.0 * k as f64);
                let p =
                    polyline_pts(&[Point::ZERO, Point::new(100.0 * a.cos(), 100.0 * a.sin()), Point::new(100.0 * b.cos(), 100.0 * b.sin())], true);
                Shape::new(p, FillRule::NonZero, i as u64)
            })
            .collect();
        let mut col = BezPath::new();
        for j in 0..squares {
            let y = 3.0 * j as f64;
            col.extend(rect(1_000.0, y, 1_001.0, y + 1.0).iter());
        }
        shapes.push(Shape::new(col, FillRule::NonZero, 1_000_000));
        shapes
    }

    /// Classes that meet at one point are each a batch, and a pass walks every segment for each batch:
    /// the batches times the segments are counted before the first pass and held to `pass_work`, in
    /// Merge, Divide, Trim and Crop. (A fan of 208 shapes beside 40,000 squares takes 2.2 s to merge.)
    #[test]
    fn a_fan_of_shapes_at_a_point_is_charged_its_passes_before_the_first() {
        let fan = fan_and_squares(30, 300);
        // Crop keeps what is inside the front shape: one over the fan, which Merge, Divide and Trim would hide.
        let mut covered = fan.clone();
        covered.push(Shape::new(rect(-200.0, -200.0, 200.0, 200.0), FillRule::NonZero, 2_000_000));
        for (op, shapes) in [(PathfinderOp::Merge, &fan), (PathfinderOp::Divide, &fan), (PathfinderOp::Trim, &fan), (PathfinderOp::Crop, &covered)] {
            let segments = arrangement(shapes, DEFAULT_PRECISION).unwrap().top.segment_indices().count();
            CONTOUR_PASSES.with(|n| n.set(0));
            work::reset();
            let free = pathfinder(op, shapes);
            let (passes, charged) = (CONTOUR_PASSES.with(std::cell::Cell::get), work::take(Work::PassSegments));
            assert!(!free.is_empty(), "{op:?}");
            // One pass for each batch: the 30 shapes at the origin are 30 batches, the squares share one.
            assert!(passes >= 30, "{op:?}: {passes} passes");
            assert_eq!(charged, passes * segments, "{op:?}: {charged} charged for {passes} passes of {segments} segments");
            // At the limit the operation is made, under it nothing is walked.
            let at = crate::Limits { pass_work: charged, ..crate::Limits::default() };
            assert_eq!(crate::with_limits(at, || pathfinder(op, shapes)), free, "{op:?}");
            let under = crate::Limits { pass_work: charged - 1, ..crate::Limits::default() };
            CONTOUR_PASSES.with(|n| n.set(0));
            work::reset();
            assert!(crate::with_limits(under, || pathfinder(op, shapes)).is_empty(), "{op:?}");
            assert_eq!((CONTOUR_PASSES.with(std::cell::Cell::get), work::take(Work::PassSegments)), (0, 0), "{op:?}: refused before the first pass");
        }
        // Shapes apart are a batch whatever their number: the charge is the segments, once.
        let apart = fan_and_squares(0, 300);
        work::reset();
        assert_eq!(pathfinder(PathfinderOp::Merge, &apart).len(), 300, "a shape for each component");
        assert_eq!(work::take(Work::PassSegments), arrangement(&apart, DEFAULT_PRECISION).unwrap().top.segment_indices().count());
    }

    /// A walk that finds its allowance gone part way through (the fallback, which no input needs, makes
    /// a pass for each class and charges it) has given some faces by then, and gives no shapes: the
    /// walk says so, and Trim, Merge, Crop and Divide keep none of what it gave.
    #[test]
    fn a_walk_that_runs_out_part_way_gives_no_shapes() {
        struct Forced;
        impl Drop for Forced {
            fn drop(&mut self) {
                FORCE_FALLBACK.with(|f| f.set(false));
            }
        }
        let squares: Vec<Shape> = (0..3u32)
            .map(|i| Shape::new(rect(f64::from(i) * 100.0, 0.0, f64::from(i) * 100.0 + 10.0, 10.0), FillRule::NonZero, u64::from(i)))
            .collect();
        let mut covered = squares.clone();
        covered.push(Shape::new(rect(-10.0, -10.0, 300.0, 20.0), FillRule::NonZero, 9));
        let segments = |shapes: &[Shape]| arrangement(shapes, DEFAULT_PRECISION).unwrap().top.segment_indices().count();
        for (op, shapes, expected) in [
            (PathfinderOp::Trim, &squares, 3),
            (PathfinderOp::Merge, &squares, 3),
            (PathfinderOp::Divide, &squares, 3),
            (PathfinderOp::Crop, &covered, 3),
        ] {
            let s = segments(shapes);
            // The classes share no vertex: one batch, so one pass planned, and the fallback needs one for each class.
            let room = |passes: usize| crate::Limits { pass_work: s * (1 + passes), ..crate::Limits::default() };
            assert_eq!(pathfinder(op, shapes).len(), expected, "{op:?} by the batches");
            let _forced = Forced;
            FORCE_FALLBACK.with(|f| f.set(true));
            // Room for all three passes of the fallback: the same shapes.
            assert_eq!(crate::with_limits(room(3), || pathfinder(op, shapes)).len(), expected, "{op:?} by the fallback");
            // Room for one: it runs out after the first class's faces were given, and none are kept.
            work::reset();
            assert!(crate::with_limits(room(1), || pathfinder(op, shapes)).is_empty(), "{op:?} with room for one pass");
            assert_eq!(work::take(Work::PassSegments), 2 * s, "the planned pass and the one the fallback made");
            // And with room for none, nothing at all.
            assert!(crate::with_limits(room(0), || pathfinder(op, shapes)).is_empty(), "{op:?} with room for none");
        }
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
        assert_eq!(passes, 1, "{passes} passes for 300 keys");
    }

    /// Merge with one contour pass for each distinct key, each face of a key its own result (what Merge
    /// was before it went by batches; VectorCraft's `old_merge` of be0e934's test).
    fn merge_a_pass_for_each_key(shapes: &[Shape], precision: f64) -> Vec<Shape> {
        let Some(arr) = arrangement(shapes, precision) else { return Vec::new() };
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
    }

    /// Overlapping squares on a grid with repeated fills, and touching squares with two fills: Merge
    /// gives the same results, in the same order, as a contour pass for each fill (VectorCraft be0e934's
    /// test; its other two inputs, a fill's island inside a hole of the same fill, are the one place Merge
    /// differs, the island being a shape of its own here: the test after this one).
    #[test]
    fn merge_matches_a_contour_pass_per_fill() {
        let mut grid = Vec::new();
        for row in 0..5u32 {
            for col in 0..5u32 {
                let (x, y) = (10.0 * f64::from(col), 10.0 * f64::from(row));
                grid.push(Shape::new(rect(x, y, x + 15.0, y + 15.0), FillRule::NonZero, u64::from((row * 5 + col) * 7 % 4)));
            }
        }
        let touching: Vec<Shape> = (0..16u32)
            .map(|k| {
                let (x, y) = (10.0 * f64::from(k % 4), 10.0 * f64::from(k / 4));
                Shape::new(rect(x, y, x + 10.0, y + 10.0), FillRule::NonZero, u64::from((k % 4 + k / 4) % 2))
            })
            .collect();
        for (label, shapes) in [("grid", grid), ("touching", touching)] {
            let actual = pathfinder_with(PathfinderOp::Merge, &shapes, DEFAULT_PRECISION);
            let expected = merge_a_pass_for_each_key(&shapes, DEFAULT_PRECISION);
            assert!(!expected.is_empty(), "{label}");
            assert_eq!(actual, expected, "{label}");
        }
    }

    /// 40 squares in a row, each overlapping the next and each with a fill of its own: Merge walks the
    /// arrangement a few times, not once for each fill, at chained sizes: the passes stay the same as the
    /// squares double (VectorCraft be0e934's test, at 40, 80 and 160), and the segments the passes are
    /// charged are the batches times the segments, which two classes that meet need two of.
    #[test]
    fn merge_with_many_fills_walks_the_arrangement_a_few_times() {
        let mut last = None;
        for count in [40u32, 80, 160] {
            let v: Vec<Shape> = (0..count)
                .map(|k| {
                    let x = 10.0 * f64::from(k);
                    Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, u64::from(k))
                })
                .collect();
            CONTOUR_PASSES.with(|n| n.set(0));
            work::reset();
            let merged = pathfinder_with(PathfinderOp::Merge, &v, DEFAULT_PRECISION);
            let (passes, charged) = (CONTOUR_PASSES.with(std::cell::Cell::get), work::take(Work::PassSegments));
            assert!(passes <= 8, "{count} squares: {passes} contour passes");
            assert_eq!(merged.len(), count as usize);
            assert!(merged.iter().enumerate().all(|(k, s)| s.key == k as u64));
            // Each pass is charged its segments before it is made: the charge is the passes' (batches') times the segments.
            let segments = arrangement(&v, DEFAULT_PRECISION).map(|a| a.top.segment_indices().count()).unwrap_or(0);
            assert_eq!(charged, passes * segments, "{count} squares: {charged} charged for {passes} passes of {segments} segments");
            if let Some((last_passes, last_charged)) = last {
                assert_eq!(passes, last_passes, "the passes do not grow with the squares");
                assert!(charged < 3 * last_charged, "{last_charged} charged for half the squares, {charged}: in proportion to them");
            }
            last = Some((passes, charged));
        }
    }

    /// Merge by batches gives what a contour pass for each key gave: the same shapes, in the same order.
    #[test]
    fn merge_is_what_a_pass_for_each_key_gives() {
        let reference = |shapes: &[Shape]| merge_a_pass_for_each_key(shapes, DEFAULT_PRECISION);
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

    // ---- the work after the map

    /// `g` horizontal and `g` vertical open lines across one another (`g` squared junctions), and `arches` open
    /// cubics that run round the grid at 50 or more from it, whose bounds hold every junction of it.
    fn grid_and_arches(g: usize, arches: usize) -> Vec<Shape> {
        let span = g as f64 + 1.0;
        let mut shapes = Vec::new();
        for i in 1..=g {
            let k = i as f64;
            shapes.push(Shape::new(polyline(&[(-1.0, k), (span, k)], false), FillRule::NonZero, 1));
            shapes.push(Shape::new(polyline(&[(k, -1.0), (k, span)], false), FillRule::NonZero, 2));
        }
        for a in 0..arches {
            let mut arch = BezPath::new();
            arch.move_to((-50.0, -50.0));
            arch.curve_to((-50.0, 4.0 * span + a as f64), (span + 50.0, 4.0 * span + a as f64), (span + 50.0, -50.0));
            shapes.push(Shape::new(arch, FillRule::NonZero, 3));
        }
        shapes
    }

    /// The searches that cut an Outline's edges at the junctions are charged before each step and held to
    /// [`crate::Limits::search_work`]: a grid of cutters makes junctions that lie in the bounds of every arch
    /// round it, where the curves are nowhere near, and each is a search for the nearest point.
    #[test]
    fn the_searches_of_an_outline_are_charged_before_they_are_made_and_held_to_the_limit() {
        let (g, arches) = (30usize, 20usize);
        let mut shapes = grid_and_arches(g, arches);
        // A copy of the first line under another key: its chains coincide with that one's, and are compared with them.
        shapes.push(Shape::new(shapes[0].path.clone(), FillRule::NonZero, 9));
        let measured = |limit: usize| {
            let l = crate::Limits { search_work: limit, ..crate::Limits::default() };
            work::reset();
            let r = crate::with_limits(l, || try_pathfinder(PathfinderOp::Outline, &shapes));
            (r, work::take(Work::SearchSteps), work::take(Work::Nearest), work::take(Work::JunctionSteps), work::take(Work::ChainCompares))
        };
        let (done, charged, nearest, steps, compares) = measured(usize::MAX);
        let done = done.expect("within the limits");
        assert!(done.len() > 2 * g, "the lines cut at their junctions, and the arches");
        // Each arch's bounds hold every junction of the grid: a search apiece, besides those on the lines.
        assert!(nearest >= arches * g * g, "{nearest} searches");
        assert!(compares > 0, "the chains of the copy are compared with those they coincide with");
        // Every step charged was made, and every step made was charged: the junctions and blocks looked at, the
        // searches, and the chains compared.
        assert_eq!(charged, crate::budget::NEAREST_STEPS * nearest + steps + CHAIN_COMPARE_STEPS * compares, "{nearest} searches, {steps} steps");
        // Short of the work, the error, and everything done was charged first: no more than the limit.
        for limit in [charged - 1, charged / 2, charged / 100, 1, 0] {
            let (r, c, n, s, k) = measured(limit);
            assert_eq!(r, Err(crate::PathOpsError::TooComplex), "{limit}");
            assert!(
                c <= limit && crate::budget::NEAREST_STEPS * n + s + CHAIN_COMPARE_STEPS * k <= c,
                "{limit}: {c} charged, {n} searches, {s} steps and {k} chains compared made"
            );
        }
        // At the work it needs, the same edges; and the forms with no error give none past it.
        assert_eq!(measured(charged).0, Ok(done.clone()));
        let low = crate::Limits { search_work: charged - 1, ..crate::Limits::default() };
        assert_eq!(crate::with_limits(low, || pathfinder(PathfinderOp::Outline, &shapes)), Vec::new());
        assert_eq!(pathfinder(PathfinderOp::Outline, &shapes), done);
    }

    /// A segment far from a grid costs the blocks of the junctions that have its x, not each of them: the
    /// junctions are in blocks with the heights they span, and a block that misses the segment's is stepped over.
    #[test]
    fn a_segment_far_from_a_grid_steps_over_its_junctions_by_blocks() {
        let g = 40usize;
        let near = grid_and_arches(g, 0);
        let mut far = near.clone();
        far.push(Shape::new(polyline(&[(-1.0, 1e4), (g as f64 + 2.0, 1e4)], false), FillRule::NonZero, 4));
        let steps = |shapes: &[Shape]| {
            work::reset();
            try_pathfinder(PathfinderOp::Outline, shapes).expect("within the limits");
            (work::take(Work::JunctionSteps), work::take(Work::Nearest))
        };
        let ((without, searches), (with, with_searches)) = (steps(&near), steps(&far));
        assert_eq!(searches, with_searches, "no junction is searched for beside the far segment");
        let blocks = (g * g).div_ceil(JUNCTION_BLOCK);
        assert!(with - without <= blocks + 1, "{} steps for a segment over a grid of {} junctions in {blocks} blocks", with - without, g * g);
        assert!(with > without, "the blocks are looked at");
    }

    /// The same for the Outline of closed shapes, whose junctions come from the filled arrangement.
    #[test]
    fn the_outline_of_closed_shapes_is_held_to_the_limit_too() {
        let g = 8usize;
        let side = 2.0 * g as f64 + 1.0;
        let mut shapes = Vec::new();
        for i in 0..g {
            let k = 2.0 * i as f64;
            shapes.push(Shape::new(rect(-1.0, k, side, k + 0.5), FillRule::NonZero, 1));
            shapes.push(Shape::new(rect(k, -1.0, k + 0.5, side), FillRule::NonZero, 2));
        }
        for a in 0..3 {
            let mut arch = BezPath::new();
            arch.move_to((-50.0, -50.0));
            arch.curve_to((-50.0, 4.0 * side + f64::from(a)), (side + 50.0, 4.0 * side + f64::from(a)), (side + 50.0, -50.0));
            arch.close_path();
            shapes.push(Shape::new(arch, FillRule::NonZero, 3));
        }
        let measured = |limit: usize| {
            let l = crate::Limits { search_work: limit, ..crate::Limits::default() };
            work::reset();
            let r = crate::with_limits(l, || try_pathfinder(PathfinderOp::Outline, &shapes));
            (r, work::take(Work::SearchSteps), work::take(Work::Nearest))
        };
        let (done, charged, nearest) = measured(usize::MAX);
        let done = done.expect("within the limits");
        assert!(nearest >= 3 * g * g, "{nearest} searches");
        for limit in [charged - 1, charged / 2, 0] {
            let (r, c, n) = measured(limit);
            assert_eq!(r, Err(crate::PathOpsError::TooComplex), "{limit}");
            assert!(c <= limit && crate::budget::NEAREST_STEPS * n <= c, "{limit}: {c} charged, {n} searches made");
        }
        assert_eq!(measured(charged).0, Ok(done));
        let low = crate::Limits { search_work: charged - 1, ..crate::Limits::default() };
        assert_eq!(crate::with_limits(low, || pathfinder(PathfinderOp::Outline, &shapes)), Vec::new());
    }

    /// A junction is searched for on a curve only where it is within the curve's own bounds (widened by the noise
    /// of the sweep), not within the box of its control points, which a curve whose handles run out beyond it
    /// has a good deal larger: a cross of lines between the top of an arch and its handles is no search of it.
    #[test]
    fn a_junction_outside_the_bounds_of_a_curve_is_not_searched_for_on_it_whatever_its_control_points_hold() {
        let mut shapes = Vec::new();
        // The arch from (0, 0) to (100, 0), whose handles are at 400: it rises to 300.
        let mut arch = BezPath::new();
        arch.move_to((0.0, 0.0));
        arch.curve_to((0.0, 400.0), (100.0, 400.0), (100.0, 0.0));
        shapes.push(Shape::new(arch, FillRule::NonZero, 0));
        // Three lines across three, at a height of 350 and a little more, between its top and its handles.
        for i in 0..3 {
            let k = 40.0 + 10.0 * f64::from(i);
            shapes.push(Shape::new(polyline(&[(30.0, 350.0 + k / 4.0), (80.0, 350.0 + k / 4.0)], false), FillRule::NonZero, 1));
            shapes.push(Shape::new(polyline(&[(k, 340.0), (k, 390.0)], false), FillRule::NonZero, 2));
        }
        work::reset();
        let edges = try_pathfinder(PathfinderOp::Outline, &shapes).expect("within the limits");
        assert!(edges.len() > 6);
        // The six lines each search for the three junctions on them: 18. The arch's control box holds all
        // nine and its bounds none.
        assert_eq!(work::take(Work::Nearest), 18);
    }

    /// The straight segment from `p` to `q`, a cubic whose handles are a third of the way: its parameter is the fraction of its length.
    fn straight(p: Point, q: Point) -> Seg {
        Seg { c: CubicBez::new(p, p.lerp(q, 1.0 / 3.0), p.lerp(q, 2.0 / 3.0), q), line: true }
    }

    /// A walk of the given segments, with junction 0 on the given segments at the given parameters.
    fn walk_with(open: bool, segs: Vec<Seg>, hits: Vec<(usize, f64)>) -> Walk {
        let mut by_seg: Vec<Vec<(f64, usize)>> = vec![Vec::new(); segs.len()];
        for (k, t) in hits {
            by_seg[k].push((t, 0));
        }
        Walk { key: 0, open, segs, hits: by_seg }
    }

    /// A junction is where paths meet: on two walks, or on one at two places that are further apart along it than 2 reaches (two lie
    /// on a path that crosses itself); a path's two ways at one place, a hair apart, are not two, however many segments they lie on.
    #[test]
    fn a_junction_on_one_walk_is_real_only_at_two_places_more_than_two_reaches_apart() {
        let reach = 0.01;
        let line10 = || vec![straight(Point::new(0.0, 0.0), Point::new(10.0, 0.0))];
        let real = |walks: &[Walk]| real_junctions(walks, reach).contains(&0);
        // A path's hits 0.019 apart (under 2 reaches, 0.02) are one place, and 0.021 apart two.
        assert!(!real(&[walk_with(true, line10(), vec![(0, 0.1), (0, 0.1019)])]));
        assert!(real(&[walk_with(true, line10(), vec![(0, 0.1), (0, 0.1021)])]));
        // A single hit is no meeting, and the two ends of an open path are two places.
        assert!(!real(&[walk_with(true, line10(), vec![(0, 0.5)])]));
        assert!(real(&[walk_with(true, line10(), vec![(0, 0.0), (0, 1.0)])]));
        // The hits of two walks make a junction whatever their places, a hit of each.
        let a = walk_with(true, line10(), vec![(0, 0.5)]);
        let b = walk_with(true, line10(), vec![(0, 0.5)]);
        assert!(real(&[a, b]));
        // A ring (a square of side 10: 40 round) comes back to where it started: its start and its end are one place, and so are hits at
        // either side of them; two places on it far apart both ways are two.
        let square = || {
            let p = [Point::new(0.0, 0.0), Point::new(10.0, 0.0), Point::new(10.0, 10.0), Point::new(0.0, 10.0), Point::new(0.0, 0.0)];
            p.windows(2).map(|w| straight(w[0], w[1])).collect::<Vec<_>>()
        };
        assert!(!real(&[walk_with(false, square(), vec![(0, 0.0), (3, 1.0)])]));
        assert!(!real(&[walk_with(false, square(), vec![(0, 0.0), (0, 0.0004), (3, 0.9996)])]));
        assert!(real(&[walk_with(false, square(), vec![(0, 0.5), (2, 0.5)])]));
        // The same hits on an open path of those segments are two places: its ends.
        assert!(real(&[walk_with(true, square(), vec![(0, 0.0), (3, 1.0)])]));
    }
}
