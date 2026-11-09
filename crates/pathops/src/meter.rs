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
//!
//! The terms are weights fitted to inputs measured on one machine (the README gives the fit and where it is loose), and the budget is set so that
//! the most the meter admits takes about as long as the star of 1,291 points.

use std::collections::HashMap;
use std::panic::{AssertUnwindSafe, catch_unwind};

use kurbo::{BezPath, PathEl};
use linesweeper::Segments;
use linesweeper::sweep::{SegmentsConnectedAtX, SweepLineBuffers, SweepLineRangeBuffers, Sweeper};

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
    let mut sweeper = Sweeper::new(segments, eps);
    let mut line_bufs = SweepLineBuffers::default();
    let mut range_bufs = SweepLineRangeBuffers::default();
    let mut connected = SegmentsConnectedAtX::default();
    let mut xs: Vec<f64> = Vec::new();
    let mut spanning = Spanning::new(segments);
    while let Some(mut line) = sweeper.next_line(&mut line_bufs) {
        counts.steps += 1;
        let y = line.y();
        let on_line = spanning.at(y);
        while let Some(mut range) = line.next_range(&mut range_bufs, segments) {
            counts.ranges += 1;
            xs.clear();
            xs.extend(range.segment_range().map(|s| segments[s].at_y(y)));
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
        arc_fan, arc_windows, bars_on_a_baseline, bowed_star, cubic_circle, cubic_fan, diagonals, fan_along_a_line, grid_of_squares, groups_of_loops,
        jittered_pencil, jittered_pies, mesh, nearly_parallel_lines, polygons_a_few_tolerances_apart, polygons_an_ulp_apart, polyline, rect, rings,
        row_of_lenses, s_strokes, stacked_arcs, stacked_pies, stacked_polygons, stacked_segments, star, wheel, windows_sharing_a_tenth,
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
        let c = Counts { pieces: 3, curved: 5, scan: 11_000, tangle: 7_000, groups: 13_000, batch: 17, pairs: 19, ..Counts::default() };
        let time = 3 * PIECE_WORK
            + 5 * CURVED_WORK
            + 11_000 * SCAN_PICOS / 1000
            + 7_000 * TANGLE_PICOS / 1000
            + 13_000 * GROUP_PICOS / 1000
            + 17 * BATCH_WORK;
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
        assert_eq!(Counts { pieces: u64::MAX, curved: u64::MAX, pairs: u64::MAX, ..Counts::default() }.work(), u64::MAX);
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
