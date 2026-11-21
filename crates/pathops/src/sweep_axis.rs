//! Which way the overlap sweep runs. `linesweeper` moves a horizontal line down the page, and the
//! time it spends at each event grows with the number of segments on that line at the time. A long,
//! low path (rows of a zigzag, a stroked line of type) can put thousands of segments on the line at
//! once, where the same path turned a quarter turn puts far fewer on it. [`turned_outlines`] and
//! [`turned_fills`] estimate both directions before the sweep; when the turned one is clearly
//! cheaper they return the input turned, the caller runs its operation on that, and [`unturn`]
//! turns the result back. A quarter turn swaps and negates coordinates, so no point moves by
//! rounding.

use craft_geom::{Anchor, FillRule, PathData, SubPath};
use kurbo::{Affine, BezPath, PathSeg, Rect};

/// Inputs with fewer segments than this are always swept as they are.
const MIN_SEGMENTS: usize = 2048;

/// The turned direction is used when its estimate is below the upright one divided by this.
const MIN_GAIN: f64 = 1.5;

/// The most segments the estimate measures.
const MAX_SAMPLE: usize = 4096;

/// (x, y) to (−y, x).
const TURN: Affine = Affine::new([0.0, 1.0, -1.0, 0.0, 0.0, 0.0]);

/// (x, y) to (y, −x): the inverse of [`TURN`].
const UNTURN: Affine = Affine::new([0.0, -1.0, 1.0, 0.0, 0.0, 0.0]);

#[cfg(test)]
thread_local! {
    /// The inputs turned on this thread.
    pub(crate) static TURNS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

/// For each of the spans from `starts[i]` to `ends[i]`, the number of spans that overlap it
/// (touching counts, itself included), summed: the segments a sweep across these extents has on
/// its line while each one is there. Sorts both lists.
fn load(starts: &mut [f64], ends: &mut [f64]) -> u64 {
    starts.sort_unstable_by(f64::total_cmp);
    ends.sort_unstable_by(f64::total_cmp);
    // Spans that start at or before each end, less spans that end before each start: every span
    // that ends before a start also starts before it, so it was counted in the first sum.
    let (mut begun, mut i) = (0u64, 0usize);
    for &e in ends.iter() {
        while starts.get(i).is_some_and(|&s| s <= e) {
            i += 1;
        }
        begun += i as u64;
    }
    let (mut over, mut j) = (0u64, 0usize);
    for &s in starts.iter() {
        while ends.get(j).is_some_and(|&e| e < s) {
            j += 1;
        }
        over += j as u64;
    }
    begun.saturating_sub(over)
}

/// The segments of `outlines`.
fn outline_segments(outlines: &[BezPath]) -> impl Iterator<Item = PathSeg> + '_ {
    outlines.iter().flat_map(BezPath::segments)
}

/// The box of a segment's control points.
fn segment_box(s: PathSeg) -> Rect {
    match s {
        PathSeg::Line(l) => Rect::from_points(l.p0, l.p1),
        PathSeg::Quad(q) => Rect::from_points(q.p0, q.p2).union_pt(q.p1),
        PathSeg::Cubic(c) => Rect::from_points(c.p0, c.p3).union_pt(c.p1).union_pt(c.p2),
    }
}

/// The subpaths of `paths` that have a fill: those of two or more anchors.
fn filled_subpaths<'a>(paths: &'a [(&PathData, FillRule)]) -> impl Iterator<Item = &'a SubPath> + 'a {
    paths.iter().flat_map(|(p, _)| &p.subpaths).filter(|sp| sp.anchors.len() >= 2)
}

/// The start and end anchor of each segment in the fills of `paths`, every subpath of two or more
/// anchors closed, as [`fill_bezpath`](crate::boolean::fill_bezpath) builds them.
fn fill_segments<'a>(paths: &'a [(&PathData, FillRule)]) -> impl Iterator<Item = (&'a Anchor, &'a Anchor)> + 'a {
    filled_subpaths(paths).flat_map(|sp| {
        let n = sp.anchors.len();
        sp.anchors.iter().enumerate().map(move |(i, a)| (a, &sp.anchors[(i + 1) % n]))
    })
}

/// The box of the control points of the fill segment from `a` to `b`.
fn fill_segment_box((a, b): (&Anchor, &Anchor)) -> Rect {
    Rect::from_points(a.p, b.p).union_pt(a.h_out).union_pt(b.h_in)
}

/// Is the sweep over the `count` `segments`, each with the box `bbox` gives it, clearly cheaper
/// turned a quarter turn? Past [`MAX_SAMPLE`] segments, one segment from each run of
/// `count / MAX_SAMPLE` (rounded up) stands for the run, at a position within it that changes from
/// run to run, and only those segments' boxes are built.
fn turn_pays<T>(count: usize, segments: impl Iterator<Item = T>, bbox: impl Fn(T) -> Rect) -> bool {
    if count < MIN_SEGMENTS {
        return false;
    }
    let stride = count.div_ceil(MAX_SAMPLE);
    let mut pick = 0;
    let boxes: Vec<Rect> = segments
        .enumerate()
        .filter(|&(i, _)| {
            if i % stride == 0 {
                pick = sample_offset(i / stride, stride);
            }
            i % stride == pick
        })
        .map(|(_, s)| bbox(s))
        .collect();
    if !boxes.iter().all(Rect::is_finite) {
        return false;
    }
    let (mut x0, mut x1): (Vec<f64>, Vec<f64>) = boxes.iter().map(|b| (b.x0, b.x1)).unzip();
    let (mut y0, mut y1): (Vec<f64>, Vec<f64>) = boxes.iter().map(|b| (b.y0, b.y1)).unzip();
    let pays = load(&mut x0, &mut x1) as f64 * MIN_GAIN < load(&mut y0, &mut y1) as f64;
    #[cfg(test)]
    if pays {
        TURNS.with(|n| n.set(n.get() + 1));
    }
    pays
}

/// The position, below `stride`, of the segment measured in run `run`: a hash of `run` (the
/// SplitMix64 finalizer), so input that repeats every few segments is measured at every position.
fn sample_offset(run: usize, stride: usize) -> usize {
    let mut z = (run as u64).wrapping_add(0x9e37_79b9_7f4a_7c15);
    z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    ((z ^ (z >> 31)) % stride as u64) as usize
}

/// `outlines` turned a quarter turn, when sweeping them that way is clearly cheaper.
pub(crate) fn turned_outlines(outlines: &[BezPath]) -> Option<Vec<BezPath>> {
    let count = outlines.iter().map(|o| o.segments().count()).sum();
    turn_pays(count, outline_segments(outlines), segment_box).then(|| outlines.iter().map(|o| TURN * o).collect())
}

/// `paths` turned a quarter turn, when sweeping their fills that way is clearly cheaper.
pub(crate) fn turned_fills(paths: &[(&PathData, FillRule)]) -> Option<Vec<(PathData, FillRule)>> {
    let count = filled_subpaths(paths).map(|sp| sp.anchors.len()).sum();
    turn_pays(count, fill_segments(paths), fill_segment_box).then(|| paths.iter().map(|(p, r)| (p.transformed(TURN), *r)).collect())
}

/// A result computed on turned input, turned back.
pub(crate) fn unturn(mut path: PathData) -> PathData {
    path.transform(UNTURN);
    path
}

/// A point of turned input, turned back.
pub(crate) fn unturn_point(p: kurbo::Point) -> kurbo::Point {
    UNTURN * p
}

#[cfg(test)]
mod tests {
    use craft_geom::{Anchor, Point, Vec2};
    use kurbo::{Join, Stroke, StrokeOpts};

    use super::*;
    use crate::boolean::{BoolOp, DEFAULT_PRECISION, fill_bezpath, try_area, try_boolean, try_unite_all_upright, unite_all_upright};
    use crate::offset::stroke_region_upright;
    use crate::{stroke_region, try_unite_all_with, unite_all_with};

    /// A small xorshift generator, so the random cases are the same on every run.
    struct Rng(u64);

    impl Rng {
        fn unit(&mut self) -> f64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            (self.0 >> 11) as f64 / (1u64 << 53) as f64
        }
    }

    /// `rows` rows of a zigzag `cols` anchors long, each anchor moved by up to `jitter`: a long, low path.
    fn zigzag(rows: usize, cols: usize, jitter: f64, rng: &mut Rng, closed: bool) -> PathData {
        let mut anchors = Vec::new();
        for row in 0..rows {
            for col in 0..cols {
                let c = if row % 2 == 0 { col } else { cols - 1 - col };
                let x = c as f64 * 1.6 + (rng.unit() - 0.5) * jitter;
                let y = row as f64 * 6.0 + if col % 2 == 0 { 0.0 } else { 2.0 } + (rng.unit() - 0.5) * jitter;
                let p = Point::new(x, y);
                let dir = if row % 2 == 0 { 0.5 } else { -0.5 };
                anchors.push(Anchor::smooth(p, p + Vec2::new(dir, 0.0)));
            }
        }
        PathData::single(SubPath::new(anchors, closed))
    }

    /// The same zigzag stood on end: a tall, narrow path.
    fn upright(p: &PathData) -> PathData {
        p.transformed(Affine::new([0.0, 1.0, 1.0, 0.0, 0.0, 0.0]))
    }

    /// A unit square with its lower left corner at (`x`, `y`).
    fn square(x: f64, y: f64) -> PathData {
        let corners = [(x, y), (x + 1.0, y), (x + 1.0, y + 1.0), (x, y + 1.0)];
        PathData::single(SubPath::new(corners.map(|(x, y)| Anchor::corner(Point::new(x, y))).to_vec(), true))
    }

    fn outline(p: &PathData) -> BezPath {
        kurbo::stroke(p.to_bezpath().iter(), &Stroke::new(1.0).with_join(Join::Round), &StrokeOpts::default(), 1e-3)
    }

    fn segments(p: &BezPath) -> usize {
        p.segments().count()
    }

    fn turns_during(f: impl FnOnce()) -> usize {
        let before = TURNS.with(std::cell::Cell::get);
        f();
        TURNS.with(std::cell::Cell::get) - before
    }

    /// `a` and `b` have the same pieces and cover the same area to within the refit precision: the
    /// area in one and not the other is less than a band [`DEFAULT_PRECISION`] wide along the outline.
    fn assert_same_region(a: &PathData, b: &PathData, what: &str) {
        use kurbo::Shape;
        assert_eq!(a.subpaths.len(), b.subpaths.len(), "{what}: subpaths");
        let band = DEFAULT_PRECISION * a.to_bezpath().perimeter(1e-3);
        let area_a = try_area(a, FillRule::NonZero).unwrap();
        let area_b = try_area(b, FillRule::NonZero).unwrap();
        assert!((area_a - area_b).abs() <= band, "{what}: area {area_a} against {area_b}, band {band}");
        let apart = try_boolean(a, FillRule::NonZero, b, FillRule::NonZero, BoolOp::Xor, DEFAULT_PRECISION).unwrap();
        let apart = try_area(&apart, FillRule::NonZero).unwrap();
        assert!(apart <= band, "{what}: {apart} of {area_a} differs, band {band}");
    }

    #[test]
    fn a_long_low_path_is_swept_turned_and_a_tall_or_small_one_as_it_is() {
        let mut rng = Rng(7);
        let low = zigzag(3, 300, 0.0, &mut rng, false);
        let stroke = outline(&low);
        // More segments than the estimate measures, so it decides from a sample.
        assert!(segments(&stroke) > MAX_SAMPLE, "{} segments", segments(&stroke));
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&stroke)))), 1);
        let fill = zigzag(3, 800, 0.0, &mut rng, true);
        assert_eq!(turns_during(|| drop(unite_all_with(&[(&fill, FillRule::NonZero)], DEFAULT_PRECISION))), 1);
        assert_eq!(turns_during(|| drop(try_unite_all_with(&[(&fill, FillRule::NonZero)], DEFAULT_PRECISION))), 1);
        // Stood on end, the same paths are already narrow across the sweep.
        let tall = outline(&upright(&low));
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&tall)))), 0);
        assert_eq!(turns_during(|| drop(unite_all_with(&[(&upright(&fill), FillRule::NonZero)], DEFAULT_PRECISION))), 0);
        // A square grid costs the same either way.
        let grid: Vec<PathData> = (0..900).map(|i| square((i % 30) as f64 * 3.0, (i / 30) as f64 * 3.0)).collect();
        let grid: Vec<(&PathData, FillRule)> = grid.iter().map(|p| (p, FillRule::NonZero)).collect();
        assert_eq!(turns_during(|| drop(unite_all_with(&grid, DEFAULT_PRECISION))), 0);
        // Below the size where the sweep costs much, nothing is turned.
        let short = outline(&zigzag(1, 40, 0.0, &mut rng, false));
        assert!(segments(&short) < MIN_SEGMENTS);
        assert_eq!(turns_during(|| drop(stroke_region(std::slice::from_ref(&short)))), 0);
    }

    #[test]
    fn offset_path_sweeps_a_long_low_path_turned_with_the_result_of_the_path_on_end() {
        let mut rng = Rng(17);
        // An open zigzag grown by 1.5, a closed one grown by 0.5 and a closed one inset by 0.3. Each
        // is swept turned when low and as it is when stood on end, and the two results cover the
        // same region. None of the three leaves a face for the distance test to drop.
        for (closed, delta) in [(false, 1.5), (true, 0.5), (true, -0.3)] {
            let what = format!("closed {closed}, offset {delta}");
            let low = zigzag(3, 300, 0.4, &mut rng, closed);
            let tall = upright(&low);
            let offset = |p: &PathData| crate::try_offset_path(p, delta, Join::Round, 4.0).unwrap();
            let mut turned = PathData::default();
            assert_eq!(turns_during(|| turned = offset(&low)), 1, "{what}: low");
            let mut on_end = PathData::default();
            assert_eq!(turns_during(|| on_end = offset(&tall)), 0, "{what}: tall");
            assert!(!turned.is_empty(), "{what}: empty");
            assert_same_region(&turned, &upright(&on_end), &what);
        }
        // A long plate with a row of round holes: growing it by 1 closes the narrow holes, which
        // the distance test drops when it measures each hole's point turned back into the plate's
        // own frame, and keeps the wide ones.
        use kurbo::Shape;
        let mut plate = kurbo::Rect::new(0.0, 0.0, 4000.0, 20.0).to_path(0.1);
        for i in 0..200u32 {
            let r = if i % 2 == 0 { 0.8 } else { 4.0 };
            plate.extend(kurbo::Circle::new((10.0 + 20.0 * f64::from(i), 10.0), r).to_path(0.01).reverse_subpaths());
        }
        let plate = PathData::from_bezpath(&plate);
        let grow = |p: &PathData| crate::try_offset_path(p, 1.0, Join::Round, 4.0).unwrap();
        let mut grown = PathData::default();
        assert_eq!(turns_during(|| grown = grow(&plate)), 1, "plate");
        assert_eq!(grown.subpaths.len(), 1 + 100, "plate");
        assert_same_region(&grown, &upright(&grow(&upright(&plate))), "plate");
    }

    #[test]
    fn a_repeating_path_is_measured_at_every_position() {
        // Short segments in a row along the bottom, with a long one stacked above the row after
        // every `period - 1` of them. The short ones alone are cheaper to sweep turned; with the
        // long ones, the path is cheaper upright.
        let add = |path: &mut BezPath, a: (f64, f64), b: (f64, f64)| {
            path.move_to(a);
            path.line_to(b);
        };
        for period in [2, 4] {
            let n = MAX_SAMPLE;
            let width = (n * period) as f64 * 3.0;
            let mut both = BezPath::new();
            let mut shorts = BezPath::new();
            for i in 0..n {
                for k in 0..period - 1 {
                    let x = (i * period + k) as f64 * 3.0;
                    add(&mut both, (x, 0.0), (x + 1.0, 0.0));
                    add(&mut shorts, (x, 0.0), (x + 1.0, 0.0));
                }
                add(&mut both, (0.0, 10.0 + i as f64), (width, 10.0 + i as f64));
            }
            assert_eq!(segments(&both), period * MAX_SAMPLE, "period {period}");
            assert!(turned_outlines(&[shorts]).is_some(), "period {period}: shorts");
            assert!(turned_outlines(&[both]).is_none(), "period {period}: both");
        }
    }

    #[test]
    fn the_load_counts_every_overlapping_pair() {
        let mut rng = Rng(5);
        for n in [0, 1, 2, 7, 60] {
            // Coarse values, so many spans share ends and touch.
            let spans: Vec<(f64, f64)> = (0..n)
                .map(|_| {
                    let a = (rng.unit() * 10.0).round();
                    let b = (rng.unit() * 10.0).round();
                    (a.min(b), a.max(b))
                })
                .collect();
            let by_hand = spans.iter().map(|&(lo, hi)| spans.iter().filter(|&&(s, e)| s <= hi && e >= lo).count() as u64).sum::<u64>();
            let (mut starts, mut ends): (Vec<f64>, Vec<f64>) = spans.iter().copied().unzip();
            assert_eq!(load(&mut starts, &mut ends), by_hand, "{n} spans");
        }
    }

    #[test]
    fn fill_boxes_are_the_boxes_of_the_fills_segments() {
        let mut rng = Rng(9);
        let open = zigzag(2, 30, 1.0, &mut rng, false);
        let closed = zigzag(1, 20, 1.0, &mut rng, true);
        let lone = PathData::single(SubPath::new(vec![Anchor::corner(Point::new(4.0, 4.0))], false));
        let paths = [(&open, FillRule::NonZero), (&closed, FillRule::EvenOdd), (&lone, FillRule::NonZero)];
        let fills: Vec<BezPath> = paths.iter().map(|(p, _)| fill_bezpath(p)).collect();
        assert_eq!(fill_segments(&paths).map(fill_segment_box).collect::<Vec<_>>(), outline_segments(&fills).map(segment_box).collect::<Vec<_>>());
        assert_eq!(filled_subpaths(&paths).map(|sp| sp.anchors.len()).sum::<usize>(), fills.iter().map(|f| f.segments().count()).sum::<usize>());
    }

    #[test]
    fn turning_and_turning_back_moves_no_point() {
        let mut rng = Rng(11);
        let p = zigzag(2, 50, 3.0, &mut rng, true);
        assert_eq!(unturn(p.transformed(TURN)), p);
        let bp = outline(&p);
        assert_eq!(UNTURN * (TURN * &bp), bp);
    }

    #[test]
    fn turned_results_match_the_upright_sweep_on_random_paths() {
        let mut rng = Rng(0x9e37_79b9_7f4a_7c15);
        for case in 0..3 {
            let rows = 2 + case;
            let low = zigzag(rows, 120 + 40 * case, 0.8, &mut rng, false);
            let stroke = [outline(&low)];
            assert!(turned_outlines(&stroke).is_some(), "case {case}: not turned");
            assert_same_region(&stroke_region(&stroke), &stroke_region_upright(&stroke), &format!("case {case}: stroke"));
            let fill = zigzag(rows, 1100, 0.8, &mut rng, true);
            let one = [(&fill, FillRule::NonZero)];
            assert!(turned_fills(&one).is_some(), "case {case}: fill not turned");
            assert_same_region(&unite_all_with(&one, DEFAULT_PRECISION), &unite_all_upright(&one, DEFAULT_PRECISION), &format!("case {case}: unite"));
            assert_same_region(
                &try_unite_all_with(&one, DEFAULT_PRECISION).unwrap(),
                &try_unite_all_upright(&one, DEFAULT_PRECISION).unwrap(),
                &format!("case {case}: try_unite"),
            );
        }
    }

    #[test]
    fn tall_and_ordinary_art_give_exactly_the_upright_result() {
        let mut rng = Rng(13);
        let tall = [outline(&upright(&zigzag(3, 300, 0.4, &mut rng, false)))];
        assert_eq!(stroke_region(&tall), stroke_region_upright(&tall), "tall stroke");
        let tall_fill = upright(&zigzag(3, 800, 0.4, &mut rng, true));
        let one = [(&tall_fill, FillRule::NonZero)];
        assert_eq!(unite_all_with(&one, DEFAULT_PRECISION), unite_all_upright(&one, DEFAULT_PRECISION), "tall unite");
        // A square grid of overlapping squares, and a small path below the size that is measured.
        let grid: Vec<PathData> = (0..900).map(|i| square((i % 30) as f64 * 0.75, (i / 30) as f64 * 0.75)).collect();
        let grid: Vec<(&PathData, FillRule)> = grid.iter().map(|p| (p, FillRule::NonZero)).collect();
        assert_eq!(unite_all_with(&grid, DEFAULT_PRECISION), unite_all_upright(&grid, DEFAULT_PRECISION), "grid");
        assert_eq!(try_unite_all_with(&grid, DEFAULT_PRECISION).unwrap(), try_unite_all_upright(&grid, DEFAULT_PRECISION).unwrap(), "grid, try");
        let small = [outline(&zigzag(1, 40, 0.4, &mut rng, false))];
        assert_eq!(stroke_region(&small), stroke_region_upright(&small), "small");
    }

    #[test]
    fn edge_cases_give_the_upright_result() {
        // Nothing to sweep.
        assert!(turned_outlines(&[]).is_none());
        assert!(stroke_region(&[]).is_empty());
        assert!(unite_all_with(&[], DEFAULT_PRECISION).is_empty());
        // A single segment.
        let line = BezPath::from_vec(vec![kurbo::PathEl::MoveTo((0.0, 0.0).into()), kurbo::PathEl::LineTo((5_000.0, 0.0).into())]);
        let one = [outline(&PathData::from_bezpath(&line))];
        assert!(turned_outlines(&one).is_none());
        assert_same_region(&stroke_region(&one), &stroke_region_upright(&one), "one segment");

        // No overlaps: a row of separate squares.
        let squares: Vec<PathData> = (0..600).map(|i| square(i as f64 * 3.0, 0.0)).collect();
        let refs: Vec<(&PathData, FillRule)> = squares.iter().map(|p| (p, FillRule::NonZero)).collect();
        assert!(turned_fills(&refs).is_some());
        let apart = unite_all_with(&refs, DEFAULT_PRECISION);
        assert_eq!(apart.subpaths.len(), 600);
        assert_same_region(&apart, &unite_all_upright(&refs, DEFAULT_PRECISION), "separate squares");

        // All anchors on one line: no area either way.
        let flat = PathData::single(SubPath::new((0..2100).map(|i| Anchor::corner(Point::new(i as f64, 0.0))).collect(), true));
        let flat = [(&flat, FillRule::NonZero)];
        assert!(turned_fills(&flat).is_some());
        assert!(unite_all_with(&flat, DEFAULT_PRECISION).is_empty());
        assert!(unite_all_upright(&flat, DEFAULT_PRECISION).is_empty());

        // Coincident segments: the same path twice unites to that path.
        let mut rng = Rng(3);
        let fill = zigzag(2, 1100, 0.0, &mut rng, true);
        let twice = [(&fill, FillRule::NonZero), (&fill, FillRule::NonZero)];
        assert!(turned_fills(&twice).is_some());
        assert_same_region(&unite_all_with(&twice, DEFAULT_PRECISION), &unite_all_upright(&[(&fill, FillRule::NonZero)], DEFAULT_PRECISION), "twice");
    }
}
