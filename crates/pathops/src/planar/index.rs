// Carved from VectorCraft's crates/pathops (src/planar/index.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::*;
use crate::work::{self, Work};

/// The shears the sweep's frame may use, `y + k x`, in the order tried ([`try_frame_for`]): by default
/// 1/32, in which only an edge sloping -1/32 in the document is horizontal. Each is a power of two,
/// so it maps typical coordinates exactly.
pub(super) const SHEARS: [f64; 10] =
    [1.0 / 32.0, 1.0 / 64.0, 1.0 / 16.0, 1.0 / 128.0, 1.0 / 8.0, 1.0 / 256.0, 1.0 / 512.0, 1.0 / 4.0, 1.0 / 1024.0, 1.0 / 2.0];

/// The shear by `k` and its inverse.
pub(super) fn shear(k: f64) -> (Affine, Affine) {
    (Affine::new([1.0, k, 0.0, 1.0, 0.0, 0.0]), Affine::new([1.0, -k, 0.0, 1.0, 0.0, 0.0]))
}

/// The paths of a map as the sweep is given them: sheared, and snapped to the sweep's tolerance.
pub(super) struct Frame {
    /// The paths in the sweep's frame, each with the index of the shape it comes from.
    pub(super) inputs: Vec<(BezPath, usize)>,
    /// The sweep's tolerance for them.
    pub(super) eps: f64,
    /// From the sweep's frame back to the document.
    pub(super) back: Affine,
}

#[cfg(test)]
impl Frame {
    /// The shear, `y + k x`.
    pub(super) fn k(&self) -> f64 {
        -self.back.as_coeffs()[1]
    }
}

/// The frame for `shapes`, in which the sweep is given no open subpath that is level, which it takes
/// the two passes of for cancelling out ([`level_sides`]); and why there is none: `Ok(None)` when the
/// shapes draw nothing, [`crate::PathOpsError::TooComplex`] past the element limit,
/// [`crate::PathOpsError::NonFinite`] for a point that is not finite, and
/// [`crate::PathOpsError::Degenerate`] when no frame is found in which every side longer than
/// [`NOISE_SIDE`] tolerances is not level.
///
/// The first of [`SHEARS`] in whose frame, after the paths are snapped to the sweep's tolerance, no
/// side of an open subpath is level is used, at the first try for ordinary art. A shear that leaves
/// a long side level is no frame. One that leaves a short side level (from [`MIN_SIDE`] to
/// [`NOISE_SIDE`] tolerances, which some shears level and others need not) is kept in case no shear
/// leaves none, and then the one that leaves the fewest is used: a short edge is lost from the map only
/// where every frame loses it. When each of the ten leaves a long side level (ten lines, one level in
/// each), one more is tried, a shear between the slopes of the long sides in which none of them is
/// level ([`free_shear`]), from 1/1024 to 1/2; and when those slopes bar every shear there (eleven
/// short lines), one from -1/2 to -1/1024, where the frame levels the slopes the ten do not: twelve
/// tries at most.
///
/// The test is made on the snapped paths because snapping is what levels an edge: the sweep orders by
/// its tolerance, which a shape far away widens (a shape at 1e11 makes it 1.4e-3), and an edge that
/// rises less than that in the frame, or whose ends land on one of the heights snapping rounds to,
/// is level whatever its slope was. A shear in which the tolerance passes its limit (the art is near
/// the largest coordinates the sweep takes) is passed over for the next.
pub(super) fn try_frame_for(shapes: &[Shape]) -> Result<Option<Frame>, crate::PathOpsError> {
    use crate::PathOpsError;
    // The paths as they are swept, in the document's frame, and the open subpaths among them.
    let mut plain: Vec<(BezPath, usize)> = Vec::new();
    let mut opens = BezPath::new();
    for (i, s) in shapes.iter().enumerate() {
        let mut bp = BezPath::new();
        for sp in crate::path::subs(&s.path).iter().filter(|sp| sp.draws_something()) {
            if sp.closed {
                sp.push_closed(&mut bp);
            } else {
                let mut open = BezPath::new();
                sp.push_open(&mut open);
                super::graph::there_and_back(&open, &mut bp);
                opens.extend(open.iter());
            }
        }
        if !bp.elements().is_empty() {
            plain.push((bp, i));
        }
    }
    if plain.is_empty() {
        return Ok(None);
    }
    if !crate::budget::fits_elements(plain.iter().map(|p| &p.0)) {
        return Err(PathOpsError::TooComplex);
    }
    // The frame that leaves the fewest short sides level (and no long one), and the widest tolerance seen.
    let mut best: Option<(usize, Frame)> = None;
    let mut widest: f64 = 0.0;
    for k in SHEARS {
        let (frame, open) = match sheared(&plain, &opens, k) {
            Sheared::Framed(frame, open) => (frame, open),
            Sheared::TooWide => continue,
            Sheared::NotFinite => return Err(PathOpsError::NonFinite),
        };
        widest = widest.max(frame.eps);
        let level = level_sides(&open, frame.eps);
        if level.long {
            continue;
        }
        if level.short == 0 {
            return Ok(Some(frame));
        }
        if best.as_ref().is_none_or(|(short, _)| level.short < *short) {
            best = Some((level.short, frame));
        }
    }
    if let Some((_, frame)) = best {
        return Ok(Some(frame));
    }
    // Each of the ten leaves a long side level: one more shear, where none of the long sides is, among
    // the shears of the ten's sign, or failing that among those of the other sign. The tolerance there
    // is no more than that of the shear of the same size and sign `x`'s, and is taken at the end of the
    // stretch where the coordinates are largest.
    let reach = |k: f64| {
        let m = plain
            .iter()
            .flat_map(|(p, _)| p.elements().iter().flat_map(crate::boolean::el_points))
            .fold(0.0f64, |m, p| m.max((p.y + k * p.x).abs()).max(p.x.abs()));
        crate::boolean::sweep_tolerance(m).max(widest)
    };
    // Only shears in which the coordinates stay within what the sweep takes are searched: the middle of
    // the widest stretch free of the long sides could be one where they do not, with another stretch
    // narrower, and in the range, that is a frame.
    let Some((valid_lo, valid_hi)) = coordinate_valid_shears(&plain) else { return Err(PathOpsError::Degenerate) };
    for (lo, hi) in [(1.0_f64 / 1024.0, 0.5_f64), (-0.5, -1.0 / 1024.0)] {
        let (lo, hi) = (lo.max(valid_lo), hi.min(valid_hi));
        if lo >= hi {
            continue;
        }
        let eps = reach(lo).max(reach(hi));
        let Some(k) = free_shear(&opens, eps, lo, hi) else { continue };
        if let Sheared::Framed(frame, open) = sheared(&plain, &opens, k)
            && !level_sides(&open, frame.eps).long
        {
            return Ok(Some(frame));
        }
    }
    Err(PathOpsError::Degenerate)
}

/// The shears `k` in which no coordinate of `plain` sheared by it (`y + k x`, and `x`) is past what the sweep
/// takes ([`crate::boolean::largest_extent`], where its tolerance is over its limit): an interval, as the
/// coordinates are linear in `k`; `None` when there is none (some `x` is past it, or a `y` with no `x`).
fn coordinate_valid_shears(plain: &[(BezPath, usize)]) -> Option<(f64, f64)> {
    let limit = crate::boolean::largest_extent();
    let (mut lo, mut hi) = (f64::NEG_INFINITY, f64::INFINITY);
    for p in plain.iter().flat_map(|(p, _)| p.elements().iter().flat_map(crate::boolean::el_points)) {
        if !(p.x.abs() <= limit && p.y.is_finite()) {
            return None;
        }
        // |y + k x| <= limit
        if p.x > 0.0 {
            lo = lo.max((-limit - p.y) / p.x);
            hi = hi.min((limit - p.y) / p.x);
        } else if p.x < 0.0 {
            lo = lo.max((limit - p.y) / p.x);
            hi = hi.min((-limit - p.y) / p.x);
        } else if p.y.abs() > limit {
            return None;
        }
    }
    (lo <= hi).then_some((lo, hi))
}

/// The frame [`try_frame_for`] finds, if any (the tests' view of it).
#[cfg(test)]
pub(super) fn frame_for(shapes: &[Shape]) -> Option<Frame> {
    try_frame_for(shapes).ok().flatten()
}

/// What shearing the paths of a map gives ([`sheared`]).
enum Sheared {
    /// The frame, and the open subpaths in it, snapped with the rest.
    Framed(Frame, BezPath),
    /// The sweep's tolerance in this frame passes its limit; in another it may not.
    TooWide,
    /// A point is not finite, in any frame.
    NotFinite,
}

/// The paths `plain` sheared by `k` and snapped to the sweep's tolerance there, and the open subpaths
/// `opens` with them (to the same heights). Counted as a try ([`Work::FrameTries`]).
fn sheared(plain: &[(BezPath, usize)], opens: &BezPath, k: f64) -> Sheared {
    work::add(Work::FrameTries, 1);
    let (forward, back) = shear(k);
    let mut inputs = plain.to_vec();
    let mut open = opens.clone();
    open.apply_affine(forward);
    for (p, _) in &mut inputs {
        p.apply_affine(forward);
    }
    let eps = match eps_for(inputs.iter().map(|p| &p.0)) {
        Ok(eps) => eps,
        Err(crate::PathOpsError::NonFinite) => return Sheared::NotFinite,
        Err(_) => return Sheared::TooWide,
    };
    snap_horizontals(&mut inputs.iter_mut().map(|p| &mut p.0).chain(std::iter::once(&mut open)).collect::<Vec<_>>(), eps);
    Sheared::Framed(Frame { inputs, eps, back }, open)
}

/// A level side longer than this many tolerances of the sweep is long: a frame that leaves one level
/// is not used. A side no longer is short, and does not make a frame fail.
///
/// A side whose two ends are within the sweep's tolerance in height is level in whichever shear is
/// used: the shear moves its far end by its length times the shear, which is under the tolerance for
/// a side this short, and the ends are snapped together. Taken as long, one such side (a separate
/// stub, a point repeated a hair apart, a handle a nanometre long) would make every shear fail and
/// empty the map. Where the heights of the two ends of a side level in all ten shears, snapping
/// puts them within one tolerance in the shear of 1/2 and in the one of 1/1024, and so the side is
/// under about 5 tolerances long (4 across, 3 up); 8 leaves room. A side that short is at the noise of the
/// sweep, which cannot tell it from a point: its two passes may cancel and the piece be lost, in every
/// frame that levels it ([`try_frame_for`] looks for one that does not).
const NOISE_SIDE: f64 = 8.0;

/// A level side no longer than this many tolerances is not counted at all: the steepest shear, of
/// 1/2, raises the far end of a side this long by no more than one tolerance, so it is level in every
/// frame, and counting it would only make every frame look as bad as the others.
const MIN_SIDE: f64 = 2.0;

/// The level sides of the open subpaths of a frame ([`level_sides`]).
struct Level {
    /// Some side longer than [`NOISE_SIDE`] tolerances is level.
    long: bool,
    /// The level sides from [`MIN_SIDE`] to [`NOISE_SIDE`] tolerances long.
    short: usize,
}

impl Level {
    /// Counts a level side `length` long, in a frame of tolerance `eps`, as long or short (or not at all).
    fn count(&mut self, length: f64, eps: f64) {
        if length > NOISE_SIDE * eps {
            self.long = true;
        } else if length > MIN_SIDE * eps {
            self.short += 1;
        }
    }
}

/// Is `d` level (rising by less than a millionth of its length)?
fn is_level(d: kurbo::Vec2) -> bool {
    d.y.abs() <= 1e-6 * (d.x.abs() + d.y.abs())
}

/// The sides of the open subpaths `open`, in a frame of tolerance `eps`, that are level: of each
/// segment, its chord and each side of its control polygon.
///
/// Each side is looked at by itself. A long line of segments each shorter than [`NOISE_SIDE`]
/// tolerances is not a long side: the map takes pieces whose ends lie within 16 tolerances of each
/// other for copies of one edge, and loses a line of such pieces in any frame, level or not.
fn level_sides(open: &BezPath, eps: f64) -> Level {
    let mut level = Level { long: false, short: 0 };
    for seg in open.segments() {
        let pts: Vec<Point> = match seg {
            PathSeg::Line(l) => vec![l.p0, l.p1],
            PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
            PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
        };
        let (first, last) = (pts.first().copied().unwrap_or(Point::ZERO), pts.last().copied().unwrap_or(Point::ZERO));
        // A line's one side is its chord, counted once.
        let controls = if pts.len() > 2 { pts.windows(2).map(|w| w[1] - w[0]).collect() } else { Vec::new() };
        for d in controls.into_iter().chain(std::iter::once(last - first)) {
            if is_level(d) {
                level.count(d.hypot(), eps);
            }
        }
    }
    level
}

/// A shear in `[lo, hi]` in which no long side of the open subpaths `opens` (in the document's
/// frame: a chord, or a side of a control polygon) is level, the tolerance being `eps` at most. A side
/// `d` is level in the shear `-dy / dx`, and in a shear `k` it rises by `|dy + k dx|`. Snapping
/// ([`snap_horizontals`]) moves each anchor down by under one tolerance, so a side between two anchors
/// is level after it only where it rose by under one tolerance (heights snapped apart are a tolerance
/// apart or more), or by under a millionth of its length more ([`is_level`]): only in the shears
/// within about `eps / |dx|` of its own. A side to a handle may be levelled by three tolerances (the
/// handle moves with its anchor and is levelled with one), so its stretch is three times as wide. The
/// middle of the widest stretch of shears that levels no long side is used, a multiple of 2^-14 where
/// the stretch holds one (so that a grid of coordinates stays exact); [`try_frame_for`] checks it like the
/// others. `None` when the long sides level every shear in the range.
fn free_shear(opens: &BezPath, eps: f64, lo: f64, hi: f64) -> Option<f64> {
    // A sixteenth of a tolerance more for the rounding of the shear itself (a unit in the last place
    // of the coordinates, which the tolerance is 64 of).
    let half_width = |d: kurbo::Vec2, tolerances: f64| tolerances * eps * (17.0 / 16.0) / d.x.abs() + 2e-6;
    let mut barred: Vec<(f64, f64)> = Vec::new();
    for seg in opens.segments() {
        let pts: Vec<Point> = match seg {
            PathSeg::Line(l) => vec![l.p0, l.p1],
            PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
            PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
        };
        let (first, last) = (pts.first().copied().unwrap_or(Point::ZERO), pts.last().copied().unwrap_or(Point::ZERO));
        // A line's one side is its chord, between its anchors; a curve's sides each have a handle.
        let handles = if pts.len() > 2 { pts.windows(2).map(|w| (w[1] - w[0], 3.0)).collect() } else { Vec::new() };
        for (d, tolerances) in handles.into_iter().chain(std::iter::once((last - first, 1.0))) {
            if d.hypot() > NOISE_SIDE * eps && d.x != 0.0 {
                let (k, w) = (-d.y / d.x, half_width(d, tolerances));
                barred.push((k - w, k + w));
            }
        }
    }
    barred.sort_by(|a, b| a.0.total_cmp(&b.0));
    // The widest stretch of `[lo, hi]` that no interval covers.
    let mut widest: Option<(f64, f64)> = None;
    let mut reach = lo;
    let mut take = |a: f64, b: f64| {
        if b > a && widest.is_none_or(|(x, y)| b - a > y - x) {
            widest = Some((a, b));
        }
    };
    for (a, b) in barred {
        if a > reach {
            take(reach, a.min(hi));
        }
        reach = reach.max(b);
        if reach >= hi {
            break;
        }
    }
    take(reach, hi);
    let (a, b) = widest?;
    let middle = 0.5 * (a + b);
    let step = 1.0 / 16384.0;
    let round = (middle / step).round() * step;
    Some(if round > a && round < b { round } else { middle })
}

/// Boundary crossings per input, counted from the far west: slot `2i` counts input `i`'s
/// segments running one way, slot `2i + 1` the other. An open path traced there and back adds one
/// to each, so its edges never cancel out of the map. Only the slots that aren't 0 are kept, in
/// order: the sweep keeps one of these for every edge, and a vector as long as the input's number
/// (as this was) made disjoint shapes cost the square of their count in memory.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(super) struct Crossings(pub(super) Vec<(usize, u32)>);

impl Crossings {
    pub(super) fn get(&self, slot: usize) -> u32 {
        self.0.binary_search_by_key(&slot, |(s, _)| *s).map_or(0, |i| self.0.get(i).map_or(0, |(_, c)| *c))
    }
}

impl std::ops::Add for Crossings {
    type Output = Crossings;
    fn add(mut self, rhs: Self) -> Crossings {
        self += rhs;
        self
    }
}

impl std::ops::AddAssign for Crossings {
    /// Adds `rhs`'s count of each slot to this one's, keeping the slots in order and none of zero.
    fn add_assign(&mut self, rhs: Self) {
        if rhs.0.is_empty() {
            return;
        }
        let mut out = Vec::with_capacity(self.0.len() + rhs.0.len());
        let (mut a, mut b) = (std::mem::take(&mut self.0).into_iter().peekable(), rhs.0.into_iter().peekable());
        loop {
            let next = match (a.peek(), b.peek()) {
                (Some(x), Some(y)) if x.0 == y.0 => {
                    let (x, y) = (a.next(), b.next());
                    x.zip(y).map(|(x, y)| (x.0, x.1.saturating_add(y.1)))
                }
                (Some(x), Some(y)) if x.0 < y.0 => a.next(),
                (Some(_), Some(_)) | (None, Some(_)) => b.next(),
                (Some(_), None) => a.next(),
                (None, None) => break,
            };
            out.extend(next.filter(|(_, c)| *c != 0));
        }
        self.0 = out;
    }
}

impl WindingNumber for Crossings {
    type Tag = usize;
    fn single(tag: usize, positive: bool) -> Self {
        Crossings(vec![(2 * tag + usize::from(!positive), 1)])
    }
    fn of_tag(&self, tag: usize) -> Self {
        Crossings([2 * tag, 2 * tag + 1].into_iter().map(|slot| (slot, self.get(slot))).filter(|(_, c)| *c != 0).collect())
    }
}

/// A point in the middle of a run of segments.
pub(super) fn mid(segs: &[Seg]) -> Option<Point> {
    segs.get(segs.len() / 2).map(|s| s.c.eval(0.5))
}

/// Rectangles looked up by a point inside them, when there are too many to test one by one for
/// every point (the map asks once per edge and once per hole, so a scan of them all made it
/// quadratic in the input, VectorCraft issue 937).
///
/// A packed R-tree (sort-tile-recursive): the rectangles are cut into leaves of [`FANOUT`] by
/// their position, the leaves into nodes of [`FANOUT`], and so on to a root, each node holding the
/// box of what is below it. A lookup descends only the nodes whose box holds the point. The tree
/// follows where the rectangles are, not how far apart they lie, so one shape far from the others,
/// or two clusters far from each other, cost nothing (a uniform grid over their union put all of
/// them in one cell). A rectangle that is long and thin on a diagonal, whose bounds take in most
/// of the art (an index of the bounds alone scanned a diagonal hatch whole for every point), is
/// filed as the bounds of its pieces instead ([`Self::with_footprint`]), or, past a budget of them,
/// as its bounds. A rectangle whose coordinates are not numbers holds no point and is left out.
pub(crate) struct RectIndex<T> {
    items: Vec<(kurbo::Rect, T)>,
    /// The rectangles filed, each with the item it is a footprint of, in the order of the leaves.
    pub(crate) entries: Vec<(kurbo::Rect, u32)>,
    nodes: Vec<Node>,
    root: Option<usize>,
    /// Items not filed (past the `u32` positions), scanned for every point. Items with bounds that
    /// are not finite are in neither: they hold no point.
    pub(crate) rest: Vec<usize>,
}

/// A node of the tree: the box of its children, which are `count` entries from `first` (a leaf) or
/// `count` nodes from `first`.
struct Node {
    rect: kurbo::Rect,
    first: u32,
    count: u32,
    leaf: bool,
}

/// Most children of a node.
const FANOUT: usize = 16;

/// Most rectangles one item may be filed as, per item and on average (past it an item is filed as
/// its bounds): a long segment filed as its pieces costs many, coincident ones a quadratic number.
const ENTRIES_PER_ITEM: usize = 64;

/// The rectangles in the order of a sort-tile-recursive packing: by the x of their centres into
/// slabs, each slab by the y of their centres, so that every `FANOUT` in a row are near.
fn tile<U>(rects: &mut [(kurbo::Rect, U)]) {
    let leaves = rects.len().div_ceil(FANOUT);
    let slabs = ((leaves as f64).sqrt().ceil() as usize).max(1);
    let centre = |r: &kurbo::Rect| r.center();
    rects.sort_by(|a, b| centre(&a.0).x.total_cmp(&centre(&b.0).x));
    for slab in rects.chunks_mut(slabs * FANOUT) {
        slab.sort_by(|a, b| centre(&a.0).y.total_cmp(&centre(&b.0).y));
    }
}

/// The side of the cells a long segment is cut into: that of a square holding one item on average
/// over the middle of the art. The middle is the box from the 5th percentile of the items' left and
/// top edges to the 95th of their right and bottom ones, so that a shape far out, or a few, do not
/// stretch it (the union of all the bounds did).
fn typical_side<T>(items: &[(kurbo::Rect, T)]) -> f64 {
    let finite: Vec<&kurbo::Rect> = items.iter().map(|(r, _)| r).filter(|r| r.is_finite()).collect();
    let edge = |f: fn(&kurbo::Rect) -> f64, upper: bool| {
        let mut v: Vec<f64> = finite.iter().map(|r| f(r)).collect();
        v.sort_by(f64::total_cmp);
        let k = v.len() / 20;
        let at = if upper { v.len().saturating_sub(1 + k) } else { k };
        v.get(at).copied().unwrap_or(0.0)
    };
    let (w, h) = (edge(|r| r.x1, true) - edge(|r| r.x0, false), edge(|r| r.y1, true) - edge(|r| r.y0, false));
    let n = finite.len().max(1) as f64;
    // A grid of collinear rectangles (zero area) is divided along its length.
    let side = (w * h / n).sqrt();
    if side > 0.0 { side } else { w.max(h) / n }.max(1e-9)
}

impl<T> RectIndex<T> {
    pub(crate) fn new(items: Vec<(kurbo::Rect, T)>) -> Self {
        Self::with_footprint(items, |r, _, _| vec![*r])
    }

    /// `footprint(bounds, item, cell side)` gives the rectangles an item is filed under: a lookup
    /// finds the item when its bounds hold the point and one footprint rectangle is within the
    /// lookup's tolerance of it. The bounds themselves (`new`) suit items as compact as their
    /// bounds; a long diagonal segment is better filed under the bounds of its pieces, about a
    /// cell long each, which keep to the segment rather than take in everything its bounds cover.
    pub(crate) fn with_footprint(items: Vec<(kurbo::Rect, T)>, footprint: impl Fn(&kurbo::Rect, &T, f64) -> Vec<kurbo::Rect>) -> Self {
        let side = typical_side(&items);
        // Also under what the tree's `u32` positions count, however many items there are.
        let mut budget = items.len().saturating_mul(ENTRIES_PER_ITEM).saturating_add(4096).min(1 << 30);
        let (mut entries, mut rest): (Vec<(kurbo::Rect, u32)>, Vec<usize>) = (Vec::new(), Vec::new());
        for (i, (r, t)) in items.iter().enumerate() {
            // Bounds that are not numbers hold no point: the item is left out, not scanned at every
            // lookup (a path of them made the lookups of the others cost its size).
            if !r.is_finite() {
                continue;
            }
            // Entries hold `u32` indices; an item past them is scanned.
            let Ok(index) = u32::try_from(i) else {
                rest.push(i);
                continue;
            };
            let pieces = footprint(r, t, side);
            if !pieces.iter().all(kurbo::Rect::is_finite) {
                continue;
            }
            if pieces.len() > budget {
                // Past the budget the item is filed under its bounds, one rectangle: a lookup reads it
                // where its bounds hold the point and nowhere else (scanned, it was read at every one,
                // and a map of a thousand tall rectangles nested read most of them for every edge).
                entries.push((*r, index));
                budget = budget.saturating_sub(1);
                continue;
            }
            budget -= pieces.len();
            entries.extend(pieces.into_iter().map(|p| (p, index)));
        }
        let (nodes, root) = Self::pack(&mut entries);
        Self { items, entries, nodes, root, rest }
    }

    /// The tree over `entries`, which it puts in the order of its leaves: its nodes, bottom level
    /// first, and the index of the root.
    fn pack(entries: &mut [(kurbo::Rect, u32)]) -> (Vec<Node>, Option<usize>) {
        if entries.is_empty() {
            return (Vec::new(), None);
        }
        tile(entries);
        let union = |rects: &mut dyn Iterator<Item = kurbo::Rect>| rects.reduce(|a, b| a.union(b)).unwrap_or(kurbo::Rect::ZERO);
        let mut level: Vec<Node> = entries
            .chunks(FANOUT)
            .enumerate()
            .map(|(k, c)| Node { rect: union(&mut c.iter().map(|e| e.0)), first: (k * FANOUT) as u32, count: c.len() as u32, leaf: true })
            .collect();
        let mut nodes: Vec<Node> = Vec::new();
        while level.len() > 1 {
            // Each level is tiled in turn, then stored, so that the children of a parent are
            // consecutive nodes.
            let mut tiled: Vec<(kurbo::Rect, Node)> = level.into_iter().map(|n| (n.rect, n)).collect();
            tile(&mut tiled);
            let base = nodes.len();
            level = tiled
                .chunks(FANOUT)
                .enumerate()
                .map(|(k, c)| Node { rect: union(&mut c.iter().map(|n| n.0)), first: (base + k * FANOUT) as u32, count: c.len() as u32, leaf: false })
                .collect();
            nodes.extend(tiled.into_iter().map(|(_, n)| n));
        }
        let root = level.into_iter().next().map(|n| {
            nodes.push(n);
            nodes.len() - 1
        });
        (nodes, root)
    }

    /// The items whose rectangle, grown by `tol`, holds `p` (and, with a footprint, one of whose
    /// footprint rectangles is within `tol` of it), in the order they were given.
    pub(crate) fn at(&self, p: Point, tol: f64) -> impl Iterator<Item = &T> {
        let mut found = self.rest.clone();
        work::add(Work::IndexCandidates, found.len());
        let mut stack: Vec<usize> = self.root.into_iter().collect();
        while let Some(i) = stack.pop() {
            let Some(node) = self.nodes.get(i) else { continue };
            work::add(Work::IndexNodes, 1);
            if !node.rect.inflate(tol, tol).contains(p) {
                continue;
            }
            let children = node.first as usize..(node.first + node.count) as usize;
            if node.leaf {
                for (r, item) in self.entries.get(children).unwrap_or_default() {
                    work::add(Work::IndexCandidates, 1);
                    if r.inflate(tol, tol).contains(p) {
                        found.push(*item as usize);
                    }
                }
            } else {
                stack.extend(children);
            }
        }
        found.sort_unstable();
        found.dedup();
        found.into_iter().filter_map(move |i| self.items.get(i)).filter(move |(r, _)| r.inflate(tol, tol).contains(p)).map(|(_, t)| t)
    }
}

/// A segment's footprint in a [`RectIndex`]: the bounds of its pieces about a cell long, so a long
/// diagonal is filed as pieces that keep to it, not as the bounds that take in everything it crosses.
pub(crate) fn segment_footprint(bounds: &kurbo::Rect, seg: &PathSeg, cell: f64) -> Vec<kurbo::Rect> {
    let pieces = (bounds.width().max(bounds.height()) / cell).ceil();
    // A count past the cap gives longer pieces, each filed under more cells.
    if pieces.is_nan() || pieces <= 1.0 {
        return vec![*bounds];
    }
    let pieces = pieces.min(4096.0) as usize;
    (0..pieces).map(|k| seg.subsegment(k as f64 / pieces as f64..(k + 1) as f64 / pieces as f64).bounding_box()).collect()
}
