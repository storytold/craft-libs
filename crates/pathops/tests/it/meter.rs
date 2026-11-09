//! The metered dry sweep, the backstop under the pricing of `budget.rs` (`MAX_SWEEP_WORK`, `Limits::sweep_work`): the sweep is run step by
//! step before the operation sweeps, and the input is refused as soon as the work it counts passes the budget.
//!
//! Every shape that passed the pricing in a review of this crate, and the two classes an automated scan of our own commit named
//! (copies moved by a unit in the last place or a few tolerances, and curves that run along one another for a tenth of their
//! length), are refused here by the meter alone: the limits that price a sweep before it is made are lifted, so that only the meter is
//! left to say no. The false-refusal set, the inputs that must be made, is admitted in a release build.

use std::process::Command;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft_pathops::*;
use kurbo::BezPath;

use crate::common::{
    arc_fan, arc_windows, cubic_circle, cubic_fan, fan_along_a_line, jittered_pencil, jittered_pies, nearly_parallel_lines,
    polygons_a_few_tolerances_apart, polygons_an_ulp_apart, stacked_pies, stacked_polygons, stacked_segments, windows_sharing_a_tenth,
};

const NZ: FillRule = FillRule::NonZero;

/// The dry sweeps that run to the budget hold up to a gigabyte, and the test binary runs tests together: those that run one take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Every limit that prices a sweep before it is made is lifted, and the meter is as it is by default: it alone can say no.
fn pricing_off() -> Limits {
    let mut limits = Limits::default();
    limits.segment_pairs = usize::MAX;
    limits.sweep_line_pairs = usize::MAX;
    limits.sweep_event_work = usize::MAX;
    limits.crossed_work = usize::MAX;
    limits.real_crossings = usize::MAX;
    limits.ends_at_a_point = usize::MAX;
    limits.ends_work = usize::MAX;
    limits.crossing_work = usize::MAX;
    limits.coverage_entries = usize::MAX;
    limits
}

/// `path` is refused by the meter, in the time of its budget and not in the minutes the sweep would take.
fn refused_by_the_meter(path: &BezPath, label: &str) {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let start = Instant::now();
    let result = with_limits(pricing_off(), || try_area(path, NZ));
    assert_eq!(result.map(|_| ()), Err(PathOpsError::TooComplex), "{label}");
    assert!(start.elapsed() < Duration::from_secs(60), "{label}: refused in {:?}", start.elapsed());
}

/// Lines through one point or crossing along their length, which passed the count of crossings (the bundles of 450 lines under 1e-10 rad, a
/// polygon zigzagging through 2,040 and through 1,500): 31 to 41 s and 3.2 GB before the pricing counted them.
#[test]
fn pencils_and_zigzags_of_lines_are_refused_by_the_meter_alone() {
    for (n, l, d, spread) in
        [(450, 2e7, 1.6, false), (450, 1e10, 1.6, false), (450, 1e10, 10.0, false), (1_500, 1e8, 1.6, true), (2_040, 1e10, 1.6, true)]
    {
        refused_by_the_meter(&nearly_parallel_lines(n, l, d, spread, true), &format!("{n} lines {l} long, {d} tolerances apart, spread {spread}"));
    }
}

/// Pies of 256 slices stacked on a diagonal (8 took 15.7 s; 4 and 5, 5.3 s, are the pricing's to refuse, and the meter admits them: its work
/// for them is 2.7e9 and 3.3e9), and pies and pencils whose apexes were moved a few tolerances (2.7 to 4.9 GB, aborting under 3).
#[test]
fn stacked_and_jittered_pies_and_pencils_are_refused_by_the_meter_alone() {
    refused_by_the_meter(&stacked_pies(8, 256), "8 pies of 256 slices");
    refused_by_the_meter(&jittered_pies(2, 256, 1e-5), "2 pies of 256 slices, the apexes moved up to 10 tolerances");
    refused_by_the_meter(&jittered_pies(4, 256, 2e-5), "4 pies of 256 slices, the apexes moved up to 20 tolerances");
    refused_by_the_meter(&jittered_pies(1, 400, 2e-5), "a pie of 400 slices, the apexes moved up to 20 tolerances");
    refused_by_the_meter(&jittered_pencil(300, 100.0, 1e-5), "300 lines through a point, moved up to 10 tolerances");
    refused_by_the_meter(&jittered_pencil(256, 100.0, 2e-6), "256 lines through a point, moved up to 2 tolerances");
}

/// Fans of lines and of cubics from one point down a diagonal, and windows of one arc (7 to 11 s beside the star's 5 to 6).
#[test]
fn fans_and_arc_windows_are_refused_by_the_meter_alone() {
    refused_by_the_meter(&fan_along_a_line(250, 0), "a fan of 250 segments down the diagonal");
    refused_by_the_meter(&fan_along_a_line(256, 0), "a fan of 256 segments down the diagonal");
    refused_by_the_meter(&cubic_fan(256), "256 straight cubics from one point down a diagonal");
    refused_by_the_meter(&arc_fan(256), "256 pieces of one arc from its start");
    refused_by_the_meter(&arc_fan(200), "200 pieces of one arc from its start");
    refused_by_the_meter(&arc_windows(256), "256 windows of one arc");
}

/// Copies of a shape one on another: squares, triangles, a segment (9 to 22 s beside the star's 6 to 8).
#[test]
fn stacked_squares_triangles_and_copies_of_a_segment_are_refused_by_the_meter_alone() {
    refused_by_the_meter(&stacked_polygons(4, 230), "230 squares one on another");
    refused_by_the_meter(&stacked_polygons(3, 255), "255 triangles one on another");
    refused_by_the_meter(&stacked_segments(1, 255), "255 copies of a segment");
    refused_by_the_meter(&stacked_segments(21, 128), "21 stacks of 128 copies of a segment");
    refused_by_the_meter(&stacked_segments(4, 200), "4 stacks of 200 copies of a segment");
}

/// Found by an automated scan of our own commit, and then in our own testing: copies were found by the bits of their points, so copies
/// moved by one unit in the last place, or by a fraction of a tolerance, were none, and the ends of short segments (under 256 tolerances)
/// were priced as a pie's, a third of what copies cost: 220 squares of 1e-4 across, each a unit in the last place from the last, took 6.2 s,
/// 240 such triangles about 6 s, and 200 squares moved by tenths of a tolerance 5.4 s, where the star of 1,291 points took 4 to 5.
#[test]
fn copies_one_ulp_or_a_fraction_of_a_tolerance_apart_are_refused_by_the_meter_alone() {
    refused_by_the_meter(&polygons_an_ulp_apart(4, 220, 1e-4), "220 small squares, each a unit in the last place from the last");
    refused_by_the_meter(&polygons_an_ulp_apart(3, 240, 1e-4), "240 small triangles, each a unit in the last place from the last");
    refused_by_the_meter(&polygons_an_ulp_apart(4, 255, 100.0), "255 squares, each a unit in the last place from the last");
    refused_by_the_meter(&polygons_a_few_tolerances_apart(4, 220, 0.15, 1e-4), "220 small squares moved by tenths of a tolerance");
    refused_by_the_meter(&polygons_a_few_tolerances_apart(4, 255, 1.0, 100.0), "255 squares, moved by up to 6 and 4 tolerances");
}

/// Found by the same scan: where curves run along one another was looked for among 24 of the curves whose tangents may, over a stretch of
/// a quarter of the one, so windows of an arc that share a tenth of it, in a clique, were counted nothing.
#[test]
fn windows_that_share_a_tenth_of_an_arc_are_refused_by_the_meter_alone() {
    refused_by_the_meter(&windows_sharing_a_tenth(300), "300 windows of an arc that share a tenth of it");
    refused_by_the_meter(&windows_sharing_a_tenth(400), "400 windows of an arc that share a tenth of it");
}

/// What the meter refuses is refused by every operation that sweeps, as the pricing's refusals are.
#[test]
fn every_operation_that_sweeps_is_refused_by_the_meter() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let path = jittered_pencil(300, 100.0, 1e-5);
    let shapes = [Shape::new(path.clone(), NZ, 0)];
    let results = with_limits(pricing_off(), || {
        [
            try_unite_all_with(&[(&path, NZ)], DEFAULT_PRECISION).map(|_| ()),
            try_normalize(&path, NZ).map(|_| ()),
            try_pathfinder_with(PathfinderOp::Intersect, &shapes, DEFAULT_PRECISION).map(|_| ()),
            try_pathfinder_with(PathfinderOp::Divide, &shapes, DEFAULT_PRECISION).map(|_| ()),
            try_shape_builder(&shapes, false).map(|_| ()),
        ]
    });
    assert_eq!(results, [Err(PathOpsError::TooComplex); 5]);
}

/// The budget is a field of `Limits`: a lower one refuses what the default makes, a lifted one leaves the meter off and the sweep is made.
#[test]
fn the_budget_is_a_limit_that_can_be_set() {
    assert_eq!(Limits::default().sweep_work, MAX_SWEEP_WORK);
    let path = crate::common::star_at(301, 0.0, 0.0, 0.0);
    assert!(try_area(&path, NZ).is_ok());
    let mut low = Limits::default();
    low.sweep_work = 1_000_000;
    assert_eq!(with_limits(low, || try_area(&path, NZ)).map(|_| ()), Err(PathOpsError::TooComplex));
    let mut off = Limits::default();
    off.sweep_work = usize::MAX;
    assert!(with_limits(off, || try_area(&path, NZ)).is_ok());
}

/// A planar map takes several times what its sweep does, in time and memory (the shape builder of a star of 501 points took 4.7 s and 1.3 GB
/// where the sweep took 0.4 s, and one of 801 aborted under a limit of 3 GB, its sweep 1.5 s), so it is built for a quarter of the pieces a
/// sweep is: the sweep of the star of 801 is made, its map is refused, in the time of the dry sweep and not of the map.
#[test]
fn a_planar_map_is_built_for_a_quarter_of_what_a_sweep_is_made_for() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let big = crate::common::star_at(801, 0.0, 0.0, 0.0);
    assert!(try_area(&big, NZ).is_ok(), "the sweep of a star of 801 points is made");
    let shapes = [Shape::new(big, NZ, 0)];
    let start = Instant::now();
    assert_eq!(try_shape_builder(&shapes, false).map(|_| ()), Err(PathOpsError::TooComplex));
    assert!(start.elapsed() < Duration::from_secs(20), "refused in {:?}", start.elapsed());
    let small = [Shape::new(crate::common::star_at(201, 0.0, 0.0, 0.0), NZ, 0)];
    assert!(try_shape_builder(&small, false).is_ok(), "the map of a star of 201 points is built");
}

/// The cases a child process runs under a limit of 3 GB of address space: the most memory the meter has been seen to hold at a refusal.
fn memory_cases() -> Vec<(&'static str, BezPath)> {
    vec![
        ("2040 lines crossing along their length", nearly_parallel_lines(2_040, 1e10, 1.6, true, true)),
        ("1500 lines crossing along their length", nearly_parallel_lines(1_500, 1e8, 1.6, true, true)),
        ("4 pies of 256 slices, apexes moved", jittered_pies(4, 256, 2e-5)),
        ("a pie of 400 slices, apexes moved", jittered_pies(1, 400, 2e-5)),
        ("300 lines through a point, moved", jittered_pencil(300, 100.0, 1e-5)),
        ("8 pies of 256 slices", stacked_pies(8, 256)),
        ("400 windows sharing a tenth", windows_sharing_a_tenth(400)),
        ("220 small squares moved by tenths of a tolerance", polygons_a_few_tolerances_apart(4, 220, 0.15, 1e-4)),
        ("240 small triangles an ulp apart", polygons_an_ulp_apart(3, 240, 1e-4)),
    ]
}

/// A child of [`a_refusal_holds_memory_under_a_limit_of_three_gigabytes`]: it runs the case named by the environment, and says what it came to.
/// Alone, with no case named, it does nothing.
#[test]
fn child_case() {
    let Ok(name) = std::env::var("PATHOPS_METER_CASE") else { return };
    let (_, path) = memory_cases().into_iter().find(|(n, _)| *n == name).expect("a case of the list");
    let result = with_limits(pricing_off(), || try_area(&path, NZ).map(|_| ()));
    println!("CASE-RESULT {name}: {result:?}");
}

/// The meter refuses each case in a child process under `ulimit -v 3000000` (a limit of 3 GB of address space, as the earlier reviews
/// measured under), cleanly: the child does not abort, and its answer is `TooComplex`.
#[cfg(unix)]
#[test]
fn a_refusal_holds_memory_under_a_limit_of_three_gigabytes() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let exe = std::env::current_exe().expect("the test binary");
    for (name, _) in memory_cases() {
        let output = Command::new("sh")
            .arg("-c")
            .arg("ulimit -v 3000000 && exec \"$0\" --exact meter::child_case --nocapture --test-threads=1")
            .arg(&exe)
            .env("PATHOPS_METER_CASE", name)
            .output()
            .expect("a shell");
        let text = String::from_utf8_lossy(&output.stdout);
        assert!(output.status.success(), "{name}: {:?}\n{}\n{}", output.status, text, String::from_utf8_lossy(&output.stderr));
        assert!(text.contains(&format!("CASE-RESULT {name}: Err(TooComplex)")), "{name}: {text}");
    }
}

/// The inputs that must be made: rings, S-strokes, a hatch of 400, a circle of 100,000 cubics at 0, 1e9 and 1e12, 250 and 1,000 bars on one
/// baseline, meshes of 140 and 170 cells a side, grids of squares of 200 and 250, 2,000 pies of 30 slices, and the star of 1,291 points.
/// Seconds and up to 2.3 GB: in a release build only.
#[cfg_attr(debug_assertions, ignore = "seconds and gigabytes: run with cargo test --release")]
#[test]
fn the_false_refusal_set_is_admitted() {
    use crate::common::{polygon_path, stacked_stars};
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let cases: Vec<(&str, BezPath)> = vec![
        ("a star of 1,291 points", stacked_stars(1, 1_291, 0.0)),
        ("a circle of 100,000 cubics at the origin", cubic_circle(100_000, 1_000.0, 0.0, 0.0)),
        ("a circle of 100,000 cubics at 1e9", cubic_circle(100_000, 1_000.0, 1e9, 1e9)),
        ("a circle of 100,000 cubics at 1e12", cubic_circle(100_000, 1_000.0, 1e12, 1e12)),
        ("2,000 pies of 30 slices", pies_of(2_000, 30)),
        ("a polygon of 262,000 sides", polygon_path(&many_sided(262_000))),
    ];
    for (name, path) in cases {
        let start = Instant::now();
        assert!(try_area(&path, NZ).is_ok(), "{name}");
        eprintln!("admitted: {name}: {:?}", start.elapsed());
    }
}

/// `count` pies of `k` slices, each pie 250 from the last on a diagonal (as the stacked pies, but small).
fn pies_of(count: usize, k: usize) -> BezPath {
    stacked_pies(count, k)
}

/// A polygon of `n` sides round a wavy circle.
fn many_sided(n: usize) -> Vec<kurbo::Point> {
    (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * i as f64 / n as f64;
            let r = 1000.0 + 30.0 * (a * 37.0).sin();
            kurbo::Point::new(r * a.cos(), r * a.sin())
        })
        .collect()
}
