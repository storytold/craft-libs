//! The places where segments cross one another are held to their own number, whatever the pieces on the sweep line
//! (`MAX_REAL_CROSSINGS`): the sweep takes about 3 microseconds and 1.6 kilobytes for each, and the count of them times the
//! pieces on the line (`MAX_CROSSED_WORK`) is small where shapes that cross a great deal are stacked so that their heights do
//! not overlap. Curves were not counted at all.

use craft_pathops::*;
use std::time::{Duration, Instant};

use crate::common::{stacked_stars, star_at};

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
