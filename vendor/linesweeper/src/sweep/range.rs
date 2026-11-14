use crate::{SegIdx, num::CheapOrderedFloat};

use super::{ChangedInterval, OutputEvent, SweepLine};

/// A horizontal fragment.
///
/// Represents a horizontal part of a segment, which could be either an actual
/// horizontal segment or a little horizontal connector of a segment in a
/// sweep-line.
#[derive(Clone, Debug, PartialEq)]
struct HFrag {
    /// The segment this horizontal fragment is a part of.
    pub seg: SegIdx,
    /// The first (smallest) horizontal position of this fragment.
    pub start: f64,
    /// The last (largest) horizontal position of this fragment.
    pub end: f64,
    /// Does this segment continue out of the sweep-line at `start`?
    ///
    /// For a horizontal segment, this will always be false. On its own,
    /// this doesn't tell you whether the segment points up or down at
    /// `start`; for that, see `enter_first`.
    pub connected_at_start: bool,
    /// Does this segment continue out of the sweep-line at `end`?
    ///
    /// For a horizontal segment, this will always be false.
    pub connected_at_end: bool,
    /// When traversing the segment in sweep-line order, does it visit
    /// `start` first?
    ///
    /// For example:
    ///
    /// ```text
    /// s_1        s_2   s_3
    ///  ╲          ╱     ╲
    ///   ╲        ╱       ╲
    ///   ─      ─           ─
    ///   ╲    ╱               ╲
    ///    ╲  ╱                 ╲
    /// ```
    ///
    /// When moving from top to bottom (i.e. sweep-line order), s_1 and s_2
    /// visit the larger horizontal position of the fragment before the smaller
    /// one, so `enter_first` is false. On the other hand, s_3 has `enter_first`
    /// as true.
    pub enter_first: bool,
    /// The position of the segment in the current sweep-line.
    ///
    /// This will be `None` if, and only if, the segment is horizontal.
    pub sweep_idx: Option<usize>,
    /// The position of the segment in the old sweep-line.
    ///
    /// This will be `None` if, and only if, the segment is horizontal.
    pub old_sweep_idx: Option<usize>,
}

impl HFrag {
    /// Given a segment's interaction with the sweep-line, returns the corresponding
    /// horizontal fragment if there is one.
    pub fn from_position(pos: OutputEvent) -> Option<Self> {
        let OutputEvent {
            x0,
            connected_above,
            x1,
            connected_below,
            ..
        } = pos;

        if x0 == x1 {
            return None;
        }

        let enter_first = x0 < x1;
        let (start, end, connected_at_start, connected_at_end) = if enter_first {
            (x0, x1, connected_above, connected_below)
        } else {
            (x1, x0, connected_below, connected_above)
        };
        Some(HFrag {
            end,
            start,
            enter_first,
            seg: pos.seg_idx,
            connected_at_start,
            connected_at_end,
            sweep_idx: pos.sweep_idx,
            old_sweep_idx: pos.old_sweep_idx,
        })
    }

    /// Does this horizontal fragment's segment stick up from the sweep-line at `x`?
    pub fn connected_above_at(&self, x: f64) -> bool {
        (x == self.start && self.enter_first && self.connected_at_start)
            || (x == self.end && !self.enter_first && self.connected_at_end)
    }

    /// Does this horizontal fragment's segment stick down from the sweep-line at `x`?
    pub fn connected_below_at(&self, x: f64) -> bool {
        (x == self.start && !self.enter_first && self.connected_at_start)
            || (x == self.end && self.enter_first && self.connected_at_end)
    }
}

/// Emits output events for a single sub-range of a single sweep-line.
///
/// This is constructed using [`SweepLine::next_range`]. By repeatedly
/// calling `SweepLineRange::increase_x` you can iterate over all
/// interesting horizontal positions, left to right (i.e. smaller `x` to larger
/// `x`).
#[derive(Debug)]
pub struct SweepLineRange<'bufs, 'state, 'segs> {
    last_x: Option<f64>,
    line: &'state SweepLine<'state, 'state, 'segs>,
    bufs: &'bufs mut SweepLineRangeBuffers,
    changed_interval: ChangedInterval,
    output_events: &'state [OutputEvent],
}

impl<'bufs, 'state, 'segs> SweepLineRange<'bufs, 'state, 'segs> {
    pub(crate) fn new(
        line: &'state SweepLine<'state, 'state, 'segs>,
        output_events: &'state [OutputEvent],
        bufs: &'bufs mut SweepLineRangeBuffers,
        changed_interval: ChangedInterval,
    ) -> Self {
        let ret = Self {
            last_x: None,
            output_events,
            changed_interval,
            line,
            bufs,
        };

        #[cfg(feature = "slow-asserts")]
        ret.check_invariants();

        ret
    }

    fn output_events(&self) -> &[OutputEvent] {
        self.output_events
    }

    /// The current horizontal position, or `None` if we're finished.
    pub fn x(&self) -> Option<f64> {
        match (
            self.bufs.active_horizontals.first(),
            self.output_events().first(),
        ) {
            (None, None) => None,
            (None, Some(pos)) => Some(pos.smaller_x()),
            (Some(h), None) => Some(h.end),
            (Some(h), Some(pos)) => Some((h.end).min(pos.smaller_x())),
        }
    }

    fn positions_at_x<'c, 'b: 'c>(&'b self, x: f64) -> impl Iterator<Item = &'b OutputEvent> + 'c {
        self.output_events()
            .iter()
            .take_while(move |p| p.smaller_x() == x)
    }

    /// Updates a [`SegmentsConnectedAtX`] to reflect the current horizontal position.
    pub fn update_segments_at_x(&self, segs: &mut SegmentsConnectedAtX) {
        segs.connected_up.clear();
        segs.connected_down.clear();

        let Some(x) = self.x() else {
            return;
        };

        for hseg in &self.bufs.active_horizontals {
            if hseg.connected_above_at(x) {
                segs.connected_up
                    .push((hseg.seg, hseg.old_sweep_idx.unwrap()));
            }

            if hseg.connected_below_at(x) {
                segs.connected_down
                    .push((hseg.seg, hseg.sweep_idx.unwrap()));
            }
        }

        for pos in self.positions_at_x(x) {
            if pos.connected_above_at(x) {
                segs.connected_up
                    .push((pos.seg_idx, pos.old_sweep_idx.unwrap()));
            }

            if pos.connected_below_at(x) {
                segs.connected_down
                    .push((pos.seg_idx, pos.sweep_idx.unwrap()));
            }
        }

        segs.connected_up
            .sort_by_key(|(_seg_idx, sweep_idx)| *sweep_idx);
        segs.connected_down
            .sort_by_key(|(_seg_idx, sweep_idx)| *sweep_idx);
    }

    /// Iterates over the horizontal segments that are active at the current position.
    ///
    /// This includes the segments that end here, but does not include the ones
    /// that start here.
    pub fn active_horizontals(&self) -> impl Iterator<Item = SegIdx> + '_ {
        self.bufs.active_horizontals.iter().map(|hseg| hseg.seg)
    }

    /// Iterates over the horizontal segments that are active at the current position, along
    /// with a boolean telling you whether the horizontal segment has the same orientation
    /// as the segment that it belongs to.
    ///
    /// This includes the segments that end here, but does not include the ones
    /// that start here.
    pub fn active_horizontals_and_orientations(&self) -> impl Iterator<Item = (SegIdx, bool)> + '_ {
        self.bufs
            .active_horizontals
            .iter()
            .map(|hseg| (hseg.seg, hseg.enter_first))
    }

    /// Returns the collection of all output events that end at the current
    /// position, or `None` if this batcher is finished.
    ///
    /// All the returned events start at the previous `x` position and end
    /// at the current `x` position. In particular, if you alternate between
    /// calling [`SweepLineRange::increase_x`] and this method, you'll
    /// receive non-overlapping batches of output events.
    pub fn events(&mut self) -> Option<Vec<OutputEvent>> {
        let next_x = self.x()?;

        let mut ret = Vec::new();
        for h in &self.bufs.active_horizontals {
            // unwrap: on the first event of this sweep line, active_horizontals is empty. So
            // we only get here after last_x is populated.
            let x0 = self.last_x.unwrap();
            let x1 = next_x.min(h.end);
            let connected_end = x1 == h.end && h.connected_at_end;
            let connected_start = x0 == h.start && h.connected_at_start;
            if h.enter_first {
                ret.push(OutputEvent::new(
                    h.seg,
                    x0,
                    connected_start,
                    x1,
                    connected_end,
                    h.sweep_idx,
                    h.old_sweep_idx,
                ));
            } else {
                ret.push(OutputEvent::new(
                    h.seg,
                    x1,
                    connected_end,
                    x0,
                    connected_start,
                    h.sweep_idx,
                    h.old_sweep_idx,
                ));
            }
        }

        // Drain the active horizontals, throwing away horizontal segments that end before
        // the new x position.
        self.drain_active_horizontals(next_x);

        // Move along to the next horizontal position, processing the x events at the current
        // position and either emitting them immediately or saving them as horizontals.
        while let Some(ev) = self.output_events.first() {
            if ev.smaller_x() > next_x {
                break;
            }
            self.output_events = &self.output_events[1..];

            if ev.x0 == ev.x1 {
                // We push output event for points immediately.
                ret.push(ev.clone());
            } else if let Some(hseg) = HFrag::from_position(ev.clone()) {
                // For horizontal segments, we don't output anything straight
                // away. When we update the horizontal position and visit our
                // horizontal segments, we'll output something.
                self.bufs.active_horizontals.push(hseg);
            }
        }
        self.bufs
            .active_horizontals
            .sort_by_key(|a| CheapOrderedFloat::from(a.end));
        self.last_x = Some(next_x);
        Some(ret)
    }

    /// Move along to the next horizontal position.
    pub fn increase_x(&mut self) {
        if let Some(x) = self.x() {
            self.drain_active_horizontals(x);

            while let Some(ev) = self.output_events.first() {
                if ev.smaller_x() > x {
                    break;
                }
                self.output_events = &self.output_events[1..];

                if let Some(hseg) = HFrag::from_position(ev.clone()) {
                    self.bufs.active_horizontals.push(hseg);
                }
            }
        }
        self.bufs
            .active_horizontals
            .sort_by_key(|a| CheapOrderedFloat::from(a.end));
    }

    fn drain_active_horizontals(&mut self, x: f64) {
        let new_start = self
            .bufs
            .active_horizontals
            .iter()
            .position(|h| h.end > x)
            .unwrap_or(self.bufs.active_horizontals.len());
        self.bufs.active_horizontals.drain(..new_start);
    }

    /// The indices within the sweep line represented by this range.
    pub fn seg_range(&self) -> ChangedInterval {
        self.changed_interval.clone()
    }

    /// Returns an iterator over the segments in this range, ordered according
    /// to the "old" sweep-line.
    ///
    /// In addition to appearing in a different order, the set of segments returned
    /// by this method and [`Self::segment_range`] may differ: segments that exit at the
    /// current sweep line will be returned here and not there, while segments that
    /// enter at the current sweep line will be returned there and not here.
    pub fn old_segment_range(&self) -> impl Iterator<Item = SegIdx> + '_ {
        let range = self.changed_interval.segs.clone();
        self.line().old_segment_range(range)
    }

    /// Returns an iterator over the segments in this range, ordered according
    /// to the "new" sweep-line.
    ///
    /// In addition to appearing in a different order, the set of segments returned
    /// by this method and [`Self::old_segment_range`] may differ: segments that exit at the
    /// current sweep line will be returned there and not here, while segments that
    /// enter at the current sweep line will be returned here and not there.
    pub fn segment_range(&self) -> impl Iterator<Item = SegIdx> + '_ {
        let range = self.changed_interval.segs.clone();
        self.line().segment_range(range)
    }

    /// The sweep line that this is a range of.
    pub fn line(&self) -> &SweepLine<'_, '_, 'segs> {
        self.line
    }

    #[cfg(feature = "slow-asserts")]
    fn check_invariants(&self) {
        for ev in self.output_events() {
            let seg = &self.line.segments()[ev.seg_idx];
            let y = self.line.y();
            let eps = self.line.eps();

            // The curve comparison is guaranteed to give a strong
            // ordering when the curves are 1.5 eps apart in ell_infty.
            // That means the perturbed points should always be within
            // 1.5 * sqrt(2) eps in ell_2, which is less than 2.5 eps. We
            // compute distance to accuracy 0.5 eps, so we'll assert
            // that our number is less than 3 eps.

            // Ok, this doesn't work because of https://github.com/linebender/kurbo/issues/446
            // For now, we'll use ell_infty comparisons.
            //
            // let n0 = seg.nearest(p0, eps / 2.0);
            // let n1 = seg.nearest(p1, eps / 2.0);

            // assert!(n0.distance_sq <= eps * eps * 9.0);
            // assert!(n1.distance_sq <= eps * eps * 9.0);

            let (lower, upper) = if seg.is_horizontal() {
                (seg.start().x - 2.0 * eps, seg.end().x + 2.0 * eps)
            } else {
                (seg.lower(y, 2.0 * eps), seg.upper(y, 2.0 * eps))
            };
            assert!(ev.smaller_x() <= upper && lower <= ev.larger_x());
        }
    }
}

/// A re-usable struct for collecting segments at a single position on a sweep-line.
///
/// See [`SweepLineRange::update_segments_at_x`] for where this is used. At
/// any given time, this struct is implicitly associated to a single horizontal
/// position: the position of the `SweepLineRange` last time we were updated.
#[derive(Debug, Default)]
pub struct SegmentsConnectedAtX {
    connected_up: Vec<(SegIdx, usize)>,
    connected_down: Vec<(SegIdx, usize)>,
}

impl SegmentsConnectedAtX {
    /// The segments that are connected up to a previous sweep-line at the
    /// current horizontal position.
    ///
    /// The returned iterator is sorted by the old sweep-line order. In other
    /// words, it will return segments clockwise when viewed from the current
    /// position.
    pub fn connected_up(&self) -> impl Iterator<Item = SegIdx> + '_ {
        self.connected_up.iter().map(|x| x.0)
    }

    /// The segments that are connected down to a subsequent sweep-line at the
    /// current horizontal position.
    ///
    /// The returned iterator is sorted by the new sweep-line order. In other
    /// words, it will return segments counter-clockwise when viewed from the current
    /// position.
    pub fn connected_down(&self) -> impl Iterator<Item = SegIdx> + '_ {
        self.connected_down.iter().map(|x| x.0)
    }
}

/// Holds some buffers that are used when iterating over a sweep-line.
///
/// Save on re-allocation by allocating this once and reusing it in multiple calls to
/// [`SweepLine::next_range`].
#[derive(Clone, Debug, Default)]
pub struct SweepLineRangeBuffers {
    /// All the horizontal segments overlapping with the sweep-line-range's current
    /// horizontal position, ordered by ending position.
    ///
    /// We could keep this in an ordered data structure, but it turns out to
    /// be faster to just sort it regularly: every time we modify this, we also
    /// advance the horizontal position and iterate over this entire collection.
    /// Therefore, there's no asymptotic run-time to be gained by having a fast
    /// way to insert/delete a single element.
    active_horizontals: Vec<HFrag>,
}

impl SweepLineRangeBuffers {
    pub(crate) fn clear(&mut self) {
        self.active_horizontals.clear();
    }
}
