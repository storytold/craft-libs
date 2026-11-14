//! Real-time checks of the worst inputs, ignored by default.
//!
//! The tests of the limits and of the meter check what is refused and what is made, and the crate's own tests count the work each does
//! (`src/refusals.rs`, `src/budget.rs`, `src/meter.rs`: sweeps built, pairs of bounds walked, work the meter counted). They no longer
//! time anything, because a clock measures the machine's load as much as the crate's work: a bound that holds on a quiet machine
//! fails on a loaded one, and a bound loose enough for a loaded one catches nothing.
//!
//! These checks keep a clock for the worst inputs, as a check to run now and then on a quiet machine, in a release build:
//!
//! `cargo test --release -p craft-pathops --test it wall_clock -- --include-ignored`
//!
//! The bounds are several times what a release build measured on one machine (0.13 s for the worst pricing refusal, 1.2 s for the
//! worst meter refusal, 7.6 s for the worst input admitted, at a load of 15 on 32 cores; each is printed): they catch a regression of
//! an order of magnitude, and are not a promise for any other machine.

use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use craft_pathops::*;
use kurbo::BezPath;

use crate::common::{
    arc_fan, circle_pairs, jittered_pencil, nearly_parallel_lines, stacked_pies, stacked_polygons, stacked_segments, stacked_stars, star_at,
};

const NZ: FillRule = FillRule::NonZero;

/// The dry sweeps that run to the budget hold up to a gigabyte, and these tests run together: they take turns.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

/// Every limit that prices a sweep before it is made is lifted: the meter alone can say no.
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

/// Runs `f`, prints how long it took, and says whether that was within `bound`.
fn timed<T>(label: &str, bound: Duration, f: impl FnOnce() -> T) -> T {
    let start = Instant::now();
    let out = f();
    let took = start.elapsed();
    eprintln!("wall clock: {label}: {took:?} (bound {bound:?})");
    assert!(took < bound, "{label}: {took:?} against a bound of {bound:?}");
    out
}

/// What the pricing refuses is refused in a fraction of a second, and not after the seconds of the sweep (the star stack's sweep took 13 s).
#[test]
#[ignore = "wall-clock: run with cargo test --release -- --include-ignored"]
fn the_pricing_refuses_the_worst_inputs_in_a_fraction_of_a_second() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let cases: Vec<(&str, BezPath)> = vec![
        ("206 stars of 201 points", stacked_stars(206, 201, 0.0)),
        ("2,040 lines 1e10 long, spread", nearly_parallel_lines(2_040, 1e10, 1.6, true, true)),
        ("30 pies of 256 slices", stacked_pies(30, 256)),
        ("4 stacks of 200 copies of a segment", stacked_segments(4, 200)),
        ("230 squares one on another", stacked_polygons(4, 230)),
    ];
    for (label, path) in cases {
        let result = timed(label, Duration::from_secs(2), || try_area(&path, NZ).map(|_| ()));
        assert_eq!(result, Err(PathOpsError::TooComplex), "{label}");
    }
}

/// What the meter alone refuses is refused in about the time of its budget (4.2e9 nanoseconds of sweep), and not in the minutes the
/// sweep would take; a planar map past its quarter of the budget is refused as quickly.
#[test]
#[ignore = "wall-clock: run with cargo test --release -- --include-ignored"]
fn the_meter_refuses_the_worst_inputs_in_about_the_time_of_its_budget() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let cases: Vec<(&str, BezPath)> = vec![
        ("300 lines through a point, moved", jittered_pencil(300, 100.0, 1e-5)),
        ("8 pies of 256 slices", stacked_pies(8, 256)),
        ("256 pieces of one arc", arc_fan(256)),
        ("2,048 pairs of circles of a million", circle_pairs(2_048, 1e6, 0.03)),
    ];
    for (label, path) in cases {
        let result = timed(label, Duration::from_secs(10), || with_limits(pricing_off(), || try_area(&path, NZ).map(|_| ())));
        assert_eq!(result, Err(PathOpsError::TooComplex), "{label}");
    }
    let shapes = [Shape::new(star_at(801, 0.0, 0.0, 0.0), NZ, 0)];
    let result = timed("the map of a star of 801 points", Duration::from_secs(10), || try_shape_builder(&shapes, false).map(|_| ()));
    assert_eq!(result, Err(PathOpsError::TooComplex));
}

/// The worst inputs that are admitted are made in seconds: a star of 1,001 points (2.4 s of sweep on a quiet machine) and 180 squares one
/// on another (2.9 s).
#[test]
#[ignore = "wall-clock: run with cargo test --release -- --include-ignored"]
fn the_worst_inputs_admitted_are_made_in_seconds() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let cases: Vec<(&str, BezPath)> = vec![
        ("a star of 1,001 points", star_at(1_001, 0.0, 0.0, 0.0)),
        ("180 squares one on another", stacked_polygons(4, 180)),
        ("150 pieces of an arc", arc_fan(150)),
    ];
    for (label, path) in cases {
        let area = timed(label, Duration::from_secs(30), || try_area(&path, NZ));
        assert!(area.is_ok(), "{label}: {area:?}");
    }
}
