use craft_time::{
    FrameRate, TICKS_PER_SECOND, Tick, TimeDisplay, fields_to_frames, format_time, format_timecode_frames, frames_to_fields, parse_timecode_relative,
};

/// Overflowing fields carry into the next one before drop-frame counting, so `01;00;00;30` is the
/// frame `01;00;01;00` names rather than a non-drop count; non-drop overflow is counted as before.
#[test]
fn overflowing_drop_frame_fields_carry_before_counting() {
    let df = FrameRate::FPS_29_97;
    // (typed, the label it reads as)
    let cases = [("01;00;00;30", "01;00;01;00"), ("00;09;59;45", "00;10;00;15"), ("00;00;90;00", "00;01;30;00"), ("01;75;00;02", "02;15;00;02")];
    for (typed, label) in cases {
        let f = parse_timecode_relative(typed, df, true, 0).unwrap();
        assert_eq!((f, format_timecode_frames(f, df, true)), (parse_timecode_relative(label, df, true, 0).unwrap(), label.to_string()), "{typed}");
    }
    assert_eq!(parse_timecode_relative("00:00:00:90", FrameRate::FPS_25, false, 0).unwrap(), 90);
    assert_eq!(parse_timecode_relative("00:00:00:90", df, false, 0).unwrap(), 90);
}

#[test]
fn drop_frame_fields_keep_cancelling_terms_wide_at_storage_limits() {
    for rate in [FrameRate::FPS_29_97, FrameRate::FPS_59_94, FrameRate::FPS_119_88] {
        for frame in [0, 1, 999_999, i64::MAX - 1, i64::MAX] {
            let (negative, h, m, s, f) = frames_to_fields(frame, rate, true);
            assert!(!negative);
            assert_eq!(fields_to_frames(h, m, s, f, rate, true), frame, "{rate}: {frame}");
            assert_eq!(fields_to_frames(-h, -m, -s, -f, rate, true), -frame);
        }
        let (negative, h, m, s, f) = frames_to_fields(i64::MIN, rate, true);
        assert!(negative);
        assert_eq!(fields_to_frames(-h, -m, -s, -f, rate, true), i64::MIN);
    }
}

#[test]
fn numeric_limits_are_total_across_frame_conversion_and_display() {
    let values = [i64::MIN, -i64::MAX, -1, 0, 1, TICKS_PER_SECOND, i64::MAX];
    for num in values {
        for den in values {
            let r = FrameRate { num, den };
            let _ = FrameRate::new(num, den);
            let _ = r.frame_duration();
            assert!(r.timecode_base() >= 1);
            for value in values {
                let tick = Tick(value);
                let _ = r.tick_of(value);
                let _ = r.frame_at(tick);
                let _ = r.snap(tick);
                let _ = r.snap_nearest(tick);
                for display in TimeDisplay::ALL {
                    let _ = format_time(tick, r, true, display, 48_000);
                }
                let (_, h, m, s, f) = frames_to_fields(value, r, true);
                let _ = fields_to_frames(h, m, s, f, r, true);
            }
        }
    }
}

#[test]
fn rational_conversions() {
    // 90 kHz MPEG timebase
    let t = Tick::from_rational(90_000, 1, 90_000);
    assert_eq!(t.0, TICKS_PER_SECOND);
    assert_eq!(t.to_rational_floor(1, 90_000), 90_000);
    assert_eq!(t.to_rational_round(1, 90_000), 90_000);
    // 1 ms timebase (Matroska): to the nearest millisecond, halves up
    let ms = |t: Tick| t.to_rational_round(1, 1000);
    assert_eq!(ms(FrameRate::FPS_30.tick_of(1)), 33);
    assert_eq!(ms(FrameRate::FPS_30.tick_of(2)), 67);
    assert_eq!(FrameRate::FPS_30.tick_of(2).to_rational_floor(1, 1000), 66);
    assert_eq!(ms(FrameRate::FPS_29_97.tick_of(15)), 501, "500.5 ms");
    assert_eq!(ms(Tick::from_rational(-4, 1, 10_000)), 0, "-0.4 ms");
    assert_eq!(ms(Tick::from_rational(-5, 1, 10_000)), 0, "-0.5 ms");
    assert_eq!(ms(Tick::from_rational(-6, 1, 10_000)), -1, "-0.6 ms");
    // a negative timebase is nonsense, but rounds the same way
    assert_eq!(FrameRate::FPS_30.tick_of(2).to_rational_round(-1, -1000), 67);
    assert_eq!(FrameRate::FPS_29_97.tick_of(15).to_rational_round(-1, -1000), 501);
    assert_eq!(FrameRate::from_f64(29.97), FrameRate::FPS_29_97);
}

/// At 11 fps a frame is not a whole number of ticks, so the ticks from one boundary to the next
/// can read as zero frames; counting between frame indices gives one.
#[test]
fn frames_between_counts_boundaries_at_fractional_tick_rates() {
    let r = FrameRate::new(11, 1);
    for f in 0..1000 {
        for n in 1..=12 {
            let (a, b) = (r.tick_of(f), r.tick_of(f + n));
            assert_eq!(r.frames_between(a, b), n, "frames {f}..{}", f + n);
        }
    }
    assert_eq!(r.frame_at(Tick(r.tick_of(2).0 - r.tick_of(1).0)), 0, "the length alone reads one short");
    // frame indices that span the whole integer range saturate instead of overflowing
    let fast = FrameRate::new(i64::MAX, 1);
    assert_eq!(fast.frames_between(Tick(i64::MIN), Tick(i64::MAX)), i64::MAX);
    assert_eq!(fast.frames_between(Tick(i64::MAX), Tick(i64::MIN)), i64::MIN);
}
