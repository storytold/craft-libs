//! The places where segments cross one another are held to their own number, whatever the pieces on the sweep line
//! (`MAX_REAL_CROSSINGS`): the sweep takes about 3 microseconds and 1.6 kilobytes for each, and the count of them times the
//! pieces on the line (`MAX_CROSSED_WORK`) is small where shapes that cross a great deal are stacked so that their heights do
//! not overlap. Curves were not counted at all.

use craft_pathops::*;
use std::time::{Duration, Instant};

use crate::common::{
    arc_fan, arc_windows, cubic_circle, cubic_fan, fan_along_a_line, jittered_pencil, jittered_pies, nearly_parallel_lines, square_with_cut_sides,
    stacked_pies, stacked_polygons, stacked_segments, stacked_stars, star_at, strip_with_cut_sides,
};

const NZ: FillRule = FillRule::NonZero;

/// Every operation that sweeps `path` says it is too complex, and says so in milliseconds, not in the seconds of the sweep
/// (a bound for a build for tests on a loaded machine, where the sweep that was admitted took 13 s in a release build).
fn refused_at_once(path: &kurbo::BezPath, label: &str) {
    let start = Instant::now();
    // `try_area` first and alone: where the limit is not there it is a sweep of seconds that answers, and it is said after one.
    assert_eq!(try_area(path, NZ).map(|_| ()), Err(PathOpsError::TooComplex), "{label}");
    assert!(start.elapsed() < Duration::from_secs(60), "{label}: refused in {:?}", start.elapsed());
    let shapes = [Shape::new(path.clone(), NZ, 0)];
    let results = [
        try_unite_all_with(&[(path, NZ)], DEFAULT_PRECISION).map(|_| ()),
        try_normalize(path, NZ).map(|_| ()),
        try_pathfinder_with(PathfinderOp::Intersect, &shapes, DEFAULT_PRECISION).map(|_| ()),
    ];
    assert_eq!(results, [Err(PathOpsError::TooComplex); 3], "{label}");
    assert!(start.elapsed() < Duration::from_secs(60), "{label}: refused in {:?}", start.elapsed());
}

/// 206 stars of 201 points (each edge to the vertex nearly opposite, 19,899 crossings in each), each above and beside the last:
/// 41,818 elements, 4.1 million pairs of bounds (the limit is 4.2 million), 201 pieces on the line, so that the limits on the
/// searches of it (8.3e8 against 4.3e9 and 1.1e9) passed, and 4.1 million crossings, which took 13 s and 6.5 GB
/// (`wasm32` aborts).
#[test]
fn stars_of_lines_stacked_so_that_their_heights_do_not_overlap_are_refused_before_the_sweep() {
    refused_at_once(&stacked_stars(206, 201, 0.0), "206 stars of 201 points");
    // The same crossings in fewer stars: 75 took 4.1 s and 1.8 GB.
    refused_at_once(&stacked_stars(75, 201, 0.0), "75 stars of 201 points");
}

/// Curves were not counted, only lines: 100 such stars whose edges are bowed cubics, 20,300 elements and 2 million places
/// where they cross, took 14.8 s and 3.7 GB.
#[test]
fn stars_of_bowed_cubics_stacked_the_same_way_are_refused_before_the_sweep() {
    refused_at_once(&stacked_stars(100, 201, 3.0), "100 stars of 201 bowed cubics");
}

/// A few of them are swept, of lines and of cubics, and the limit can be set: a stack fits at its crossings and not under them
/// by one (two stars of lines cross in 39,798 places, and each is counted once; the same of cubics counts twice).
#[test]
fn a_few_stacked_stars_are_made_and_the_limit_on_their_crossings_can_be_set() {
    let lines = stacked_stars(2, 201, 0.0);
    let cubics = stacked_stars(2, 201, 3.0);
    for path in [&lines, &cubics] {
        let area = try_area(path, NZ).expect("two stars");
        assert!(area > 0.0);
    }
    let places = 2 * 201 * 99;
    for (limit, made) in [(places, true), (places - 1, false)] {
        let at = with_limits_of(limit);
        assert_eq!(with_limits(at, || try_area(&lines, NZ)).is_ok(), made, "lines, a limit of {limit}");
    }
    // A cubic's place counts for two, and as a bound that is not below the number there are (a third over here), so the
    // same limit refuses the stack of cubics, and four times it does not.
    let at = with_limits_of(places);
    assert_eq!(with_limits(at, || try_area(&cubics, NZ)), Err(PathOpsError::TooComplex));
    let at = with_limits_of(4 * places);
    assert!(with_limits(at, || try_area(&cubics, NZ)).is_ok());
    // A single star's edges that cross are counted as well: 301 points, 44,849 places.
    let star = star_at(301, 0.0, 0.0, 0.0);
    assert!(with_limits(with_limits_of(44_849), || try_area(&star, NZ)).is_ok());
    assert_eq!(with_limits(with_limits_of(44_848), || try_area(&star, NZ)), Err(PathOpsError::TooComplex));
}

fn with_limits_of(places: usize) -> Limits {
    let mut limits = Limits::default();
    limits.real_crossings = places;
    limits
}

/// The default is the constant: 2^20.
#[test]
fn the_default_limit_on_crossings_is_a_million() {
    assert_eq!(MAX_REAL_CROSSINGS, 1 << 20);
    assert_eq!(Limits::default().real_crossings, MAX_REAL_CROSSINGS);
}

/// Lines nearly parallel were taken for lines that cross nowhere (under 1e-10 rad), so a polygon of them passed every count and
/// the sweep took the time of the crossings that were not counted. 450 lines through one point took 25 to 35 s at lengths from
/// 2e7 to 1e10 (and the same where the angles straddle 1e-10: ten tolerances apart); one polygon zigzagging through 2,040 of them
/// 1e10 long, 2.08 million crossings spread along the lines, 14 s and 3.2 GB, and 1,500 of them 1e8 long 6.4 s and 1.6 GB.
#[test]
fn lines_nearly_parallel_that_cross_are_counted_and_refused_before_the_sweep() {
    for (n, l, d, spread) in
        [(450, 2e7, 1.6, false), (450, 1e10, 1.6, false), (450, 1e10, 10.0, false), (2_040, 1e10, 1.6, true), (1_500, 1e8, 1.6, true)]
    {
        refused_at_once(&nearly_parallel_lines(n, l, d, spread, true), &format!("{n} lines {l} long, {d} tolerances apart, spread {spread}"));
    }
}

/// The same lines where they cross nowhere (the ends of one lie on one side of every other) are not counted: they are admitted,
/// as they were, in milliseconds, and the area is the polygon's.
#[test]
fn lines_nearly_parallel_that_do_not_cross_stay_admitted() {
    for (n, l, d) in [(450, 1e10, 1.6), (450, 2e7, 10.0), (2_040, 1e10, 1.6)] {
        let path = nearly_parallel_lines(n, l, d, false, false);
        let start = Instant::now();
        let area = try_area(&path, NZ).expect("a bundle of lines that cross nowhere");
        assert!(area.is_finite() && area > 0.0, "{n} lines: {area}");
        assert!(start.elapsed() < Duration::from_secs(20), "{n} lines {l} long: {:?}", start.elapsed());
    }
}

/// Pies of 256 slices (512 ends at each centre, the limit at one point) stacked on a diagonal, so that their heights do not overlap:
/// the limit on ends at one point holds none of them, and 1, 2, 4, 8 and 30 took 1.3, 2.2, 4.8, 15.7 and 65.8 s (30 are 30,720
/// elements and 2.7 GB). The work of the ends, summed over the points, refuses four and more in milliseconds.
#[test]
fn pies_stacked_are_held_to_the_work_of_their_ends() {
    for count in [4, 8, 30] {
        refused_at_once(&stacked_pies(count, 256), &format!("{count} pies of 256 slices"));
    }
}

/// A circle of 100,000 cubics, radius 1,000, is a path whose joints are exact: the neighbours cost nothing wherever it is, at 1e12
/// (where a handle of 0.021 is of the order of the sweep's tolerance, 0.014) as at the origin. It was `TooComplex` at 1e12.
#[test]
fn a_circle_of_many_cubics_is_made_wherever_it_is() {
    let exact = std::f64::consts::PI * 1e6;
    for (x, y) in [(0.0, 0.0), (1e9, 1e9), (1e12, 1e12)] {
        let area = try_area(&cubic_circle(100_000, 1_000.0, x, y), NZ).unwrap_or_else(|e| panic!("circle at {x}: {e:?}"));
        assert!((area - exact).abs() < 1e-3 * exact, "circle at {x}: {area}");
    }
}

/// Segments that share an end exactly were taken for segments that meet there only, so a fan of them along one line (every two share
/// their start and overlap) passed every limit: 250 down the diagonal took 11 s and down the vertical 9 s (500 are over the limit on
/// the ends at one point, which refuses them at once). Overlapping on one line they are counted as crossing somewhere along.
#[test]
fn fans_of_segments_along_one_line_are_refused_before_the_sweep() {
    refused_at_once(&fan_along_a_line(250, 0), "a fan of 250 segments down the diagonal");
    refused_at_once(&fan_along_a_line(250, 1), "a fan of 250 segments down the vertical");
    refused_at_once(&fan_along_a_line(256, 0), "a fan of 256 segments down the diagonal");
}

/// What is not counted stays admitted: a fan along one level (pieces of no height are not on the sweep line: 250 take 11 ms), the
/// same square or polygon stacked on itself (coincident edges share both ends, and cost what the ends at their corners cost), and
/// sides cut into many segments that meet end to end on one line (level, upright and diagonal).
#[test]
fn what_meets_at_an_end_on_one_line_without_overlapping_stays_admitted() {
    let start = Instant::now();
    let level = try_area(&fan_along_a_line(250, 2), NZ).expect("a fan along one level");
    assert!(level.abs() < 1e-9, "{level}");
    assert!(start.elapsed() < Duration::from_secs(20), "{:?}", start.elapsed());
    let square = try_area(&square_with_cut_sides(2_000), NZ).expect("a square whose sides are cut in 2,000");
    assert!((square - 1e4).abs() < 1e-6, "{square}");
    let strip = try_area(&strip_with_cut_sides(2_000), NZ).expect("a strip whose long sides are cut in 2,000");
    assert!((strip - 1e4).abs() < 1.0, "{strip}");
    let mut copies = kurbo::BezPath::new();
    for _ in 0..100 {
        copies.extend(crate::common::rectangle(kurbo::Rect::new(0.0, 0.0, 10.0, 10.0)).iter());
    }
    let area = try_area(&copies, NZ).expect("100 squares one on another");
    assert!(area.is_finite() && area > 0.0, "{area}");
}

/// The curve forms of the fan: straight cubics from one point down a diagonal (each closed by a line back, 512 segments on one line)
/// counted nothing of work and took 7.8 s as the star of 1,291 points took 5.3 s beside them (12.5 s at the load of the
/// reviewer's); 256 pieces of one arc from its start 7.2 s beside 5.7, and 256 windows of one arc, which share no end, 7.3 s beside
/// 5.9. They run along one another, and cost as lines on one line do.
#[test]
fn curves_that_run_along_one_another_are_refused_before_the_sweep() {
    refused_at_once(&cubic_fan(256), "256 straight cubics from one point down a diagonal");
    refused_at_once(&arc_fan(256), "256 pieces of one arc from its start");
    refused_at_once(&arc_fan(200), "200 pieces of one arc from its start");
    refused_at_once(&arc_windows(256), "256 windows of one arc");
}

/// Fewer of them are made, and what runs along nothing is: 100 straight cubics, 150 pieces of an arc, 160 windows of it (a second
/// or less each).
#[test]
fn a_few_curves_that_run_along_one_another_are_made() {
    for (name, path) in [("100 straight cubics", cubic_fan(100)), ("150 pieces of an arc", arc_fan(150)), ("160 windows of an arc", arc_windows(160))]
    {
        let start = Instant::now();
        assert!(try_area(&path, NZ).is_ok(), "{name}");
        assert!(start.elapsed() < Duration::from_secs(60), "{name}: {:?}", start.elapsed());
    }
}

/// Lines through a point, each moved sideways by a few tolerances (10 to 40: 1e-5 to 4e-5), crossed one another within a few
/// tolerances and slipped past the work of lines at one point (which grouped them within 2): 300 of them took 2.6 GB and aborted
/// under a limit of 3 GB, at 40 tolerances 0.6 s and 400 MB. A pie of 256 slices whose apexes were moved so took 1.1 GB, and 4 of
/// them 4.5 GB, a pie of 400 slices 4.9 GB (the exact pie 88 MB).
#[test]
fn lines_and_pies_moved_a_few_tolerances_are_refused_before_the_sweep() {
    refused_at_once(&jittered_pencil(300, 100.0, 1e-5), "300 lines through a point, moved up to 10 tolerances");
    refused_at_once(&jittered_pencil(300, 100.0, 4e-5), "300 lines through a point, moved up to 40 tolerances");
    refused_at_once(&jittered_pies(4, 256, 2e-5), "4 pies of 256 slices, the apexes moved up to 20 tolerances");
    refused_at_once(&jittered_pies(2, 256, 1e-5), "2 pies of 256 slices, the apexes moved up to 10 tolerances");
    refused_at_once(&jittered_pies(1, 400, 2e-5), "a pie of 400 slices, the apexes moved up to 20 tolerances");
}

/// Moved far enough, or fewer, they are made: 300 lines moved 200 tolerances (0.2 s), 200 moved 10 (0.8 s and 0.8 GB), and a pie
/// of 128 slices moved 10 (0.2 s).
#[test]
fn lines_and_pies_moved_a_few_tolerances_are_made_when_they_are_fewer_or_farther() {
    for (name, path) in [
        ("300 lines moved 200 tolerances", jittered_pencil(300, 100.0, 2e-4)),
        ("200 lines moved 10 tolerances", jittered_pencil(200, 100.0, 1e-5)),
        ("a pie of 128 slices moved 10 tolerances", jittered_pies(1, 128, 1e-5)),
    ] {
        assert!(try_area(&path, NZ).is_ok(), "{name}");
    }
}

/// Stacks of copies cost the sweep more than the ends at their corners were priced at: 230 squares took 9.6 s as the star of 1,291
/// points took 6.4 s beside them, 255 triangles 10 s beside 7, 255 copies of a segment 9.7 s beside 9.1, 21 stacks of 128 copies
/// 21 s beside 5.9, 4 stacks of 200 22 s beside 8.1. Priced as copies, the cube of their ends, each is refused.
#[test]
fn stacks_of_copies_are_refused_before_the_sweep() {
    refused_at_once(&stacked_polygons(4, 230), "230 squares one on another");
    refused_at_once(&stacked_polygons(3, 255), "255 triangles one on another");
    refused_at_once(&stacked_segments(1, 255), "255 copies of a segment");
    refused_at_once(&stacked_segments(21, 128), "21 stacks of 128 copies of a segment");
    refused_at_once(&stacked_segments(4, 200), "4 stacks of 200 copies of a segment");
}

/// And fewer are made, each a little under the limit and under the star: 150 squares (1.0 s as the star took 4.5), 180 (2.9 s),
/// 200 triangles (1.5 s), 200 copies of a segment (2.0 s), 6 stacks of 128 (1.9 s).
#[test]
fn stacks_of_fewer_copies_are_made() {
    for (name, path) in [
        ("150 squares", stacked_polygons(4, 150)),
        ("180 squares", stacked_polygons(4, 180)),
        ("200 triangles", stacked_polygons(3, 200)),
        ("200 copies of a segment", stacked_segments(1, 200)),
        ("6 stacks of 128 copies", stacked_segments(6, 128)),
    ] {
        let start = Instant::now();
        assert!(try_area(&path, NZ).is_ok(), "{name}");
        assert!(start.elapsed() < Duration::from_secs(60), "{name}: {:?}", start.elapsed());
    }
}
