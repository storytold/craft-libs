use craft_time::{
    FrameRate, SAMPLE_RATES, TICKS_PER_SECOND, Tick, TimeDisplay, TimeRange, format_feet_frames, format_time, format_timecode_ae,
    format_timecode_frames, frames_to_fields, parse_feet_frames, parse_timecode,
};
use proptest::prelude::*;

#[test]
fn ticks_per_second_divides_all_standard_rates_evenly() {
    assert_eq!(TICKS_PER_SECOND, 254_016_000_000);

    // Divisible by all standard frame rate numerators and denominators
    for r in FrameRate::COMMON {
        let den = r.den as i128;
        let num = r.num as i128;
        let product = TICKS_PER_SECOND as i128 * den;
        assert_eq!(product % num, 0, "Rate {} ({} / {}) does not divide TICKS_PER_SECOND evenly", r.label(), r.num, r.den);
    }

    // Audio sample rates
    for &sr in &SAMPLE_RATES {
        assert_eq!(TICKS_PER_SECOND % sr, 0, "Sample rate {sr} does not divide TICKS_PER_SECOND evenly");
    }
}

#[test]
fn frame_roundtrip_all_common_rates() {
    for r in FrameRate::COMMON {
        for f in -10_000..=10_000 {
            let t = r.tick_of(f);
            let back = r.frame_at(t);
            assert_eq!(back, f, "Roundtrip failed for {} at frame {}", r.label(), f);
            assert_eq!(r.snap(t), t, "Snap of exact boundary should be identity");
            assert_eq!(r.snap_nearest(t), t, "Snap nearest of exact boundary should be identity");
        }
    }
}

#[test]
fn time_range_invariants() {
    let r1 = TimeRange::new(Tick(100), Tick(50));
    assert_eq!(r1.end(), Tick(150));
    assert!(r1.contains(Tick(100)));
    assert!(r1.contains(Tick(149)));
    assert!(!r1.contains(Tick(150)));

    let r2 = TimeRange::new(Tick(120), Tick(40));
    assert!(r1.overlaps(&r2));
    let inter = r1.intersect(&r2);
    assert_eq!(inter, Some(TimeRange::new(Tick(120), Tick(30))));

    let empty = TimeRange::new(Tick(100), Tick(0));
    assert!(empty.is_empty());
}

#[test]
fn tick_checked_and_saturating_arithmetic_contract() {
    // 1. Division overflow: i64::MIN / -1
    let min_tick = Tick(i64::MIN);
    assert_eq!(min_tick.checked_div(-1), None, "checked_div must return None on i64::MIN / -1");
    assert_eq!(min_tick.saturating_div(-1), Some(Tick(i64::MAX)), "saturating_div must saturate to Tick(i64::MAX)");
    assert_eq!(min_tick / -1, Tick(i64::MAX), "Operator Div must saturate i64::MIN / -1 to Tick(i64::MAX) without panicking");
    assert_eq!(Tick(i64::MIN + 1) / -1, Tick(i64::MAX));
    assert_eq!(Tick(i64::MIN + 2) / -1, Tick(i64::MAX - 1));

    // 2. Division by zero
    let ten = Tick(10);
    assert_eq!(ten.checked_div(0), None, "checked_div must return None on division by zero");
    assert_eq!(ten.saturating_div(0), None, "saturating_div must return None on division by zero");
    assert_eq!(ten / 0, Tick::ZERO, "Operator Div fallback returns Tick::ZERO on zero divisor");

    // 3. Remainder overflow & zero divisor
    assert_eq!(min_tick.checked_rem(Tick(-1)), None, "checked_rem must return None on i64::MIN % -1");
    assert_eq!(ten.checked_rem(Tick(0)), None, "checked_rem must return None on zero divisor");
    assert_eq!(min_tick % Tick(-1), Tick::ZERO, "Operator Rem on i64::MIN % -1 produces Tick::ZERO");
    assert_eq!(ten % Tick(0), Tick::ZERO, "Operator Rem on zero divisor produces Tick::ZERO");

    // 4. Checked Add / Sub / Mul
    assert_eq!(Tick(i64::MAX).checked_add(Tick(1)), None);
    assert_eq!(Tick(i64::MIN).checked_sub(Tick(1)), None);
    assert_eq!(Tick(i64::MAX).checked_mul(2), None);
    assert_eq!(Tick(100).checked_add(Tick(50)), Some(Tick(150)));
    assert_eq!(Tick(100).checked_sub(Tick(50)), Some(Tick(50)));
    assert_eq!(Tick(100).checked_mul(5), Some(Tick(500)));

    // 5. Saturating Add / Sub / Mul
    assert_eq!(Tick(i64::MAX).saturating_add(Tick(1)), Tick(i64::MAX));
    assert_eq!(Tick(i64::MIN).saturating_sub(Tick(1)), Tick(i64::MIN));
    assert_eq!(Tick(i64::MAX).saturating_mul(2), Tick(i64::MAX));
}

#[test]
fn hostile_rates_and_timecodes_never_panic() {
    let zero = FrameRate { num: 0, den: 0 };
    for r in [
        zero,
        FrameRate { num: 0, den: 1 },
        FrameRate { num: 25, den: 0 },
        FrameRate { num: -30, den: 1 },
        FrameRate::from_f64(0.0),
        FrameRate::from_f64(f64::NAN),
    ] {
        assert_eq!(r.tick_of(10), FrameRate::default().tick_of(10));
        assert_eq!(r.frame_at(Tick(TICKS_PER_SECOND)), FrameRate::default().frame_at(Tick(TICKS_PER_SECOND)));
        assert!(r.frame_duration() > Tick::ZERO);
        assert!(r.timecode_base() >= 1);
        let _ = r.snap_nearest(Tick(12345));
        let _ = format_time(Tick(TICKS_PER_SECOND), r, true, TimeDisplay::Timecode, 48_000);
    }
    assert_eq!(Tick::from_rational(100, 1, 0), Tick::ZERO);
    assert_eq!(Tick::from_units(100, 0), Tick::ZERO);
    assert_eq!(Tick(5).to_rational_floor(0, 1), 0);
    assert_eq!(Tick(5).to_rational_round(0, 1), 0);
    for t in [i64::MIN, -1, 0, 1, i64::MAX] {
        for (n, d) in [(i64::MIN, 1), (-1, 1000), (1, i64::MAX), (i64::MAX, i64::MIN), (1, 0)] {
            let _ = Tick(t).to_rational_round(n, d);
        }
    }
    assert_eq!(Tick(5).mul_ratio(1, 0), Tick::ZERO);
    assert_eq!(Tick(5).clamp(Tick(10), Tick(0)), Tick(0));
    let _ = format_timecode_frames(i64::MIN, FrameRate::FPS_29_97, true);
    for tc in ["99999999999:99:99:99", "9223372036854775807", "+9223372036854775807", "99999999999;59;59;29"] {
        let _ = parse_timecode(tc, FrameRate::FPS_29_97, true, i64::MAX);
        let _ = parse_timecode(tc, FrameRate::FPS_25, false, i64::MIN);
    }
}

#[test]
fn boundary_rates_saturate_without_wrapping_and_keep_representable_values_exact() {
    let slow = FrameRate { num: 1, den: i64::MAX };
    assert_eq!(slow.frame_duration(), Tick(i64::MAX));
    assert_eq!(slow.tick_of(i64::MAX), Tick(i64::MAX));
    assert_eq!(slow.tick_of(i64::MIN), Tick(i64::MIN));
    assert_eq!(slow.snap_nearest(Tick(i64::MAX)), Tick::ZERO, "compare the real boundary before saturating");
    assert_eq!(slow.snap_nearest(Tick(i64::MIN)), Tick::ZERO);

    let fast = FrameRate { num: i64::MAX, den: 1 };
    assert_eq!(fast.tick_of(i64::MAX), Tick(TICKS_PER_SECOND), "large factors can cancel exactly");
    assert_eq!(fast.tick_of(-i64::MAX), Tick(-TICKS_PER_SECOND));
    assert_eq!(fast.frame_at(Tick(TICKS_PER_SECOND)), i64::MAX);
    assert_eq!(fast.frame_at(Tick(i64::MAX)), i64::MAX);
    assert_eq!(fast.frame_at(Tick(i64::MIN)), i64::MIN);
    assert_eq!(fast.timecode_base(), i64::MAX);

    for t in [i64::MIN, -TICKS_PER_SECOND, -1, 0, 1, TICKS_PER_SECOND, i64::MAX] {
        assert_eq!(fast.snap(Tick(t)), Tick(t), "sub-tick frames include every integer tick");
        assert_eq!(fast.snap_nearest(Tick(t)), Tick(t));
    }

    let balanced = FrameRate { num: i64::MAX, den: i64::MAX };
    assert_eq!(balanced.frame_duration(), Tick(TICKS_PER_SECOND));
    assert_eq!(balanced.tick_of(2), Tick(2 * TICKS_PER_SECOND));
    assert_eq!(balanced.frame_at(Tick(2 * TICKS_PER_SECOND)), 2);
    assert_eq!(balanced.timecode_base(), 1);
    assert_eq!(FrameRate::new(i64::MIN, i64::MIN), FrameRate { num: -1, den: -1 });
    assert_eq!(FrameRate::new(i64::MIN, 0), FrameRate { num: -1, den: 0 });
    assert_eq!(FrameRate::new(i64::MIN, 1), FrameRate { num: i64::MIN, den: 1 });
}

#[test]
fn scalar_time_conversions_saturate_at_storage_limits() {
    assert_eq!(Tick::from_units(i64::MAX, 1), Tick(i64::MAX));
    assert_eq!(Tick::from_units(i64::MIN, 1), Tick(i64::MIN));
    assert_eq!(Tick(i64::MAX).to_units_floor(i64::MAX), i64::MAX);
    assert_eq!(Tick(i64::MIN).to_units_floor(i64::MAX), i64::MIN);
    assert_eq!(Tick::from_rational(i64::MAX, i64::MAX, 1), Tick(i64::MAX));
    assert_eq!(Tick::from_rational(i64::MIN, i64::MAX, 1), Tick(i64::MIN));
    assert_eq!(Tick::from_rational(i64::MIN, i64::MAX, -1), Tick(i64::MAX));
    assert_eq!(Tick::from_rational(i64::MAX, i64::MAX, 0), Tick::ZERO);
    assert_eq!(Tick(i64::MAX).to_rational_floor(1, i64::MAX), i64::MAX);
    assert_eq!(Tick(i64::MIN).to_rational_round(1, i64::MAX), i64::MIN);
    assert_eq!(Tick(i64::MAX).mul_ratio(i64::MAX, 1), Tick(i64::MAX));
    assert_eq!(Tick(i64::MIN).abs(), Tick(i64::MAX));
}

#[test]
fn smpte_drop_frame_exact_counts_and_negative_boundaries() {
    let r = FrameRate::FPS_29_97;
    assert!(r.supports_drop_frame());

    // 10 minutes at 29.97: 10 * 60 * 30 - 9 * 2 = 17982 frames
    let ten_min_frames = 17982;
    let tc = format_timecode_frames(ten_min_frames, r, true);
    assert_eq!(tc, "00;10;00;00");
    assert_eq!(parse_timecode(&tc, r, true, 0), Ok(ten_min_frames));

    // 1 minute drop-frame boundary: frame 1800 is 00;01;00;02
    let one_min_frame = 1800;
    let tc_1m = format_timecode_frames(one_min_frame, r, true);
    assert_eq!(tc_1m, "00;01;00;02");
    assert_eq!(parse_timecode(&tc_1m, r, true, 0), Ok(one_min_frame));

    // Negative drop-frame boundaries
    let neg_1m = -1800;
    let neg_tc = format_timecode_frames(neg_1m, r, true);
    assert_eq!(neg_tc, "-00;01;00;02");
    assert_eq!(parse_timecode(&neg_tc, r, true, 0), Ok(neg_1m));

    let neg_10m = -17982;
    let neg_10m_tc = format_timecode_frames(neg_10m, r, true);
    assert_eq!(neg_10m_tc, "-00;10;00;00");
    assert_eq!(parse_timecode(&neg_10m_tc, r, true, 0), Ok(neg_10m));

    let neg_1h = -107892;
    let neg_1h_tc = format_timecode_frames(neg_1h, r, true);
    assert_eq!(neg_1h_tc, "-01;00;00;00");
    assert_eq!(parse_timecode(&neg_1h_tc, r, true, 0), Ok(neg_1h));
}

#[test]
fn effectcraft_ae_differential_formatting() {
    let r30 = FrameRate::FPS_30;
    let r_df = FrameRate::FPS_29_97;

    // EffectCraft / After Effects formats hours without zero-padding
    assert_eq!(format_timecode_ae(75, r30, false), "0:00:02:15");
    assert_eq!(format_timecode_ae(75, r_df, true), "0;00;02;15");

    // FilmCraft / Premiere formats hours with 2 zero-padded digits
    assert_eq!(format_timecode_frames(75, r30, false), "00:00:02:15");
    assert_eq!(format_timecode_frames(75, r_df, true), "00;00;02;15");

    // Negative frames in AE style
    assert_eq!(format_timecode_ae(-75, r30, false), "-0:00:02:15");
    assert_eq!(format_timecode_ae(-1800, r_df, true), "-0;01;00;02");

    // Multi-hour parity
    assert_eq!(format_timecode_ae(3600 * 30 * 12, r30, false), "12:00:00:00");
    assert_eq!(format_timecode_frames(3600 * 30 * 12, r30, false), "12:00:00:00");
}

#[test]
fn feet_and_frames_film_standards() {
    // 35mm: 16 frames per foot
    assert_eq!(format_feet_frames(0, 16), "0000+00");
    assert_eq!(format_feet_frames(199, 16), "0012+07");
    assert_eq!(parse_feet_frames("12+07", 16, 0), Ok(199));
    assert_eq!(parse_feet_frames("0012+07", 16, 0), Ok(199));
    assert_eq!(format_feet_frames(-199, 16), "-0012+07");
    assert_eq!(parse_feet_frames("-0012+07", 16, 0), Ok(-199));

    // 16mm: 40 frames per foot
    assert_eq!(format_feet_frames(95, 40), "0002+15");
    assert_eq!(parse_feet_frames("2+15", 40, 0), Ok(95));
}

#[test]
fn exotic_frame_rate_durations() {
    let r = FrameRate::from_f64(37.516);
    assert_eq!(r.frame_duration().0, 6_770_871_094);

    // Standard rates must remain exact
    assert_eq!(FrameRate::FPS_23_976.frame_duration().0, 10_594_584_000);
    assert_eq!(FrameRate::FPS_24.frame_duration().0, 10_584_000_000);
    assert_eq!(FrameRate::FPS_25.frame_duration().0, 10_160_640_000);
    assert_eq!(FrameRate::FPS_29_97.frame_duration().0, 8_475_667_200);
    assert_eq!(FrameRate::FPS_30.frame_duration().0, 8_467_200_000);
    assert_eq!(FrameRate::FPS_60.frame_duration().0, 4_233_600_000);
}

proptest! {
    #[test]
    fn prop_monotonic_frame_math(
        frame in -100_000i64..100_000,
        rate_idx in 0usize..FrameRate::COMMON.len(),
    ) {
        let r = FrameRate::COMMON[rate_idx];
        let t1 = r.tick_of(frame);
        let t2 = r.tick_of(frame + 1);
        prop_assert!(t1 < t2, "tick_of must be strictly increasing");
        prop_assert_eq!(r.frame_at(t1), frame);
        prop_assert_eq!(r.frame_at(t2 - Tick(1)), frame);
    }

    #[test]
    fn prop_feet_frames_roundtrip(
        frame in -50_000i64..50_000,
        per_foot in 1i64..120,
    ) {
        let s = format_feet_frames(frame, per_foot);
        let parsed = parse_feet_frames(&s, per_foot, 0);
        prop_assert_eq!(parsed, Ok(frame));
    }

    #[test]
    fn prop_drop_frame_bijection(f in 0i64..(24 * 107_892)) {
        let r = FrameRate::FPS_29_97;
        let (_, _h, m, s, fr) = frames_to_fields(f, r, true);
        // dropped labels never appear
        prop_assert!(!(s == 0 && fr < 2 && m % 10 != 0));
        let txt = format_timecode_frames(f, r, true);
        prop_assert_eq!(parse_timecode(&txt, r, true, 0).unwrap(), f);
    }

    #[test]
    fn prop_ndf_parse_roundtrip(f in -500_000i64..500_000, ri in 0usize..11) {
        let r = FrameRate::COMMON[ri];
        let txt = format_timecode_frames(f, r, false);
        prop_assert_eq!(parse_timecode(&txt, r, false, 0).unwrap(), f);
    }

    #[test]
    fn prop_tick_div_monotonicity(
        a in any::<i64>(),
        b in any::<i64>(),
        d in any::<i64>().prop_filter("non-zero", |&x| x != 0),
    ) {
        if a <= b {
            if d > 0 {
                prop_assert!((Tick(a) / d) <= (Tick(b) / d));
            } else {
                prop_assert!((Tick(a) / d) >= (Tick(b) / d));
            }
        }
    }
}
