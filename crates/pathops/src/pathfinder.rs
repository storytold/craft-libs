//! Illustrator's Pathfinder panel and Shape Builder regions, over an ordered stack of shapes
//! (index 0 = back-most, last = front-most).

use std::collections::{HashMap, HashSet};
use std::ops::ControlFlow;

use craft_geom::{FillRule, PathData};
use kurbo::{BezPath, Line, ParamCurve, ParamCurveNearest, PathSeg, Point, Shape as _};
use linesweeper::topology::{ContourIdx, Contours};

use crate::boolean::{
    Arrangement, DEFAULT_PRECISION, Multi, Seg, Tidy, all_contours_to_path, contours_to_path, eps_for, fill_bezpath, normalize_bez, segs_to_subpath,
    to_seg, unite_all, unite_all_with,
};

#[cfg(test)]
thread_local! {
    /// The face outlines [`regions_of_within`] has built on this thread.
    static FACE_PATHS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// A filled shape in a Pathfinder stack. `key` identifies its paint (e.g. a hashed fill colour);
/// results carry the key of the object whose paint they keep.
#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub path: PathData,
    pub rule: FillRule,
    pub key: u64,
}

impl Shape {
    pub fn new(path: PathData, rule: FillRule, key: u64) -> Self {
        Self { path, rule, key }
    }
}

/// Pathfinder panel and effect operations.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
    /// Effect only: the faces of the filled arrangement, each keyed by its front-most shape. For
    /// closed inputs these are [`PathfinderOp::Divide`]'s faces; an open path keeps its implicit
    /// closure here, where Divide treats it as a cutting edge. Hard Mix takes the highest of each
    /// CMYK channel of the shapes covering a face (Illustrator Effect → Pathfinder → Hard Mix);
    /// this crate holds no paint, so the caller mixes it from [`regions`], whose
    /// [`Region::sources`] lists those shapes back to front.
    HardMix,
    /// Effect only: the faces of the filled arrangement, as with [`PathfinderOp::HardMix`]. Soft
    /// Mix blends fills in CMYK bottom to top by a mixing rate the caller holds (Illustrator
    /// Effect → Pathfinder → Soft Mix), mixed from [`regions`].
    SoftMix,
}

/// One face of the planar arrangement of a set of shapes (Shape Builder / Live Paint face).
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    /// Face outline (outer contour plus holes, consistently oriented).
    pub path: PathData,
    /// Indices of the input shapes covering this face, ascending.
    pub sources: Vec<usize>,
}

impl Region {
    /// Front-most covering shape (`None` only for an uncovered face, which regions never are).
    pub fn top(&self) -> Option<usize> {
        self.sources.last().copied()
    }
    /// Does the face contain `p`?
    pub fn contains(&self, p: Point) -> bool {
        self.path.to_bezpath().winding(p) != 0
    }
}

/// The arrangement of `shapes`, its faces refit to `precision` and its slivers thinner than it dropped.
fn arrangement(shapes: &[Shape], precision: f64) -> Option<Arrangement> {
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

fn one(path: PathData, key: u64) -> Vec<Shape> {
    if path.is_empty() { Vec::new() } else { vec![Shape::new(path, FillRule::NonZero, key)] }
}

/// Run a Pathfinder operation over `shapes` (back → front).
pub fn pathfinder(op: PathfinderOp, shapes: &[Shape]) -> Vec<Shape> {
    pathfinder_with(op, shapes, DEFAULT_PRECISION)
}

/// [`pathfinder`] at Pathfinder Options › Precision: results are refit to within `precision`
/// (document units) and pieces thinner than it are dropped. A smaller value follows the inputs more
/// closely. A value that isn't positive and finite is [`DEFAULT_PRECISION`].
pub fn pathfinder_with(op: PathfinderOp, shapes: &[Shape], precision: f64) -> Vec<Shape> {
    let precision = if precision.is_finite() && precision > 0.0 { precision } else { DEFAULT_PRECISION };
    let n = shapes.len();
    if n == 0 {
        return Vec::new();
    }
    let front_key = shapes[n - 1].key;
    let unite = || {
        let v: Vec<(&PathData, FillRule)> = shapes.iter().map(|s| (&s.path, s.rule)).collect();
        one(unite_all_with(&v, precision), front_key)
    };
    // Unite doesn't need the arrangement.
    if op == PathfinderOp::Unite {
        return unite();
    }
    // Open paths are cutting edges for Divide; the filled-only operations keep their
    // implicit closure. Reuse Shape Builder's bounded planar map and its fallback.
    if op == PathfinderOp::Divide && crate::has_open_path(shapes) {
        return crate::planar::builder(shapes, false, 0.0, precision)
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
            let mut subpaths = vec![Vec::new(); n];
            each_classified_face(
                &arr,
                n,
                |w| arr.covered(w).next_back(),
                |i, c, group| {
                    if let Some(sp) = subpaths.get_mut(i) {
                        let p = contours_to_path(c, group.iter().copied(), &arr.tidy);
                        sp.extend(p.subpaths);
                    }
                },
            );
            subpaths.into_iter().zip(shapes).filter(|(sp, _)| !sp.is_empty()).flat_map(|(sp, s)| one(PathData::new(sp), s.key)).collect()
        }
        PathfinderOp::Merge => {
            // The distinct keys in first-seen order, and the index in `keys` of each shape's key.
            let mut keys: Vec<u64> = Vec::new();
            let mut index: HashMap<u64, usize> = HashMap::new();
            let class: Vec<usize> = shapes
                .iter()
                .map(|s| {
                    *index.entry(s.key).or_insert_with(|| {
                        keys.push(s.key);
                        keys.len() - 1
                    })
                })
                .collect();
            // topmost(m) is a shape with key k; a same-key island in a hole stays with the
            // contour around it, as `Contours::grouped` keeps it.
            let mut faces: Vec<Vec<PathData>> = vec![Vec::new(); keys.len()];
            each_classified_group(
                &arr,
                keys.len(),
                |w| arr.covered(w).next_back().and_then(|t| class.get(t).copied()),
                |c, owner| contour_trees(c, owner, false),
                |i, c, group| {
                    if let Some(f) = faces.get_mut(i) {
                        f.push(contours_to_path(c, group.iter().copied(), p));
                    }
                },
            );
            keys.into_iter().zip(faces).flat_map(|(k, fs)| fs.into_iter().flat_map(move |f| one(f, k))).collect()
        }
        PathfinderOp::Crop => {
            if n < 2 {
                return Vec::new();
            }
            let mut subpaths = vec![Vec::new(); n - 1];
            each_classified_face(
                &arr,
                n - 1,
                |w| {
                    let mut c = arr.covered(w).rev();
                    if c.next() == Some(n - 1) { c.next() } else { None }
                },
                |i, c, group| {
                    if let Some(sp) = subpaths.get_mut(i) {
                        let p = contours_to_path(c, group.iter().copied(), &arr.tidy);
                        sp.extend(p.subpaths);
                    }
                },
            );
            subpaths.into_iter().zip(shapes).filter(|(sp, _)| !sp.is_empty()).flat_map(|(sp, s)| one(PathData::new(sp), s.key)).collect()
        }
        PathfinderOp::Outline => outline_edges(&arr, shapes),
        PathfinderOp::HardMix | PathfinderOp::SoftMix => regions_of(&arr)
            .into_iter()
            .filter_map(|r| {
                let key = shapes.get(r.top()?)?.key;
                Some(Shape::new(r.path, FillRule::NonZero, key))
            })
            .collect(),
    }
}

/// The faces of `arr`, each distinct coverage mask's in turn (sorted), each face an outer contour
/// and the holes of its mask directly inside it ([`face_groups`]).
fn regions_of(arr: &Arrangement) -> Vec<Region> {
    regions_of_within(arr, usize::MAX).unwrap_or_default()
}

/// [`regions_of`] while there are at most `limit` faces: `None` as soon as one more is found,
/// without building the faces after it.
fn regions_of_within(arr: &Arrangement, limit: usize) -> Option<Vec<Region>> {
    let masks = arr.distinct_masks();
    let refs: Vec<&[usize]> = masks.iter().map(Vec::as_slice).collect();
    let mut faces: Vec<Vec<PathData>> = vec![Vec::new(); masks.len()];
    let mut found = 0usize;
    let walk = each_face_until(arr, &refs, |m, c, group| {
        #[cfg(test)]
        FACE_PATHS.with(|n| n.set(n.get() + 1));
        let path = contours_to_path(c, group.iter().copied(), &arr.tidy);
        if path.is_empty() {
            return ControlFlow::Continue(());
        }
        found += 1;
        if found > limit {
            return ControlFlow::Break(());
        }
        if let Some(f) = faces.get_mut(m) {
            f.push(path);
        }
        ControlFlow::Continue(())
    });
    if walk.is_break() {
        return None;
    }
    let mut out = Vec::new();
    for (sources, paths) in masks.iter().zip(faces) {
        out.extend(paths.into_iter().filter(|p| !p.is_empty()).map(|path| Region { path, sources: sources.clone() }));
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
fn each_classified_face(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    f: impl FnMut(usize, &Contours, &[ContourIdx]),
) {
    each_classified_group(arr, num_classes, classify, face_groups, f);
}

/// [`each_classified_face`] with each contour pass split into groups by `group` instead of
/// [`face_groups`]: `group` gets a pass and the class of each of its contours.
fn each_classified_group(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    group: impl Fn(&Contours, &[usize]) -> Vec<(usize, Vec<ContourIdx>)>,
    mut f: impl FnMut(usize, &Contours, &[ContourIdx]),
) {
    let _ = each_classified_group_until(arr, num_classes, classify, group, |m, c, group| {
        f(m, c, group);
        ControlFlow::Continue(())
    });
}

/// [`each_classified_face`] until `f` breaks: no face after that one is walked.
fn each_classified_face_until(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    f: impl FnMut(usize, &Contours, &[ContourIdx]) -> ControlFlow<()>,
) -> ControlFlow<()> {
    each_classified_group_until(arr, num_classes, classify, face_groups, f)
}

/// [`each_classified_group`] until `f` breaks: no group after that one is walked.
fn each_classified_group_until(
    arr: &Arrangement,
    num_classes: usize,
    classify: impl Fn(&Multi) -> Option<usize>,
    group: impl Fn(&Contours, &[usize]) -> Vec<(usize, Vec<ContourIdx>)>,
    mut f: impl FnMut(usize, &Contours, &[ContourIdx]) -> ControlFlow<()>,
) -> ControlFlow<()> {
    if num_classes == 0 {
        return ControlFlow::Continue(());
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
                for (_, g) in group(&c, &owner) {
                    if f(m, &c, &g).is_break() {
                        return ControlFlow::Break(());
                    }
                }
            }
            continue;
        };
        for (m, g) in group(&c, &owner) {
            if f(m, &c, &g).is_break() {
                return ControlFlow::Break(());
            }
        }
    }
    ControlFlow::Continue(())
}

/// Calls `f` with each face of `arr` whose coverage is one of `masks` (distinct, each the inputs
/// it covers, ascending as [`Arrangement::covered`] gives them): the index of its mask in
/// `masks`, the contour pass it is in, and its contours, an outer contour and the holes of its
/// mask directly inside it ([`face_groups`]). Each mask's faces come in the order of that mask's
/// own contour pass.
///
/// A contour pass walks the whole arrangement, so one pass per mask would cost masks × edges
/// (two blends crossing make thousands of each). Instead the masks go into batches whose faces
/// share no vertex (so no edge either): one pass per batch walks each face's contours exactly as
/// its own mask's pass would, and the mask a contour bounds is the one of its batch at its first
/// point.
fn each_face(arr: &Arrangement, masks: &[&[usize]], mut f: impl FnMut(usize, &Contours, &[ContourIdx])) {
    let _ = each_face_until(arr, masks, |m, c, group| {
        f(m, c, group);
        ControlFlow::Continue(())
    });
}

/// [`each_face`] until `f` breaks: no face after that one is walked.
fn each_face_until(arr: &Arrangement, masks: &[&[usize]], f: impl FnMut(usize, &Contours, &[ContourIdx]) -> ControlFlow<()>) -> ControlFlow<()> {
    if masks.is_empty() {
        return ControlFlow::Continue(());
    }
    // Each mask is found by the inputs it covers.
    let ids = CoverIds::new(masks.iter().map(|m| m.to_vec()).collect());
    each_classified_face_until(arr, masks.len(), |w| ids.of(arr, w), f)
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
    contour_trees(c, owner, true)
}

/// Groups a contour pass by the sweep's tree, within each mask (`owner` gives each contour's
/// mask): a contour goes under the nearest enclosing contour of its mask, and each contour with
/// none starts a group, with every contour under it in depth-first order. With `islands_apart`,
/// an outer contour also starts a group of its own ([`face_groups`]); without it, each group is
/// what [`Contours::grouped`] gives for a pass of that mask alone.
fn contour_trees(c: &Contours, owner: &[usize], islands_apart: bool) -> Vec<(usize, Vec<ContourIdx>)> {
    let mut children = vec![Vec::new(); owner.len()];
    let mut roots = Vec::new();
    for (i, &m) in owner.iter().enumerate() {
        let ct = &c[ContourIdx(i)];
        let mut up = if islands_apart && ct.outer { None } else { ct.parent };
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
    arrangement(shapes, precision).map(|a| regions_of(&a)).unwrap_or_default()
}

/// [`regions`] when there are at most `limit` of them; `None` when there are more, found as soon
/// as the face past `limit` is built (the faces after it are not built).
pub fn regions_within(shapes: &[Shape], limit: usize) -> Option<Vec<Region>> {
    match arrangement(shapes, DEFAULT_PRECISION) {
        Some(a) => regions_of_within(&a, limit),
        None => Some(Vec::new()),
    }
}

/// The face under `point`, if any (Shape Builder hover/click).
pub fn region_at(shapes: &[Shape], point: Point) -> Option<Region> {
    regions(shapes).into_iter().find(|r| r.contains(point))
}

/// Merge Shape Builder regions into one path (their union).
pub fn merge_regions(regions: &[&Region]) -> PathData {
    let v: Vec<(&PathData, FillRule)> = regions.iter().map(|r| (&r.path, FillRule::NonZero)).collect();
    unite_all(&v)
}

/// Merges faces of the planar arrangement of a set of shapes (as [`regions`] gives them) into one
/// path, from the arrangement itself, made once for every merge of the set: the faces' contours
/// share their edges, so the edges between merged faces vanish (when two faces' copies of an edge
/// differ in the first merge to unite contours, every point within 16 eps of a point kept before
/// it moves onto that point first, in that merge and every later one). Uniting the faces' tidied
/// outlines ([`merge_regions`]) could leave those edges in, as a hairline gap or a spur, where
/// neighbouring faces' outlines had been refit apart. Shapes with open paths (faces lines cut, as
/// [`shape_builder`](crate::shape_builder) gives them) merge in their planar map the same way
/// (#893), and so do faces with a gap closed paths enclose, which no shape covers (#1691). Either
/// falls back to [`merge_regions`] when it can't place a face.
pub struct FaceMerger {
    /// The arrangement (`None`: open paths, or the sweep failed) and the number of shapes.
    arr: Option<Arrangement>,
    shapes: usize,
    /// The shapes, and their planar map once a merge has needed it (open paths, or a gap).
    lines: Option<(Vec<Shape>, std::cell::OnceCell<Option<crate::planar::Map>>)>,
    /// Whether every merge moves the points of the faces' contours together first: the first
    /// merge that unites contours decides it (`unite_faces`).
    snap: std::cell::OnceCell<bool>,
}

impl FaceMerger {
    pub fn new(shapes: &[Shape]) -> Self {
        let has_open = crate::has_open_path(shapes);
        // The Shape Builder's faces come from the planar map only up to its cap.
        let segments: usize = shapes.iter().flat_map(|s| &s.path.subpaths).map(|sp| sp.segment_count()).sum();
        let lines = (segments <= crate::SHAPE_BUILDER_MAX_SEGMENTS).then(|| (shapes.to_vec(), std::cell::OnceCell::new()));
        Self {
            arr: if has_open { None } else { arrangement(shapes, DEFAULT_PRECISION) },
            shapes: shapes.len(),
            lines,
            snap: std::cell::OnceCell::new(),
        }
    }

    /// Merge faces found with Gap Detection using the same temporary bridges as Live Paint.
    /// Source indices stay those of the original inputs; bridges have no fill and are never art.
    pub fn with_gap(shapes: &[Shape], gap: f64) -> Self {
        Self::new(&crate::planar::gap_inputs(shapes, gap))
    }

    /// The union of `faces`.
    pub fn merge(&self, faces: &[&Region]) -> PathData {
        self.merge_with(faces, None)
    }

    /// What is left of input `shape` when `merged` merge on their own: the union of `faces`, as
    /// [`merge`](Self::merge) gives it, except that a sliver goes in only when `shape` covers it
    /// and the merge of `merged` does not take it in.
    pub fn merge_rest(&self, shape: usize, faces: &[&Region], merged: &[&Region]) -> PathData {
        self.merge_with(faces, Some((shape, merged)))
    }

    fn merge_with(&self, faces: &[&Region], rest: Option<(usize, &[&Region])>) -> PathData {
        self.exact(faces, rest).or_else(|| self.planar(faces, rest)).filter(|p| !p.is_empty()).unwrap_or_else(|| merge_regions(faces))
    }

    /// `faces` merged in the planar map of the shapes: each face found again by a point inside
    /// it, and the result checked to cover what they cover.
    fn planar(&self, faces: &[&Region], rest: Option<(usize, &[&Region])>) -> Option<PathData> {
        let (shapes, map) = self.lines.as_ref()?;
        let map = map.get_or_init(|| crate::planar::map(shapes, true)).as_ref()?;
        let inside: Vec<Point> = faces.iter().map(|r| crate::planar::interior_point(&r.path)).collect::<Option<_>>()?;
        let rest = rest.map(|(shape, merged)| crate::planar::Rest {
            shape,
            merged: merged.iter().filter_map(|r| crate::planar::interior_point(&r.path)).collect(),
            cover: cover(merged, shapes.len()),
        });
        let merged = map.merge(&inside, &cover(faces, shapes.len()), rest.as_ref())?;
        // A face not found as itself (two points in one face of the map) shows in the area.
        let want: f64 = faces.iter().map(|r| crate::area(&r.path, FillRule::NonZero)).sum();
        let outlines: f64 = faces.iter().map(|r| r.path.to_bezpath().perimeter(1e-6)).sum();
        ((crate::area(&merged, FillRule::NonZero) - want).abs() <= 1e-9 * want.max(1.0) + crate::DEFAULT_PRECISION * outlines).then_some(merged)
    }

    fn exact(&self, faces: &[&Region], rest: Option<(usize, &[&Region])>) -> Option<PathData> {
        // A gap no shape covers is no face of the arrangement: it merges in the planar map.
        if faces.iter().any(|r| r.sources.is_empty()) {
            return None;
        }
        let arr = self.arr.as_ref()?;
        let mut raw = self.contours_of(arr, faces)?;
        // The faces merged on their own and their corners.
        let rest = rest.map(|(shape, merged)| {
            let merged: Vec<&Region> = merged.iter().copied().filter(|r| !r.sources.is_empty()).collect();
            let corners = self
                .contours_of(arr, &merged)
                .map(|c| c.elements().iter().filter_map(|el| el.end_point()).map(vertex_key).collect())
                .unwrap_or_default();
            Rest { shape, merged, corners }
        });
        add_slivers(arr, faces, rest.as_ref(), &mut raw);
        unite_faces(&raw, &arr.tidy, &self.snap)
    }

    /// The contours of the faces of `arr` that are `faces`, each found by its coverage and a point
    /// inside it.
    fn contours_of(&self, arr: &Arrangement, faces: &[&Region]) -> Option<BezPath> {
        let mut picks: Vec<(Vec<usize>, Point)> = Vec::with_capacity(faces.len());
        for r in faces {
            if r.sources.iter().any(|&s| s >= self.shapes) {
                return None;
            }
            let mut mask = r.sources.clone();
            mask.sort_unstable();
            mask.dedup();
            picks.push((mask, crate::planar::interior_point(&r.path)?));
        }
        // `masks` holds the coverages in the order they were first picked, `points` the picked
        // points of each, and `found` the faces of each that contain one of its points.
        let mut masks: Vec<&[usize]> = vec![];
        let mut points: Vec<Vec<Point>> = vec![];
        let mut index: HashMap<&[usize], usize> = HashMap::new();
        for (mask, p) in &picks {
            let m = *index.entry(mask.as_slice()).or_insert_with(|| {
                masks.push(mask);
                points.push(Vec::new());
                masks.len() - 1
            });
            if let Some(at) = points.get_mut(m) {
                at.push(*p);
            }
        }
        let mut found: Vec<Vec<BezPath>> = vec![Vec::new(); masks.len()];
        each_face(arr, &masks, |m, c, group| {
            let mut face = BezPath::new();
            for &i in group {
                face.extend(c[i].path.iter());
            }
            if let (Some(at), Some(out)) = (points.get(m), found.get_mut(m))
                && at.iter().any(|p| face.winding(*p) != 0)
            {
                out.push(face);
            }
        });
        let mut raw = BezPath::new();
        for face in found.iter().flatten() {
            raw.extend(face.iter());
        }
        Some(raw)
    }
}

/// What is left of input `shape` when the faces `merged` merge on their own, and the corners of
/// those faces ([`add_slivers`]).
struct Rest<'a> {
    shape: usize,
    merged: Vec<&'a Region>,
    corners: HashSet<(u64, u64)>,
}

/// For each of `shapes` inputs, whether it covers one of `faces`.
pub(crate) fn cover(faces: &[&Region], shapes: usize) -> Vec<bool> {
    let mut cover = vec![false; shapes];
    for &i in faces.iter().flat_map(|r| &r.sources) {
        if let Some(c) = cover.get_mut(i) {
            *c = true;
        }
    }
    cover
}

/// Adds to `raw` the faces of `arr` that [`regions`] drops as slivers ([`is_sliver`]) when they lie
/// next to a face with the coverage of one of `faces` and only shapes that cover `faces` cover
/// them, such as the overlap of two shapes that overlap by less than the precision. Without them,
/// merging the faces on both sides of a sliver leaves a hairline gap between them. A sliver that
/// touches none of `faces` stays a contour of its own, which [`unite_faces`] drops.
///
/// With `rest`, the merge is what is left of an input when other faces merge on their own: a
/// sliver goes in only when that input covers it, and not when it has an edge along one of those
/// faces and only shapes that cover them cover it, as it goes to their merge.
///
/// [`is_sliver`]: crate::boolean::is_sliver
fn add_slivers(arr: &Arrangement, faces: &[&Region], rest: Option<&Rest>, raw: &mut BezPath) {
    let top = &arr.top;
    let shapes = arr.rules.len();
    let cover = cover(faces, shapes);
    let merged_cover = rest.map(|r| self::cover(&r.merged, shapes)).unwrap_or_default();
    let picked = CoverIds::new(faces.iter().map(|r| r.sources.clone()).collect());
    let beside = CoverIds::new(rest.iter().flat_map(|r| r.merged.iter().map(|f| f.sources.clone())).collect());
    let is_picked = |w: &Multi| picked.of(arr, w).is_some();
    let within = |w: &Multi, c: &[bool]| arr.covered(w).next().is_some() && arr.covered(w).all(|i| c.get(i) == Some(&true));
    let ours = |w: &Multi| rest.is_none_or(|r| arr.covered(w).any(|i| i == r.shape));
    // The coverage of each face next to a picked one, if only shapes of `faces` cover it, and the
    // ends of the edges along faces of `rest` that would go to their merge.
    let mut next_to: Vec<Vec<usize>> = Vec::new();
    let mut taken: HashSet<(u64, u64)> = HashSet::new();
    for s in top.segment_indices() {
        let h = s.first_half();
        let (a, b) = (top.winding_clockwise(h), top.winding_counter_clockwise(h));
        for (w, other) in [(a, b), (b, a)] {
            if within(w, &cover) && ours(w) && !is_picked(w) && is_picked(other) && !next_to.iter().any(|m| arr.covered(w).eq(m.iter().copied())) {
                next_to.push(arr.covered(w).collect());
            }
            if let Some(r) = rest
                && within(w, &merged_cover)
                && beside.of(arr, other).is_some()
            {
                let ends = [h, h.other_half()].map(|e| vertex_key(top.point(e).to_kurbo()));
                if ends.iter().all(|k| r.corners.contains(k)) {
                    taken.extend(ends);
                }
            }
        }
    }
    let along_merged = |path: &BezPath| path.elements().iter().any(|el| el.end_point().is_some_and(|p| taken.contains(&vertex_key(p))));
    let masks: Vec<&[usize]> = next_to.iter().map(Vec::as_slice).collect();
    let mut found: Vec<Vec<BezPath>> = vec![Vec::new(); masks.len()];
    each_face(arr, &masks, |m, c, group| {
        if contours_to_path(c, group.iter().copied(), &arr.tidy).is_empty()
            && !group.iter().any(|&i| along_merged(&c[i].path))
            && let Some(out) = found.get_mut(m)
        {
            let mut face = BezPath::new();
            for &i in group {
                face.extend(c[i].path.iter());
            }
            out.push(face);
        }
    });
    for face in found.iter().flatten() {
        raw.extend(face.iter());
    }
}

/// The union of the faces whose contours are `raw`, tidied with `tidy`. The points of the
/// contours move together first ([`snap_points`]) when `snap` holds `true`. When `snap` is empty,
/// it is set to whether two copies of an edge in `raw` differ.
fn unite_faces(raw: &BezPath, tidy: &Tidy, snap: &std::cell::OnceCell<bool>) -> Option<PathData> {
    // The planar map joins the ends of copies of an edge within 16 eps too.
    let eps = eps_for([raw]).ok()?;
    let snapped = if snap.get() == Some(&false) {
        None
    } else {
        let (snapped, differ) = snap_points(raw, 16.0 * eps);
        snap.get_or_init(|| differ).then_some(snapped)
    };
    let merged = normalize_bez(snapped.as_ref().unwrap_or(raw), FillRule::NonZero).ok()?;
    Some(all_contours_to_path(&merged, tidy))
}

/// `raw` with the points of its contours gathered onto points more than `tol` apart: each point
/// moves onto a point kept before it within `tol`, or is kept, and the handles next to it move
/// along. A piece left within `tol` of one point is dropped. It also returns whether two copies of
/// an edge differ, before the move or after it ([`copies_differ`]).
///
/// Two faces' contours can carry the edge they share as pieces whose ends lie a few eps apart, the
/// difference left as pieces shorter than eps: linesweeper 0.5 rebuilds a contour's run of pieces
/// of one input curve from that curve and does not move the rebuilt ends onto the sweep's points.
/// Uniting the faces as they are can leave the space between the two copies in, as a hairline that
/// reaches a hole through those short pieces (a spur) or as a seam that splits the outline (#1770).
/// Copies that share their ends leave at most a hairline contour of their own, which
/// [`is_sliver`](crate::boolean::is_sliver) drops.
fn snap_points(raw: &BezPath, tol: f64) -> (BezPath, bool) {
    let cell = |p: Point| ((p.x / tol).floor() as i64, (p.y / tol).floor() as i64);
    let mut grid: HashMap<(i64, i64), Vec<(usize, Point)>> = HashMap::new();
    let mut snapped: HashMap<(u64, u64), (usize, Point)> = HashMap::new();
    let mut kept: usize = 0;
    let mut snap = |p: Point| {
        *snapped.entry(vertex_key(p)).or_insert_with(|| {
            let (cx, cy) = cell(p);
            let near = (cx - 1..=cx + 1)
                .flat_map(|x| (cy - 1..=cy + 1).map(move |y| (x, y)))
                .filter_map(|k| grid.get(&k))
                .flatten()
                .find(|(_, q)| q.distance(p) <= tol)
                .copied();
            near.unwrap_or_else(|| {
                let v = (kept, p);
                kept += 1;
                grid.entry((cx, cy)).or_default().push(v);
                v
            })
        })
    };
    let mut out = BezPath::new();
    // Each contour's pieces, each with the segment it was and the indices of the two points it runs
    // between.
    let mut contours: Vec<Vec<(PathSeg, PathSeg, [usize; 2])>> = Vec::new();
    for (range, closed) in craft_geom::bez::subpaths(raw) {
        let Some(els) = raw.elements().get(range) else { continue };
        let mut pieces = Vec::new();
        for s in craft_geom::bez::segments(els) {
            let ((i, a), (j, b)) = (snap(s.start()), snap(s.end()));
            let mut seg = to_seg(s);
            crate::planar::pin_ends(std::slice::from_mut(&mut seg), a, b);
            if a == b && seg.c.p1.distance(a) <= tol && seg.c.p2.distance(a) <= tol {
                continue;
            }
            if pieces.is_empty() {
                out.move_to(a);
            }
            let piece = if (a, b) == (s.start(), s.end()) {
                s
            } else if seg.line {
                PathSeg::Line(Line::new(a, b))
            } else {
                PathSeg::Cubic(seg.c)
            };
            out.push(piece.as_path_el());
            pieces.push((piece, s, [i, j]));
        }
        if !pieces.is_empty() && closed {
            out.close_path();
        }
        contours.push(pieces);
    }
    let differ = copies_differ(&contours, kept, tol);
    (out, differ)
}

/// Whether two copies of an edge in `contours` differ. Each piece comes with the segment it was
/// before the move and the indices of the two points it runs between, out of `points`. A copy is a
/// chain of pieces between junctions, the points where other than two piece ends meet. Two chains
/// that run opposite ways between the same two junctions, with the middle and the end of each
/// piece within `tol` of the other chain, are copies of one edge, and they differ when one is not
/// the exact reverse of the other, after the move or before it.
fn copies_differ(contours: &[Vec<(PathSeg, PathSeg, [usize; 2])>], points: usize, tol: f64) -> bool {
    let mut degree = vec![0usize; points];
    for &v in contours.iter().flatten().flat_map(|(_, _, ends)| ends) {
        if let Some(d) = degree.get_mut(v) {
            *d += 1;
        }
    }
    let junction = |v: usize| degree.get(v) != Some(&2);
    // (pieces, the segments they were, first point, last point). A contour through no junction
    // shares no point with another contour.
    let mut chains: Vec<(Vec<PathSeg>, Vec<PathSeg>, usize, usize)> = Vec::new();
    for pieces in contours {
        let Some(first) = pieces.iter().position(|(_, _, [a, _])| junction(*a)) else { continue };
        let (mut cur, mut was): (Vec<PathSeg>, Vec<PathSeg>) = (Vec::new(), Vec::new());
        let (mut from, mut to) = (0, 0);
        for &(s, w, [a, b]) in pieces.iter().cycle().skip(first).take(pieces.len()) {
            if junction(a) && !cur.is_empty() {
                chains.push((std::mem::take(&mut cur), std::mem::take(&mut was), from, to));
            }
            if cur.is_empty() {
                from = a;
            }
            cur.push(s);
            was.push(w);
            to = b;
        }
        chains.push((cur, was, from, to));
    }
    let key = |a: usize, b: usize| (a.min(b), a.max(b));
    let mut by_ends: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (k, (_, _, a, b)) in chains.iter().enumerate() {
        by_ends.entry(key(*a, *b)).or_default().push(k);
    }
    let reversed = |a: &[PathSeg], b: &[PathSeg]| a.iter().rev().map(|s| s.reverse()).eq(b.iter().copied());
    let mut paired = vec![false; chains.len()];
    for (k, (segs, was, a, b)) in chains.iter().enumerate() {
        if paired.get(k) != Some(&false) {
            continue;
        }
        let Some(candidates) = by_ends.get(&key(*a, *b)) else { continue };
        let found = candidates.iter().filter(|&&j| j > k && paired.get(j) == Some(&false)).find_map(|&j| {
            let (other, other_was, c, d) = chains.get(j)?;
            (c == b && d == a && along(segs, other, tol)).then_some((j, other, other_was))
        });
        let Some((j, other, other_was)) = found else { continue };
        for i in [k, j] {
            if let Some(p) = paired.get_mut(i) {
                *p = true;
            }
        }
        if !reversed(segs, other) || !reversed(was, other_was) {
            return true;
        }
    }
    false
}

/// Whether the middle and the end of every piece of `a` lie within `tol` of `b`, and the middle
/// and the end of every piece of `b` lie within `tol` of `a`. `a` and `b` run opposite ways.
fn along(a: &[PathSeg], b: &[PathSeg], tol: f64) -> bool {
    let covers = |from: &[PathSeg], to: &[PathSeg]| {
        // Each point's search goes back along `to` from the piece the point before it was near.
        let mut at = to.len();
        from.iter().flat_map(|s| [s.eval(0.5), s.end()]).all(|p| {
            let mut order = to.iter().enumerate().take(at).rev().chain(to.iter().enumerate().skip(at).rev());
            let Some((i, _)) = order.find(|(_, t)| t.nearest(p, 1e-12).distance_sq.sqrt() <= tol) else { return false };
            at = i + 1;
            true
        })
    };
    covers(a, b) && covers(b, a)
}

/// Pathfinder Outline: split every shape's boundary at junctions of the arrangement.
fn outline_edges(arr: &Arrangement, shapes: &[Shape]) -> Vec<Shape> {
    // Sorted by x (then y), so a segment needs only the junctions inside its control box.
    let junctions = arr.junctions();
    let scale = shapes.iter().filter_map(|s| s.path.control_bounds()).map(|r| r.width().max(r.height())).fold(1.0, f64::max);
    let tol = scale * 1e-7;
    let reach = tol.max(1e-6);
    // (chain, key, source index)
    let mut chains: Vec<(Vec<Seg>, u64)> = Vec::new();
    for s in shapes {
        for sp in &s.path.subpaths {
            if sp.anchors.len() < 2 {
                continue;
            }
            let mut sp = sp.clone();
            sp.closed = true;
            // Split each segment at junctions; record which piece starts are breaks.
            let mut pieces: Vec<(Seg, bool)> = Vec::new();
            for i in 0..sp.segment_count() {
                let c = sp.segment(i);
                let line = sp.segment_is_line(i);
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
                // end of the segment is that end. Else a sliver piece is left between them (#3870).
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
            let sp = segs_to_subpath(&chain, false)?;
            Some(Shape::new(PathData::single(sp), FillRule::NonZero, key))
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
    use craft_geom::{Rect, SubPath, shapes};
    use proptest::prelude::*;

    /// The faces one contour pass per mask finds: what [`regions_of`] must match exactly when no
    /// face lies inside a hole of its own mask (`grouped` puts such an island in the frame's face).
    fn regions_one_mask_at_a_time(arr: &Arrangement) -> Vec<Region> {
        let mut out = Vec::new();
        for mask in flag_masks(arr) {
            let c = arr.contours(|m| m == mask.as_slice());
            let sources: Vec<usize> = mask.iter().enumerate().filter(|(_, b)| **b).map(|(i, _)| i).collect();
            for g in c.grouped() {
                let path = contours_to_path(&c, g, &arr.tidy);
                if !path.is_empty() {
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
        assert_eq!(arr.distinct_masks(), listed);
        assert_eq!(regions_of(&arr), regions_one_mask_at_a_time(&arr));
    }

    fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> PathData {
        shapes::rectangle(Rect::new(x0, y0, x1, y1))
    }

    /// Pathfinder Options › Precision: pieces thinner than the precision are dropped, so a 0.5 pt
    /// strip that the default keeps goes at a precision of 1 pt, whether the Pathfinder works on
    /// the arrangement (Divide), the planar map (Divide with an open path) or the union (Unite).
    #[test]
    fn precision_drops_pieces_thinner_than_it() {
        let overlapping =
            [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(rect(0.0, 99.5, 100.0, 200.0), FillRule::NonZero, 2)];
        let line = PathData::single(SubPath::polyline(&[Point::new(-10.0, 99.5), Point::new(110.0, 99.5)], false));
        let cut = [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(line, FillRule::NonZero, 2)];
        let apart =
            [Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 1), Shape::new(rect(0.0, 150.0, 100.0, 150.5), FillRule::NonZero, 2)];
        for (what, op, shapes, kept) in [
            ("Divide", PathfinderOp::Divide, &overlapping, 3),
            ("Divide with an open path", PathfinderOp::Divide, &cut, 2),
            ("Unite", PathfinderOp::Unite, &apart, 2),
        ] {
            let count = |r: Vec<Shape>| r.iter().map(|s| s.path.subpaths.len()).sum::<usize>();
            assert_eq!(pathfinder_with(op, shapes, DEFAULT_PRECISION), pathfinder(op, shapes), "{what}: the default is the plain Pathfinder");
            assert_eq!(count(pathfinder(op, shapes)), kept, "{what}: the default keeps the strip");
            assert_eq!(count(pathfinder_with(op, shapes, 1.0)), kept - 1, "{what}: 1 pt drops it");
            assert_eq!(pathfinder_with(op, shapes, f64::NAN), pathfinder(op, shapes), "{what}: an invalid precision is the default");
        }
    }

    /// The planar map that Divide with an open path uses merges two edges between the same points
    /// only when they are closer than the precision: a lens 0.005 pt thick between two curves is a
    /// piece at a precision of 0.001 pt, and not at the default.
    #[test]
    fn open_path_divide_keeps_a_lens_thicker_than_the_precision() {
        // Curves monotonic in x and y, so the sweep keeps each one whole between the shared ends;
        // their middles are 0.005 pt apart.
        let shift = 0.005 / 0.75 / std::f64::consts::SQRT_2;
        let curve = |d: f64| [Point::new(0.0, 0.0), Point::new(d, 50.0 - d), Point::new(50.0 + d, 100.0 - d), Point::new(100.0, 100.0)];
        let (top, bottom) = (curve(0.0), curve(shift));
        let mut bp = kurbo::BezPath::new();
        bp.move_to(top[0]);
        bp.curve_to(top[1], top[2], top[3]);
        bp.curve_to(bottom[2], bottom[1], bottom[0]);
        bp.close_path();
        let lens = Shape::new(PathData::from_bezpath(&bp), FillRule::NonZero, 1);
        let line = PathData::single(SubPath::polyline(&[Point::new(200.0, 0.0), Point::new(300.0, 0.0)], false));
        let shapes = [lens, Shape::new(line, FillRule::NonZero, 2)];
        let area = |r: &[Shape]| r.iter().map(|s| crate::area(&s.path, FillRule::NonZero)).sum::<f64>();
        let fine = pathfinder_with(PathfinderOp::Divide, &shapes, 0.001);
        assert!(area(&fine) > 0.1, "the lens is a piece: {} pt²", area(&fine));
        assert_eq!(area(&pathfinder(PathfinderOp::Divide, &shapes)), 0.0, "thinner than the default precision");
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
                p.subpaths.extend(rect(x0, y0, cx.max(x0 + 5.0), cy.max(y0 + 5.0)).subpaths);
                p
            }),
            r.prop_map(|(x0, y0, x1, y1)| shapes::ellipse(Rect::new(x0, y0, x1, y1))),
            prop::collection::vec((c(), c()), 3..7).prop_map(|pts| {
                let pts: Vec<Point> = pts.into_iter().map(|(x, y)| Point::new(x, y)).collect();
                PathData::single(SubPath::polyline(&pts, true))
            }),
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
        frame.subpaths.extend(rect(60.0, 110.0, 65.0, 115.0).subpaths);
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

    /// The coverages come in the order of their flag masks, with the same regions, for shapes that
    /// overlap, nest, touch or lie apart, and for more than 64 shapes stacked out of their order
    /// across the page.
    #[test]
    fn coverages_keep_the_order_of_their_flag_masks() {
        let r = |x0: f64, y0: f64, x1: f64, y1: f64| Shape::new(rect(x0, y0, x1, y1), FillRule::NonZero, 1);
        let overlapping = vec![r(0.0, 0.0, 100.0, 100.0), r(50.0, 50.0, 150.0, 150.0), r(25.0, 75.0, 125.0, 175.0)];
        let nested = vec![r(0.0, 0.0, 300.0, 300.0), r(50.0, 50.0, 250.0, 250.0), r(100.0, 100.0, 200.0, 200.0), r(140.0, 140.0, 160.0, 160.0)];
        let touching = vec![r(0.0, 0.0, 100.0, 100.0), r(100.0, 0.0, 200.0, 100.0), r(0.0, 100.0, 100.0, 200.0), r(100.0, 100.0, 200.0, 200.0)];
        let apart = vec![r(0.0, 0.0, 10.0, 10.0), r(50.0, 0.0, 60.0, 10.0), r(0.0, 50.0, 10.0, 60.0)];
        // 70 strips, each overlapping its neighbors, stacked in an order other than across the
        // page, and two bars across them.
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

    proptest! {
        #![proptest_config(ProptestConfig { cases: 256, failure_persistence: None, rng_seed: proptest::test_runner::RngSeed::Fixed(0x5eed_9a7f), ..ProptestConfig::default() })]

        /// Batching the masks changes nothing: the same regions, in the same order, with the same
        /// contours in each.
        #[test]
        fn batched_regions_match_one_mask_at_a_time(v in prop::collection::vec(arb_shape(), 1..8)) {
            same_as_one_mask_at_a_time(&v);
        }
    }

    /// The edge that the two faces of the #1770 tests share.
    fn shared_edge() -> kurbo::CubicBez {
        kurbo::CubicBez::new((100.0, 0.0), (130.0, 30.0), (130.0, 70.0), (100.0, 100.0))
    }

    /// Two faces on either side of [`shared_edge`]. The right face's copy of the edge has its ends
    /// moved along the bottom and top edges by `bottom` and `top`, its handles moved with them, and
    /// its first handle moved by `nudge` more.
    fn faces_apart(bottom: f64, top: f64, nudge: f64) -> BezPath {
        use kurbo::{CubicBez, Vec2};
        let edge = shared_edge();
        let (p0, p3) = (edge.p0 + Vec2::new(bottom, 0.0), edge.p3 + Vec2::new(top, 0.0));
        let copy = CubicBez::new(p0, edge.p1 + (p0 - edge.p0) + Vec2::new(0.0, nudge), edge.p2 + (p3 - edge.p3), p3);
        let mut raw = BezPath::new();
        raw.move_to((0.0, 0.0));
        raw.line_to(edge.p0);
        raw.curve_to(edge.p1, edge.p2, edge.p3);
        raw.line_to((0.0, 100.0));
        raw.close_path();
        raw.move_to(copy.p0);
        raw.line_to((200.0, 0.0));
        raw.line_to((200.0, 100.0));
        raw.line_to(copy.p3);
        raw.curve_to(copy.p2, copy.p1, copy.p0);
        raw.close_path();
        raw
    }

    /// Two faces whose copies of [`shared_edge`] are exact reverses of each other. The left face
    /// has a point 5e-6 pt from another point.
    fn faces_matching() -> BezPath {
        let edge = shared_edge();
        let mut raw = BezPath::new();
        raw.move_to((0.0, 0.0));
        raw.line_to(edge.p0);
        raw.curve_to(edge.p1, edge.p2, edge.p3);
        raw.line_to((0.0, 100.0));
        raw.curve_to((-20.0, 90.0), (-30.0, 70.0), (-30.0, 50.0));
        raw.line_to((-30.0, 50.0 - 5e-6));
        raw.curve_to((-20.0, 40.0), (-20.0, 10.0), (0.0, 0.0));
        raw.close_path();
        raw.move_to(edge.p0);
        raw.line_to((200.0, 0.0));
        raw.line_to((200.0, 100.0));
        raw.line_to(edge.p3);
        raw.curve_to(edge.p2, edge.p1, edge.p0);
        raw.close_path();
        raw
    }

    /// #1770: two faces' contours carry the edge they share as copies whose ends lie a few eps
    /// apart, at one end or at both, with or without a handle a rounding error apart. The faces
    /// unite into one outline, with no hairline left between the copies.
    #[test]
    fn faces_unite_into_one_outline_when_their_copies_of_an_edge_end_a_few_eps_apart() {
        let tidy = Tidy::free(crate::DEFAULT_PRECISION);
        for (bottom, top, nudge) in [(0.0, 3e-6, 4e-7), (3e-6, 3e-6, 4e-7), (0.0, 3e-6, 0.0), (3e-6, 3e-6, 0.0)] {
            let merged = unite_faces(&faces_apart(bottom, top, nudge), &tidy, &std::cell::OnceCell::new()).unwrap();
            assert_eq!(merged.subpaths.len(), 1, "ends apart by {bottom} and {top}, nudge {nudge}: {merged:?}");
            assert!((merged.length() - 600.0).abs() < 1e-3, "ends apart by {bottom} and {top}, nudge {nudge}: {} pt of outline", merged.length());
        }
    }

    /// #1770: when the faces' copies of every edge they share are exact reverses of each other,
    /// the faces unite as they are, and a point 5e-6 pt from another point stays where it is.
    #[test]
    fn faces_whose_copies_of_each_shared_edge_match_unite_as_they_are() {
        let tidy = Tidy::free(crate::DEFAULT_PRECISION);
        let raw = faces_matching();
        let as_they_are = all_contours_to_path(&normalize_bez(&raw, FillRule::NonZero).unwrap(), &tidy);
        assert_eq!(as_they_are.subpaths.len(), 1, "{as_they_are:?}");
        assert_eq!(unite_faces(&raw, &tidy, &std::cell::OnceCell::new()), Some(as_they_are));
    }

    /// #1770: the first union decides for every later union of the same set whether the points
    /// move together. After faces whose copies of an edge end apart, the faces whose copies match
    /// lose their point 5e-6 pt from another point.
    #[test]
    fn later_unions_move_points_as_the_first_union_decided() {
        let tidy = Tidy::free(crate::DEFAULT_PRECISION);
        let snap = std::cell::OnceCell::new();
        let first = unite_faces(&faces_apart(0.0, 3e-6, 0.0), &tidy, &snap).unwrap();
        assert_eq!(first.subpaths.len(), 1, "{first:?}");
        let later = unite_faces(&faces_matching(), &tidy, &snap).unwrap();
        let moved = Point::new(-30.0, 50.0 - 5e-6);
        assert!(later.subpaths.iter().flat_map(|sp| &sp.anchors).all(|a| a.p != moved), "{later:?}");
    }

    /// A merge walks the arrangement once for each batch of the faces' coverages. 49 rectangles in
    /// a row, each overlapping the next, make 97 faces, each with a coverage of its own.
    #[test]
    fn merging_many_faces_walks_the_arrangement_a_few_times() {
        let v: Vec<Shape> = (0..49)
            .map(|k| {
                let x = 10.0 * f64::from(k);
                Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, 1)
            })
            .collect();
        let faces = regions(&v);
        assert_eq!(faces.len(), 97);
        // The merge takes every face but the one only rectangle 20 covers. That face lies next to
        // merged faces, so the merge also looks for slivers there.
        let left = faces.iter().position(|r| r.sources == [20]).unwrap();
        let picked: Vec<&Region> = faces.iter().enumerate().filter(|(k, _)| *k != left).map(|(_, r)| r).collect();
        let merger = FaceMerger::new(&v);
        CONTOUR_PASSES.with(|n| n.set(0));
        let merged = merger.merge(&picked);
        let passes = CONTOUR_PASSES.with(std::cell::Cell::get);
        assert!(passes <= 8, "{passes} contour passes");
        // The result is the row, 495 pt long, less the 5 pt that only rectangle 20 covers.
        assert_eq!(merged.subpaths.len(), 2, "{merged:?}");
        let area = crate::area(&merged, FillRule::NonZero);
        assert!((area - 490.0 * 40.0).abs() < 1e-6, "{area}");
    }

    /// The row of 49 rectangles makes 97 faces: within a limit of 97 or more they are [`regions`]'s
    /// faces, and over a lower one the build stops at the face past it.
    #[test]
    fn regions_within_a_limit_stop_at_the_face_past_it() {
        let v: Vec<Shape> = (0..49)
            .map(|k| {
                let x = 10.0 * f64::from(k);
                Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, 1)
            })
            .collect();
        let all = regions(&v);
        assert_eq!(all.len(), 97);
        for limit in [97, 98, usize::MAX] {
            assert_eq!(regions_within(&v, limit).as_ref(), Some(&all), "limit {limit}");
        }
        for limit in [0, 1, 10, 96] {
            FACE_PATHS.with(|n| n.set(0));
            assert_eq!(regions_within(&v, limit), None, "limit {limit}");
            assert_eq!(FACE_PATHS.with(std::cell::Cell::get), limit + 1, "limit {limit}: faces built");
        }
        assert_eq!(regions_within(&[], 0), Some(vec![]));
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

    /// A frame and the island in its hole, both of one even-odd shape, are two faces with the
    /// same coverage: merging the frame leaves the island out.
    #[test]
    fn merging_a_frame_leaves_out_the_island_in_its_hole() {
        let mut p = rect(0.0, 0.0, 100.0, 100.0);
        p.subpaths.extend(rect(20.0, 20.0, 80.0, 80.0).subpaths);
        p.subpaths.extend(rect(40.0, 40.0, 60.0, 60.0).subpaths);
        let v = [Shape::new(p, FillRule::EvenOdd, 1)];
        let faces = regions(&v);
        assert_eq!(faces.len(), 2);
        let frame = faces.iter().find(|r| r.contains(Point::new(10.0, 10.0))).unwrap();
        let merged = FaceMerger::new(&v).merge(&[frame]);
        let area = crate::area(&merged, FillRule::NonZero);
        assert!((area - (100.0 * 100.0 - 60.0 * 60.0)).abs() < 1e-6, "{area}");
    }

    #[test]
    fn divide_open_paths_cut_faces_without_taking_the_cutters_paint() {
        let line = |a: (f64, f64), b: (f64, f64)| shapes::line(Point::from(a), Point::from(b));
        let mut cases = vec![
            ("horizontal", vec![line((-10.0, 50.0), (110.0, 50.0))], 2),
            ("reverse", vec![line((110.0, 50.0), (-10.0, 50.0))], 2),
            ("vertical", vec![line((50.0, -10.0), (50.0, 110.0))], 2),
            ("diagonal", vec![line((-10.0, -10.0), (110.0, 110.0))], 2),
            ("crossed", vec![line((-10.0, 50.0), (110.0, 50.0)), line((50.0, -10.0), (50.0, 110.0))], 4),
            ("outside", vec![line((-10.0, 150.0), (110.0, 150.0))], 1),
            ("dangling", vec![line((-10.0, 50.0), (50.0, 50.0))], 1),
            ("boundary", vec![line((-10.0, 0.0), (110.0, 0.0))], 1),
            ("tangent", vec![line((-10.0, 10.0), (10.0, -10.0))], 1),
        ];
        let mut curve = BezPath::new();
        curve.move_to((-10.0, 50.0));
        curve.curve_to((30.0, 15.0), (70.0, 85.0), (110.0, 50.0));
        cases.push(("curved", vec![PathData::from_bezpath(&curve)], 2));
        let mut bent = BezPath::new();
        bent.move_to((-10.0, 25.0));
        bent.line_to((50.0, 75.0));
        bent.line_to((110.0, 25.0));
        cases.push(("bent", vec![PathData::from_bezpath(&bent)], 2));
        for (label, cutters, count) in cases {
            let mut inputs = vec![Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 7)];
            inputs.extend(cutters.into_iter().map(|p| Shape::new(p, FillRule::NonZero, 99)));
            let divided = pathfinder(PathfinderOp::Divide, &inputs);
            assert_eq!(divided.len(), count, "{label}");
            assert!(divided.iter().all(|s| s.key == 7 && s.path.subpaths.iter().all(|sp| sp.closed)), "{label}");
            let total: f64 = divided.iter().map(|s| crate::area(&s.path, s.rule)).sum();
            assert!((total - 10000.0).abs() < 0.01, "{label}: area {total}");
            for p in [Point::new(20.0, 31.0), Point::new(79.0, 37.0), Point::new(81.0, 68.0), Point::new(17.0, 85.0)] {
                assert_eq!(divided.iter().filter(|s| s.path.to_bezpath().winding(p) != 0).count(), 1, "{label}: {p:?}");
            }
        }
    }

    #[test]
    fn divide_open_paths_preserve_frontmost_fills_and_holes() {
        let cutter = || Shape::new(shapes::line(Point::new(-10.0, 50.0), Point::new(160.0, 50.0)), FillRule::NonZero, 99);
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
        ring.subpaths.extend(rect(30.0, 30.0, 70.0, 70.0).subpaths);
        let faces = pathfinder(PathfinderOp::Divide, &[Shape::new(ring, FillRule::EvenOdd, 7), cutter()]);
        assert_eq!(faces.len(), 2);
        assert!(faces.iter().all(|s| s.key == 7 && s.path.to_bezpath().winding(Point::new(50.0, 40.0)) == 0));
        let total: f64 = faces.iter().map(|s| crate::area(&s.path, s.rule)).sum();
        assert!((total - 8400.0).abs() < 1e-6);
    }

    #[test]
    fn divide_open_paths_retains_the_existing_segment_cap() {
        let mut inputs = vec![Shape::new(rect(0.0, 0.0, 100.0, 100.0), FillRule::NonZero, 7)];
        // The square's four sides and one line past the cap.
        let n = crate::SHAPE_BUILDER_MAX_SEGMENTS - 3;
        inputs.extend((0..n).map(|_| Shape::new(shapes::line(Point::new(-10.0, 50.0), Point::new(110.0, 50.0)), FillRule::NonZero, 99)));
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

    /// Merge with one contour pass per distinct key, each face of a key its own result.
    fn old_merge(shapes: &[Shape], precision: f64) -> Vec<Shape> {
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

    /// Overlapping squares on a grid with repeated fills, touching squares with two fills, and a
    /// fill's island inside a hole of the same fill, with another fill around it: Merge gives the
    /// same results, in the same order, as a contour pass per fill.
    #[test]
    fn merge_matches_a_contour_pass_per_fill() {
        let mut grid = Vec::new();
        for row in 0..5u32 {
            for col in 0..5u32 {
                let (x, y) = (10.0 * f64::from(col), 10.0 * f64::from(row));
                grid.push(Shape::new(rect(x, y, x + 15.0, y + 15.0), FillRule::NonZero, u64::from((row * 5 + col) * 7 % 4)));
            }
        }
        let mut ring = rect(0.0, 0.0, 100.0, 100.0);
        ring.subpaths.extend(rect(30.0, 30.0, 70.0, 70.0).subpaths);
        let nested = vec![
            Shape::new(ring, FillRule::EvenOdd, 1),
            Shape::new(rect(40.0, 40.0, 60.0, 60.0), FillRule::NonZero, 2),
            Shape::new(rect(45.0, 45.0, 55.0, 55.0), FillRule::NonZero, 1),
        ];
        // A fill's island sits inside a ring of another fill that shares no point with it, both
        // inside a hole of the first fill.
        let mut frame = rect(0.0, 0.0, 100.0, 100.0);
        frame.subpaths.extend(rect(30.0, 30.0, 70.0, 70.0).subpaths);
        let mut between = rect(35.0, 35.0, 65.0, 65.0);
        between.subpaths.extend(rect(42.0, 42.0, 58.0, 58.0).subpaths);
        let apart = vec![
            Shape::new(frame, FillRule::EvenOdd, 1),
            Shape::new(between, FillRule::EvenOdd, 3),
            Shape::new(rect(45.0, 45.0, 55.0, 55.0), FillRule::NonZero, 1),
        ];
        let touching: Vec<Shape> = (0..16u32)
            .map(|k| {
                let (x, y) = (10.0 * f64::from(k % 4), 10.0 * f64::from(k / 4));
                Shape::new(rect(x, y, x + 10.0, y + 10.0), FillRule::NonZero, u64::from((k % 4 + k / 4) % 2))
            })
            .collect();
        for (label, shapes) in [("grid", grid), ("nested", nested), ("apart", apart), ("touching", touching)] {
            let actual = pathfinder_with(PathfinderOp::Merge, &shapes, DEFAULT_PRECISION);
            let expected = old_merge(&shapes, DEFAULT_PRECISION);
            assert!(!expected.is_empty(), "{label}");
            assert_eq!(actual, expected, "{label}");
        }
    }

    /// 40 squares in a row, each overlapping the next and each with a fill of its own: Merge walks
    /// the arrangement a few times, not once per fill.
    #[test]
    fn merge_with_many_fills_walks_the_arrangement_a_few_times() {
        let v: Vec<Shape> = (0..40u32)
            .map(|k| {
                let x = 10.0 * f64::from(k);
                Shape::new(rect(x, 0.0, x + 15.0, 40.0), FillRule::NonZero, u64::from(k))
            })
            .collect();
        CONTOUR_PASSES.with(|n| n.set(0));
        let merged = pathfinder_with(PathfinderOp::Merge, &v, DEFAULT_PRECISION);
        let passes = CONTOUR_PASSES.with(std::cell::Cell::get);
        assert!(passes <= 8, "{passes} contour passes");
        assert_eq!(merged.len(), 40);
        assert!(merged.iter().enumerate().all(|(k, s)| s.key == k as u64));
    }
}
