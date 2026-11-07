//! A sweep-line implementation using weak orderings.
//!
//! This algorithm is documented in `docs/sweep.typ`.

use std::cell::{RefCell, RefMut};

#[cfg(feature = "slow-asserts")]
use std::ops::DerefMut;

use crate::{
    curve::{self, CurveInteraction, CurveOrder, Order},
    geom::Segment,
    num::CheapOrderedFloat,
    order::ComparisonCache,
    segments::{SegIdx, Segments},
    treevec::TreeVec,
};

use super::{OutputEvent, SweepLineRange, SweepLineRangeBuffers};

/// Because we associate curve touches and crossings as ranges of
/// heights (and not just a single height), we have some choice as to
/// where we actually handle the crossing. In the write-up, there are
/// a few requirements for the intersection event depending on the
/// context where it was calculated (e.g. whether it was for an
/// intersection event or an exit event).
///
/// In the actual code, we're a little less careful about the strict
/// requirements than the write-up. But we do need to distinguish a
/// couple of cases, at least.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum WhichTouch {
    /// Choose a touch height that's sort of in the middle of the
    /// allowed range.
    Mid,
    /// Choose a touch height that's sort of at the end of the allowed
    /// range.
    Late,
}

/// How hard we should try to make the output actually correct.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
#[cfg_attr(test, derive(serde::Serialize))]
pub enum Correctness {
    /// The output contours are only guaranteed to be approximately
    /// correctly ordered. If you evaluate them with exact math, you
    /// may find, for example, that two sets we claim to be disjoint
    /// in fact have a small overlap. The size of this hypothetical
    /// overlap is bounded by the tolerance parameter.
    ///
    /// This is usually the right choice, especially for graphics
    /// applications.
    GoodEnough,
    /// The output contours are guaranteed to be correctly ordered: if
    /// you evaluate the output contours using exact math, you'll find
    /// that that satisfy the claimed disjointness and containment
    /// properties. This sounds like a good thing, but it comes at a
    /// substantial performance cost, and it usually produces output
    /// contours with many more subdivisions than necessary.
    ReallyCorrect,
}

#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy)]
pub(crate) struct SegmentOrderEntry {
    pub(crate) seg: SegIdx,
    /// True if this segment is about to leave the sweep-line.
    ///
    /// We handle enter/exits like this:
    ///
    /// 1. insert the newly entered segments
    /// 2. mark the about-to-exit segments
    /// 3. process intersections and re-shuffle things
    /// 4. output the geometry
    /// 5. remove the exited segments that we marked in step 2.
    ///
    /// The reason we don't remove about-to-exit segments immediately in
    /// step 2 is to make it easier to compare the old and new orderings.
    /// We keep track of the re-shuffling in step 3 (see `old_idx` below),
    /// and the lack of deletions means that we don't need to worry about
    /// indices shifting.
    exit: bool,
    enter: bool,
    /// This is epsilon below this segment's smallest horizontal position. All
    /// horizontal positions smaller than this are guaranteed not to interact
    /// with this segment.
    lower_bound: f64,
    /// This is epsilon above this segment's largest horizontal position. All
    /// horizontal positions larger than this are guaranteed not to interact
    /// with this segment.
    upper_bound: f64,
    /// This is filled out during `compute_changed_intervals`, where we use it as
    /// a sort of "dirty" flag to avoid processing an entry twice.
    in_changed_interval: bool,
    /// We need to keep track of two sweep-line orders at once: the previous one
    /// and the current one. And if a large sweep-line has only one small change,
    /// we want the cost of this tracking to be small. We do this by keeping the
    /// sweep line in the "current" order, and then whenever some segments have
    /// their order changed, we remember their old positions.
    ///
    /// So, for example, say we're in a situation like this, where the dashed
    /// horizontal line is the position of the sweep:
    ///
    /// ```text
    /// s_1  s_3   s_5  s_7
    ///  │     ╲   ╱     ╲
    ///  │      ╲ ╱       ╲
    /// ╌│╌╌╌╌╌╌╌╳╌╌╌╌╌╌╌╌╌╲
    ///  │      ╱ ╲         ╲
    /// ```
    ///
    /// The old order is [s_1, s_3, s_5, s_7], and we start off with all the old_idx
    /// fields set to `None`. Then we swap `s_3` and `s_5`, so the current order
    /// is [s_1, s_5, s_3, s_7] and we set the old_idx fields to be
    /// [None, Some(2), Some(1), None]. This allows us to reconstruct the original
    /// order when we need to.
    old_idx: Option<usize>,
    /// When two segments are contour-adjacent, we allow them to "share" the same
    /// sweep-line slot. This helps performance (because we aren't constantly
    /// removing and inserting segments in the middle of the sweep-line), and makes
    /// it easier to generate good output (because if we have both contour-adjacent
    /// segments handy, it's easy to avoid unnecessary horizontal segments).
    ///
    /// So in a situation like this, for example:
    ///
    /// ```text
    /// s_1   s_3     s_7
    ///  │      ╲       ╲
    ///  │       ╲       ╲
    ///  │       ╱        ╲
    ///  │      ╱          ╲
    ///       s_5
    /// ```
    ///
    /// when the sweep-line hits the "kink" the new order will be [s_1, s_5, s_7]
    /// and the old_seg values will be [None, Some(s_3), None].
    ///
    /// Note that `old_seg` is not guaranteed to get used for all contour-adjacent
    /// segments, even if they're monotonic in y: if there are some other annoying
    /// segments nearby, the new segment and the old segment might get separated
    /// in the sweep-line. In this case, they will get their own entries.
    ///
    /// If this is `Some`, both `enter` and `exit` will be true.
    old_seg: Option<SegIdx>,
}

impl std::fmt::Debug for SegmentOrderEntry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // TODO: maybe a verbose mode to show all the state?
        if let Some(old_seg) = self.old_seg {
            write!(f, "{old_seg:?}->{:?}", self.seg)
        } else {
            write!(f, "{:?}", self.seg)
        }
    }
}

impl SegmentOrderEntry {
    fn new(seg: SegIdx, segments: &Segments, eps: f64) -> Self {
        let x0 = segments[seg].min_x();
        let x1 = segments[seg].max_x();
        Self {
            seg,
            exit: false,
            enter: false,
            lower_bound: x0.min(x1) - eps,
            upper_bound: x0.max(x1) + eps,
            in_changed_interval: false,
            old_idx: None,
            old_seg: None,
        }
    }

    fn reset_state(&mut self) {
        self.exit = false;
        self.enter = false;
        self.in_changed_interval = false;
        self.old_idx = None;
        self.old_seg = None;
    }

    fn set_old_idx_if_unset(&mut self, i: usize) {
        if self.old_idx.is_none() {
            self.old_idx = Some(i);
        }
    }

    fn old_seg(&self) -> SegIdx {
        self.old_seg.unwrap_or(self.seg)
    }

    pub(crate) fn is_in_changed_interval(&self) -> bool {
        self.in_changed_interval
    }

    pub(crate) fn will_really_exit(&self) -> bool {
        self.exit && self.old_seg.is_none()
    }
}

#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Default)]
pub(crate) struct SegmentOrder {
    pub(crate) segs: TreeVec<SegmentOrderEntry, 128>,
}

impl std::fmt::Debug for SegmentOrder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.segs.iter()).finish()
    }
}

impl SegmentOrder {
    fn seg(&self, i: usize) -> SegIdx {
        self.segs[i].seg
    }

    #[cfg(feature = "slow-asserts")]
    fn is_exit(&self, i: usize) -> bool {
        let seg = &self.segs[i];
        seg.will_really_exit()
    }
}

#[derive(Clone, Debug, PartialEq)]
struct IntersectionEvent {
    pub y: f64,
    /// This segment used to be to the left, and after the intersection it will be to the right.
    ///
    /// In our sweep line intersection, this segment might have already been moved to the right by
    /// some other constraints. That's ok.
    pub left: SegIdx,
    /// This segment used to be to the right, and after the intersection it will be to the left.
    pub right: SegIdx,
}

impl Eq for IntersectionEvent {}

impl Ord for IntersectionEvent {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        (CheapOrderedFloat::from(self.y), self.left, self.right).cmp(&(
            CheapOrderedFloat::from(other.y),
            other.left,
            other.right,
        ))
    }
}

impl PartialOrd for IntersectionEvent {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Default)]
struct EventQueue {
    /// The enter events are stored in `Segments`; this is the index of the first
    /// one that we haven't processed yet.
    next_enter_idx: usize,
    /// The index of the first exit event that we haven't processed yet.
    next_exit_idx: usize,
    intersection: std::collections::BTreeSet<IntersectionEvent>,
}

impl EventQueue {
    pub fn push(&mut self, ev: IntersectionEvent) {
        self.intersection.insert(ev);
    }

    pub fn next_y<'a>(&'a self, segments: &'a Segments) -> Option<f64> {
        let enter_y = segments
            .entrances()
            .get(self.next_enter_idx)
            .map(|(y, _)| *y);
        let exit_y = segments.exits().get(self.next_exit_idx).map(|(y, _)| *y);
        let int_y = self.intersection.first().map(|i| i.y);

        [enter_y, exit_y, int_y]
            .into_iter()
            .flatten()
            .min_by_key(|y| CheapOrderedFloat::from(*y))
    }

    pub fn entrances_at_y<'a>(&mut self, y: &f64, segments: &'a Segments) -> &'a [(f64, SegIdx)] {
        let entrances = &segments.entrances()[self.next_enter_idx..];
        let count = entrances
            .iter()
            .position(|(enter_y, _)| enter_y > y)
            .unwrap_or(entrances.len());
        self.next_enter_idx += count;
        &entrances[..count]
    }

    pub fn exits_at_y<'a>(&mut self, y: &f64, segments: &'a Segments) -> &'a [(f64, SegIdx)] {
        let exits = &segments.exits()[self.next_exit_idx..];
        let count = exits
            .iter()
            .position(|(exit_y, _)| exit_y > y)
            .unwrap_or(exits.len());
        self.next_exit_idx += count;
        &exits[..count]
    }

    pub fn next_intersection_at_y(&mut self, y: f64) -> Option<IntersectionEvent> {
        if self.intersection.first().map(|i| i.y) == Some(y) {
            self.intersection.pop_first()
        } else {
            None
        }
    }
}

/// Holds some buffers that are used when iterating over a sweep-line.
///
/// Save on re-allocation by allocating this once and reusing it in multiple calls to
/// [`Sweeper::next_line`].
#[derive(Clone, Debug, Default)]
pub struct SweepLineBuffers {
    /// A subset of the old sweep-line.
    old_line: Vec<SegmentOrderEntry>,
    /// A vector of (segment, min allowable horizontal position, max allowable horizontal position).
    positions: Vec<(SegmentOrderEntry, f64, f64)>,
    output_events: Vec<OutputEvent>,
}

/// Encapsulates the state of the sweep-line algorithm and allows iterating over sweep lines.
#[derive(Clone, Debug)]
pub struct Sweeper<'a> {
    y: f64,
    eps: f64,
    correctness: Correctness,
    pub(crate) line: SegmentOrder,
    events: EventQueue,
    segments: &'a Segments,

    horizontals: Vec<SegIdx>,

    // The collection of segments that we know need to be given explicit
    // positions in the current sweep line.
    //
    // These include:
    // - any segments that changed order with any other segments
    // - any segments that entered or exited
    //
    // These segments are identified by their index in the current order, so that
    // it's fast to find them. It means that we need to do some fixing-up of indices after
    // inserting all the new segments.
    segs_needing_positions: Vec<usize>,
    changed_intervals: Vec<ChangedInterval>,

    comparisons: RefCell<ComparisonCache>,
    conservative_comparisons: RefCell<ComparisonCache>,
}

impl<'segs> Sweeper<'segs> {
    /// Creates a new sweeper for a collection of segments, and with a
    /// given tolerance.
    ///
    /// The really correct mode doesn't actually change any of the
    /// output orders, but it does emit extra events whenever lines
    /// touch without crossing.
    pub fn new_with_correctness(
        segments: &'segs Segments,
        eps: f64,
        correctness: Correctness,
    ) -> Self {
        let events = EventQueue::default();
        Sweeper {
            eps,
            line: SegmentOrder::default(),
            correctness,
            y: events.next_y(segments).unwrap_or(f64::INFINITY),
            events,
            segments,
            segs_needing_positions: Vec::new(),
            changed_intervals: Vec::new(),
            horizontals: Vec::new(),
            comparisons: RefCell::new(ComparisonCache::new(eps, eps / 2.0)),
            conservative_comparisons: RefCell::new(ComparisonCache::new(4.0 * eps, eps / 2.0)),
        }
    }

    /// Creates a new sweeper for a collection of segments, and with a
    /// given tolerance.
    pub fn new(segments: &'segs Segments, eps: f64) -> Self {
        Self::new_with_correctness(segments, eps, Correctness::GoodEnough)
    }

    fn compare_segments(&self, i: SegIdx, j: SegIdx) -> RefMut<'_, CurveOrder> {
        RefMut::map(self.comparisons.borrow_mut(), |c| {
            c.compare_segments(self.segments, i, j)
        })
    }

    fn compare_segments_conservatively(&self, i: SegIdx, j: SegIdx) -> RefMut<'_, CurveOrder> {
        RefMut::map(self.conservative_comparisons.borrow_mut(), |c| {
            c.compare_segments(self.segments, i, j)
        })
    }

    /// Moves the sweep forward, returning the next sweep line.
    ///
    /// Returns `None` when sweeping is complete.
    pub fn next_line<'slf, 'buf>(
        &'slf mut self,
        bufs: &'buf mut SweepLineBuffers,
    ) -> Option<SweepLine<'buf, 'slf, 'segs>> {
        self.check_invariants();

        let y = self.events.next_y(self.segments)?;
        debug_assert!(y >= self.y);
        self.advance(y);
        self.check_invariants();

        // Process all the enter events at this y.
        {
            let enters = self.events.entrances_at_y(&y, self.segments);
            for (enter_y, idx) in enters {
                debug_assert_eq!(enter_y, &y);
                self.handle_enter(*idx);
                self.check_invariants();
            }
        }

        // Process all the exit events.
        {
            let exits = self.events.exits_at_y(&y, self.segments);
            for (exit_y, idx) in exits {
                debug_assert_eq!(exit_y, &y);
                self.handle_exit(*idx);
                self.check_invariants();
            }
        }

        // Process all the intersection events at this y.
        while let Some(intersection) = self.events.next_intersection_at_y(y) {
            self.handle_intersection(intersection.left, intersection.right);
            self.check_invariants();
        }

        self.compute_changed_intervals();
        Some(SweepLine {
            state: self,
            next_changed_interval: 0,
            bufs,
        })
    }

    fn advance(&mut self, y: f64) {
        // All the exiting segments should be in segs_needing_positions, so find them all and remove them.
        self.segs_needing_positions
            .retain(|idx| self.line.segs[*idx].exit && self.line.segs[*idx].old_seg.is_none());

        // Reset the state flags for all segments. All segments with non-trivial state flags should
        // belong to the changed intervals. This needs to go before we remove the exiting segments,
        // because that messes up the indices.
        for r in self.changed_intervals.drain(..) {
            for seg in self.line.segs.range_mut(r.segs) {
                seg.reset_state();
            }
        }

        // Remove the exiting segments in reverse order, so the indices stay good.
        self.segs_needing_positions.sort();
        self.segs_needing_positions.dedup();
        for &idx in self.segs_needing_positions.iter().rev() {
            self.line.segs.remove(idx);
        }
        self.y = y;
        self.segs_needing_positions.clear();
        self.horizontals.clear();
    }

    fn intersection_scan_right(&mut self, start_idx: usize, which: WhichTouch) {
        let seg_idx = self.line.seg(start_idx);
        let y = self.y;

        // We're allowed to take a potentially-smaller height bound by taking
        // into account the current queue. A larger height bound is still ok,
        // just a little slower.
        let seg = &self.segments[seg_idx];
        // Looking explicitly at the ending `y` coordinate is more reliable than
        // checking `is_exit`, because this method can get called before exits
        // get processed. (TODO: do we actually need to store exit and enter
        // as separate states? Can't we just always look at the position?)
        if seg.end().y == y {
            return;
        }
        let mut height_bound = seg.end().y;

        for j in (start_idx + 1)..self.line.segs.len() {
            let other_idx = self.line.seg(j);
            let other = &self.segments[other_idx];
            if other.end().y == y {
                continue;
            }

            // Our basic comparisons use eps tolerance and eps/2 accuracy, so if
            // our max_x is 1.5 eps less than their min_x then we're guaranteed
            // to always compare Order::Left of them.
            if seg.max_x() + 2.0 * self.eps < other.min_x() {
                break;
            }
            height_bound = height_bound.min(other.end().y);

            // TODO: there's a choice to be made here: do we distinguish in the
            // event queue between actual intersections and near-touches? For
            // now, no. We will distinguish them in handle_intersection

            let touch = self
                .compare_segments(seg_idx, other_idx)
                .next_touch_after(y);
            if let Some(int_y) = touch.and_then(|touch| choose_cross_height(touch, y, which)) {
                let (left_idx, right_idx) = match touch {
                    Some(curve::CurveInteraction::Touch(..)) => (j, start_idx),
                    _ => (start_idx, j),
                };
                // We only keep intersections that happen before the end
                // of a segment. This is allowed because the ordering
                // invariants are closed and so they won't be violated right
                // at the end. And keeping out intersections-at-the-end
                // out works better with the boundary conditions of
                // [CurveOrder::entry_at].
                if int_y < seg.end().y && int_y < other.end().y {
                    self.events.push(IntersectionEvent {
                        y: int_y,
                        left: self.line.seg(left_idx),
                        right: self.line.seg(right_idx),
                    });
                    height_bound = height_bound.min(int_y);
                }
            }

            let cmp = self.compare_segments_conservatively(seg_idx, other_idx);
            if let Some((_y0, y1, order)) = cmp.iter().find(|(_, y1, _)| *y1 > y)
                && order == Order::Left
                && y1 >= height_bound
            {
                break;
            }
        }
    }

    // This is basically the same as `intersection_scan_right`, so maybe it's worth combining them.
    fn intersection_scan_left(&mut self, start_idx: usize, which: WhichTouch) {
        let seg_idx = self.line.seg(start_idx);
        let y = self.y;

        let seg = &self.segments[seg_idx];
        if seg.end().y == y {
            return;
        }
        let mut height_bound = seg.end().y;

        for j in (0..start_idx).rev() {
            let other_idx = self.line.seg(j);
            let other = &self.segments[other_idx];
            if other.end().y == y {
                continue;
            }

            if seg.min_x() - 2.0 * self.eps > other.max_x() {
                break;
            }
            height_bound = height_bound.min(other.end().y);

            let touch = self
                .compare_segments(other_idx, seg_idx)
                .next_touch_after(y);
            if let Some(int_y) = touch.and_then(|touch| choose_cross_height(touch, y, which)) {
                let (left_idx, right_idx) = match touch {
                    Some(curve::CurveInteraction::Touch(..)) => (start_idx, j),
                    _ => (j, start_idx),
                };
                if int_y < seg.end().y && int_y < other.end().y {
                    self.events.push(IntersectionEvent {
                        y: int_y,
                        left: self.line.seg(left_idx),
                        right: self.line.seg(right_idx),
                    });
                    height_bound = height_bound.min(int_y);
                }
            }

            let cmp = self.compare_segments_conservatively(other_idx, seg_idx);
            if let Some((_y0, y1, order)) = cmp.iter().find(|(_, y1, _)| *y1 > y)
                && order == Order::Left
                && y1 >= height_bound
            {
                break;
            }
        }
    }

    fn scan_for_removal(&mut self, pos: usize) {
        if pos > 0 {
            self.intersection_scan_right(pos - 1, WhichTouch::Mid);
            self.intersection_scan_left(pos - 1, WhichTouch::Mid);
        }
    }

    fn insert(&mut self, pos: usize, seg: SegmentOrderEntry) {
        self.line.segs.insert(pos, seg);
        self.intersection_scan_right(pos, WhichTouch::Mid);
        self.intersection_scan_left(pos, WhichTouch::Mid);
    }

    fn handle_enter(&mut self, seg_idx: SegIdx) {
        let new_seg = &self.segments[seg_idx];

        if new_seg.is_horizontal() {
            self.horizontals.push(seg_idx);
            return;
        }

        // Do we have a contour neighbor? If so, use it for the initial position
        // (which may be adjusted by intersections later). If the contour neighbor
        // ends here, we just take their slot. If the contour neighbor starts here,
        // we take a neighboring slot.
        let contour_prev_idx = if self.segments.positively_oriented(seg_idx) {
            self.segments.contour_prev(seg_idx)
        } else {
            self.segments.contour_next(seg_idx)
        };

        let mut pos = None;
        if let Some(contour_prev_idx) = contour_prev_idx {
            let contour_prev = &self.segments[contour_prev_idx];
            if contour_prev.start().y < self.y {
                let pos = self.line.position(contour_prev_idx).unwrap();
                debug_assert!(contour_prev.end() == new_seg.start());
                self.handle_contour_continuation(seg_idx, new_seg, pos);
                return;
            } else if let Some(contour_prev_pos) = self.line.position(contour_prev_idx) {
                // Both of us entered on this sweep-line.
                debug_assert!(contour_prev.start() == new_seg.start());
                pos = match self
                    .compare_segments(seg_idx, contour_prev_idx)
                    .order_after(self.y)
                {
                    Order::Left => Some(contour_prev_pos),
                    Order::Right => Some(contour_prev_pos + 1),
                    Order::Ish => {
                        // Maybe this is overkill, but it gives better results for our snapshot
                        // tests (which admittedly have a very large "ish" parameter). If two
                        // segments enter at the same `y` and they are always close to one another,
                        // choose their ordering according to their average slope. We could also
                        // go by the initial slope, but that's less numerically stable.
                        let diff = new_seg.end().to_kurbo() - new_seg.start().to_kurbo();
                        let contour_prev_diff =
                            contour_prev.end().to_kurbo() - contour_prev.start().to_kurbo();
                        // Assuming the y-diffs are non-negative, this is basically
                        // `if diff.x / diff.y < contour_prev_diff.x / contour_prev_diff.y`
                        if diff.x * contour_prev_diff.y < contour_prev_diff.x * diff.y {
                            Some(contour_prev_pos)
                        } else {
                            Some(contour_prev_pos + 1)
                        }
                    }
                };
            }
        }

        let pos = pos.unwrap_or_else(|| {
            self.line.insertion_idx(
                self.y,
                self.segments,
                seg_idx,
                &mut self.comparisons.borrow_mut(),
            )
        });

        let mut entry = SegmentOrderEntry::new(seg_idx, self.segments, self.eps);
        entry.enter = true;
        entry.exit = false;
        self.insert(pos, entry);
        self.add_seg_needing_position(pos, true);
    }

    fn add_seg_needing_position(&mut self, pos: usize, insert: bool) {
        // Fix up the index of any other segments that we got inserted before
        // (at this point, segs_needing_positions only contains newly-inserted
        // segments, and it's sorted increasing).
        //
        // We sorted all the to-be-inserted segments by horizontal position
        // before inserting them, so we expect these two loops to be short most
        // of the time.
        if insert {
            for other_pos in self.segs_needing_positions.iter_mut().rev() {
                if *other_pos >= pos {
                    *other_pos += 1;
                } else {
                    break;
                }
            }
        }
        let insert_pos = self
            .segs_needing_positions
            .iter()
            .rposition(|p| *p < pos)
            .map(|p| p + 1)
            .unwrap_or(0);
        self.segs_needing_positions.insert(insert_pos, pos);
        debug_assert!(self.segs_needing_positions.is_sorted());
    }

    // A special case of handle-enter, in which the entering segment is
    // continuing the contour of an exiting segment.
    fn handle_contour_continuation(&mut self, seg_idx: SegIdx, seg: &Segment, pos: usize) {
        let x0 = seg.min_x();
        let x1 = seg.max_x();
        self.line.segs[pos].old_seg = Some(self.line.segs[pos].seg);
        self.line.segs[pos].seg = seg_idx;
        self.line.segs[pos].enter = true;
        self.line.segs[pos].exit = true;
        self.line.segs[pos].lower_bound = x0.min(x1) - self.eps;
        self.line.segs[pos].upper_bound = x0.max(x1) + self.eps;
        self.intersection_scan_right(pos, WhichTouch::Mid);
        self.intersection_scan_left(pos, WhichTouch::Mid);
        self.add_seg_needing_position(pos, false);
    }

    /// Marks a segment as needing to exit, but doesn't actually remove it
    /// from the sweep-line. See `SegmentOrderEntry::exit` for an explanation.
    fn handle_exit(&mut self, seg_idx: SegIdx) {
        let pos = self.line.position(seg_idx).unwrap();

        if self.line.segs[pos].old_seg == Some(seg_idx) {
            return;
        }

        // It's important that this goes before `scan_for_removal`, so that
        // the scan doesn't get confused by the segment that should be marked
        // for exit.
        self.line.segs[pos].exit = true;
        self.scan_for_removal(pos);
        self.segs_needing_positions.push(pos);
    }

    fn handle_intersection(&mut self, left: SegIdx, right: SegIdx) {
        let left_idx = self.line.position(left).unwrap();
        let right_idx = self.line.position(right).unwrap();
        if left_idx < right_idx {
            // We're going to put `left_seg` after `right_seg` in the
            // sweep line, and while doing so we need to "push" along
            // all segments that are strictly bigger than `left_seg`
            // (slight false positives are allowed; no false negatives).
            let mut to_move = vec![left_idx];
            for j in (left_idx + 1)..right_idx {
                let seg_j_entry = &self.line.segs[j];
                if seg_j_entry.will_really_exit() {
                    continue;
                }

                let seg_j_idx = seg_j_entry.seg;
                let (_, y1, left_j_order) = self.compare_segments(left, seg_j_idx).entry_at(self.y);
                if left_j_order == Order::Left {
                    // Because the order isn't transitive, it's possible that
                    // we've been asked to push `seg_j` to the right of `right`,
                    // even though `seg_j` compares left of `right`. If this
                    // happens, we delay the intersection. This delay won't cause
                    // a bad ordering between `left` and `right`, because that
                    // would imply an ordering cycle
                    // (`left` < `seg_j` < `right` < `left`).
                    let (_, y2, j_right_order) =
                        self.compare_segments(seg_j_idx, right).entry_at(self.y);
                    if j_right_order == Order::Left {
                        self.events.push(IntersectionEvent {
                            y: y1.min(y2),
                            left,
                            right,
                        });
                        return;
                    }

                    to_move.push(j);
                }
            }

            // Keep track of segment movement. TODO: reference a more detailed write-up
            self.segs_needing_positions.extend(left_idx..=right_idx);
            for (i, entry) in self.line.segs.range_mut(left_idx..=right_idx).enumerate() {
                if entry.old_idx.is_none() {
                    entry.old_idx = Some(left_idx + i);
                }
            }

            // Remove them in reverse to make indexing easier.
            let mut removed = Vec::with_capacity(to_move.len());
            for &j in to_move.iter().rev() {
                removed.push(self.line.segs.remove(j));
                self.scan_for_removal(j);
            }

            // We want to insert them at what was previously `right_idx + 1`, but the
            // index changed because of the removal.
            let insertion_pos = right_idx + 1 - to_move.len();

            for seg in removed {
                // This is almost the same as
		// self.insert(insertion_pos, seg), except that we use
		// the "late" touching point in some
		// circumstances. The late touching point is allowed
		// here (because the removal scans above have ensured
		// the crossing invariant for everything except the
		// segment that we're "inserting").
		//
		// Choosing the late touching point ensures that our y
		// keeps moving forwards, because if our shuffling in
		// this `handle_intersection` has caused some new
		// intersection event and we choose that event's
		// latest possible `y`, that event must have strictly
		// larger `y` than our current `y` (if not, it would
		// violate the cyclic order property).
                let which = WhichTouch::Late;
                self.line.segs.insert(insertion_pos, seg);
                self.intersection_scan_right(insertion_pos, which);
                self.intersection_scan_left(insertion_pos, which);
            }
        } else {
            // This is for handling the "touch but don't cross" case.
            // As discussed in `intersection_scan_right`, maybe we could handle
            // this with a different kind of intersection event.
            //
            // Currently, this leads to over-division for intersections
            // that were already handled.
            if self.correctness == Correctness::ReallyCorrect {
                self.segs_needing_positions.extend(right_idx..=left_idx);
            }

            // We haven't changed the order of any segments, but the "touch" event
            // we just processed might have been a witness for the crossing invariant.
            // So we need to scan.
            self.intersection_scan_right(right_idx, WhichTouch::Mid);
            self.intersection_scan_left(left_idx, WhichTouch::Mid);
        }
    }

    // If we have a segment in a changed interval, then every other segment
    // that it compares "Ish" to should also be in that changed interval.
    #[cfg(feature = "slow-asserts")]
    fn check_changed_interval_closeness(&self) {
        let y = self.y;
        for iv in &self.changed_intervals {
            for j in iv.segs.clone() {
                let seg_j = self.line.seg(j);
                for i in 0..iv.segs.start {
                    let seg_i = self.line.seg(i);
                    let cmp = self.compare_segments(seg_i, seg_j);
                    if self.blocking_segment(i, j).is_none() {
                        assert_ne!(cmp.order_at(y), Order::Right);
                    }
                }

                for k in iv.segs.end..self.line.segs.len() {
                    let seg_k = self.line.seg(k);
                    let cmp = self.compare_segments(seg_j, seg_k);
                    if self.blocking_segment(j, k).is_none() {
                        assert_ne!(cmp.order_at(y), Order::Right);
                    }
                }
            }
        }
    }

    #[cfg(feature = "slow-asserts")]
    fn blocking_segment(&self, i: usize, j: usize) -> Option<usize> {
        let seg_i = self.line.seg(i);
        let seg_j = self.line.seg(j);
        self.line
            .segs
            .range((i + 1)..j)
            .position(|entry| {
                let mid_seg = entry.seg;
                self.compare_segments_conservatively(seg_i, mid_seg)
                    .order_at(self.y)
                    == Order::Left
                    || self
                        .compare_segments_conservatively(mid_seg, seg_j)
                        .order_at(self.y)
                        == Order::Left
            })
            .map(|idx| idx + i)
    }

    #[cfg(feature = "slow-asserts")]
    fn check_invariants(&self) {
        for seg_entry in self.line.segs.iter() {
            let seg_idx = seg_entry.seg;
            let seg = &self.segments[seg_idx];
            assert!(
                (&seg.start().y..=&seg.end().y).contains(&&self.y),
                "segment {seg:?} out of range at y={:?}",
                self.y
            );
        }

        // All segments marked as entering or exiting must be in `self.segs_needing_positions`
        for (idx, seg_entry) in self.line.segs.iter().enumerate() {
            if seg_entry.exit || seg_entry.enter {
                assert!(self.segs_needing_positions.contains(&idx));
            }
        }

        let invalid_order = self.line.find_invalid_order(
            self.y,
            self.segments,
            self.comparisons.borrow_mut().deref_mut(),
        );
        if let Some(order) = invalid_order {
            // There are two cases in which we allow invalid orders:
            // - if there's an intersection event queued exactly at this `y` value.
            // - if there's some "blocking" segment between the two segments
            //   with invalid orders.
            let has_matching_event = self
                .events
                .intersection
                .iter()
                .take_while(|ev| ev.y == self.y)
                .any(|ev| ev.left == order.0 && ev.right == order.1);

            let left_idx = self.line.position(order.0).unwrap();
            let right_idx = self.line.position(order.1).unwrap();
            let has_blocking_segment = self.blocking_segment(left_idx, right_idx).is_some();
            if !has_matching_event && !has_blocking_segment {
                dbg!(&self.line);
                panic!("invalid order {order:?}");
            }
        }

        for i in 0..self.line.segs.len() {
            if self.line.is_exit(i) {
                continue;
            }
            for j in (i + 1)..self.line.segs.len() {
                if self.line.is_exit(j) {
                    continue;
                }
                let segi = self.line.seg(i);
                let segj = self.line.seg(j);

                let touch = self.compare_segments(segi, segj).next_touch_after(self.y);
                if let Some(y_int) =
                    touch.and_then(|touch| choose_cross_height(touch, self.y, WhichTouch::Late))
                {
                    let is_cross = matches!(touch, Some(CurveInteraction::Cross(..)));
                    if y_int >= self.y && is_cross {
                        // Find an event between i and j.
                        let is_between = |idx: SegIdx| -> bool {
                            self.line
                                .position(idx)
                                .is_some_and(|pos| i <= pos && pos <= j)
                        };
                        let has_exit_witness = self
                            .line
                            .segs
                            .range(i..=j)
                            .any(|seg_entry| self.segments[seg_entry.seg].end().y <= y_int);

                        let has_intersection_witness = self.events.intersection.iter().any(|ev| {
                            let is_between = is_between(ev.left) && is_between(ev.right);
                            let before_y = ev.y <= y_int;
                            is_between && before_y
                        });
                        let has_witness = has_exit_witness || has_intersection_witness;
                        let has_blocking_segment = self.blocking_segment(i, j).is_some();
                        assert!(
                            has_witness || has_blocking_segment,
                            "segments {:?} and {:?} cross at {:?}, but there is no witness. y={}, intersections={:?}, line={:?}",
                            self.line.segs[i].seg,
                            self.line.segs[j].seg,
                            y_int,
                            self.y,
                            self.events.intersection,
                            &self.line,
                        );
                    }
                }
            }
        }

        self.check_changed_interval_closeness();
    }

    #[cfg(not(feature = "slow-asserts"))]
    fn check_invariants(&self) {}

    fn compute_horizontal_changed_intervals(&mut self) {
        self.horizontals
            .sort_by_key(|seg_idx| CheapOrderedFloat::from(self.segments[*seg_idx].start().x));
        let mut last_end = f64::NEG_INFINITY;

        for (idx, &seg_idx) in self.horizontals.iter().enumerate() {
            let seg = &self.segments[seg_idx];

            // Find the index of some segment that might overlap with this
            // horizontal segment, but the index before definitely doesn't.
            let start_idx = self.line.segs.partition_point(|other_entry| {
                let other_seg = &self.segments[other_entry.seg];

                other_seg.upper(self.y, self.eps) + self.eps <= seg.start().x
            });

            let mut end_idx = start_idx;
            for j in start_idx..self.line.segs.len() {
                let other_entry = &mut self.line.segs[j];
                let other_seg = &self.segments[other_entry.seg];

                if other_seg.lower(self.y, self.eps) - self.eps <= seg.end().x {
                    // Ensure that every segment in the changed interval has `old_idx` set;
                    // see also `compute_changed_intervals`.
                    self.line.segs[j].set_old_idx_if_unset(j);
                    end_idx = j + 1;
                } else {
                    break;
                }
            }

            // If this horizontal interval overlaps with the previous one, extend
            // the changed interval instead of pushing a new one.
            if seg.start().x <= last_end {
                // unwrap: since our segments are finite, we can only get in this
                // branch if we already passed through the main loop and added something
                // to changed_intervals
                let last = self.changed_intervals.last_mut().unwrap();
                // unwrap: all our changed intervals in this function have horizontals.
                last.horizontals.as_mut().unwrap().end = idx + 1;
                last.segs.end = last.segs.end.max(end_idx);
            } else {
                let changed = ChangedInterval {
                    segs: start_idx..end_idx,
                    horizontals: Some(idx..(idx + 1)),
                };
                self.changed_intervals.push(changed);
            }

            last_end = last_end.max(seg.end().x);
        }
    }

    /// When a segment enters or exits, we insert a horizontal line from
    /// its negotiated position to its original endpoint position. To ensure
    /// that the topology gets correctly computed for segments between these
    /// two horizontal positions, we need to make them all part of a changed
    /// interval.
    ///
    /// This method adds all the changed intervals that are needed to handle this case.
    fn compute_contour_adjacent_changed_intervals(&mut self) {
        for &j in &self.segs_needing_positions {
            let seg_entry = self.line.segs[j];

            if seg_entry.old_seg.is_some() {
                // We ignore contour continuations that share a slot. This
                // might not be quite correct right now, because in this case
                // we currently still insert horizontal segments to the original
                // endpoints. But those two horizontal segments "cancel out"
                // topologically (and we'd like to remove them eventually).
                continue;
            }
            if !(seg_entry.enter || seg_entry.exit) {
                // This only applies to entering or exiting segments.
                continue;
            }

            // TODO: this logic is pretty much copied from the horizontals segments
            // method.
            let seg = &self.segments[seg_entry.seg];
            let p = if seg_entry.enter {
                kurbo::Point::new(seg.start().x, seg.start().y)
            } else {
                kurbo::Point::new(seg.end().x, seg.end().y)
            };
            let mut start_idx = j;
            let mut end_idx = j;
            for i in (0..j).rev() {
                let other_entry = &mut self.line.segs[i];
                let other_seg = &self.segments[other_entry.seg];

                if other_seg.upper(self.y, self.eps) + self.eps >= p.x {
                    // Ensure that every segment in the changed interval has `old_idx` set;
                    // see also `compute_changed_intervals`.
                    self.line.segs[i].set_old_idx_if_unset(i);
                    start_idx = i;
                } else {
                    break;
                }
            }
            for k in (j + 1)..self.line.segs.len() {
                let other_entry = &mut self.line.segs[k];
                let other_seg = &self.segments[other_entry.seg];

                if other_seg.lower(self.y, self.eps) - self.eps <= p.x {
                    // Ensure that every segment in the changed interval has `old_idx` set;
                    // see also `compute_changed_intervals`.
                    self.line.segs[k].set_old_idx_if_unset(k);
                    end_idx = k;
                } else {
                    break;
                }
            }

            if end_idx > start_idx {
                self.changed_intervals.push(ChangedInterval {
                    segs: start_idx..(end_idx + 1),
                    horizontals: None,
                });
            }
        }
    }

    /// Updates our internal `changed_intervals` state based on the segments marked
    /// as needing positions.
    ///
    /// For each segment marked as needing a position, we expand it to a range of
    /// physically nearby segments, and then we deduplicate and merge those ranges.
    fn compute_changed_intervals(&mut self) {
        debug_assert!(self.changed_intervals.is_empty());
        self.compute_horizontal_changed_intervals();
        self.compute_contour_adjacent_changed_intervals();

        for &idx in &self.segs_needing_positions {
            if self.line.segs[idx].in_changed_interval {
                continue;
            }
            // Ensure that every segment in the changed interval has `old_idx` set. This
            // isn't strictly necessary (because segments without `old_idx` set haven't
            // changed their indices), but it's more convenient to just say that all
            // segments in a changed interval must have `old_idx` set.
            self.line.segs[idx].set_old_idx_if_unset(idx);

            self.line.segs[idx].in_changed_interval = true;
            let mut start_idx = idx;
            let mut end_idx = idx + 1;

            for i in (idx + 1)..self.line.segs.len() {
                // Note that end_idx is mutated in the loop, so this points to the segment
                // that we most recently added to the changed interval. Modulo some subtleties
                // involving conservative/non-conservative comparisons, this will usually be
                // `i - 1`.
                let seg_idx = self.line.seg(end_idx - 1);
                let next_seg_idx = self.line.seg(i);
                let strong_order = self
                    .compare_segments_conservatively(seg_idx, next_seg_idx)
                    .order_at(self.y);
                if strong_order == Order::Left {
                    break;
                } else {
                    // TODO: re-enable this (and below) once our strong
                    // comparisons are implemented properly with minkowski sums
                    //debug_assert_eq!(strong_cmp.order_at(self.y), Order::Ish);
                    let order = self
                        .compare_segments(seg_idx, next_seg_idx)
                        .order_at(self.y);
                    if order != Order::Left {
                        for j in end_idx..=i {
                            self.line.segs[j].in_changed_interval = true;
                            self.line.segs[j].set_old_idx_if_unset(j);
                        }

                        end_idx = i + 1;
                    }
                }
            }

            for i in (0..start_idx).rev() {
                let prev_seg_idx = self.line.seg(i);
                // Note that start_idx is mutated in the loop, so this points to the segment
                // that we most recently added to the changed interval.
                let seg_idx = self.line.seg(start_idx);
                let strong_order = self
                    .compare_segments_conservatively(prev_seg_idx, seg_idx)
                    .order_at(self.y);
                if strong_order == Order::Left {
                    break;
                } else {
                    //debug_assert_eq!(strong_cmp.order_at(self.y), Order::Ish);
                    let order = self
                        .compare_segments(prev_seg_idx, seg_idx)
                        .order_at(self.y);
                    if order != Order::Left {
                        for j in i..start_idx {
                            self.line.segs[j].in_changed_interval = true;
                            self.line.segs[j].set_old_idx_if_unset(j);
                        }
                        start_idx = i;
                    }
                }
            }
            self.changed_intervals
                .push(ChangedInterval::from_seg_interval(start_idx..end_idx));
        }
        self.changed_intervals.sort_by_key(|r| r.segs.start);

        // By merging adjacent intervals, we ensure that there is no horizontal segment
        // that spans two ranges. That's because horizontal segments mark everything they
        // cross as needing a position. Any collection of subranges that are crossed by
        // a horizontal segment are therefore adjacent and will be merged here.
        merge_adjacent(&mut self.changed_intervals);
    }
}

/// Represents a sub-interval of the sweep-line where some subdivisions need to happen.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Debug, Clone)]
pub struct ChangedInterval {
    // Indices into the sweep-line's segment array.
    pub(crate) segs: std::ops::Range<usize>,
    // Indices into the array of horizontal segments.
    pub(crate) horizontals: Option<std::ops::Range<usize>>,
}

impl ChangedInterval {
    fn merge(&mut self, other: &ChangedInterval) {
        self.segs.end = self.segs.end.max(other.segs.end);
        self.segs.start = self.segs.start.min(other.segs.start);

        self.horizontals = match (self.horizontals.clone(), other.horizontals.clone()) {
            (None, None) => None,
            (None, Some(r)) | (Some(r), None) => Some(r),
            (Some(a), Some(b)) => Some(a.start.min(b.start)..a.end.max(b.end)),
        };
    }

    fn from_seg_interval(segs: std::ops::Range<usize>) -> Self {
        Self {
            segs,
            horizontals: None,
        }
    }
}

impl SegmentOrder {
    // If the ordering invariants fail, returns a pair of indices witnessing that failure.
    // Used in tests, and when enabling slow-asserts
    #[allow(dead_code)]
    fn find_invalid_order(
        &self,
        y: f64,
        segments: &Segments,
        cmp: &mut ComparisonCache,
    ) -> Option<(SegIdx, SegIdx)> {
        for i in 0..self.segs.len() {
            let segi = self.seg(i);
            // We ignore segments ending at the current y, because our intersection
            // scans also ignore such segments.
            //
            // TODO: I think this is correct enough for continuous contours
            // (because the adjacent starting contour will get a validly-ordered
            // position and then we'll insert a horizontal segment joining
            // them), but if the input is just a collection of segments then it
            // can mess up the endpoints.
            if segments[segi].end().y == y {
                continue;
            }
            for j in (i + 1)..self.segs.len() {
                let segj = self.seg(j);
                if segments[segj].end().y == y {
                    continue;
                }

                let order = cmp.compare_segments(segments, segi, segj);
                if order.order_at(y) == Order::Right {
                    return Some((segi, segj));
                }
            }
        }

        None
    }

    // Finds an index into this sweep line where it's ok to insert this new segment.
    fn insertion_idx(
        &self,
        y: f64,
        segments: &Segments,
        seg_idx: SegIdx,
        comparer: &mut ComparisonCache,
    ) -> usize {
        let seg = &segments[seg_idx];
        let seg_x = seg.start().x;

        // Fast path: we first do a binary search just with the horizontal bounding intervals.
        // `pos` is an index where we're to the left of that segment's right-most point, and
        // we're to the right of the segment at `pos - 1`'s right-most point.
        // (It isn't necessarily the first such index because things aren't quite sorted; see
        // below about the binary search.)
        let pos = self
            .segs
            .partition_point(|entry| seg_x >= entry.upper_bound);

        if pos >= self.segs.len() {
            return pos;
        } else if pos + 1 >= self.segs.len() || self.segs[pos + 1].lower_bound >= seg_x {
            // At this point, the segment before pos is definitely to our left, and the segment
            // after pos is definitely to our right (or there is no such segment). That means
            // we can insert either before or after `pos`. We'll choose based on what the order
            // will be in the future, so as to minimize swapping.
            // (We could also try checking whether the new segment is a contour neighbor of
            // the segment at pos. That should be a pretty common case.)
            let cmp = comparer.compare_segments(segments, seg_idx, self.segs[pos].seg);
            return match cmp.order_after(y) {
                Order::Right => pos + 1,
                Order::Ish => pos,
                Order::Left => pos,
            };
        }

        // A predicate that tests whether `other` is definitely to the left of `seg` at `y`.
        let lower_pred = |other: &SegmentOrderEntry| -> bool {
            let cmp = comparer.compare_segments(segments, other.seg, seg_idx);
            cmp.order_at(y) == Order::Left
        };

        // The rust stdlib docs say that we're not allowed to do this, because
        // our array isn't sorted with respect to `lower_pred`.
        // But for now at least, their implementation does a normal
        // binary search and so it's guaranteed to return an index where
        // `lower_pred` fails but the index before it succeeds.
        //
        // `search_start` is `i_- + 1` in the write-up; it's the first index
        // where the predicate returns false.
        let search_start = self.segs.partition_point(lower_pred);
        let mut idx = search_start;
        for i in search_start..self.segs.len() {
            let cmp = comparer.compare_segments(segments, self.segs[i].seg, seg_idx);
            match cmp.order_at(y) {
                Order::Right => {
                    // The segment at i is definitely to the right of the new one.
                    break;
                }
                Order::Ish => {
                    // We could go either way, and we'll update our preference based on the
                    // future ordering.
                    if cmp.order_after(y) == Order::Left {
                        idx = i + 1;
                    }
                }
                Order::Left => {
                    // The segment at i is definitely to the left of the new one.
                    idx = i + 1;
                }
            }
        }
        idx
    }

    // Find the position of the given segment in our array.
    //
    // Returns an index pointing to a `SegmentOrderEntry` for which
    // `seg_idx` is either the `seg` or the `old_seg`.
    fn position(&self, seg_idx: SegIdx) -> Option<usize> {
        self.segs
            .iter()
            .position(|x| x.seg == seg_idx || x.old_seg == Some(seg_idx))
    }
}

/// Finds a feasible `x` position for each segment.
///
/// The point is that we've decided on a horizontal ordering of the segments, but
/// their numerical positions might not completely agree with that ordering. For
/// each segment, this function computes a range of `x` coordinates with the
/// guarantee that if you go from left to right (or right to left) and assign
/// each segment an `x` coordinate within its range then you won't paint
/// yourself into a corner: subsequent points can be positioned with the right
/// ordering *and* within the designated range.
fn feasible_horizontal_positions<G: Fn(&SegmentOrderEntry) -> SegIdx>(
    entries: &[SegmentOrderEntry],
    entry_seg: G,
    y: f64,
    segments: &Segments,
    eps: f64,
    out: &mut Vec<(SegmentOrderEntry, f64, f64)>,
) {
    out.clear();
    let mut max_so_far = f64::NEG_INFINITY;
    let mut max = f64::NEG_INFINITY;
    let mut min = f64::INFINITY;

    for entry in entries {
        let seg = &segments[entry_seg(entry)];
        max_so_far = max_so_far.max(seg.lower(y, eps));
        // Fill out the minimum allowed positions, with a placeholder for the maximum.
        out.push((*entry, max_so_far, 0.0));

        let x = seg.at_y(y);
        max = max.max(x);
        min = min.min(x);
    }

    let mut min_so_far = f64::INFINITY;

    for (entry, min_allowed, max_allowed) in out.iter_mut().rev() {
        let x = segments[entry_seg(entry)].upper(y, eps);
        min_so_far = min_so_far.min(x);
        *max_allowed = min_so_far.min(max);
        *min_allowed = min_allowed.max(min);
    }
}

/// A sweep-line, as output by a [`Sweeper`].
///
/// This contains all the information about how the input line segments interact
/// with a sweep-line, including which segments start here, which segments end here,
/// and which segments intersect here.
///
/// A sweep-line stores a `y` coordinate along with two (potentially) different
/// orderings of the segments: the ordering just above `y` and the ordering just
/// below `y`. If two line segments intersect at the sweep-line, their orderings
/// above `y` and below `y` will be different.
#[derive(Debug)]
pub struct SweepLine<'buf, 'state, 'segs> {
    pub(crate) state: &'state Sweeper<'segs>,
    bufs: &'buf mut SweepLineBuffers,
    // Index into state.changed_intervals
    next_changed_interval: usize,
}

impl<'segs> SweepLine<'_, '_, 'segs> {
    /// The vertical position of this sweep-line.
    pub fn y(&self) -> f64 {
        self.state.y
    }

    /// Our tolerance parameter.
    pub fn eps(&self) -> f64 {
        self.state.eps
    }

    /// The collection of segments that we're sweeping over.
    pub fn segments(&self) -> &Segments {
        self.state.segments
    }

    /// Get the line segment at position `idx` in the new order.
    pub fn line_segment(&self, idx: usize) -> Option<SegIdx> {
        self.state.line.segs.get(idx).map(|entry| entry.seg)
    }

    /// Get the line segment at position `idx` in the new order.
    pub(crate) fn line_entry(&self, idx: usize) -> Option<&SegmentOrderEntry> {
        self.state.line.segs.get(idx)
    }

    /// Returns an iterator over the segments in `range`, according to the "old"
    /// sweep-line order.
    ///
    /// `range` must be a full changed interval, for example the one returned by
    /// `cur_interval`.
    pub(crate) fn old_segment_range(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = SegIdx> + '_ {
        let mut ret = vec![None; range.end - range.start];
        for (idx, entry) in self.state.line.segs.range(range.clone()).enumerate() {
            let seg = if entry.enter {
                match entry.old_seg {
                    Some(s) => s,
                    None => continue,
                }
            } else {
                entry.seg
            };

            let idx = entry.old_idx.unwrap_or(idx);
            ret[idx - range.start] = Some(seg);
        }

        ret.into_iter().flatten()
    }

    /// Returns an iterator over the segments in `range`, according to the "new"
    /// sweep-line order.
    pub fn segment_range(
        &self,
        range: std::ops::Range<usize>,
    ) -> impl Iterator<Item = SegIdx> + '_ {
        self.state.line.segs.range(range).filter_map(|entry| {
            if entry.exit && entry.old_seg.is_none() {
                None
            } else {
                Some(entry.seg)
            }
        })
    }

    /// Returns the index ranges of segments in this sweep-line that need to be
    /// given explicit positions.
    ///
    /// Not every line segment that passes through a sweep-line needs to be
    /// subdivided at that sweep-line; in order to have a fast sweep-line
    /// implementation, we need to be able to ignore the segments that don't
    /// need subdivision.
    ///
    /// This method returns a range of segments that need to be subdivided at
    /// the current sweep-line. If you call [`SweepLine::next_range`] and then
    /// call this again, it will give you a non-overlapping range.
    pub fn cur_interval(&self) -> Option<&ChangedInterval> {
        self.next_changed_interval
            .checked_sub(1)
            .and_then(|idx| self.state.changed_intervals.get(idx))
    }

    fn next_range_single_seg<'a, 'bufs>(
        &'a mut self,
        bufs: &'bufs mut SweepLineRangeBuffers,
        segments: &Segments,
        idx: usize,
    ) -> Option<SweepLineRange<'bufs, 'a, 'segs>> {
        bufs.clear();
        self.bufs.output_events.clear();

        let entry = &self.state.line.segs[idx];
        let x = segments[entry.seg].at_y(self.y());
        if let Some(old_seg) = entry.old_seg {
            // This entry is on a contour, where one segment ends and the next begins.
            // We output two events (one per segment) at the same position.
            self.bufs.output_events.push(OutputEvent {
                x0: x,
                connected_above: true,
                x1: x,
                connected_below: false,
                seg_idx: old_seg,
                sweep_idx: None,
                old_sweep_idx: entry.old_idx,
            });

            self.bufs.output_events.push(OutputEvent {
                x0: x,
                connected_above: false,
                x1: x,
                connected_below: true,
                seg_idx: entry.seg,
                sweep_idx: Some(idx),
                old_sweep_idx: None,
            });
        } else {
            // It's a single segment either entering or exiting at this height.
            // We can handle them both in a single case.
            self.bufs.output_events.push(OutputEvent {
                x0: x,
                connected_above: !entry.enter,
                x1: x,
                connected_below: !entry.exit,
                seg_idx: entry.seg,
                sweep_idx: Some(idx),
                old_sweep_idx: entry.old_idx,
            });
        }

        let changed_interval = ChangedInterval {
            segs: idx..(idx + 1),
            horizontals: None,
        };

        Some(SweepLineRange::new(
            self,
            &self.bufs.output_events,
            bufs,
            changed_interval,
        ))
    }

    /// Returns a [`SweepLineRange`] for visiting and processing all positions within
    /// a range of segments.
    pub fn next_range<'a, 'bufs>(
        &'a mut self,
        bufs: &'bufs mut SweepLineRangeBuffers,
        segments: &Segments,
    ) -> Option<SweepLineRange<'bufs, 'a, 'segs>> {
        let range = self
            .state
            .changed_intervals
            .get(self.next_changed_interval)?
            .clone();
        self.next_changed_interval += 1;

        if range.segs.len() == 1 && range.horizontals.is_none() {
            return self.next_range_single_seg(bufs, segments, range.segs.start);
        }

        let dummy_entry = SegmentOrderEntry {
            seg: SegIdx(424242),
            exit: false,
            enter: false,
            lower_bound: 0.0,
            upper_bound: 0.0,
            in_changed_interval: false,
            old_idx: None,
            old_seg: None,
        };

        let buffers = &mut self.bufs;
        buffers
            .old_line
            .resize(range.segs.end - range.segs.start, dummy_entry);
        for entry in self.state.line.segs.range(range.segs.clone()) {
            buffers.old_line[entry.old_idx.unwrap() - range.segs.start] = *entry;
        }

        // Assign horizontal positions to all the points in the old sweep line.
        // First, compute the feasible positions.
        feasible_horizontal_positions(
            &buffers.old_line,
            |entry| entry.old_seg(),
            self.state.y,
            segments,
            self.state.eps,
            &mut buffers.positions,
        );

        // Now go through and assign the actual positions.
        //
        // The two positioning arrays should have the same segments, but possibly in a different
        // order. We build them up in the old-sweep-line order.
        buffers.output_events.clear();
        let events = &mut buffers.output_events;
        let mut max_so_far = f64::NEG_INFINITY;
        for (entry, min_x, max_x) in &buffers.positions {
            let preferred_x = if entry.exit {
                // The best possible position is the true segment-ending position.
                // (This could change if we want to be more sophisticated at joining contours.)
                segments[entry.old_seg()].end().x
            } else if entry.enter {
                // The best possible position is the true segment-starting position.
                // (This could change if we want to be more sophisticated at joining contours.)
                segments[entry.seg].start().x
            } else {
                segments[entry.seg].at_y(self.state.y)
            };
            let x = preferred_x.max(*min_x).max(max_so_far).min(*max_x);
            max_so_far = x;
            events.push(OutputEvent {
                x0: x,
                connected_above: entry.old_seg.is_some() || !entry.enter,
                // This will be filled out when we traverse new_xs.
                x1: 42.42,
                connected_below: !entry.exit,
                seg_idx: entry.old_seg(),
                sweep_idx: None,
                old_sweep_idx: entry.old_idx,
            });
        }

        // And now we repeat for the new sweep line: compute the feasible positions
        // and then choose the actual positions.
        buffers.old_line.clear();
        buffers
            .old_line
            .extend(self.state.line.segs.range(range.segs.clone()).cloned());
        feasible_horizontal_positions(
            &buffers.old_line,
            |entry| entry.seg,
            self.state.y,
            segments,
            self.state.eps,
            &mut buffers.positions,
        );
        let mut max_so_far = f64::NEG_INFINITY;
        for (idx, (entry, min_x, max_x)) in buffers.positions.iter().enumerate() {
            let ev = &mut events[entry.old_idx.unwrap() - range.segs.start];
            ev.sweep_idx = Some(range.segs.start + idx);
            debug_assert_eq!(ev.seg_idx, entry.old_seg());
            let preferred_x = if *min_x <= ev.x0 && ev.x0 <= *max_x {
                // Try snapping to the previous position if possible.
                ev.x0
            } else {
                segments[entry.seg].at_y(self.state.y)
            };
            ev.x1 = preferred_x.max(*min_x).max(max_so_far).min(*max_x);
            max_so_far = ev.x1;

            let x1 = ev.x1;
            if entry.old_seg.is_some() {
                events.push(OutputEvent {
                    x0: x1,
                    x1,
                    connected_above: false,
                    // This will be filled out when we traverse new_xs.
                    connected_below: true,
                    seg_idx: entry.seg,
                    sweep_idx: Some(range.segs.start + idx),
                    old_sweep_idx: None,
                });
            }
        }

        // Modify the positions so that entering and exiting segments get their exact position.
        //
        // This is the easiest way to maintain the continuity of contours, but
        // eventually we should change this to minimize horizontal jank.
        for ev in &mut *events {
            let seg = &segments[ev.seg_idx];
            if !ev.connected_above {
                ev.x0 = seg.start().x;
            }
            if !ev.connected_below {
                ev.x1 = seg.end().x;
            }
        }

        if let Some(range) = &range.horizontals {
            for &seg_idx in &self.state.horizontals[range.clone()] {
                let seg = &self.state.segments[seg_idx];
                events.push(OutputEvent {
                    x0: seg.start().x,
                    connected_above: false,
                    x1: seg.end().x,
                    connected_below: false,
                    seg_idx,
                    sweep_idx: None,
                    old_sweep_idx: None,
                });
            }
        }
        events.sort();
        bufs.clear();

        Some(SweepLineRange::new(
            self,
            &self.bufs.output_events,
            bufs,
            range,
        ))
    }
}

/// Given a list of intervals, sorted by starting point, merge the overlapping ones.
///
/// For example, [1..2, 4..5, 5..6] is turned into [1..2, 4..6].
fn merge_adjacent(intervals: &mut Vec<ChangedInterval>) {
    if intervals.is_empty() {
        return;
    }

    let mut write_idx = 0;
    for read_idx in 1..intervals.len() {
        let last_end = intervals[write_idx].segs.end;
        let cur = intervals[read_idx].clone();
        if last_end < cur.segs.start {
            write_idx += 1;
            intervals[write_idx] = cur;
        } else {
            intervals[write_idx].merge(&cur);
        }
    }
    intervals.truncate(write_idx + 1);
}

/// Choose a valid crossing height given a range of possibilities and
/// the current height of the sweep-line.
fn choose_cross_height(
    interaction: CurveInteraction,
    cur_y: f64,
    which: WhichTouch,
) -> Option<f64> {
    // TODO: This "mid" will have the effect of emitting an intersection event
    // halfway (in y) between the two definite orders. This doesn't necessarily
    // match the guarantees that we require: for the "stronger" crossing
    // invariants mentioned in the write-up, when scanning left from index j
    // and comparing to index i, we need to emit the intersection event before
    // i's lower bound crosses j.
    //
    // We could just emit y0. That would be safe, but give less pretty results
    // in practice.
    let (mid, end) = match interaction {
        CurveInteraction::Cross(y0, y1) | CurveInteraction::Touch(y0, y1) => ((y0 + y1) / 2.0, y1),
    };
    debug_assert!(end >= cur_y);
    match (interaction, which, mid <= cur_y) {
        (CurveInteraction::Cross(..), WhichTouch::Late, true) => Some(end),
        (CurveInteraction::Touch(..), _, true) => None,
        _ => Some(mid.max(cur_y)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use kurbo::{BezPath, CubicBez};
    use proptest::prelude::*;

    use crate::{
        geom::Point,
        perturbation::{
            F64Perturbation, Perturbation, PointPerturbation, f64_perturbation, perturbation,
            realize_perturbation,
        },
        segments::Segments,
    };

    fn mk_segs(xs: &[(f64, f64)]) -> Segments {
        let mut segs = Segments::default();

        for &(x0, x1) in xs {
            segs.add_points([Point::new(x0, 0.0), Point::new(x1, 1.0)]);
        }
        segs
    }

    #[test]
    fn merge_adjacent() {
        fn merge(
            v: impl IntoIterator<Item = std::ops::Range<usize>>,
        ) -> Vec<std::ops::Range<usize>> {
            let mut v = v
                .into_iter()
                .map(ChangedInterval::from_seg_interval)
                .collect();
            super::merge_adjacent(&mut v);
            v.into_iter().map(|ci| ci.segs).collect()
        }

        assert_eq!(merge([1..2, 3..4]), vec![1..2, 3..4]);
        assert_eq!(merge([1..2, 2..4]), vec![1..4]);
        assert_eq!(merge([1..2, 3..4, 4..5]), vec![1..2, 3..5]);
        assert_eq!(merge([1..2, 1..3, 4..5]), vec![1..3, 4..5]);
        assert_eq!(merge([1..2, 1..4, 4..5]), vec![1..5]);
        assert_eq!(merge([1..4, 1..2, 4..5]), vec![1..5]);
        assert_eq!(merge([]), Vec::<std::ops::Range<_>>::new());
    }

    #[test]
    fn invalid_order() {
        fn check_order(xs: &[(f64, f64)], at: f64, eps: f64) -> Option<(usize, usize)> {
            let y = at;
            let segs = mk_segs(xs);

            let line: SegmentOrder = SegmentOrder {
                segs: (0..xs.len())
                    .map(|i| SegmentOrderEntry::new(SegIdx(i), &segs, eps))
                    .collect(),
            };

            let mut cmp = ComparisonCache::new(eps, eps / 2.0);
            line.find_invalid_order(y, &segs, &mut cmp)
                .map(|(a, b)| (a.0, b.0))
        }

        let crossing = &[(-1.0, 1.0), (1.0, -1.0)];
        let eps = 1.0 / 128.0;
        assert!(check_order(crossing, 0.0, eps).is_none());
        assert!(check_order(crossing, 0.5, eps).is_none());
        assert_eq!(check_order(crossing, 0.99, eps), Some((0, 1)));

        let not_quite_crossing = &[(-0.5 * eps, 0.5 * eps), (0.5 * eps, -0.5 * eps)];
        assert!(check_order(not_quite_crossing, 0.0, eps).is_none());
        assert!(check_order(not_quite_crossing, 0.5, eps).is_none());
        assert!(check_order(not_quite_crossing, 1.0, eps).is_none());

        let barely_crossing = &[(-1.5 * eps, 1.5 * eps), (1.5 * eps, -1.5 * eps)];
        assert!(check_order(barely_crossing, 0.0, eps).is_none());
        assert!(check_order(barely_crossing, 0.5, eps).is_none());
        assert_eq!(check_order(barely_crossing, 0.99, eps), Some((0, 1)));

        let non_adj_crossing = &[(-eps, eps), (0.0, 0.0), (eps, -eps)];
        assert!(check_order(non_adj_crossing, 0.0, eps).is_none());
        assert!(check_order(non_adj_crossing, 0.5, eps).is_none());
        assert_eq!(check_order(non_adj_crossing, 0.99, eps), Some((0, 2)));

        let flat_crossing = &[(-1e6, 1e6), (-10.0 * eps, -10.0 * eps)];
        assert_eq!(check_order(flat_crossing, 0.5 - eps, eps), None);

        let end_crossing_bevel = &[(2.5 * eps, 2.5 * eps), (-1e6, 0.0)];
        assert_eq!(check_order(end_crossing_bevel, 0.99, eps), Some((0, 1)));

        let start_crossing_bevel = &[(2.5 * eps, 2.5 * eps), (0.0, -1e6)];
        assert_eq!(check_order(start_crossing_bevel, 0.99, eps), Some((0, 1)));
    }

    #[test]
    fn insertion_idx() {
        fn insert(xs: &[(f64, f64)], new: (f64, f64), at: f64, eps: f64) -> usize {
            let y = at;
            let mut xs: Vec<_> = xs.to_owned();
            xs.push(new);
            let segs = mk_segs(&xs);

            let new_idx = SegIdx(xs.len() - 1);

            let mut line: SegmentOrder = SegmentOrder {
                segs: (0..(xs.len() - 1))
                    .map(|i| SegmentOrderEntry::new(SegIdx(i), &segs, eps))
                    .collect(),
            };
            let mut comparer = ComparisonCache::new(eps, eps / 2.0);
            let idx = line.insertion_idx(y, &segs, new_idx, &mut comparer);

            dbg!(&line);
            assert!(dbg!(line.find_invalid_order(y, &segs, &mut comparer)).is_none());
            line.segs.insert(
                idx,
                SegmentOrderEntry::new(SegIdx(xs.len() - 1), &segs, eps),
            );
            assert!(line.find_invalid_order(y, &segs, &mut comparer,).is_none());
            idx
        }

        let eps = 1.0 / 128.0;
        assert_eq!(
            insert(&[(-1.0, -1.0), (1.0, 1.0)], (-2.0, 0.0), 0.0, eps),
            0
        );
        assert_eq!(insert(&[(-1.0, -1.0), (1.0, 1.0)], (0.0, 0.0), 0.0, eps), 1);
        assert_eq!(insert(&[(-1.0, -1.0), (1.0, 1.0)], (2.0, 0.0), 0.0, eps), 2);

        assert_eq!(
            insert(
                &[(-1e6, 1e6), (-1.0, -1.0), (1.0, 1.0)],
                (0.0, 0.0),
                0.5 - 1e-6,
                eps
            ),
            2
        );
        // This test doesn't really work any more, now that we've tightened
        // up our invalid order test.
        // assert_eq!(
        //     insert(
        //         &[
        //             (-1e6, 1e6),
        //             (-1e6, 1e6),
        //             (-1e6, 1e6),
        //             (-1.0, -1.0),
        //             (1.0, 1.0),
        //             (-1e6, 1e6),
        //             (-1e6, 1e6),
        //             (-1e6, 1e6),
        //         ],
        //         (0.0, 0.0),
        //         dbg!(0.5 - 1e-6 / 2.0),
        //         eps
        //     ),
        //     4
        // );

        insert(&[(2.0, 2.0), (-100.0, 100.0)], (1.0, 1.0), 0.51, 0.25);
    }

    #[test]
    fn test_sweep() {
        let eps = 0.01;

        let segs = mk_segs(&[(0.0, 0.0), (1.0, 1.0), (-2.0, 2.0)]);
        dbg!(&segs);
        crate::sweep::sweep(&segs, eps, |_, ev| {
            dbg!(ev);
        });
    }

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    fn run_perturbation(ps: Vec<Perturbation<F64Perturbation>>) {
        let base = vec![vec![
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(1.0, -1.0),
            p(2.0, 0.0),
            p(1.0, 1.0),
            p(1.0, -1.0),
        ]];

        let perturbed_polylines = ps
            .iter()
            .map(|p| realize_perturbation(&base, p))
            .collect::<Vec<_>>();
        let mut segs = Segments::default();
        for poly in perturbed_polylines {
            segs.add_closed_polyline(poly);
        }
        let eps = 0.1;
        crate::sweep::sweep(&segs, eps, |_, _| {});
    }

    #[derive(serde::Serialize, Debug)]
    struct Output {
        y: f64,
        order: SegmentOrder,
        changed: Vec<ChangedInterval>,
    }

    /// Runs a sweep-line on the given segments, collecting the sweep line orders
    /// at every interesting y.
    fn snapshot_outputs(segs: Segments, eps: f64) -> Vec<Output> {
        let mut outputs = Vec::new();
        let mut sweeper = Sweeper::new(&segs, eps);
        let mut line_bufs = SweepLineBuffers::default();
        while let Some(line) = sweeper.next_line(&mut line_bufs) {
            outputs.push(Output {
                y: line.y(),
                order: line.state.line.clone(),
                changed: line.state.changed_intervals.clone(),
            });
        }
        outputs
    }

    #[test]
    fn square() {
        let segs =
            Segments::from_closed_polyline([p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]);
        insta::assert_ron_snapshot!(snapshot_outputs(segs, 0.01));
    }

    #[test]
    fn square_and_diamond() {
        let square = [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let diamond = [p(0.0, 0.0), p(1.0, 1.0), p(0.0, 2.0), p(-1.0, 1.0)];
        let eps = 0.01;

        let mut segs = Segments::default();
        segs.add_closed_polylines([square, diamond]);
        insta::assert_ron_snapshot!(snapshot_outputs(segs, eps));
    }

    #[test]
    fn regression_position_state() {
        let ps = [Perturbation::Point {
            perturbation: PointPerturbation {
                x: F64Perturbation::Ulp(0),
                y: F64Perturbation::Ulp(-1),
            },
            idx: 14924312967467343829,
            next: Box::new(Perturbation::Base { idx: 0 }),
        }];
        let base = vec![vec![
            p(0.0, 0.0),
            p(1.0, 1.0),
            p(1.0, -1.0),
            p(2.0, 0.0),
            p(1.0, 1.0),
            p(1.0, -1.0),
        ]];
        let perturbed_polylines = ps
            .iter()
            .map(|p| realize_perturbation(&base, p))
            .collect::<Vec<_>>();
        let mut segs = Segments::default();
        for poly in perturbed_polylines {
            segs.add_closed_polyline(poly);
        }
        insta::assert_ron_snapshot!(snapshot_outputs(segs, 0.1));
    }

    #[test]
    fn two_squares() {
        let a = vec![p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)];
        let b = vec![p(-0.5, -0.5), p(0.5, -0.5), p(0.5, 0.5), p(-0.5, 0.5)];
        let mut segs = Segments::default();
        segs.add_closed_polyline(a);
        segs.add_closed_polyline(b);
        insta::assert_ron_snapshot!(snapshot_outputs(segs, 0.1));
    }

    #[test]
    fn conservative_line_order() {
        // Here's an example where c0 gets "ish" with c1 in the middle, c0 gets "ish" with c2
        // in the middle, but c1 is always definitely left of c2.
        let c0 = CubicBez::new((-1.0, 0.0), (0.6, 0.6), (0.6, 0.4), (-1.0, 1.0));
        let c1 = CubicBez::new((0.0, 0.0), (0.0, 1.0 / 3.0), (0.0, 2.0 / 3.0), (0.0, 1.0));
        let c2 = CubicBez::new((0.3, 0.0), (0.3, 1.0 / 3.0), (0.3, 2.0 / 3.0), (0.3, 1.0));
        dbg!(curve::intersect_cubics(c0, c1, 0.125, 0.25));
        dbg!(curve::intersect_cubics(c0, c2, 0.125, 0.25));
        dbg!(curve::intersect_cubics(c1, c2, 0.125, 0.25));

        let to_path = |c: CubicBez| {
            let mut p = BezPath::new();
            p.move_to(c.p0);
            p.curve_to(c.p1, c.p2, c.p3);
            p
        };

        let mut segs = Segments::default();
        segs.add_non_closed_bez_path(
            &[to_path(c0), to_path(c1), to_path(c2)]
                .into_iter()
                .flatten()
                .collect(),
        )
        .unwrap();
        insta::assert_ron_snapshot!(snapshot_outputs(segs, 0.25));
    }

    #[test]
    fn empty_sweep_line() {
        let segs = Segments::default();
        let mut sweep = Sweeper::new(&segs, 1.0);
        let mut bufs = SweepLineBuffers::default();
        assert!(sweep.next_line(&mut bufs).is_none());
    }

    proptest! {
    #[test]
    fn perturbation_test_f64(perturbations in prop::collection::vec(perturbation(f64_perturbation(0.1)), 1..5)) {
        run_perturbation(perturbations);
    }

    }
}
