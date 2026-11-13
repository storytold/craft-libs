//! Utilities for computing topological properties of closed paths.
//!
//! This consumes the output of the sweep-line algorithm and does things
//! like winding number computations and boolean operations.

use std::collections::VecDeque;

use kurbo::{BezPath, ParamCurve, Rect, Shape};

use crate::{
    FillRule,
    curve::{Order, solve_t_for_y, y_subsegment},
    geom::Point,
    order::ComparisonCache,
    position::PositionedOutputSeg,
    segments::{NonClosedPath, SegIdx, SegVec, Segments},
    sweep::{
        Correctness, SegmentsConnectedAtX, SweepLineBuffers, SweepLineRange, SweepLineRangeBuffers,
        Sweeper,
    },
};

/// An abstraction over winding numbers.
///
/// The windings numbers of a set can just be represented by integers. But if
/// you're doing boolean operations over two sets, you need both winding numbers
/// to determine whether a point is inside or outside the output set. Since we
/// also want to support more complicated situations (ternary operations, non-closed
/// paths for which winding numbers aren't defined, etc.), our topologies use generic
/// winding numbers: anything that implements this trait can be used.
pub trait WindingNumber:
    Clone + std::fmt::Debug + std::ops::Add<Output = Self> + std::ops::AddAssign + Default + Eq
{
    /// A tag for categorizing segments.
    ///
    /// When building a topology, you can assign to each segment a tag, which can then be
    /// used to figure out that segment's winding number contribution.
    #[cfg(not(test))]
    type Tag: Copy + std::fmt::Debug + Eq;

    /// A tag for categorizing segments.
    ///
    /// When building a topology, you can assign to each segment a tag, which can then be
    /// used to figure out that segment's winding number contribution.
    #[cfg(test)]
    type Tag: Copy + std::fmt::Debug + Eq + serde::Serialize;

    /// What is the winding number of a simple curve with tag `tag`?
    fn single(tag: Self::Tag, positive: bool) -> Self;

    /// What if we restrict this winding number to only look at the set with tag `tag`?
    fn of_tag(&self, tag: Self::Tag) -> Self;
}

/// Winding numbers for binary set operations.
///
/// This is just two integers, one for each set. Segments are tagged by
/// booleans, where `true` means that the segment is part of the first set.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq, Default)]
pub struct BinaryWindingNumber {
    /// The winding number of the first shape.
    pub shape_a: i32,
    /// The winding number of the second shape.
    pub shape_b: i32,
}

/// A tag for binary set operations, where we call the two options
/// "set a" and "set b".
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq, Default)]
pub enum OneOfTwo {
    /// The tag for "set a".
    #[default]
    A,
    /// The tag for "set b".
    B,
}

impl WindingNumber for BinaryWindingNumber {
    type Tag = OneOfTwo;

    fn single(tag: Self::Tag, positive: bool) -> Self {
        let sign = if positive { 1 } else { -1 };
        match tag {
            OneOfTwo::A => Self {
                shape_a: sign,
                shape_b: 0,
            },
            OneOfTwo::B => Self {
                shape_a: 0,
                shape_b: sign,
            },
        }
    }

    fn of_tag(&self, tag: OneOfTwo) -> Self {
        match tag {
            OneOfTwo::A => Self {
                shape_a: self.shape_a,
                shape_b: 0,
            },
            OneOfTwo::B => Self {
                shape_a: 0,
                shape_b: self.shape_b,
            },
        }
    }
}

impl std::ops::Add for BinaryWindingNumber {
    type Output = Self;

    fn add(self, rhs: Self) -> Self::Output {
        Self {
            shape_a: self.shape_a + rhs.shape_a,
            shape_b: self.shape_b + rhs.shape_b,
        }
    }
}

impl std::ops::AddAssign for BinaryWindingNumber {
    fn add_assign(&mut self, rhs: Self) {
        *self = *self + rhs
    }
}

impl std::fmt::Debug for BinaryWindingNumber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}a + {}b", self.shape_a, self.shape_b)
    }
}

impl WindingNumber for i32 {
    type Tag = ();

    fn single((): Self::Tag, positive: bool) -> Self {
        if positive { 1 } else { -1 }
    }

    fn of_tag(&self, _tag: ()) -> Self {
        *self
    }
}

/// For a segment, we store two winding numbers (one on each side of the segment).
///
/// For simple segments, the winding numbers on two sides only differ by one. Once
/// we merge segments, they can differ by more.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq, Default)]
pub struct HalfSegmentWindingNumbers<W: WindingNumber> {
    /// This half-segment is incident to a point. Imagine you're standing at
    /// that point, looking out along the segment. This is the winding number of
    /// the area just counter-clockwise (to the left, from your point of view)
    /// of the segment.
    pub counter_clockwise: W,
    /// This half-segment is incident to a point. Imagine you're standing at
    /// that point, looking out along the segment. This is the winding number of
    /// the area just clockwise (to the right, from your point of view) of the segment.
    pub clockwise: W,
}

impl<W: WindingNumber> HalfSegmentWindingNumbers<W> {
    /// A half-segment's winding numbers are trivial if they're the same on both sides.
    /// In this case, the segment is invisible to the topology of the sets.
    fn is_trivial(&self) -> bool {
        self.counter_clockwise == self.clockwise
    }
}

impl<W: WindingNumber> std::fmt::Debug for HalfSegmentWindingNumbers<W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "cw {:?} | cc {:?}",
            self.clockwise, self.counter_clockwise
        )
    }
}

/// An index into the set of output segments.
///
/// There's no compile-time magic preventing misuse of this index, but you
/// should only use this to index into the [`Topology`] that you got it from.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct OutputSegIdx(usize);

impl OutputSegIdx {
    /// Returns an index to the first half of this output segment.
    pub fn first_half(self) -> HalfOutputSegIdx {
        HalfOutputSegIdx {
            idx: self,
            first_half: true,
        }
    }

    /// Returns an index to the second half of this output segment.
    pub fn second_half(self) -> HalfOutputSegIdx {
        HalfOutputSegIdx {
            idx: self,
            first_half: false,
        }
    }
}

/// A vector indexed by output segments.
///
/// See [`OutputSegIdx`] for more about output segments.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct OutputSegVec<T> {
    inner: Vec<T>,
}

impl_typed_vec!(OutputSegVec, OutputSegIdx, "o");

/// An index that refers to one end of an output segment.
///
/// The two ends of an output segment are sweep-line ordered: the "first" half
/// has a smaller `y` coordinate (or smaller `x` coordinate if the `y`s are
/// tied) than the "second" half.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct HalfOutputSegIdx {
    idx: OutputSegIdx,
    first_half: bool,
}

impl HalfOutputSegIdx {
    /// Returns the index pointing to the other end of this output segment.
    pub fn other_half(self) -> Self {
        Self {
            idx: self.idx,
            first_half: !self.first_half,
        }
    }

    /// Do we point to the first half of the output segment?
    pub fn is_first_half(self) -> bool {
        self.first_half
    }
}

impl std::fmt::Debug for HalfOutputSegIdx {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.first_half {
            write!(f, "{:?}->", self.idx)
        } else {
            write!(f, "->{:?}", self.idx)
        }
    }
}

/// A vector indexed by half-output segments.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Hash, PartialEq, Eq)]
pub struct HalfOutputSegVec<T> {
    start: Vec<T>,
    end: Vec<T>,
}

impl<T> HalfOutputSegVec<T> {
    /// Creates a new vector that can store `cap` output segments without re-allocating.
    pub fn with_capacity(cap: usize) -> Self {
        Self {
            start: Vec::with_capacity(cap),
            end: Vec::with_capacity(cap),
        }
    }
}

impl<T: Default> HalfOutputSegVec<T> {
    /// Creates a new vector with `size` output segments.
    ///
    /// Both halves of each output segment are initialized to their default values.
    pub fn with_size(size: usize) -> Self {
        Self {
            start: std::iter::from_fn(|| Some(T::default()))
                .take(size)
                .collect(),
            end: std::iter::from_fn(|| Some(T::default()))
                .take(size)
                .collect(),
        }
    }
}

impl<T: std::fmt::Debug> std::fmt::Debug for HalfOutputSegVec<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        struct Entry<'a, T> {
            idx: usize,
            start: &'a T,
            end: &'a T,
        }

        impl<T: std::fmt::Debug> std::fmt::Debug for Entry<'_, T> {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                write!(
                    f,
                    "{idx:4}: {start:?} -> {end:?}",
                    idx = self.idx,
                    start = self.start,
                    end = self.end
                )
            }
        }

        let mut list = f.debug_list();
        for (idx, (start, end)) in self.start.iter().zip(&self.end).enumerate() {
            list.entry(&Entry { idx, start, end });
        }
        list.finish()
    }
}

impl<T> Default for HalfOutputSegVec<T> {
    fn default() -> Self {
        Self {
            start: Vec::new(),
            end: Vec::new(),
        }
    }
}

impl<T> std::ops::Index<HalfOutputSegIdx> for HalfOutputSegVec<T> {
    type Output = T;

    fn index(&self, index: HalfOutputSegIdx) -> &Self::Output {
        if index.first_half {
            &self.start[index.idx.0]
        } else {
            &self.end[index.idx.0]
        }
    }
}

impl<T> std::ops::IndexMut<HalfOutputSegIdx> for HalfOutputSegVec<T> {
    fn index_mut(&mut self, index: HalfOutputSegIdx) -> &mut T {
        if index.first_half {
            &mut self.start[index.idx.0]
        } else {
            &mut self.end[index.idx.0]
        }
    }
}

impl<T> std::ops::Index<HalfOutputSegIdx> for OutputSegVec<T> {
    type Output = T;

    fn index(&self, index: HalfOutputSegIdx) -> &Self::Output {
        &self.inner[index.idx.0]
    }
}

/// An index into the set of points.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
pub struct PointIdx(usize);

#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Hash, PartialEq, Eq)]
struct PointVec<T> {
    inner: Vec<T>,
}

impl_typed_vec!(PointVec, PointIdx, "p");

#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Hash, PartialEq, Eq)]
struct PointNeighbors {
    clockwise: HalfOutputSegIdx,
    counter_clockwise: HalfOutputSegIdx,
}

impl std::fmt::Debug for PointNeighbors {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{:?} o {:?}", self.counter_clockwise, self.clockwise)
    }
}

/// Consumes sweep-line output and computes topology.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Debug)]
pub struct Topology<W: WindingNumber> {
    eps: f64,
    correctness: Correctness,
    /// The user-provided tags for each segment.
    tag: SegVec<W::Tag>,
    /// For each input segment, this is the list of output segments that we've started
    /// recording but haven't finished with. There can be up to three of them, because
    /// consider a segment that passes through a sweep-line like this:
    ///
    /// ```text
    ///           /
    ///          /
    /// (*) /---/
    ///    /
    ///   /
    /// ```
    ///
    /// When we come to process the sweep-line at height (*), we'll already have the
    /// unfinished output segment coming from above. But before dealing with it, we'll
    /// first encounter the output segment pointing down and add an unfinished segment
    /// for that. Then we'll add an output segment for the horizontal line and so
    /// at that point there will be three unfinished output segments.
    open_segs: SegVec<VecDeque<OutputSegIdx>>,
    /// Winding numbers of each segment.
    ///
    /// This is sort of logically indexed by `HalfOutputSegIdx`, because we can look at the
    /// `HalfSegmentWindingNumbers` for each `HalfOutputSegIdx`. But since the two halves of
    /// the winding numbers are determined by one another, we only store the winding numbers
    /// for the start half of the output segment.
    winding: OutputSegVec<HalfSegmentWindingNumbers<W>>,
    /// The output points.
    points: PointVec<Point>,
    /// The segment endpoints, as indices into `points`.
    point_idx: HalfOutputSegVec<PointIdx>,
    /// For each output half-segment, its neighboring segments are the ones that share a point with it.
    point_neighbors: HalfOutputSegVec<PointNeighbors>,
    /// Marks the output segments that have been deleted due to merges of coindident segments.
    deleted: OutputSegVec<bool>,
    /// For each non-horizontal output segment, if we go just a tiny bit south of its starting
    /// position then which output segment is to the west?
    ///
    /// The map from a segment to its scan-west neighbor is always strictly decreasing (in the
    /// index). This ensures that a scan will always terminate, and it also means that we can
    /// build the contours in increasing `OutputSegIdx` order.
    scan_west: OutputSegVec<Option<OutputSegIdx>>,
    /// For each non-horizontal output segment that's at the east edge of its sweep-line-range,
    /// this is the segment to the east... but *only* if the segment to the east continues through
    /// the sweep-line where this output segment started.
    ///
    /// That is, in this situation (where the `->` points to our sweep-line) there will be no
    /// "scan_east" relation from s1 to s2:
    ///
    /// ```text
    /// \            /
    ///  \          /
    ///   \        /
    /// -> ◦      ◦
    ///    |      |
    ///    |      |
    ///    |      |
    ///  (s1)   (s2)
    /// ```
    ///
    /// However, in this situation there will be a "scan_east" relation from s1 to s2.
    ///
    /// ```text
    /// \         |
    ///  \        |
    ///   \       |
    /// -> ◦      |
    ///    |      |
    ///    |      |
    ///    |      |
    ///   (s1)   (s2)
    /// ```
    ///
    /// The reason behind this seemingly-arbitrary choice is that in the former situation
    /// it's harder to build up `scan_east` (because the output segment `s2` hasn't bee
    /// created when we process `s1`) and it's redundant with the `scan_west` relation from
    /// `s2` to `s1`.
    scan_east: OutputSegVec<Option<OutputSegIdx>>,
    /// This stores all the situations where two segments become scan-line
    /// neighbors because something in between dropped out. For example:
    ///
    /// ```text
    ///  |    \  |      |
    ///  |     \ |      |
    ///  |      \|      |
    ///  |       ◦      | <- y
    ///  |              |
    ///  |              |
    ///  |              |
    /// (s1)           (s2)
    /// ```
    ///
    /// Here we would add an entry `(y, s1, s2)` to `scan_after`. If `s2`
    /// weren't there (and there was nothing else further right), we'd add `(y,
    /// s1, None)` to denote that `s1` has no scan-east neighbor after `y`.
    scan_after: Vec<(f64, Option<OutputSegIdx>, Option<OutputSegIdx>)>,
    /// For each output segment, the input segment that it came from. Will probably become
    /// private at some point (TODO)
    pub orig_seg: OutputSegVec<SegIdx>,
    // TODO: probably don't have this owned
    /// The collection of segments used to build this topology. Will probably become private
    /// at some point (TODO)
    #[cfg_attr(test, serde(skip))]
    pub segments: Segments,
    /// Does the sweep-line orientation of each output segment agree with its original
    /// orientation from the input?
    ///
    /// In principle, this is redundant with the winding numbers. But since the winding
    /// numbers are generic, that information isn't available to us.
    positively_oriented: OutputSegVec<bool>,
}

impl<W: WindingNumber> Topology<W> {
    /// Construct a new topology from a collection of paths and tags.
    ///
    /// See [`WindingNumber`] about the tags are for.
    pub fn from_paths<'a, Iter>(paths: Iter, eps: f64) -> Result<Self, NonClosedPath>
    where
        Iter: IntoIterator<Item = (&'a BezPath, W::Tag)>,
    {
	Self::from_paths_with_correctness(paths, eps, Correctness::GoodEnough)
    }

    /// Construct a new topology from a collection of paths and tags.
    ///
    /// See [`WindingNumber`] about the tags are for.
    pub fn from_paths_with_correctness<'a, Iter>(paths: Iter, eps: f64, correctness: Correctness) -> Result<Self, NonClosedPath>
    where
        Iter: IntoIterator<Item = (&'a BezPath, W::Tag)>,
    {
        let mut segments = Segments::default();
        let mut tag = Vec::new();
        for (p, t) in paths {
            segments.add_path_without_updating_enter_exit(p, true)?;
            tag.resize(segments.len(), t);
        }
        segments.update_enter_exit(0);
        segments.check_invariants();
        Ok(Self::from_segments(
            segments,
            SegVec::from_vec(tag),
            eps,
	    correctness,
        ))
    }

    fn from_segments(
        segments: Segments,
        tag: SegVec<W::Tag>,
        eps: f64,
        correctness: Correctness,
    ) -> Self {
        let mut ret = Self {
            eps,
            correctness,
            tag,
            open_segs: SegVec::with_size(segments.len()),

            // We have at least as many output segments as input segments, so preallocate enough for them.
            winding: OutputSegVec::with_capacity(segments.len()),
            points: PointVec::with_capacity(segments.len()),
            point_idx: HalfOutputSegVec::with_capacity(segments.len()),
            point_neighbors: HalfOutputSegVec::with_capacity(segments.len()),
            deleted: OutputSegVec::with_capacity(segments.len()),
            scan_west: OutputSegVec::with_capacity(segments.len()),
            scan_east: OutputSegVec::with_capacity(segments.len()),
            scan_after: Vec::new(),
            orig_seg: OutputSegVec::with_capacity(segments.len()),
            positively_oriented: OutputSegVec::with_capacity(segments.len()),
            segments: Segments::default(),
        };
        let mut sweep_state = Sweeper::new_with_correctness(&segments, eps, correctness);
        let mut range_bufs = SweepLineRangeBuffers::default();
        let mut line_bufs = SweepLineBuffers::default();
        while let Some(mut line) = sweep_state.next_line(&mut line_bufs) {
            while let Some(positions) = line.next_range(&mut range_bufs, &segments) {
                let range = positions.seg_range();
                let scan_west_seg = if range.segs.start == 0 {
                    None
                } else {
                    let prev_seg = positions.line().line_segment(range.segs.start - 1).unwrap();
                    debug_assert!(!ret.open_segs[prev_seg].is_empty());
                    ret.open_segs[prev_seg].front().copied()
                };
                ret.process_sweep_line_range(positions, &segments, scan_west_seg);
            }
        }
        ret.segments = segments;

        #[cfg(feature = "slow-asserts")]
        ret.check_invariants();

        ret.merge_coincident();

        #[cfg(feature = "slow-asserts")]
        ret.check_invariants();

        ret
    }

    /// We're working on building up a list of half-segments that all meet at a point.
    /// Maybe we've done a few already, but there's a region where we may add more.
    /// Something like this:
    ///
    /// ```text
    ///         \
    ///          \  ?
    ///           \  ?
    ///       -----o  ?
    ///           /|  ?
    ///          / |?
    ///         /  |
    /// ```
    ///
    /// `first_seg` is the most-counter-clockwise segment we've added so far
    /// (the one pointing down in the picture above, and `last_seg` is the
    /// most-clockwise one (pointing up-left in the picture above). These can be
    /// `None` if we haven't actually added any segments left.
    ///
    /// This method adds more segments to the picture, starting at `last_seg`
    /// and working clockwise. It works only with segment indices, so it's the
    /// caller's responsibility to ensure that the geometry is correct, and that
    /// the provided segments actually go in clockwise order (relative to each
    /// other, and also relative to the segments we've already placed).
    fn add_segs_clockwise(
        &mut self,
        first_seg: &mut Option<HalfOutputSegIdx>,
        last_seg: &mut Option<HalfOutputSegIdx>,
        segs: impl Iterator<Item = HalfOutputSegIdx>,
        p: PointIdx,
    ) {
        for seg in segs {
            self.point_idx[seg] = p;
            if first_seg.is_none() {
                *first_seg = Some(seg);
            }
            if let Some(last) = last_seg {
                self.point_neighbors[*last].clockwise = seg;
                self.point_neighbors[seg].counter_clockwise = *last;
            }
            *last_seg = Some(seg);
        }
        if let Some((first, last)) = first_seg.zip(*last_seg) {
            self.point_neighbors[last].clockwise = first;
            self.point_neighbors[first].counter_clockwise = last;
        }
    }

    /// Like `add_segs_clockwise`, but adds them on the other side.
    fn add_segs_counter_clockwise(
        &mut self,
        first_seg: &mut Option<HalfOutputSegIdx>,
        last_seg: &mut Option<HalfOutputSegIdx>,
        segs: impl Iterator<Item = HalfOutputSegIdx>,
        p: PointIdx,
    ) {
        for seg in segs {
            self.point_idx[seg] = p;
            if last_seg.is_none() {
                *last_seg = Some(seg);
            }
            if let Some(first) = first_seg {
                self.point_neighbors[*first].counter_clockwise = seg;
                self.point_neighbors[seg].clockwise = *first;
            }
            *first_seg = Some(seg);
        }
        if let Some((first, last)) = first_seg.zip(*last_seg) {
            self.point_neighbors[last].clockwise = first;
            self.point_neighbors[first].counter_clockwise = last;
        }
    }

    /// Takes some segments where we've already placed the first half, and
    /// gets ready to place the second half.
    ///
    /// The state-tracking is subtle and should be re-considered. The basic
    /// issue is that (as discussed in the documentation for `open_segs`) a
    /// single segment index can have three open half-segments at any one time,
    /// so how do we know which one is ready for its second half? The short
    /// answer is that we use a double-ended queue, and see `new_half_seg`
    /// for how we use it.
    fn second_half_segs<'a, 'slf: 'a>(
        &'slf mut self,
        segs: impl Iterator<Item = SegIdx> + 'a,
    ) -> impl Iterator<Item = HalfOutputSegIdx> + 'a {
        segs.map(|s| {
            self.open_segs[s]
                .pop_front()
                .expect("should be open")
                .second_half()
        })
    }

    /// Creates a new half-segment.
    ///
    /// This needs to update the open segment state to be compatible with `second_half_segs`.
    ///
    /// The key is that we know the order that the segments are processed: any
    /// horizontal segments will be closed first, followed by segments coming
    /// from an earlier sweep-line, followed by segments extending down from
    /// this sweep-line (which won't be closed until we move on to the next
    /// sweep-line). Therefore, we push horizontal half-segments to the front
    /// of the queue so that they can be taken next. We push non-horizontal
    /// half-segments to the back of the queue, so that the older ones (coming
    /// from the previous sweep-line) will get taken before the new ones.
    fn new_half_seg(
        &mut self,
        idx: SegIdx,
        p: PointIdx,
        winding: HalfSegmentWindingNumbers<W>,
        horizontal: bool,
        positively_oriented: bool,
    ) -> OutputSegIdx {
        let out_idx = OutputSegIdx(self.winding.inner.len());
        if horizontal {
            self.open_segs[idx].push_front(out_idx);
        } else {
            self.open_segs[idx].push_back(out_idx);
        }
        self.point_idx.start.push(p);
        self.point_idx
            .end
            // TODO: maybe an option instead of this weird sentinel
            .push(PointIdx(usize::MAX));

        let no_nbrs = PointNeighbors {
            clockwise: out_idx.first_half(),
            counter_clockwise: out_idx.first_half(),
        };
        self.point_neighbors.start.push(no_nbrs);
        self.point_neighbors.end.push(no_nbrs);
        self.winding.inner.push(winding);
        self.deleted.inner.push(false);
        self.scan_west.inner.push(None);
        self.scan_east.inner.push(None);
        self.orig_seg.inner.push(idx);
        self.positively_oriented.inner.push(positively_oriented);
        out_idx
    }

    fn process_sweep_line_range(
        &mut self,
        mut pos: SweepLineRange,
        segments: &Segments,
        mut scan_west: Option<OutputSegIdx>,
    ) {
        let y = pos.line().y();
        let mut winding = scan_west
            .map(|idx| self.winding[idx].counter_clockwise.clone())
            .unwrap_or_default();

        // A re-usable buffer for holding temporary lists of segment indices. We
        // need to collect the output of second_half_segments because it holds
        // a &mut self reference.
        let mut seg_buf = Vec::new();
        let mut connected_segs = SegmentsConnectedAtX::default();
        // The last segment we saw that points down from this sweep line.
        let mut last_connected_down_seg: Option<OutputSegIdx> = None;

        while let Some(next_x) = pos.x() {
            let p = PointIdx(self.points.inner.len());
            self.points.inner.push(Point::new(next_x, y));
            // The first segment at our current point, in clockwise order.
            let mut first_seg = None;
            // The last segment at our current point, in clockwise order.
            let mut last_seg = None;

            // Close off the horizontal segments from the previous point in this sweep-line.
            let hsegs = pos.active_horizontals();
            seg_buf.clear();
            seg_buf.extend(self.second_half_segs(hsegs));
            self.add_segs_clockwise(&mut first_seg, &mut last_seg, seg_buf.iter().copied(), p);

            // Find all the segments that are connected to something above this sweep-line at next_x.
            pos.update_segments_at_x(&mut connected_segs);
            seg_buf.clear();
            seg_buf.extend(self.second_half_segs(connected_segs.connected_up()));
            self.add_segs_clockwise(&mut first_seg, &mut last_seg, seg_buf.iter().copied(), p);

            // Then: gather the output segments from half-segments starting here and moving
            // to later sweep-lines. Allocate new output segments for them.
            // Also, calculate their winding numbers and update `winding`.
            seg_buf.clear();
            for new_seg in connected_segs.connected_down() {
                let prev_winding = winding.clone();
                let orientation = segments.positively_oriented(new_seg);
                winding += W::single(self.tag[new_seg], !orientation);
                let windings = HalfSegmentWindingNumbers {
                    clockwise: prev_winding,
                    counter_clockwise: winding.clone(),
                };
                let half_seg = self.new_half_seg(new_seg, p, windings, false, orientation);
                self.scan_west[half_seg] = scan_west;
                scan_west = Some(half_seg);
                seg_buf.push(half_seg.first_half());

                last_connected_down_seg = Some(half_seg);
            }
            self.add_segs_counter_clockwise(
                &mut first_seg,
                &mut last_seg,
                seg_buf.iter().copied(),
                p,
            );

            // Bump the current x position, which will get rid of horizontals ending here
            // and add any horizontals starting here.
            pos.increase_x();

            // Gather the output segments from horizontal segments starting
            // here. Allocate new output segments for them and calculate their
            // winding numbers.
            let hsegs = pos.active_horizontals_and_orientations();

            // We don't want to update our "global" winding number state because that's supposed
            // to keep track of the winding number below the current sweep line.
            let mut w = winding.clone();
            seg_buf.clear();
            for (new_seg, same_orientation) in hsegs {
                let prev_w = w.clone();
                let orientation = same_orientation == segments.positively_oriented(new_seg);
                w += W::single(self.tag[new_seg], !orientation);
                let windings = HalfSegmentWindingNumbers {
                    counter_clockwise: w.clone(),
                    clockwise: prev_w,
                };
                let half_seg = self.new_half_seg(new_seg, p, windings, true, orientation);
                self.scan_west[half_seg] = scan_west;
                seg_buf.push(half_seg.first_half());
            }
            self.add_segs_counter_clockwise(
                &mut first_seg,
                &mut last_seg,
                seg_buf.iter().copied(),
                p,
            );
        }

        if let Some(seg) = last_connected_down_seg
            && let Some(east_nbr) = pos.line().line_entry(pos.seg_range().segs.end)
            && !east_nbr.is_in_changed_interval()
        {
            self.scan_east[seg] = self.open_segs[east_nbr.seg].front().copied()
        }

        // If something was connected up but nothing was connected down, let's remember that
        // the scan-line order has changed.
        if last_connected_down_seg.is_none() {
            let seg_range = pos.seg_range().segs;
            let west_nbr = seg_range
                .start
                .checked_sub(1)
                .and_then(|idx| pos.line().line_segment(idx));
            let east_nbr = pos.line().line_segment(pos.seg_range().segs.end);
            let west = west_nbr.map(|idx| *self.open_segs[idx].front().unwrap());
            let east = east_nbr.map(|idx| *self.open_segs[idx].front().unwrap());
            if west.is_some() || east.is_some() {
                self.scan_after.push((y, west, east));
            }
        }
    }

    fn delete_half(&mut self, half_seg: HalfOutputSegIdx) {
        let nbr = self.point_neighbors[half_seg];
        self.point_neighbors[nbr.clockwise].counter_clockwise = nbr.counter_clockwise;
        self.point_neighbors[nbr.counter_clockwise].clockwise = nbr.clockwise;
    }

    fn delete(&mut self, seg: OutputSegIdx) {
        self.deleted[seg] = true;
        self.delete_half(seg.first_half());
        self.delete_half(seg.second_half());
    }

    fn is_horizontal(&self, seg: OutputSegIdx) -> bool {
        self.point(seg.first_half()).y == self.point(seg.second_half()).y
    }

    /// After generating the topology, there's a good chance we end up with
    /// coincident output segments. This method removes coincident segments. If
    /// a collection of coincident segments has a net winding number of zero,
    /// this method just removes them all. Otherwise, they are replaced by a
    /// single segment.
    ///
    /// In principle, we could do this as we build the topology. The thing that
    /// makes it a little bit tricky is that (except for horizontal segments)
    /// we don't know whether two segments are coincident until we've processed
    /// their second endpoint.
    fn merge_coincident(&mut self) {
        for idx in 0..self.winding.inner.len() {
            let idx = OutputSegIdx(idx);
            if self.deleted[idx] {
                continue;
            }
            let cw_nbr = self.point_neighbors[idx.first_half()].clockwise;
            if self.point_idx[idx.second_half()] == self.point_idx[cw_nbr.other_half()] {
                // We've found two segments with the same starting and ending points, but
                // that doesn't mean they're coincident!
                // TODO: we could use the comparison cache here, if we could keep it around somewhere.
                let y0 = self.point(idx.first_half()).y;
                let y1 = self.point(idx.second_half()).y;

                // Because the closeness comparison below isn't transitive, we need to be careful
                // to only merge sweep-line neighbors. As an example for what can go wrong, suppose
                // we have 4 segments with the same starting and ending points. Let's say that
                // their sweep-line order is a, b, c, d, and suppose that a and d are close but
                // b and c aren't. Then merging a and d would be bad -- even if they're clockwise
                // neighbors by going around the way that avoids b and c -- because it would mess up
                // the winding numbers of b and c.
                if y0 != y1 && self.scan_west[idx] != Some(cw_nbr.idx) {
                    continue;
                }
                if y0 != y1 {
                    let s1 = self.orig_seg[idx];
                    let s2 = self.orig_seg[cw_nbr];
                    let s1 = y_subsegment(self.segments[s1].to_kurbo_cubic(), y0, y1);
                    let s2 = y_subsegment(self.segments[s2].to_kurbo_cubic(), y0, y1);
                    if (s1.p0 - s2.p0).hypot() > self.eps
                        || (s1.p1 - s2.p1).hypot() > self.eps
                        || (s1.p2 - s2.p2).hypot() > self.eps
                        || (s1.p3 - s2.p3).hypot() > self.eps
                    {
                        continue;
                    }
                }

                // All output segments are in sweep line order, so if they're
                // coincident then they'd better both be first halves.
                debug_assert!(cw_nbr.first_half);
                self.delete(idx);
                self.winding[cw_nbr.idx].counter_clockwise =
                    self.winding[idx].counter_clockwise.clone();

                if self.winding[cw_nbr.idx].is_trivial() {
                    self.delete(cw_nbr.idx);
                }
            }
        }
    }

    /// Iterates over indices of all output segments.
    pub fn segment_indices(&self) -> impl Iterator<Item = OutputSegIdx> + '_ {
        (0..self.winding.inner.len())
            .map(OutputSegIdx)
            .filter(|i| !self.deleted[*i])
    }

    /// Returns the winding numbers of the region on the clockwise side of an
    /// output half-segment.
    pub fn winding_clockwise(&self, idx: HalfOutputSegIdx) -> &W {
        if idx.first_half {
            &self.winding[idx.idx].clockwise
        } else {
            &self.winding[idx.idx].counter_clockwise
        }
    }

    /// Returns the winding numbers of the region on the counter-clockwise side of an
    /// output half-segment.
    pub fn winding_counter_clockwise(&self, idx: HalfOutputSegIdx) -> &W {
        if idx.first_half {
            &self.winding[idx.idx].counter_clockwise
        } else {
            &self.winding[idx.idx].clockwise
        }
    }

    /// Returns the endpoint of an output half-segment.
    pub fn point(&self, idx: HalfOutputSegIdx) -> &Point {
        &self.points[self.point_idx[idx]]
    }

    /// Checks whether two of the input sets to this topology are disjoint.
    ///
    /// The `inside1` and `inside2` callbacks test whether a winding number
    /// is inside the first set or inside the second set, respectively.
    ///
    /// If the two input sets intersect at a vertex (but not along an edge),
    /// they are considered disjoint. Also, the disjointness is tested after
    /// some topological simplification, and so it might not "see" thin spikes.
    ///
    /// The result of this method is not guaranteed to agree with the output
    /// of [`Topology::contours`] because there are approximations involved.
    /// For example, we might return `true` even when the contours are just
    /// barely overlapping.
    pub fn is_disjoint_tags(
        &self,
        inside1: impl Fn(&W) -> bool,
        inside2: impl Fn(&W) -> bool,
    ) -> bool {
        self.winding.indices().all(|out_idx| {
            let w = &self.winding[out_idx];

            let touches1 = inside1(&w.clockwise) || inside1(&w.counter_clockwise);
            let touches2 = inside2(&w.clockwise) || inside2(&w.counter_clockwise);
            dbg!(touches1, touches2);
            !(touches1 && touches2)
        })
    }

    /// Returns an iterator over all output segments from the given set.
    ///
    /// Note that this includes segments that were merged because they were coincident with something else.
    fn out_segments_of(&self, tag: W::Tag) -> impl Iterator<Item = OutputSegIdx> + use<'_, W> {
        self.orig_seg
            .indices()
            .filter(move |&i| self.tag[self.orig_seg[i]] == tag)
    }

    /// Checks whether one of the input sets has a "normal" boundary.
    ///
    /// We say the boundary is "normal" if the winding number just to the right
    /// of the boundary is always 1. Our convention is that the winding number is 1
    /// for a clockwise curve in y-down coordinates.
    ///
    /// For example, this set (traced out by two disjoint contours) has a normal
    /// boundary:
    ///
    /// ```text
    ///   ╭───>───╮
    ///   │xxxxxxx│
    ///   │xxxxxxx│  ╭───>───╮
    ///   ╰───<───╯  │xxxxxxx│
    ///              │xxxxxxx│
    ///              ╰───<───╯
    /// ```
    ///
    /// This example does not have a normal boundary, because one of the contours
    /// goes the wrong way (and so its winding numbers are -1, not 1).
    ///
    /// ```text
    ///   ╭───>───╮
    ///   │xxxxxxx│
    ///   │xxxxxxx│  ╭───<───╮
    ///   ╰───<───╯  │xxxxxxx│
    ///              │xxxxxxx│
    ///              ╰───>───╯
    /// ```
    ///
    /// This example does not have a normal boundary, because it has some places with
    /// a winding number of 2.
    ///
    /// ```text
    ///   ╭───>───╮
    ///   │xxxxxxx│
    ///   │xxxxx╭───<───╮
    ///   ╰───<─│─╯xxxxx│
    ///         │xxxxxxx│
    ///         ╰───>───╯
    /// ```
    ///
    /// Up to precision issues, the output of [`Topology::contours`] will
    /// have a normal boundary, so `has_normal_contours` is a reasonable proxy
    /// for checking whether an input set will be passed through unchanged by
    /// `Topology::contours`. For example, suppose you're trying to compute the
    /// difference between set `a` and set `b`. If [`Topology::is_disjoint`]
    /// says that the two sets are disjoint and `has_normal_contours` returns
    /// `true`, you can decide that the intersection is just `a`. This isn't
    /// a hard guarantee that `Topology::contours` will return `a` exactly:
    /// it probably won't, because of perturbations and ordering changes.
    pub fn has_normal_contours(&self, tag: W::Tag) -> bool {
        self.out_segments_of(tag).all(|out_idx| {
            let w = &self.winding[out_idx];

            // Find the winding number to the right, as we walk in the original
            // orientation.
            let w = if self.positively_oriented[out_idx] {
                &w.clockwise
            } else {
                &w.counter_clockwise
            };
            w.of_tag(tag) == W::single(tag, true)
        })
    }

    /// The segments in `segs` form a closed path, and each one is the ending half
    /// of its segment.
    ///
    /// This basically does two things:
    ///   - puts the segments in the correct orientation
    ///   - merges (if possible) segments that were divided because of the
    ///     monotonicity constraints.
    fn segs_to_path(
        &self,
        segs: &[HalfOutputSegIdx],
        positions: &OutputSegVec<PositionedOutputSeg>,
    ) -> BezPath {
        let mut ret = BezPath::default();
        let mut unfinished: Option<f64> = None;

        // Find a segment that can't be merged into the previous one.
        let start_idx = segs
            .iter()
            .position(|s| self.can_merge(s.other_half(), positions).is_none())
            .unwrap();
        ret.move_to(self.point(segs[start_idx].other_half()).to_kurbo());
        for seg in segs[start_idx..].iter().chain(&segs[..start_idx]) {
            let path = &positions[seg.idx];

            // TODO: avoid the allocation in reverse_subpaths
            let rev_path;
            let oriented_path = if seg.is_first_half() {
                rev_path = path.path.reverse_subpaths();
                &rev_path
            } else {
                &path.path
            };
            // skip(1) leaves off the initial MoveTo, which is unnecessary
            // because this path starts where the last one ended.
            let mut elems = oriented_path.iter().skip(1);
            if let (Some(our_t), Some(unfinished_t)) =
                (self.can_merge(seg.other_half(), positions), unfinished)
            {
                // Reconstruct part of the input curve
                let input = self.segments.input_segs[self.orig_seg[seg.idx]];
                let sub_input = if our_t > unfinished_t {
                    input.subsegment(unfinished_t..our_t)
                } else {
                    input
                        .reverse()
                        .subsegment((1.0 - unfinished_t)..(1.0 - our_t))
                };
                // Maybe we should "fix up" the endpoints of `sub_input` to agree with
                // what was in `positions`? It shouldn't matter too much, though: they
                // should be close anyway, and any errors won't cause gaps in the path
                // because we skip the MoveTo.
                //
                // unwrap: if unfinished had something, the path must have been non-empty.
                *ret.elements_mut().last_mut().unwrap() = sub_input.as_path_el();
                // Skip the first element in this output segment, because we merged it
                // to the previous output segment.
                elems.next();

                // If we could merge the beginning of this output segment *and*
                // the end, it could be part of a longer merge. Therefore, we
                // don't update `unfinished`.
                if self.can_merge(*seg, positions).is_none() {
                    unfinished = None;
                }
            } else if let Some(t) = self.can_merge(*seg, positions) {
                unfinished = Some(t);
            } else {
                unfinished = None;
            }
            ret.extend(elems);
        }

        ret.close_path();
        ret
    }

    /// Is this intersection point a simple one, where at most two output segments meet?
    fn is_simple_point(&self, idx: HalfOutputSegIdx) -> bool {
        self.point_neighbors[idx].clockwise == self.point_neighbors[idx].counter_clockwise
    }

    /// Checks whether this output segment can merge with its neighbor.
    ///
    /// Imagine you have a single input segment that looks like this:
    ///
    /// ```text
    ///                  /
    ///     /-x-\       /
    ///    /     \     /
    ///   /       \-x-/
    ///  /           
    /// ```
    ///
    /// `Segments` will turn it into three monotonic pieces (by splitting it at
    /// the marked `x`s), but when generating output we try to put the pieces
    /// back together into a single segment. We can't always do this, for example
    /// if there is an intersection with something else:
    ///
    /// ```text
    ///       |
    ///     /-x-\
    ///    /  |  \
    ///   /   |   \
    /// ```
    ///
    /// or a near-intersection that causes some segment to get perturbed.
    ///
    /// This function returns `Some` for a half-output-segment that can be
    /// merged to its neighbor. The value in the `Some` is in the parameter
    /// space of the original input segment, and it says how far that merge
    /// can go. In pictures, suppose that the curved segment goes from `t=0`
    /// at the left to `t=1` at the right.
    ///
    /// ```text
    ///   1 -> <- 2
    ///     /-x-\
    ///    /     \
    ///   /       \
    /// ```
    ///
    /// The `1 -> <- 2` in the diagram assigns names to the two
    /// half-output-segments that meet at the `x`. If you call this function
    /// with for half-output-segment `1`, it will return `Some(0.0)`, because
    /// the output segment ending at `1` can be merged from input parameter
    /// `0.0` all the way to the `x`. If you call this function with
    /// half-output-segment `1`, it will return `Some(1.0)`.
    ///
    /// ```text
    ///   1 -> <- 2
    ///     /-x-\  /
    ///    /     \/
    ///   /      /\
    /// ```
    ///
    /// Now imagine that there is some other intersection preventing
    /// the entire half-output-segment `2` from merging. In this case,
    /// this function will return something like `Some(0.8)`, because the
    /// output segment ending at `2` can be merged with its neighbor
    /// ending at `1`, but not all the way to `1.0`.
    fn can_merge(
        &self,
        seg: HalfOutputSegIdx,
        positions: &OutputSegVec<PositionedOutputSeg>,
    ) -> Option<f64> {
        let pos = &positions[seg.idx];
        let orig_idx = self.orig_seg[seg.idx];

        // We're currently using y positions on the output segment to figure out
        if self.segments[orig_idx].is_horizontal() {
            return None;
        }

        let extremal_seg_is_copied = if seg.is_first_half() {
            pos.copied_idx == Some(0)
        } else {
            // "- 2" because copied_idx counts segments, not elements
            pos.copied_idx == Some(pos.path.elements().len() - 2)
        };

        // Is the curve from `seg` to `seg.other_half()` oriented the same way
        // as the input segment? There are two possible changes of orientation:
        // the segment might have had its orientation switched from the input
        // segment, and our current contour direction might not agree with the
        // output segment's orientation.
        let has_orig_orientation =
            seg.is_first_half() == self.segments.positively_oriented(orig_idx);

        let start_y = self.point(seg).y;
        let end_y = self.point(seg.other_half()).y;
        let split_from_pred = self.segments.split_from_predecessor[orig_idx];
        let can_merge = if has_orig_orientation {
            let seg_in_input_t = split_from_pred.0;
            if (seg_in_input_t > 0.0)
                    && extremal_seg_is_copied
                    // Is this out seg the "first" one of the input seg?
                    && start_y == self.segments.oriented_start(orig_idx).y
            {
                let t = solve_t_for_y(self.segments[orig_idx].to_kurbo_cubic(), end_y);
                Some(self.input_seg_t(seg.idx, t))
            } else {
                None
            }
        } else {
            let seg_in_input_t = split_from_pred.1;
            if seg_in_input_t < 1.0
                && extremal_seg_is_copied
                && start_y == self.segments.oriented_end(orig_idx).y
            {
                let t = solve_t_for_y(self.segments[orig_idx].to_kurbo_cubic(), end_y);
                Some(self.input_seg_t(seg.idx, t))
            } else {
                None
            }
        };
        if self.is_simple_point(seg) {
            can_merge
        } else {
            None
        }
    }

    fn input_seg_t(&self, out_seg: OutputSegIdx, t: f64) -> f64 {
        let orig_idx = self.orig_seg[out_seg];
        let split_from_pred = self.segments.split_from_predecessor[orig_idx];
        let ret = if self.segments.positively_oriented(orig_idx) {
            split_from_pred.0 + t * (split_from_pred.1 - split_from_pred.0)
        } else {
            split_from_pred.1 - t * (split_from_pred.1 - split_from_pred.0)
        };
        ret.clamp(0.0, 1.0)
    }

    /// Returns the contours of some set defined by this topology.
    ///
    /// The callback function `inside` takes a winding number and returns `true`
    /// if a point with that winding number should be in the resulting set. For example,
    /// to compute a boolean "and" using the non-zero winding rule, `inside` should be
    /// `|w| w.shape_a != 0 && w.shape_b != 0`.
    pub fn contours(&self, inside: impl Fn(&W) -> bool) -> Contours {
	let positions = self.compute_positions();
        self.contours_for_positions(inside, positions)
    }

    fn contours_for_positions(
        &self,
        inside: impl Fn(&W) -> bool,
        positions: OutputSegVec<PositionedOutputSeg>,
    ) -> Contours {
        // We walk contours in sweep-line order of their smallest point. This mostly ensures
        // that we visit outer contours before we visit their children. However, when the inner
        // and outer contours share a point, we run into a problem. For example:
        //
        // /---------o--------\
        // |        / \       |
        // |       /   \      |
        // \       \   /     /
        //  \       \_/     /
        //   \             /
        //    -------------
        // (where the top-middle point is supposed to have 4 segments coming out of it; it's
        // a hard to draw it in ASCII). In this case, we'll "notice" the inner contour when
        // we realize that we've visited a point twice. At that point, we extract the inner part
        // into a separate contour and mark it as a child of the outer one. This requires some
        // slightly sketch indexing, because we need to refer to the outer contour even though
        // we haven't finished generating it. We solve this by reserving a slot for the unfinished
        // outer contour as soon as we start walking it.
        //
        // You might worry, as I did, that this construction can be nested. In fact, it doesn't
        // matter if it is because we always walk the contours with the set to our right. So if
        // our inner loop above had a further inner loop (say, attached to its parent at the
        // bottom of the parent) then our initial contour walk won't visit that innermost loop. It
        // will get picked up on a subsequent walk, by which point its parent will already be
        // constructed and have an index.
        let mut ret = Contours::default();
        let mut seg_contour: Vec<Option<ContourIdx>> = vec![None; self.winding.inner.len()];

        let bdy = |idx: OutputSegIdx| -> bool {
            inside(&self.winding[idx].clockwise) != inside(&self.winding[idx].counter_clockwise)
        };

        let mut visited = vec![false; self.winding.inner.len()];
        // Keep track of the points that were visited on this walk, so that if we re-visit a
        // point we can split out an additional contour.
        let mut last_visit = PointVec::with_capacity(self.points.inner.len());
        last_visit.inner.resize(self.points.inner.len(), None);
        for idx in self.segment_indices() {
            if visited[idx.0] {
                continue;
            }

            if !bdy(idx) {
                continue;
            }

            // We found a boundary segment. Let's start by scanning west to figure out where we
            // are relative to existing contours.
            let contour_idx = ContourIdx(ret.contours.len());
            let mut contour = Contour::default();
            let mut west_seg = self.scan_west[idx];
            while let Some(left) = west_seg {
                if self.deleted[left] || !bdy(left) {
                    west_seg = self.scan_west[left];
                } else {
                    break;
                }
            }
            if let Some(west) = west_seg {
                if let Some(west_contour) = seg_contour[west.0] {
                    // Is the thing just to our west inside or outside the output set?
                    let outside = !inside(self.winding_counter_clockwise(west.first_half()));
                    if outside == ret.contours[west_contour.0].outer {
                        // They're an outer contour, and there's exterior between us and them,
                        // or they're an inner contour and there's interior between us.
                        // That means they're our sibling.
                        contour.parent = ret.contours[west_contour.0].parent;
                        contour.outer = outside;
                        debug_assert!(outside || contour.parent.is_some());
                    } else {
                        // They're an outer contour, and there's interior between us and them,
                        // or they're an inner contour and there's exterior between us.
                        // That means they're our parent.
                        contour.parent = Some(west_contour);
                        contour.outer = !ret.contours[west_contour.0].outer;
                    }
                } else {
                    panic!("I'm {idx:?}, west is {west:?}. Y u no have contour?");
                }
            };
            // Reserve a space for the unfinished outer contour, as described above.
            ret.contours.push(contour);

            // First, arrange the orientation so that the interior is on our
            // right as we walk.
            let (start, mut next) = if inside(self.winding_clockwise(idx.first_half())) {
                (idx.first_half(), idx.second_half())
            } else {
                (idx.second_half(), idx.first_half())
            };
            // `segs` collects the endpoint of each segment as we walk along it.
            // "endpoint" here means "as we walk the contour;" it could be either a first_half
            // or a second_half as far as `HalfOutputSegIdx` is concerned.
            let mut segs = Vec::new();
            last_visit[self.point_idx[start]] = Some(0);

            debug_assert!(inside(self.winding_clockwise(start)));
            loop {
                visited[next.idx.0] = true;

                debug_assert!(inside(self.winding_counter_clockwise(next)));
                debug_assert!(!inside(self.winding_clockwise(next)));

                // Walk counter-clockwise around the point until we find the next segment
                // that's on the boundary.
                let mut nbr = self.point_neighbors[next].counter_clockwise;
                debug_assert!(inside(self.winding_clockwise(nbr)));
                while inside(self.winding_counter_clockwise(nbr)) {
                    nbr = self.point_neighbors[nbr].counter_clockwise;
                }

                segs.push(next);
                if nbr == start {
                    break;
                }

                let p = self.point_idx[nbr];
                let last_visit_idx = last_visit[p]
                    // We don't clean up `last_visit` when we finish walking a
                    // contour because it could be expensive if there are many
                    // small contours. This means that `last_visit[p]` could be
                    // a false positive from an earlier contours. We handle this
                    // by double-checking that it was actually a time when this
                    // contour visited `p`.
                    .filter(|&idx| idx < segs.len() && self.point_idx[segs[idx].other_half()] == p);
                if let Some(seg_idx) = last_visit_idx {
                    // We repeated a point, meaning that we've found an inner contour. Extract
                    // it and remove it from the current contour.

                    // seg_idx should point to the end of a segment whose start is at p.
                    debug_assert_eq!(self.point_idx[segs[seg_idx].other_half()], p);

                    let loop_contour_idx = ContourIdx(ret.contours.len());
                    for &seg in &segs[seg_idx..] {
                        seg_contour[seg.idx.0] = Some(loop_contour_idx);
                    }
                    ret.contours.push(Contour {
                        path: self.segs_to_path(&segs[seg_idx..], &positions),
                        parent: Some(contour_idx),
                        outer: !ret.contours[contour_idx.0].outer,
                    });
                    segs.truncate(seg_idx);
                    // In principle, we should also be unsetting `last_visit`
                    // for all points in the contour we just removed. I *think*
                    // we don't need to, because it's impossible for the outer
                    // contour to visit any of them anyway. Should check this
                    // more carefully.
                } else {
                    last_visit[p] = Some(segs.len());
                }

                next = nbr.other_half();
            }
            for &seg in &segs {
                seg_contour[seg.idx.0] = Some(contour_idx);
            }
            ret.contours[contour_idx.0].path = self.segs_to_path(&segs, &positions);
        }

        ret
    }

    /// Returns a rectangle bounding all of our segments.
    pub fn bounding_box(&self) -> kurbo::Rect {
        let mut rect = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        for seg in self.segments.segments() {
            rect = rect.union(seg.to_kurbo_cubic().bounding_box());
        }
        rect
    }

    // Doesn't account for deleted segments. I mean, they're still present in the output.
    fn build_scan_line_orders(&self) -> ScanLineOrder {
        let mut west_map: OutputSegVec<Vec<(f64, Option<OutputSegIdx>)>> =
            OutputSegVec::with_size(self.winding.len());
        let mut east_map: OutputSegVec<Vec<(f64, Option<OutputSegIdx>)>> =
            OutputSegVec::with_size(self.winding.len());

        // This slightly funny iteration order ensures that we push everything
        // in increasing y. `scan_west` (resp. east) contains the *first* west
        // (resp east) neighbor of an output segment, so that's the one we
        // should push into the west_map (resp east_map) first.

        // Filter out horizontal segments, because we don't care about them when
        // doing scan line orders (they're present in scan_west because we do
        // care about them when doing topology).
        for idx in self.scan_west.indices().filter(|i| !self.is_horizontal(*i)) {
            let y = self.point(idx.first_half()).y;
            if let Some(west) = self.scan_west[idx] {
                west_map[idx].push((y, Some(west)));
            }
        }

        for idx in self.scan_east.indices() {
            let y = self.point(idx.first_half()).y;
            if let Some(east) = self.scan_east[idx] {
                east_map[idx].push((y, Some(east)));
                // Double-check that we're inserting in order.
                if let Some((last_y, _)) = west_map[east].last() {
                    debug_assert!(y > *last_y);
                }

                west_map[east].push((y, Some(idx)));
            }
        }

        for idx in self.scan_west.indices().filter(|i| !self.is_horizontal(*i)) {
            let y = self.point(idx.first_half()).y;
            if let Some(west) = self.scan_west[idx] {
                // Double-check that we're inserting in order.
                if let Some((last_y, _)) = east_map[west].last() {
                    debug_assert!(y > *last_y);
                }
                east_map[west].push((y, Some(idx)));
            }
        }

        // Account for the scan-line changes that were triggered by segments
        // ending and revealing other things behind them. It would be nice if
        // we could process these in sweep-line order, to avoid the need to sort
        // and dedup...
        for &(y, west, east) in &self.scan_after {
            if let Some(east) = east {
                west_map[east].push((y, west));
            }
            if let Some(west) = west {
                east_map[west].push((y, east));
            }
        }
        for vec in &mut west_map.inner {
            vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            vec.dedup();
        }
        for vec in &mut east_map.inner {
            vec.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap());
            vec.dedup();
        }

        let ret = ScanLineOrder::new(west_map, east_map);
        #[cfg(feature = "slow-asserts")]
        ret.check_invariants(&self.orig_seg, &self.segments);
        ret
    }

    /// Computes paths for all the output segments.
    pub fn compute_positions(&self) -> OutputSegVec<PositionedOutputSeg> {
        match self.correctness {
            Correctness::GoodEnough => self.compute_positions_good_enough(),
            Correctness::ReallyCorrect => self.compute_positions_correct(),
        }
    }

    /// Computes paths for all the output segments.
    ///
    /// TODO: We should allow passing in an "inside" callback and then only do
    /// positioning for the segments that are on the boundary.
    fn compute_positions_good_enough(&self) -> OutputSegVec<PositionedOutputSeg> {
        let mut output = OutputSegVec::with_size(self.orig_seg.len());
        for idx in self.orig_seg.indices() {
            let start = self.points[self.point_idx[idx.first_half()]].to_kurbo();
            let end = self.points[self.point_idx[idx.second_half()]].to_kurbo();

            let orig_idx = self.orig_seg[idx];
            let seg = &self.segments[orig_idx];

            output[idx] = PositionedOutputSeg::from_seg(seg.to_kurbo_seg(), start, end);
        }
        output
    }

    /// Computes paths for all the output segments, with strong correctness guarantees
    /// for the orderings.
    ///
    /// The sweep-line decides on orderings between segments, which are approximately
    /// correct but subject to numerical error. This method perturbs the segments so
    /// that the sweep-line's ordering decisions are exactly correct.
    ///
    /// The main downside of this method (as compared with [`Topology::compute_positions_good_enough`])
    /// is that it may cause many more subdivisions. It's also usually slower.
    fn compute_positions_correct(&self) -> OutputSegVec<PositionedOutputSeg> {
        // TODO: reuse the cache from the sweep-line
        let mut cmp = ComparisonCache::new(self.eps, self.eps / 2.0);
        let mut endpoints = HalfOutputSegVec::with_size(self.orig_seg.len());
        for idx in self.orig_seg.indices() {
            endpoints[idx.first_half()] = self.points[self.point_idx[idx.first_half()]].to_kurbo();
            endpoints[idx.second_half()] =
                self.points[self.point_idx[idx.second_half()]].to_kurbo();
        }

        crate::position::compute_positions(
            &self.segments,
            &self.orig_seg,
            &mut cmp,
            &endpoints,
            &self.build_scan_line_orders(),
            // The sweep-line guarantees that any two segments coming within
            // eps / 2 will get noticed by the sweep-line. That means we
            // can perturb things by eps / 4 without causing any unexpected
            // collisions.
            self.eps / 4.0,
        )
    }

    #[cfg(feature = "slow-asserts")]
    fn check_invariants(&self) {
        // Check that the winding numbers are locally consistent around every point.
        for out_idx in self.segment_indices() {
            if !self.deleted[out_idx] {
                for half in [out_idx.first_half(), out_idx.second_half()] {
                    let cw_nbr = self.point_neighbors[half].clockwise;
                    if self.winding_clockwise(half) != self.winding_counter_clockwise(cw_nbr) {
                        #[cfg(feature = "debug-svg")]
                        {
                            dbg!(self);
                            let svg = self.dump_svg(|_| "black".to_owned());
                            svg::save("out.svg", &svg).unwrap();
                        }
                        dbg!(half, cw_nbr);
                        panic!();
                    }
                }
            }
        }

        // Check the continuity of contours.
        let mut out_segs = SegVec::<Vec<OutputSegIdx>>::with_size(self.segments.len());
        for out_seg in self.winding.indices() {
            out_segs[self.orig_seg[out_seg]].push(out_seg);
        }
        // For each segment, figure out the first and last points of its output-segment-polyline,
        // in that segment's natural orientation.
        let mut realized_endpoints = Vec::new();
        for (in_seg, out_segs) in out_segs.iter_mut() {
            // Sort the out segments so that they're in the same order that the segment will
            // visit them if it's traversed in sweep-line order. This is almost the same as
            // sorting the out segments by sweep-line order, but there's one little wrinkle
            // for horizontal segments: in a situation like this:
            //
            //            /
            //           /
            //    o--o--o
            //   /
            //  /
            //
            // where there are multiple horizontal out segments on the same sweep line, we
            // need to sort those segments relative to one another depending on the direction
            // in which the input segment traverses that sweep line.
            out_segs.sort_by(|&o1, &o2| {
                let p11 = self.point(o1.first_half());
                let p12 = self.point(o1.second_half());
                let p21 = self.point(o2.first_half());
                let p22 = self.point(o2.second_half());
                let horiz1 = p11.y == p12.y;
                let horiz2 = p21.y == p22.y;

                // Compare the y positions first, so that horizontal segments will get sorted
                // after segments coming from above and before segments going down, no matter the x positions.
                let cmp = (p11.y, p12.y).partial_cmp(&(p21.y, p22.y)).unwrap();
                let cmp = cmp.then((p11.x, p12.x).partial_cmp(&(p21.x, p22.x)).unwrap());
                if horiz1 && horiz2 {
                    // We can create multiple horizontal segments for a segment in the same
                    // sweep line, but currently we don't create any backtracking: all the
                    // horizontal segments we create have the same orientation.
                    debug_assert_eq!(self.positively_oriented[o1], self.positively_oriented[o2]);
                    if self.positively_oriented[o1] == self.segments.positively_oriented(in_seg) {
                        cmp
                    } else {
                        cmp.reverse()
                    }
                } else {
                    cmp
                }
            });

            let mut first = None;
            let mut last = None;

            for &out_seg in &*out_segs {
                let same_orientation =
                    self.positively_oriented[out_seg] == self.segments.positively_oriented(in_seg);
                let (first_endpoint, second_endpoint) = if same_orientation {
                    (out_seg.first_half(), out_seg.second_half())
                } else {
                    (out_seg.second_half(), out_seg.first_half())
                };
                if first.is_none() {
                    first = Some(first_endpoint);
                }

                // When walking the output segments in sweep-line order, the last endpoint of
                // the previous one should be the first endpoint of this one.
                if let Some(last) = last {
                    assert_eq!(self.point_idx[first_endpoint], self.point_idx[last]);
                }
                last = Some(second_endpoint);
            }

            let first = first.unwrap();
            let last = last.unwrap();
            let (first, last) = if self.segments.positively_oriented(in_seg) {
                (first, last)
            } else {
                (last, first)
            };
            realized_endpoints.push((self.point_idx[first], self.point_idx[last]));
        }

        for seg in self.segments.indices() {
            if let Some(prev) = self.segments.contour_prev(seg) {
                assert_eq!(realized_endpoints[prev.0].1, realized_endpoints[seg.0].0);
            }
        }
    }

    /// Renders out our state as an svg.
    #[cfg(feature = "debug-svg")]
    pub fn dump_svg(&self, tag_color: impl Fn(W::Tag) -> String) -> svg::Document {
        let mut bbox = Rect::new(
            f64::INFINITY,
            f64::INFINITY,
            f64::NEG_INFINITY,
            f64::NEG_INFINITY,
        );
        let mut document = svg::Document::new();
        let p = |point: Point| (point.x, point.y);

        for seg in self.segment_indices() {
            let p0 = p(*self.point(seg.first_half()));
            let p1 = p(*self.point(seg.second_half()));
            bbox = bbox.union_pt(p0);
            bbox = bbox.union_pt(p1);
        }

        bbox = bbox.inset(10.0);
        let bbox_size = bbox.width().max(bbox.height());
        let stroke_width = 1.0 * bbox_size / 1024.0;
        let point_radius = 1.5 * bbox_size / 1024.0;
        let font_size = 8.0 * bbox_size / 1024.0;

        for seg_idx in self.segments.indices() {
            let mut data = svg::node::element::path::Data::new();
            let seg = self.segments[seg_idx].to_kurbo_cubic();
            let p0 = (seg.p0.x, seg.p0.y);
            data = data.move_to(p0);
            data = data.cubic_curve_to((
                (seg.p1.x, seg.p1.y),
                (seg.p2.x, seg.p2.y),
                (seg.p3.x, seg.p3.y),
            ));
            let color = tag_color(self.tag[seg_idx]);
            let path = svg::node::element::Path::new()
                .set("id", format!("{seg:?}"))
                .set("class", format!("{:?}", seg_idx))
                .set("stroke", color)
                .set("stroke-width", stroke_width)
                .set("stroke-linecap", "round")
                .set("stroke-linejoin", "round")
                .set("opacity", 0.2)
                .set("fill", "none")
                .set("d", data);
            document = document.add(path);
        }

        for seg in self.segment_indices() {
            let mut data = svg::node::element::path::Data::new();
            let p0 = p(*self.point(seg.first_half()));
            let p1 = p(*self.point(seg.second_half()));
            data = data.move_to(p0);
            data = data.line_to(p1);
            let color = tag_color(self.tag[self.orig_seg[seg]]);
            let path = svg::node::element::Path::new()
                .set("id", format!("{seg:?}"))
                .set("class", format!("{:?}", self.orig_seg[seg]))
                .set("stroke", color)
                .set("stroke-width", stroke_width)
                .set("stroke-linecap", "round")
                .set("stroke-linejoin", "round")
                .set("opacity", 0.2)
                .set("fill", "none")
                .set("d", data);
            document = document.add(path);

            let text = svg::node::element::Text::new(format!("{seg:?}",))
                .set("font-size", font_size)
                .set("text-anchor", "middle")
                .set("x", (p0.0 + p1.0) / 2.0)
                .set("y", (p0.1 + p1.1) / 2.0);
            document = document.add(text);
        }

        for p_idx in self.points.indices() {
            let p = self.points[p_idx];
            let c = svg::node::element::Circle::new()
                .set("id", format!("{p_idx:?}"))
                .set("cx", p.x)
                .set("cy", p.y)
                .set("r", point_radius)
                .set("stroke", "none")
                .set("fill", "black");
            document = document.add(c);

            let text = svg::node::element::Text::new(format!("{p_idx:?}",))
                .set("font-size", font_size)
                .set("text-anchor", "left")
                .set("x", p.x)
                .set("y", p.y);
            document = document.add(text);
        }

        document = document.set(
            "viewBox",
            (bbox.min_x(), bbox.min_y(), bbox.width(), bbox.height()),
        );
        document
    }
}

impl Topology<i32> {
    /// Construct a new topology from a single path.
    pub fn from_path(path: &BezPath, eps: f64) -> Result<Self, NonClosedPath> {
        Self::from_paths(std::iter::once((path, ())), eps)
    }
}

impl Topology<BinaryWindingNumber> {
    /// Creates a new `Topology` for two collections of polylines and a given tolerance.
    ///
    /// Each "set" is a collection of sequences of points; each sequence of
    /// points is interpreted as a closed polyline (i.e. the last point will be
    /// connected back to the first one).
    pub fn from_polylines_binary(
        set_a: impl IntoIterator<Item = impl IntoIterator<Item = Point>>,
        set_b: impl IntoIterator<Item = impl IntoIterator<Item = Point>>,
        eps: f64,
    ) -> Self {
        let mut segments = Segments::default();
        let mut shape_a = Vec::new();
        segments.add_closed_polylines(set_a);
        shape_a.resize(segments.len(), OneOfTwo::A);
        segments.add_closed_polylines(set_b);
        shape_a.resize(segments.len(), OneOfTwo::B);
        Self::from_segments(
            segments,
            SegVec::from_vec(shape_a),
            eps,
            Correctness::GoodEnough,
        )
    }

    /// Creates a new `Topology` from two Bézier paths.
    ///
    /// The two Bézier paths represent two different sets for the purpose of boolean set operations.
    pub fn from_paths_binary(
        set_a: &BezPath,
        set_b: &BezPath,
        eps: f64,
    ) -> Result<Self, NonClosedPath> {
        Self::from_paths([(set_a, OneOfTwo::A), (set_b, OneOfTwo::B)], eps)
    }

    /// Checks whether the two input sets to this topology are disjoint.
    ///
    /// If the two input sets intersect at a vertex (but not along an edge),
    /// they are considered disjoint. Also, the disjointness is tested after
    /// some topological simplification, and so it might not "see" thin spikes.
    ///
    /// The result of this method is not guaranteed to agree with the output
    /// of [`Topology::contours`] because there are approximations involved.
    /// For example, we might return `true` even when the contours are just
    /// barely overlapping.
    pub fn is_disjoint(&self, fill_rule: FillRule) -> bool {
        self.is_disjoint_tags(
            |w| fill_rule.is_inside(w.shape_a),
            |w| fill_rule.is_inside(w.shape_b),
        )
    }
}

/// One direction of a `ScanLineOrder`.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Debug)]
struct HalfScanLineOrder {
    inner: OutputSegVec<Vec<(f64, Option<OutputSegIdx>)>>,
}

impl HalfScanLineOrder {
    fn neighbor_after(&self, seg: OutputSegIdx, y: f64) -> Option<OutputSegIdx> {
        // TODO: maybe binary search, if this might get big?
        self.iter(seg)
            .take_while(|(y0, _, _)| *y0 <= y)
            .find(|(_, y1, _)| *y1 > y)
            .and_then(|(_, _, idx)| idx)
    }

    /// Returns an iterator over `(y0, y1, maybe_seg)`.
    ///
    /// Each item in the iterator tells us that `maybe_seg` is `seg`'s neighbor
    /// between heights `y0` and `y1`. If `maybe_seg` is `None`, it means `seg`
    /// has no neighbor between heights `y0` and `y1`. The y intervals in this
    /// iterator are guaranteed to be in increasing order, which each `y0` equal to
    /// the previous `y1`. The last `y1` will be `f64::INFINITY`.
    fn iter(
        &self,
        seg: OutputSegIdx,
    ) -> impl Iterator<Item = (f64, f64, Option<OutputSegIdx>)> + '_ {
        let ends = self.inner[seg]
            .iter()
            .map(|(y0, _)| *y0)
            .skip(1)
            .chain(std::iter::once(f64::INFINITY));
        self.inner[seg]
            .iter()
            .zip(ends)
            .map(|((y0, maybe_seg), y1)| (*y0, y1, *maybe_seg))
    }

    fn close_neighbor_height_after(
        &self,
        seg: OutputSegIdx,
        y: f64,
        orig_seg: &OutputSegVec<SegIdx>,
        segs: &Segments,
        cmp: &mut ComparisonCache,
    ) -> Option<f64> {
        for (_, y1, other_seg) in self.iter(seg) {
            if y1 <= y {
                continue;
            }

            if let Some(other_seg) = other_seg {
                let order = cmp.compare_segments(segs, orig_seg[seg], orig_seg[other_seg]);
                let next_ish = order
                    .iter()
                    .take_while(|(order_y0, _, _)| *order_y0 < y1)
                    .filter(|(_, _, order)| *order == Order::Ish)
                    .find(|(order_y0, _, _)| *order_y0 >= y);
                if let Some((order_y0, _, _)) = next_ish {
                    return Some(order_y0);
                }
            }
        }
        None
    }
}

/// A summary of all the east-west ordering relations between non-horizontal
/// output segments.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Debug)]
pub struct ScanLineOrder {
    /// Each entry is a list of `(y, west_neighbor)`: starting at `y`, `west_neighbor`
    /// is the segment to my west. The `y`s are in increasing order.
    west_map: HalfScanLineOrder,
    /// Each entry is a list of `(y, east_neighbor)`: starting at `y`, `east_neighbor`
    /// is the segment to my east. The `y`s are in increasing order.
    east_map: HalfScanLineOrder,
}

impl ScanLineOrder {
    fn new(
        west: OutputSegVec<Vec<(f64, Option<OutputSegIdx>)>>,
        east: OutputSegVec<Vec<(f64, Option<OutputSegIdx>)>>,
    ) -> Self {
        Self {
            west_map: HalfScanLineOrder { inner: west },
            east_map: HalfScanLineOrder { inner: east },
        }
    }

    /// Returns the neighbor to the west of `seg` just after height `y`.
    pub fn west_neighbor_after(&self, seg: OutputSegIdx, y: f64) -> Option<OutputSegIdx> {
        self.west_map.neighbor_after(seg, y)
    }

    /// Returns the neighbor to the east of `seg` just after height `y`.
    pub fn east_neighbor_after(&self, seg: OutputSegIdx, y: f64) -> Option<OutputSegIdx> {
        self.east_map.neighbor_after(seg, y)
    }

    /// Returns the next height (greater than or equal to `y`) at which `seg` has a close
    /// neighbor to the east.
    pub fn close_east_neighbor_height_after(
        &self,
        seg: OutputSegIdx,
        y: f64,
        orig_seg: &OutputSegVec<SegIdx>,
        segs: &Segments,
        cmp: &mut ComparisonCache,
    ) -> Option<f64> {
        self.east_map
            .close_neighbor_height_after(seg, y, orig_seg, segs, cmp)
    }

    /// Returns the next height (greater than or equal to `y`) at which `seg` has a close
    /// neighbor to the west.
    pub fn close_west_neighbor_height_after(
        &self,
        seg: OutputSegIdx,
        y: f64,
        orig_seg: &OutputSegVec<SegIdx>,
        segs: &Segments,
        cmp: &mut ComparisonCache,
    ) -> Option<f64> {
        self.west_map
            .close_neighbor_height_after(seg, y, orig_seg, segs, cmp)
    }

    #[cfg(feature = "slow-asserts")]
    fn check_invariants(&self, orig_seg: &OutputSegVec<SegIdx>, segs: &Segments) {
        for idx in self.east_map.inner.indices() {
            for &(y, east_idx) in &self.east_map.inner[idx] {
                if let Some(east_idx) = east_idx {
                    let seg = &segs[orig_seg[idx]];
                    let east_seg = &segs[orig_seg[east_idx]];
                    assert!(y >= seg.start().y && y >= east_seg.start().y);
                    assert!(y < seg.end().y && y < east_seg.end().y);
                }
            }
            for &(y, west_idx) in &self.west_map.inner[idx] {
                if let Some(west_idx) = west_idx {
                    let seg = &segs[orig_seg[idx]];
                    let west_seg = &segs[orig_seg[west_idx]];
                    assert!(y >= seg.start().y && y >= west_seg.start().y);
                    assert!(y < seg.end().y && y < west_seg.end().y);
                }
            }
        }
    }
}

/// An index for a [`Contour`] within [`Contours`].
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Copy, Debug, Hash, PartialEq, Eq)]
pub struct ContourIdx(pub usize);

/// A simple, closed path.
///
/// A contour has no repeated points, and its segments do not intersect.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Debug, PartialEq)]
pub struct Contour {
    /// The contour's path, which is simple and closed.
    pub path: BezPath,

    /// A contour can have a parent, so that sets with holes can be represented as nested contours.
    /// For example, the shaded set below:
    ///
    /// ```text
    ///   ----------------------
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxx/\xxxxxxxxx|
    ///   |xxxxxxxx/  \xxxxxxxx|
    ///   |xxxxxxx/    \xxxxxxx|
    ///   |xxxxxxx\    /xxxxxxx|
    ///   |xxxxxxxx\  /xxxxxxxx|
    ///   |xxxxxxxxx\/xxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   ----------------------
    /// ```
    ///
    /// is represented as a square contour with no parent, and a diamond contour with the square
    /// as its parent.
    ///
    /// A contour can share at most one point with its parent. For example, if you translate the
    /// diamond above upwards until it touches the top of the square, it will share that top point
    /// with its parent. You can't make them share two points, though: if you try to translate the
    /// diamond to a corner...
    ///
    /// ```text
    ///   ----------------------
    ///   |xx/\xxxxxxxxxxxxxxxx|
    ///   |x/  \xxxxxxxxxxxxxxx|
    ///   |/    \xxxxxxxxxxxxxx|
    ///   |\    /xxxxxxxxxxxxxx|
    ///   |x\  /xxxxxxxxxxxxxxx|
    ///   |xx\/xxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   ----------------------
    /// ```
    ///
    /// ...then it will be interpreted as two contours without a parent/child relationship:
    ///
    /// ```text
    ///   ----
    ///   |xx/
    ///   |x/
    ///   |/
    /// ```
    ///
    /// and
    ///
    /// ```text
    ///       ------------------
    ///       \xxxxxxxxxxxxxxxx|
    ///        \xxxxxxxxxxxxxxx|
    ///         \xxxxxxxxxxxxxx|
    ///   |\    /xxxxxxxxxxxxxx|
    ///   |x\  /xxxxxxxxxxxxxxx|
    ///   |xx\/xxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   |xxxxxxxxxxxxxxxxxxxx|
    ///   ----------------------
    /// ```
    ///
    pub parent: Option<ContourIdx>,

    /// Whether this contour is "outer" or not. A contour with no parent is "outer", and
    /// then they alternate: a contour is "outer" if and only if its parent isn't.
    ///
    /// As you walk along a contour, the "occupied" part of the set it represents is
    /// on your right. This means that outer contours wind clockwise and inner
    /// contours wind counter-clockwise.
    pub outer: bool,
}

impl Default for Contour {
    fn default() -> Self {
        Self {
            path: BezPath::default(),
            outer: true,
            parent: None,
        }
    }
}

/// A collection of [`Contour`]s, representing a set.
///
/// Can be indexed with a [`ContourIdx`].
///
/// A `Contour` represents a set as a hierarchical collection of closed paths, where
/// each path has the set on its right (in a y-down coordinate system). A very simple
/// set is represented as just a single closed path:
///
/// ```text
///   ╭───>───╮
///   │xxxxxxx│
///   │xxxxxxx│
///   ╰───<───╯
/// ```
///
/// (The `x`s represent the interior of the set, and the arrows show the orientation
/// of the curve.)
///
/// Two disjoint sets are represented as two unrelated closed paths:
///
/// ```text
///   ╭───>───╮
///   │xxxxxxx│
///   │xxxxxxx│  ╭───>───╮
///   ╰───<───╯  │xxxxxxx│
///              │xxxxxxx│
///              ╰───<───╯
/// ```
///
/// The hierarchical structure appears when you have sets with holes: a set with a single
/// hole is represented as a contour (the outer boundary) with a child contour (the inner
/// boundary). Notice how the curves are oriented so that the set is always on the right.
///
/// ```text
///   ╭───>──────╮
///   │xxxxxxxxxx│
///   │xxx╭<─╮xxx│
///   │xxx│  │xxx│
///   │xxx╰─>╯xxx│
///   │xxxxxxxxxx│
///   ╰──────<───╯
/// ```
///
/// A set can have multiple holes (and so a contour can have multiple children),
/// and those holes can contain more parts of the set. So in general, the
/// collection of contours forms a forest.
///
/// Because of the way we organize the contour directions, the set described by
/// our contours has a winding number of 1 and its complement has a winding
/// number of 0. In particular, if you gather up all the contours and put them
/// in an SVG, it won't make a difference whether you fill them with a non-zero
/// or an even-odd fill rule.
#[cfg_attr(test, derive(serde::Serialize))]
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Contours {
    contours: Vec<Contour>,
}

impl Contours {
    /// Returns all of the contour indices, grouped by containment.
    ///
    /// For each of the inner vecs, the first element is an outer contour with
    /// no parent. All of the other contours in that inner vec lie inside that
    /// outer contour.
    pub fn grouped(&self) -> Vec<Vec<ContourIdx>> {
        let mut children = vec![Vec::new(); self.contours.len()];
        let mut top_level = Vec::new();
        for i in 0..self.contours.len() {
            if let Some(parent) = self.contours[i].parent {
                children[parent.0].push(ContourIdx(i));
            } else {
                top_level.push(ContourIdx(i));
            }
        }

        let mut ret = Vec::with_capacity(top_level.len());
        for top in top_level {
            let mut tree = Vec::new();
            fn visit(idx: ContourIdx, children: &[Vec<ContourIdx>], acc: &mut Vec<ContourIdx>) {
                acc.push(idx);
                for &child in &children[idx.0] {
                    visit(child, children, acc);
                }
            }
            visit(top, &children, &mut tree);
            ret.push(tree);
        }

        ret
    }

    /// Returns the number of contours in the collection.
    pub const fn len(&self) -> usize {
        self.contours.len()
    }

    /// Returns `true` if the collection contains no contours.
    pub const fn is_empty(&self) -> bool {
        self.contours.is_empty()
    }

    /// Iterates over all of the contours, ignoring the hierarchical structure.
    ///
    /// For example, if you're creating an SVG path out of all these contours then
    /// you don't need the hierarchical structure: the SVG renderer can figure that
    /// out by itself.
    pub fn contours(&self) -> impl Iterator<Item = &Contour> + '_ {
        self.contours.iter()
    }

    /// Creates an owned iterator over all of the contours, ignoring the hierarchical
    /// structure.
    ///
    /// For example, if you're creating an SVG path out of all these contours then
    /// you don't need the hierarchical structure: the SVG renderer can figure that
    /// out by itself.
    pub fn into_contours(self) -> impl Iterator<Item = Contour> {
        self.contours.into_iter()
    }
}

impl std::ops::Index<ContourIdx> for Contours {
    type Output = Contour;

    fn index(&self, index: ContourIdx) -> &Self::Output {
        &self.contours[index.0]
    }
}

#[cfg(test)]
mod tests {
    use kurbo::BezPath;
    use proptest::prelude::*;

    use crate::{
        FillRule, SegIdx,
        geom::Point,
        order::ComparisonCache,
        perturbation::{
            F64Perturbation, Perturbation, f64_perturbation, perturbation, realize_perturbation,
        },
    };

    use super::{OneOfTwo, Topology};

    fn p(x: f64, y: f64) -> Point {
        Point::new(x, y)
    }

    const EMPTY: [[Point; 0]; 0] = [];

    #[test]
    fn square() {
        let segs = [[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(segs, EMPTY, eps);
        //check_intersections(&top);

        insta::assert_ron_snapshot!(top);
    }

    #[test]
    fn diamond() {
        let segs = [[p(0.0, 0.0), p(1.0, 1.0), p(0.0, 2.0), p(-1.0, 1.0)]];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(segs, EMPTY, eps);
        //check_intersections(&top);

        insta::assert_ron_snapshot!(top);
    }

    #[test]
    fn square_and_diamond() {
        let square = [[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]];
        let diamond = [[p(0.0, 0.0), p(1.0, 1.0), p(0.0, 2.0), p(-1.0, 1.0)]];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(square, diamond, eps);
        //check_intersections(&top);

        insta::assert_ron_snapshot!(top);
    }

    #[test]
    fn square_with_double_back() {
        let segs = [[
            p(0.0, 0.0),
            p(0.5, 0.0),
            p(0.5, 0.5),
            p(0.5, 0.0),
            p(1.0, 0.0),
            p(1.0, 1.0),
            p(0.0, 1.0),
        ]];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(segs, EMPTY, eps);
        //check_intersections(&top);

        insta::assert_ron_snapshot!(top);
    }

    #[test]
    fn nested_squares() {
        let outer = [[p(-2.0, -2.0), p(2.0, -2.0), p(2.0, 2.0), p(-2.0, 2.0)]];
        let inner = [[p(-1.0, -1.0), p(1.0, -1.0), p(1.0, 1.0), p(-1.0, 1.0)]];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(outer, inner, eps);
        let contours = top.contours(|w| (w.shape_a + w.shape_b) % 2 != 0);

        insta::assert_ron_snapshot!((&top, contours, top.build_scan_line_orders()));

        let out_idx = top.orig_seg.indices().collect::<Vec<_>>();
        let orders = top.build_scan_line_orders();
        assert_eq!(orders.west_neighbor_after(out_idx[0], -2.0), None);
        assert_eq!(orders.east_neighbor_after(out_idx[0], -2.2), None);
        assert_eq!(
            orders.east_neighbor_after(out_idx[0], -2.0),
            Some(out_idx[2])
        );
        assert_eq!(
            orders.east_neighbor_after(out_idx[0], -1.5),
            Some(out_idx[2])
        );
        assert_eq!(
            orders.east_neighbor_after(out_idx[0], -1.0),
            Some(out_idx[3])
        );
        assert_eq!(
            orders.east_neighbor_after(out_idx[0], 1.0),
            Some(out_idx[2])
        );
        // Maybe this is a little surprising, but it's what the current implementation does.
        assert_eq!(
            orders.east_neighbor_after(out_idx[0], 2.0),
            Some(out_idx[2])
        );
    }

    #[test]
    fn squares_with_gaps() {
        let mid = [[p(-2.0, -2.0), p(2.0, -2.0), p(2.0, 2.0), p(-2.0, 2.0)]];
        let left_right = [
            [p(-4.0, -1.0), p(-3.0, -1.0), p(-3.0, 1.0), p(-4.0, 1.0)],
            [p(4.0, -1.0), p(3.0, -1.0), p(3.0, -0.5), p(4.0, -0.5)],
            [p(4.0, 1.0), p(3.0, 1.0), p(3.0, 0.5), p(4.0, 0.5)],
        ];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(mid, left_right, eps);
        let contours = top.contours(|w| (w.shape_a + w.shape_b) % 2 != 0);

        insta::assert_ron_snapshot!((&top, contours, top.build_scan_line_orders()));
    }

    #[test]
    fn close_neighbor_height() {
        let big = [[p(0.0, 0.0), p(0.0, 10.0), p(1.0, 10.0), p(1.0, 0.0)]];
        let left = [
            [p(-10.0, 0.5), p(-0.25, 2.0), p(-10.0, 10.0)],
            [p(-10.0, 0.5), p(-0.25, 10.0), p(-10.0, 10.0)],
        ];
        let eps = 0.5;
        let top = Topology::from_polylines_binary(big, left, eps);

        // The output segs coming from the left side of the big square.
        let indices: Vec<_> = top
            .orig_seg
            .indices()
            .filter(|i| top.orig_seg[*i] == SegIdx(0))
            .collect();

        assert_eq!(indices.len(), 2);
        let orders = top.build_scan_line_orders();
        let mut cmp = ComparisonCache::new(eps, eps / 2.0);

        let h = orders
            .close_west_neighbor_height_after(
                indices[0],
                0.0,
                &top.orig_seg,
                &top.segments,
                &mut cmp,
            )
            .unwrap();
        assert!((0.5..=2.0).contains(&h));

        let h = orders
            .close_west_neighbor_height_after(
                indices[0],
                0.6,
                &top.orig_seg,
                &top.segments,
                &mut cmp,
            )
            .unwrap();
        assert!((0.5..=2.0).contains(&h));

        let h = orders
            .close_west_neighbor_height_after(
                indices[1],
                5.0,
                &top.orig_seg,
                &top.segments,
                &mut cmp,
            )
            .unwrap();
        assert!((8.0..=10.0).contains(&h));
    }

    #[test]
    fn inner_loop() {
        let outer = [[p(-2.0, -2.0), p(2.0, -2.0), p(2.0, 2.0), p(-2.0, 2.0)]];
        let inners = [
            [p(-1.5, -1.0), p(0.0, 2.0), p(1.5, -1.0)],
            [p(-0.1, 0.0), p(0.0, 2.0), p(0.1, 0.0)],
        ];
        let eps = 0.01;
        let top = Topology::from_polylines_binary(outer, inners, eps);
        let contours = top.contours(|w| (w.shape_a + w.shape_b) % 2 != 0);

        insta::assert_ron_snapshot!((top, contours));
    }

    #[test]
    fn merge_monotonic_segs() {
        let mut bez = BezPath::default();
        bez.move_to((-1., 0.));
        // An "N-shaped" curve, so it divides into 3 monotonic pieces.
        bez.curve_to((-1., -1.), (1., 1.), (1., 0.));
        // Finish off the shape in a way that doesn't cause any intersections.
        bez.line_to((2., 4.));
        bez.line_to((-2., 4.));
        bez.close_path();

        let top = Topology::from_path(&bez, 1e-6).unwrap();
        let contours = top.contours(|w| *w != 0);

        insta::assert_ron_snapshot!((top, contours));
    }

    #[test]
    fn merge_monotonic_segs_with_cut() {
        let mut bez = BezPath::default();
        bez.move_to((-1., 0.));
        // An "N-shaped" curve, so it divides into 3 monotonic pieces.
        bez.curve_to((-1., -1.), (1., 1.), (1., 0.));
        // Finish off the shape in a way that cuts through the middle.
        bez.close_path();

        let top = Topology::from_path(&bez, 1e-6).unwrap();
        let contours = top.contours(|w| *w != 0);

        dbg!(&top.segments.split_from_predecessor);
        insta::assert_ron_snapshot!((top, contours));
    }

    #[test]
    fn disjoint() {
        let s1 = [[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]];
        let s2 = [[p(2.0, 0.0), p(3.0, 0.0), p(3.0, 1.0), p(2.0, 1.0)]];
        assert!(Topology::from_polylines_binary(s1, s2, 1e-5).is_disjoint(FillRule::EvenOdd));

        // These touch at a corner: still disjoint.
        let s2 = [[p(1.0, 0.0), p(3.0, 0.0), p(3.0, 1.0), p(2.0, 1.0)]];
        assert!(Topology::from_polylines_binary(s1, s2, 1e-5).is_disjoint(FillRule::EvenOdd));

        // These touch on an edge: not disjoint.
        let s2 = [[p(1.0, 0.0), p(3.0, 0.0), p(3.0, 1.0), p(1.0, 1.0)]];
        assert!(!Topology::from_polylines_binary(s1, s2, 1e-5).is_disjoint(FillRule::EvenOdd));

        // These overlap fully.
        let s2 = [[p(0.5, 0.5), p(1.5, 0.5), p(1.5, 1.5), p(0.5, 1.5)]];
        assert!(!Topology::from_polylines_binary(s1, s2, 1e-5).is_disjoint(FillRule::EvenOdd));

        // The second set is repeated, so according to the EvenOdd fill rule it's empty.
        // Note that we make the second set completely contain the first for this test,
        // because we don't really specify what happens exactly at the doubled-up lines.
        let s2 = [
            [p(-0.5, -0.5), p(1.5, -0.5), p(1.5, 1.5), p(-0.5, 1.5)],
            [p(-0.5, -0.5), p(1.5, -0.5), p(1.5, 1.5), p(-0.5, 1.5)],
        ];
        assert!(Topology::from_polylines_binary(s1, s2, 1e-5).is_disjoint(FillRule::EvenOdd));
    }

    #[test]
    fn normal_contours() {
        let s = [[p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)]];
        assert!(Topology::from_polylines_binary(s, EMPTY, 1e-5).has_normal_contours(OneOfTwo::A));

        // This one goes the wrong way around.
        let s = [[p(0.0, 0.0), p(0.0, 1.0), p(1.0, 1.0), p(1.0, 0.0)]];
        assert!(!Topology::from_polylines_binary(s, EMPTY, 1e-5).has_normal_contours(OneOfTwo::A));

        let s = [
            [p(0.0, 0.0), p(4.0, 0.0), p(4.0, 4.0), p(0.0, 4.0)],
            [p(1.0, 1.0), p(1.0, 2.0), p(2.0, 2.0), p(2.0, 1.0)],
        ];
        assert!(Topology::from_polylines_binary(s, EMPTY, 1e-5).has_normal_contours(OneOfTwo::A));

        // The inner contour goes the wrong way
        let s = [
            [p(0.0, 0.0), p(0.0, 4.0), p(4.0, 4.0), p(4.0, 0.0)],
            [p(1.0, 1.0), p(2.0, 1.0), p(2.0, 2.0), p(1.0, 2.0)],
        ];
        assert!(!Topology::from_polylines_binary(s, EMPTY, 1e-5).has_normal_contours(OneOfTwo::A));

        // Overlapping squares
        let s = [
            [p(0.0, 0.0), p(1.0, 0.0), p(1.0, 1.0), p(0.0, 1.0)],
            [p(0.5, 0.5), p(1.5, 0.5), p(1.5, 1.5), p(0.5, 1.5)],
        ];
        assert!(!Topology::from_polylines_binary(s, EMPTY, 1e-5).has_normal_contours(OneOfTwo::A));
    }

    #[test]
    fn invalid_sweep_order_repro() {
        let polys = [
            vec![p(1.0, 0.33681665819045015), p(1.0, -1.0)],
            vec![p(2.0, 0.0), p(0.9109561480029991, 1.000000000000005)],
            vec![
                p(1.0, 1.0),
                p(1.0, -1.0),
                p(1.0877400346065769, 1.0000000000000102),
                p(1.0, -1.0),
            ],
        ];
        let _top = Topology::from_polylines_binary(polys, EMPTY, 0.1);
    }

    #[test]
    fn nested_loop_heirarchy() {
        // For a doubly-nested loop like this:
        //            / \
        //           /   \
        //          /     \
        //         /       \
        //        /         \
        //       /           \
        //      /             \
        //     /   ________    \
        //    /    \  /\  /     \
        //   /      \ └┘ /       \
        //  /        \  /         \
        // /          \/           \
        // -------------------------
        // Our contour-walking (which always produces simple curves) is supposed
        // to create three nested contours.
        let mut outer_triangle = BezPath::new();
        outer_triangle.move_to((0.0, -100.0));
        outer_triangle.line_to((100.0, 0.0));
        outer_triangle.line_to((-100.0, 0.0));
        outer_triangle.close_path();

        let mut mid_triangle = outer_triangle;
        mid_triangle.move_to((0.0, 0.0));
        mid_triangle.line_to((10.0, -10.0));
        mid_triangle.line_to((-10.0, -10.0));
        mid_triangle.close_path();

        let mut inner_triangle = mid_triangle.clone();
        inner_triangle.move_to((0.0, -10.0));
        inner_triangle.line_to((1.0, -9.0));
        inner_triangle.line_to((-1.0, -9.0));
        inner_triangle.close_path();

        let top = Topology::from_path(&inner_triangle, 1e-6).unwrap();
        let contours = top.contours(|&w| w % 2 != 0);
        let contours = contours.contours().collect::<Vec<_>>();
        assert_eq!(contours.len(), 3);
        assert_eq!(contours[1].parent.unwrap().0, 0);
        assert!(!contours[1].outer);
        assert_eq!(contours[2].parent.unwrap().0, 1);
        assert!(contours[2].outer);

        // If we flip the innermost loop, it gets parented to the outermost.
        //            / \
        //           /   \
        //          /     \
        //         /       \
        //        /         \
        //       /           \
        //      /             \
        //     /   ________    \
        //    /    \  /\  /     \
        //   /      \ └┘ /       \
        //  /        \  /         \
        // /          \/           \
        // -------------------------
        // Our contour-walking (which always produces simple curves) is supposed
        // to create three nested contours.
        let mut flipped_inner_triangle = mid_triangle;
        flipped_inner_triangle.move_to((0.0, -10.0));
        flipped_inner_triangle.line_to((1.0, -11.0));
        flipped_inner_triangle.line_to((-1.0, -11.0));
        flipped_inner_triangle.close_path();
        let top = Topology::from_path(&flipped_inner_triangle, 1e-6).unwrap();
        let contours = top.contours(|&w| w % 2 != 0);
        let contours = contours.contours().collect::<Vec<_>>();
        assert_eq!(contours.len(), 3);
        assert_eq!(contours[1].parent.unwrap().0, 0);
        assert!(!contours[1].outer);
        assert_eq!(contours[2].parent.unwrap().0, 0);
        assert!(!contours[2].outer);
    }

    // Checks that all output segments intersect one another only at endpoints.
    // fn check_intersections(top: &Topology) {
    //     for i in 0..top.winding.inner.len() {
    //         for j in (i + 1)..top.winding.inner.len() {
    //             let i = OutputSegIdx(i);
    //             let j = OutputSegIdx(j);

    //             let p0 = top.point(i.first_half()).clone();
    //             let p1 = top.point(i.second_half()).clone();
    //             let q0 = top.point(j.first_half()).clone();
    //             let q1 = top.point(j.second_half()).clone();

    //             let s = Segment::new(p0.clone().min(p1.clone()), p1.max(p0)).to_exact();
    //             let t = Segment::new(q0.clone().min(q1.clone()), q1.max(q0)).to_exact();

    //             if s.end.y >= t.start.y && t.end.y >= s.start.y {
    //                 // FIXME
    //                 // if let Some(y) = s.exact_intersection_y(&t) {
    //                 //     dbg!(y);
    //                 //     assert!(
    //                 //         s.start == t.start
    //                 //             || s.start == t.end
    //                 //             || s.end == t.start
    //                 //             || s.end == t.end
    //                 //     );
    //                 // }
    //             }
    //         }
    //     }
    // }

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
        let eps = 0.1;
        let _top = Topology::from_polylines_binary(perturbed_polylines, EMPTY, eps);
        //check_intersections(&top);
    }

    proptest! {
    #[test]
    fn perturbation_test_f64(perturbations in prop::collection::vec(perturbation(f64_perturbation(0.1)), 1..5)) {
        run_perturbation(perturbations);
    }
    }
}
