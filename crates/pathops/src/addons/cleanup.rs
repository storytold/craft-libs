//! Artwork Cleanup's path repairs, each on one subpath and returning how much it changed (so a
//! check is the repair run on a copy): collinear anchors, duplicate anchors, tiny handles and
//! nearly closed paths; [`near_point_groups`] finds anchors to snap together, and [`rejoin`] welds
//! open pieces whose ends meet back into paths.

use std::collections::{BTreeSet, HashMap};

use craft_geom::{Anchor, AnchorKind, EPS, SubPath};
use kurbo::{Point, Vec2};

use crate::fit::{end_tangent, start_tangent};

/// The fewest anchors a subpath keeps: two when open, three when closed.
fn min_anchors(sp: &SubPath) -> usize {
    if sp.closed { 3 } else { 2 }
}

/// `a`'s handle offset `h - a.p` moved onto `p`.
fn moved(p: Point, a_p: Point, h: Point) -> Point {
    p + (h - a_p)
}

/// Remove the corner anchors (no handles, straight segments both sides) where a straight run goes
/// on in the same direction: the sine of the angle it turns there is at most `tolerance`. Open ends
/// stay, and a subpath keeps two anchors (three when closed) → how many went.
///
/// Each anchor is tested once, first to last, against the nearest anchors still kept on either
/// side. On a closed subpath, an anchor with none kept before it has the last anchor as its
/// previous neighbor, and the last anchor has the first one kept as its next neighbor. The kept
/// anchors move into place once at the end, so the work grows linearly with the number of anchors.
pub fn remove_collinear_anchors(sp: &mut SubPath, tolerance: f64) -> usize {
    let n = sp.anchors.len();
    let min = min_anchors(sp);
    if n <= min {
        return 0;
    }
    let mut keep = vec![true; n];
    let (mut left, mut first_kept, mut last_kept) = (n, None, None);
    #[cfg(test)]
    let mut work = 0;
    for i in 0..n {
        if left <= min {
            break;
        }
        #[cfg(test)]
        {
            work += 1;
        }
        // Every anchor after `i` is still there, the last one included.
        let p = last_kept.unwrap_or(n - 1);
        let q = if i + 1 < n { i + 1 } else { first_kept.unwrap_or(i) };
        let inner = sp.closed || (last_kept.is_some() && i + 1 < n);
        let (Some(prev), Some(cur), Some(next)) = (sp.anchors.get(p), sp.anchors.get(i), sp.anchors.get(q)) else { break };
        let straight = !prev.has_out() && !cur.has_in() && !cur.has_out() && !next.has_in();
        let (a, b) = (cur.p - prev.p, next.p - cur.p);
        let len = a.hypot() * b.hypot();
        if inner && straight && len > EPS && a.dot(b) > 0.0 && a.cross(b).abs() / len <= tolerance {
            if let Some(k) = keep.get_mut(i) {
                *k = false;
            }
            left -= 1;
        } else {
            first_kept.get_or_insert(i);
            last_kept = Some(i);
        }
    }
    if left < n {
        let mut kept = keep.iter();
        sp.anchors.retain(|_| kept.next().copied().unwrap_or(true));
        #[cfg(test)]
        {
            work += n;
        }
    }
    #[cfg(test)]
    COLLINEAR_WORK.with(|w| w.set(work));
    n - left
}

#[cfg(test)]
thread_local! {
    /// Anchors tested plus anchors moved by the last [`remove_collinear_anchors`] on this thread.
    static COLLINEAR_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Merge each anchor lying within `distance` of the one before it (and a closed subpath's last
/// anchor within `distance` of its first) into that one, which keeps its in handle and takes the
/// merged anchor's out handle → how many went.
pub fn merge_duplicate_anchors(sp: &mut SubPath, distance: f64) -> usize {
    let before = sp.anchors.len();
    let mut out: Vec<Anchor> = Vec::with_capacity(before);
    for a in sp.anchors.drain(..) {
        match out.last_mut() {
            Some(l) if l.p.distance(a.p) <= distance => *l = Anchor::with_handles(l.p, l.h_in, moved(l.p, a.p, a.h_out)),
            _ => out.push(a),
        }
    }
    if sp.closed
        && out.len() > 1
        && let (Some(first), Some(last)) = (out.first(), out.last())
        && first.p.distance(last.p) <= distance
        && let Some(last) = out.pop()
        && let Some(f) = out.first_mut()
    {
        *f = Anchor::with_handles(f.p, moved(f.p, last.p, last.h_in), f.h_out);
    }
    sp.anchors = out.into();
    before - sp.anchors.len()
}

/// Retract the handles shorter than `length` (but not already on their anchor); an anchor that
/// loses a handle becomes a corner → how many handles were retracted.
pub fn retract_tiny_handles(sp: &mut SubPath, length: f64) -> usize {
    let mut count = 0;
    for a in &mut sp.anchors {
        for out in [false, true] {
            let h = if out { &mut a.h_out } else { &mut a.h_in };
            let l = h.distance(a.p);
            if l > EPS && l < length {
                *h = a.p;
                a.kind = AnchorKind::Corner;
                count += 1;
            }
        }
    }
    count
}

/// Close an open subpath of three or more anchors whose ends lie within `distance` of each other:
/// the last anchor merges into the first, which takes its in handle → whether it closed.
pub fn close_nearly_closed(sp: &mut SubPath, distance: f64) -> bool {
    let ends_meet = sp.anchors.first().zip(sp.anchors.last()).is_some_and(|(f, l)| f.p.distance(l.p) <= distance);
    if sp.closed || sp.anchors.len() < 3 || !ends_meet {
        return false;
    }
    let Some(last) = sp.anchors.pop() else { return false };
    if let Some(first) = sp.anchors.first_mut() {
        *first = Anchor::with_handles(first.p, moved(first.p, last.p, last.h_in), first.h_out);
    }
    sp.closed = true;
    true
}

/// The cell of a grid of `size`-wide squares that `p` lies in.
fn grid_cell(p: Point, size: f64) -> (i64, i64) {
    ((p.x / size).floor() as i64, (p.y / size).floor() as i64)
}

/// The cell `(x, y)` and the cells around it, each once.
fn around((x, y): (i64, i64)) -> Vec<(i64, i64)> {
    let mut out: Vec<(i64, i64)> = [-1, 0, 1].into_iter().flat_map(|dx| [-1, 0, 1].map(|dy| (x.saturating_add(dx), y.saturating_add(dy)))).collect();
    out.sort_unstable();
    out.dedup();
    out
}

/// The root of `i`'s set in the union-find forest `parent`.
fn root(parent: &mut [usize], mut i: usize) -> usize {
    while let Some(&p) = parent.get(i).filter(|&&p| p != i) {
        let up = parent.get(p).copied().unwrap_or(p);
        if let Some(slot) = parent.get_mut(i) {
            *slot = up;
        }
        i = p;
    }
    i
}

/// Groups of `points` to snap together: points within `distance` of one another (and so on, one
/// to the next) that aren't all at the same place, skipping the pairs `linked` names (such as
/// neighbouring anchors of one path). Each group lists its points in order; snapping them all to
/// the first one makes them coincide. Non-finite points and a distance that isn't a positive number
/// find nothing.
///
/// Two points are compared when their cells of a `distance`-wide grid touch. Each group grows from
/// one point at a time, which looks only at points no group has taken yet, so a cluster of points
/// all within `distance` of one another is grouped with work that grows linearly with its size.
pub fn near_point_groups(points: &[Point], distance: f64, linked: impl Fn(usize, usize) -> bool) -> Vec<Vec<usize>> {
    if !(distance > 0.0 && distance.is_finite()) {
        return vec![];
    }
    let mut cells = buckets(points, distance);
    let mut near = Near {
        distance,
        linked,
        parent: (0..points.len()).collect(),
        taken: points.iter().map(|p| !p.is_finite()).collect(),
        queue: vec![],
        #[cfg(test)]
        work: 0,
    };
    for (start, &p) in points.iter().enumerate() {
        if near.taken.get(start) != Some(&false) {
            continue;
        }
        if let Some(t) = near.taken.get_mut(start) {
            *t = true;
        }
        let mut at = Some((start, p));
        while let Some((i, p)) = at {
            near.reach(&mut cells, i, p);
            at = near.queue.pop().and_then(|j| Some((j, *points.get(j)?)));
        }
    }
    #[cfg(test)]
    NEAR_WORK.with(|w| w.set(near.work));
    // Only pairs that aren't at the same place were joined, so no group is all at one place.
    let mut groups: HashMap<usize, Vec<usize>> = HashMap::new();
    for i in 0..points.len() {
        groups.entry(root(&mut near.parent, i)).or_default().push(i);
    }
    let mut out: Vec<Vec<usize>> = groups.into_values().filter(|g| g.len() > 1).collect();
    out.sort_unstable();
    out
}

#[cfg(test)]
thread_local! {
    /// Buckets, places and points looked at by the last [`near_point_groups`] on this thread.
    static NEAR_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// The finite points of one cell of a grid that lie in one square a quarter of a cell wide, with
/// the box around them.
struct Bucket {
    lo: Point,
    hi: Point,
    /// Each place (bit for bit) and the points there that no group has taken yet, though some of
    /// them may have been taken since.
    places: Vec<(Point, Vec<usize>)>,
}

impl Bucket {
    /// Whether [`Point::distance`] puts every point here further than `distance` from `p`: one
    /// coordinate alone differs by more. The square of a `distance` of 1e-150 or more does not
    /// underflow, so the measured distance is at least that difference.
    fn beyond(&self, p: Point, distance: f64) -> bool {
        distance >= 1e-150 && [self.lo.x - p.x, p.x - self.hi.x, self.lo.y - p.y, p.y - self.hi.y].into_iter().any(|g| g > distance)
    }
}

/// The finite `points` in buckets, by grid cell.
fn buckets(points: &[Point], distance: f64) -> HashMap<(i64, i64), Vec<Bucket>> {
    let quarter = distance / 4.0;
    let mut cells: HashMap<(i64, i64), Vec<Bucket>> = HashMap::new();
    // Each bucket by its cell and square, and each place by its cell, bucket and coordinates.
    let mut bucket_at: HashMap<[i64; 4], usize> = HashMap::new();
    let mut place_at: HashMap<(usize, [i64; 2], [u64; 2]), usize> = HashMap::new();
    for (i, &p) in points.iter().enumerate().filter(|(_, p)| p.is_finite()) {
        let c = grid_cell(p, distance);
        let list = cells.entry(c).or_default();
        let square = grid_cell(p, quarter);
        let b = *bucket_at.entry([c.0, c.1, square.0, square.1]).or_insert_with(|| {
            list.push(Bucket { lo: p, hi: p, places: vec![] });
            list.len() - 1
        });
        let Some(bucket) = list.get_mut(b) else { continue };
        bucket.lo = Point::new(bucket.lo.x.min(p.x), bucket.lo.y.min(p.y));
        bucket.hi = Point::new(bucket.hi.x.max(p.x), bucket.hi.y.max(p.y));
        let at = *place_at.entry((b, [c.0, c.1], [p.x.to_bits(), p.y.to_bits()])).or_insert_with(|| {
            bucket.places.push((p, vec![]));
            bucket.places.len() - 1
        });
        if let Some((_, here)) = bucket.places.get_mut(at) {
            here.push(i);
        }
    }
    cells
}

/// The grouping behind [`near_point_groups`].
struct Near<F> {
    distance: f64,
    linked: F,
    /// The union-find forest of the groups.
    parent: Vec<usize>,
    /// Whether each point is in a group being grown or grown already (non-finite ones never are).
    taken: Vec<bool>,
    /// Points taken into the group being grown whose own neighbors are still to be looked at.
    queue: Vec<usize>,
    #[cfg(test)]
    work: usize,
}

impl<F: Fn(usize, usize) -> bool> Near<F> {
    /// Take into `i`'s group every point no group has taken yet that lies within the distance of
    /// `p`, `i`'s place, in the cells around it, at a different place and not linked to `i`.
    fn reach(&mut self, cells: &mut HashMap<(i64, i64), Vec<Bucket>>, i: usize, p: Point) {
        for c in around(grid_cell(p, self.distance)) {
            let Some(buckets) = cells.get_mut(&c) else { continue };
            buckets.retain_mut(|b| {
                #[cfg(test)]
                {
                    self.work += 1;
                }
                if !b.beyond(p, self.distance) {
                    b.places.retain_mut(|place| {
                        self.visit(i, p, place);
                        !place.1.is_empty()
                    });
                }
                !b.places.is_empty()
            });
        }
    }

    /// Take into `i`'s group the points at `place` that `i` joins, and drop the ones taken.
    fn visit(&mut self, i: usize, p: Point, (at, here): &mut (Point, Vec<usize>)) {
        #[cfg(test)]
        {
            self.work += 1;
        }
        let d = p.distance(*at);
        if !(d <= self.distance && d > 0.0) {
            return;
        }
        here.retain(|&j| {
            if self.taken.get(j) != Some(&false) {
                return false;
            }
            #[cfg(test)]
            {
                self.work += 1;
            }
            if (self.linked)(i.min(j), i.max(j)) {
                return true;
            }
            let (ri, rj) = (root(&mut self.parent, i), root(&mut self.parent, j));
            if let Some(slot) = self.parent.get_mut(ri.max(rj)) {
                *slot = ri.min(rj);
            }
            if let Some(t) = self.taken.get_mut(j) {
                *t = true;
            }
            self.queue.push(j);
            false
        });
    }
}

/// Open pieces welded into one path by [`rejoin`]: `members` index the pieces in the order they
/// run along `path`.
#[derive(Clone, Debug, PartialEq)]
pub struct Rejoined {
    pub members: Vec<usize>,
    pub path: SubPath,
}

/// One end of a piece: (piece, true for its last anchor).
type End = (usize, bool);

/// The direction pointing out of the piece at `end`.
fn outward(sp: &SubPath, last: bool) -> Option<Vec2> {
    let n = sp.segment_count();
    Some(match last {
        true => end_tangent(&sp.segment(n.checked_sub(1)?)),
        false => -start_tangent(&sp.segment(0)),
    })
}

/// Weld the open `pieces` whose ends lie within `gap` of each other back into paths: an end meets
/// at most one other, and where an end could meet several, it meets the one the path turns least
/// to go on into (then the nearest). A piece may run either way; a chain whose ends meet closes.
/// Each junction keeps the earlier piece's anchor and the later piece's out handle. Pieces that are
/// closed, have fewer than two anchors or a non-finite coordinate stay out. → the chains of two or
/// more pieces, and the single pieces that closed.
///
/// Two ends are compared when their cells of a `gap`-wide grid touch, and pairs are taken in order
/// of the turn, then the distance, then the order of the ends. Each end looks for its first
/// partner in that order among the ends still free, searching outward from the reverse of its own
/// direction, and two ends that are each other's first partner meet; these are the pairs that
/// taking every pair in that order gives.
pub fn rejoin(pieces: &[SubPath], gap: f64) -> Vec<Rejoined> {
    if !(gap >= 0.0 && gap.is_finite()) {
        return vec![];
    }
    let usable = |sp: &&SubPath| {
        !sp.closed && sp.anchors.len() >= 2 && sp.anchors.iter().all(|a| a.p.is_finite() && a.h_in.is_finite() && a.h_out.is_finite())
    };
    // Each usable end: where it is and the way out of its piece there.
    let ends: Vec<EndAt> = pieces
        .iter()
        .enumerate()
        .filter(|(_, sp)| usable(sp))
        .flat_map(|(i, sp)| [(i, sp, false), (i, sp, true)])
        .filter_map(|(i, sp, last)| {
            let a = if last { sp.anchors.last() } else { sp.anchors.first() }?;
            Some(((i, last), a.p, outward(sp, last)?))
        })
        .collect();
    let link = Weld::new(pieces, &ends, gap).link();
    chains(pieces, &link)
}

/// An end, where it lies and the unit direction pointing out of its piece there.
type EndAt = (End, Point, Vec2);

/// Where a pair of ends (`a` < `b` in the list of ends) comes in the order [`rejoin`] takes pairs:
/// the turn across the junction, the distance between the ends, `a`, the place of `b`'s grid cell
/// among the cells around `a`'s, and `b`.
type Order = (f64, f64, usize, usize, usize);

/// Whether pair `x` comes before pair `y`.
fn earlier(x: &Order, y: &Order) -> bool {
    x.0.total_cmp(&y.0).then(x.1.total_cmp(&y.1)).then((x.2, x.3, x.4).cmp(&(y.2, y.3, y.4))).is_lt()
}

/// The place of cell `to` among the cells around `from`, listed by x step and then y step from -1
/// to 1 (cells past the end of the grid repeat their neighbor; the first place counts).
fn listed_at(from: (i64, i64), to: (i64, i64)) -> usize {
    let mut steps = [-1, 0, 1].into_iter().flat_map(|dx| [-1, 0, 1].map(|dy| (from.0.saturating_add(dx), from.1.saturating_add(dy))));
    steps.position(|c| c == to).unwrap_or(9)
}

/// `angle` as an integer in the same order.
fn angle_key(angle: f64) -> i64 {
    let bits = angle.to_bits() as i64;
    bits ^ (((bits >> 63) as u64) >> 1) as i64
}

/// The angle between the directions at angles `a` and `b`, from 0 to π.
fn apart(a: f64, b: f64) -> f64 {
    let d = (a - b).abs();
    d.min(std::f64::consts::TAU - d)
}

/// How far past the turn of the best pair found so far [`Weld::first_partner`] still looks. The
/// turn [`rejoin`] measures (the arc cosine of a dot product of unit vectors) is within 1e-7 of
/// the angle between the directions, and the angles the search sorts by are within 1e-14 of it.
const TURN_SLACK: f64 = 1e-6;

/// Ends in one grid cell at one place with one outward direction (bit for bit): every other end
/// pairs with each of them at the same turn and distance.
struct Class {
    at: Point,
    angle: f64,
    /// The ends here no pair has taken yet.
    free: BTreeSet<usize>,
}

/// The pairing behind [`rejoin`].
struct Weld<'a> {
    pieces: &'a [SubPath],
    ends: &'a [EndAt],
    gap: f64,
    /// The width of the grid's cells.
    size: f64,
    classes: Vec<Class>,
    /// Each end's class.
    class_of: Vec<usize>,
    /// Each grid cell's classes with free ends, by the angle of their direction.
    cells: HashMap<(i64, i64), BTreeSet<(i64, usize)>>,
    /// Whether each end is still free.
    free: Vec<bool>,
    /// Whether each end has looked for its first partner. Another end's first partner in a class
    /// is one of the class's first three free ends, so the ends after those cannot meet next and
    /// look only once they move up.
    awake: Vec<bool>,
    /// Each awake end's first partner, and where that pair comes in the order.
    first: Vec<Option<(Order, usize)>>,
    /// The ends whose first partner each end is (some may have moved on since).
    wanted_by: Vec<Vec<usize>>,
    /// Ends to check for a partner whose first partner they are.
    todo: Vec<usize>,
    #[cfg(test)]
    work: std::cell::Cell<usize>,
}

impl<'a> Weld<'a> {
    fn new(pieces: &'a [SubPath], ends: &'a [EndAt], gap: f64) -> Self {
        let size = gap.max(1e-9);
        let (mut classes, mut class_of, mut cells) = (vec![], vec![], HashMap::new());
        let mut class_at: HashMap<((i64, i64), [u64; 4]), usize> = HashMap::new();
        for (i, &(_, p, out)) in ends.iter().enumerate() {
            let c = grid_cell(p, size);
            let k = *class_at.entry((c, [p.x, p.y, out.x, out.y].map(f64::to_bits))).or_insert_with(|| {
                let angle = out.y.atan2(out.x);
                cells.entry(c).or_insert_with(BTreeSet::new).insert((angle_key(angle), classes.len()));
                classes.push(Class { at: p, angle, free: BTreeSet::new() });
                classes.len() - 1
            });
            if let Some(class) = classes.get_mut(k) {
                class.free.insert(i);
            }
            class_of.push(k);
        }
        let n = ends.len();
        Weld {
            pieces,
            ends,
            gap,
            size,
            classes,
            class_of,
            cells,
            free: vec![true; n],
            awake: vec![false; n],
            first: vec![None; n],
            wanted_by: vec![vec![]; n],
            todo: vec![],
            #[cfg(test)]
            work: std::cell::Cell::new(0),
        }
    }

    /// The links between the ends that meet: two free ends that are each other's first partner
    /// meet, and the ends that wanted either of them look again, until no such pair is left.
    fn link(mut self) -> HashMap<End, End> {
        for k in 0..self.classes.len() {
            self.wake(k);
        }
        let mut link: HashMap<End, End> = HashMap::new();
        while let Some(v) = self.todo.pop() {
            let Some(w) = self.partner(v) else { continue };
            if self.free.get(v) != Some(&true) || self.free.get(w) != Some(&true) || self.partner(w) != Some(v) {
                continue;
            }
            let (Some(&(ev, ..)), Some(&(ew, ..))) = (self.ends.get(v), self.ends.get(w)) else { continue };
            link.insert(ev, ew);
            link.insert(ew, ev);
            self.take(v);
            self.take(w);
            for e in [v, w] {
                if let Some(&k) = self.class_of.get(e) {
                    self.wake(k);
                }
            }
            let waiting: Vec<usize> = [v, w].iter().flat_map(|&e| self.wanted_by.get_mut(e).map(std::mem::take).unwrap_or_default()).collect();
            for x in waiting {
                if self.free.get(x) == Some(&true) && matches!(self.partner(x), Some(p) if p == v || p == w) {
                    self.look(x);
                }
            }
        }
        #[cfg(test)]
        REJOIN_WORK.with(|w| w.set(self.work.get()));
        link
    }

    /// End `v`'s first partner, once it has looked.
    fn partner(&self, v: usize) -> Option<usize> {
        self.first.get(v).and_then(|f| f.as_ref()).map(|f| f.1)
    }

    /// Have the first three free ends of class `k` that have not looked for a partner look.
    fn wake(&mut self, k: usize) {
        let Some(class) = self.classes.get(k) else { return };
        let sleeping: Vec<usize> = class.free.iter().take(3).copied().filter(|&e| self.awake.get(e) == Some(&false)).collect();
        for e in sleeping {
            if let Some(a) = self.awake.get_mut(e) {
                *a = true;
            }
            self.look(e);
        }
    }

    /// Find free end `e`'s first partner, and queue `e` to check whether that end wants it too.
    fn look(&mut self, e: usize) {
        let next = self.first_partner(e);
        if let Some(list) = next.as_ref().and_then(|f| self.wanted_by.get_mut(f.1)) {
            list.push(e);
        }
        if let Some(f) = self.first.get_mut(e) {
            *f = next;
        }
        self.todo.push(e);
    }

    /// End `e` has met another: it is no longer free.
    fn take(&mut self, e: usize) {
        if let Some(f) = self.free.get_mut(e) {
            *f = false;
        }
        let Some(&k) = self.class_of.get(e) else { return };
        let Some(class) = self.classes.get_mut(k) else { return };
        class.free.remove(&e);
        if class.free.is_empty()
            && let Some(&(_, p, _)) = self.ends.get(e)
            && let Some(set) = self.cells.get_mut(&grid_cell(p, self.size))
        {
            set.remove(&(angle_key(class.angle), k));
        }
    }

    /// The free end `v` meets first, and where that pair comes in the order: the classes in the
    /// cells around `v`'s are looked at by how far their direction is from the reverse of `v`'s,
    /// until that is more than [`TURN_SLACK`] past the best turn found.
    fn first_partner(&self, v: usize) -> Option<(Order, usize)> {
        let &((piece, _), p, out) = self.ends.get(v)?;
        let toward = (-out.y).atan2(-out.x);
        let split = (angle_key(toward), 0);
        let mut best: Option<(Order, usize)> = None;
        for c in around(grid_cell(p, self.size)) {
            let Some(set) = self.cells.get(&c) else { continue };
            // Both ways round from `toward`; between them they meet every class once.
            let mut up = set.range(split..).chain(set.range(..split)).peekable();
            let mut down = set.range(..split).rev().chain(set.range(split..).rev()).peekable();
            let turn = |e: Option<&&(i64, usize)>| e.and_then(|e| self.classes.get(e.1)).map_or(f64::INFINITY, |k| apart(k.angle, toward));
            for _ in 0..set.len() {
                let (tu, td) = (turn(up.peek()), turn(down.peek()));
                if best.as_ref().is_some_and(|b| tu.min(td) > b.0.0 + TURN_SLACK) {
                    break;
                }
                let Some(&(_, k)) = (if tu <= td { up.next() } else { down.next() }) else { break };
                if let Some(pair) = self.pair_with(v, p, piece, k)
                    && best.as_ref().is_none_or(|b| earlier(&pair.0, &b.0))
                {
                    best = Some(pair);
                }
            }
        }
        best
    }

    /// The first free end of class `k` that free end `v` (at `p`, on `piece`) may meet, and where
    /// that pair comes in the order. Within a class the pairs differ only in the ends' order, so
    /// the end listed first comes first.
    fn pair_with(&self, v: usize, p: Point, piece: usize, k: usize) -> Option<(Order, usize)> {
        #[cfg(test)]
        self.work.set(self.work.get() + 1);
        let class = self.classes.get(k)?;
        if p.distance(class.at) > self.gap {
            return None;
        }
        // A piece's own two ends meet only when closing it leaves a shape (three anchors).
        let short = self.pieces.get(piece).is_none_or(|sp| sp.anchors.len() < 3);
        let u = *class.free.iter().find(|&&u| u != v && !(short && self.ends.get(u).is_some_and(|e| e.0.0 == piece)))?;
        let (a, b) = (v.min(u), v.max(u));
        let (&(_, pa, oa), &(_, pb, ob)) = (self.ends.get(a)?, self.ends.get(b)?);
        let at = listed_at(grid_cell(pa, self.size), grid_cell(pb, self.size));
        Some(((oa.dot(-ob).clamp(-1.0, 1.0).acos(), pa.distance(pb), a, at, b), u))
    }
}

#[cfg(test)]
thread_local! {
    /// Classes of ends looked at by the last [`rejoin`] on this thread.
    static REJOIN_WORK: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// Walk the links between piece ends into chains.
fn chains(pieces: &[SubPath], link: &HashMap<End, End>) -> Vec<Rejoined> {
    let mut seen = vec![false; pieces.len()];
    let mut out = vec![];
    for start in 0..pieces.len() {
        if seen.get(start) != Some(&false) || !(link.contains_key(&(start, false)) || link.contains_key(&(start, true))) {
            continue;
        }
        // A member is (piece, reversed): it comes in by end (piece, reversed) and leaves by
        // (piece, !reversed). Back up to the chain's first piece, or round to `start` in a loop.
        let mut first = (start, false);
        let mut closed = false;
        for _ in 0..pieces.len() {
            let Some(&(p, at_last)) = link.get(&first) else { break };
            // The piece before leaves by its end `at_last`.
            let prev = (p, !at_last);
            if prev.0 == start {
                closed = true;
                break;
            }
            first = prev;
        }
        let members = walk(link, first, closed, &mut seen);
        if let Some(path) = assemble(pieces, &members, closed) {
            out.push(Rejoined { members: members.iter().map(|m| m.0).collect(), path });
        }
    }
    out
}

/// The pieces of the chain that begins with `first` (piece, reversed), in order.
fn walk(link: &HashMap<End, End>, first: (usize, bool), closed: bool, seen: &mut [bool]) -> Vec<(usize, bool)> {
    let mut members = vec![];
    let mut cur = first;
    loop {
        match seen.get_mut(cur.0) {
            Some(s) if !*s => *s = true,
            _ => break,
        }
        members.push(cur);
        // A piece run forwards leaves by its last anchor.
        let Some(&(p, at_last)) = link.get(&(cur.0, !cur.1)) else { break };
        let next = (p, at_last);
        if closed && next.0 == first.0 {
            break;
        }
        cur = next;
    }
    members
}

/// The anchors of `pieces` along the chain `members`, each piece's start merged into the end before it.
fn assemble(pieces: &[SubPath], members: &[(usize, bool)], closed: bool) -> Option<SubPath> {
    if members.len() < 2 && !closed {
        return None;
    }
    let mut anchors: Vec<Anchor> = vec![];
    for &(i, reversed) in members {
        let mut sp = pieces.get(i)?.clone();
        if reversed {
            sp.reverse();
        }
        let mut rest = sp.anchors.into_iter();
        match (anchors.last_mut(), rest.next()) {
            (Some(l), Some(f)) => *l = Anchor::with_handles(l.p, l.h_in, moved(l.p, f.p, f.h_out)),
            (None, Some(f)) => anchors.push(f),
            _ => {}
        }
        anchors.extend(rest);
    }
    let mut sp = SubPath::new(anchors, false);
    if closed && !close_nearly_closed(&mut sp, f64::INFINITY) {
        sp.closed = sp.anchors.len() >= 2;
    }
    Some(sp)
}

#[cfg(test)]
mod tests;
