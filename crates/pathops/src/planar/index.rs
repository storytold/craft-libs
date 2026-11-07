// Carved from VectorCraft's crates/pathops (src/planar/index.rs) at storytold/vectorcraft ff9deee46d1c59dc9ecef0b668c25e57cd80b55e.
// Copyright (c) 2026 ArtCraft Team and the VectorCraft contributors. MIT OR Apache-2.0.

use super::*;
use crate::work::{self, Work};

/// The shears the sweep's frame may use, `y + k x`, the first that leaves no open path level in it:
/// by default 1/32, in which only an edge sloping -1/32 in the document is horizontal. Each is a
/// power of two, so it maps typical coordinates exactly.
pub(super) const SHEARS: [f64; 6] = [1.0 / 32.0, 1.0 / 64.0, 1.0 / 16.0, 1.0 / 128.0, 1.0 / 8.0, 1.0 / 256.0];

/// The shear by `k` and its inverse.
pub(super) fn shear(k: f64) -> (Affine, Affine) {
    (Affine::new([1.0, k, 0.0, 1.0, 0.0, 0.0]), Affine::new([1.0, -k, 0.0, 1.0, 0.0, 0.0]))
}

/// The shear for `shapes`: the first of [`SHEARS`] in whose frame no open subpath has a side (of a
/// segment's chord or control polygon) that is level, which the sweep takes the two passes of
/// for cancelling out. All of them level for one is not a case worth more: the first is used.
pub(super) fn shear_for(shapes: &[Shape]) -> f64 {
    let level = |k: f64| {
        shapes.iter().any(|s| {
            crate::path::subs(&s.path).iter().filter(|sp| !sp.closed).any(|sp| {
                sp.segments().any(|seg| {
                    let pts: Vec<Point> = match seg {
                        PathSeg::Line(l) => vec![l.p0, l.p1],
                        PathSeg::Quad(q) => vec![q.p0, q.p1, q.p2],
                        PathSeg::Cubic(c) => vec![c.p0, c.p1, c.p2, c.p3],
                    };
                    let (first, last) = (pts.first().copied().unwrap_or(Point::ZERO), pts.last().copied().unwrap_or(Point::ZERO));
                    let sides = pts.windows(2).map(|w| w[1] - w[0]).chain(std::iter::once(last - first));
                    sides.into_iter().any(|d| d.hypot2() > 0.0 && (d.y + k * d.x).abs() <= 1e-6 * (d.x.abs() + d.y.abs()))
                })
            })
        })
    };
    SHEARS.iter().copied().find(|&k| !level(k)).unwrap_or(SHEARS[0])
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
/// filed as the bounds of its pieces instead ([`Self::with_footprint`]), or, failing that, scanned
/// for every point. A rectangle whose coordinates are not numbers holds no point and is left out.
pub(crate) struct RectIndex<T> {
    items: Vec<(kurbo::Rect, T)>,
    /// The rectangles filed, each with the item it is a footprint of, in the order of the leaves.
    pub(crate) entries: Vec<(kurbo::Rect, u32)>,
    nodes: Vec<Node>,
    root: Option<usize>,
    /// Items not filed (past the entry budget, or the `u32` positions), scanned for every point.
    /// Items with bounds that are not finite are in neither: they hold no point.
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

/// Most rectangles one item may be filed as, per item and on average (past it the item is
/// scanned): a long segment filed as its pieces costs many, coincident ones a quadratic number.
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
                rest.push(i);
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
pub(super) fn segment_footprint(bounds: &kurbo::Rect, seg: &PathSeg, cell: f64) -> Vec<kurbo::Rect> {
    let pieces = (bounds.width().max(bounds.height()) / cell).ceil();
    // A count past the cap gives longer pieces, each filed under more cells.
    if pieces.is_nan() || pieces <= 1.0 {
        return vec![*bounds];
    }
    let pieces = pieces.min(4096.0) as usize;
    (0..pieces).map(|k| seg.subsegment(k as f64 / pieces as f64..(k + 1) as f64 / pieces as f64).bounding_box()).collect()
}
