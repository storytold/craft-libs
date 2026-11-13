//! Formatting and parsing time through a [`TimeFormat`].

use crate::rates::{FrameRate, SampleRate};
use crate::time::{TICKS_PER_SECOND, Tick};
use crate::timecode::{TimeDisplay, TimeFormat, format_feet_frames, format_time, parse_frames, parse_time};
use craft_ui::testing::trial;

fn df() -> TimeFormat {
    TimeFormat::timecode(FrameRate::FPS_29_97).drop_frame(true)
}

#[trial]
fn seconds_display_has_no_negative_zero() {
    let f = |t: i64| format_time(Tick(t), &TimeFormat::timecode(FrameRate::default()).drop_frame(false).display(TimeDisplay::Seconds));
    assert_eq!(f(-1), "0.000");
    assert_eq!(f(-2), "0.000");
    assert_eq!(f(-(TICKS_PER_SECOND / 2500)), "0.000");
    assert_eq!(f(-TICKS_PER_SECOND), "-1.000");
    assert_eq!(f(-TICKS_PER_SECOND / 1000), "-0.001");
}

#[trial]
fn parses_premiere_input() {
    let rate = FrameRate::FPS_25;
    let tc = TimeFormat::timecode(rate);
    let frames = |input: &str, current: i64| rate.frame_at(parse_time(input, &tc, rate.tick_of(current)).expect(input));
    assert_eq!(frames("01:02:03:04", 0), 93_079);
    assert_eq!(frames("01;02;03;04", 0), 93_079);
    assert_eq!(frames("1.2.3.4", 0), 93_079);
    assert_eq!(frames("1000", 0), 250);
    assert_eq!(frames("0:0:0:90", 0), 90);
    assert_eq!(frames("+15", 100), 115);
    assert_eq!(frames("-1.00", 100), 75);
    assert_eq!(parse_time("01;00;00;02", &df(), Tick::ZERO), Ok(FrameRate::FPS_29_97.tick_of(107_894)));

    let other = |display, input: &str| parse_time(input, &tc.display(display), Tick::ZERO).map(|t| rate.frame_at(t));
    assert_eq!(other(TimeDisplay::Frames, "250"), Ok(250));
    assert_eq!(parse_time("250", &TimeFormat { frame_offset: 1, ..tc }.display(TimeDisplay::Frames), Tick::ZERO).map(|t| rate.frame_at(t)), Ok(249));
    assert_eq!(other(TimeDisplay::Feet35, "15+10"), Ok(250));
    assert_eq!(other(TimeDisplay::Feet16, "6+10"), Ok(250));
    assert_eq!(other(TimeDisplay::Seconds, "10.5"), Ok(262));
    assert_eq!(parse_time("00:00:10:24000", &tc.display(TimeDisplay::AudioSamples), Tick::ZERO), Ok(Tick(TICKS_PER_SECOND * 21 / 2)));
    assert_eq!(SampleRate::default().hz(), 48_000);
}

#[trial]
fn feet_frames_roll_over_at_the_foot() {
    let r = FrameRate::FPS_24;
    let f = |frame: i64, d| format_time(r.tick_of(frame), &TimeFormat::timecode(r).drop_frame(false).display(d));
    for (frame, ft35, ft16) in
        [(0, "0+00", "0+00"), (15, "0+15", "0+15"), (16, "1+00", "0+16"), (39, "2+07", "0+39"), (40, "2+08", "1+00"), (41, "2+09", "1+01")]
    {
        assert_eq!(f(frame, TimeDisplay::Feet35), ft35, "35mm frame {frame}");
        assert_eq!(f(frame, TimeDisplay::Feet16), ft16, "16mm frame {frame}");
        assert_eq!(f(frame, TimeDisplay::Frames), frame.to_string());
    }
    // negative: sign in front, magnitude as usual
    assert_eq!(f(-1, TimeDisplay::Feet35), "-0+01");
    assert_eq!(f(-16, TimeDisplay::Feet35), "-1+00");
    assert_eq!(f(-41, TimeDisplay::Feet16), "-1+01");
    assert_eq!(f(-5, TimeDisplay::Frames), "-5");
    // large: a 10 hour show at 24 fps
    assert_eq!(f(864_000, TimeDisplay::Feet35), "54000+00");
    assert_eq!(f(864_039, TimeDisplay::Feet16), "21600+39");
    assert_eq!(f(864_000, TimeDisplay::Frames), "864000");
    assert_eq!(format_feet_frames(i64::MIN, 16), "-576460752303423488+00");
    assert_eq!(format_feet_frames(5, 0), "5+00", "a zero foot never divides by zero");
}

#[trial]
fn feet_frames_follow_the_frame_not_the_rate() {
    // 1 s of film is the rate's frame count whatever the rate
    for (r, frames) in [(FrameRate::FPS_23_976, 24), (FrameRate::FPS_24, 24), (FrameRate::FPS_25, 25), (FrameRate::FPS_30, 30)] {
        let one_second = Tick(TICKS_PER_SECOND);
        // 23.976 fps is 24 frames in 1.001 s: one second is frame 23 (floor)
        let want = r.frame_at(one_second);
        assert!((frames - 1..=frames).contains(&want), "{r:?}");
        assert_eq!(format_time(one_second, &TimeFormat::timecode(r).drop_frame(false).display(TimeDisplay::Frames)), want.to_string());
        assert_eq!(
            format_time(one_second, &TimeFormat::timecode(r).drop_frame(false).display(TimeDisplay::Feet35)),
            format!("{}+{:02}", want / 16, want % 16)
        );
        assert_eq!(
            format_time(one_second, &TimeFormat::timecode(r).drop_frame(false).display(TimeDisplay::Feet16)),
            format!("{}+{:02}", want / 40, want % 40)
        );
    }
    // drop-frame only changes timecode
    let r = FrameRate::FPS_29_97;
    let t = r.tick_of(1800);
    assert_eq!(
        format_time(t, &TimeFormat::timecode(r).drop_frame(true).display(TimeDisplay::Frames)),
        format_time(t, &TimeFormat::timecode(r).drop_frame(false).display(TimeDisplay::Frames))
    );
    assert_eq!(format_time(t, &TimeFormat::timecode(r).drop_frame(true).display(TimeDisplay::Feet35)), "112+08");
    assert_ne!(
        format_time(t, &TimeFormat::timecode(r).drop_frame(true).display(TimeDisplay::Timecode)),
        format_time(t, &TimeFormat::timecode(r).drop_frame(false).display(TimeDisplay::Timecode))
    );
}

#[trial]
fn feet_frames_and_frames_parse_back() {
    let r = FrameRate::FPS_24;
    let p = |s: &str, d, cur| parse_frames(s, &TimeFormat::timecode(r).display(d), cur);
    assert_eq!(p("2+08", TimeDisplay::Feet35, 0), Ok(40));
    assert_eq!(p("1+00", TimeDisplay::Feet16, 0), Ok(40));
    assert_eq!(p("0+15", TimeDisplay::Feet35, 0), Ok(15));
    assert_eq!(p("1+20", TimeDisplay::Feet35, 0), Ok(36), "the frames field rolls over");
    assert_eq!(p("2.08", TimeDisplay::Feet35, 0), Ok(40));
    assert_eq!(p("40", TimeDisplay::Feet35, 0), Ok(40), "a bare number is frames");
    assert_eq!(p("40", TimeDisplay::Frames, 0), Ok(40));
    assert_eq!(p(" 7 ", TimeDisplay::Frames, 0), Ok(7));
    assert_eq!(p("+5", TimeDisplay::Frames, 100), Ok(105));
    assert_eq!(p("-5", TimeDisplay::Feet16, 100), Ok(95));
    assert_eq!(p("+1+02", TimeDisplay::Feet35, 10), Ok(28));
    assert!(p("", TimeDisplay::Frames, 0).is_err());
    assert!(p("1+2", TimeDisplay::Frames, 0).is_err(), "Frames has no feet");
    assert!(p("1+2+3", TimeDisplay::Feet35, 0).is_err());
    assert!(p("a+1", TimeDisplay::Feet35, 0).is_err());
    assert!(p("99999999999999999999", TimeDisplay::Frames, 0).is_err());
    assert_eq!(p("999999999999999999+15", TimeDisplay::Feet35, 0), Ok(i64::MAX), "saturates");
    // timecode displays still parse as timecode
    assert_eq!(p("00:00:10:00", TimeDisplay::Timecode, 0), Ok(240));
    // formatting and parsing agree for every format, around the foot boundaries
    for d in [TimeDisplay::Feet35, TimeDisplay::Feet16, TimeDisplay::Frames] {
        for frame in [0, 1, 15, 16, 17, 39, 40, 41, 1000, 864_000] {
            let text = format_time(r.tick_of(frame), &TimeFormat::timecode(r).drop_frame(false).display(d));
            assert_eq!(p(&text, d, 0), Ok(frame), "{d:?} {text}");
        }
    }
}
