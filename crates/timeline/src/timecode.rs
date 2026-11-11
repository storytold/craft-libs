//! Time as text: SMPTE timecode (drop and non-drop), frames, feet + frames, audio samples and
//! seconds, formatted and parsed the way Premiere's timecode fields do.
//!
//! Timecode is only a display of [`Tick`]s. The words around the values (the display names in a
//! menu, the error notices) are the UI's.

use crate::rates::{FrameRate, SampleRate};
use crate::time::{TICKS_PER_SECOND, Tick};

/// How time is shown (Premiere: Timecode, Feet + Frames 16 mm / 35 mm, Frames, Audio Samples;
/// After Effects' frame numbers; seconds for scripts).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum TimeDisplay {
    /// `HH:MM:SS:FF`, `HH;MM;SS;FF` for drop-frame; three frame digits from 100 fps.
    #[default]
    Timecode,
    /// The frame number, counted from [`TimeFormat::frame_offset`] (After Effects' Start
    /// Numbering Frames At).
    Frames,
    /// `feet+frames` at 40 frames a foot.
    Feet16,
    /// `feet+frames` at 16 frames a foot.
    Feet35,
    /// `HH:MM:SS:sssss`, the samples past the second.
    AudioSamples,
    /// Seconds with three decimals.
    Seconds,
}

impl TimeDisplay {
    /// The order of Premiere's Time Display menu.
    pub const ALL: [TimeDisplay; 6] =
        [TimeDisplay::Timecode, TimeDisplay::Feet35, TimeDisplay::Feet16, TimeDisplay::Frames, TimeDisplay::AudioSamples, TimeDisplay::Seconds];
}

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

/// SMPTE fields of a frame count.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct TimecodeFields {
    /// Whether the time is before zero.
    pub negative: bool,
    /// Hours, from zero.
    pub hours: i64,
    /// Minutes, 0 to 59.
    pub minutes: i64,
    /// Seconds, 0 to 59.
    pub seconds: i64,
    /// Frames within the second, below the nominal frame rate.
    pub frames: i64,
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

/// The SMPTE fields of `frame` at `rate`. Drop-frame (when asked and the rate supports it) skips
/// `base / 15` labels at the start of each minute except every tenth: 2 at 29.97, 4 at 59.94.
// filmcraft: crates/time/src/lib.rs:363 `frames_to_fields` (and `df_params` above it).
pub fn frames_to_fields(frame: i64, rate: FrameRate, drop_frame: bool) -> TimecodeFields {
    let negative = frame < 0;
    let mut n = i128::from(frame).abs();
    let (drop, base) = df_params(rate);
    if drop_frame && rate.supports_drop_frame() {
        let per_min = base * 60 - drop;
        let per_10 = per_min * 10 + drop;
        let d = n / per_10;
        let m = n % per_10;
        n = n.saturating_add(drop * 9 * d);
        if m > drop {
            n = n.saturating_add(drop * ((m - drop) / per_min));
        }
    }
    TimecodeFields {
        negative,
        hours: clamp_i64(n / (base * 3600)),
        minutes: clamp_i64((n / (base * 60)) % 60),
        seconds: clamp_i64((n / base) % 60),
        frames: clamp_i64(n % base),
    }
}

/// The frame count of SMPTE fields; the inverse of [`frames_to_fields`], in i128 so nominal
/// drop-frame labels past `i64` still give a frame that fits.
// filmcraft: crates/time/src/lib.rs:386 `fields_to_frames`.
pub fn fields_to_frames(fields: TimecodeFields, rate: FrameRate, drop_frame: bool) -> i64 {
    let (drop, base) = df_params(rate);
    let seconds = i128::from(fields.hours) * 3600 + i128::from(fields.minutes) * 60 + i128::from(fields.seconds);
    let mut n = seconds.saturating_mul(base).saturating_add(i128::from(fields.frames));
    if drop_frame && rate.supports_drop_frame() {
        let total_min = i128::from(fields.hours) * 60 + i128::from(fields.minutes);
        n = n.saturating_sub(drop * (total_min - total_min / 10));
    }
    clamp_i64(n)
}

/// `t` as text in `format`: timecode `00:00:10:00` (`;` separators for drop-frame, three frame
/// digits from a 100 fps base, a leading `-` for negative times); frames `250` (plus the
/// offset); feet + frames `15+10` (16 frames a foot for 35 mm, 40 for 16 mm); samples
/// `00:00:10:00000` (five digits of samples past the second); seconds `10.000` (`-0.000` shown
/// as `0.000`).
// filmcraft: crates/time/src/lib.rs:399 `format_timecode_frames`, `:408` `format_time`.
pub fn format_time(t: Tick, format: &TimeFormat) -> String {
    let rate = format.rate;
    let frame = rate.frame_at(t);
    match format.display {
        TimeDisplay::Timecode => format_timecode_frames(frame, rate, format.drop_frame),
        TimeDisplay::Frames => frame.saturating_add(format.frame_offset).to_string(),
        TimeDisplay::Feet35 | TimeDisplay::Feet16 => {
            let per_foot = feet_divisor(format.display);
            let a = frame.unsigned_abs();
            format!("{}{}+{:02}", sign(frame < 0), a / per_foot, a % per_foot)
        }
        TimeDisplay::AudioSamples => {
            let a = t.0.unsigned_abs();
            let tps = TICKS_PER_SECOND.unsigned_abs();
            let secs = a / tps;
            let samples = Tick((a % tps) as i64).to_units_floor(i64::from(format.sample_rate.hz()));
            format!("{}{:02}:{:02}:{:02}:{samples:05}", sign(t.0 < 0), secs / 3600, (secs / 60) % 60, secs % 60)
        }
        TimeDisplay::Seconds => {
            let s = format!("{:.3}", t.seconds());
            if s == "-0.000" { "0.000".to_string() } else { s }
        }
    }
}

/// Typed time in `format` as a tick, the way Premiere's fields read it:
/// - `01:02:03:04`, `01;02;03;04`, `1.2.3.4` (any of `:;.,` separates fields);
/// - bare digits are right-aligned as `HHMMSSFF` (`1000` is 00:00:10:00);
/// - a leading `+` or `-` is relative to `current` (`+15` is 15 frames later, `-1.00` one
///   second earlier);
/// - in [`TimeDisplay::Frames`] a bare number is a frame number (less the offset); in
///   [`TimeDisplay::Seconds`] a decimal number of seconds; feet + frames reads `ft+ff`.
///
/// The result is the first tick of the frame.
// filmcraft: crates/time/src/lib.rs:450 `parse_timecode`; soundcraft: crates/time/src/format.rs:90
// `parse_position` (the other displays).
pub fn parse_time(input: &str, format: &TimeFormat, current: Tick) -> Result<Tick, TimecodeError> {
    let text = input.trim();
    if text.is_empty() {
        return Err(TimecodeError::Empty);
    }
    let bad = || TimecodeError::NotATime(input.to_string());
    let rate = format.rate;
    let (relative, body) = match text.as_bytes()[0] {
        b'+' => (Some(1), &text[1..]),
        b'-' => (Some(-1), &text[1..]),
        _ => (None, text),
    };
    let current_frame = rate.frame_at(current);
    let tick = |frame: i64| rate.tick_of(frame);
    match format.display {
        TimeDisplay::Timecode => {
            let frames = parse_timecode_frames(body, relative, format, current_frame).ok_or_else(bad)?;
            Ok(tick(frames))
        }
        TimeDisplay::Frames => {
            let n = parse_digits(body).ok_or_else(bad)?;
            Ok(tick(match relative {
                Some(sign) => current_frame.saturating_add(n.saturating_mul(sign)),
                None => n.saturating_sub(format.frame_offset),
            }))
        }
        TimeDisplay::Feet16 | TimeDisplay::Feet35 => {
            let per_foot = feet_divisor(format.display) as i64;
            let Some((feet, frames)) = body.split_once('+') else {
                // A bare number is a relative frame count: `+15`, `-3`.
                let n = parse_digits(body).ok_or_else(bad)?;
                return relative.map(|sign| tick(current_frame.saturating_add(n.saturating_mul(sign)))).ok_or_else(bad);
            };
            let total = parse_digits(feet.trim())
                .zip(parse_digits(frames.trim()))
                .map(|(ft, fr)| ft.saturating_mul(per_foot).saturating_add(fr))
                .ok_or_else(bad)?;
            Ok(tick(match relative {
                Some(1) => current_frame.saturating_add(total),
                Some(_) => total.saturating_neg(),
                None => total,
            }))
        }
        TimeDisplay::AudioSamples => {
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

/// Drop-frame parameters: labels dropped per minute and the nominal base (30 gives 2, 60 gives 4).
fn df_params(rate: FrameRate) -> (i128, i128) {
    let base = rate.timecode_base();
    (i128::from(base / 15), i128::from(base))
}

fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn sign(negative: bool) -> &'static str {
    if negative { "-" } else { "" }
}

/// Frames in a foot of film for a feet display.
fn feet_divisor(display: TimeDisplay) -> u64 {
    if display == TimeDisplay::Feet35 { 16 } else { 40 }
}

fn format_timecode_frames(frame: i64, rate: FrameRate, drop_frame: bool) -> String {
    let drop = drop_frame && rate.supports_drop_frame();
    let f = frames_to_fields(frame, rate, drop);
    let sep = if drop { ';' } else { ':' };
    let width = if rate.timecode_base() >= 100 { 3 } else { 2 };
    format!("{}{:02}{sep}{:02}{sep}{:02}{sep}{:0width$}", sign(f.negative), f.hours, f.minutes, f.seconds, f.frames)
}

/// A string of ASCII digits as a number; `None` when empty, not all digits or too large.
fn parse_digits(s: &str) -> Option<i64> {
    if s.is_empty() || !s.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse().ok()
}

/// Typed timecode as a frame count: separated fields, or bare digits read right-aligned in pairs
/// (a relative bare number is a frame count). Fields may overflow their unit (90 frames).
fn parse_timecode_frames(body: &str, relative: Option<i64>, format: &TimeFormat, current: i64) -> Option<i64> {
    let rate = format.rate;
    let parts: Vec<&str> = body.split([':', ';', '.', ',']).collect();
    let nums: Vec<i64> = if parts.len() == 1 {
        let digits = parts[0];
        if relative.is_some() {
            vec![parse_digits(digits)?]
        } else {
            if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let mut v = Vec::new();
            let mut end = digits.len();
            while end > 0 {
                let start = end.saturating_sub(2);
                v.push(digits[start..end].parse::<i64>().unwrap_or(0));
                end = start;
            }
            v.reverse();
            v
        }
    } else {
        parts.iter().map(|p| if p.is_empty() { Some(0) } else { parse_digits(p) }).collect::<Option<_>>()?
    };
    if nums.len() > 4 {
        return None;
    }
    let mut fields = [0i64; 4];
    fields[4 - nums.len()..].copy_from_slice(&nums);
    let [h, m, s, f] = fields;
    let base = rate.timecode_base();
    let frames = if nums.len() == 1 && relative.is_some() {
        f
    } else if format.drop_frame && rate.supports_drop_frame() {
        // Fields past their range carry into the next one first, then drop-frame counting applies:
        // 01;00;00;30 is the frame 01;00;01;00 names.
        let (s, f) = (s.saturating_add(f / base), f % base);
        let (m, s) = (m.saturating_add(s / 60), s % 60);
        let (h, m) = (h.saturating_add(m / 60), m % 60);
        fields_to_frames(TimecodeFields { negative: false, hours: h, minutes: m, seconds: s, frames: f }, rate, true)
    } else {
        h.saturating_mul(3600).saturating_add(m.saturating_mul(60)).saturating_add(s).saturating_mul(base).saturating_add(f)
    };
    Some(match relative {
        Some(sign) => current.saturating_add(frames.saturating_mul(sign)),
        None => frames,
    })
}
