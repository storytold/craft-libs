//! Time as text: SMPTE timecode (drop and non-drop), frames, feet + frames, audio samples and
//! seconds, formatted and parsed the way Premiere's timecode fields do.
//!
//! Timecode is only a display of [`Tick`]s. The words around the values (the display names in a
//! menu, the error notices) are the UI's.

use crate::rates::{FrameRate, SampleRate};
use crate::time::{TICKS_PER_SECOND, Tick};

pub use craft_time::{TimeDisplay, format_timecode_frames};

/// Everything a time display needs: the rate, drop-frame, the display, the sample rate (for
/// [`TimeDisplay::AudioSamples`]) and the first frame's number.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct TimeFormat {
    /// The frame rate the display counts in.
    pub rate: FrameRate,
    /// Count in drop-frame timecode; ignored for rates without drop-frame support.
    pub drop_frame: bool,
    /// How the time is shown.
    pub display: TimeDisplay,
    /// The rate for [`TimeDisplay::AudioSamples`].
    pub sample_rate: SampleRate,
    /// Added to the frame number in [`TimeDisplay::Frames`] (0 or 1 in After Effects).
    pub frame_offset: i64,
}

impl TimeFormat {
    /// Timecode at `rate`, non-drop.
    pub fn timecode(rate: FrameRate) -> Self {
        Self { rate, ..Self::default() }
    }

    /// This format with drop-frame set to `on`.
    pub fn drop_frame(mut self, on: bool) -> Self {
        self.drop_frame = on;
        self
    }

    /// This format with the given display.
    pub fn display(mut self, display: TimeDisplay) -> Self {
        self.display = display;
        self
    }
}

/// Why typed time was refused. The UI shows it as a notice through its own words.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TimecodeError {
    /// Nothing typed.
    Empty,
    /// Not a time in the field's display (`"1:x"`); carries the input.
    NotATime(String),
}

impl std::fmt::Display for TimecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TimecodeError::Empty => f.write_str("empty timecode"),
            TimecodeError::NotATime(s) => write!(f, "not a timecode: `{s}`"),
        }
    }
}

impl std::error::Error for TimecodeError {}

/// `t` as text in `format`: timecode `00:00:10:00` (as [`format_timecode_frames`]); frames `250`
/// (plus the offset); feet + frames `15+10` (as [`format_feet_frames`]); samples `00:00:10:00000`
/// (five digits of samples past the second); seconds `10.000` (`-0.000` shown as `0.000`).
// filmcraft: crates/time/src/lib.rs:462 `format_time`.
pub fn format_time(t: Tick, format: &TimeFormat) -> String {
    let rate = format.rate;
    match format.display {
        TimeDisplay::Timecode => format_timecode_frames(rate.frame_at(t), rate, format.drop_frame),
        TimeDisplay::Frames => rate.frame_at(t).saturating_add(format.frame_offset).to_string(),
        TimeDisplay::Feet35 | TimeDisplay::Feet16 => format_feet_frames(rate.frame_at(t), format.display.frames_per_foot().unwrap_or(16)),
        TimeDisplay::AudioSamples | TimeDisplay::Seconds => {
            craft_time::format_time(t, rate, format.drop_frame, format.display, i64::from(format.sample_rate.hz()))
        }
    }
}

/// A frame count as film feet + frames, `ft+ff` (`2+08`; `-2+08` before zero). A
/// `frames_per_foot` below 1 counts as 1, so a bad value never divides by zero.
// filmcraft: crates/time/src/lib.rs:404.
pub fn format_feet_frames(frame: i64, frames_per_foot: i64) -> String {
    let per_foot = frames_per_foot.max(1).unsigned_abs();
    let a = frame.unsigned_abs();
    format!("{}{}+{:02}", if frame < 0 { "-" } else { "" }, a / per_foot, a % per_foot)
}

/// Typed time in `format` as a tick, the way Premiere's fields read it:
/// - `01:02:03:04`, `01;02;03;04`, `1.2.3.4` (any of `:;.,` separates fields);
/// - bare digits are right-aligned as `HHMMSSFF` (`1000` is 00:00:10:00);
/// - a leading `+` or `-` is relative to `current` (`+15` is 15 frames later, `-1.00` one
///   second earlier);
/// - in [`TimeDisplay::Frames`] a bare number is a frame number (less the offset); in
///   [`TimeDisplay::Seconds`] a decimal number of seconds; feet + frames reads `ft+ff` (any of
///   `+:;.,` separates; frames past the foot roll over, `1+20` at 35 mm is 36 frames; a leading
///   sign counts from `current` as in the other displays) and a bare number as frames.
///
/// The result is the first tick of the frame; [`TimeDisplay::AudioSamples`] gives the sample's
/// tick.
// soundcraft: crates/time/src/format.rs:90 `parse_position` (samples and seconds).
pub fn parse_time(input: &str, format: &TimeFormat, current: Tick) -> Result<Tick, TimecodeError> {
    let rate = format.rate;
    match format.display {
        TimeDisplay::Timecode | TimeDisplay::Frames | TimeDisplay::Feet16 | TimeDisplay::Feet35 => {
            parse_frames(input, format, rate.frame_at(current)).map(|frame| rate.tick_of(frame))
        }
        TimeDisplay::AudioSamples => {
            let (relative, body) = split_sign(input)?;
            let bad = || TimecodeError::NotATime(input.to_string());
            let parts: Vec<i64> = body.split([':', ';', '.', ',']).map(parse_digits).collect::<Option<_>>().ok_or_else(bad)?;
            let [h, m, s, samples] = parts[..] else { return Err(bad()) };
            let seconds = i128::from(h) * 3600 + i128::from(m) * 60 + i128::from(s);
            let ticks = seconds
                .saturating_mul(i128::from(TICKS_PER_SECOND))
                .saturating_add(i128::from(Tick::from_units(samples, i64::from(format.sample_rate.hz())).0));
            let t = Tick(clamp_i64(ticks));
            Ok(match relative {
                Some(sign) => current + if sign < 0 { -t } else { t },
                None => t,
            })
        }
        TimeDisplay::Seconds => {
            let (relative, body) = split_sign(input)?;
            let bad = || TimecodeError::NotATime(input.to_string());
            let value: f64 = body.trim().parse().map_err(|_| bad())?;
            if !value.is_finite() || body.trim().starts_with(['+', '-']) {
                return Err(bad());
            }
            let t = Tick::from_seconds_f64(value);
            Ok(rate.snap(match relative {
                Some(sign) => current + if sign < 0 { -t } else { t },
                None => t,
            }))
        }
    }
}

/// Typed time in `format` as a frame count at `format.rate`, read as [`parse_time`] reads it, with
/// `current` the frame a relative entry counts from. Timecode, frames and feet + frames give the
/// count as typed; samples and seconds give the frame holding the time.
// filmcraft: crates/time/src/lib.rs:499 `parse_timecode`, `:575` `parse_time_display`.
pub fn parse_frames(input: &str, format: &TimeFormat, current: i64) -> Result<i64, TimecodeError> {
    let rate = format.rate;
    let (relative, _) = split_sign(input)?;
    match format.display {
        TimeDisplay::Timecode | TimeDisplay::Frames | TimeDisplay::Feet16 | TimeDisplay::Feet35 => {
            let frames = craft_time::parse_time_display_relative(input, rate, format.drop_frame, format.display, current)
                .map_err(|_| TimecodeError::NotATime(input.to_string()))?;
            Ok(match (format.display, relative) {
                (TimeDisplay::Frames, None) => frames.saturating_sub(format.frame_offset),
                _ => frames,
            })
        }
        TimeDisplay::AudioSamples | TimeDisplay::Seconds => parse_time(input, format, rate.tick_of(current)).map(|t| rate.frame_at(t)),
    }
}

fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

/// The trimmed input as its leading sign (`+` is 1, `-` is -1: relative to the current time) and
/// the rest.
fn split_sign(input: &str) -> Result<(Option<i64>, &str), TimecodeError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(TimecodeError::Empty);
    }
    Ok(if let Some(rest) = text.strip_prefix('+') {
        (Some(1), rest)
    } else if let Some(rest) = text.strip_prefix('-') {
        (Some(-1), rest)
    } else {
        (None, text)
    })
}

/// A string of ASCII digits as a number; `None` when empty, not all digits or too large.
fn parse_digits(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}
