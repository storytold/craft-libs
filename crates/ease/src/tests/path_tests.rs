use crate::*;

/// A quarter circle of radius 100 about the origin, from (100, 0) to (0, 100).
fn quarter() -> MotionPath {
    let k = 4.0 / 3.0 * (std::f64::consts::PI / 8.0).tan() * 100.0;
    MotionPath::new(&[Vec2::new(100.0, 0.0), Vec2::new(100.0, k), Vec2::new(k, 100.0), Vec2::new(0.0, 100.0)]).unwrap()
}

/// The length of the curve through `n` evenly spaced points along it.
fn polyline_length(p: &MotionPath, n: usize) -> f64 {
    (1..=n).map(|i| p.point_at_length(p.length() * i as f64 / n as f64).distance(p.point_at_length(p.length() * (i - 1) as f64 / n as f64))).sum()
}

/// A tiny curve is walked at constant speed like the same curve at full size.
#[test]
fn a_tiny_curve_is_walked_at_constant_speed() {
    let r = 1e-15;
    let k = 4.0 / 3.0 * (std::f64::consts::PI / 8.0).tan() * r;
    let tiny = MotionPath::new(&[Vec2::new(r, 0.0), Vec2::new(r, k), Vec2::new(k, r), Vec2::new(0.0, r)]).unwrap();
    let big = quarter();
    for i in 1..20 {
        let t = i as f64 / 20.0;
        let (a, b) = (tiny.point_at(t), big.point_at(t));
        assert!((a.x / r - b.x / 100.0).abs() < 1e-9 && (a.y / r - b.y / 100.0).abs() < 1e-9, "{t}: {a:?} vs {b:?}");
    }
}

#[test]
fn a_curve_is_measured_and_walked_at_constant_speed() {
    let p = quarter();
    // The Bezier quarter circle is within 0.03 % of the circle's length.
    assert!((p.length() - 50.0 * std::f64::consts::PI).abs() < 0.05, "{}", p.length());
    assert!((polyline_length(&p, 20_000) - p.length()).abs() < 1e-6, "{} vs {}", polyline_length(&p, 20_000), p.length());
    // Equal steps of distance are equal chords.
    let n = 200;
    let steps: Vec<f64> = (1..=n).map(|i| p.point_at(i as f64 / n as f64).distance(p.point_at((i - 1) as f64 / n as f64))).collect();
    let (lo, hi) = steps.iter().fold((f64::MAX, f64::MIN), |(lo, hi), s| (lo.min(*s), hi.max(*s)));
    assert!(hi - lo < 1e-6, "{lo}..{hi}");
    assert_eq!(p.point_at(0.0), Vec2::new(100.0, 0.0));
    let end = p.point_at(1.0);
    assert!(end.distance(Vec2::new(0.0, 100.0)) < 1e-9, "{end:?}");
    // Each point is on the curve: about 100 from the center.
    for i in 0..=50 {
        let r = p.point_at(i as f64 / 50.0).length();
        assert!((r - 100.0).abs() < 0.03, "{r}");
    }
}

#[test]
fn a_straight_segment_with_uneven_handles_still_moves_evenly() {
    // Handles on the chord but bunched near the start: the curve's own parameter speeds up.
    let p = MotionPath::new(&[Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(100.0, 0.0)]).unwrap();
    assert_eq!(p.length(), 100.0);
    for i in 0..=10 {
        assert_eq!(p.point_at(i as f64 / 10.0), Vec2::new(i as f64 * 10.0, 0.0));
    }
}

#[test]
fn past_the_ends_the_motion_continues_along_the_path() {
    let p = quarter();
    // At the end the circle heads in −x; past it the point keeps going that way.
    let past = p.point_at_length(p.length() + 10.0);
    assert!(past.distance(Vec2::new(-10.0, 100.0)) < 1e-6, "{past:?}");
    let before = p.point_at_length(-5.0);
    assert!(before.distance(Vec2::new(100.0, -5.0)) < 1e-6, "{before:?}");
    // A track's overshooting ease drives the same distance through point_at.
    let over = Easing::BackOut;
    let peak = (1..100).map(|i| over.apply(i as f64 / 100.0)).fold(f64::MIN, f64::max);
    let beyond = p.point_at(peak);
    assert!(beyond.x < 0.0 && (beyond.y - 100.0).abs() < 1e-6, "{beyond:?}");
    let d = p.direction_at_length(p.length() * 2.0);
    assert!(d.distance(Vec2::new(-1.0, 0.0)) < 1e-9, "{d:?}");
    assert_eq!(p.point_at_length(f64::INFINITY), p.point_at(1.0));
    assert_eq!(p.point_at_length(f64::NAN), Vec2::new(100.0, 0.0));
    assert_eq!(p.point_at_length(f64::NEG_INFINITY), Vec2::new(100.0, 0.0));
    let flat = MotionPath::new(&[Vec2::ZERO, Vec2::new(1.0, 0.0), Vec2::new(2.0, 0.0), Vec2::new(3.0, 0.0)]).unwrap();
    assert_eq!(flat.point_at_length(f64::NEG_INFINITY), Vec2::ZERO);
}

/// Moving a handle off the chord by a rounding error does not change where an overshoot goes.
#[test]
fn an_overshoot_goes_the_same_way_on_a_straight_and_an_almost_straight_path() {
    let straight = MotionPath::new(&[Vec2::ZERO, Vec2::new(30.0, 0.0), Vec2::new(70.0, 0.0), Vec2::new(100.0, 0.0)]).unwrap();
    let bent = MotionPath::new(&[Vec2::ZERO, Vec2::new(30.0, 1e-6), Vec2::new(70.0, 0.0), Vec2::new(100.0, 0.0)]).unwrap();
    for progress in [-0.2, 1.1, 1.25] {
        let (a, b) = (straight.point_at(progress), bent.point_at(progress));
        assert!(a.distance(b) < 0.01, "{progress}: {a:?} vs {b:?}");
    }
    assert!(straight.point_at(1.25).distance(Vec2::new(125.0, 0.0)) < 1e-9);
}

#[test]
fn a_path_through_anchors_passes_through_each_one() {
    let anchors = [Vec2::new(0.0, 0.0), Vec2::new(100.0, 50.0), Vec2::new(200.0, 0.0), Vec2::new(300.0, 80.0)];
    let p = MotionPath::through(&anchors).unwrap();
    assert_eq!(p.segments(), 3);
    // Each anchor is reached at some distance along it.
    for a in anchors {
        let nearest = (0..=4000).map(|i| p.point_at(i as f64 / 4000.0).distance(a)).fold(f64::MAX, f64::min);
        assert!(nearest < 0.1, "{a:?}: {nearest}");
    }
    // Smooth through the inner anchors: no turn in direction across them.
    let mut prev = p.direction_at_length(0.0);
    for i in 1..=2000 {
        let d = p.direction_at_length(p.length() * i as f64 / 2000.0);
        assert!(d.dot(prev) > 0.99, "a corner at {i}");
        prev = d;
    }
    // Two anchors at one place make a segment of zero length, which is skipped over.
    let q = MotionPath::through(&[Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 0.0), Vec2::new(20.0, 0.0)]).unwrap();
    assert!((q.length() - 20.0).abs() < 1e-9, "{}", q.length());
    assert!(q.point_at(0.75).distance(Vec2::new(15.0, 0.0)) < 1e-6);
    // Anchors repeated at either end still let the motion continue past it.
    let r = MotionPath::through(&[Vec2::ZERO, Vec2::ZERO, Vec2::new(10.0, 0.0), Vec2::new(10.0, 0.0)]).unwrap();
    assert!(r.point_at(1.2).distance(Vec2::new(12.0, 0.0)) < 1e-6, "{:?}", r.point_at(1.2));
    assert!(r.point_at(-0.1).distance(Vec2::new(-1.0, 0.0)) < 1e-6, "{:?}", r.point_at(-0.1));
    assert!(r.direction_at_length(20.0).distance(Vec2::new(1.0, 0.0)) < 1e-9);
    assert!(r.direction_at_length(-1.0).distance(Vec2::new(1.0, 0.0)) < 1e-9);
}

#[test]
fn bad_paths_are_refused() {
    assert_eq!(MotionPath::new(&[Vec2::ZERO; 3]), Err(EaseError::TooFew { what: "a motion path", need: 4 }));
    assert!(matches!(MotionPath::new(&[Vec2::ZERO; 5]), Err(EaseError::OutOfRange { .. })));
    assert_eq!(MotionPath::new(&[Vec2::ZERO, Vec2::new(f64::NAN, 0.0), Vec2::ZERO, Vec2::ZERO]), Err(EaseError::NotFinite("path point")));
    assert_eq!(MotionPath::new(&vec![Vec2::ZERO; 3 * MAX_PATH_SEGMENTS + 4]), Err(EaseError::Full(MAX_PATH_SEGMENTS)));
    assert_eq!(MotionPath::through(&[Vec2::ZERO]), Err(EaseError::TooFew { what: "a motion path", need: 2 }));
    assert_eq!(MotionPath::through(&[Vec2::ZERO, Vec2::new(0.0, f64::INFINITY)]), Err(EaseError::NotFinite("path point")));
    // A path too long to measure.
    assert_eq!(
        MotionPath::new(&[Vec2::new(-f64::MAX, 0.0), Vec2::new(-f64::MAX, 0.0), Vec2::new(f64::MAX, 0.0), Vec2::new(f64::MAX, 0.0)]),
        Err(EaseError::Overflow)
    );
    // A path of one point stands still.
    let still = MotionPath::new(&[Vec2::new(3.0, 4.0); 4]).unwrap();
    assert_eq!((still.length(), still.point_at(0.5), still.direction_at_length(1.0)), (0.0, Vec2::new(3.0, 4.0), Vec2::ZERO));
}

/// A curve with huge coordinates (whose squares overflow) is walked as the same curve scaled,
/// not taken for a straight line.
#[test]
fn a_huge_curve_is_not_taken_for_a_straight_line() {
    let shape = |k: f64| MotionPath::new(&[Vec2::ZERO, Vec2::new(0.0, k), Vec2::new(0.0, k), Vec2::new(2.0 * k, 0.0)]).unwrap();
    let (small, huge) = (shape(1.0), shape(1e154));
    for i in 0..=10 {
        let t = i as f64 / 10.0;
        let (a, b) = (small.point_at(t), huge.point_at(t));
        assert!((b.x / 1e154 - a.x).abs() < 1e-9 && (b.y / 1e154 - a.y).abs() < 1e-9, "{t}: {a:?} vs {b:?}");
    }
    assert!(huge.point_at(0.5).y > 5e153, "{:?}", huge.point_at(0.5));
    // A long chord with handles a small distance off it still bends.
    let long = MotionPath::new(&[Vec2::ZERO, Vec2::new(1e12 / 3.0, 1000.0), Vec2::new(2e12 / 3.0, 1000.0), Vec2::new(1e12, 0.0)]).unwrap();
    let mid = long.point_at_length(long.length() / 2.0);
    assert!((mid.y - 750.0).abs() < 1.0, "{mid:?}");
}

/// Paths whose length is representable are measured even when their scale is near the f64 limit.
#[test]
fn a_path_near_the_largest_length_is_measured() {
    let straight = MotionPath::new(&[Vec2::ZERO, Vec2::new(1e307 / 3.0, 0.0), Vec2::new(2e307 / 3.0, 0.0), Vec2::new(1e307, 0.0)]).unwrap();
    assert_eq!(straight.length(), 1e307);
    assert!((straight.point_at(0.5).x / 5e306 - 1.0).abs() < 1e-12, "{:?}", straight.point_at(0.5));
    let curved = MotionPath::new(&[Vec2::ZERO, Vec2::ZERO, Vec2::new(0.0, 1e306), Vec2::new(5e307, 0.0)]).unwrap();
    assert!(curved.length() > 5e307 && curved.length() < 5.2e307, "{}", curved.length());
    assert!(curved.point_at(1.0).distance(Vec2::new(5e307, 0.0)) < 1e295);
}

/// A distance close to a start where the curve stands still is found on the curve, not where a
/// few Newton steps from the table's guess happen to stop.
#[test]
fn a_short_distance_from_a_still_start_is_found() {
    let p = MotionPath::new(&[Vec2::ZERO, Vec2::ZERO, Vec2::new(0.0, 1.0), Vec2::new(1.0, 1.0)]).unwrap();
    for s in [1e-10, 1e-8, 1e-6] {
        let q = p.point_at_length(s);
        // The curve leaves the start straight up, so the point is `s` above it.
        assert!((q.y - s).abs() < 1e-11 && q.x.abs() < 1e-9, "{s}: {q:?}");
    }
}

#[test]
fn a_vector_of_any_finite_size_has_a_unit_direction() {
    let h = std::f64::consts::FRAC_1_SQRT_2;
    let big = Vec2::new(f64::MAX, f64::MAX).normalized();
    assert!((big.x - h).abs() < 1e-15 && (big.y - h).abs() < 1e-15, "{big:?}");
    assert_eq!(Vec2::new(-f64::MAX, 0.0).normalized(), Vec2::new(-1.0, 0.0));
    let tiny = Vec2::new(f64::from_bits(1), f64::from_bits(1)).normalized();
    assert!((tiny.x - h).abs() < 1e-15 && (tiny.y - h).abs() < 1e-15, "{tiny:?}");
    let odd = Vec2::new(3.0 * f64::from_bits(1), -4.0 * f64::from_bits(1)).normalized();
    assert!((odd.x - 0.6).abs() < 1e-15 && (odd.y + 0.8).abs() < 1e-15, "{odd:?}");
    assert_eq!(Vec2::ZERO.normalized(), Vec2::ZERO);
    assert_eq!(Vec2::new(f64::INFINITY, 1.0).normalized(), Vec2::ZERO);
    assert_eq!(Vec2::new(1.0, f64::NAN).normalized(), Vec2::ZERO);
}

#[test]
fn a_long_path_finds_its_segment_by_binary_search() {
    let anchors: Vec<Vec2> = (0..=MAX_PATH_SEGMENTS).map(|i| Vec2::new(i as f64, ((i as f64) * 0.1).sin())).collect();
    let p = MotionPath::through(&anchors).unwrap();
    assert_eq!(p.segments(), MAX_PATH_SEGMENTS);
    let mid = p.point_at(0.5);
    assert!(mid.x > 4000.0 && mid.x < 6000.0, "{mid:?}");
}
