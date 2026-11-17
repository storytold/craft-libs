use crate::*;

/// A spring released from 0 toward 1, `t` seconds in, from the closed forms written out
/// separately: under-damped, critically damped and over-damped.
fn released(w: f64, z: f64, t: f64) -> f64 {
    if z < 1.0 {
        let wd = w * (1.0 - z * z).sqrt();
        1.0 - (-z * w * t).exp() * ((wd * t).cos() + z * w / wd * (wd * t).sin())
    } else if (z - 1.0).abs() < 1e-9 {
        1.0 - (-w * t).exp() * (1.0 + w * t)
    } else {
        let s = (z * z - 1.0).sqrt();
        let (r1, r2) = (-w / (z + s), -w * (z + s));
        1.0 - (r2 * (r1 * t).exp() - r1 * (r2 * t).exp()) / (r2 - r1)
    }
}

const PRESETS: [(f64, f64, f64); 5] = [(1.0, 100.0, 15.0), (1.0, 300.0, 20.0), (1.0, 600.0, 15.0), (1.0, 80.0, 20.0), (2.5, 40.0, 2.0)];

#[test]
fn a_physical_spring_ease_matches_the_closed_form() {
    for (m, k, c) in PRESETS {
        let spring = Spring::from_physical(m, k, c).unwrap();
        let (w, z) = ((k / m).sqrt(), c / (2.0 * (k * m).sqrt()));
        assert!((spring.omega() - w).abs() < 1e-12 && (spring.damping() - z).abs() < 1e-12);
        let rate = if z < 1.0 { z * w } else { w / (z + (z * z - 1.0).sqrt()) };
        let lead = if z < 1.0 { (6.9 - 0.5 * (1.0 - z * z).ln()).min(9.2) } else { 9.2 };
        let settle = (lead / rate).clamp(0.05, 10.0);
        assert!((spring.settle_time() - settle).abs() < 1e-12);
        let ease = Easing::Spring(SpringEase { mass: m, stiffness: k, damping: c });
        // A spring cut short at 10 seconds has what it is still off taken out evenly.
        let rest = if settle < 10.0 { 0.0 } else { 1.0 - released(w, z, settle) };
        for i in 1..1000 {
            let t = i as f64 / 1000.0;
            let want = released(w, z, t * settle) + t * rest;
            assert!((ease.apply(t) - want).abs() < 1e-12, "{m} {k} {c} at {t}: {} vs {want}", ease.apply(t));
        }
        // It ends within 0.1 % (0.2 % for the slower lead factor) of 1 before the jump to 1.
        assert!((ease.apply(0.999_999) - 1.0).abs() < 2e-3, "{m} {k} {c}");
    }
}

/// A step of no time returns the value unchanged, even when its distance to the target is too
/// large to represent.
#[test]
fn a_zero_length_step_keeps_a_far_value() {
    let spring = Spring::new(1.0, 0.5).unwrap();
    let mut s = SpringState::at(1e308);
    assert_eq!(s.step(&spring, -1e308, 0.0), Ok(1e308));
    assert_eq!(s, SpringState::at(1e308));
    // A state that is not finite is still refused.
    let mut bad = SpringState::at(f64::NAN);
    assert_eq!(bad.step(&spring, 0.0, 0.0), Err(EaseError::Overflow));
}

/// A value and target at opposite ends of the f64 range move like any others.
#[test]
fn a_step_across_the_whole_range_does_not_overflow() {
    let spring = Spring::new(1.0, 1.0).unwrap();
    let mut s = SpringState::at(f64::MAX);
    let v = s.step(&spring, -f64::MAX, 10.0).unwrap();
    assert!(v.is_finite() && v < 0.0 && s.velocity.is_finite(), "{s:?}");
}

/// The smallest positive target is reached.
#[test]
fn a_spring_reaches_a_subnormal_target() {
    let spring = Spring::new(1.0, 1.0).unwrap();
    let mut s = SpringState::at(0.0);
    assert_eq!(s.step(&spring, f64::from_bits(1), 1000.0), Ok(f64::from_bits(1)));
}

/// A spring of the lowest frequency still moves at its velocity over a step.
#[test]
fn a_very_slow_spring_steps_without_overflow() {
    let spring = Spring::new(1e-320, 0.0).unwrap();
    let mut s = SpringState { value: 0.0, velocity: 1.0 };
    let v = s.step(&spring, 0.0, 1.0).unwrap();
    assert!((v - 1.0).abs() < 1e-12 && (s.velocity - 1.0).abs() < 1e-12, "{s:?}");
}

/// Physical values whose damping ratio is above the limit are refused, not changed.
#[test]
fn physical_values_past_the_damping_ratio_limit_are_refused() {
    assert_eq!(
        Spring::from_physical(0.01, 0.01, 1_000_000.0),
        Err(EaseError::OutOfRange { what: "damping ratio", min: 0.0, max: MAX_DAMPING_RATIO })
    );
    assert!(Spring::from_physical(0.01, 0.01, 19.0).is_ok());
}

/// A spring just under critical damping is within about 0.1 % of 1 just before the end, so the
/// ease does not jump when it reaches 1.
#[test]
fn a_nearly_critical_spring_ease_ends_without_a_jump() {
    for z in [0.5, 0.9, 0.99, 0.999, 0.9999, 1.0 - 2e-4] {
        let s = Spring::new(1.0, z).unwrap();
        let end = s.ease(1.0 - 1e-9);
        assert!((end - 1.0).abs() < 1.1e-3, "ζ {z}: {end}");
        // The bound holds after the settle time too, not only at it.
        let late = 1.0 + s.response(-1.0, 0.0, 1.5 * s.settle_time()).0;
        assert!((late - 1.0).abs() < 1.1e-3, "ζ {z} later: {late}");
    }
}

/// An undamped or barely damped spring ease still arrives at 1 without a jump.
#[test]
fn a_spring_that_never_settles_still_ends_at_one() {
    for (m, k, c) in [(1.0, 100.0, 0.0), (1.0, 100.0, 0.01), (10.0, 0.5, 0.0)] {
        let e = Easing::Spring(SpringEase { mass: m, stiffness: k, damping: c });
        assert!((e.apply(1.0 - 1e-9) - 1.0).abs() < 1e-6, "{m} {k} {c}: {}", e.apply(1.0 - 1e-9));
        assert!((1..1000).all(|i| e.apply(i as f64 / 1000.0).is_finite()));
    }
}

/// Stepping a spring from rest toward 1 frame by frame traces its ease.
#[test]
fn stepping_from_rest_traces_the_spring_ease() {
    // Springs that settle within 10 seconds (the others have their ease adjusted to end at 1).
    for (m, k, c) in PRESETS.into_iter().filter(|&(m, k, c)| Spring::from_physical(m, k, c).unwrap().settle_time() < 10.0) {
        let s = Spring::from_physical(m, k, c).unwrap();
        let ease = Easing::Spring(SpringEase { mass: m, stiffness: k, damping: c });
        let (settle, frames) = (s.settle_time(), 600);
        let mut st = SpringState::at(0.0);
        for f in 1..frames {
            let v = st.step(&s, 1.0, settle / frames as f64).unwrap();
            let want = ease.apply(f as f64 / frames as f64);
            assert!((v - want).abs() < 1e-9, "{m} {k} {c} frame {f}: {v} vs {want}");
        }
    }
}

#[test]
fn the_three_damping_regimes_and_the_critical_band_agree() {
    // Either side of critical damping, the solutions meet the critically damped one.
    let w = 10.0;
    for t in [0.01, 0.1, 0.3, 1.0] {
        let crit = released(w, 1.0, t);
        for z in [1.0 - 2e-4, 1.0 - 5e-5, 1.0, 1.0 + 5e-5, 1.0 + 2e-4] {
            let s = Spring::new(w / std::f64::consts::TAU, z).unwrap();
            let mut st = SpringState::at(0.0);
            let v = st.step(&s, 1.0, t).unwrap();
            assert!((v - crit).abs() < 1e-3, "ζ {z} at {t}: {v} vs {crit}");
        }
    }
}

#[test]
fn a_spring_settles_on_its_target() {
    for (f, z) in [(2.0, 0.2), (1.0, 1.0), (0.5, 3.0), (8.0, 0.05)] {
        let s = Spring::new(f, z).unwrap();
        let mut st = SpringState::at(0.0);
        for _ in 0..60 * 120 {
            st.step(&s, 10.0, 1.0 / 60.0).unwrap();
        }
        assert!(st.settled(10.0, 1e-6), "{f} Hz ζ {z}: {st:?}");
    }
    // An undamped spring keeps swinging, its energy kept.
    let s = Spring::new(1.0, 0.0).unwrap();
    let mut st = SpringState::at(0.0);
    for _ in 0..60 * 100 {
        st.step(&s, 1.0, 1.0 / 60.0).unwrap();
    }
    let w = s.omega();
    let energy = (st.value - 1.0).powi(2) + (st.velocity / w).powi(2);
    assert!((energy - 1.0).abs() < 1e-9, "{energy}");
}

/// Many short steps land where one long step does: the step is exact, not an integrator.
#[test]
fn many_small_steps_equal_one_long_step() {
    for (f, z) in [(3.0, 0.3), (1.0, 1.0), (1.0, 4.0), (0.7, 0.0)] {
        let s = Spring::new(f, z).unwrap();
        let mut one = SpringState { value: -2.0, velocity: 5.0 };
        let mut many = one;
        one.step(&s, 1.0, 2.0).unwrap();
        for _ in 0..240 {
            many.step(&s, 1.0, 2.0 / 240.0).unwrap();
        }
        assert!((one.value - many.value).abs() < 1e-9 && (one.velocity - many.velocity).abs() < 1e-8, "{f} {z}: {one:?} {many:?}");
    }
    // Variable frame lengths too.
    let s = Spring::new(2.0, 0.4).unwrap();
    let (mut a, mut b) = (SpringState::at(0.0), SpringState::at(0.0));
    a.step(&s, 1.0, 0.5).unwrap();
    for dt in [0.1, 0.016, 0.05, 0.034, 0.2, 0.1] {
        b.step(&s, 1.0, dt).unwrap();
    }
    assert!((a.value - b.value).abs() < 1e-9);
}

#[test]
fn huge_steps_land_on_the_target() {
    for (f, z) in [(MAX_SPRING_HZ, 0.0001), (1e-6, 0.5), (1.0, MAX_DAMPING_RATIO), (5.0, 1.0)] {
        let s = Spring::new(f, z).unwrap();
        for dt in [1e6, 1e300, f64::MAX] {
            let mut st = SpringState { value: 5.0, velocity: -3.0 };
            let v = st.step(&s, 2.0, dt).unwrap();
            assert!(v.is_finite() && st.velocity.is_finite(), "{f} {z} {dt}");
        }
    }
    let s = Spring::new(4.0, 0.5).unwrap();
    let mut st = SpringState { value: 5.0, velocity: -3.0 };
    assert_eq!(st.step(&s, 2.0, 1e300).unwrap(), 2.0);
    assert_eq!(st.velocity, 0.0);
    // An undamped spring keeps its energy over any finite step, the longest included.
    let s = Spring::new(MAX_SPRING_HZ, 0.0).unwrap();
    let w = s.omega();
    for dt in [1e300, f64::MAX] {
        let mut st = SpringState { value: 5.0, velocity: -3.0 };
        st.step(&s, 2.0, dt).unwrap();
        let energy = (st.value - 2.0).powi(2) + (st.velocity / w).powi(2);
        assert!((energy - (9.0 + (3.0 / w).powi(2))).abs() < 1e-9, "{dt}: {st:?}");
    }
    let s = Spring::new(4.0, 0.5).unwrap();
    // A zero step changes nothing.
    let mut st = SpringState { value: 5.0, velocity: -3.0 };
    assert_eq!(st.step(&s, 2.0, 0.0).unwrap(), 5.0);
    assert_eq!(st.velocity, -3.0);
}

#[test]
fn bad_inputs_are_refused_and_leave_the_state_alone() {
    assert_eq!(Spring::new(f64::NAN, 0.5), Err(EaseError::NotFinite("frequency")));
    assert!(matches!(Spring::new(0.0, 0.5), Err(EaseError::OutOfRange { what: "frequency", .. })));
    assert!(matches!(Spring::new(MAX_SPRING_HZ * 2.0, 0.5), Err(EaseError::OutOfRange { what: "frequency", .. })));
    assert!(matches!(Spring::new(1.0, -0.1), Err(EaseError::OutOfRange { what: "damping", .. })));
    assert_eq!(Spring::new(1.0, f64::INFINITY), Err(EaseError::NotFinite("damping")));
    assert!(matches!(Spring::from_controls(1.5, 0.5), Err(EaseError::OutOfRange { what: "strength", .. })));
    assert!(matches!(Spring::from_physical(1.0, 0.0, 1.0), Err(EaseError::OutOfRange { what: "stiffness", .. })));
    let s = Spring::new(1.0, 0.5).unwrap();
    let start = SpringState { value: 1.0, velocity: 2.0 };
    for (target, dt, err) in [
        (f64::NAN, 0.1, EaseError::NotFinite("target")),
        (f64::INFINITY, 0.1, EaseError::NotFinite("target")),
        (0.0, f64::NAN, EaseError::NotFinite("step")),
        (0.0, f64::INFINITY, EaseError::NotFinite("step")),
    ] {
        let mut st = start;
        assert_eq!(st.step(&s, target, dt), Err(err));
        assert_eq!(st, start);
    }
    let mut st = start;
    assert!(matches!(st.step(&s, 0.0, -0.1), Err(EaseError::OutOfRange { what: "step", .. })));
    assert_eq!(st, start);
    // A value too far from the target to measure is refused, not turned into NaN.
    let mut far = SpringState { value: f64::MAX, velocity: 0.0 };
    assert_eq!(far.step(&s, -f64::MAX, 0.1), Err(EaseError::Overflow));
    assert_eq!(far, SpringState { value: f64::MAX, velocity: 0.0 });
}

#[test]
fn the_controls_map_onto_frequency_and_damping() {
    let s = Spring::from_controls(0.0, 0.0).unwrap();
    assert!((s.omega() - std::f64::consts::TAU * 0.5).abs() < 1e-12 && s.damping() == 0.0);
    let s = Spring::from_controls(1.0, 2.0 / 3.0).unwrap();
    assert!((s.omega() - std::f64::consts::TAU * 10.0).abs() < 1e-9 && (s.damping() - 1.0).abs() < 1e-12);
    assert_eq!(Spring::from_controls(0.5, 1.0).unwrap().damping(), MAX_DAMPING_RATIO);
}
