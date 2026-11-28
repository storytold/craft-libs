//! Exact rational fractions and SMPTE timecode representations for job specifications.

use serde::{Deserialize, Serialize};

use crate::DispatchError;

/// Exact 64-bit rational number (numerator / denominator) for frame rates and aspect ratios.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Rational64 {
    pub num: i64,
    pub den: i64,
}

impl Rational64 {
    /// Standard 23.976 fps (NTSC Cinema).
    pub const FPS_23_976: Self = Self { num: 24_000, den: 1_001 };
    /// Standard 24.0 fps (True Cinema).
    pub const FPS_24: Self = Self { num: 24, den: 1 };
    /// Standard 25.0 fps (PAL).
    pub const FPS_25: Self = Self { num: 25, den: 1 };
    /// Standard 29.97 fps (NTSC Broadcast).
    pub const FPS_29_97: Self = Self { num: 30_000, den: 1_001 };
    /// Standard 30.0 fps.
    pub const FPS_30: Self = Self { num: 30, den: 1 };
    /// Standard 50.0 fps (PAL High-Rate).
    pub const FPS_50: Self = Self { num: 50, den: 1 };
    /// Standard 59.94 fps (NTSC High-Rate).
    pub const FPS_59_94: Self = Self { num: 60_000, den: 1_001 };
    /// Standard 60.0 fps (Web / Gaming).
    pub const FPS_60: Self = Self { num: 60, den: 1 };

    /// Creates a new rational fraction, reducing it to lowest terms.
    pub fn new(num: i64, den: i64) -> Result<Self, DispatchError> {
        if den == 0 {
            return Err(DispatchError::InvalidJobSpec("rational denominator cannot be zero".into()));
        }

        // Reduce on unsigned magnitudes so `i64::MIN` cannot overflow.
        let g = gcd(num.unsigned_abs(), den.unsigned_abs());
        let negative = (num < 0) != (den < 0);
        let overflow = || DispatchError::InvalidJobSpec("overflow reducing rational fraction".into());
        let magnitude_num = i128::from(num.unsigned_abs() / g);
        let signed_num = if negative { -magnitude_num } else { magnitude_num };
        let reduced_num = i64::try_from(signed_num).map_err(|_| overflow())?;
        let reduced_den = i64::try_from(den.unsigned_abs() / g).map_err(|_| overflow())?;

        Ok(Self { num: reduced_num, den: reduced_den })
    }

    /// Checks that the value is a usable rate: a positive numerator over a positive denominator.
    ///
    /// `Deserialize` bypasses [`Rational64::new`], so job validation calls this on decoded rates.
    pub fn validate_positive(&self, field: &str) -> Result<(), DispatchError> {
        if self.num <= 0 || self.den <= 0 {
            return Err(DispatchError::InvalidJobSpec(format!(
                "{field} must have a positive numerator and denominator, got {}/{}",
                self.num, self.den
            )));
        }
        Ok(())
    }

    /// Converts to floating-point representation.
    pub fn as_f64(&self) -> f64 {
        if self.den == 0 {
            return 0.0;
        }
        self.num as f64 / self.den as f64
    }

    /// Inverts the rational fraction (swaps numerator and denominator).
    pub fn invert(&self) -> Result<Self, DispatchError> {
        Self::new(self.den, self.num)
    }
}

fn gcd(mut a: u64, mut b: u64) -> u64 {
    while b != 0 {
        let t = b;
        b = a % b;
        a = t;
    }
    if a == 0 { 1 } else { a }
}

/// SMPTE Timecode (Hours:Minutes:Seconds:Frames) with drop-frame support.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MediaTimecode {
    pub hours: u8,
    pub minutes: u8,
    pub seconds: u8,
    pub frames: u8,
    pub drop_frame: bool,
    pub rate: Rational64,
}

impl MediaTimecode {
    /// Creates a timecode from hours, minutes, seconds, frames, and frame rate.
    pub fn new(hours: u8, minutes: u8, seconds: u8, frames: u8, drop_frame: bool, rate: Rational64) -> Self {
        Self { hours, minutes, seconds, frames, drop_frame, rate }
    }

    /// Standard timecode start at 01:00:00:00 (broadcast hour 1).
    pub fn start_at_hour_one(rate: Rational64) -> Self {
        Self::new(1, 0, 0, 0, false, rate)
    }

    /// Zero timecode (00:00:00:00).
    pub fn zero(rate: Rational64) -> Self {
        Self::new(0, 0, 0, 0, false, rate)
    }

    /// Calculates the total continuous frame index from this timecode.
    pub fn to_frame_index(&self) -> i64 {
        let nominal_fps = ((self.rate.num as f64) / (self.rate.den as f64)).round() as i64;
        let nominal_fps = nominal_fps.max(1);

        let total_seconds =
            (self.hours as i64).saturating_mul(3600).saturating_add((self.minutes as i64).saturating_mul(60)).saturating_add(self.seconds as i64);

        let raw_frames = total_seconds.saturating_mul(nominal_fps).saturating_add(self.frames as i64);

        if self.drop_frame && (nominal_fps == 30 || nominal_fps == 60) {
            let drop_count = if nominal_fps == 30 { 2 } else { 4 };
            let total_minutes = (self.hours as i64).saturating_mul(60).saturating_add(self.minutes as i64);
            let tens_of_minutes = total_minutes / 10;
            let dropped = (total_minutes.saturating_sub(tens_of_minutes)).saturating_mul(drop_count);
            raw_frames.saturating_sub(dropped)
        } else {
            raw_frames
        }
    }

    /// Formats the timecode as an industry-standard SMPTE string.
    /// Uses `:` for non-drop-frame and `;` before frames for drop-frame (e.g. `01:00:00;00`).
    pub fn to_smpte_string(&self) -> String {
        let sep = if self.drop_frame { ';' } else { ':' };
        format!("{:02}:{:02}:{:02}{}{:02}", self.hours, self.minutes, self.seconds, sep, self.frames)
    }

    /// Parses an SMPTE string like `"01:00:00:00"` or `"01:00:00;00"`.
    pub fn parse_smpte(s: &str, rate: Rational64) -> Result<Self, DispatchError> {
        let cleaned = s.trim();
        if cleaned.len() != 11 {
            return Err(DispatchError::InvalidJobSpec(format!("invalid timecode string length (expected 11 chars, got {})", cleaned.len())));
        }

        let is_drop = cleaned.contains(';');
        let parts: Vec<&str> = cleaned.split([':', ';']).collect();
        if parts.len() != 4 {
            return Err(DispatchError::InvalidJobSpec(format!("invalid timecode components in `{cleaned}`")));
        }

        let parse_part = |idx: usize| -> Result<u8, DispatchError> {
            parts
                .get(idx)
                .and_then(|p| p.parse::<u8>().ok())
                .ok_or_else(|| DispatchError::InvalidJobSpec(format!("invalid integer in timecode `{cleaned}`")))
        };

        let hours = parse_part(0)?;
        let minutes = parse_part(1)?;
        let seconds = parse_part(2)?;
        let frames = parse_part(3)?;

        let timecode = Self::new(hours, minutes, seconds, frames, is_drop, rate);
        timecode.validate()?;
        Ok(timecode)
    }

    /// Checks that the label exists at this rate: components in range, drop-frame only at 29.97
    /// or 59.94 fps, and none of the frame labels drop-frame skips at the start of a minute that
    /// is not a multiple of ten.
    pub fn validate(&self) -> Result<(), DispatchError> {
        self.rate.validate_positive("timecode rate")?;
        let reduced = Rational64::new(self.rate.num, self.rate.den)?;
        let nominal = (reduced.num / reduced.den + i64::from(reduced.num % reduced.den >= reduced.den / 2 + reduced.den % 2)).max(1);
        let invalid = |what: &str| DispatchError::InvalidJobSpec(format!("invalid timecode {}: {what}", self.to_smpte_string()));
        if self.hours > 23 || self.minutes > 59 || self.seconds > 59 {
            return Err(invalid("component out of range"));
        }
        if i64::from(self.frames) >= nominal {
            return Err(invalid("frame count beyond the frame rate"));
        }
        if self.drop_frame {
            let drop = match (reduced.num, reduced.den) {
                (30_000, 1_001) => 2,
                (60_000, 1_001) => 4,
                _ => return Err(invalid("drop-frame needs 29.97 or 59.94 fps")),
            };
            if self.seconds == 0 && !self.minutes.is_multiple_of(10) && self.frames < drop {
                return Err(invalid("frame label is skipped by drop-frame counting"));
            }
        }
        Ok(())
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::{MediaTimecode, Rational64};

    #[test]
    fn test_rational_reduction() {
        let r = Rational64::new(1920, 1080).unwrap();
        assert_eq!(r.num, 16);
        assert_eq!(r.den, 9);
        assert!((r.as_f64() - 1.7777777777777777).abs() < 1e-9);
    }

    #[test]
    fn test_timecode_to_string() {
        let tc = MediaTimecode::new(1, 23, 45, 12, false, Rational64::FPS_24);
        assert_eq!(tc.to_smpte_string(), "01:23:45:12");

        let tc_drop = MediaTimecode::new(0, 10, 0, 0, true, Rational64::FPS_29_97);
        assert_eq!(tc_drop.to_smpte_string(), "00:10:00;00");
    }

    #[test]
    fn test_timecode_parsing() {
        let tc = MediaTimecode::parse_smpte("02:15:30:18", Rational64::FPS_25).unwrap();
        assert_eq!(tc.hours, 2);
        assert_eq!(tc.minutes, 15);
        assert_eq!(tc.seconds, 30);
        assert_eq!(tc.frames, 18);
        assert!(!tc.drop_frame);
    }

    #[test]
    fn test_timecode_frame_index() {
        let tc = MediaTimecode::new(0, 0, 1, 0, false, Rational64::FPS_25);
        assert_eq!(tc.to_frame_index(), 25);
    }

    #[test]
    fn extreme_rationals_reduce_without_overflow() {
        assert_eq!(Rational64::new(i64::MIN, i64::MIN).unwrap(), Rational64 { num: 1, den: 1 });
        assert_eq!(Rational64::new(i64::MIN, 2).unwrap().den, 1);
        assert!(Rational64::new(i64::MIN, -1).is_err(), "2^63 does not fit");
    }

    #[test]
    fn decoded_rates_must_be_positive() {
        let decoded: Rational64 = serde_json::from_str(r#"{"num":25,"den":0}"#).unwrap();
        assert!(decoded.validate_positive("frameRate").is_err());
        assert!(Rational64::FPS_29_97.validate_positive("frameRate").is_ok());
    }

    #[test]
    fn drop_frame_labels_that_do_not_exist_are_refused() {
        let rate = Rational64::FPS_29_97;
        assert!(MediaTimecode::parse_smpte("00:01:00;00", rate).is_err());
        assert!(MediaTimecode::parse_smpte("00:01:00;01", rate).is_err());
        assert!(MediaTimecode::parse_smpte("00:01:00;02", rate).is_ok());
        assert!(MediaTimecode::parse_smpte("00:01:00;02", Rational64 { num: 60_000, den: 2_002 }).is_ok());
        assert!(MediaTimecode::parse_smpte("00:01:00;01", Rational64 { num: 60_000, den: 2_002 }).is_err());
        assert!(MediaTimecode::parse_smpte("00:00:00;29", Rational64 { num: 9_222_000_000_000_000_000, den: 307_707_400_000_000_000 }).is_ok());
        assert!(MediaTimecode::parse_smpte("00:00:00:00", Rational64 { num: i64::MAX - 1, den: i64::MAX }).is_ok());
        assert!(MediaTimecode::parse_smpte("00:00:00:01", Rational64 { num: i64::MAX - 1, den: i64::MAX }).is_err());
        assert!(MediaTimecode::parse_smpte("00:00:00:00", Rational64 { num: 1, den: i64::MAX }).is_ok());
        assert!(MediaTimecode::parse_smpte("00:00:00:01", Rational64 { num: 1, den: i64::MAX }).is_err());
        assert!(MediaTimecode::parse_smpte("00:10:00;00", rate).is_ok());
        assert!(MediaTimecode::parse_smpte("00:01:00;02", Rational64::FPS_25).is_err(), "drop-frame needs 29.97/59.94");
        assert!(MediaTimecode::parse_smpte("00:00:00:25", Rational64::FPS_25).is_err(), "frame beyond the rate");
    }
}
