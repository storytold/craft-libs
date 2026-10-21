#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

//! Exact media time for the Craft suite.
//!
//! All time is an integer number of [`Tick`]s at [`TICKS_PER_SECOND`] = 254 016 000 000/s. That
//! rate divides evenly into every broadcast frame duration (23.976, 24, 25, 29.97, 30, 48, 50,
//! 59.94, 60, 119.88, 120 fps) and standard audio sample rates (44.1 kHz, 48 kHz, and their
//! multiples), so edits, frame math, audio alignment, and visual effects never drift.
//!
//! Timecode (SMPTE drop/non-drop, frames, feet+frames, audio samples) is a display representation
//! of ticks. Every conversion is total: invalid, zero, or negative rates produce well-defined defaults
//! rather than division-by-zero panics.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Rem, Sub, SubAssign};

/// Clock ticks per second: 254 016 000 000.
pub const TICKS_PER_SECOND: i64 = 254_016_000_000;

/// A point or duration in time, in ticks.
/// Arithmetic saturates at storage limits; representable results remain exact.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Tick(pub i64);

impl Tick {
    pub const ZERO: Tick = Tick(0);
    pub const MAX: Tick = Tick(i64::MAX / 4);
    pub const MIN: Tick = Tick(i64::MIN / 4);

    /// Seconds to ticks, clamped to [`Tick::MIN`]..=[`Tick::MAX`].
    pub fn from_seconds_f64(s: f64) -> Tick {
        let t = (s * TICKS_PER_SECOND as f64).round();
        Tick((t as i64).clamp(Tick::MIN.0, Tick::MAX.0))
    }

    pub fn seconds(self) -> f64 {
        self.0 as f64 / TICKS_PER_SECOND as f64
    }

    /// Exact conversion from a count of `units` at `per_second` (e.g. samples at 48 000).
    pub fn from_units(units: i64, per_second: i64) -> Tick {
        Tick(clamp_i64((units as i128 * TICKS_PER_SECOND as i128).checked_div(per_second as i128).unwrap_or(0)))
    }

    /// Floor conversion to a count of units at `per_second`.
    pub fn to_units_floor(self, per_second: i64) -> i64 {
        clamp_i64((self.0 as i128 * per_second as i128).div_euclid(TICKS_PER_SECOND as i128))
    }

    /// Conversion from a rational timestamp `pts * num / den` seconds (container timebases).
    pub fn from_rational(pts: i64, num: i64, den: i64) -> Tick {
        if den == 0 {
            return Tick::ZERO;
        }
        let Some(t) = (pts as i128 * num as i128).checked_mul(TICKS_PER_SECOND as i128) else {
            return Tick(if (pts < 0) ^ (num < 0) ^ (den < 0) { i64::MIN } else { i64::MAX });
        };
        Tick(clamp_i64(t.div_euclid(den as i128)))
    }

    /// Inverse of [`Tick::from_rational`], floored to the timebase.
    pub fn to_rational_floor(self, num: i64, den: i64) -> i64 {
        clamp_i64((self.0 as i128 * den as i128).checked_div_euclid(num as i128 * TICKS_PER_SECOND as i128).unwrap_or(0))
    }

    /// Inverse of [`Tick::from_rational`], rounded to the nearest timebase unit (halves up).
    pub fn to_rational_round(self, num: i64, den: i64) -> i64 {
        let n = self.0 as i128 * den as i128;
        let unit = num as i128 * TICKS_PER_SECOND as i128;
        let (n, unit) = if unit < 0 { (-n, -unit) } else { (n, unit) };
        let (Some(q), Some(r)) = (n.checked_div_euclid(unit), n.checked_rem_euclid(unit)) else { return 0 };
        clamp_i64(if r * 2 >= unit { q + 1 } else { q })
    }

    pub fn abs(self) -> Tick {
        Tick(self.0.saturating_abs())
    }

    pub fn signum(self) -> i64 {
        self.0.signum()
    }

    pub fn min(self, o: Tick) -> Tick {
        Tick(self.0.min(o.0))
    }

    pub fn max(self, o: Tick) -> Tick {
        Tick(self.0.max(o.0))
    }

    pub fn clamp(self, lo: Tick, hi: Tick) -> Tick {
        Tick(self.0.max(lo.0).min(hi.0))
    }

    /// Scale by a rational factor (`num/den`), flooring.
    pub fn mul_ratio(self, num: i64, den: i64) -> Tick {
        Tick(clamp_i64((self.0 as i128 * num as i128).checked_div_euclid(den as i128).unwrap_or(0)))
    }

    /// Divide by a rational factor (`num/den`), flooring.
    pub fn div_ratio(self, num: i64, den: i64) -> Tick {
        self.mul_ratio(den, num)
    }

    /// Checked addition. Returns `None` if overflow occurred.
    pub fn checked_add(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_add(rhs.0).map(Tick)
    }

    /// Checked subtraction. Returns `None` if overflow occurred.
    pub fn checked_sub(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_sub(rhs.0).map(Tick)
    }

    /// Checked multiplication. Returns `None` if overflow occurred.
    pub fn checked_mul(self, rhs: i64) -> Option<Tick> {
        self.0.checked_mul(rhs).map(Tick)
    }

    /// Checked division. Returns `None` if `rhs == 0` or if overflow occurred (`Tick(i64::MIN) / -1`).
    pub fn checked_div(self, rhs: i64) -> Option<Tick> {
        self.0.checked_div(rhs).map(Tick)
    }

    /// Checked remainder. Returns `None` if `rhs.0 == 0` or if overflow occurred (`Tick(i64::MIN) % Tick(-1)`).
    pub fn checked_rem(self, rhs: Tick) -> Option<Tick> {
        self.0.checked_rem(rhs.0).map(Tick)
    }

    /// Saturating addition.
    pub fn saturating_add(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_add(rhs.0))
    }

    /// Saturating subtraction.
    pub fn saturating_sub(self, rhs: Tick) -> Tick {
        Tick(self.0.saturating_sub(rhs.0))
    }

    /// Saturating multiplication.
    pub fn saturating_mul(self, rhs: i64) -> Tick {
        Tick(self.0.saturating_mul(rhs))
    }

    /// Saturating division. Returns `None` if `rhs == 0`, and saturates on overflow (`Tick(i64::MIN) / -1 -> Tick(i64::MAX)`).
    pub fn saturating_div(self, rhs: i64) -> Option<Tick> {
        if rhs == 0 { None } else { Some(Tick(self.0.saturating_div(rhs))) }
    }
}

impl Add for Tick {
    type Output = Tick;
    fn add(self, o: Tick) -> Tick {
        Tick(self.0.saturating_add(o.0))
    }
}

impl Sub for Tick {
    type Output = Tick;
    fn sub(self, o: Tick) -> Tick {
        Tick(self.0.saturating_sub(o.0))
    }
}

impl Neg for Tick {
    type Output = Tick;
    fn neg(self) -> Tick {
        Tick(self.0.saturating_neg())
    }
}

impl AddAssign for Tick {
    fn add_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_add(o.0);
    }
}

impl SubAssign for Tick {
    fn sub_assign(&mut self, o: Tick) {
        self.0 = self.0.saturating_sub(o.0);
    }
}

impl Mul<i64> for Tick {
    type Output = Tick;
    fn mul(self, rhs: i64) -> Tick {
        Tick(self.0.saturating_mul(rhs))
    }
}

impl Div<i64> for Tick {
    type Output = Tick;
    fn div(self, rhs: i64) -> Tick {
        // Operator division policy: Div saturates on overflow (e.g. Tick(i64::MIN) / -1 -> Tick(i64::MAX))
        // using self.0.saturating_div(rhs) to preserve monotonicity across the full range.
        // Division by zero yields Tick::ZERO. For fallible division that detects zero divisors,
        // use Tick::checked_div().
        if rhs == 0 { Tick::ZERO } else { Tick(self.0.saturating_div(rhs)) }
    }
}

impl Rem<Tick> for Tick {
    type Output = Tick;
    fn rem(self, rhs: Tick) -> Tick {
        // Operator remainder policy: Rem produces exact remainder where defined,
        // and Tick::ZERO for zero divisor or Tick(i64::MIN) % Tick(-1) (which is mathematically 0).
        // For fallible remainder, use Tick::checked_rem().
        if rhs.0 == 0 || (self.0 == i64::MIN && rhs.0 == -1) { Tick::ZERO } else { Tick(self.0 % rhs.0) }
    }
}

impl fmt::Display for Tick {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} ticks", self.0)
    }
}

/// A half-open time range `[start, start + duration)`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: Tick,
    pub duration: Tick,
}

impl TimeRange {
    pub fn new(start: Tick, duration: Tick) -> Self {
        Self { start, duration }
    }

    pub fn from_bounds(start: Tick, end: Tick) -> Self {
        Self { start, duration: end - start }
    }

    pub fn end(&self) -> Tick {
        self.start + self.duration
    }

    pub fn contains(&self, t: Tick) -> bool {
        t >= self.start && t < self.end()
    }

    pub fn overlaps(&self, o: &TimeRange) -> bool {
        self.start < o.end() && o.start < self.end()
    }

    pub fn intersect(&self, o: &TimeRange) -> Option<TimeRange> {
        let s = self.start.max(o.start);
        let e = self.end().min(o.end());
        (e > s).then(|| TimeRange::from_bounds(s, e))
    }

    pub fn is_empty(&self) -> bool {
        self.duration.0 <= 0
    }
}

/// A frame rate as an exact rational (`num / den` frames per second).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct FrameRate {
    pub num: i64,
    pub den: i64,
}

impl Default for FrameRate {
    fn default() -> Self {
        FrameRate::FPS_23_976
    }
}

impl FrameRate {
    pub const FPS_23_976: FrameRate = FrameRate { num: 24000, den: 1001 };
    pub const FPS_24: FrameRate = FrameRate { num: 24, den: 1 };
    pub const FPS_25: FrameRate = FrameRate { num: 25, den: 1 };
    pub const FPS_29_97: FrameRate = FrameRate { num: 30000, den: 1001 };
    pub const FPS_30: FrameRate = FrameRate { num: 30, den: 1 };
    pub const FPS_48: FrameRate = FrameRate { num: 48, den: 1 };
    pub const FPS_50: FrameRate = FrameRate { num: 50, den: 1 };
    pub const FPS_59_94: FrameRate = FrameRate { num: 60000, den: 1001 };
    pub const FPS_60: FrameRate = FrameRate { num: 60, den: 1 };
    pub const FPS_119_88: FrameRate = FrameRate { num: 120000, den: 1001 };
    pub const FPS_120: FrameRate = FrameRate { num: 120, den: 1 };

    /// Standard broadcast and cinematic rates.
    pub const COMMON: [FrameRate; 11] = [
        Self::FPS_23_976,
        Self::FPS_24,
        Self::FPS_25,
        Self::FPS_29_97,
        Self::FPS_30,
        Self::FPS_48,
        Self::FPS_50,
        Self::FPS_59_94,
        Self::FPS_60,
        Self::FPS_119_88,
        Self::FPS_120,
    ];

    pub fn new(num: i64, den: i64) -> Self {
        let g = i128::from(gcd(num.unsigned_abs(), den.unsigned_abs()).max(1));
        FrameRate { num: (i128::from(num) / g) as i64, den: (i128::from(den) / g) as i64 }
    }

    /// Closest standard rate for a floating-point frame rate.
    pub fn from_f64(fps: f64) -> Self {
        for r in Self::COMMON {
            if (r.as_f64() - fps).abs() < 0.005 {
                return r;
            }
        }
        FrameRate::new((fps * 1000.0).round() as i64, 1000)
    }

    /// Returns this frame rate if positive, or the default rate ([`FrameRate::FPS_23_976`]).
    ///
    /// ### Application Compatibility Note
    /// For non-positive or zero rates (e.g. `{0, 0}`), `craft-time` falls back to
    /// [`FrameRate::FPS_23_976`] (timecode base 24), providing a standard broadcast default.
    /// In contrast, EffectCraft falls back to timecode base 1.
    pub fn sane(self) -> FrameRate {
        if self.num > 0 && self.den > 0 { self } else { FrameRate::default() }
    }

    pub fn as_f64(self) -> f64 {
        self.num as f64 / self.den as f64
    }

    /// Exact duration of one frame (floored for exotic rates).
    pub fn frame_duration(self) -> Tick {
        let r = self.sane();
        Tick(clamp_i64((TICKS_PER_SECOND as i128 * r.den as i128) / r.num as i128))
    }

    /// Index of the frame containing `t` (floor).
    pub fn frame_at(self, t: Tick) -> i64 {
        clamp_i64(frame_index(self.sane(), t))
    }

    /// Start tick of frame `f` (ceiling division).
    pub fn tick_of(self, f: i64) -> Tick {
        Tick(clamp_i64(frame_boundary(self.sane(), i128::from(f))))
    }

    /// Snap `t` down to a frame boundary.
    pub fn snap(self, t: Tick) -> Tick {
        let r = self.sane();
        Tick(clamp_i64(frame_boundary(r, frame_index(r, t))))
    }

    /// Snap `t` to the nearest frame boundary.
    pub fn snap_nearest(self, t: Tick) -> Tick {
        let r = self.sane();
        let f = frame_index(r, t);
        let a = frame_boundary(r, f);
        let b = frame_boundary(r, f.saturating_add(1));
        let t = i128::from(t.0);
        Tick(clamp_i64(if t - a <= b - t { a } else { b }))
    }

    /// Timecode base (nominal frames per second): 30 for 29.97, 24 for 23.976.
    pub fn timecode_base(self) -> i64 {
        let r = self.sane();
        (r.num / r.den).saturating_add(i64::from(r.num % r.den != 0)).max(1)
    }

    pub fn is_ntsc(self) -> bool {
        self.den == 1001
    }

    pub fn supports_drop_frame(self) -> bool {
        self.is_ntsc() && self.timecode_base() % 30 == 0
    }

    pub fn label(self) -> String {
        if self.den == 1 {
            format!("{}", self.num)
        } else {
            let v = self.as_f64();
            let s = format!("{v:.3}");
            s.trim_end_matches('0').trim_end_matches('.').to_string()
        }
    }
}

impl fmt::Display for FrameRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} fps", self.label())
    }
}

fn clamp_i64(value: i128) -> i64 {
    value.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

fn frame_index(rate: FrameRate, t: Tick) -> i128 {
    (i128::from(t.0) * i128::from(rate.num)).div_euclid(i128::from(TICKS_PER_SECOND) * i128::from(rate.den))
}

fn frame_boundary(rate: FrameRate, frame: i128) -> i128 {
    let Some(n) = frame.checked_mul(i128::from(TICKS_PER_SECOND)).and_then(|n| n.checked_mul(i128::from(rate.den))) else {
        return if frame < 0 { i128::MIN } else { i128::MAX };
    };
    let d = i128::from(rate.num);
    n.div_euclid(d) + i128::from(n.rem_euclid(d) != 0)
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        (a, b) = (b, a % b);
    }
    a
}

/// How time is formatted for display.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TimeDisplay {
    #[default]
    Timecode,
    Frames,
    Feet16,
    Feet35,
    AudioSamples,
    Seconds,
}

impl TimeDisplay {
    pub const ALL: [TimeDisplay; 6] =
        [TimeDisplay::Timecode, TimeDisplay::Feet35, TimeDisplay::Feet16, TimeDisplay::Frames, TimeDisplay::AudioSamples, TimeDisplay::Seconds];

    pub fn label(self) -> &'static str {
        match self {
            TimeDisplay::Timecode => "Timecode",
            TimeDisplay::Frames => "Frames",
            TimeDisplay::Feet16 => "Feet + Frames 16mm",
            TimeDisplay::Feet35 => "Feet + Frames 35mm",
            TimeDisplay::AudioSamples => "Audio Samples",
            TimeDisplay::Seconds => "Seconds",
        }
    }
}

fn df_params(rate: FrameRate) -> (i64, i64) {
    let base = rate.timecode_base();
    (base / 15, base)
}

/// Convert a frame count to SMPTE fields `(negative, h, m, s, f)`.
pub fn frames_to_fields(frame: i64, rate: FrameRate, drop_frame: bool) -> (bool, i64, i64, i64, i64) {
    let neg = frame < 0;
    let mut n = i128::from(frame).abs();
    let (drop, base) = df_params(rate);
    let (drop, base) = (i128::from(drop), i128::from(base));
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
    let f = n % base;
    let s = (n / base) % 60;
    let mi = (n / (base * 60)) % 60;
    let h = n / (base * 3600);
    (neg, clamp_i64(h), clamp_i64(mi), clamp_i64(s), clamp_i64(f))
}

/// Convert SMPTE fields back to a frame count.
pub fn fields_to_frames(h: i64, m: i64, s: i64, f: i64, rate: FrameRate, drop_frame: bool) -> i64 {
    let (drop, base) = df_params(rate);
    let seconds = i128::from(h) * 3600 + i128::from(m) * 60 + i128::from(s);
    let mut n = seconds.saturating_mul(i128::from(base)).saturating_add(i128::from(f));
    if drop_frame && rate.supports_drop_frame() {
        let total_min = i128::from(h) * 60 + i128::from(m);
        n = n.saturating_sub(i128::from(drop) * (total_min - total_min / 10));
    }
    clamp_i64(n)
}

/// Format a frame count as SMPTE timecode (`HH:MM:SS:FF`, or `HH;MM;SS;FF` for drop-frame).
pub fn format_timecode_frames(frame: i64, rate: FrameRate, drop_frame: bool) -> String {
    let df = drop_frame && rate.supports_drop_frame();
    let (neg, h, m, s, f) = frames_to_fields(frame, rate, df);
    let sep = if df { ';' } else { ':' };
    let fw = if rate.timecode_base() >= 100 { 3 } else { 2 };
    format!("{}{h:02}{sep}{m:02}{sep}{s:02}{sep}{f:0fw$}", if neg { "-" } else { "" })
}

/// Format a frame count the way After Effects / EffectCraft displays the current time: hours not zero-padded
/// (`0:00:02:15`, or `0;00;02;15` for drop-frame).
pub fn format_timecode_ae(frame: i64, rate: FrameRate, drop_frame: bool) -> String {
    let df = drop_frame && rate.supports_drop_frame();
    let (neg, h, m, s, f) = frames_to_fields(frame, rate, df);
    let sep = if df { ';' } else { ':' };
    let fw = if rate.timecode_base() >= 100 { 3 } else { 2 };
    format!("{}{h}{sep}{m:02}{sep}{s:02}{sep}{f:0fw$}", if neg { "-" } else { "" })
}

/// Format a tick according to a display mode.
pub fn format_time(t: Tick, rate: FrameRate, drop_frame: bool, display: TimeDisplay, sample_rate: i64) -> String {
    let frame = rate.frame_at(t);
    match display {
        TimeDisplay::Timecode => format_timecode_frames(frame, rate, drop_frame),
        TimeDisplay::Frames => format!("{frame}"),
        TimeDisplay::Feet35 | TimeDisplay::Feet16 => {
            let per_ft = if display == TimeDisplay::Feet35 { 16 } else { 40 };
            let neg = frame < 0;
            let a = frame.unsigned_abs();
            format!("{}{}+{:02}", if neg { "-" } else { "" }, a / per_ft, a % per_ft)
        }
        TimeDisplay::AudioSamples => {
            let neg = t.0 < 0;
            let abs_ticks = t.0.unsigned_abs();
            let total_secs = abs_ticks / (TICKS_PER_SECOND as u64);
            let rem_ticks = abs_ticks % (TICKS_PER_SECOND as u64);
            let rem_samples = (rem_ticks as u128 * sample_rate as u128) / (TICKS_PER_SECOND as u128);
            let h = total_secs / 3600;
            let m = (total_secs / 60) % 60;
            let s = total_secs % 60;
            format!("{}{:02}:{:02}:{:02}:{:05}", if neg { "-" } else { "" }, h, m, s, rem_samples)
        }
        TimeDisplay::Seconds => format!("{:.3}", t.seconds()),
    }
}

/// Format a frame count as Feet + Frames (film footage counting).
pub fn format_feet_frames(frame: i64, per_foot: i64) -> String {
    let pf = per_foot.max(1);
    let sign = if frame < 0 { "-" } else { "" };
    let a = frame.unsigned_abs();
    let pf = pf as u64;
    let w = if pf > 100 { 3 } else { 2 };
    format!("{sign}{:04}+{:0w$}", a / pf, a % pf)
}

/// Error from timecode parsing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParseError(pub String);

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for ParseError {}

/// Parse user-typed timecode into a frame count.
///
/// Supports standard formats (`HH:MM:SS:FF`, `HH;MM;SS;FF`), short entry (`1000` -> `00:00:10:00`),
/// and relative offsets (`+15`, `-1.00`).
///
/// ### Application Compatibility Note
/// For oversized timecodes whose frame calculations exceed 64-bit limits, `craft-time`
/// saturates to `i64::MAX` (or `i64::MIN`) to keep calculations total without errors.
/// In contrast, EffectCraft returns `Err(ParseError("timecode too large"))`.
pub fn parse_timecode(input: &str, rate: FrameRate, drop_frame: bool, current: i64) -> Result<i64, ParseError> {
    let s = input.trim();
    if s.is_empty() {
        return Err(ParseError("empty timecode".into()));
    }
    let (sign_char, body) = match s.as_bytes()[0] {
        b'+' => (Some('+'), &s[1..]),
        b'-' => (Some('-'), &s[1..]),
        _ => (None, s),
    };
    let parts: Vec<&str> = body.split([':', ';', '.', ',']).collect();
    let nums: Vec<i64> = if parts.len() == 1 {
        let digits = parts[0];
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return Err(ParseError(format!("not a timecode: `{input}`")));
        }
        if sign_char.is_some() {
            vec![digits.parse().map_err(|_| ParseError("number too large".into()))?]
        } else {
            let mut v = Vec::new();
            let b = digits.as_bytes();
            let mut end = b.len();
            while end > 0 {
                let start = end.saturating_sub(2);
                v.push(digits[start..end].parse::<i64>().unwrap_or(0));
                end = start;
            }
            v.reverse();
            v
        }
    } else {
        parts
            .iter()
            .map(|p| {
                if p.is_empty() {
                    Ok(0)
                } else {
                    if !p.bytes().all(|b| b.is_ascii_digit()) {
                        return Err(ParseError(format!("bad field `{p}`")));
                    }
                    p.parse::<i64>().map_err(|_| ParseError(format!("bad field `{p}`")))
                }
            })
            .collect::<Result<_, _>>()?
    };
    if nums.len() > 4 {
        return Err(ParseError("too many timecode fields".into()));
    }
    let mut f4 = [0i64; 4];
    let off = 4 - nums.len();
    f4[off..].copy_from_slice(&nums);
    let [h, m, sec, fr] = f4;
    let base = rate.timecode_base();
    let frames = if nums.len() == 1 && sign_char.is_some() {
        fr
    } else if drop_frame && rate.supports_drop_frame() && m < 60 && sec < 60 && fr < base {
        fields_to_frames(h, m, sec, fr, rate, true)
    } else {
        h.saturating_mul(3600).saturating_add(m.saturating_mul(60)).saturating_add(sec).saturating_mul(base).saturating_add(fr)
    };

    match sign_char {
        Some('+') => Ok(current.saturating_add(frames)),
        Some('-') => {
            // A full 4-field timecode with leading '-' represents an absolute negative timecode (e.g. preroll).
            // Single-number or partial relative offsets (e.g. -15 or -1.00) offset from current.
            if nums.len() == 4 { Ok(-frames) } else { Ok(current.saturating_sub(frames)) }
        }
        None | Some(_) => Ok(frames),
    }
}

/// Parse a Feet + Frames field into a frame count.
pub fn parse_feet_frames(input: &str, per_foot: i64, current: i64) -> Result<i64, ParseError> {
    let s = input.trim();
    let bad = || ParseError(format!("not feet+frames: `{input}`"));
    let num = |t: &str| -> Result<i64, ParseError> {
        let t = t.trim();
        if t.is_empty() {
            return Ok(0);
        }
        if !t.bytes().all(|b| b.is_ascii_digit()) {
            return Err(bad());
        }
        t.parse::<i64>().map_err(|_| ParseError("number too large".into()))
    };
    if s.is_empty() {
        return Err(ParseError("empty feet+frames".into()));
    }
    let (sign, body) = match s.as_bytes()[0] {
        b'-' => (-1, &s[1..]),
        b'+' => (1, &s[1..]),
        _ => (0, s),
    };
    match body.rfind('+') {
        Some(i) => {
            let (feet, frames) = (num(&body[..i])?, num(&body[i + 1..])?);
            let v = feet.checked_mul(per_foot.max(1)).and_then(|v| v.checked_add(frames)).ok_or_else(|| ParseError("number too large".into()))?;
            Ok(if sign < 0 { -v } else { v })
        }
        None => {
            let v = num(body)?;
            Ok(match sign {
                0 => v,
                -1 => current.saturating_sub(v),
                _ => current.saturating_add(v),
            })
        }
    }
}

/// Common audio sample rates offered in sequence settings.
pub const SAMPLE_RATES: [i64; 6] = [32_000, 44_100, 48_000, 88_200, 96_000, 192_000];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tick_conversions() {
        assert_eq!(Tick::from_seconds_f64(1.0).0, TICKS_PER_SECOND);
        assert_eq!(Tick::from_units(48000, 48000).0, TICKS_PER_SECOND);
        assert_eq!(Tick(TICKS_PER_SECOND).to_units_floor(48000), 48000);
        assert_eq!(Tick::from_rational(1, 1, 1).0, TICKS_PER_SECOND);
        assert_eq!(Tick(TICKS_PER_SECOND).to_rational_floor(1, 1), 1);
        assert_eq!(Tick(TICKS_PER_SECOND).to_rational_round(1, 1), 1);
    }

    #[test]
    fn test_hostile_rates_never_panic() {
        let zero = FrameRate { num: 0, den: 0 };
        for r in [zero, FrameRate { num: 0, den: 1 }, FrameRate { num: 25, den: 0 }, FrameRate { num: -30, den: 1 }] {
            assert_eq!(r.tick_of(10), FrameRate::default().tick_of(10));
            assert_eq!(r.frame_at(Tick(TICKS_PER_SECOND)), FrameRate::default().frame_at(Tick(TICKS_PER_SECOND)));
            assert!(r.frame_duration() > Tick::ZERO);
            assert!(r.timecode_base() >= 1);
            let _ = r.snap_nearest(Tick(12345));
            let _ = format_time(Tick(TICKS_PER_SECOND), r, true, TimeDisplay::Timecode, 48_000);
        }
    }

    #[test]
    fn test_broadcast_frame_rates() {
        for r in FrameRate::COMMON {
            assert_eq!(r.frame_at(r.tick_of(100)), 100);
            assert_eq!(r.snap(r.tick_of(50)), r.tick_of(50));
            assert_eq!(r.snap_nearest(r.tick_of(50)), r.tick_of(50));
        }
    }

    #[test]
    fn test_smpte_timecode_roundtrip() {
        for r in [FrameRate::FPS_24, FrameRate::FPS_25, FrameRate::FPS_30, FrameRate::FPS_29_97] {
            for f in [0, 1, 24, 25, 30, 100, 1800, 54000] {
                let df = r.supports_drop_frame();
                let tc = format_timecode_frames(f, r, df);
                let parsed = parse_timecode(&tc, r, df, 0);
                assert_eq!(parsed, Ok(f), "rate: {}, frame: {}, tc: {}", r, f, tc);
            }
        }
    }

    #[test]
    fn test_audio_samples_formatting_negative_ticks() {
        let r = FrameRate::FPS_24;
        let neg_1s = Tick(-TICKS_PER_SECOND);
        assert_eq!(format_time(neg_1s, r, false, TimeDisplay::AudioSamples, 48_000), "-00:00:01:00000");

        let pos_1s = Tick(TICKS_PER_SECOND);
        assert_eq!(format_time(pos_1s, r, false, TimeDisplay::AudioSamples, 48_000), "00:00:01:00000");

        let neg_complex = Tick(-3661 * TICKS_PER_SECOND);
        assert_eq!(format_time(neg_complex, r, false, TimeDisplay::AudioSamples, 48_000), "-01:01:01:00000");

        let zero = Tick::ZERO;
        assert_eq!(format_time(zero, r, false, TimeDisplay::AudioSamples, 48_000), "00:00:00:00000");
    }

    #[test]
    fn test_parse_timecode_negative_absolute_independent_of_current() {
        let r = FrameRate::FPS_29_97;
        // Negative full 4-field timecode must parse as absolute negative frames, not offset from current
        assert_eq!(parse_timecode("-00;01;00;02", r, true, 5000), Ok(-1800));
        assert_eq!(parse_timecode("-00;01;00;02", r, true, 0), Ok(-1800));

        let r24 = FrameRate::FPS_24;
        assert_eq!(parse_timecode("-01:00:00:00", r24, false, 9999), Ok(-86400));

        // Single-number relative entries must offset from current
        assert_eq!(parse_timecode("-15", r24, false, 5000), Ok(4985));
        assert_eq!(parse_timecode("+15", r24, false, 5000), Ok(5015));
    }

    #[test]
    fn test_parse_timecode_rejects_signed_inner_fields() {
        let r = FrameRate::FPS_24;
        assert!(parse_timecode("00:-30:00:00", r, false, 0).is_err());
        assert!(parse_timecode("01:+5:00:00", r, false, 0).is_err());
        assert!(parse_timecode("00:00:-01:00", r, false, 0).is_err());
    }

    #[test]
    fn test_feet_frames_roundtrip() {
        for pf in [16, 40] {
            for f in [0, 1, 15, 16, 95, 199, 1234] {
                let s = format_feet_frames(f, pf);
                let parsed = parse_feet_frames(&s, pf, 0);
                assert_eq!(parsed, Ok(f));
            }
        }
    }
}
