// Written for craft-pathops. MIT OR Apache-2.0.

//! A metered dry sweep: the backstop under the pricing in `budget.rs`.
//!
//! The pricing counts what a sweep will cost before it runs, from the geometry, and four rounds of review
//! (and an automated scan of our own commit) have found shapes it does not see: copies a few tolerances
//! apart, curves that run along one another for a tenth of their length, pencils of lines whose crossings
//! lie a few tolerances apart. This module does not look at the geometry. It runs the sweep itself, step by
//! step through the public interface of `linesweeper` (the same segments, the same tolerance, the same
//! loop over sweep lines and their ranges that `Topology::from_paths` runs, without what it builds), counts
//! the work the sweep makes in the units below, and stops, refusing the input, as soon as the count passes
//! the budget ([`crate::Limits::sweep_work`]).
//!
//! The sweep is run only for inputs of more than [`GATE_PIECES`] monotone pieces: the sweep's cost
//! cannot be hidden from a count of the pieces it is given, and under the gate no input costs more than
//! about half a second (the tests build the worst they know of and time it). Every input past the gate pays
//! for the dry sweep, which costs a little less than the sweep it guards.
//!
//! # The units
//!
//! The unit is the nanosecond of sweep that a count is worth on the machine the weights were measured on (a release build, one thread),
//! so that the budget reads as a time. The work is the greater of two counts, the time the sweep will take and the memory the topology
//! will hold (also in nanoseconds: a piece of the topology costs [`PIECE_MEMORY`], 1.8 microseconds, which makes the budget 2.3 million pieces, 0.9 to 1.6 GB
//! of them). The time is a sum of terms, each priced for the kind of input that makes it large:
//!
//! - **Output pieces** ([`PIECE_WORK`] each, and [`CURVED_WORK`] more for a piece of a curve): every piece of a segment between two of the
//!   points the sweep makes, and every horizontal fragment. These are the entries of the topology.
//! - **Scan** ([`SCAN_PICOS`] for each piece and each segment on the sweep line when it is made): the sweep scans the line for the place
//!   of a piece, and where segments cross each scan reads a good part of it: the star of 1,291 points, a hatch of 400 strokes and a bundle
//!   of 1,500 lines that cross along their length all take a nanosecond for each. The segments on the line are those that span its height.
//! - **Tangle** ([`TANGLE_PICOS`] a piece times the square of the cluster it is made in): where many segments lie within a few tolerances of one
//!   another the sweep takes time for every pair of them. For each range of the sweep line (the segments that need a position at one height) the
//!   cluster is the most of its segments that lie within eight tolerances of one another across, and each piece the range makes counts the square.
//! - **Ends at one place**, counted from the segments before the sweep (what the sweep does at a height, it does in one step, which no count made
//!   as it goes can stop in time: a stack of copies of one shape is a dozen steps and nine seconds): [`GROUP_PICOS`] for the fourth power of the
//!   segments that start at one height within four tolerances of one another across (or end at one), and [`BATCH_WORK`] for the square of those that
//!   start at one height wherever they are across (a row of dots on one line, whose shapes put their pieces into the line at once).
//! - **Steps of comparisons** ([`COMPARE_NODE_WORK`] each, less what a curved piece is paid for, with [`SAMPLE_WORK`] for each reading of the gap
//!   and [`LOOKUP_WORK`] for each place of the line read to find the neighbours): the sweep compares two cubics by halving the stretch of height it
//!   looks at until the error of its approximations is under the gap between them, so that curves that run along one another for most of their
//!   length (copies moved a few hundredths) take thousands of steps, which none of the terms above sees. The steps are estimated from the third
//!   differences of the curves and the gaps between them for every two segments that are next to one another on the sweep line, found before the
//!   sweep (the segments that start and end at one height) and as it goes (every range of a line: the segments of it, and those beside them
//!   that its curves came in beside, crossed, or left between).
//!
//! The terms are weights fitted to inputs measured on one machine (the README gives the fit and where it is loose), and the budget is set so that
//! the most the meter admits takes about as long as the star of 1,291 points.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};

use kurbo::{BezPath, PathEl};
use linesweeper::sweep::{SegmentsConnectedAtX, SweepLine, SweepLineBuffers, SweepLineRangeBuffers, Sweeper};
use linesweeper::{SegIdx, Segment, Segments};

use crate::PathOpsError;
use crate::budget::limits;
use crate::work::{self, Work};

/// Monotone pieces an input may have and not be metered. The sweep of a few more than 190 pieces can still
/// take about half a second (a hundred copies of a segment one on another, 150 lines out of one point down a
/// diagonal), and none of the inputs the tests build under it takes more.
pub(crate) const GATE_PIECES: usize = 192;

/// What an output piece is worth in sweep time, in nanoseconds.
pub(crate) const PIECE_WORK: u64 = 600;

/// What an output piece is worth in memory, in the nanoseconds the budget is counted in: the topology holds 400 to 700 bytes for each, so
/// that the pieces a budget admits, at 1.8 microseconds each, 2.3 million, are 0.9 to 1.6 gigabytes (the star holds 1.96 million, 0.9 GB; 50
/// stars stacked 2.3 million, 1.6 GB).
pub(crate) const PIECE_MEMORY: u64 = 1_800;

/// What a pair of segments that start or end together is worth in memory, in the nanoseconds the budget is counted in, added to the
/// pieces': the sweep keeps the order of each pair it compares in a cache that it never clears, about 650 bytes for each once the table
/// has grown to hold them, so that the pairs a budget admits, at 1.1 microseconds each, 3.8 million, are 2.4 GB (and a piece, at 1.8
/// microseconds, 1.3 kilobytes: the same budget is 3 GB of either).
pub(crate) const CACHE_MEMORY: u64 = 1_100;

/// What a pair of segments that start or end together is worth against the budget alone, in nanoseconds: the cache's tables grow by
/// doubling, and what did not fit under a limit of 3 GB was the next table of the second (conservative) cache, 877 MB at about 1.8
/// million entries, which for a pie holds about 0.49 of the pairs counted. Pies ran with 3.68 million pairs counted (2,150 of 30
/// slices) and 3.71 million (5,000 of 20) and aborted at 2.37 GB with 3.76 million (2,200 of 30) and 3.85 million (5,200 of 20); the pies
/// of 60 and of 100 slices, whose second cache holds less, run on to 4.5 million pairs but need 2.9 GB from 3.7 million. The budget
/// admits no more than 3.56 million pairs, whatever else the sweep holds: 4% under the cliff, for pies.
pub(crate) const PAIR_WORK: u64 = 1_180;

/// What a piece of a curve is worth beyond that, in nanoseconds: the pieces of near-identical closed loops of one cubic each cost
/// four times a line's.
pub(crate) const CURVED_WORK: u64 = 4_000;

/// What a piece made where `n` segments span the sweep line is worth, for each `n`, in picoseconds: the sweep scans the line for the
/// place of a piece, and where segments cross one another each scan reads a good part of it (the star of 1,291 points, a hatch of 400
/// strokes, a bundle of 1,500 lines that cross along their length all take a nanosecond for each segment on the line, for each piece).
pub(crate) const SCAN_PICOS: u64 = 1_050;

/// What a piece made in a cluster of `k` segments is worth, for each `k` squared, in picoseconds.
pub(crate) const TANGLE_PICOS: u64 = 230;

/// What the fourth power of the segments that start (or end) together is worth, in picoseconds.
pub(crate) const GROUP_PICOS: u64 = 70;

/// What the square of the segments that start (or end) at one height is worth, in nanoseconds.
pub(crate) const BATCH_WORK: u64 = 3;

/// What a step of the comparison of two cubics is worth in sweep time, in nanoseconds: the comparison of two curves halves the stretch of
/// height it looks at until the bounds of the gap between them are tighter than the tolerance, and a step is one such stretch (about 0.56
/// microsecond measured, with the cache and the bookkeeping around it 575 estimated).
pub(crate) const COMPARE_NODE_WORK: u64 = 575;

/// The steps of comparison that each curved piece has been paid for already, in [`CURVED_WORK`]: the steps of all the pairs, less this for each curved
/// piece, are charged (so that a curved piece is never worth less than the pieces' own weights make it, whatever the pairs it is in).
pub(crate) const FREE_NODES: u64 = CURVED_WORK / COMPARE_NODE_WORK;

/// The steps of the comparison of two cubics for each cube root of the third differences of the pair over their gap: fitted to circles,
/// ellipses (turned and not), and blobs of arcs, whose pairs the sweep compares in 1.0, 1.0 and 0.9 of the steps this counts (a turned ellipse
/// in 1.7 of them), at gaps of 3 parts in a hundred thousand of their size to 1 in 300 million.
const NODES_PER_ROOT: f64 = 7.0;

/// The heights, evenly across the height two curves share, at which the gap between them is read: the steps of their comparison are taken from
/// the mean of what the gap at each makes of them (an integral over the shared height, by the midpoint rule: where the curves are close for a
/// stretch of it, and far elsewhere, the stretch is charged for its share of the height; where they cross, the gap is no more than the
/// tolerance at the one place, and that place is charged for its width). A stretch of less than a thirty-second of the height can fall between
/// two of the readings.
const GAP_SAMPLES: usize = 32;

/// The gap, in tolerances, under which two curves that are so all along are taken as one and their comparison as no steps: it stopped at
/// once (5 to 8 milliseconds for 64 pairs of circles of a million, 0.9 tolerance apart or none), and past it took 2.2 seconds, at one tolerance
/// and a twentieth to three (0.87 at one exactly); the cap is a tenth under the one tolerance, for the rounding of the readings.
const COINCIDENT: f64 = 0.9;

/// The least gap, in tolerances, that the steps of a comparison are taken from: the cube root of the third differences over the gap is 1.5 to 4
/// times the steps the sweep took at one to ten tolerances (64 pairs of circles of a million: 2.2 seconds at one to three and 3.1 at seven, 7.2 to
/// 4.8 by the cube root), which covers what is not counted (curves in a crowd are put in order by comparisons of segments that are not next
/// to one another, charged below at the places a power of two from one another, and the gaps between samples).
const GAP_FLOOR: f64 = 1.0;

/// The readings of the gap that are made first, evenly across the shared height, and the steps (a pair that has the nearest of the two as its gap
/// takes no more than these is not counted, and is read no further).
const COARSE_SAMPLES: usize = 2;

/// See [`COARSE_SAMPLES`]: more than a pair of the two sides of a stroke is estimated at (27), and than the circles of a hundred in radius, one apart,
/// are (29 a pair of arcs, 14 microseconds by the sweep's count: the 52,000 pairs of them that the element limit admits are 4.9 seconds of sweep, and
/// the weights of the pieces alone count 2.1), and less than the circles of a thousand, one apart and three, are (62 and 43).
const COARSE_NODES: u64 = 32;

/// More segments than this in a window of the line are a crowd, whose members are put in order by comparisons that are not all between neighbours.
const CROWD: usize = 4;

/// More than this many in a window are not a crowd within the tolerance (the terms of the sweep refuse those long before: the fourth power of what starts
/// together, the square of a cluster), but what a range that a level segment crosses holds, far apart across: they are not charged with those a power
/// of two from them, which would take them as long as the sweep takes for the range, and nothing of it.
const CROWD_MAX: usize = 4_096;

/// What one reading of the gap is worth in sweep time, in nanoseconds: two places of a curve across at one height and the root of the quotient.
pub(crate) const SAMPLE_WORK: u64 = 100;

/// What reading one place of the sweep line in a search for a segment is worth, in nanoseconds (the place of a curve across at the height of the
/// line, and a comparison of it).
pub(crate) const LOOKUP_WORK: u64 = 40;

/// How far apart, in tolerances of the sweep, segments may lie across and be one cluster.
const CLUSTER_WIDTH: f64 = 8.0;

/// Points in a window of one cell that are too few to count by cell: their fourth powers are bounded by the cube of the most there are.
const FEW_IN_A_CELL: u64 = 6;

/// How far apart, in tolerances, segments that start (or end) may lie, across and up, and start (or end) together.
const GROUP_WIDTH: f64 = 4.0;

/// What a dry sweep counted.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Counts {
    /// Monotone pieces of the input.
    pub(crate) segments: u64,
    /// Pieces of segments the sweep made between its points, and its horizontal fragments: the pieces the topology holds.
    pub(crate) pieces: u64,
    /// Of those, the pieces of curves.
    pub(crate) curved: u64,
    /// Heights the sweep stopped at.
    pub(crate) steps: u64,
    /// Ranges of the sweep line that were given positions.
    pub(crate) ranges: u64,
    /// The sum over ranges of the pieces they make times the square of their cluster.
    pub(crate) tangle: u64,
    /// The sum over ranges of the pieces they make times the segments that span the sweep line at their height.
    pub(crate) scan: u64,
    /// The sum of the fourth powers of the segments that start together, and of those that end together.
    pub(crate) groups: u64,
    /// The sum of the squares of the segments that start at one height, and of those that end at one.
    pub(crate) batch: u64,
    /// The pairs of segments that start or end together: the entries the sweep makes in its cache of comparisons.
    pub(crate) pairs: u64,
    /// The steps of the comparisons of curves that lie close to one another (of the pairs that can pass what a curved piece is paid for): the time
    /// takes off the [`FREE_NODES`] of each curved piece, which [`CURVED_WORK`] has paid for already.
    pub(crate) compared: u64,
    /// The readings of the gap between two curves that the steps were estimated from.
    pub(crate) sampled: u64,
    /// The places of the sweep line read in the searches for the neighbours of a range.
    pub(crate) probed: u64,
}

impl Counts {
    /// The time of sweep these counts come to, in nanoseconds.
    pub(crate) fn time(&self) -> u64 {
        self.pieces
            .saturating_mul(PIECE_WORK)
            .saturating_add(self.curved.saturating_mul(CURVED_WORK))
            .saturating_add(self.scan.saturating_mul(SCAN_PICOS) / 1000)
            .saturating_add(self.tangle.saturating_mul(TANGLE_PICOS) / 1000)
            .saturating_add(self.groups.saturating_mul(GROUP_PICOS) / 1000)
            .saturating_add(self.batch.saturating_mul(BATCH_WORK))
            .saturating_add(self.compared.saturating_sub(self.curved.saturating_mul(FREE_NODES)).saturating_mul(COMPARE_NODE_WORK))
            .saturating_add(self.sampled.saturating_mul(SAMPLE_WORK))
            .saturating_add(self.probed.saturating_mul(LOOKUP_WORK))
    }

    /// The memory the sweep would hold, in the nanoseconds the budget is counted in: the topology of the pieces, and the cache of
    /// comparisons for the pairs of segments that start or end together.
    pub(crate) fn memory(&self) -> u64 {
        self.pieces.saturating_mul(PIECE_MEMORY).saturating_add(self.pairs.saturating_mul(CACHE_MEMORY))
    }

    /// The work these counts come to: the greatest of the time, the memory, and the pairs against the budget alone, each in nanoseconds, so
    /// that one budget holds all three.
    pub(crate) fn work(&self) -> u64 {
        self.time().max(self.memory()).max(self.pairs.saturating_mul(PAIR_WORK))
    }
}

/// The segments the sweep of `paths` is made of, as `Topology::from_paths` makes them: the monotone pieces of
/// every path in one arena. `None` where it would refuse them or panic making them (a subpath that does not close, a path
/// that starts with a close): the sweep itself says so, before it sweeps.
pub(crate) fn segments_of<'a>(paths: impl IntoIterator<Item = &'a BezPath>) -> Option<Segments> {
    // The paths go in one path so that the arena sorts its entrances and exits once, not once for each.
    // Each path's subpaths start fresh, as they do in the topology: a path that does not start with a move
    // starts at its first end, as the iteration of its segments takes it to.
    let mut all = BezPath::new();
    for p in paths {
        let els = p.elements();
        match els.first() {
            None | Some(PathEl::MoveTo(_)) => {}
            Some(PathEl::LineTo(q) | PathEl::QuadTo(_, q) | PathEl::CurveTo(_, _, q)) => all.push(PathEl::MoveTo(*q)),
            Some(PathEl::ClosePath) => return None,
        }
        for el in els {
            all.push(*el);
        }
    }
    catch_unwind(AssertUnwindSafe(|| {
        let mut segments = Segments::default();
        segments.add_bez_path(&all).ok().map(|()| segments)
    }))
    .ok()
    .flatten()
}

/// The most of `xs` (sorted in place) that lie within `width` of one another.
fn tightest(xs: &mut [f64], width: f64) -> usize {
    xs.sort_by(f64::total_cmp);
    let (mut best, mut lo) = (0usize, 0usize);
    for hi in 0..xs.len() {
        while xs.get(hi).zip(xs.get(lo)).is_some_and(|(h, l)| h - l > width) {
            lo += 1;
        }
        best = best.max(hi - lo + 1);
    }
    best
}

/// The sum of the fourth powers of the points in each cell of a grid of cells `cell` across, the greatest over the four grids that are
/// the grid and the grid moved half a cell across, up and both: a cluster of points less than half a cell across lies in one cell of one
/// of them.
fn grid_fourth_powers(points: &[(f64, f64)], cell: f64) -> u64 {
    // Most points of any cell lie in one window `cell` wide across, so where no window holds more than a handful (most inputs) the
    // fourth powers are at most the cube of that times the points, and are not made by four passes of a hash map (which are most of what
    // the meter costs before it sweeps, a quarter of a second for 500,000 segments).
    let mut xs: Vec<f64> = points.iter().map(|p| p.0).collect();
    let most = tightest(&mut xs, cell) as u64;
    if most <= FEW_IN_A_CELL {
        return most.saturating_pow(3).saturating_mul(points.len() as u64);
    }
    cells_by_grid(points, cell).map(|cells| cells.values().fold(0u64, |t, &n| t.saturating_add(n.saturating_pow(4)))).max().unwrap_or(0)
}

/// The pairs of `points` that lie in one cell of a grid of cells `cell` across, the greatest over the four grids (as
/// [`grid_fourth_powers`] counts), and where no window `cell` across holds more than a handful the pairs that lie within a cell of one
/// another across and up, which are all of them that any cell holds, counted by a pass of the points in order across.
fn cluster_pairs(points: &[(f64, f64)], cell: f64) -> u64 {
    let mut by_x = points.to_vec();
    by_x.sort_by(|a, b| a.0.total_cmp(&b.0));
    let mut xs: Vec<f64> = by_x.iter().map(|p| p.0).collect();
    if tightest(&mut xs, cell) as u64 <= FEW_IN_A_CELL {
        let (mut pairs, mut lo) = (0u64, 0usize);
        for (hi, &(x, y)) in by_x.iter().enumerate() {
            while by_x.get(lo).is_some_and(|l| x - l.0 > cell) {
                lo += 1;
            }
            let near = by_x.get(lo..hi).unwrap_or_default().iter().filter(|q| (y - q.1).abs() <= cell).count();
            pairs = pairs.saturating_add(near as u64);
        }
        return pairs;
    }
    cells_by_grid(points, cell)
        .map(|cells| cells.values().fold(0u64, |t, &n| t.saturating_add(n.saturating_mul(n.saturating_sub(1)) / 2)))
        .max()
        .unwrap_or(0)
}

/// The count of points in each cell of each of the four grids of cells `cell` across that are the grid and the grid moved half a cell across,
/// up and both.
fn cells_by_grid(points: &[(f64, f64)], cell: f64) -> impl Iterator<Item = HashMap<(i64, i64), u64>> + '_ {
    [(0.0, 0.0), (0.5, 0.0), (0.0, 0.5), (0.5, 0.5)].into_iter().map(move |(dx, dy)| {
        let mut cells: HashMap<(i64, i64), u64> = HashMap::new();
        for &(x, y) in points {
            // A float converted with `as` saturates, and a coordinate this far out is no cluster of any size.
            *cells.entry((((x / cell) + dx).floor() as i64, ((y / cell) + dy).floor() as i64)).or_insert(0) += 1;
        }
        cells
    })
}

/// What the segments' starts and ends come to before the sweep: the sum of the fourth powers of the segments that start together
/// (within `GROUP_WIDTH` tolerances of one another across and up: copies moved a fraction of a tolerance start at heights that differ,
/// and together are as costly as copies exactly one on another), and of those that end together; the sum of the squares of the
/// segments that start at one height (exactly), and of those that end at one, wherever they are across; and the pairs of segments that
/// start or end together (ends of both kinds at one place, as the ends of the slices of a pie are): those the sweep compares, each pair
/// once, into a cache it does not clear.
fn static_counts(segments: &Segments, eps: f64) -> (u64, u64, u64) {
    // The starts and ends in order of height, by height (exactly): a level segment is not on the sweep line, and a fan of them along
    // one level costs nothing there.
    let starts: Vec<(f64, f64)> =
        segments.entrances().iter().filter(|e| !segments[e.1].is_horizontal()).map(|&(y, idx)| (segments[idx].start().x, y)).collect();
    let ends: Vec<(f64, f64)> = segments.exits().iter().map(|&(y, idx)| (segments[idx].end().x, y)).collect();
    let cell = 2.0 * GROUP_WIDTH * eps;
    let groups = [&starts, &ends].iter().fold(0u64, |t, pts| t.saturating_add(grid_fourth_powers(pts, cell)));
    let both: Vec<(f64, f64)> = starts.iter().chain(ends.iter()).copied().collect();
    let pairs = cluster_pairs(&both, cell);
    let squares = |points: &[(f64, f64)]| {
        // `points` are in order of height already (the arena sorts them).
        let mut total = 0u64;
        let mut run = 0u64;
        let mut last = None;
        for &(_, y) in points {
            if last.is_some_and(|l| l != y) {
                total = total.saturating_add(run.saturating_pow(2));
                run = 0;
            }
            last = Some(y);
            run += 1;
        }
        total.saturating_add(run.saturating_pow(2))
    };
    (groups, squares(&starts).saturating_add(squares(&ends)), pairs)
}

/// The greatest third difference of a segment's control points, over the two axes: how far a cubic is from a quadratic, and so how many
/// times the comparison of it with another halves the stretch it looks at. Nothing for a line.
fn third_difference(segment: &Segment) -> f64 {
    if segment.is_line() {
        return 0.0;
    }
    let c = segment.to_kurbo_cubic();
    let dx = (c.p3.x - 3.0 * c.p2.x + 3.0 * c.p1.x - c.p0.x).abs();
    let dy = (c.p3.y - 3.0 * c.p2.y + 3.0 * c.p1.y - c.p0.y).abs();
    dx.max(dy)
}

/// What a segment is for the estimate of a comparison: its third difference ([`third_difference`]) and the extent across of its control points (a
/// curve lies within the box of them).
struct Features {
    third: f64,
    left: f64,
    right: f64,
}

impl Features {
    fn of(segment: &Segment) -> Self {
        let c = segment.to_kurbo_cubic();
        let xs = [c.p0.x, c.p1.x, c.p2.x, c.p3.x];
        let (left, right) = (xs.iter().copied().fold(f64::INFINITY, f64::min), xs.iter().copied().fold(f64::NEG_INFINITY, f64::max));
        let third = if segment.is_line() {
            0.0
        } else {
            (c.p3.x - 3.0 * c.p2.x + 3.0 * c.p1.x - c.p0.x).abs().max((c.p3.y - 3.0 * c.p2.y + 3.0 * c.p1.y - c.p0.y).abs())
        };
        Self { third, left, right }
    }

    /// The least distance across between this segment and `other`, from the boxes of their control points: no gap between the curves is less.
    /// Nothing for segments whose boxes overlap in their extent across.
    fn separation(&self, other: &Self) -> f64 {
        (self.left - other.right).max(other.left - self.right).max(0.0)
    }
}

/// The most steps the comparison of two segments can take that a curved piece has not been paid for already, from the third differences alone
/// and how far apart the boxes of the curves are (or the tolerance, where they are not): the gap between the curves is not under that, so the
/// steps are at most what the cube root of the third differences over it makes of them. Curves of ordinary size (a circle of 100,000 cubics,
/// a stroke), and curves that are a good way from one another, cannot pass it, and are told without looking at where they lie.
fn may_take_more_than_is_paid(a: &Features, b: &Features, eps: f64) -> bool {
    let gap = a.separation(b).max(eps);
    // A float converted with `as` saturates (a count that is not a number is none), and a count this large is charged.
    (NODES_PER_ROOT * ((a.third + b.third) / gap).cbrt()) as u64 > FREE_NODES
}

/// The steps the sweep takes to compare two segments, estimated from what makes them many: the comparison of two cubics halves a stretch of
/// height until the error of its quadratic approximations, which goes with the third differences of the two curves, is under the gap
/// between them, so that the stretch at a height takes as many steps as the cube root of the third differences over the gap there (circles
/// of a million in radius, 0.03 apart, take 320 for each comparison), and the comparison the mean of that over the height the two share
/// ([`GAP_SAMPLES`] readings, none under the tolerance), times the share of the taller one's height that they share. Two curves that are far
/// apart take one, whatever their third differences are, and so do two that share no height. The steps, and the readings they came from.
#[cfg(test)]
fn comparison_steps(a: &Segment, b: &Segment, eps: f64) -> (u64, u64) {
    comparison_steps_of(a, b, third_difference(a) + third_difference(b), eps)
}

/// [`comparison_steps`] for two segments whose third differences add up to `third`.
fn comparison_steps_of(a: &Segment, b: &Segment, third: f64, eps: f64) -> (u64, u64) {
    let span = |s: &Segment| (s.start().y.min(s.end().y), s.start().y.max(s.end().y));
    let ((a0, a1), (b0, b1)) = (span(a), span(b));
    let (y0, y1) = (a0.max(b0), a1.min(b1));
    // Level segments, and pairs that share no height, are not compared along their length.
    if y1.partial_cmp(&y0) != Some(std::cmp::Ordering::Greater) {
        return (0, 0);
    }
    let shared = (y1 - y0) / (a1 - a0).max(b1 - b0);
    let gap_at = |i: usize, of: usize| {
        let y = y0 + (y1 - y0) * (i as f64 + 0.5) / of as f64;
        (a.at_y(y) - b.at_y(y)).abs()
    };
    // A pair that the nearest of two readings shows to be cheap, whatever the rest of it is, takes no steps worth counting and is not read further:
    // curves of ordinary size a good way apart (the two sides of a stroke, which take 6 steps by the sweep's own count where the estimate says 27) are
    // most of the pairs a drawing has, and cost the sweep what its pieces are paid for.
    let nearest = (0..COARSE_SAMPLES).map(|i| gap_at(i, COARSE_SAMPLES)).fold(f64::INFINITY, f64::min).max(GAP_FLOOR * eps);
    let at_nearest = NODES_PER_ROOT * (third / nearest).cbrt() * shared;
    if at_nearest <= COARSE_NODES as f64 {
        return (0, COARSE_SAMPLES as u64);
    }
    let (mut roots, mut close) = (0.0, 0usize);
    for i in 0..GAP_SAMPLES {
        let gap = gap_at(i, GAP_SAMPLES);
        close += usize::from(gap < COINCIDENT * eps);
        // `max` takes the floor for a gap that is not a number, as for one under it.
        roots += (third / gap.max(GAP_FLOOR * eps)).cbrt();
    }
    // Curves under the tolerance from one another all along are one curve to the comparison, which stops at once: copies on one another, and
    // what the sweep's own rounding moves, take no steps. Where they are close along a part only, that part is charged as if the gap were a tolerance.
    if close == GAP_SAMPLES {
        return (0, (COARSE_SAMPLES + GAP_SAMPLES) as u64);
    }
    // A float converted with `as` saturates, and a count this large is refused.
    (((NODES_PER_ROOT * roots / GAP_SAMPLES as f64 * shared).max(1.0)) as u64, (COARSE_SAMPLES + GAP_SAMPLES) as u64)
}

/// Where a segment lies across the sweep line at height `y` (a segment that ends at the line, or is the least out of its range by the
/// rounding of the tolerance, is read at its nearest end).
fn across(segment: &Segment, y: f64) -> f64 {
    let (low, high) = (segment.start().y.min(segment.end().y), segment.start().y.max(segment.end().y));
    segment.at_y(y.clamp(low, high))
}

/// Whether `segment` is still on the sweep line after the line at `y`: it ends higher than the line (the sweep makes a line at the height of
/// each end, and what ends at the line is gone after it).
fn stays(segment: &Segment, y: f64) -> bool {
    segment.start().y.max(segment.end().y) > y
}

/// The segments on the sweep line from the one just before the places that lie across from `left` less [`CLUSTER_WIDTH`] tolerances to the
/// one just after those that lie to `right` and as much more (and, past each end, the segments that leave at this line as far as the first that
/// stays): every segment that can be next to one of a range that lies between them after it. The line is in order across (but for the places within a tolerance or two of one another, which are not told apart: the width
/// is eight), so the first place that does not lie to the left of the window is found by halving and the rest are read in turn, as many as
/// there are (a crowd of segments within the tolerance is read through, and paid for: each place read is counted in `probes`).
fn line_window(line: &SweepLine<'_, '_, '_>, left: f64, right: f64, eps: f64, probes: &mut u64) -> Vec<SegIdx> {
    let segments = line.segments();
    let y = line.y();
    let width = CLUSTER_WIDTH * eps;
    let read = std::cell::Cell::new(0u64);
    let at = |i: usize| {
        read.set(read.get() + 1);
        line.line_segment(i).map(|s| across(&segments[s], y))
    };
    let left_of_it = |i: usize| at(i).is_some_and(|x| x < left - width);
    // A place past the end of the line is not left of anything: one that is not, found by doubling, ends the search.
    let mut high = 1usize;
    while left_of_it(high) {
        match high.checked_mul(2) {
            Some(h) => high = h,
            None => break,
        }
    }
    let mut low = 0usize;
    while low < high {
        let mid = low + (high - low) / 2;
        if left_of_it(mid) {
            low = mid + 1;
        } else {
            high = mid;
        }
    }
    let mut window = Vec::new();
    // The segments before the first place that is not left of the window, as far as the first of them that stays on the line after this one (the
    // ones that leave lie between it and the window, and are gone): the ones next to a range's first.
    let mut i = low;
    while let Some(before) = i.checked_sub(1) {
        read.set(read.get() + 1);
        let Some(s) = line.line_segment(before) else { break };
        window.push(s);
        i = before;
        if stays(&segments[s], y) {
            break;
        }
    }
    window.reverse();
    let mut i = low;
    let mut past = false;
    loop {
        read.set(read.get() + 1);
        let Some(s) = line.line_segment(i) else { break };
        window.push(s);
        // The first place past the window, and the first one after it that stays on the line, are its last: next to the range's last on that side.
        past = past || across(&segments[s], y) > right + width;
        if past && stays(&segments[s], y) {
            break;
        }
        i += 1;
    }
    *probes = probes.saturating_add(read.get());
    window
}

/// The comparisons the sweep makes of segments that lie close to one another, which each pair of segments has once (the sweep keeps the order
/// of the pairs it has compared): the steps of each, beyond those a curved piece is paid for, charged the first time the pair is seen, and what
/// reading the gaps and searching the line for them took.
struct Comparisons<'a> {
    segments: &'a Segments,
    eps: f64,
    /// The greatest third difference of any segment: no pair has more.
    max_third: f64,
    seen: HashSet<(SegIdx, SegIdx)>,
    /// What has been charged: steps, readings of the gap, places of the line read.
    steps: u64,
    samples: u64,
    probes: u64,
}

impl<'a> Comparisons<'a> {
    fn new(segments: &'a Segments, eps: f64) -> Self {
        let max_third = segments.segments().map(third_difference).fold(0.0, f64::max);
        Self { segments, eps, max_third, seen: HashSet::new(), steps: 0, samples: 0, probes: 0 }
    }

    /// Whether any pair of segments of the input can pass what a curved piece is paid for: not where no curve of it is bent enough.
    fn costly(&self) -> bool {
        (NODES_PER_ROOT * (2.0 * self.max_third / self.eps).cbrt()) as u64 > FREE_NODES
    }

    /// Whether `s` can be one of a pair that passes it, with the most bent segment of the input for the other.
    fn can_pass(&self, s: SegIdx) -> bool {
        (NODES_PER_ROOT * ((third_difference(&self.segments[s]) + self.max_third) / self.eps).cbrt()) as u64 > FREE_NODES
    }

    /// What has been charged, into `counts`.
    fn copy_into(&self, counts: &mut Counts) {
        counts.compared = self.steps;
        counts.sampled = self.samples;
        counts.probed = self.probes;
    }

    /// The steps of the comparison of `a` and `b` that are counted (the pieces' own are taken off in [`Counts::time`]): none if either is a line
    /// (below), or if the pair was seen before.
    fn charge(&mut self, a: SegIdx, b: SegIdx) -> u64 {
        let (sa, sb) = (&self.segments[a], &self.segments[b]);
        // A curve and a line are close along a stretch only where the curve is no more bent than the gap is wide (it keeps to within the gap of the
        // line, and a cubic that does has a third difference of the order of the gap): their comparison is a few steps, a tangent circle's included.
        if a == b || sa.is_line() || sb.is_line() {
            return 0;
        }
        let (fa, fb) = (Features::of(sa), Features::of(sb));
        self.charge_featured(a, &fa, b, &fb)
    }

    /// [`Self::charge`] for two segments (not both lines) whose [`Features`] are known.
    fn charge_featured(&mut self, a: SegIdx, fa: &Features, b: SegIdx, fb: &Features) -> u64 {
        // The pairs a curved piece pays for are not kept: only those that may be charged are.
        if a == b || !may_take_more_than_is_paid(fa, fb, self.eps) || !self.seen.insert((a.min(b), a.max(b))) {
            return 0;
        }
        let (steps, samples) = comparison_steps_of(&self.segments[a], &self.segments[b], fa.third + fb.third, self.eps);
        self.steps = self.steps.saturating_add(steps);
        self.samples = self.samples.saturating_add(samples);
        steps
    }

    /// The pairs next to one another in `list`, and in a crowd (more than [`CROWD`] of them) those a power of two of places from one another: a crowd
    /// is put in order by comparisons that are not all between neighbours (a new segment finds its place by halving the crowd, and a range is sorted).
    fn charge_list(&mut self, list: &[SegIdx]) {
        // The features of each segment are made once, and a pair of lines costs nothing.
        let made: Vec<(Features, bool)> = list.iter().map(|&s| (Features::of(&self.segments[s]), self.segments[s].is_line())).collect();
        let pair = |this: &mut Self, i: usize, j: usize| {
            if let (Some(&a), Some(&b), Some((fa, a_line)), Some((fb, b_line))) = (list.get(i), list.get(j), made.get(i), made.get(j))
                && !(*a_line || *b_line)
            {
                this.charge_featured(a, fa, b, fb);
            }
        };
        // Each segment looked at and each pair looked at is paid for as a place of the line read.
        let mut looked = list.len() as u64;
        for i in 1..list.len() {
            pair(self, i - 1, i);
            looked += 2;
        }
        if list.len() > CROWD && list.len() <= CROWD_MAX {
            for i in 0..list.len() {
                let mut d = 2usize;
                while d < list.len() {
                    pair(self, i, i.saturating_add(d));
                    looked += 2;
                    d = d.saturating_mul(2);
                }
            }
        }
        self.probes = self.probes.saturating_add(looked);
    }

    /// Charges the pairs the sweep compares to put the segments of a range in order, and those that the range makes next to one another with the
    /// segments just outside it. `ids` are the segments of the range after it (those that stay and those that enter, in order across), `old` those
    /// it had before (those that stay and those that leave), where `ids` is empty: where it is not the leaving ones lie between the ones that stay
    /// and are on the line until it is done with. After the range every two segments next
    /// to one another on the line, from the one before the places that lie across from the range's segments to the one after, are compared if they
    /// were not (a copy that starts a little higher than the one it runs along is put in beside it; two curves that were kept apart by one that has
    /// left are next to one another; the ends of the segments that cross are next to the ones outside them), and every such pair is charged.
    fn in_range(&mut self, ids: &[SegIdx], old: &[SegIdx], line: &SweepLine<'_, '_, '_>) {
        self.charge_list(ids);
        // A range of lines only makes pairs with a line in each, which cost the sweep nothing worth counting.
        if !ids.is_empty() && ids.iter().all(|&s| self.segments[s].is_line()) {
            return;
        }
        // Nothing near a range is charged where no member of it, with the most bent segment of the input, can pass what a curved piece is paid for.
        if !ids.iter().chain(old).any(|&s| self.can_pass(s)) {
            return;
        }
        let y = line.y();
        let places = ids.iter().chain(old).map(|&s| across(&self.segments[s], y));
        let (left, right) = places.fold((f64::INFINITY, f64::NEG_INFINITY), |(l, r), x| (l.min(x), r.max(x)));
        if left > right {
            return;
        }
        let window = line_window(line, left, right, self.eps, &mut self.probes);
        // The segments that end at this line are on it still, and are gone after it: the pairs next to one another now, and those that are after.
        let staying: Vec<SegIdx> = window.iter().copied().filter(|&s| stays(&self.segments[s], y)).collect();
        self.charge_list(&window);
        if staying.len() != window.len() {
            self.charge_list(&staying);
        }
    }

    /// The pairs the sweep compares as segments come to its line together, before it sweeps: the segments that start at one height (and
    /// those that end at one) are put in the order of the line at once, and a pair of them next to one another across is compared. For
    /// copies of a shape moved a little each, this is where the sweep meets the pairs that cost it most, in one step, which no count made
    /// as it goes can stop in time. Charged into `counts` and held to `budget`: `true` if it passes.
    fn over_budget_at_one_height(&mut self, counts: &mut Counts, budget: u64) -> bool {
        let segments = self.segments;
        let mut run: Vec<(f64, SegIdx)> = Vec::new();
        let mut height = f64::NAN;
        let entrances = segments.entrances().iter().filter(|e| !segments[e.1].is_horizontal()).map(|&(y, idx)| (y, segments[idx].start().x, idx));
        let exits = segments.exits().iter().map(|&(y, idx)| (y, segments[idx].end().x, idx));
        for ends in [entrances.collect::<Vec<_>>(), exits.collect::<Vec<_>>()] {
            for (y, x, idx) in ends {
                // The ends come in order of height; a run is the ends at exactly one.
                if y != height && self.over_budget_in_run(&mut run, counts, budget) {
                    return true;
                }
                height = y;
                run.push((x, idx));
            }
            if self.over_budget_in_run(&mut run, counts, budget) {
                return true;
            }
        }
        false
    }

    /// The pairs next to one another across in `run` (emptied), charged into `counts`: `true` if they pass `budget`.
    fn over_budget_in_run(&mut self, run: &mut Vec<(f64, SegIdx)>, counts: &mut Counts, budget: u64) -> bool {
        run.sort_by(|p, q| p.0.total_cmp(&q.0));
        let mut over = false;
        for w in run.windows(2) {
            if let [(_, a), (_, b)] = *w {
                self.charge(a, b);
                self.copy_into(counts);
                if counts.work() > budget {
                    over = true;
                    break;
                }
            }
        }
        run.clear();
        over
    }
}

/// How many segments span the sweep line at each height, as the sweep reaches them: those that have started (a level segment is not on
/// the line) and not ended above it. A contour that goes on through a point takes one place there for the two segments that meet, so this
/// is the line's length or the segments that start there more.
struct Spanning<'a> {
    segments: &'a Segments,
    entered: usize,
    left: usize,
    spanning: u64,
}

impl<'a> Spanning<'a> {
    fn new(segments: &'a Segments) -> Self {
        Self { segments, entered: 0, left: 0, spanning: 0 }
    }

    /// The segments that span the line at `y`, the heights asked for increasing.
    fn at(&mut self, y: f64) -> u64 {
        let (entrances, exits) = (self.segments.entrances(), self.segments.exits());
        while let Some(&(_, idx)) = entrances.get(self.entered).filter(|e| e.0 <= y) {
            self.spanning += u64::from(!self.segments[idx].is_horizontal());
            self.entered += 1;
        }
        while exits.get(self.left).is_some_and(|e| e.0 < y) {
            self.spanning = self.spanning.saturating_sub(1);
            self.left += 1;
        }
        self.spanning
    }
}

/// Sweeps `segments` at tolerance `eps` as the topology does, counting its work, and stops with the counts so far as soon as
/// they come to more than `budget` (`Err`).
pub(crate) fn dry_sweep(segments: &Segments, eps: f64, budget: u64) -> Result<Counts, Counts> {
    let (groups, batch, pairs) = static_counts(segments, eps);
    let mut counts = Counts { segments: segments.len() as u64, groups, batch, pairs, ..Counts::default() };
    work::add(Work::MeterGroups, usize::try_from(counts.groups).unwrap_or(usize::MAX));
    if counts.work() > budget {
        return Err(counts);
    }
    // The pairs that cost the sweep what a piece is not paid for are those of curves bent enough: an input of lines, or of curves of ordinary
    // size, has none to look for.
    let mut compared = Comparisons::new(segments, eps);
    let costly = compared.costly();
    if costly && compared.over_budget_at_one_height(&mut counts, budget) {
        return Err(counts);
    }
    let mut sweeper = Sweeper::new(segments, eps);
    let mut line_bufs = SweepLineBuffers::default();
    let mut range_bufs = SweepLineRangeBuffers::default();
    let mut connected = SegmentsConnectedAtX::default();
    let mut xs: Vec<f64> = Vec::new();
    let mut ids: Vec<SegIdx> = Vec::new();
    let mut old: Vec<SegIdx> = Vec::new();
    let mut spanning = Spanning::new(segments);
    while let Some(mut line) = sweeper.next_line(&mut line_bufs) {
        counts.steps += 1;
        let y = line.y();
        let on_line = spanning.at(y);
        while let Some(mut range) = line.next_range(&mut range_bufs, segments) {
            counts.ranges += 1;
            ids.clear();
            ids.extend(range.segment_range());
            xs.clear();
            xs.extend(ids.iter().map(|&s| segments[s].at_y(y)));
            if costly {
                // The segments a range had before are wanted for where it lies only where nothing stays or enters (a range of curves that leave).
                old.clear();
                if ids.is_empty() {
                    old.extend(range.old_segment_range());
                }
                compared.in_range(&ids, &old, range.line());
                compared.copy_into(&mut counts);
            }
            // A range of one or two cannot be a cluster that costs more than its pieces.
            let cluster = if xs.len() <= 2 { xs.len() } else { tightest(&mut xs, CLUSTER_WIDTH * eps) } as u64;
            let mut made = 0u64;
            let mut curved = 0u64;
            // The loop of `Topology`: the segments that go down from each position, and the horizontal fragments that start there.
            while range.x().is_some() {
                range.update_segments_at_x(&mut connected);
                for s in connected.connected_down() {
                    made += 1;
                    curved += u64::from(!segments[s].is_line());
                }
                range.increase_x();
                made += range.active_horizontals().count() as u64;
            }
            counts.pieces = counts.pieces.saturating_add(made);
            counts.curved = counts.curved.saturating_add(curved);
            counts.tangle = counts.tangle.saturating_add(made.saturating_mul(cluster * cluster));
            counts.scan = counts.scan.saturating_add(made.saturating_mul(on_line));
            if counts.work() > budget {
                return Err(counts);
            }
        }
    }
    Ok(counts)
}

/// The sweep of `paths` at tolerance `eps` (as the operation will make it) is run, counted, and held to
/// [`crate::Limits::sweep_work`], if it has more than [`GATE_PIECES`] monotone pieces: `Err(TooComplex)` when it passes.
///
/// Where the segments cannot be made (an open subpath, a path that starts with a close) it is `Ok`: the sweep
/// that follows refuses those before it sweeps.
pub(crate) fn within<'a>(paths: impl IntoIterator<Item = &'a BezPath>, eps: f64) -> Result<(), PathOpsError> {
    within_a_part(paths, eps, 1)
}

/// [`within`] for a planar map, which takes several times what its sweep takes, in time and in memory (a piece of the sweep is 400
/// bytes and about 0.6 microsecond, and one of the map 1 to 4.4 kilobytes and 6 to 16 microseconds): the budget a map is held to is
/// the `1 / `[`MAP_FACTOR`] part of the sweep's.
pub(crate) fn within_for_a_map<'a>(paths: impl IntoIterator<Item = &'a BezPath>, eps: f64) -> Result<(), PathOpsError> {
    within_a_part(paths, eps, MAP_FACTOR)
}

/// What the budget of a planar map is divided by: the map takes 8 to 15 times the sweep it is made of, and 3 to 4 times its memory, for
/// crossing-heavy input (the shape builder of a star of 501 points took 4.7 s and 1.3 GB, and the sweep of it 0.4 s and 0.4 GB), and
/// about 4 times the sweep for a grid of lines, so that a quarter is a compromise: a star of 705 points, the most it admits (711 is refused),
/// takes 10 to 12 s and 2.6 GB to map.
pub(crate) const MAP_FACTOR: u64 = 4;

/// The sweep of `paths` held to `1 / part` of the budget.
fn within_a_part<'a>(paths: impl IntoIterator<Item = &'a BezPath>, eps: f64, part: u64) -> Result<(), PathOpsError> {
    let budget = limits().sweep_work;
    // A limit lifted to the most is a meter left off: nothing is run.
    if budget == usize::MAX {
        return Ok(());
    }
    let budget = budget as u64 / part;
    let Some(segments) = segments_of(paths) else { return Ok(()) };
    if segments.len() <= GATE_PIECES {
        return Ok(());
    }
    work::add(Work::MeteredSweeps, 1);
    // The sweep panics on a few degenerate inputs, as the topology's does, and the topology reports it.
    let Ok(verdict) = catch_unwind(AssertUnwindSafe(|| dry_sweep(&segments, eps, budget))) else { return Ok(()) };
    match verdict {
        Ok(counts) => {
            work::add(Work::MeterNanos, usize::try_from(counts.work()).unwrap_or(usize::MAX));
            Ok(())
        }
        Err(counts) => {
            work::add(Work::MeterNanos, usize::try_from(counts.work()).unwrap_or(usize::MAX));
            Err(PathOpsError::TooComplex)
        }
    }
}

#[cfg(test)]
mod tests {
    use kurbo::{BezPath, Point};
    use linesweeper::topology::Topology;

    use std::time::{Duration, Instant};

    use super::*;
    use crate::boolean::eps_for;
    use crate::testutil::{
        arc_fan, arc_windows, bars_on_a_baseline, bowed_star, circle_pairs, circle_pairs_apart, circle_pairs_with_blockers,
        circle_pairs_with_blockers_apart, cubic_circle, cubic_fan, curves_beside_a_crowd, diagonals, fan_along_a_line, grid_of_squares,
        groups_of_loops, jittered_pencil, jittered_pies, mesh, nearly_parallel_lines, polygons_a_few_tolerances_apart, polygons_an_ulp_apart,
        polyline, rect, rings, row_of_lenses, s_strokes, stacked_arcs, stacked_pies, stacked_polygons, stacked_segments, star, wheel,
        windows_sharing_a_tenth,
    };
    use crate::{FillRule, Limits, with_limits};

    /// `path` with every subpath closed (the sweep takes closed subpaths).
    fn closed(path: &BezPath) -> BezPath {
        let mut out = BezPath::new();
        let mut open = false;
        for el in path.elements() {
            match el {
                PathEl::MoveTo(_) => {
                    if open {
                        out.close_path();
                    }
                    open = true;
                    out.push(*el);
                }
                PathEl::ClosePath => {
                    out.push(*el);
                    open = false;
                }
                _ => out.push(*el),
            }
        }
        if open {
            out.close_path();
        }
        out
    }

    /// The inputs the proofs of sameness run on: the fixtures of the suites, and inputs made to differ (paths that start with no
    /// move, several paths, quadratics, horizontal segments, a polygon that doubles back on itself).
    fn inputs() -> Vec<(String, Vec<BezPath>)> {
        let one = |name: &str, p: BezPath| (name.to_string(), vec![closed(&p)]);
        // `extend` takes what `push` refuses in a build for tests (a path that does not begin with a move); in a release build
        // it can be made either way, and is input.
        let mut no_move = BezPath::new();
        no_move.extend([
            PathEl::LineTo(Point::new(30.0, 10.0)),
            PathEl::LineTo(Point::new(60.0, 40.0)),
            PathEl::LineTo(Point::new(10.0, 40.0)),
            PathEl::ClosePath,
        ]);
        let mut quads = BezPath::new();
        quads.move_to((0.0, 0.0));
        quads.quad_to((50.0, 80.0), (100.0, 0.0));
        quads.quad_to((50.0, -80.0), (0.0, 0.0));
        quads.close_path();
        vec![
            one("star 201", star(201)),
            one("star 51 bowed", bowed_star(51, 3.0, 0.0, 0.0)),
            one("diagonals 20", diagonals(20)),
            one("pies 3 x 40", stacked_pies(3, 40)),
            one("wheel 60", wheel(60)),
            one("jittered pies", jittered_pies(1, 80, 2e-5)),
            one("jittered pencil", jittered_pencil(60, 100.0, 1e-5)),
            one("pencil", nearly_parallel_lines(60, 1e8, 1.6, false, true)),
            one("zigzag", nearly_parallel_lines(60, 1e8, 1.6, true, true)),
            one("cubic fan", cubic_fan(60)),
            one("arc fan", arc_fan(40)),
            one("arc windows", arc_windows(40)),
            one("stacked polygons", stacked_polygons(5, 30)),
            one("stacked segments", stacked_segments(2, 40)),
            one("stacked arcs", stacked_arcs(30)),
            one("circle", cubic_circle(400, 100.0, 0.0, 0.0)),
            one("circle far out", cubic_circle(400, 100.0, 1e12, 1e12)),
            one("row of lenses", row_of_lenses(50)),
            one("quadratics", quads),
            ("a path with no move".to_string(), vec![no_move.clone()]),
            (
                "several paths".to_string(),
                vec![
                    closed(&star(21)),
                    closed(&rect(10.0, 10.0, 60.0, 60.0)),
                    no_move,
                    closed(&polyline(&[(0.0, 0.0), (80.0, 0.0), (80.0, 5.0)], true)),
                ],
            ),
        ]
    }

    /// The segments of the dry sweep are the ones the topology builds (the same pieces of the same curves in the same
    /// order, and the same entrances and exits), and the pieces it counts are the output segments the topology makes (the
    /// ones a coincident pair is merged out of included: `compute_positions` has an entry for every one made).
    #[test]
    fn the_dry_sweep_is_the_sweep_the_topology_runs() {
        for (name, paths) in inputs() {
            let eps = eps_for(paths.iter()).expect("finite");
            let mine = segments_of(paths.iter()).unwrap_or_else(|| panic!("{name}: no segments"));
            let top = Topology::<i32>::from_paths(paths.iter().map(|p| (p, ())), eps).unwrap_or_else(|_| panic!("{name}: no topology"));
            assert_eq!(mine.len(), top.segments.len(), "{name}: segments");
            assert!(mine.segments().eq(top.segments.segments()), "{name}: the same segments");
            assert_eq!(mine.entrances(), top.segments.entrances(), "{name}: entrances");
            assert_eq!(mine.exits(), top.segments.exits(), "{name}: exits");
            let counts = dry_sweep(&mine, eps, u64::MAX).expect("no budget");
            assert_eq!(counts.pieces as usize, top.compute_positions().len(), "{name}: the pieces the topology holds");
            assert!(counts.pieces >= mine.len() as u64 || mine.len() == 0, "{name}: a piece for every segment at least");
        }
    }

    /// What the topology refuses or cannot make is no segments here, and `within` leaves the refusal to the sweep.
    #[test]
    fn what_the_sweep_refuses_is_left_to_it() {
        let mut open = BezPath::new();
        open.move_to((0.0, 0.0));
        for i in 1..400 {
            open.line_to((f64::from(i), f64::from(i % 7)));
        }
        assert!(segments_of([&open]).is_none());
        let mut starts_closed = BezPath::new();
        starts_closed.extend([PathEl::ClosePath, PathEl::MoveTo(Point::new(1.0, 1.0)), PathEl::LineTo(Point::new(5.0, 9.0)), PathEl::ClosePath]);
        assert!(segments_of([&starts_closed]).is_none());
        assert_eq!(within([&open], 1e-6), Ok(()));
        assert_eq!(within([&starts_closed], 1e-6), Ok(()));
    }

    /// An input of the gate's size or fewer is not swept, and one piece more is.
    #[test]
    fn the_gate_is_a_count_of_monotone_pieces() {
        let polygon = |n: usize| {
            let pts: Vec<(f64, f64)> = (0..n)
                .map(|i| {
                    let a = std::f64::consts::TAU * i as f64 / n as f64;
                    (100.0 * a.cos(), 100.0 * a.sin() * 0.99)
                })
                .collect();
            polyline(&pts, true)
        };
        // A polygon of n corners has n pieces, but for the horizontal ones; ask the segments themselves.
        let pieces = |path: &BezPath| segments_of([path]).map_or(0, |s| s.len());
        let small = (GATE_PIECES..GATE_PIECES + 6).map(polygon).find(|p| pieces(p) == GATE_PIECES).expect("a polygon of the gate's size");
        let big = (GATE_PIECES..GATE_PIECES + 6).map(polygon).find(|p| pieces(p) == GATE_PIECES + 1).expect("a polygon of one more");
        let eps = eps_for([&small]).expect("finite");
        work::reset();
        assert_eq!(within([&small], eps), Ok(()));
        assert_eq!(work::take(Work::MeteredSweeps), 0, "{GATE_PIECES} pieces are not swept");
        assert_eq!(within([&big], eps), Ok(()));
        assert_eq!(work::take(Work::MeteredSweeps), 1, "{} pieces are", GATE_PIECES + 1);
        // Curves count by the monotone pieces they are cut into: a circle of 64 cubics is 4 more than its quarters.
        assert!(pieces(&cubic_circle(64, 100.0, 0.0, 0.0)) >= 64);
    }

    /// The budget is held at an exact count: the sweep of an input is admitted by the work it counted and refused by one less, and a
    /// refused sweep stops where it passed the budget, not at the end.
    #[test]
    fn the_budget_is_an_exact_bound_on_the_counted_work() {
        let paths = [closed(&star(301))];
        let eps = eps_for(paths.iter()).expect("finite");
        let segments = segments_of(paths.iter()).expect("segments");
        let full = dry_sweep(&segments, eps, u64::MAX).expect("no budget");
        let at = full.work();
        assert!(at > 0);
        assert_eq!(dry_sweep(&segments, eps, at), Ok(full));
        let refused = dry_sweep(&segments, eps, at - 1).expect_err("one less");
        assert!(refused.work() > at - 1 && refused.work() <= at);
        // What is counted before the sweep is held to the budget as the rest is: a budget that equals it lets the sweep start, and the
        // sweep is refused at its first range.
        let (groups, batch, pairs) = static_counts(&segments, eps);
        let before = Counts { groups, batch, pairs, ..Counts::default() }.work();
        assert!(before > 0);
        assert!(dry_sweep(&segments, eps, before).expect_err("the first range passes it").steps > 0);
        assert_eq!(dry_sweep(&segments, eps, before - 1).expect_err("before the sweep").steps, 0);
        // A tenth of the budget stops it long before the end.
        let early = dry_sweep(&segments, eps, at / 10).expect_err("a tenth");
        assert!(early.steps < full.steps / 5, "stopped at {} of {} steps", early.steps, full.steps);
        // Through the limit: the same budget, as `Limits::sweep_work`.
        let limit = |n: usize| Limits { sweep_work: n, ..Limits::default() };
        let at = usize::try_from(at).expect("fits");
        assert_eq!(with_limits(limit(at), || within(paths.iter(), eps)), Ok(()));
        assert_eq!(with_limits(limit(at - 1), || within(paths.iter(), eps)), Err(PathOpsError::TooComplex));
    }

    /// A limit lifted to the most leaves the meter off: nothing is swept.
    #[test]
    fn a_lifted_limit_leaves_the_meter_off() {
        let paths = [closed(&star(301))];
        let eps = eps_for(paths.iter()).expect("finite");
        work::reset();
        let off = Limits { sweep_work: usize::MAX, ..Limits::default() };
        assert_eq!(with_limits(off, || within(paths.iter(), eps)), Ok(()));
        assert_eq!(work::take(Work::MeteredSweeps), 0);
        assert_eq!(within(paths.iter(), eps), Ok(()));
        assert_eq!(work::take(Work::MeteredSweeps), 1);
    }

    /// `tightest` finds the most values within a width.
    #[test]
    fn the_tightest_cluster_is_the_most_within_a_width() {
        assert_eq!(tightest(&mut [], 1.0), 0);
        assert_eq!(tightest(&mut [5.0], 1.0), 1);
        assert_eq!(tightest(&mut [0.0, 10.0, 20.0], 1.0), 1);
        assert_eq!(tightest(&mut [0.0, 0.5, 1.0, 10.0, 10.2, 10.4, 10.6, 10.8, 20.0], 1.0), 5);
        // Unsorted, and exactly the width apart.
        assert_eq!(tightest(&mut [3.0, 1.0, 2.0, 0.0], 3.0), 4);
        assert_eq!(tightest(&mut [3.0, 1.0, 2.0, 0.0], 2.999), 3);
    }

    /// The segments that start (or end) together are counted by the fourth power of their number: a stack of copies of one segment
    /// (each out and back, so two pieces that start at its upper end and end at its lower) is two clusters whatever the order,
    /// separate places are separate groups, and copies moved a fraction of a tolerance, across or up, are one group as copies one on another are.
    #[test]
    fn ends_that_start_together_are_counted_by_the_fourth_power() {
        let eps = 1e-6;
        let count = |path: &BezPath| static_counts(&segments_of([path]).expect("segments"), eps).0;
        // 10 copies are 20 pieces: the 20 that start at the upper end make one group, the 20 that end at the lower another.
        assert_eq!(count(&stacked_segments(1, 10)), 2 * 20u64.pow(4));
        // Two stacks far apart count each alone.
        assert_eq!(count(&stacked_segments(2, 10)), 4 * 20u64.pow(4));
        // Copies moved sideways by a tenth of a tolerance each are one group, by ten tolerances each are not.
        let moved = copies_moved;
        assert_eq!(count(&moved(0.1 * eps, 0.0)), 2 * 20u64.pow(4));
        assert_eq!(count(&moved(10.0 * eps, 0.0)), 10 * 2 * 2u64.pow(4));
        assert_eq!(count(&moved(1.0, 0.0)), 10 * 2 * 2u64.pow(4));
        // Copies that start at heights a fraction of a tolerance apart start together, though the sweep takes them a step at a time.
        assert_eq!(count(&moved(0.0, 0.2 * eps)), 2 * 20u64.pow(4));
        // And ones a tolerance and a half or more apart, in a grid, do not all.
        assert!(count(&moved(0.0, 30.0 * eps)) < 2 * 20u64.pow(4) / 10);
        // Pieces that start at heights a unit in the last place apart are one group; their ends, a unit apart, are not.
        let mut ulp = BezPath::new();
        let mut y = 100.0f64;
        for i in 0..10 {
            ulp.move_to((0.0, y));
            ulp.line_to((7.0, 150.0 + f64::from(i)));
            ulp.close_path();
            y = f64::from_bits(y.to_bits() + 1);
        }
        assert_eq!(count(&ulp), 20u64.pow(4) + 10 * 2u64.pow(4));
    }

    /// The terms of the work are charged by their weights.
    #[test]
    fn the_work_is_the_sum_of_its_terms_at_their_weights() {
        let c = Counts {
            pieces: 3,
            curved: 5,
            scan: 11_000,
            tangle: 7_000,
            groups: 13_000,
            batch: 17,
            pairs: 19,
            compared: 230,
            sampled: 29,
            probed: 31,
            ..Counts::default()
        };
        let time = 3 * PIECE_WORK
            + 5 * CURVED_WORK
            + 11_000 * SCAN_PICOS / 1000
            + 7_000 * TANGLE_PICOS / 1000
            + 13_000 * GROUP_PICOS / 1000
            + 17 * BATCH_WORK
            + (230 - 5 * FREE_NODES) * COMPARE_NODE_WORK
            + 29 * SAMPLE_WORK
            + 31 * LOOKUP_WORK;
        assert_eq!(c.time(), time);
        assert_eq!(c.memory(), 3 * PIECE_MEMORY + 19 * CACHE_MEMORY);
        assert_eq!(c.work(), time.max(3 * PIECE_MEMORY + 19 * CACHE_MEMORY).max(19 * PAIR_WORK));
        // Pieces that are many and cheap are held by the memory they take, not the time.
        let many = Counts { pieces: 3_000_000, ..Counts::default() };
        assert!(many.memory() > many.time());
        assert_eq!(many.work(), many.memory());
        // Pairs that are many are held by the cache they fill, and by more than they take with the pieces: the least of them that fills
        // the table that does not fit costs the whole budget.
        let pairs = Counts { pairs: 1_000_000, ..Counts::default() };
        assert_eq!(pairs.work(), 1_000_000 * PAIR_WORK);
        assert!(pairs.work() > pairs.memory() && pairs.work() > pairs.time());
        // And pieces and pairs that are many together add: their memory is held at once.
        let both = Counts { pieces: 1_000_000, pairs: 3_000_000, ..Counts::default() };
        assert_eq!(both.work(), both.memory());
        assert_eq!(both.memory(), 1_000_000 * PIECE_MEMORY + 3_000_000 * CACHE_MEMORY);
        assert!(both.memory() > 3_000_000 * PAIR_WORK);
        assert_eq!(Counts { pieces: u64::MAX, curved: u64::MAX, pairs: u64::MAX, compared: u64::MAX, ..Counts::default() }.work(), u64::MAX);
        assert_eq!(Counts { sampled: u64::MAX, ..Counts::default() }.work(), u64::MAX);
        assert_eq!(Counts { probed: u64::MAX, ..Counts::default() }.work(), u64::MAX);
        // The readings of the gaps and the places of the line read are paid for, alone.
        assert_eq!(Counts { sampled: 1_000, ..Counts::default() }.work(), 1_000 * SAMPLE_WORK);
        assert_eq!(Counts { probed: 1_000, ..Counts::default() }.work(), 1_000 * LOOKUP_WORK);
        // Steps of comparisons that are many are held by the time they take.
        let steps = Counts { compared: 1_000_000, ..Counts::default() };
        assert_eq!(steps.work(), 1_000_000 * COMPARE_NODE_WORK);
    }

    /// The budget admits 3.56 million pairs that start or end together and no more, whatever else the sweep holds, as the cache of
    /// comparisons is a table that doubles: the second cache's next one, at 1.8 million entries (for pies, 3.7 million pairs counted),
    /// aborted a process held to 3 GB.
    #[test]
    fn the_budget_admits_three_point_five_six_million_pairs() {
        let budget = crate::budget::MAX_SWEEP_WORK as u64;
        let pairs = budget / PAIR_WORK;
        assert!((3_550_000..=3_570_000).contains(&pairs), "{pairs}");
        assert!(Counts { pairs, ..Counts::default() }.work() <= budget);
        assert!(Counts { pairs: pairs + 1, ..Counts::default() }.work() > budget);
        // A pair is worth less in the sum with the pieces than alone, and more than the 650 bytes it holds: the 3 GB is 3.8 million.
        const { assert!(CACHE_MEMORY < PAIR_WORK) };
        assert!((3_700_000..=3_900_000).contains(&(budget / CACHE_MEMORY)));
    }

    /// The steps the comparisons of the pairs of circles that start together are charged before the sweep, and the dry sweep makes of them.
    fn steps_charged_before_the_sweep(path: &BezPath) -> u64 {
        let eps = eps_for([path]).expect("finite");
        let segments = segments_of([path]).expect("segments");
        let mut counts = Counts::default();
        assert!(!Comparisons::new(&segments, eps).over_budget_at_one_height(&mut counts, u64::MAX));
        counts.compared
    }

    /// Curves that run along one another are charged by the steps of the comparison of them: the same for each pair at chained sizes (twice the
    /// pairs, twice the steps), and more the closer they run, by the cube root of the gap.
    #[test]
    fn curves_that_run_along_one_another_are_charged_by_the_steps_of_their_comparison() {
        let charged = |n: usize, r: f64, d: f64| steps_charged_before_the_sweep(&circle_pairs(n, r, d));
        let sizes = [64usize, 128, 256, 512, 1_024];
        let steps: Vec<u64> = sizes.iter().map(|&n| charged(n, 1e6, 0.03)).collect();
        for (w, n) in steps.windows(2).zip(sizes.windows(2)) {
            let ratio = w[1] as f64 / w[0] as f64;
            assert!((1.95..=2.05).contains(&ratio), "{} pairs to {}: {ratio:.3} times the steps ({steps:?})", n[0], n[1]);
        }
        // A pair of circles of a million 0.03 apart takes the sweep about 7,900 steps (503,000 counted in the sweep for 64 pairs), and the count says
        // 7,924: within a twentieth.
        let each = steps[0] as f64 / sizes[0] as f64;
        assert!((7_500.0..=8_300.0).contains(&each), "{each:.0} steps for each pair");
        // The cube root: a hundred times the gap, a fifth of the steps (4.64), a thousand times a tenth.
        let (close, far, farther) = (charged(64, 1e5, 0.003), charged(64, 1e5, 0.3), charged(64, 1e5, 3.0));
        let (r1, r2) = (close as f64 / far as f64, far as f64 / farther as f64);
        assert!((4.0..=5.2).contains(&r1), "a hundred times the gap: {r1:.2} times the steps ({close}, {far})");
        assert!((1.9..=2.4).contains(&r2), "ten times the gap: {r2:.2} times the steps ({far}, {farther})");
        // Pairs of circles a radius apart cross: a few dozen steps for each pair, where the tolerance as the gap would count thousands.
        assert!(charged(64, 1e5, 1e5) < 64 * 200);
        // Lines, however close they run, take no steps (the comparison of two lines is a formula).
        let lines = nearly_parallel_lines(60, 1e8, 1.6, true, true);
        assert_eq!(steps_charged_before_the_sweep(&lines), 0);
    }

    /// The third difference of a segment is the greater of those of its control points over the two axes (the way off a quadratic a cubic is), and
    /// a line has none.
    #[test]
    fn the_third_difference_is_the_greater_over_the_two_axes() {
        let cubic = |p: [(f64, f64); 4]| {
            Segment::from_kurbo_cubic(kurbo::CubicBez::new(
                Point::new(p[0].0, p[0].1),
                Point::new(p[1].0, p[1].1),
                Point::new(p[2].0, p[2].1),
                Point::new(p[3].0, p[3].1),
            ))
        };
        // x: 3 - 3(-2) + 3(7) - 0 = 30; y: 10 - 3(4) + 3(1) - 0 = 1 (and the sign does not matter).
        assert_eq!(third_difference(&cubic([(0.0, 0.0), (7.0, 1.0), (-2.0, 4.0), (3.0, 10.0)])), 30.0);
        assert_eq!(third_difference(&cubic([(3.0, 0.0), (-2.0, 1.0), (7.0, 4.0), (0.0, 10.0)])), 30.0);
        // y: 10 - 0 + 0 - 0 = 10; x: 3 - 6 + 3 - 0 = 0.
        assert_eq!(third_difference(&cubic([(0.0, 0.0), (1.0, 0.0), (2.0, 0.0), (3.0, 10.0)])), 10.0);
        // y: 10 - 3(10) + 3(10) - 0 = 10 beats x: 3 - 3(2) + 3(1) - 0 = 0.
        assert_eq!(third_difference(&cubic([(0.0, 0.0), (1.0, 10.0), (2.0, 10.0), (3.0, 10.0)])), 10.0);
        // y: 8 - 3(4) + 3(1) - 0 = -1 (a negative third difference counts by its size) and x: 9 - 3(9) + 3(5) - 2 = -5.
        assert_eq!(third_difference(&cubic([(2.0, 0.0), (5.0, 1.0), (9.0, 4.0), (9.0, 8.0)])), 5.0);
        // A line, and a cubic that is one, have none.
        assert_eq!(third_difference(&Segment::from_kurbo_cubic(kurbo::CubicBez::new((0.0, 0.0), (1.0, 1.0), (2.0, 2.0), (3.0, 3.0)))), 0.0);
        let level = Segment::from_kurbo_cubic(kurbo::CubicBez::new((0.0, 5.0), (1.0, 6.0), (9.0, 6.0), (3.0, 5.0)));
        assert!(level.is_line());
        assert_eq!(third_difference(&level), 0.0);
    }

    /// [`may_take_more_than_is_paid`] for two segments.
    fn may_take(a: &Segment, b: &Segment, eps: f64) -> bool {
        may_take_more_than_is_paid(&Features::of(a), &Features::of(b), eps)
    }

    /// A pair whose third differences cannot make it more steps than a curved piece is paid for (6) is passed at once: seven times the cube root of
    /// the sum of the third differences over the tolerance, as a whole number of steps, must be more than 6.
    #[test]
    fn a_pair_that_cannot_pass_what_a_piece_is_paid_for_is_passed_at_once() {
        // A cubic whose third difference is `third` along x (its y is evenly spaced, so none along y).
        let cubic = |third: f64| Segment::from_kurbo_cubic(kurbo::CubicBez::new((0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (third, 3.0)));
        let level = || Segment::from_kurbo_cubic(kurbo::CubicBez::new((0.0, 5.0), (1.0, 6.0), (9.0, 6.0), (3.0, 5.0)));
        let passes = |a: f64, b: f64, eps: f64| may_take(&cubic(a), &if b == 0.0 { level() } else { cubic(b) }, eps);
        assert_eq!(FREE_NODES, 6);
        // 7 * cbrt(0.8 / 2) = 5.2 steps: passed.
        assert!(!passes(0.8, 0.0, 2.0));
        // 7 * cbrt(1.7 / 2) = 6.6, which is 6 whole steps: passed, and 7 * cbrt(2.2 / 2) = 7.2, which is 7: not.
        assert!(!passes(1.7, 0.0, 2.0));
        assert!(passes(2.2, 0.0, 2.0));
        // The two curves' third differences add: 1.1 and 1.1 are 2.2.
        assert!(passes(1.1, 1.1, 2.0));
        assert!(passes(0.0001, 2.2, 2.0));
        // A tolerance a tenth the size is ten times the ratio.
        assert!(passes(0.8, 0.0, 0.2));
    }

    /// The pair of arcs `g` across and `d` further at the far end (a close stretch of cubics at the start of a pair of long arcs, and far apart
    /// by the end), and the steps of the model integrated over the shared height by the finest of means (8,192 readings), for comparison.
    fn tailed_pair(g: f64, d: f64) -> (Segment, Segment) {
        let circle = cubic_circle(4, 1e6, 0.0, 0.0);
        let (p0, c1, c2, p3) = match circle.elements() {
            [PathEl::MoveTo(p0), PathEl::CurveTo(c1, c2, p3), ..] => (*p0, *c1, *c2, *p3),
            other => panic!("{other:?}"),
        };
        let across = |x: f64| kurbo::Vec2::new(x, 0.0);
        let a = Segment::from_kurbo_cubic(kurbo::CubicBez::new(p0, c1, c2, p3));
        let b = Segment::from_kurbo_cubic(kurbo::CubicBez::new(p0 + across(g), c1 + across(g), c2 + across(g), p3 + across(d)));
        (a, b)
    }

    /// What the model makes of a pair over its shared height by a fine midpoint rule, and by the median of five readings (as the estimate was
    /// before the gap was integrated): in steps.
    fn fine_integral_and_median(a: &Segment, b: &Segment, eps: f64) -> (f64, f64) {
        let third = third_difference(a) + third_difference(b);
        let (y0, y1) = (a.start().y.min(a.end().y), a.start().y.max(a.end().y));
        let gap_at = |i: usize, n: usize| {
            let y = y0 + (y1 - y0) * (i as f64 + 0.5) / n as f64;
            (a.at_y(y) - b.at_y(y)).abs().max(eps)
        };
        let n = 8_192;
        let fine = NODES_PER_ROOT * (0..n).map(|i| (third / gap_at(i, n)).cbrt()).sum::<f64>() / n as f64;
        let mut five: Vec<f64> = (0..5).map(|i| gap_at(i, 5)).collect();
        five.sort_by(f64::total_cmp);
        (fine, NODES_PER_ROOT * (third / five[2]).cbrt())
    }

    /// Two curves that are close for a stretch of their height and far apart for the rest (a pair of arcs a thousandth apart at the start and
    /// a hundred thousand at the end, sized like the circles): the median of five readings of the gap read the far part and charged a third
    /// of what the stretch costs (the crafted gap: 3.2 times too few at the one and 4.1 at the other); the mean over the height counts
    /// the stretch for its share of it, within a third (a stretch under a thirty-second of the height can fall between two readings).
    #[test]
    fn a_close_stretch_is_charged_for_its_share_of_the_height() {
        let eps = 1e-6;
        for (g, d, most_under, at_least) in [(0.03, 1e6, 3.0, 0.8), (0.03, 1e8, 3.5, 0.6), (0.03, 1e3, 1.8, 0.95), (0.03, 10.0, 1.1, 0.95)] {
            let (a, b) = tailed_pair(g, d);
            let (fine, median) = fine_integral_and_median(&a, &b, eps);
            let (steps, samples) = comparison_steps(&a, &b, eps);
            assert_eq!(samples, (COARSE_SAMPLES + GAP_SAMPLES) as u64);
            assert!(fine / median > most_under - 0.7, "the pair should beat the median: {fine:.1} against {median:.1}");
            assert!(steps as f64 >= at_least * fine && steps as f64 <= 1.1 * fine, "d {d}: {steps} steps against {fine:.1} by the fine rule");
            if d >= 1e6 {
                assert!(median < 0.45 * steps as f64, "d {d}: the median {median:.1} against {steps}");
            }
        }
    }

    /// An input can have a pair that passes what a curved piece is paid for only where its most bent segment, with another as bent, makes more than
    /// 6 whole steps over the tolerance (`costly`), and a segment can be in one only where it, with the most bent, does (`can_pass`): checked at the edge
    /// of 6 steps, for a cubic whose third difference is 0.8 (6.5 steps for the two, over a tolerance of 2), 1.2 (7.4) and 1.7, and the line that closes it.
    #[test]
    fn what_can_pass_what_a_piece_is_paid_for_is_told_at_the_edge_of_six_steps() {
        let with_third = |third: f64| {
            let mut path = BezPath::new();
            path.move_to((0.0, 0.0));
            path.curve_to((0.0, 1.0), (0.0, 2.0), (third, 3.0));
            path.close_path();
            path
        };
        for (third, costly, line_can, bent_can) in
            [(0.8, false, false, false), (1.2, true, false, true), (1.7, true, false, true), (4.0, true, true, true)]
        {
            let path = with_third(third);
            let segments = segments_of([&path]).expect("segments");
            let compared = Comparisons::new(&segments, 2.0);
            assert_eq!(compared.costly(), costly, "a third difference of {third}");
            let (line, bent) = {
                let ids: Vec<SegIdx> = segments.entrances().iter().map(|e| e.1).collect();
                let bent = ids.iter().copied().find(|&i| !segments[i].is_line()).expect("the cubic");
                (ids.iter().copied().find(|&i| segments[i].is_line()).expect("the line"), bent)
            };
            assert_eq!(compared.can_pass(line), line_can, "the line, with a third difference of {third}");
            assert_eq!(compared.can_pass(bent), bent_can, "the cubic, with a third difference of {third}");
        }
    }

    /// Every place of the line that the window reads is counted: for each range of a few inputs, at least the places of the window it gives, and not
    /// more than those with the halving to find where it starts and the two it reads past.
    #[test]
    fn the_window_of_the_line_counts_the_places_it_reads() {
        let mut all = 0usize;
        for (name, path) in
            [("circles with a crescent", circle_pairs_with_blockers(3, 1e6, 0.03)), ("rings", rings(12, 4.0)), ("lenses", row_of_lenses(30))]
        {
            let path = closed(&path);
            let eps = eps_for([&path]).expect("finite");
            let segments = segments_of([&path]).expect("segments");
            let mut sweeper = Sweeper::new(&segments, eps);
            let (mut line_bufs, mut range_bufs) = (SweepLineBuffers::default(), SweepLineRangeBuffers::default());
            let mut windows = 0usize;
            while let Some(mut line) = sweeper.next_line(&mut line_bufs) {
                while let Some(range) = line.next_range(&mut range_bufs, &segments) {
                    let y = range.line().y();
                    let places: Vec<f64> = range.segment_range().map(|s| across(&segments[s], y)).collect();
                    let (Some(&left), Some(&right)) = (places.iter().min_by(|a, b| a.total_cmp(b)), places.iter().max_by(|a, b| a.total_cmp(b)))
                    else {
                        continue;
                    };
                    let mut probes = 0u64;
                    let window = line_window(range.line(), left, right, eps, &mut probes);
                    windows += 1;
                    let n = window.len() as u64;
                    // The window, and the places read to find where it starts (the halving reads at least one).
                    assert!(probes >= n + 2, "{name}: {probes} places counted for a window of {n}");
                    // The halving reads at most twice the log of the line, the window itself, and the places read back from where it starts.
                    assert!(probes <= 2 * 64 + 2 * n + 64, "{name}: {probes} places counted for a window of {n}");
                }
            }
            all += windows;
            assert!(windows > 0, "{name}: no windows");
        }
        assert!(all > 50, "{all} windows");
    }

    /// The steps of a pair are charged for the share of the taller one's height that the two have: a cubic and its copy a unit across, whose
    /// nearest reading makes 50 steps for the whole height, and the first half of it (the copy cut at half) 21, which is under the 32 that are counted
    /// as none.
    #[test]
    fn a_pair_that_shares_half_its_height_is_charged_half() {
        use kurbo::ParamCurve;
        let eps = 1e-6;
        let a = kurbo::CubicBez::new((0.0, 0.0), (0.0, 1.0), (0.0, 2.0), (182.0, 3.0));
        let moved = |c: kurbo::CubicBez| Segment::from_kurbo_cubic(kurbo::Affine::translate((1.0, 0.0)) * c);
        let whole = comparison_steps(&Segment::from_kurbo_cubic(a), &moved(a), eps).0;
        let half = comparison_steps(&Segment::from_kurbo_cubic(a), &moved(a.subsegment(0.0..0.5)), eps).0;
        // 7 * cbrt(364) = 50 by the nearest reading, more by the readings across the height (the gap is the same all along: 50 exactly).
        assert!((49..=51).contains(&whole), "{whole} steps for the whole height");
        assert_eq!(half, 0, "half the height, at 21 steps by the nearest reading, is counted as none");
        // Past the cap, the share is what is charged: a cut at 0.8 (7 * cbrt(364 * 0.76) * 0.8) is 36 steps.
        let most = comparison_steps(&Segment::from_kurbo_cubic(a), &moved(a.subsegment(0.0..0.8)), eps).0;
        assert!((32..=40).contains(&most), "{most} steps for four fifths of the height");
    }

    /// Curves under the tolerance from one another all along are one curve to the comparison, and take no steps; from the tolerance, or
    /// under it along a part only, they are charged: at a gap just under the cap (0.9 tolerance) none, and just over it the tolerance's worth.
    #[test]
    fn curves_under_the_tolerance_take_no_steps_and_just_over_it_the_tolerances_worth() {
        let eps = 1e-6;
        let arc_apart = |g: f64| tailed_pair(g, g);
        for g in [0.0, 1e-9, 0.5 * eps, 0.88 * eps, 0.89 * eps] {
            let (a, b) = arc_apart(g);
            assert_eq!(comparison_steps(&a, &b, eps), (0, (COARSE_SAMPLES + GAP_SAMPLES) as u64), "a gap of {g}");
        }
        let (a, b) = arc_apart(0.91 * eps);
        let at_cap = comparison_steps(&a, &b, eps).0;
        assert!(at_cap > 10_000, "{at_cap} steps for a gap just over the cap");
        // The same at the tolerance and at a few: the floor is the tolerance, and the cube root takes it down from there.
        let (a, b) = arc_apart(eps);
        assert_eq!(comparison_steps(&a, &b, eps).0, at_cap);
        let (a, b) = arc_apart(8.0 * eps);
        let at_eight = comparison_steps(&a, &b, eps).0;
        assert!(at_eight < at_cap && at_eight as f64 > 0.45 * at_cap as f64, "{at_eight} against {at_cap}");
        // Under the tolerance along the first part only (the rest of the arc is far): the part is charged, not none.
        // A pair of curves that are little bent (third differences of 25 each, which make 26 steps at a gap of one and 2,500 at the tolerance) and
        // under the tolerance all along is read in full, and none; its nearest reading is under the tolerance, which is not the gap of a unit.
        let little = |dx: f64| Segment::from_kurbo_cubic(kurbo::CubicBez::new((dx, 0.0), (dx, 1.0), (dx, 2.0), (dx + 25.0, 3.0)));
        assert_eq!(comparison_steps(&little(0.0), &little(0.5 * eps), eps), (0, (COARSE_SAMPLES + GAP_SAMPLES) as u64));
        let (a, b) = tailed_pair(0.5 * eps, 100.0);
        assert!(comparison_steps(&a, &b, eps).0 > 100);
    }

    /// The pairs a curved piece pays for (curves of ordinary size, whose steps cannot pass what the piece is paid) are charged nothing, and kept
    /// nowhere: the set the dry sweep holds is of the pairs that may be charged.
    #[test]
    fn pairs_a_curved_piece_pays_for_are_not_kept() {
        let path = closed(&cubic_circle(4_000, 100.0, 0.0, 0.0));
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let mut compared = Comparisons::new(&segments, eps);
        let ids: Vec<SegIdx> = segments.entrances().iter().map(|e| e.1).chain(segments.exits().iter().map(|e| e.1)).collect();
        assert!(ids.len() > 100);
        for w in ids.windows(2) {
            if let [a, b] = *w {
                assert_eq!(compared.charge(a, b), 0);
            }
        }
        assert!(compared.seen.is_empty(), "{} pairs kept", compared.seen.len());
        // The pairs of circles that run along one another are.
        let path = circle_pairs(4, 1e6, 0.03);
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let mut compared = Comparisons::new(&segments, eps);
        let mut counts = Counts::default();
        assert!(!compared.over_budget_at_one_height(&mut counts, u64::MAX));
        assert!(counts.compared > 0 && !compared.seen.is_empty());
    }

    /// A pair of curves that cross, or touch, is charged by the gap that is typical of it (the median of five), not by the nearest, which is the
    /// tolerance where they meet: 64 pairs of circles that overlap by half their width, or touch, are a few dozen to a hundred steps for each pair, where the tolerance
    /// as the gap would charge thousands.
    #[test]
    fn curves_that_cross_are_charged_by_the_typical_gap_and_not_the_nearest() {
        let crossing = steps_charged_before_the_sweep(&circle_pairs(64, 1e5, 5e4));
        assert!(crossing < 64 * 300, "{crossing} steps for 64 pairs of circles that cross");
        // Tangent circles are close along a stretch that is a tenth of the height at the most (the gap grows as the square): the mean of the cube
        // roots counts that stretch, and is a few dozen steps for each pair where the tolerance as the gap would charge thousands.
        let touching = steps_charged_before_the_sweep(&circle_pairs(64, 1e5, 2e5));
        assert!(touching < 64 * 300, "{touching} steps for 64 pairs of tangent circles");
    }

    /// The budget holds the steps charged before the sweep as exactly as it holds the rest: a budget of the work they come to lets the sweep start,
    /// and one less refuses it before the first step.
    #[test]
    fn the_steps_charged_before_the_sweep_are_held_to_the_budget_exactly() {
        let path = circle_pairs(32, 1e6, 0.03);
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let (groups, batch, pairs) = static_counts(&segments, eps);
        let mut counts = Counts { groups, batch, pairs, ..Counts::default() };
        assert!(!Comparisons::new(&segments, eps).over_budget_at_one_height(&mut counts, u64::MAX));
        let at = counts.work();
        assert!(counts.compared > 0);
        let again = |budget: u64| {
            let mut counts = Counts { groups, batch, pairs, ..Counts::default() };
            (Comparisons::new(&segments, eps).over_budget_at_one_height(&mut counts, budget), counts)
        };
        assert!(!again(at).0, "the work they come to is admitted");
        let (over, stopped) = again(at - 1);
        assert!(over, "one less is refused");
        assert!(stopped.work() > at - 1 && stopped.work() <= at);
        // And a tenth of it stops long before the last pair.
        let (over, early) = again(at / 10);
        assert!(over && early.compared < counts.compared / 5, "stopped at {} of {} steps", early.compared, counts.compared);
        // Through the sweep, as for the pieces: the whole of the sweep's work is admitted at the work it counts and refused by one less.
        let full = dry_sweep(&segments, eps, u64::MAX).expect("no budget");
        assert_eq!(dry_sweep(&segments, eps, full.work()), Ok(full));
        assert!(dry_sweep(&segments, eps, full.work() - 1).is_err());
    }

    /// The comparisons are charged once for each pair, whether the pair is met among the segments that start together, in the ranges of the
    /// sweep, or in both: the sweep keeps the order of a pair it has compared.
    #[test]
    fn a_pair_is_charged_once_wherever_it_is_met() {
        let path = circle_pairs(8, 1e6, 0.03);
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let before = steps_charged_before_the_sweep(&path);
        assert!(before > 0);
        let full = dry_sweep(&segments, eps, u64::MAX).expect("no budget");
        // The pairs the ranges add to those next to one another at their start are few: the same steps, and not twice.
        assert!(full.compared >= before && (full.compared as f64) < 1.1 * before as f64, "{} against {before} before the sweep", full.compared);
        // Seen twice, the pair is charged the once.
        let mut seen = Comparisons::new(&segments, eps);
        let first = segments.entrances().first().expect("an entrance").1;
        let other = segments.entrances().iter().map(|e| e.1).find(|&i| i != first).expect("another");
        let _ = seen.charge(first, other);
        assert_eq!(seen.charge(first, other), 0);
        assert_eq!(seen.charge(other, first), 0);
    }

    /// A copy that starts a little higher or lower than the one it runs along is compared with it as one that starts together with it is: the
    /// pair is met in the sweep, where the new curve is at the edge of its range and its neighbour on the line is not in it, and the steps
    /// are those of the same pair moved to start together (within a tenth), which the count before the sweep alone does not see.
    #[test]
    fn a_copy_that_starts_at_another_height_is_charged_as_one_that_starts_together_is() {
        let steps = |path: &BezPath| {
            let eps = eps_for([path]).expect("finite");
            dry_sweep(&segments_of([path]).expect("segments"), eps, u64::MAX).expect("no budget").compared
        };
        let together = steps(&circle_pairs(8, 1e6, 0.03));
        assert!(together > 0);
        for dy in [3e-5, 3e-4, 3e-3, 0.03] {
            let apart = circle_pairs_apart(8, 1e6, 0.03, dy);
            let (full, before) = (steps(&apart), steps_charged_before_the_sweep(&apart));
            assert!((0.8 * together as f64..=1.3 * together as f64).contains(&(full as f64)), "dy {dy}: {full} steps against {together}");
            assert!((before as f64) < 0.5 * full as f64, "dy {dy}: {before} before the sweep of {full}");
        }
    }

    /// After the last range of each line of a sweep with the pricing of the ranges, every two segments next to one another on it (those that
    /// stay) are among the pairs charged, or are ones that cost nothing: the count of such pairs, all of them charged.
    fn every_pair_next_to_one_another_is_charged(name: &str, path: &BezPath) -> usize {
        let path = closed(path);
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let mut compared = Comparisons::new(&segments, eps);
        let mut sweeper = Sweeper::new(&segments, eps);
        let (mut line_bufs, mut range_bufs) = (SweepLineBuffers::default(), SweepLineRangeBuffers::default());
        let (mut ids, mut old) = (Vec::new(), Vec::new());
        let mut checked = 0usize;
        while let Some(mut line) = sweeper.next_line(&mut line_bufs) {
            while let Some(range) = line.next_range(&mut range_bufs, &segments) {
                ids.clear();
                ids.extend(range.segment_range());
                old.clear();
                if ids.is_empty() {
                    old.extend(range.old_segment_range());
                }
                compared.in_range(&ids, &old, range.line());
            }
            let y = line.y();
            let on_line: Vec<SegIdx> = (0..).map_while(|i| line.line_segment(i)).filter(|&s| stays(&segments[s], y)).collect();
            for w in on_line.windows(2) {
                let [a, b] = *w else { continue };
                let (sa, sb) = (&segments[a], &segments[b]);
                if sa.is_line() || sb.is_line() || !may_take(sa, sb, eps) {
                    continue;
                }
                checked += 1;
                assert!(compared.seen.contains(&(a.min(b), a.max(b))), "{name}: two segments next to one another at {y} are not charged");
            }
        }
        checked
    }

    /// The pairs of segments next to one another on the line after each of its ranges are all charged: for every line of every input here. The
    /// inputs make adjacencies in every way a range can: copies that start at other heights, curves kept apart by others that leave before them
    /// (a crescent between two circles, which ends at a quarter of each arc), curves that cross, and ordinary shapes.
    #[test]
    fn every_pair_next_to_one_another_on_the_line_is_charged() {
        let inputs = [
            ("circles, the copy higher", circle_pairs_apart(4, 1e6, 0.03, 3e-4)),
            ("circles, the copy higher by a tolerance", circle_pairs_apart(4, 1e6, 0.03, 1e-6)),
            ("circles with a crescent between", circle_pairs_with_blockers(3, 1e6, 0.03)),
            ("circles with a crescent between, the copy higher", circle_pairs_with_blockers_apart(3, 1e6, 0.03, 3e-4)),
            ("circles that cross", circle_pairs(4, 1e6, 5e5)),
            ("ellipses and arcs", stacked_arcs(30)),
            ("lenses", row_of_lenses(30)),
            ("rings", rings(12, 4.0)),
            ("a star of lines", star(41)),
        ];
        for (name, path) in inputs {
            let checked = every_pair_next_to_one_another_is_charged(name, &path);
            // The circles and the arcs have pairs that may cost; the star of lines has none.
            assert_eq!(checked > 0, name != "a star of lines", "{name}: {checked} pairs");
        }
    }

    /// The same for a curve that comes in beside the one it runs along in the middle of a crowd of segments within the tolerance of both, so
    /// that a search for its place that read the thirty-two places from the first within the tolerance did not find it (release build only:
    /// the order of curves this near is checked by `linesweeper`'s own assertions in a debug build, and is refused there).
    #[cfg_attr(debug_assertions, ignore = "linesweeper checks curves this near in a debug build: run with cargo test --release")]
    #[test]
    fn every_pair_next_to_one_another_in_a_crowd_is_charged() {
        let checked = every_pair_next_to_one_another_is_charged("a copy beside a curve in a crowd", &curves_beside_a_crowd(2, 1e6, 1.9e-6, 60));
        assert!(checked > 100, "{checked} pairs");
    }

    /// A crowd of segments within the tolerance is read through for each range that has them, and paid for: the places of the line read are counted
    /// ([`Counts::probed`]), and grow as the square of the crowd (each of its ranges reads all of it), where a search that stopped at thirty-two
    /// charged nothing past them (release build only, as the test above).
    #[cfg_attr(debug_assertions, ignore = "linesweeper checks curves this near in a debug build: run with cargo test --release")]
    #[test]
    fn a_crowd_within_the_tolerance_is_read_through_and_paid_for() {
        let probed: Vec<(usize, u64, u64)> = [40usize, 80, 160]
            .iter()
            .map(|&k| {
                let (counts, _) = counted(&curves_beside_a_crowd(1, 1e6, 1.9e-6, k));
                assert!(counts.work() >= counts.probed * LOOKUP_WORK, "{counts:?}");
                (k, counts.probed, counts.work())
            })
            .collect();
        for &(k, probed, _) in &probed {
            assert!(probed >= (k * k / 2) as u64, "a crowd of {k}: {probed} places read");
        }
        for w in probed.windows(2) {
            let ratio = w[1].1 as f64 / w[0].1 as f64;
            assert!((2.5..=4.5).contains(&ratio), "twice the crowd, {ratio:.2} times the places read ({probed:?})");
        }
    }

    /// Two curves that a third kept apart at the start, and that is gone before they are, are next to one another from then on, and compared: the
    /// crescent between each pair of circles (it ends a quarter of the way along each arc) keeps every pair of arcs from being next to one another in a
    /// range when they come in, and the dry sweep counts the steps of each as it counts those of circles with none between. (The first version counted
    /// 0.71 to 0.80 of the time the sweep takes for such circles; the count of steps by itself did not tell it, for the crescents' own pairs make up
    /// the difference: the test above, of every pair next to another on the line, is the one that fails there.)
    #[test]
    fn curves_that_a_third_kept_apart_are_charged_once_it_has_gone() {
        let steps = |path: &BezPath| {
            let eps = eps_for([path]).expect("finite");
            dry_sweep(&segments_of([path]).expect("segments"), eps, u64::MAX).expect("no budget").compared
        };
        let plain = steps(&circle_pairs(4, 1e6, 0.03));
        let hidden = steps(&circle_pairs_with_blockers(4, 1e6, 0.03));
        assert!(hidden as f64 >= 0.95 * plain as f64, "{hidden} steps against {plain} for the circles alone");
        assert!(hidden as f64 <= 2.5 * plain as f64, "{hidden} steps against {plain} for the circles alone");
        // What starts and ends together sees a part of them, and the crescents' own pairs: the rest is what the leaving of the crescents made.
        let before = steps_charged_before_the_sweep(&circle_pairs_with_blockers(4, 1e6, 0.03));
        assert!((before as f64) < 0.8 * hidden as f64, "{before} charged before the sweep of {hidden}");
    }

    /// A window of the line is charged with the pairs a power of two of places from one another when it is a crowd (more than [`CROWD`], up to
    /// [`CROWD_MAX`]), and each pair looked at is paid for as a place of the line read: the crowd of the most that is one, and one more that is not
    /// (a range that a level segment crosses, far apart across), read as neighbours only.
    #[test]
    fn a_crowd_is_charged_with_the_places_a_power_of_two_away_up_to_the_cap() {
        let path = closed(&cubic_circle(5_000, 1e6, 0.0, 0.0));
        let eps = eps_for([&path]).expect("finite");
        let segments = segments_of([&path]).expect("segments");
        let ids: Vec<SegIdx> = segments.entrances().iter().map(|e| e.1).collect();
        assert!(ids.len() > CROWD_MAX + 1, "{} segments", ids.len());
        let looked = |n: usize| {
            let mut compared = Comparisons::new(&segments, eps);
            compared.charge_list(&ids[..n]);
            compared.probes
        };
        // Neighbours alone: each segment once, and each pair for two.
        let neighbours = |n: usize| (n + 2 * (n - 1)) as u64;
        assert_eq!(looked(CROWD), neighbours(CROWD), "a window of {CROWD} is no crowd");
        assert!(looked(CROWD + 1) > neighbours(CROWD + 1), "one more is");
        // The most that is a crowd: n log n pairs; one more: neighbours alone.
        let at_cap = looked(CROWD_MAX);
        assert!(at_cap > 8 * neighbours(CROWD_MAX), "{at_cap} places for the crowd of the most");
        assert_eq!(looked(CROWD_MAX + 1), neighbours(CROWD_MAX + 1));
        // Each is paid for in the work, at the weight of a place read.
        let counts = Counts { probed: at_cap, ..Counts::default() };
        assert_eq!(counts.work(), at_cap * LOOKUP_WORK);
    }

    /// An input of lines, and one of curves of ordinary size, are not looked into at all: no place of the line is read, no gap is.
    #[test]
    fn what_cannot_pass_what_a_piece_is_paid_for_is_not_read() {
        for (name, path) in [
            ("a star of lines", star(301)),
            ("a circle of 4,000 arcs", cubic_circle(4_000, 100.0, 0.0, 0.0)),
            ("a circle of 100,000 cubics", cubic_circle(100_000, 1_000.0, 0.0, 0.0)),
        ] {
            let (counts, _) = counted(&path);
            assert_eq!((counts.probed, counts.sampled, counts.compared), (0, 0, 0), "{name}: {counts:?}");
        }
        // Where some segment can pass it, they are read: the circles of a million.
        let (counts, _) = counted(&circle_pairs(4, 1e6, 0.03));
        assert!(counts.probed > 0 && counts.sampled > 0 && counts.compared > 0);
    }

    /// The pairs of circles of a million in radius, the second of each 0.03 from the first, took the sweep 17.5 s for 2,048 pairs and 50 s
    /// for the 6,000 the map admits, which no count of pieces or of ends at one place had seen (the star of 1,291 points takes 4): they are
    /// refused before the sweep, by the steps of the comparisons of the pairs that start together, and a budget that is a time of 1 s
    /// refuses the 512 pairs that take the sweep 2.3 s.
    #[test]
    fn pairs_of_circles_that_take_the_sweep_a_minute_are_refused_before_it() {
        for n in [2_048, 6_000] {
            let path = circle_pairs(n, 1e6, 0.03);
            let eps = eps_for([&path]).expect("finite");
            let segments = segments_of([&path]).expect("segments");
            let refused = dry_sweep(&segments, eps, limits().sweep_work as u64).expect_err("a sweep of seventeen seconds or more");
            assert_eq!(refused.steps, 0, "{n} pairs: refused before the first step");
            assert!(refused.compared > 0 && refused.work() > limits().sweep_work as u64);
            assert_eq!(within([&path], eps), Err(PathOpsError::TooComplex), "{n} pairs");
        }
        let path = circle_pairs(512, 1e6, 0.03);
        let eps = eps_for([&path]).expect("finite");
        let one_second = Limits { sweep_work: 1_000_000_000, ..Limits::default() };
        assert_eq!(with_limits(one_second, || within([&path], eps)), Err(PathOpsError::TooComplex));
    }

    /// The pairs of `points` that lie within `cell` of one another across and up, counted one at a time.
    fn pairs_within(points: &[(f64, f64)], cell: f64) -> u64 {
        let mut pairs = 0;
        for (i, a) in points.iter().enumerate() {
            for b in &points[i + 1..] {
                pairs += u64::from((a.0 - b.0).abs() <= cell && (a.1 - b.1).abs() <= cell);
            }
        }
        pairs
    }

    /// Where few points lie in a window (most inputs) the pairs are counted by a pass in order across: those within a cell of one another
    /// across and up, a cell exactly apart too, and none across a gap.
    #[test]
    fn a_few_points_are_paired_when_a_cell_apart_across_and_up() {
        assert_eq!(cluster_pairs(&[], 1.0), 0);
        assert_eq!(cluster_pairs(&[(3.0, 4.0)], 1.0), 0);
        // Three together, one far across: three pairs.
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (0.5, 0.0), (0.5, 0.4), (10.0, 0.0)], 1.0), 3);
        // The same three, one of them far up: one pair.
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (0.5, 0.0), (0.5, 5.0)], 1.0), 1);
        // Exactly a cell apart across, and up, is a pair; a hair more is not.
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (1.0, 0.0)], 1.0), 1);
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (0.0, 1.0)], 1.0), 1);
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (1.0001, 0.0)], 1.0), 0);
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (0.0, 1.0001)], 1.0), 0);
        // In any order of the input, and as many as the window holds (6 at one place are 15 pairs).
        assert_eq!(cluster_pairs(&[(1.0, 0.0), (0.0, 0.0)], 1.0), 1);
        assert_eq!(cluster_pairs(&[(0.0, 0.0); 6], 1.0), 15);
        // Points that are many but each alone in its window, as the vertices of a polygon of many sides are, make no pair.
        let line: Vec<(f64, f64)> = (0..1_000).map(|i| (f64::from(i) * 3.0, 0.0)).collect();
        assert_eq!(cluster_pairs(&line, 1.0), 0);
        // A line of points as close as the window holds is a pair for each of its neighbours within a cell: 0, 0.5, 1.0 and 1.5 are 5.
        assert_eq!(cluster_pairs(&[(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (1.5, 0.0)], 1.0), 5);
    }

    /// Where many points lie in a window the pairs are counted by cell: all of those at one place, and those that lie across the edge of
    /// a cell, as in one of the four grids.
    #[test]
    fn many_points_are_paired_by_the_cell_they_lie_in() {
        assert_eq!(cluster_pairs(&[(0.0, 0.0); 7], 1.0), 21);
        assert_eq!(cluster_pairs(&[(0.0, 0.0); 10], 1.0), 45);
        // Two places far apart count each alone.
        let two: Vec<(f64, f64)> = [(0.0, 0.0); 10].into_iter().chain([(100.0, 100.0); 10]).collect();
        assert_eq!(cluster_pairs(&two, 1.0), 90);
        // Ten points that straddle the edge of a cell of the grid at 1.0 (across, up and both) are all in one cell of another grid.
        for (ax, ay) in [(1.0, 0.1), (0.1, 1.0), (1.0, 1.0)] {
            let straddling: Vec<(f64, f64)> = (0..10).map(|i| (ax - 0.045 + 0.01 * f64::from(i), ay - 0.045 + 0.01 * f64::from(i))).collect();
            assert_eq!(cluster_pairs(&straddling, 1.0), 45, "at {ax},{ay}");
        }
        // The fourth powers of the same, by the same grids.
        assert_eq!(grid_fourth_powers(&[(0.0, 0.0); 10], 1.0), 10_000);
        assert_eq!(grid_fourth_powers(&[(0.0, 0.0); 7], 1.0), 2_401);
        // A coordinate out of the range of a count is no cell of any size, and counts as the place it is.
        assert_eq!(cluster_pairs(&[(f64::INFINITY, 0.0); 8], 1.0), 28);
    }

    /// The starts (but those of level segments) and the ends of `segments`, as the counts before the sweep take them.
    fn ends_of(segments: &Segments) -> Vec<(f64, f64)> {
        segments
            .entrances()
            .iter()
            .filter(|e| !segments[e.1].is_horizontal())
            .map(|&(y, i)| (segments[i].start().x, y))
            .chain(segments.exits().iter().map(|&(y, i)| (segments[i].end().x, y)))
            .collect()
    }

    /// The pairs the cache is counted by are those of the segments that start or end where the segments of a pie meet, apexes and the
    /// points of the slices both, as a count of every pair of the ends within a cell gives it.
    #[test]
    fn the_pairs_of_pies_are_the_pairs_of_their_ends() {
        let eps = 1e-6;
        let cell = 2.0 * GROUP_WIDTH * eps;
        for (count, slices) in [(1, 30), (3, 30), (2, 7), (4, 3), (3, 100)] {
            let segments = segments_of([&closed(&stacked_pies(count, slices))]).expect("segments");
            let pairs = static_counts(&segments, eps).2;
            assert_eq!(pairs, pairs_within(&ends_of(&segments), cell), "{count} pies of {slices}");
            // An apex is the 2k - 1 ends (the level segment of a slice does not start there) that meet there, and a pie is no fewer pairs.
            let apex = 2 * slices as u64 - 1;
            assert!(pairs >= count as u64 * apex * (apex - 1) / 2, "{count} pies of {slices}: {pairs}");
        }
    }

    /// Ten copies of a segment, each out and back, the i-th moved by `i` times `dx` across and by `i % 5` times `dy` up.
    fn copies_moved(dx: f64, dy: f64) -> BezPath {
        let mut bp = BezPath::new();
        for i in 0..10 {
            let (x, y) = (f64::from(i) * dx, f64::from(i % 5) * dy);
            bp.move_to((x, y));
            bp.line_to((x + 7.0, y + 100.0));
            bp.close_path();
        }
        bp
    }

    /// Ends pair when they lie within 8 tolerances of one another: copies one on another (20 starts, 20 ends: 190 pairs each), copies
    /// 5 tolerances apart across (each with its neighbours and itself: 46 pairs each of the starts and of the ends, where a cell of 4
    /// tolerances would pair only the two of a copy), and copies further apart than a cell not at all. Copies in levels 5 tolerances apart,
    /// all in one window across, are paired by the cell they lie in: the pairs across the levels that lie in one cell of the best of four
    /// grids, more than those on the levels alone and no more than all within a cell.
    #[test]
    fn ends_within_a_cell_of_eight_tolerances_are_paired() {
        let eps = 1e-6;
        let cell = 2.0 * GROUP_WIDTH * eps;
        let counted = |dx: f64, dy: f64| static_counts(&segments_of([&copies_moved(dx, dy)]).expect("segments"), eps).2;
        let brute = |dx: f64, dy: f64| pairs_within(&ends_of(&segments_of([&copies_moved(dx, dy)]).expect("segments")), cell);
        for dx in [0.0, 0.1 * eps, 5.0 * eps, 7.9 * eps, 10.0 * eps, 1.0] {
            assert_eq!(counted(dx, 0.0), brute(dx, 0.0), "copies moved by {dx} across");
        }
        assert_eq!(counted(0.0, 0.0), 2 * 190);
        assert_eq!(counted(0.1 * eps, 0.0), 2 * 190);
        assert_eq!(counted(5.0 * eps, 0.0), 2 * 46);
        assert_eq!(counted(10.0 * eps, 0.0), 2 * 10);
        assert_eq!(counted(1.0, 0.0), 2 * 10);
        // Five levels 5 tolerances apart of 4 starts each: 6 pairs on a level, 16 between neighbouring levels, in one window across.
        let levels = counted(0.0, 5.0 * eps);
        assert!(levels > 2 * 5 * 6 && levels <= brute(0.0, 5.0 * eps), "{levels}");
    }

    /// The vertices of a polygon of many sides, each of two segments that meet, are a pair each and nothing more, however close in one
    /// direction they lie (the sides at the left and right of a round polygon of 262,000 lie within a cell across of five others).
    #[test]
    fn a_polygon_of_many_sides_has_a_pair_for_each_vertex() {
        let sides = 262_000u32;
        let wavy: Vec<(f64, f64)> = (0..sides)
            .map(|i| {
                let a = std::f64::consts::TAU * f64::from(i) / f64::from(sides);
                let r = 1000.0 + 30.0 * (a * 37.0).sin();
                (r * a.cos(), r * a.sin())
            })
            .collect();
        let segments = segments_of([&closed(&polyline(&wavy, true))]).expect("segments");
        let pairs = static_counts(&segments, 1e-6).2;
        // The vertex a path closes on is two, and every other is a start and an end, two starts or two ends: a pair each.
        assert!((u64::from(sides)..=u64::from(sides) + 2).contains(&pairs), "{pairs}");
    }

    /// The pies that aborted a process held to 3 GB are refused before the sweep that would hold them, and the 2,000 that did not are not:
    /// 2,200 pies of 30 slices, and 5,000 of 20.
    #[test]
    fn the_pies_that_filled_the_cache_are_refused_before_the_sweep() {
        let budget = crate::budget::MAX_SWEEP_WORK as u64;
        let eps = 1e-6;
        for (count, slices, admitted) in [(2_000, 30, true), (2_200, 30, false), (5_000, 20, false)] {
            let segments = segments_of([&closed(&stacked_pies(count, slices))]).expect("segments");
            let (groups, batch, pairs) = static_counts(&segments, eps);
            let before = Counts { groups, batch, pairs, ..Counts::default() };
            assert_eq!(before.work() <= budget, admitted, "{count} pies of {slices}: {pairs} pairs");
            if !admitted {
                assert_eq!(dry_sweep(&segments, eps, budget).expect_err("refused").steps, 0, "{count} pies of {slices}: before the sweep");
            }
        }
    }

    /// The pieces of the sweep of `path`.
    fn pieces_of(path: &BezPath) -> usize {
        segments_of([&closed(path)]).map_or(0, |s| s.len())
    }

    /// The largest `make(n)` of at most the gate's pieces, and its `n`.
    fn largest_under_the_gate(make: impl Fn(usize) -> BezPath) -> (usize, BezPath) {
        let mut best = (1, make(1));
        for n in 2..10_000 {
            let path = make(n);
            if pieces_of(&path) > GATE_PIECES {
                break;
            }
            best = (n, path);
        }
        best
    }

    /// Under the gate nothing is swept by the meter, and the worst input known there takes a fraction of a second: each of the shapes
    /// that cost the most for their pieces (fans and stacks of copies, pies, pencils and bundles of lines that cross along their length,
    /// stars whose every edge crosses every other, curves that run along one another), at the largest size that has no more pieces than
    /// the gate. They took 0.01 to 0.2 s in a release build.
    #[test]
    fn under_the_gate_the_worst_inputs_known_are_cheap() {
        type Make = Box<dyn Fn(usize) -> BezPath>;
        let shapes: Vec<(&str, Make)> = vec![
            ("fan of segments", Box::new(fan_along_a_line)),
            ("fan of straight cubics", Box::new(cubic_fan)),
            ("fan of arc pieces", Box::new(arc_fan)),
            ("windows of an arc", Box::new(arc_windows)),
            ("windows sharing a tenth", Box::new(windows_sharing_a_tenth)),
            ("copies of a segment", Box::new(|n| stacked_segments(1, n))),
            ("squares one on another", Box::new(|n| stacked_polygons(4, n))),
            ("triangles one on another", Box::new(|n| stacked_polygons(3, n))),
            ("copies of an arc", Box::new(stacked_arcs)),
            ("squares an ulp apart", Box::new(|n| polygons_an_ulp_apart(4, n, 1e-4))),
            ("long squares an ulp apart", Box::new(|n| polygons_an_ulp_apart(4, n, 100.0))),
            ("squares a tolerance apart", Box::new(|n| polygons_a_few_tolerances_apart(4, n, 0.15, 1e-4))),
            ("long squares a few tolerances apart", Box::new(|n| polygons_a_few_tolerances_apart(4, n, 2.0, 100.0))),
            ("pie", Box::new(|n| stacked_pies(1, n))),
            ("wheel", Box::new(wheel)),
            ("pie, apexes moved", Box::new(|n| jittered_pies(1, n, 1e-5))),
            ("pencil, lines moved", Box::new(|n| jittered_pencil(n, 100.0, 1e-5))),
            ("lines through a point", Box::new(|n| nearly_parallel_lines(n, 1e8, 1.6, false, true))),
            ("lines crossing along their length", Box::new(|n| nearly_parallel_lines(n, 1e8, 1.6, true, true))),
            ("star of lines", Box::new(|n| star(2 * n + 1))),
            ("star of bowed cubics", Box::new(|n| bowed_star(2 * n + 1, 3.0, 0.0, 0.0))),
            ("diagonals", Box::new(diagonals)),
        ];
        let mut worst = (Duration::ZERO, "");
        for (name, make) in &shapes {
            let (n, path) = largest_under_the_gate(make);
            let pieces = pieces_of(&path);
            assert!(pieces <= GATE_PIECES, "{name}");
            work::reset();
            let start = Instant::now();
            let result = crate::try_area(&path, FillRule::NonZero);
            let took = start.elapsed();
            assert!(result.is_ok(), "{name} {n}: {result:?}");
            assert_eq!(work::take(Work::MeteredSweeps), 0, "{name}: under the gate it is not metered");
            // And the planar map of it, which takes several times the sweep.
            let shapes = [crate::Shape::new(path.clone(), FillRule::NonZero, 0)];
            let start = Instant::now();
            let map = crate::try_shape_builder(&shapes, false);
            let map_took = start.elapsed();
            assert!(map.is_ok(), "{name} {n}: {:?}", map.map(|_| ()));
            eprintln!("under the gate: {name} {n}: {pieces} pieces, {took:?}, its planar map {map_took:?}");
            assert!(took < Duration::from_secs(3), "{name} {n}: {took:?}");
            // The map is the crate's own code, which a build for tests does not optimise (the sweep it rests on is).
            let map_limit = if cfg!(debug_assertions) { Duration::from_secs(30) } else { Duration::from_secs(2) };
            assert!(map_took < map_limit, "{name} {n}: its map {map_took:?}");
            for t in [took, map_took] {
                if t > worst.0 {
                    worst = (t, name);
                }
            }
        }
        eprintln!("under the gate, the worst of {}: {} {:?}", shapes.len(), worst.1, worst.0);
    }
    /// The work the dry sweep counts for `path` at its own tolerance, to the end (no budget), and the counts.
    fn counted(path: &BezPath) -> (Counts, Duration) {
        let paths = [closed(path)];
        let eps = eps_for(paths.iter()).expect("finite");
        let segments = segments_of(paths.iter()).expect("segments");
        let start = Instant::now();
        let counts = dry_sweep(&segments, eps, u64::MAX).expect("no budget");
        (counts, start.elapsed())
    }

    /// The inputs that must be made are under the budget, and what the dry sweep counts for each is reported: rings, S-strokes, a hatch of
    /// 400, a circle of 100,000 cubics at 0, 1e9 and 1e12, level bars on a baseline, meshes of 140 and 170 cells a side, grids of squares of
    /// 200 and 250, 2,000 pies of 30 slices, and the star of 1,291 points (which is the most the budget admits: its work is the budget's
    /// size). Seconds and up to 2.3 GB: in a release build only.
    #[cfg_attr(debug_assertions, ignore = "seconds and gigabytes: run with cargo test --release")]
    #[test]
    fn the_false_refusal_set_is_admitted_and_what_the_dry_sweep_counts_for_it_is_reported() {
        let budget = crate::budget::MAX_SWEEP_WORK as u64;
        let set: Vec<(&str, BezPath)> = vec![
            ("star of 1,291 points", star(1_291)),
            ("hatch of 400 and 400 strokes", diagonals(400)),
            ("15,000 rings", rings(15_000, 8.5)),
            ("16,000 rings", rings(16_000, 8.5)),
            ("40,000 S-strokes", s_strokes(40_000, 2.0)),
            ("circle of 100,000 cubics at 0", cubic_circle(100_000, 1_000.0, 0.0, 0.0)),
            ("circle of 100,000 cubics at 1e9", cubic_circle(100_000, 1_000.0, 1e9, 1e9)),
            ("circle of 100,000 cubics at 1e12", cubic_circle(100_000, 1_000.0, 1e12, 1e12)),
            ("250 bars on one baseline", bars_on_a_baseline(250)),
            ("1,000 bars on one baseline", bars_on_a_baseline(1_000)),
            ("mesh of 140 by 140 cells", mesh(140)),
            ("mesh of 170 by 170 cells", mesh(170)),
            ("grid of 200 by 200 squares", grid_of_squares(200)),
            ("grid of 250 by 250 squares", grid_of_squares(250)),
            ("2,000 pies of 30 slices", stacked_pies(2_000, 30)),
        ];
        let mut star_work = 0;
        for (name, path) in set {
            let (counts, took) = counted(&path);
            let work = counts.work();
            eprintln!("admitted: {name}: work {work} ({:.0}% of the budget), {took:.2?}, {counts:?}", work as f64 / budget as f64 * 100.0);
            assert!(work <= budget, "{name}: {work} against {budget}");
            if name.starts_with("star") {
                star_work = work;
            }
        }
        // The star is what the budget is: the work of the most it admits is within a tenth of the budget.
        assert!(star_work > budget / 10 * 9, "the star's work {star_work} is the budget's size");
        // One of 1,501 points, which no limit admits, is over it.
        assert!(counted(&star(1_501)).0.work() > budget);
    }

    /// The new holes were not seen by the pricing: it admits them (220 small squares, each a unit in the last place from the last, took 6.2
    /// s; 240 such triangles about 6 s; 220 moved by tenths of a tolerance 7.7 s), and the meter refuses them. (The pricing is
    /// `sweep_fits_within`, the first filter; the meter is `within`.)
    #[test]
    fn the_pricing_admits_what_the_meter_refuses() {
        let cap = crate::budget::MAX_SEGMENT_PAIRS;
        let eps = |p: &BezPath| eps_for([p]).expect("finite");
        for (name, path) in [
            ("220 small squares a unit in the last place apart", polygons_an_ulp_apart(4, 220, 1e-4)),
            ("240 small triangles a unit in the last place apart", polygons_an_ulp_apart(3, 240, 1e-4)),
            ("220 small squares moved by a few tenths of a tolerance", polygons_a_few_tolerances_apart(4, 220, 0.15, 1e-4)),
        ] {
            let path = closed(&path);
            assert!(crate::budget::sweep_fits_within([&path], cap), "{name}: the pricing refuses it");
            assert_eq!(within([&path], eps(&path)), Err(PathOpsError::TooComplex), "{name}: the meter admits it");
        }
    }
    /// The memory a piece is worth makes the budget 2.3 million pieces, the 0.9 to 1.6 GB the topology of them holds.
    #[test]
    fn the_budget_admits_two_point_three_million_pieces_of_memory() {
        let pieces = crate::budget::MAX_SWEEP_WORK as u64 / PIECE_MEMORY;
        assert!((2_200_000..=2_400_000).contains(&pieces), "{pieces}");
        assert_eq!(Counts { pieces: pieces + 1, ..Counts::default() }.work(), (pieces + 1) * PIECE_MEMORY);
    }

    /// The pieces of curves cost more than those of lines: 100 groups of 50 near-identical closed loops of one cubic each, a million pieces
    /// of curves, took 2.4 s of sweep beside the 1.3 s that a million pieces of lines crossing as much take.
    #[test]
    fn loops_of_one_cubic_each_are_charged_for_their_curves() {
        let (counts, _) = counted(&groups_of_loops(100, 50));
        assert!(counts.curved > 400_000, "{counts:?}");
        let budget = crate::budget::MAX_SWEEP_WORK as u64;
        assert!(counts.work() > budget / 2 && counts.work() < budget, "{}", counts.work());
    }

    /// A row of dots on one line puts the pieces of all of them into the sweep line at one height, in one step, and is charged for the square of
    /// what starts there: 5,500 of them, of 22,000 pieces, took 1.4 s of sweep.
    #[test]
    fn a_row_of_dots_is_charged_for_the_square_of_what_starts_at_one_height() {
        let (counts, _) = counted(&row_of_lenses(3_000));
        assert!(counts.batch >= 100_000_000, "{counts:?}");
        assert!(counts.work() > 300_000_000, "{}", counts.work());
    }

    /// 40 lines (each out and back, 80 segments) that enter the sweep at one height `gap` tolerances apart, in one range.
    fn lines_apart(gap: f64) -> BezPath {
        let eps = 1e-6;
        let mut bp = BezPath::new();
        for i in 0..40 {
            let x = f64::from(i) * gap * eps;
            bp.move_to((x, 0.0));
            bp.line_to((x + 0.5, 100.0));
            bp.close_path();
        }
        bp
    }

    /// The cluster a piece is made in is the most of its range within 8 tolerances of one another across: lines 20 tolerances apart are
    /// clusters of the two segments of each (out and back), lines 3 tolerances apart of the six of the three within 8.
    #[test]
    fn a_cluster_is_the_segments_within_eight_tolerances_across() {
        let (apart, _) = counted(&lines_apart(20.0));
        assert!(apart.tangle <= apart.pieces * 9, "{apart:?}");
        let (near, _) = counted(&lines_apart(3.0));
        assert!(near.tangle >= near.pieces * 25, "{near:?}");
    }

    /// The counts of one triangle, by hand: two pieces go down from the top with two segments on the line, and the piece that goes on
    /// from the middle vertex, where one segment ends and one begins, with three.
    #[test]
    fn the_counts_of_a_triangle_are_what_the_sweep_does_to_it() {
        let (counts, _) = counted(&polyline(&[(0.0, 0.0), (2.0, 1.0), (1.0, 3.0)], true));
        assert_eq!((counts.segments, counts.pieces, counts.curved, counts.ranges), (3, 3, 0, 3), "{counts:?}");
        assert_eq!(counts.scan, 2 * 2 + 3, "{counts:?}");
    }
    /// A fan of level segments from one point, 500 pieces that start together, is counted nothing before the sweep: a level segment is not on
    /// the sweep line (250 of them took 11 ms).
    #[test]
    fn a_fan_of_level_segments_is_counted_nothing_before_the_sweep() {
        let mut fan = BezPath::new();
        for k in 1..=250 {
            fan.move_to((0.0, 0.0));
            fan.line_to((f64::from(k), 0.0));
            fan.close_path();
        }
        assert_eq!(static_counts(&segments_of([&fan]).expect("segments"), 1e-6), (0, 0, 0));
    }
}
