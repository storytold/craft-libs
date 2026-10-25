#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![forbid(unsafe_code)]

//! Exact length units for the Craft office apps.
//!
//! WordCraft, DeckCraft and GridCraft all read and write Office Open XML, which mixes several
//! length units in one document: English Metric Units (EMU) for DrawingML, twips for
//! WordprocessingML, points for print and font sizes, and CSS pixels for the web builds. Each app
//! previously carried its own copy of these conversions, in `f32` in one and `f64` in another.
//! This crate is the single implementation, so a length that round-trips in one app round-trips
//! in all of them.
//!
//! [`Length`] stores an exact signed integer count of EMU, the finest unit OOXML uses
//! (914 400 per inch, 12 700 per point, 635 per twip). Converting to a coarser unit rounds half
//! away from zero, so `0.5 pt` becomes `10 twips` and `-0.5 pt` becomes `-10 twips`. Converting a
//! floating-point value to EMU clamps to the `i64` range instead of overflowing, and maps
//! non-finite input (`NaN`, infinities) to zero, matching the behaviour the apps already had.
//!
//! Nothing in this crate panics on hostile input: no `unwrap`, no indexing, no arithmetic that can
//! overflow.
//!
//! ```
//! use craft_units::Length;
//!
//! let page = Length::from_inches(8.5);
//! assert_eq!(page.emu(), 7_772_400);
//! assert_eq!(page.points(), 612.0);
//! assert_eq!(page.twips(), 12_240);
//! ```

/// EMU per inch.
pub const EMU_PER_INCH: i64 = 914_400;
/// EMU per point (1/72 inch).
pub const EMU_PER_POINT: i64 = 12_700;
/// EMU per twip (1/20 point).
pub const EMU_PER_TWIP: i64 = 635;
/// EMU per centimetre.
pub const EMU_PER_CENTIMETRE: i64 = 360_000;
/// EMU per millimetre.
pub const EMU_PER_MILLIMETRE: i64 = 36_000;
/// EMU per CSS pixel at the CSS reference density of 96 pixels per inch.
pub const EMU_PER_CSS_PIXEL: i64 = 9_525;

/// Points per inch.
pub const POINTS_PER_INCH: i64 = 72;
/// Twips per point.
pub const TWIPS_PER_POINT: i64 = 20;
/// Twips per inch.
pub const TWIPS_PER_INCH: i64 = 1_440;
/// CSS pixels per inch at the CSS reference density.
pub const CSS_PIXELS_PER_INCH: i64 = 96;

/// A length stored as an exact count of English Metric Units.
///
/// The representation is deliberately integer: OOXML lengths are integers, so a document parsed
/// and written back unchanged keeps every length bit-for-bit. Use the [`f64`] accessors for
/// layout and rendering, and [`Length::from_emu`] when reading a value straight out of a file.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Length(i64);

impl Length {
    /// Zero length.
    pub const ZERO: Self = Self(0);

    /// Builds a length from a raw EMU count, as stored in DrawingML attributes.
    #[must_use]
    pub const fn from_emu(emu: i64) -> Self {
        Self(emu)
    }

    /// The raw EMU count, for writing a DrawingML attribute.
    #[must_use]
    pub const fn emu(self) -> i64 {
        self.0
    }

    /// Builds a length from points. Non-finite input becomes zero; out-of-range input saturates.
    #[must_use]
    pub fn from_points(points: f64) -> Self {
        Self(scaled(points, EMU_PER_POINT))
    }

    /// The length in points.
    #[must_use]
    pub fn points(self) -> f64 {
        divided(self.0, EMU_PER_POINT)
    }

    /// Builds a length from inches. Non-finite input becomes zero; out-of-range input saturates.
    #[must_use]
    pub fn from_inches(inches: f64) -> Self {
        Self(scaled(inches, EMU_PER_INCH))
    }

    /// The length in inches.
    #[must_use]
    pub fn inches(self) -> f64 {
        divided(self.0, EMU_PER_INCH)
    }

    /// Builds a length from centimetres.
    #[must_use]
    pub fn from_centimetres(centimetres: f64) -> Self {
        Self(scaled(centimetres, EMU_PER_CENTIMETRE))
    }

    /// The length in centimetres.
    #[must_use]
    pub fn centimetres(self) -> f64 {
        divided(self.0, EMU_PER_CENTIMETRE)
    }

    /// Builds a length from millimetres.
    #[must_use]
    pub fn from_millimetres(millimetres: f64) -> Self {
        Self(scaled(millimetres, EMU_PER_MILLIMETRE))
    }

    /// The length in millimetres.
    #[must_use]
    pub fn millimetres(self) -> f64 {
        divided(self.0, EMU_PER_MILLIMETRE)
    }

    /// Builds a length from CSS pixels at the CSS reference density of 96 dpi.
    #[must_use]
    pub fn from_css_pixels(pixels: f64) -> Self {
        Self(scaled(pixels, EMU_PER_CSS_PIXEL))
    }

    /// The length in CSS pixels at the CSS reference density of 96 dpi.
    #[must_use]
    pub fn css_pixels(self) -> f64 {
        divided(self.0, EMU_PER_CSS_PIXEL)
    }

    /// Builds a length from twips, saturating instead of overflowing.
    #[must_use]
    pub const fn from_twips(twips: i64) -> Self {
        Self(twips.saturating_mul(EMU_PER_TWIP))
    }

    /// The length in twips, rounded half away from zero.
    #[must_use]
    pub fn twips(self) -> i64 {
        rounded_div(self.0, EMU_PER_TWIP)
    }

    /// Adds two lengths, returning `None` on overflow.
    #[must_use]
    pub const fn checked_add(self, other: Self) -> Option<Self> {
        match self.0.checked_add(other.0) {
            Some(sum) => Some(Self(sum)),
            None => None,
        }
    }

    /// Subtracts one length from another, returning `None` on overflow.
    #[must_use]
    pub const fn checked_sub(self, other: Self) -> Option<Self> {
        match self.0.checked_sub(other.0) {
            Some(difference) => Some(Self(difference)),
            None => None,
        }
    }

    /// Adds two lengths, clamping at the `i64` range on overflow.
    #[must_use]
    pub const fn saturating_add(self, other: Self) -> Self {
        Self(self.0.saturating_add(other.0))
    }

    /// Subtracts one length from another, clamping at the `i64` range on overflow.
    #[must_use]
    pub const fn saturating_sub(self, other: Self) -> Self {
        Self(self.0.saturating_sub(other.0))
    }

    /// Parses a measurement such as `12.5pt`, `2cm`, `1in`, `96px`, `1440tw` or `914400emu`.
    ///
    /// The number may carry a leading sign and a decimal point. The unit suffix is required and is
    /// matched ignoring case. Returns `None` for an empty string, a missing or unknown unit, a
    /// number that does not parse, or a non-finite number such as `inf` or `NaN`.
    #[must_use]
    pub fn parse(text: &str) -> Option<Self> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let bytes = text.as_bytes();
        let mut unit_start = bytes.len();
        while unit_start > 0 && bytes[unit_start.saturating_sub(1)].is_ascii_alphabetic() {
            unit_start = unit_start.saturating_sub(1);
        }
        if unit_start == bytes.len() || unit_start == 0 {
            return None;
        }
        let number: f64 = text.get(..unit_start)?.trim().parse().ok()?;
        if !number.is_finite() {
            return None;
        }
        let unit = text.get(unit_start..)?;
        Some(match unit.to_ascii_lowercase().as_str() {
            "emu" => Self::from_emu(number as i64),
            "pt" | "point" | "points" => Self::from_points(number),
            "in" | "inch" | "inches" => Self::from_inches(number),
            "cm" => Self::from_centimetres(number),
            "mm" => Self::from_millimetres(number),
            "px" => Self::from_css_pixels(number),
            "tw" | "twip" | "twips" => Self::from_twips(number as i64),
            _ => return None,
        })
    }
}

/// Multiplies a finite count of whole units by the EMU size of one unit.
///
/// Non-finite input is zero, matching the apps' existing `pt_to_emu` and `to_emu` behaviour.
/// The cast from `f64` to `i64` saturates, so an enormous finite value clamps instead of wrapping.
fn scaled(value: f64, emu_per_unit: i64) -> i64 {
    if !value.is_finite() {
        return 0;
    }
    (value * emu_per_unit as f64).round() as i64
}

/// Divides an EMU count into whole units.
fn divided(emu: i64, emu_per_unit: i64) -> f64 {
    emu as f64 / emu_per_unit as f64
}

/// Divides `numerator` by a positive `denominator`, rounding half away from zero.
///
/// Computed in `i128` so `numerator * 2` cannot overflow, then clamped back into `i64`.
fn rounded_div(numerator: i64, denominator: i64) -> i64 {
    let numerator = i128::from(numerator);
    let denominator = i128::from(denominator);
    if denominator <= 0 {
        return 0;
    }
    let quotient = numerator / denominator;
    let remainder = numerator % denominator;
    let rounded =
        if remainder.unsigned_abs().saturating_mul(2) >= denominator.unsigned_abs() { quotient.saturating_add(remainder.signum()) } else { quotient };
    rounded.clamp(i128::from(i64::MIN), i128::from(i64::MAX)) as i64
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn constants_are_mutually_consistent() {
        assert_eq!(EMU_PER_POINT * POINTS_PER_INCH, EMU_PER_INCH);
        assert_eq!(EMU_PER_TWIP * TWIPS_PER_POINT, EMU_PER_POINT);
        assert_eq!(EMU_PER_TWIP * TWIPS_PER_INCH, EMU_PER_INCH);
        assert_eq!(EMU_PER_CSS_PIXEL * CSS_PIXELS_PER_INCH, EMU_PER_INCH);
        assert_eq!(EMU_PER_CENTIMETRE * 254, EMU_PER_INCH * 100);
        assert_eq!(EMU_PER_MILLIMETRE * 254, EMU_PER_INCH * 10);
    }

    #[test]
    fn letter_page_is_exact_in_every_unit() {
        let page = Length::from_inches(8.5);
        assert_eq!(page.emu(), 7_772_400);
        assert_eq!(page.points(), 612.0);
        assert_eq!(page.twips(), 12_240);
        assert_eq!(page.centimetres(), 21.59);
        assert_eq!(page.millimetres(), 215.9);
        assert!((page.css_pixels() - 816.0).abs() < f64::EPSILON);
    }

    #[test]
    fn integer_emu_survives_point_round_trip() {
        for emu in [0_i64, 1, 12_699, 12_700, 914_400, 12_192_000, 6_858_000, -5, 123_456_789] {
            let length = Length::from_emu(emu);
            assert_eq!(Length::from_points(length.points()).emu(), emu, "{emu}");
        }
    }

    #[test]
    fn non_finite_input_is_zero() {
        for value in [f64::NAN, f64::INFINITY, f64::NEG_INFINITY] {
            assert_eq!(Length::from_points(value).emu(), 0, "{value}");
            assert_eq!(Length::from_inches(value).emu(), 0, "{value}");
            assert_eq!(Length::from_centimetres(value).emu(), 0, "{value}");
            assert_eq!(Length::from_css_pixels(value).emu(), 0, "{value}");
        }
    }

    #[test]
    fn enormous_input_saturates() {
        assert_eq!(Length::from_points(1.0e300).emu(), i64::MAX);
        assert_eq!(Length::from_points(-1.0e300).emu(), i64::MIN);
        assert_eq!(Length::from_emu(i64::MAX).twips(), i64::MAX / EMU_PER_TWIP);
    }

    #[test]
    fn rounding_is_half_away_from_zero() {
        assert_eq!(Length::from_emu(317).twips(), 0);
        assert_eq!(Length::from_emu(318).twips(), 1);
        assert_eq!(Length::from_emu(-317).twips(), 0);
        assert_eq!(Length::from_emu(-318).twips(), -1);
        assert_eq!(Length::from_points(0.5).twips(), 10);
        assert_eq!(Length::from_points(-0.5).twips(), -10);
    }

    #[test]
    fn twips_saturate_instead_of_overflowing() {
        assert_eq!(Length::from_twips(i64::MAX).emu(), i64::MAX);
        assert_eq!(Length::from_twips(i64::MIN).emu(), i64::MIN);
    }

    #[test]
    fn checked_arithmetic_reports_overflow() {
        let max = Length::from_emu(i64::MAX);
        let min = Length::from_emu(i64::MIN);
        assert_eq!(max.checked_add(Length::from_emu(1)), None);
        assert_eq!(min.checked_sub(Length::from_emu(1)), None);
        assert_eq!(max.saturating_add(Length::from_emu(1)).emu(), i64::MAX);
        assert_eq!(min.saturating_sub(Length::from_emu(1)).emu(), i64::MIN);
        assert_eq!(Length::from_emu(10).checked_add(Length::from_emu(5)), Some(Length::from_emu(15)));
    }

    #[test]
    fn parse_accepts_every_supported_unit() {
        assert_eq!(Length::parse("12.5pt"), Some(Length::from_points(12.5)));
        assert_eq!(Length::parse(" 2 cm "), Some(Length::from_centimetres(2.0)));
        assert_eq!(Length::parse("1in"), Some(Length::from_inches(1.0)));
        assert_eq!(Length::parse("96px"), Some(Length::from_css_pixels(96.0)));
        assert_eq!(Length::parse("1440tw"), Some(Length::from_twips(1_440)));
        assert_eq!(Length::parse("914400emu"), Some(Length::from_emu(914_400)));
        assert_eq!(Length::parse("-3MM"), Some(Length::from_millimetres(-3.0)));
        assert_eq!(Length::parse("1e3pt"), Some(Length::from_points(1_000.0)));
    }

    #[test]
    fn parse_rejects_malformed_input() {
        for text in ["", "   ", "12", "pt", "12ptx", "NaNpt", "infpt", "12.5", "-", "1e3"] {
            assert_eq!(Length::parse(text), None, "{text:?}");
        }
    }
}
