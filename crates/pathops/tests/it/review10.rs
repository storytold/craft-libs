//! Claims made in the tenth review of the pull request, each as the smallest test that fails when the
//! claim is true.

use craft_pathops::*;
use std::time::{Duration, Instant};

use kurbo::{BezPath, Shape as _};

use crate::common::poly;

const NZ: FillRule = FillRule::NonZero;

// ---- interior points between slanted edges

/// A parallelogram `width` wide and `height` high whose sides slope by `slant` in x for each unit of y, at `x`.
fn parallelogram(x: f64, width: f64, height: f64, slant: f64) -> BezPath {
    poly(&[(x, 0.0), (x + width, 0.0), (x + width + slant * height, height), (x + slant * height, height)], true)
}

/// The two crossings of a slanted pair of sides were each allowed 0.9 for rounding at x = 1e15 (eight times
/// the unit in the last place there, 0.125), so a gap under 1.8 wide was a retrace and not a part: a
/// parallelogram a unit wide, with sides slanted by 0.125, had no point. The vertical sides of a square were
/// excepted from that already; slanted ones cross at a computed x, which is off by a few units in the last
/// place of it, not eight each.
#[test]
fn a_slanted_parallelogram_a_unit_wide_at_a_quadrillion_has_a_point() {
    // Eight units in the last place wide: a unit at 1e15 (0.125 each) and 4 at 4e15 (0.5 each).
    for (x, width, slant) in [(1e15, 1.0, 0.125), (1e15, 1.0, 0.25), (4e15, 4.0, 0.5), (1e14, 0.1, 0.1), (1e15, 1.0, -0.125)] {
        for height in [1.0, 8.0, 1000.0] {
            let shape = parallelogram(x, width, height, slant);
            let p =
                interior_point(&shape).unwrap_or_else(|| panic!("no point in a parallelogram {width} wide at x = {x}, slant {slant}, {height} high"));
            assert!(shape.winding(p) != 0, "{p:?} is not inside it");
            // Strictly: between the sides at its height, and between its top and bottom.
            let (left, right) = (x + slant * p.y, x + width + slant * p.y);
            assert!(p.x > left && p.x < right && p.y > 0.0 && p.y < height, "{p:?} is not strictly inside: sides at {left} and {right}");
        }
    }
}

/// What the allowance is for: out and back along a slanted line, through points that are rounded each way,
/// encloses nothing, at any x and y (the gaps between its two passes are the rounding of its points: a unit in the
/// last place of x or so, and where the line is steep in x and high up in y a good deal more: a hundred times the
/// unit of y at a slope of a hundred, or a tenth of the 0.125 of y = 1e15 at a slope of 0.3, where a unit of x is
/// 3.5e-15). It was found a point of, by 9 variants of 20 at (1e8, 1e8) with a slope of 100, by 17 at (0, 1e15) with a
/// slope of 0.3, and by 19 with one of 10.
#[test]
fn a_retrace_along_a_slanted_line_has_no_point_wherever_it_is() {
    for (x0, y0, slope) in [
        (0.0, 0.0, 0.3),
        (1e8, 1e8, 3.0),
        (1e8, 1e8, 100.0),
        (1e15, 0.0, 0.3),
        (1e15, 1e15, 10.0),
        (0.0, 1e15, 0.3),
        (0.0, 1e15, 10.0),
        (4e15, 4e15, 100.0),
    ] {
        for variant in 0..20 {
            let shift = f64::from(variant) * 0.0371 * 0.01;
            let out: Vec<(f64, f64)> = (0..=100).map(|y| (x0 + slope * f64::from(y), y0 + f64::from(y))).collect();
            let back: Vec<(f64, f64)> =
                (0..100).rev().map(|y| (x0 + slope * (f64::from(y) + 0.5 + shift), y0 + f64::from(y) + 0.5 + shift)).chain([(x0, y0)]).collect();
            let stub = poly(&[out, back].concat(), false);
            assert_eq!(interior_point(&stub), None, "a stub out and back along a line of slope {slope} at ({x0:e}, {y0:e}), variant {variant}");
        }
    }
}

// ---- a polygon whose edges all cross one another

/// A closed polygon of `n` edges (`n` odd) whose vertex `i` is at the angle `2 pi (k i mod n) / n` on a circle, `k = (n - 1) / 2`:
/// each edge goes to a vertex nearly opposite and crosses most of the others, `n (k - 1)` crossings in all.
fn star(n: usize) -> BezPath {
    let k = (n - 1) / 2;
    let points: Vec<(f64, f64)> = (0..n)
        .map(|i| {
            let a = std::f64::consts::TAU * ((k * i) % n) as f64 / n as f64;
            (100.0 * a.cos(), 100.0 * a.sin())
        })
        .collect();
    poly(&points, true)
}

/// The limits before it passed 2,001 edges: 2.0 million pairs of edges whose bounds overlap, under the 4.2 million of
/// `MAX_SEGMENT_PAIRS`, and 4.0e9 searches of the sweep line by those pairs, under the 4.3e9 of `MAX_SWEEP_EVENT_WORK`,
/// admitted a polygon every edge of which crosses nearly every other: `try_area`, `try_unite_all_with` and the
/// Pathfinder took 19 s and 3.5 GB to make 2 million crossings. Every pair there crosses, where the count of
/// the event limit is priced for a hatch, whose pairs of bounds that overlap cross a fifth of the time; the lines
/// that cross are counted now (`MAX_CROSSED_WORK`). 2,001 and 3,001 edges are refused in milliseconds (the latter by
/// the limit on pairs, as 4,001 were), 1,001 are made.
#[test]
fn a_polygon_whose_edges_all_cross_one_another_is_refused_in_milliseconds_or_made_in_seconds() {
    let refused = |n: usize| {
        let path = star(n);
        let shapes = [Shape::new(path.clone(), NZ, 0), Shape::new(path.clone(), NZ, 1)];
        let start = Instant::now();
        let results = [
            try_area(&path, NZ).map(|_| ()),
            try_unite_all_with(&[(&path, NZ)], DEFAULT_PRECISION).map(|_| ()),
            try_pathfinder_with(PathfinderOp::Intersect, &shapes[..1], DEFAULT_PRECISION).map(|_| ()),
            try_normalize(&path, NZ).map(|_| ()),
        ];
        (results, start.elapsed())
    };
    for n in [2001, 3001, 4001] {
        let (results, took) = refused(n);
        assert_eq!(results, [Err(PathOpsError::TooComplex); 4], "{n} edges");
        // Milliseconds in a release build; the bound is for a build for tests and a loaded machine, and the
        // sweep that was admitted took a minute or more.
        assert!(took < Duration::from_secs(60), "{n} edges: refused in {took:?}");
    }
    // 401 are made, at any build; 1,001 take 2.4 s in a release build and are made there.
    let sizes: &[usize] = if cfg!(debug_assertions) { &[401] } else { &[401, 1001] };
    for &n in sizes {
        let start = Instant::now();
        let area = try_area(&star(n), NZ).unwrap_or_else(|e| panic!("{n} edges: {e:?}"));
        assert!(area > 0.0, "{n} edges: area {area}");
        assert!(start.elapsed() < Duration::from_secs(60), "{n} edges: {:?}", start.elapsed());
    }
}

// ---- a slit near the sweep's tolerance

/// A square 100 across with a slit `w` wide at x = 50, from y = 20 to its top edge, cut into it as one subpath,
/// translated by `t` in x and y.
fn notched_square(w: f64, t: f64) -> BezPath {
    let half = w / 2.0;
    poly(
        &[
            (0.0, 0.0),
            (100.0, 0.0),
            (100.0, 100.0),
            (50.0 + half, 100.0),
            (50.0 + half, 20.0),
            (50.0 - half, 20.0),
            (50.0 - half, 100.0),
            (0.0, 100.0),
        ]
        .map(|(x, y)| (x + t, y + t)),
        true,
    )
}

/// A slit narrower than the sweep's tolerance is lost by the sweep itself: at 1e11 the tolerance is 1.4e-3 (64
/// units in the last place of the coordinates), and `try_area` of the notched square, which the sweep resolves,
/// has no slit at 1e-3, and has it at 1.5e-3. The rounding the inset's boundary allows a contour for the passes of a
/// retrace (`rounding_of`: the same 64 units, of the contour's own coordinates) is never more than that, so it
/// cannot take for a retrace a slit that the sweep keeps. The slit is widened by the inset where the sweep keeps it.
#[test]
fn a_slit_is_kept_by_the_sweep_and_the_inset_from_its_tolerance_up() {
    let solid = |t: f64| 10_000.0 - 0.0 * t;
    for (w, kept) in [(2e-3, true), (1.5e-3, true), (1e-3, false), (5e-4, false)] {
        let t = 1e11;
        let swept = area(&notched_square(w, t), NZ);
        let want = solid(t) - if kept { 80.0 * w } else { 0.0 };
        assert!((swept - want).abs() < 0.02, "slit {w} wide at {t}: area {swept}, {want} expected");
        // The inset by 5 of what the sweep kept: the square 90 across less the slit grown by 5 each side (to y = 15 at its bottom).
        let inset = area(&offset_path(&notched_square(w, t), -5.0, Join::Miter, 4.0), NZ);
        let want = 8100.0 - if kept { (w + 10.0) * 80.0 } else { 0.0 };
        assert!((inset - want).abs() < 0.1, "slit {w} wide at {t}: inset {inset}, {want} expected");
    }
}

// ---- a refit near the rounding of the coordinates

/// A circle of radius 100 cut into `n` cubics, centred at `(t, t)`.
fn cut_circle(n: usize, t: f64) -> BezPath {
    use kurbo::Vec2;
    let k = 4.0 / 3.0 * (std::f64::consts::PI / (2.0 * n as f64)).tan();
    let at = |a: f64| kurbo::Point::new(t + 100.0 * a.cos(), t + 100.0 * a.sin());
    let tangent = |a: f64| Vec2::new(-100.0 * a.sin(), 100.0 * a.cos());
    let mut path = BezPath::new();
    path.move_to(at(0.0));
    for i in 0..n {
        let (a0, a1) = (std::f64::consts::TAU * i as f64 / n as f64, std::f64::consts::TAU * (i + 1) as f64 / n as f64);
        let end = if i + 1 == n { at(0.0) } else { at(a1) };
        path.curve_to(at(a0) + tangent(a0) * k, end - tangent(a1) * k, end);
    }
    path.close_path();
    path
}

/// How far the samples of `merged` are from `original`, both translated back by `t` (exactly: the coordinates are
/// within a factor of two of it).
fn deviation(merged: &BezPath, original: &BezPath, t: f64) -> f64 {
    use kurbo::{ParamCurve, ParamCurveNearest};
    let back = |p: &BezPath| {
        let mut q = p.clone();
        q.apply_affine(kurbo::Affine::translate((-t, -t)));
        q
    };
    let (merged, original) = (back(merged), back(original));
    let mut worst = 0.0f64;
    for seg in merged.segments() {
        for k in 0..=32 {
            let p = seg.eval(f64::from(k) / 32.0);
            let nearest = original.segments().map(|o| o.nearest(p, 1e-9).distance_sq.sqrt()).fold(f64::INFINITY, f64::min);
            worst = worst.max(nearest);
        }
    }
    worst
}

/// A refit's bound takes the points of the run it is made from as the sweep's arithmetic gives them. The review
/// found that near the rounding of the coordinates (a tolerance of 1e-4 at 1e12, where a unit in the last place is
/// 1.2e-4) the evaluated points might be off by more than the bound allows, and the accepted cubic past the
/// tolerance. Not reproduced: a circle of 60 cubics translated from 1e8 to 3e12, at tolerances from 1e-4 to 1e-2,
/// is within the tolerance of itself in every case (0.95 of it at the most; where the tolerance is a few units
/// in the last place nothing is merged), measured from the translated-back coordinates.
#[test]
fn a_refit_near_the_rounding_of_the_coordinates_is_within_the_tolerance() {
    for t in [0.0, 1e8, 1e10, 1e11, 1e12, 3e12] {
        for tolerance in [1e-4, 1e-3, 1e-2] {
            let circle = cut_circle(60, t);
            let merged = remove_redundant_points(&circle, tolerance);
            let worst = deviation(&merged, &circle, t);
            assert!(worst <= tolerance, "at {t:e}, a tolerance of {tolerance}: {} elements, {worst} off", merged.elements().len());
        }
    }
}
