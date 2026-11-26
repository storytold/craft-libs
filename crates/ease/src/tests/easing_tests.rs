use crate::*;
use std::sync::Arc;

fn all() -> Vec<Easing> {
    let mut v: Vec<Easing> = Easing::named().collect();
    v.extend([Easing::Classic(-60.0), Easing::Classic(100.0), Easing::CubicBezier(0.25, 0.1, 0.25, 1.0), Easing::Custom(Arc::new(wave()))]);
    v
}

/// Every ease kind, values included.
fn every_kind() -> Vec<Easing> {
    let mut v = all();
    v.extend([Easing::Hold, Easing::Spring(SpringEase::GENTLE), Easing::Sampled(Arc::new(steps()))]);
    v
}

/// Three segments that overshoot both ways.
fn wave() -> CustomEase {
    CustomEase::from_flat(
        &[(0.0, 0.0), (0.1, 0.6), (0.2, 1.3), (0.3, 1.3), (0.4, 1.3), (0.5, -0.2), (0.6, 0.4), (0.7, 1.0), (0.9, 1.0), (1.0, 1.0)]
            .map(|(x, y)| Vec2::new(x, y)),
    )
    .unwrap()
}

#[test]
fn a_narrow_custom_segment_is_evaluated_not_skipped() {
    let d = 1e-10;
    let e = CustomEase::from_flat(
        &[
            (0.0, 0.0),
            (0.1, 0.0),
            (0.4, 0.0),
            (0.5, 0.0),
            (0.5 + d / 3.0, 1.0 / 3.0),
            (0.5 + 2.0 * d / 3.0, 2.0 / 3.0),
            (0.5 + d, 1.0),
            (0.6, 1.0),
            (0.9, 1.0),
            (1.0, 1.0),
        ]
        .map(|(x, y)| Vec2::new(x, y)),
    )
    .unwrap();
    let mid = e.apply(0.5 + d / 2.0);
    assert!((mid - 0.5).abs() < 0.05, "{mid}");
}

/// Four steps, as a stepped ease samples them.
fn steps() -> SampledEase {
    SampledEase::new("Stepped 4", "org.example.stepped-ease", "steps4", (0..257).map(|i| ((i as f64 / 256.0) * 4.0).floor() / 4.0).collect())
}

#[test]
fn endpoints_are_exact() {
    for e in every_kind() {
        assert_eq!(e.apply(0.0), 0.0, "{e:?}");
        assert_eq!(e.apply(1.0), 1.0, "{e:?}");
        assert_eq!(e.apply(-0.0), 0.0, "{e:?}");
        assert_eq!(e.apply(-5.0), 0.0, "{e:?}");
        assert_eq!(e.apply(5.0), 1.0, "{e:?}");
        assert_eq!(e.apply(f64::NEG_INFINITY), 0.0, "{e:?}");
        assert_eq!(e.apply(f64::INFINITY), 1.0, "{e:?}");
        assert_eq!(e.apply(f64::NAN), 0.0, "{e:?}");
    }
}

#[test]
fn presets_at_known_times() {
    use std::f64::consts::{FRAC_1_SQRT_2, PI};
    let cases: [(Easing, f64, f64); 22] = [
        (Easing::Linear, 0.3, 0.3),
        (Easing::QuadIn, 0.5, 0.25),
        (Easing::QuadOut, 0.5, 0.75),
        (Easing::QuadInOut, 0.25, 0.125),
        (Easing::CubicIn, 0.5, 0.125),
        (Easing::CubicOut, 0.5, 0.875),
        (Easing::QuartIn, 0.5, 0.0625),
        (Easing::QuartInOut, 0.75, 0.96875),
        (Easing::QuintIn, 0.5, 0.03125),
        (Easing::QuintOut, 0.5, 0.96875),
        (Easing::SineIn, 0.5, 1.0 - FRAC_1_SQRT_2),
        (Easing::SineOut, 1.0 / 3.0, 0.5),
        (Easing::SineInOut, 0.25, 0.5 * (1.0 - FRAC_1_SQRT_2)),
        // (c + 1)/8 − c/4 with c = 1.70158: dips below zero first.
        (Easing::BackIn, 0.5, -0.087_697_5),
        (Easing::BackOut, 0.5, 1.087_697_5),
        (Easing::CircIn, 0.6, 0.2),
        (Easing::CircOut, 0.5, 0.866_025_403_784_438_6),
        (Easing::BounceOut, 0.5, 0.765_625),
        (Easing::BounceIn, 0.5, 0.234_375),
        (Easing::BounceOut, 1.0 / 2.75, 1.0),
        // −2⁻⁵ · sin(−5.75 · 2π/3) = −1/64; Out is its mirror.
        (Easing::ElasticIn, 0.5, -0.015_625),
        (Easing::ElasticOut, 0.5, 1.015_625),
    ];
    for (e, t, want) in cases {
        assert!((e.apply(t) - want).abs() < 1e-9, "{e:?} at {t}: {} vs {want}", e.apply(t));
    }
    assert!((Easing::Classic(-100.0).apply(0.5) - 0.25).abs() < 1e-12);
    assert!((Easing::Classic(100.0).apply(0.5) - 0.75).abs() < 1e-12);
    assert!((Easing::SineIn.apply(1.0 / 3.0) - (1.0 - (PI / 6.0).cos())).abs() < 1e-12);
}

/// Back and Elastic In Out follow the published In Out equations, which are not In's halves.
#[test]
fn back_and_elastic_in_out_follow_the_published_equations() {
    for (e, t, want) in [
        (Easing::BackInOut, 0.25, -0.099_68),
        (Easing::BackInOut, 0.75, 1.099_68),
        (Easing::ElasticInOut, 0.25, 0.011_97),
        (Easing::ElasticInOut, 0.4, -0.117_46),
        (Easing::ElasticInOut, 0.75, 0.988_03),
    ] {
        assert!((e.apply(t) - want).abs() < 5e-6, "{e:?} at {t}: {}", e.apply(t));
    }
    // Back In Out: ((2t)²((c+1)2t − c))/2 with c = 1.70158 × 1.525.
    let c = 1.701_58 * 1.525;
    for k in 1..20 {
        let t = k as f64 / 40.0;
        let want = (2.0 * t).powi(2) * ((c + 1.0) * 2.0 * t - c) / 2.0;
        assert!((Easing::BackInOut.apply(t) - want).abs() < 1e-12, "{t}");
    }
}

#[test]
fn out_mirrors_in_and_in_out_is_symmetric() {
    for f in EaseFamily::ALL {
        let (i, o, io) = (Easing::preset(f, EaseDir::In), Easing::preset(f, EaseDir::Out), Easing::preset(f, EaseDir::InOut));
        for k in 0..=40 {
            let t = k as f64 / 40.0;
            assert!((o.apply(t) - (1.0 - i.apply(1.0 - t))).abs() < 1e-12, "{f:?} at {t}");
            assert!((io.apply(t) + io.apply(1.0 - t) - 1.0).abs() < 1e-12, "{f:?} at {t}");
            // In Out is In's halves for every family but Back and Elastic.
            if !matches!(f, EaseFamily::Back | EaseFamily::Elastic) {
                assert!((io.apply(t * 0.5) - 0.5 * i.apply(t)).abs() < 1e-12, "{f:?} at {t}");
            }
        }
        assert_eq!(Easing::preset(f, EaseDir::In).family(), Some((f, EaseDir::In)));
    }
}

#[test]
fn monotonic_families_never_turn_back_and_the_others_overshoot() {
    for e in all() {
        let overshoots = matches!(e.family(), Some((EaseFamily::Back | EaseFamily::Elastic, _))) || e.custom().is_some();
        // Bounce turns back (off the floor) without leaving 0..1.
        let turns = overshoots || matches!(e.family(), Some((EaseFamily::Bounce, _)));
        let (mut lo, mut hi, mut prev, mut turned) = (0.0f64, 1.0f64, -1e-6f64, false);
        for i in 0..=400 {
            let v = e.apply(i as f64 / 400.0);
            turned |= v < prev - 1e-4;
            prev = v;
            lo = lo.min(v);
            hi = hi.max(v);
        }
        assert_eq!(turned, turns, "{e:?}");
        assert_eq!(lo < -1e-3 || hi > 1.0 + 1e-3, overshoots, "{e:?}: {lo}..{hi}");
    }
}

/// The eases meant to be monotonic never step backwards on a fine grid, not even by rounding.
#[test]
fn monotonic_eases_are_non_decreasing_everywhere() {
    let mut eases: Vec<Easing> = [EaseFamily::Quad, EaseFamily::Cubic, EaseFamily::Quart, EaseFamily::Quint, EaseFamily::Sine, EaseFamily::Circ]
        .into_iter()
        .flat_map(|f| EaseDir::ALL.map(|d| Easing::preset(f, d)))
        .collect();
    eases.extend([
        Easing::Linear,
        Easing::Hold,
        Easing::Classic(-100.0),
        Easing::Classic(37.0),
        Easing::Classic(100.0),
        Easing::SMOOTH,
        Easing::CubicBezier(1.0, 0.0, 0.0, 1.0),
        Easing::CubicBezier(0.0, 1.0, 1.0, 0.0),
        Easing::CubicBezier(0.9, 0.0, 0.1, 1.0),
    ]);
    eases.push(Easing::from_temporal(TemporalEase::EASY, TemporalEase::EASY).unwrap());
    eases.push(Easing::from_temporal(TemporalEase { speed: 0.0, influence: 1.0 }, TemporalEase { speed: 0.0, influence: 1.0 }).unwrap());
    eases.push(Easing::Spring(SpringEase { mass: 1.0, stiffness: 100.0, damping: 40.0 }));
    for e in eases {
        let mut prev = 0.0;
        for i in 0..=20_000 {
            let v = e.apply(i as f64 / 20_000.0);
            assert!(v >= prev, "{e:?} steps back at {i}: {prev} → {v}");
            assert!((0.0..=1.0).contains(&v), "{e:?} leaves 0..1 at {i}: {v}");
            prev = v;
        }
    }
}

#[test]
fn hostile_input_is_clamped() {
    assert_eq!(Easing::Linear.apply(f64::NAN), 0.0);
    assert_eq!(Easing::QuadIn.apply(5.0), 1.0);
    assert_eq!(Easing::ElasticOut.apply(f64::INFINITY), 1.0);
    assert!(Easing::CubicBezier(f64::NAN, 0.0, 1.0, 1.0).apply(0.5).is_finite());
    assert!(Easing::Classic(f64::INFINITY).apply(0.5).is_finite());
    assert!(Easing::Custom(Arc::new(wave())).apply(f64::NAN).is_finite());
    assert!(Easing::Spring(SpringEase { mass: f64::NAN, stiffness: -1.0, damping: f64::INFINITY }).apply(0.5).is_finite());
    // Huge handles are kept within ±MAX_EASE_Y when evaluated.
    let wild = Easing::CubicBezier(0.5, 1e300, 0.5, -1e300);
    for k in 0..=100 {
        let v = wild.apply(k as f64 / 100.0);
        assert!(v.abs() <= MAX_EASE_Y, "{k}: {v}");
    }
    // Times next to the ends, and subnormal ones.
    for e in all() {
        for t in [f64::MIN_POSITIVE, 5e-324, 1e-300, 1.0 - f64::EPSILON / 2.0, 1.0 - 1e-16] {
            assert!(e.apply(t).is_finite(), "{e:?} at {t}");
        }
    }
}

#[test]
fn checked_constructors_refuse_bad_values() {
    assert!(Easing::cubic_bezier(0.2, 0.0, 0.8, 1.0).is_ok());
    assert_eq!(Easing::cubic_bezier(f64::NAN, 0.0, 0.8, 1.0), Err(EaseError::NotFinite("x1")));
    assert!(matches!(Easing::cubic_bezier(1.2, 0.0, 0.8, 1.0), Err(EaseError::OutOfRange { what: "x1", .. })));
    assert!(matches!(Easing::cubic_bezier(0.2, 0.0, -0.1, 1.0), Err(EaseError::OutOfRange { what: "x2", .. })));
    assert!(matches!(Easing::cubic_bezier(0.2, 11.0, 0.8, 1.0), Err(EaseError::OutOfRange { what: "y1", .. })));
    assert_eq!(Easing::cubic_bezier(0.2, 0.0, 0.8, f64::INFINITY), Err(EaseError::NotFinite("y2")));
    let ok = TemporalEase { speed: 0.0, influence: 0.5 };
    assert!(matches!(
        Easing::from_temporal(TemporalEase { speed: 0.0, influence: 0.0 }, ok),
        Err(EaseError::OutOfRange { what: "out influence", .. })
    ));
    assert!(matches!(
        Easing::from_temporal(ok, TemporalEase { speed: 0.0, influence: 1.5 }),
        Err(EaseError::OutOfRange { what: "in influence", .. })
    ));
    assert_eq!(Easing::from_temporal(TemporalEase { speed: f64::NAN, influence: 0.5 }, ok), Err(EaseError::NotFinite("out speed")));
    assert!(matches!(Easing::from_temporal(TemporalEase { speed: 1e6, influence: 0.5 }, ok), Err(EaseError::OutOfRange { .. })));
    assert!(Easing::spring(1.0, 100.0, 15.0).is_ok());
    assert!(matches!(Easing::spring(0.0, 100.0, 15.0), Err(EaseError::OutOfRange { what: "mass", .. })));
    assert_eq!(Easing::spring(1.0, f64::NAN, 15.0), Err(EaseError::NotFinite("stiffness")));
}

/// Sample values from a keyframe velocity dialog: two keys a second apart, both sides eased
/// with speed (units a second) and influence. The values are the ease's progress there.
#[test]
fn speed_and_influence_give_the_dialog_curves() {
    for (out, into, samples) in [
        (
            TemporalEase { speed: 0.0, influence: 0.8 },
            TemporalEase { speed: 0.0, influence: 0.8 },
            [(0.1, 0.005_924_393_903_632_7), (0.25, 0.047_643_803_060_876_4), (0.4, 0.184_284_394_271_562), (0.75, 0.952_356_196_939_123)],
        ),
        (
            TemporalEase { speed: 0.2, influence: 0.8 },
            TemporalEase { speed: 0.8, influence: 0.6 },
            [(0.1, 0.022_575_494_262_157_7), (0.25, 0.069_984_188_941_293_1), (0.4, 0.149_334_636_959_052), (0.75, 0.738_196_785_378_389)],
        ),
        (
            TemporalEase { speed: 0.2, influence: 0.6 },
            TemporalEase { speed: 0.8, influence: 0.8 },
            [(0.1, 0.023_604_533_665_406), (0.25, 0.084_029_514_287_08), (0.4, 0.254_632_042_525_662), (0.75, 0.776_210_872_246_874)],
        ),
    ] {
        let e = Easing::from_temporal(out, into).unwrap();
        for (t, want) in samples {
            assert!((e.apply(t) - want).abs() < 1e-9, "{out:?} {into:?} at {t}: {} vs {want}", e.apply(t));
        }
        let (o, i) = e.temporal().unwrap();
        assert!((o.speed - out.speed).abs() < 1e-12 && (o.influence - out.influence).abs() < 1e-12);
        assert!((i.speed - into.speed).abs() < 1e-12 && (i.influence - into.influence).abs() < 1e-12);
    }
    // The straight line's speed with a third of influence on both sides is no ease.
    let line = Easing::from_temporal(TemporalEase { speed: 1.0, influence: 1.0 / 3.0 }, TemporalEase { speed: 1.0, influence: 1.0 / 3.0 }).unwrap();
    for k in 0..=20 {
        let t = k as f64 / 20.0;
        assert!((line.apply(t) - t).abs() < 1e-12, "{t}");
    }
    // Easy ease is symmetric and slow at its ends.
    let easy = Easing::from_temporal(TemporalEase::EASY, TemporalEase::EASY).unwrap();
    assert!((easy.apply(0.5) - 0.5).abs() < 1e-12);
    assert!(easy.apply(0.1) > 0.0 && easy.apply(0.1) < 0.1);
}

/// Both handles at full influence leave the time curve flat halfway; the closed-form inverse
/// keeps that point exact.
#[test]
fn full_influence_is_solved_exactly_at_the_flat_midpoint() {
    let e = Easing::from_temporal(TemporalEase { speed: 0.0, influence: 1.0 }, TemporalEase { speed: 0.0, influence: 1.0 }).unwrap();
    for t in [0.5 - 1e-9, 0.5 - 1e-12, 0.5, 0.5 + 1e-12, 0.5 + 1e-9, 0.1, 0.9] {
        // x = 0.5 + 4 (u − 0.5)³ exactly, and y = bez(u, [0, 0, 1, 1]).
        let u = 0.5 + ((t - 0.5) / 4.0f64).cbrt();
        let want = 3.0 * (1.0 - u) * u * u + u * u * u;
        assert!((e.apply(t) - want).abs() < 1e-12, "{t}: {} vs {want}", e.apply(t));
    }
}

/// The Bezier solver converges inside its step limit for any monotonic curve, to its tolerance.
#[test]
fn the_bezier_solver_converges_within_its_bound() {
    let mut seed = 0x9e37_79b9_7f4a_7c15u64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_001) as f64 / 1_000_000.0
    };
    let mut worst = 0;
    for _ in 0..20_000 {
        let (x1, x2, t) = (rnd(), rnd(), rnd());
        let p = [0.0, x1, x2, 1.0];
        let (s, steps) = crate::solve_counted(t, p);
        worst = worst.max(steps);
        assert!(steps < crate::SOLVE_STEPS, "{p:?} at {t}");
        assert!((crate::bez(s, p) - t).abs() <= crate::SOLVE_TOLERANCE, "{p:?} at {t}: {}", crate::bez(s, p));
    }
    // Near-flat curves, where Newton steps fail and halving takes over.
    for (x1, x2) in [(1.0, 0.0), (0.999_999, 0.0), (1.0, 1e-9), (0.0, 1.0), (0.0, 0.0), (1.0, 1.0)] {
        for k in 0..=1000 {
            let t = k as f64 / 1000.0;
            let p = [0.0, x1, x2, 1.0];
            let (s, steps) = crate::solve_counted(t, p);
            assert!(steps < crate::SOLVE_STEPS && (crate::bez(s, p) - t).abs() <= crate::SOLVE_TOLERANCE, "{p:?} at {t}: {steps} steps");
        }
    }
    assert!(worst <= 20, "{worst} steps");
    // A narrow segment far from 0, where the curve's x values differ only in their last bits:
    // x(s) = a + span s³, so the query halfway across the span is at s = 0.5^(1/3).
    let (a, span) = (0.5, 2f64.powi(-52));
    let p = [a, a, a, a + span];
    let s = crate::solve_monotonic(a + span / 2.0, p);
    assert!((s - 0.5f64.cbrt()).abs() < 1e-9, "{s}");
}

/// A handle so short that its speed overflows has no speed and influence form.
#[test]
fn a_handle_too_short_for_a_finite_speed_has_no_temporal_form() {
    assert_eq!(Easing::CubicBezier(f64::from_bits(1), 1.0, 0.5, 0.5).temporal(), None);
    assert!(Easing::CubicBezier(0.0, 0.0, 1.0, 1.0).temporal().is_some());
}

/// A Bezier with a handle past ±MAX_EASE_Y plays the same curve the graph editor shows for it.
#[test]
fn a_bezier_plays_the_curve_its_custom_form_draws() {
    for e in [Easing::CubicBezier(0.3, 20.0, 0.6, 1.0), Easing::CubicBezier(0.3, -1e30, 0.6, 1e30)] {
        let c = e.to_custom();
        for k in 0..=40 {
            let t = k as f64 * 0.025;
            assert!((e.apply(t) - c.apply(t)).abs() < 1e-5, "{e:?} at {t}: {} vs {}", e.apply(t), c.apply(t));
        }
    }
}

/// Any speed and influence a caller can supply either is refused or plays a finite curve.
#[test]
fn speed_and_influence_over_their_whole_range_play_finite_curves() {
    let mut seed = 0x51_7cc1_b727_220au64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 1_000_001) as f64 / 1_000_000.0
    };
    let mut accepted = 0;
    for _ in 0..20_000 {
        // Speeds from 1e-4 to 1e4 either way, spread evenly in magnitude.
        let side = |r: f64, i: f64| TemporalEase {
            speed: (r - 0.5).signum() * 10f64.powf((r - 0.5).abs() * 16.0 - 4.0),
            influence: MIN_INFLUENCE + i * (1.0 - MIN_INFLUENCE),
        };
        let (out, into) = (side(rnd(), rnd()), side(rnd(), rnd()));
        let Ok(e) = Easing::from_temporal(out, into) else { continue };
        accepted += 1;
        for k in 0..=20 {
            let v = e.apply(k as f64 / 20.0);
            assert!(v.is_finite() && v.abs() <= MAX_EASE_Y, "{out:?} {into:?}: {v}");
        }
    }
    assert!(accepted > 5_000, "{accepted}");
}

/// Eases, tracks and paths can be shared between render and export threads.
#[test]
fn the_types_are_send_and_sync() {
    fn check<T: Send + Sync>() {}
    check::<Easing>();
    check::<Track<f64>>();
    check::<Track<Vec2>>();
    check::<MotionPath>();
    check::<Spring>();
    check::<SpringState>();
}

#[test]
fn bezier_forms_draw_the_same_curves() {
    // Exact for these.
    for e in [
        Easing::Linear,
        Easing::QuadIn,
        Easing::QuadOut,
        Easing::CubicIn,
        Easing::CubicOut,
        Easing::BackIn,
        Easing::BackOut,
        Easing::Classic(-60.0),
        Easing::Classic(100.0),
        Easing::CubicBezier(0.2, 1.4, 0.6, -0.3),
    ] {
        let (a, b, c, d) = e.as_bezier().unwrap();
        let bez = Easing::CubicBezier(a, b, c, d);
        for i in 0..=20 {
            let t = i as f64 / 20.0;
            assert!((e.apply(t) - bez.apply(t)).abs() < 1e-9, "{e:?} at {t}");
        }
    }
    for e in [Easing::QuadInOut, Easing::CubicInOut] {
        let (a, b, c, d) = e.as_bezier().unwrap();
        let bez = Easing::CubicBezier(a, b, c, d);
        assert!((0..=20).all(|i| (e.apply(i as f64 / 20.0) - bez.apply(i as f64 / 20.0)).abs() < 0.08), "{e:?}");
    }
    // One curve can't draw a bounce, a hold or a multi-segment ease.
    assert_eq!(Easing::BounceOut.as_bezier(), None);
    assert_eq!(Easing::Hold.as_bezier(), None);
    assert_eq!(Easing::Custom(Arc::new(wave())).as_bezier(), None);
    let one = Easing::Custom(Arc::new(CustomEase::from_bezier(0.1, 0.9, 0.3, 1.2)));
    let (a, b, c, d) = one.as_bezier().unwrap();
    assert!((a - 0.1).abs() < 1e-15 && (b - 0.9).abs() < 1e-15 && (c - 0.3).abs() < 1e-15 && (d - 1.2).abs() < 1e-15);
}

#[test]
fn every_preset_becomes_a_custom_curve_for_the_graph() {
    for e in all() {
        let c = e.to_custom();
        assert!(c.points().len() <= MAX_EASE_POINTS, "{e:?}");
        let exact =
            matches!(e.family(), Some((EaseFamily::Quad | EaseFamily::Cubic | EaseFamily::Back | EaseFamily::Bounce, _))) || e.family().is_none();
        let tol = if exact { 2e-4 } else { 5e-3 };
        let mut worst = 0.0f64;
        for i in 0..=500 {
            let t = i as f64 / 500.0;
            worst = worst.max((c.apply(t) - e.apply(t)).abs());
        }
        assert!(worst < tol, "{e:?}: off by {worst} with {} points", c.points().len());
    }
    // A hold stays flat until a jump at the end; a spring is its polyline.
    let h = Easing::Hold.to_custom();
    for t in [1e-9, 0.25, 0.5, 0.9, 0.999_999] {
        assert_eq!(h.apply(t), Easing::Hold.apply(t), "{t}");
    }
    assert_eq!(h.apply(1.0), 1.0);
    let s = Easing::Spring(SpringEase::GENTLE);
    let sc = s.to_custom();
    assert_eq!(sc.points().len(), POLYLINE_POINTS);
    assert!((0..=100).all(|i| (sc.apply(i as f64 / 100.0) - s.apply(i as f64 / 100.0)).abs() < 0.02));
}

#[test]
fn ease_in_is_slow_at_start() {
    assert!(Easing::QuadIn.apply(0.25) < 0.25);
    assert!(Easing::QuadOut.apply(0.25) > 0.25);
    assert!(Easing::Classic(-100.0).apply(0.25) < 0.25);
    assert!(Easing::Classic(100.0).apply(0.25) > 0.25);
}

#[test]
fn hold_stays_at_the_start_until_the_end() {
    for t in [1e-300, 0.25, 0.5, 0.999_999_999] {
        assert_eq!(Easing::Hold.apply(t), 0.0);
    }
    assert_eq!(Easing::Hold.apply(1.0), 1.0);
    assert_eq!(Easing::from_name("hold"), Some(Easing::Hold));
}

#[test]
fn names_round_trip_and_older_aliases_still_read() {
    for e in Easing::named().chain([Easing::Hold]) {
        let n = e.name().unwrap();
        assert_eq!(Easing::from_name(&n), Some(e.clone()), "{n}");
        // The file form is the same name.
        #[cfg(feature = "serde")]
        assert_eq!(serde_json::to_string(&e).unwrap(), format!("\"{n}\""));
    }
    assert_eq!(Easing::from_name("back in-out"), Some(Easing::BackInOut));
    assert_eq!(Easing::from_name("ease_in"), Some(Easing::QuadIn));
    assert_eq!(Easing::from_name("Smooth"), Some(Easing::CubicBezier(0.25, 0.1, 0.25, 1.0)));
    assert_eq!(Easing::from_name("wobble"), None);
    // Separators do not count toward the longest name; a name longer than any preset is unknown.
    assert_eq!(Easing::from_name(&format!("{}elastic in out{}", " ".repeat(10_000), "-".repeat(10_000))), Some(Easing::ElasticInOut));
    assert_eq!(Easing::from_name(&"x".repeat(100_000)), None);
    assert_eq!(Easing::from_name("ElasticInOutX"), None);
    assert_eq!(Easing::named().count(), 28);
    assert_eq!(Easing::BounceInOut.label(), "Bounce In Out");
    // Files written with the variant names read back.
    #[cfg(feature = "serde")]
    {
        let old: Vec<Easing> = serde_json::from_str(r#"["QuadIn", {"Classic": 30.0}, {"CubicBezier": [0.1, 0.2, 0.3, 0.4]}, "Linear"]"#).unwrap();
        assert_eq!(old, [Easing::QuadIn, Easing::Classic(30.0), Easing::CubicBezier(0.1, 0.2, 0.3, 0.4), Easing::Linear]);
        let spring: Easing = serde_json::from_str(r#"{"Spring": {"mass": 1.0, "stiffness": 600.0, "damping": 15.0}}"#).unwrap();
        assert_eq!(spring, Easing::Spring(SpringEase { mass: 1.0, stiffness: 600.0, damping: 15.0 }));
        assert_eq!(
            serde_json::to_string(&Easing::Spring(SpringEase::GENTLE)).unwrap(),
            r#"{"Spring":{"mass":1.0,"stiffness":100.0,"damping":15.0}}"#
        );
    }
}

#[test]
fn a_multi_segment_curve_evaluates_continuously_through_its_points() {
    let c = wave();
    assert_eq!(c.points().len(), 4);
    for p in c.points() {
        assert!((c.apply(p.at.x) - p.at.y).abs() < 1e-9 || p.at.x == 0.0, "{p:?}");
    }
    let mut prev = c.apply(0.0);
    for i in 1..=10_000 {
        let v = c.apply(i as f64 / 10_000.0);
        assert!((v - prev).abs() < 2e-3, "a jump at {i}: {prev} → {v}");
        prev = v;
    }
    // Rises past the end value, then falls back below where it was, as drawn.
    assert!(c.apply(0.3) > 1.2 && c.apply(0.55) < 0.4);
}

#[test]
fn adding_a_point_keeps_the_shape() {
    let c = wave();
    let (d, i) = c.insert_at(0.42).unwrap();
    assert_eq!((d.points().len(), i), (5, 2));
    assert!(d.points()[2].smooth);
    for k in 0..=200 {
        let t = k as f64 / 200.0;
        assert!((c.apply(t) - d.apply(t)).abs() < 1e-4, "at {t}");
    }
    // Not at an end or on a point.
    assert!(c.insert_at(0.0).is_none() && c.insert_at(1.0).is_none() && c.insert_at(0.3).is_none() && c.insert_at(f64::NAN).is_none());
}

/// Splitting a steep curve can put a piece's handle past that piece's other end in x; the piece
/// still runs forwards, so the handle is kept and the curve keeps its shape.
#[test]
fn adding_a_point_keeps_the_shape_where_the_pieces_handles_cross() {
    let c = CustomEase::from_bezier(1.0, 0.0, 0.0, 1.0);
    for t in [7.0 / 16.0, 0.2, 0.49, 0.51, 0.8] {
        let (d, i) = c.insert_at(t).unwrap();
        assert_eq!(i, 1);
        assert_forwards(&d);
        // x is flat at 0.5, so a solve within 1e-12 in x is within about 1e-4 in y there.
        assert!((d.apply(0.5) - 0.5).abs() < 2e-4, "{t}: {}", d.apply(0.5));
        for k in 0..=400 {
            let u = k as f64 / 400.0;
            let tol = if (u - 0.5).abs() > 0.05 { 1e-9 } else { 2e-4 };
            assert!((c.apply(u) - d.apply(u)).abs() < tol, "{t} at {u}: {} vs {}", c.apply(u), d.apply(u));
        }
        // Removing the point again leaves a valid curve of one segment.
        assert_forwards(&d.remove(1).unwrap());
    }
    // A handle past the far end is kept only while x still runs forwards.
    let one = |out_x: f64| {
        CustomEase::new(vec![
            EasePoint { at: Vec2::ZERO, out: Vec2::new(out_x, 0.0), ..Default::default() },
            EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-0.4, 0.0), ..Default::default() },
        ])
    };
    let (kept, cut) = (one(1.2), one(2.5));
    assert_eq!((kept.points()[0].out.x, cut.points()[0].out.x), (1.2, 1.0));
    assert_forwards(&kept);
    assert_forwards(&cut);
    // Such a curve is not a cubic-bezier() one, and from_bezier keeps x in the unit box.
    assert_eq!(Easing::Custom(Arc::new(kept)).as_bezier(), None);
    assert_eq!(Easing::Custom(Arc::new(cut)).as_bezier(), Some((1.0, 0.0, 0.6, 1.0)));
    assert_eq!(CustomEase::from_bezier(1.2, 0.0, -0.1, 1.0).points()[0].out.x, 1.0);
    assert_eq!(CustomEase::from_bezier(1.2, 0.0, -0.1, 1.0).points()[1].h_in.x, -1.0);
}

/// A smooth point near the value limit has its handles shortened along their line, so they stay
/// in line.
#[test]
fn a_smooth_point_near_the_value_limit_keeps_its_handles_in_line() {
    let c = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, ..Default::default() },
        EasePoint { at: Vec2::new(0.1, 9.9), ..Default::default() },
        EasePoint { at: Vec2::new(1.0, 1.0), ..Default::default() },
    ]);
    let s = c.set_smooth(1, true).unwrap();
    let p = s.points()[1];
    assert!(p.smooth && p.out.length() > 0.05 && p.h_in.length() > 0.01 && in_line(&p), "{p:?}");
    assert!(p.at.y + p.out.y <= MAX_EASE_Y && p.at.y + p.h_in.y >= -MAX_EASE_Y, "{p:?}");
    assert_forwards(&s);
    // Moving the smooth point up to the limit, or next to a neighbor, keeps them in line too.
    for to in [Vec2::new(0.5, 9.99), Vec2::new(0.999, 5.0), Vec2::new(0.001, -9.99)] {
        let m = s.move_point(1, to).unwrap();
        let q = m.points()[1];
        assert!(q.smooth && in_line(&q), "{to:?}: {q:?}");
        assert_forwards(&m);
    }
}

#[test]
fn point_edits_keep_time_running_forwards() {
    let c = wave();
    // Dragging a point past its neighbor stops at the neighbor.
    let d = c.move_point(1, Vec2::new(0.95, 2.0)).unwrap();
    assert_eq!(d.points()[1].at, Vec2::new(0.6, 2.0));
    // The ends don't move.
    assert!(c.move_point(0, Vec2::new(0.5, 0.5)).is_none() && c.move_point(3, Vec2::new(0.5, 0.5)).is_none());
    // A handle dragged backwards in time stops at its point.
    let h = c.move_handle(1, Handle::Out, Vec2::new(0.0, 5.0)).unwrap();
    assert_eq!(h.points()[1].out.x, 0.0);
    // A smooth point's other handle turns with it.
    let s = c.set_smooth(1, true).unwrap();
    let t = s.move_handle(1, Handle::Out, Vec2::new(0.45, 1.5)).unwrap();
    let p = t.points()[1];
    assert!(p.smooth && (p.h_in.normalized().dot(p.out.normalized()) + 1.0).abs() < 1e-4, "{p:?}");
    // A corner has its handles on the point.
    let k = s.set_smooth(1, false).unwrap();
    assert_eq!((k.points()[1].h_in, k.points()[1].out), (Vec2::ZERO, Vec2::ZERO));
    // Removing.
    assert_eq!(c.remove(2).unwrap().points().len(), 3);
    assert!(c.remove(0).is_none() && c.remove(3).is_none());
}

/// Whether a smooth inner point's handles are in line (or one of them has no length).
fn in_line(p: &EasePoint) -> bool {
    let (a, b) = (p.h_in, p.out);
    a.length() < 1e-9 || b.length() < 1e-9 || (a.normalized().dot(b.normalized()) + 1.0).abs() < 1e-9
}

/// Turning a smooth point's other handle keeps it in line even where it has less room on its
/// side than its length: it is shortened along the line instead of clamped out of it.
#[test]
fn a_smooth_point_keeps_its_handles_in_line_near_a_neighbor() {
    let c = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::new(0.3, 0.0), smooth: false },
        EasePoint { at: Vec2::new(0.9, 0.5), h_in: Vec2::new(-0.1, -0.3), out: Vec2::new(0.1, 0.3), smooth: true },
        EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-0.05, 0.0), out: Vec2::ZERO, smooth: false },
    ]);
    let d = c.move_handle(1, Handle::In, Vec2::new(0.5, 0.1)).unwrap();
    let p = d.points()[1];
    assert!(p.smooth && p.out.length() > 0.05 && in_line(&p), "{p:?}");
    assert!(p.out.x <= 0.1 + 1e-12, "{p:?}");
    assert_forwards(&d);
    // The same near the y limit.
    let e = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, h_in: Vec2::ZERO, out: Vec2::ZERO, smooth: false },
        EasePoint { at: Vec2::new(0.5, 9.9), h_in: Vec2::new(-0.2, 0.0), out: Vec2::new(0.2, 0.0), smooth: true },
        EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::ZERO, out: Vec2::ZERO, smooth: false },
    ]);
    let f = e.move_handle(1, Handle::Out, Vec2::new(0.6, 9.0)).unwrap();
    let q = f.points()[1];
    assert!(q.h_in.length() > 0.05 && in_line(&q) && q.at.y + q.h_in.y <= MAX_EASE_Y + 1e-12, "{q:?}");
}

/// Turning a smooth point's other handle keeps that handle's length when it fits, however short.
#[test]
fn turning_a_smooth_point_keeps_a_short_other_handle_short() {
    let c = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, ..Default::default() },
        EasePoint { at: Vec2::new(0.5, 0.5), h_in: Vec2::new(-0.0005, 0.0), out: Vec2::new(0.2, 0.0), smooth: true },
        EasePoint { at: Vec2::new(1.0, 1.0), ..Default::default() },
    ]);
    let d = c.move_handle(1, Handle::Out, Vec2::new(0.7, 0.6)).unwrap();
    let p = d.points()[1];
    assert!(p.smooth && in_line(&p) && (p.h_in.length() - 0.0005).abs() < 1e-15, "{p:?}");
    // A handle with no length is given one.
    let z = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, ..Default::default() },
        EasePoint { at: Vec2::new(0.5, 0.5), out: Vec2::new(0.2, 0.0), smooth: true, ..Default::default() },
        EasePoint { at: Vec2::new(1.0, 1.0), ..Default::default() },
    ]);
    let q = z.move_handle(1, Handle::Out, Vec2::new(0.7, 0.6)).unwrap().points()[1];
    assert!(q.smooth && in_line(&q) && (q.h_in.length() - 1e-3).abs() < 1e-15, "{q:?}");
}

/// A point marked smooth whose handles are not in line is read as a corner, its handles kept.
#[test]
fn a_smooth_mark_on_a_corner_is_cleared() {
    let pts = |out: Vec2| {
        vec![
            EasePoint { at: Vec2::ZERO, out: Vec2::new(0.1, 0.0), smooth: true, ..Default::default() },
            EasePoint { at: Vec2::new(0.5, 0.5), h_in: Vec2::new(-0.1, 0.0), out, smooth: true },
            EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-0.1, 0.0), smooth: true, ..Default::default() },
        ]
    };
    let corner = CustomEase::new(pts(Vec2::new(0.1, 0.1)));
    let p = corner.points()[1];
    assert!(!p.smooth && p.h_in == Vec2::new(-0.1, 0.0) && p.out == Vec2::new(0.1, 0.1), "{p:?}");
    let smooth = CustomEase::new(pts(Vec2::new(0.2, 0.0)));
    assert!(smooth.points()[1].smooth);
    // The ends have one handle each and are never smooth.
    assert!(!smooth.points()[0].smooth && !smooth.points()[2].smooth);
    // Past the value limit, such a point's handle is clipped as a corner's: its x is kept.
    let high = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, ..Default::default() },
        EasePoint { at: Vec2::new(0.5, 9.5), h_in: Vec2::new(-0.2, 2.0), out: Vec2::new(0.1, 0.0), smooth: true },
        EasePoint { at: Vec2::new(1.0, 1.0), ..Default::default() },
    ]);
    let q = high.points()[1];
    assert!(!q.smooth && q.h_in == Vec2::new(-0.2, 0.5), "{q:?}");
    #[cfg(feature = "serde")]
    {
        let f: CustomEase = serde_json::from_str(
            r#"{"points": [
                {"at": {"x": 0.0, "y": 0.0}, "out": {"x": 0.1, "y": 0.0}, "smooth": true},
                {"at": {"x": 0.5, "y": 0.5}, "in": {"x": -0.1, "y": 0.0}, "out": {"x": 0.1, "y": 0.1}, "smooth": true},
                {"at": {"x": 1.0, "y": 1.0}, "in": {"x": -0.1, "y": 0.0}, "smooth": true}
            ]}"#,
        )
        .unwrap();
        assert_eq!(f, corner);
    }
}

/// A point can be added at any time strictly inside the curve that is not already a point's.
#[test]
fn a_point_can_be_added_close_to_an_end_or_a_point() {
    let line = CustomEase::linear();
    let (d, i) = line.insert_at(0.00005).unwrap();
    assert_eq!((d.points().len(), i), (3, 1));
    assert!((d.points()[1].at.x - 0.00005).abs() < 1e-15, "{:?}", d.points()[1]);
    assert_forwards(&d);
    let (e, _) = d.insert_at(0.00006).unwrap();
    assert_eq!(e.points().len(), 4);
    assert_forwards(&e);
    for k in 0..=200 {
        let t = k as f64 / 200.0;
        assert!((e.apply(t) - t).abs() < 1e-9, "at {t}");
    }
    assert!(d.insert_at(d.points()[1].at.x).is_none());
    // Where the curve is too flat in x to solve for the time, no point is added rather than one
    // at another time.
    let flat = CustomEase::from_bezier(0.0, 0.0, 0.0, 0.0);
    assert!(flat.insert_at(1e-13).is_none());
    let (g, _) = flat.insert_at(0.3).unwrap();
    assert!((g.points()[1].at.x - 0.3).abs() < 1e-9, "{:?}", g.points()[1]);
}

/// On a segment far narrower than its handle's reach can underflow when squared, a handle that
/// would make x run backwards is still brought back.
#[test]
fn a_very_narrow_segment_does_not_run_backwards() {
    let c = CustomEase::new(vec![
        EasePoint { at: Vec2::ZERO, out: Vec2::new(1.5e-180, 0.0), ..Default::default() },
        EasePoint { at: Vec2::new(1e-180, 0.5), ..Default::default() },
        EasePoint { at: Vec2::new(1.0, 1.0), ..Default::default() },
    ]);
    let [a, b, cc, d] = c.segment(0).unwrap();
    // x on the span 0..1, sampled: it never decreases.
    let span = d.x - a.x;
    let xs = [0.0, (b.x - a.x) / span, (cc.x - a.x) / span, 1.0];
    let mut prev = 0.0;
    for k in 0..=1000 {
        let x = crate::bez(k as f64 / 1000.0, xs);
        assert!(x >= prev - 1e-15, "{xs:?} at {k}: {x} after {prev}");
        prev = x;
    }
    assert_forwards(&c);
}

/// Every segment's x never decreases: its handles reach toward their neighbors, and x′ (a
/// quadratic with Bernstein coefficients b − a, cc − b and d − cc) is never negative.
fn assert_forwards(c: &CustomEase) {
    let pts = c.points();
    assert!(pts.len() >= 2 && pts.len() <= MAX_EASE_POINTS);
    assert_eq!((pts[0].at, pts[pts.len() - 1].at), (Vec2::ZERO, Vec2::new(1.0, 1.0)));
    for i in 0..pts.len() - 1 {
        let [a, b, cc, d] = c.segment(i).unwrap();
        // Divided by the span, so squaring does not underflow on a narrow segment.
        let unit = if d.x > a.x { d.x - a.x } else { 1.0 };
        let (k0, k1, k2) = ((b.x - a.x) / unit, (cc.x - b.x) / unit, (d.x - cc.x) / unit);
        let forwards = a.x <= d.x && k0 >= 0.0 && k2 >= 0.0 && (k1 >= 0.0 || k1 * k1 <= k0 * k2 * (1.0 + 1e-9));
        assert!(forwards, "segment {i}: {a:?} {b:?} {cc:?} {d:?}");
        for v in [a, b, cc, d] {
            assert!(v.is_finite() && v.y.abs() <= MAX_EASE_Y + 1e-9);
        }
    }
    for k in 0..=100 {
        assert!(c.apply(k as f64 / 100.0).is_finite());
    }
}

#[test]
fn x_stays_monotonic_after_any_edit() {
    let mut seed = 0x2545_f491_4f6c_dd1du64;
    let mut rnd = || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        (seed % 10_000) as f64 / 10_000.0
    };
    let mut c = wave();
    for _ in 0..3000 {
        let n = c.points().len();
        let i = (rnd() * n as f64) as usize % n;
        let to = Vec2::new(rnd() * 1.6 - 0.3, rnd() * 30.0 - 15.0);
        let op = (rnd() * 6.0) as u32;
        let next = match op {
            0 => c.insert_at(rnd()).map(|r| r.0),
            1 => c.move_point(i, to),
            2 => c.move_handle(i, Handle::In, to),
            3 => c.move_handle(i, Handle::Out, to),
            4 => c.set_smooth(i, rnd() > 0.5),
            _ => {
                if n > 6 {
                    c.remove(i)
                } else {
                    None
                }
            }
        };
        if let Some(d) = next {
            assert_forwards(&d);
            if matches!(op, 2 | 3)
                && let Some(p) = d.points().get(i).filter(|p| p.smooth)
            {
                assert!(in_line(p), "{p:?}");
            }
            c = d;
        }
    }
}

#[test]
fn the_largest_index_is_a_missing_point() {
    let w = wave();
    assert!(w.segment(usize::MAX).is_none());
    assert!(w.move_point(usize::MAX, Vec2::new(0.5, 0.5)).is_none());
    assert!(w.move_handle(usize::MAX, Handle::Out, Vec2::new(0.1, 0.1)).is_none());
    assert!(w.remove(usize::MAX).is_none());
    assert!(w.set_smooth(usize::MAX, true).is_none());
}

#[test]
fn hostile_curves_are_repaired() {
    // NaN, reversed x, out-of-range values and handles reaching backwards.
    let nan = EasePoint { at: Vec2::new(f64::NAN, 0.5), ..Default::default() };
    let pts = vec![
        EasePoint { at: Vec2::new(1.0, 1.0), h_in: Vec2::new(-5.0, f64::INFINITY), ..Default::default() },
        EasePoint { at: Vec2::new(0.6, 400.0), h_in: Vec2::new(3.0, 0.0), out: Vec2::new(-2.0, 0.0), smooth: true },
        nan,
        EasePoint { at: Vec2::new(0.2, -1.0), ..Default::default() },
        EasePoint { at: Vec2::new(-3.0, 0.0), out: Vec2::new(f64::NAN, 1.0), ..Default::default() },
    ];
    assert_eq!(CustomEase::try_new(pts.clone()), Err(EaseError::NotFinite("ease point")));
    let c = CustomEase::new(pts);
    assert_eq!(c.points().len(), 4);
    assert_forwards(&c);
    assert_eq!(c.points()[2].at, Vec2::new(0.6, MAX_EASE_Y));
    // Thousands of points are thinned to the cap, and stay a valid curve.
    let many: Vec<EasePoint> = (0..20_000)
        .map(|i| EasePoint {
            at: Vec2::new(((i * 7919) % 20_000) as f64 / 20_000.0, (i as f64 * 0.37).sin()),
            out: Vec2::new(0.5, 0.0),
            h_in: Vec2::new(-0.5, 0.0),
            smooth: false,
        })
        .collect();
    assert_eq!(CustomEase::try_new(many.clone()), Err(EaseError::Full(MAX_EASE_POINTS)));
    let c = CustomEase::new(many);
    assert_eq!(c.points().len(), MAX_EASE_POINTS);
    assert_forwards(&c);
    // Too few points: the line.
    assert_eq!(CustomEase::new(vec![nan]), CustomEase::linear());
    assert_eq!(CustomEase::try_new(vec![EasePoint::default()]), Err(EaseError::TooFew { what: "a custom ease", need: 2 }));
    // From a file, the same repairs.
    #[cfg(feature = "serde")]
    {
        let f: CustomEase = serde_json::from_str(
            r#"{"points": [{"at": {"x": 0.9, "y": 3.0}}, {"at": {"x": 0.1, "y": 0.0}, "out": {"x": -1.0, "y": 0.0}}, {"at": {"x": 0.5, "y": 0.5}}]}"#,
        )
        .unwrap();
        assert_forwards(&f);
        let e: Easing = serde_json::from_str(r#"{"Custom": {"points": []}}"#).unwrap();
        assert_eq!(e, Easing::Custom(Arc::new(CustomEase::linear())));
    }
}

#[test]
fn custom_eases_round_trip_through_files_and_flat_lists() {
    #[cfg(feature = "serde")]
    {
        let e = Easing::Custom(Arc::new(wave()));
        let back: Easing = serde_json::from_str(&serde_json::to_string(&e).unwrap()).unwrap();
        assert_eq!(back, e);
    }
    let flat = wave().to_flat();
    assert_eq!(flat.len(), 10);
    assert_eq!(CustomEase::from_flat(&flat), Some(wave()));
    assert!(CustomEase::from_flat(&flat[..9]).is_none());
}

#[test]
fn a_sampled_ease_reads_its_table_between_samples() {
    let s = steps();
    let e = Easing::Sampled(Arc::new(s.clone()));
    assert_eq!((e.apply(0.0), e.apply(0.1), e.apply(0.25), e.apply(0.6), e.apply(0.9), e.apply(1.0)), (0.0, 0.0, 0.25, 0.5, 0.75, 1.0));
    // Between two samples, the straight line between them.
    let lin = SampledEase::new("Line", "p", "l", vec![0.0, 0.5, 1.0]);
    assert!((lin.apply(0.25) - 0.25).abs() < 1e-12 && (lin.apply(0.75) - 0.75).abs() < 1e-12);
    // Time outside 0..1, or not a number, is clamped.
    assert_eq!((e.apply(-3.0), e.apply(7.0), e.apply(f64::NAN)), (0.0, 1.0, 0.0));
    assert_eq!(e.label(), "Stepped 4");
    assert!(e.family().is_none() && e.as_bezier().is_none());
    assert_eq!(e.sampled().map(|s| (s.plugin(), s.ease())), Some(("org.example.stepped-ease", "steps4")));
}

#[test]
fn a_sampled_ease_is_always_valid() {
    let s = SampledEase::new("  ", "bad id!/..", "x\u{202e}y", vec![0.3, f64::NAN, 99.0, -f64::INFINITY, 0.2]);
    assert_eq!(s.values(), &[0.0, 0.25, MAX_EASE_Y, 0.75, 1.0]);
    assert_eq!((s.name(), s.plugin(), s.ease()), ("Sampled ease", "badid..", "xy"));
    assert_eq!(SampledEase::new("x", "p", "e", vec![]).values(), &[0.0, 1.0]);
    assert_eq!(SampledEase::new("x", "p", "e", vec![0.5]).values(), &[0.0, 1.0]);
    let long = SampledEase::new("x", "p", "e", (0..100_000).map(|i| i as f64 / 99_999.0).collect());
    assert_eq!(long.values().len(), MAX_EASE_SAMPLES);
    assert!((long.apply(0.5) - 0.5).abs() < 1e-3);
    // The checked form refuses instead.
    assert_eq!(SampledEase::try_new("x", "p", "e", vec![0.0, f64::NAN, 1.0]), Err(EaseError::NotFinite("sample")));
    assert!(matches!(SampledEase::try_new("x", "p", "e", vec![0.0, 99.0, 1.0]), Err(EaseError::OutOfRange { .. })));
    assert_eq!(SampledEase::try_new("x", "p", "e", vec![0.0]), Err(EaseError::TooFew { what: "a sampled ease", need: 2 }));
    assert_eq!(SampledEase::try_new("x", "p", "e", vec![0.5; MAX_EASE_SAMPLES + 1]), Err(EaseError::Full(MAX_EASE_SAMPLES)));
    assert_eq!(SampledEase::try_new("x", "p", "e", vec![0.2, 0.5, 0.9]).unwrap().values(), &[0.0, 0.5, 1.0]);
}

/// Thinning a table picks evenly spread samples, first and last included, for any length a
/// `usize` can hold (a 32-bit target overflows `k · (n − 1)` from about 4.2 million values).
#[test]
fn thinning_a_huge_table_does_not_overflow() {
    use crate::sampled::thinned;
    let last = MAX_EASE_SAMPLES - 1;
    for n in [MAX_EASE_SAMPLES + 1, 5_000_000, usize::MAX / 3, usize::MAX] {
        assert_eq!(thinned(0, n), 0);
        assert_eq!(thinned(last, n), n - 1, "{n}");
        assert!(thinned(last / 2, n) >= (n - 1) / 2 - 1 && thinned(last / 2, n) <= (n - 1) / 2, "{n}");
        assert!((1..=last).all(|k| thinned(k, n) > thinned(k - 1, n)), "{n}");
    }
}

/// Invisible format characters do not count as a name, so a name of only those shows as
/// "Sampled ease"; they are taken out of a visible name too.
#[test]
fn a_sampled_ease_name_of_invisible_characters_is_no_name() {
    let blank: String = "\u{200b}\u{200d}\u{feff}\u{2060}\u{ad}".repeat(13);
    assert_eq!(SampledEase::new(&blank, "p", "e", vec![0.0, 1.0]).name(), "Sampled ease");
    assert_eq!(SampledEase::new(" \u{200b} Bounce\u{200d}y \u{feff}", "p", "e", vec![0.0, 1.0]).name(), "Bouncey");
    // A visible name longer than the limit keeps its first 64 characters, after leading padding.
    let padded = format!("{}{}", " \u{200b}".repeat(100), "a".repeat(80));
    assert_eq!(SampledEase::new(&padded, "p", "e", vec![0.0, 1.0]).name(), "a".repeat(64));
}

#[cfg(feature = "serde")]
#[test]
fn a_sampled_ease_round_trips_through_the_file_format_and_survives_bad_files() {
    let e = Easing::Sampled(Arc::new(steps()));
    let text = serde_json::to_string(&e).unwrap();
    assert!(
        text.starts_with("{\"Sampled\":{\"name\":\"Stepped 4\",\"plugin\":\"org.example.stepped-ease\",\"ease\":\"steps4\",\"values\":[0.0,"),
        "{text}"
    );
    assert_eq!(serde_json::from_str::<Easing>(&text).unwrap(), e);
    // A hand-edited table is made valid on load; a value that is not a number is a broken file.
    assert!(serde_json::from_str::<Easing>(r#"{"Sampled":{"values":[5,0.5,"x"]}}"#).is_err());
    let odd: Easing = serde_json::from_str(r#"{"Sampled":{"name":"Odd","values":[5,0.5,-20]}}"#).unwrap();
    assert_eq!(odd.sampled().unwrap().values(), &[0.0, 0.5, 1.0]);
}

/// A file with a huge list loads from its first 65,536 entries; the rest are skipped unread,
/// so they are never stored (here they would not even parse as entries).
#[cfg(feature = "serde")]
#[test]
fn huge_lists_in_files_are_read_only_up_to_a_limit() {
    let mut values = String::from(r#"{"Sampled":{"values":["#);
    for i in 0..70_000 {
        values.push_str(if i < 65_536 { "0.5," } else { r#"{"not":"a value"},"# });
    }
    values.push_str("1]}}");
    let e: Easing = serde_json::from_str(&values).unwrap();
    assert_eq!(e.sampled().unwrap().values().len(), MAX_EASE_SAMPLES);
    let mut points = String::from(r#"{"Custom":{"points":["#);
    for i in 0..70_000 {
        points.push_str(if i < 65_536 { r#"{"at":{"x":0.5,"y":0.5}},"# } else { r#""not a point","# });
    }
    points.push_str(r#"{"at":{"x":1,"y":1}}]}}"#);
    let c: Easing = serde_json::from_str(&points).unwrap();
    assert!(matches!(&c, Easing::Custom(f) if f.points().len() <= MAX_EASE_POINTS), "{c:?}");
    // A long name keeps only its first characters.
    let long = format!(r#"{{"Sampled":{{"name":"{}","plugin":"p","ease":"e","values":[0,1]}}}}"#, "é".repeat(100_000));
    let s: Easing = serde_json::from_str(&long).unwrap();
    assert_eq!(s.sampled().unwrap().name(), "é".repeat(64));
    // Characters the cleanup removes do not push the kept text out of reach.
    let padded = format!(r#"{{"Sampled":{{"name":"{}Curve","plugin":"{}p1","ease":"e","values":[0,1]}}}}"#, " ".repeat(10_000), "!".repeat(10_000));
    let s: Easing = serde_json::from_str(&padded).unwrap();
    assert_eq!((s.sampled().unwrap().name(), s.sampled().unwrap().plugin()), ("Curve", "p1"));
}

#[test]
fn a_sampled_ease_becomes_a_custom_curve_through_its_samples() {
    let wave = SampledEase::new(
        "Wave",
        "p",
        "w",
        (0..257)
            .map(|i| {
                let t = i as f64 / 256.0;
                t + 0.2 * (t * std::f64::consts::TAU).sin()
            })
            .collect(),
    );
    let c = wave.to_polyline();
    assert_eq!(c.points().len(), POLYLINE_POINTS);
    for i in 0..=100 {
        let t = i as f64 / 100.0;
        assert!((c.apply(t) - wave.apply(t)).abs() < 0.01, "t = {t}: {} vs {}", c.apply(t), wave.apply(t));
    }
    let short = SampledEase::new("Short", "p", "s", vec![0.0, 0.8, 1.0]).to_polyline();
    assert_eq!(short.points().len(), 3);
    assert!((short.apply(0.5) - 0.8).abs() < 1e-3);
}
