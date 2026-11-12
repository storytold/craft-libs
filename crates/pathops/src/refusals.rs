// Written for craft-pathops. MIT OR Apache-2.0.

//! What the integration tests (`tests/it`) said with a clock, said with counts.
//!
//! Those tests check that an input too tangled to sweep is refused, and that the worst input still admitted is made, through the public
//! functions. They once asserted how long that took ("refused in milliseconds", "made in under a minute"), and a clock measures the
//! machine's load as much as the crate's work: the assertion failed on a loaded machine for a refusal that had been fast. What the
//! clock stood for is counted here, for the same inputs, with the counts of [`crate::work`] that an integration test cannot read:
//!
//! - a refusal by the pricing (`budget.rs`) is made from the geometry, before any sweep is built or metered, after walking at most
//!   [`MAX_SEGMENT_PAIRS`] pairs of bounds;
//! - a refusal by the meter (`meter.rs`) is made before any sweep is built, from a dry sweep that stops as soon as the work it
//!   counted passes the budget (or from the counts made before it starts, which are the work counted);
//! - an input that is admitted passes the pricing, and the work the meter counted for it is within the budget;
//! - a planar map refused for being past its quarter of the budget is refused by the dry sweep, before the map's own sweep is built.
//!
//! The integration tests keep what they assert about results, and have a real-time twin that is ignored by default
//! (`tests/it/wall_clock.rs`).

use std::sync::{Mutex, PoisonError};

use kurbo::BezPath;

use crate::budget::{MAX_SEGMENT_PAIRS, MAX_SWEEP_WORK};
use crate::meter::MAP_FACTOR;
use crate::testutil::{
    arc_fan, arc_windows, circle_pairs, cubic_fan, fan_along_a_line, jittered_pencil, jittered_pies, nearly_parallel_lines,
    polygons_a_few_tolerances_apart, polygons_an_ulp_apart, stacked_pies, stacked_polygons, stacked_segments, stacked_stars, star,
    windows_sharing_a_tenth,
};
use crate::work::{self, Work};
use crate::{DEFAULT_PRECISION, FillRule, Limits, PathOpsError, PathfinderOp, Shape, with_limits};

const NZ: FillRule = FillRule::NonZero;

/// Inputs, each with the words that say which it is.
type Cases = Vec<(String, BezPath)>;

/// The dry sweeps that run to the budget hold up to a gigabyte, and the tests of the crate run together: those that run one take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Every limit that prices a sweep before it is made is lifted, and the meter is as it is by default: it alone can say no.
fn pricing_off() -> Limits {
    Limits {
        segment_pairs: usize::MAX,
        sweep_line_pairs: usize::MAX,
        sweep_event_work: usize::MAX,
        crossed_work: usize::MAX,
        real_crossings: usize::MAX,
        ends_at_a_point: usize::MAX,
        ends_work: usize::MAX,
        crossing_work: usize::MAX,
        coverage_entries: usize::MAX,
        ..Limits::default()
    }
}

/// The inputs that the pricing refuses, each as a test of `tests/it/crossings.rs` or `tests/it/review10.rs` makes them.
fn refused_by_the_pricing() -> Cases {
    let mut v: Cases = vec![
        ("206 stars of 201 points".into(), stacked_stars(206, 201, 0.0)),
        ("75 stars of 201 points".into(), stacked_stars(75, 201, 0.0)),
        ("100 stars of 201 bowed cubics".into(), stacked_stars(100, 201, 3.0)),
        ("a fan of 250 segments down the diagonal".into(), fan_along_a_line(250)),
        ("a fan of 256 segments down the diagonal".into(), fan_along_a_line(256)),
        ("256 straight cubics from one point down a diagonal".into(), cubic_fan(256)),
        ("256 pieces of one arc from its start".into(), arc_fan(256)),
        ("200 pieces of one arc from its start".into(), arc_fan(200)),
        ("256 windows of one arc".into(), arc_windows(256)),
        ("300 lines through a point, moved up to 10 tolerances".into(), jittered_pencil(300, 100.0, 1e-5)),
        ("300 lines through a point, moved up to 40 tolerances".into(), jittered_pencil(300, 100.0, 4e-5)),
        ("4 pies of 256 slices, the apexes moved up to 20 tolerances".into(), jittered_pies(4, 256, 2e-5)),
        ("2 pies of 256 slices, the apexes moved up to 10 tolerances".into(), jittered_pies(2, 256, 1e-5)),
        ("a pie of 400 slices, the apexes moved up to 20 tolerances".into(), jittered_pies(1, 400, 2e-5)),
        ("230 squares one on another".into(), stacked_polygons(4, 230)),
        ("255 triangles one on another".into(), stacked_polygons(3, 255)),
        ("255 copies of a segment".into(), stacked_segments(1, 255)),
        ("21 stacks of 128 copies of a segment".into(), stacked_segments(21, 128)),
        ("4 stacks of 200 copies of a segment".into(), stacked_segments(4, 200)),
    ];
    for (n, l, d, spread) in
        [(450, 2e7, 1.6, false), (450, 1e10, 1.6, false), (450, 1e10, 10.0, false), (2_040, 1e10, 1.6, true), (1_500, 1e8, 1.6, true)]
    {
        v.push((format!("{n} lines {l} long, {d} tolerances apart, spread {spread}"), nearly_parallel_lines(n, l, d, spread, true)));
    }
    for count in [4, 8, 30] {
        v.push((format!("{count} pies of 256 slices"), stacked_pies(count, 256)));
    }
    for n in [2_001, 3_001, 4_001] {
        v.push((format!("a polygon of {n} edges that all cross"), star(n)));
    }
    v
}

/// The inputs the meter alone refuses (the pricing is lifted), each as the tests of `tests/it/meter.rs` that run in every build make them:
/// the ones the dry sweep stops on as it goes, and the ones whose counts before it starts are past the budget already.
fn refused_by_the_meter() -> (Cases, Cases) {
    let mut as_it_goes: Cases = Vec::new();
    for (n, l, d, spread) in
        [(450, 2e7, 1.6, false), (450, 1e10, 1.6, false), (450, 1e10, 10.0, false), (1_500, 1e8, 1.6, true), (2_040, 1e10, 1.6, true)]
    {
        as_it_goes.push((format!("{n} lines {l} long, {d} tolerances apart, spread {spread}"), nearly_parallel_lines(n, l, d, spread, true)));
    }
    as_it_goes.push(("2 pies of 256 slices, the apexes moved up to 10 tolerances".into(), jittered_pies(2, 256, 1e-5)));
    as_it_goes.push(("4 pies of 256 slices, the apexes moved up to 20 tolerances".into(), jittered_pies(4, 256, 2e-5)));
    as_it_goes.push(("a pie of 400 slices, the apexes moved up to 20 tolerances".into(), jittered_pies(1, 400, 2e-5)));
    as_it_goes.push(("300 lines through a point, moved up to 10 tolerances".into(), jittered_pencil(300, 100.0, 1e-5)));
    as_it_goes.push(("256 lines through a point, moved up to 2 tolerances".into(), jittered_pencil(256, 100.0, 2e-6)));
    as_it_goes.push(("200 pieces of one arc from its start".into(), arc_fan(200)));
    for (n, r, d) in [(1_024, 1e6, 0.03), (2_048, 1e6, 0.03), (6_000, 1e6, 0.03), (1_024, 1e5, 0.003), (6_000, 1e5, 0.3), (6_000, 1e6, 3.0)] {
        as_it_goes.push((format!("{n} pairs of circles of {r}, {d} apart"), circle_pairs(n, r, d)));
    }
    let before_the_sweep: Cases = vec![
        ("8 pies of 256 slices".into(), stacked_pies(8, 256)),
        ("a fan of 250 segments down the diagonal".into(), fan_along_a_line(250)),
        ("a fan of 256 segments down the diagonal".into(), fan_along_a_line(256)),
        ("256 straight cubics from one point down a diagonal".into(), cubic_fan(256)),
        ("256 pieces of one arc from its start".into(), arc_fan(256)),
        ("256 windows of one arc".into(), arc_windows(256)),
        ("230 squares one on another".into(), stacked_polygons(4, 230)),
        ("255 triangles one on another".into(), stacked_polygons(3, 255)),
        ("255 copies of a segment".into(), stacked_segments(1, 255)),
        ("21 stacks of 128 copies of a segment".into(), stacked_segments(21, 128)),
        ("4 stacks of 200 copies of a segment".into(), stacked_segments(4, 200)),
        ("220 small squares, each a unit in the last place from the last".into(), polygons_an_ulp_apart(4, 220, 1e-4)),
        ("240 small triangles, each a unit in the last place from the last".into(), polygons_an_ulp_apart(3, 240, 1e-4)),
        ("255 squares, each a unit in the last place from the last".into(), polygons_an_ulp_apart(4, 255, 100.0)),
        ("220 small squares moved by tenths of a tolerance".into(), polygons_a_few_tolerances_apart(4, 220, 0.15, 1e-4)),
        ("255 squares, moved by up to 6 and 4 tolerances".into(), polygons_a_few_tolerances_apart(4, 255, 1.0, 100.0)),
        ("300 windows of an arc that share a tenth of it".into(), windows_sharing_a_tenth(300)),
        ("400 windows of an arc that share a tenth of it".into(), windows_sharing_a_tenth(400)),
    ];
    (as_it_goes, before_the_sweep)
}

/// Every operation that sweeps `path` says it is too complex, with no sweep built (nor metered, which is the next check) and no more
/// pairs of bounds walked than the limit on them: the work of a refusal, counted.
fn refused_by_the_pricing_at_once(path: &BezPath, label: &str) {
    work::reset();
    assert_eq!(crate::try_area(path, NZ).map(|_| ()), Err(PathOpsError::TooComplex), "{label}");
    let pairs = work::take(Work::CrossingPairs);
    assert!(pairs <= MAX_SEGMENT_PAIRS, "{label}: {pairs} pairs of bounds were walked");
    assert_eq!(
        (work::take(Work::Sweeps), work::take(Work::MeteredSweeps), work::take(Work::MeterNanos)),
        (0, 0, 0),
        "{label}: refused before any sweep"
    );
    let shapes = [Shape::new(path.clone(), NZ, 0)];
    let results = [
        crate::try_unite_all_with(&[(path, NZ)], DEFAULT_PRECISION).map(|_| ()),
        crate::try_normalize(path, NZ).map(|_| ()),
        crate::try_pathfinder_with(PathfinderOp::Intersect, &shapes, DEFAULT_PRECISION).map(|_| ()),
    ];
    assert_eq!(results, [Err(PathOpsError::TooComplex); 3], "{label}");
    assert_eq!((work::take(Work::Sweeps), work::take(Work::MeteredSweeps)), (0, 0), "{label}: every operation refuses it before any sweep");
}

#[test]
fn the_pricing_refuses_what_the_integration_tests_expect_refused_before_any_sweep() {
    for (label, path) in refused_by_the_pricing() {
        refused_by_the_pricing_at_once(&path, &label);
    }
}

/// The meter refuses `path` (alone: the pricing is lifted) with no sweep built, from one dry sweep, which has counted more than the budget
/// by the time it says so. Where it stops as it goes, the work it counted is the budget and what one step adds (under a hundredth more): it
/// did not run to the end, which is minutes.
fn refused_by_the_meter_alone(path: &BezPath, label: &str, as_it_goes: bool) {
    let budget = MAX_SWEEP_WORK as u64;
    work::reset();
    let result = with_limits(pricing_off(), || crate::try_area(path, NZ).map(|_| ()));
    assert_eq!(result, Err(PathOpsError::TooComplex), "{label}");
    assert_eq!(work::take(Work::Sweeps), 0, "{label}: refused before the sweep was built");
    assert_eq!(work::take(Work::MeteredSweeps), 1, "{label}: one dry sweep");
    let counted = work::take(Work::MeterNanos) as u64;
    assert!(counted > budget, "{label}: counted {counted}, refused at a budget of {budget}");
    if as_it_goes {
        assert!(counted <= budget + budget / 100, "{label}: counted {counted}, past the budget of {budget} by more than a step");
    }
}

#[test]
fn the_meter_refuses_what_the_integration_tests_expect_refused_by_it_before_any_sweep() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let (as_it_goes, before_the_sweep) = refused_by_the_meter();
    for (label, path) in &as_it_goes {
        refused_by_the_meter_alone(path, label, true);
    }
    for (label, path) in &before_the_sweep {
        refused_by_the_meter_alone(path, label, false);
    }
}

/// What the integration tests expect made: each passes the pricing and the meter (its dry sweep counts within the budget).
fn admitted() -> Cases {
    let mut v: Cases = vec![
        ("100 straight cubics".into(), cubic_fan(100)),
        ("150 pieces of an arc".into(), arc_fan(150)),
        ("160 windows of an arc".into(), arc_windows(160)),
        ("150 squares".into(), stacked_polygons(4, 150)),
        ("180 squares".into(), stacked_polygons(4, 180)),
        ("200 triangles".into(), stacked_polygons(3, 200)),
        ("200 copies of a segment".into(), stacked_segments(1, 200)),
        ("6 stacks of 128 copies".into(), stacked_segments(6, 128)),
        ("300 lines moved 200 tolerances".into(), jittered_pencil(300, 100.0, 2e-4)),
        ("150 lines moved 10 tolerances".into(), jittered_pencil(150, 100.0, 1e-5)),
        ("a pie of 128 slices moved 10 tolerances".into(), jittered_pies(1, 128, 1e-5)),
        ("a polygon of 401 edges that all cross".into(), star(401)),
    ];
    for (n, l, d) in [(450, 1e10, 1.6), (450, 2e7, 10.0), (2_040, 1e10, 1.6)] {
        v.push((format!("{n} lines {l} long that cross nowhere, {d} tolerances apart"), nearly_parallel_lines(n, l, d, false, false)));
    }
    // 1,001 edges take seconds of sweep: a release build only, as the integration test.
    if !cfg!(debug_assertions) {
        v.push(("a polygon of 1,001 edges that all cross".into(), star(1_001)));
    }
    v
}

#[test]
fn what_the_integration_tests_expect_made_is_admitted_by_the_pricing_and_within_the_budget_of_the_meter() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    for (label, path) in admitted() {
        // The two filters that stand before the sweep (the pricing, then the meter), and the work the meter counted for it.
        assert!(crate::budget::fits_elements([&path]), "{label}: the elements");
        let eps = crate::boolean::eps_for([&path]).expect("finite");
        assert!(crate::budget::sweep_fits([&path]), "{label}: the pricing admits it");
        work::reset();
        assert_eq!(crate::meter::within([&path], eps), Ok(()), "{label}: the meter admits it");
        let counted = work::take(Work::MeterNanos);
        assert!(counted <= MAX_SWEEP_WORK, "{label}: the meter counted {counted} of a budget of {MAX_SWEEP_WORK}");
    }
}

/// A planar map takes several times what its sweep does, so it is held to a quarter of the budget (`MAP_FACTOR`): the sweep of the star of
/// 801 points is made (the dry sweep counts it within the budget, past a quarter of it), and its map is refused, by the dry sweep, as it passes
/// the quarter, and before the map's own sweep is built, which is the time and the memory of the map.
#[test]
fn a_planar_map_past_its_quarter_of_the_budget_is_refused_by_the_dry_sweep_before_its_own_sweep() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let (budget, quarter) = (MAX_SWEEP_WORK as u64, MAX_SWEEP_WORK as u64 / MAP_FACTOR);
    let big = star(801);
    work::reset();
    assert!(crate::try_area(&big, NZ).is_ok(), "the sweep of a star of 801 points is made");
    assert_eq!(work::take(Work::Sweeps), 1);
    let counted = work::take(Work::MeterNanos) as u64;
    assert!(counted > quarter && counted <= budget, "{counted} against a budget of {budget}, of which a map has {quarter}");
    let shapes = [Shape::new(big, NZ, 0)];
    work::reset();
    assert_eq!(crate::try_shape_builder(&shapes, false).map(|_| ()), Err(PathOpsError::TooComplex));
    assert_eq!(work::take(Work::Sweeps), 0, "refused before the map's sweep was built");
    assert_eq!(work::take(Work::MeteredSweeps), 1, "by the dry sweep");
    let counted = work::take(Work::MeterNanos) as u64;
    assert!(counted > quarter && counted <= quarter + quarter / 100, "{counted} against a quarter of the budget, {quarter}");
    // A star of 201 points is under it, and its map is built.
    let small = [Shape::new(star(201), NZ, 0)];
    work::reset();
    assert!(crate::try_shape_builder(&small, false).is_ok(), "the map of a star of 201 points is built");
    assert!(work::take(Work::Sweeps) >= 1, "its sweep was built");
}
