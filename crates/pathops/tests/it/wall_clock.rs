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

/// The dry sweeps that run to the budget hold up to about 2.7 GB (the README, "Known limits"), and these tests run together: they take turns.
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

/// The spine of "The crossings" in the README: `n` segments that go back and forth between (0, 0) and (100, 20), every point moved up by
/// `jitter` times its index modulo 7.
fn back_and_forth(n: usize, jitter: f64) -> BezPath {
    let mut spine = BezPath::new();
    for i in 0..=n {
        let end = (i % 2) as f64;
        let p = (100.0 * end, 20.0 * end + jitter * (i % 7) as f64);
        if i == 0 {
            spine.move_to(p);
        } else {
            spine.line_to(p);
        }
    }
    spine
}

/// The stroke of a spine that goes back and forth over one line is made in seconds, and a long one is refused after a few windows, and not
/// after the dozens that its sweeps were priced at a fifth of (a window took 0.07 of the meter's 4.2 and 0.8 s: 62 of 71 were swept, in
/// 110 s). The stroke of 300 segments took 2.1, 5.4 and 9.5 s with round joins (a tenth, a thousandth and a ten-thousandth of jitter),
/// 1.4 and 1.8 with miters, and the stroke of 18,000 segments 25 s to be refused, in a release build at a load of 8 to 12. In a build with
/// debug assertions it took 5.4, 12, 19 s and 2.6 and 3.0, and 47 s (`polycool` being optimised, as the workspace builds it now: it was 18, 88 and 178 s).
/// The bounds are several times that, for a quiet machine in a release build and for a loaded one in a debug build.
#[test]
#[ignore = "wall-clock: run with cargo test --release -- --include-ignored (or without --release, for the bounds of a debug build)"]
fn a_stroke_that_goes_back_and_forth_is_made_in_seconds_and_a_long_one_refused_in_a_few_windows() {
    let _turn = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    let (each, long) = if cfg!(debug_assertions) {
        (Duration::from_secs(240), Duration::from_secs(300))
    } else {
        (Duration::from_secs(60), Duration::from_secs(90))
    };
    for (join, jitter) in [(Join::Round, 0.1), (Join::Round, 1e-3), (Join::Round, 1e-4), (Join::Miter, 0.1), (Join::Miter, 1e-3), (Join::Bevel, 1e-3)]
    {
        let label = format!("300 segments, {join:?} joins, jitter {jitter:e}");
        let made = timed(&label, each, || try_outline_stroke(&back_and_forth(300, jitter), 6.0, Cap::Butt, join, 4.0));
        assert!(made.is_ok(), "{label}: {made:?}");
    }
    let label = "18,000 segments, miter joins, jitter 1e-3";
    let refused = timed(label, long, || try_outline_stroke(&back_and_forth(18_000, 1e-3), 6.0, Cap::Butt, Join::Miter, 4.0));
    assert_eq!(refused, Err(PathOpsError::TooComplex), "{label}");
}
