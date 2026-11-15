//! Attribute and text values as numbers and booleans. Every function trims surrounding
//! whitespace and returns `None` for anything it does not read.

/// A decimal `u32`. Signs other than `+`, fractions and out-of-range values are `None`.
pub fn to_u32(s: &str) -> Option<u32> {
    s.trim().parse().ok()
}

/// A decimal `i32`.
pub fn to_i32(s: &str) -> Option<i32> {
    s.trim().parse().ok()
}

/// A decimal `i64`.
pub fn to_i64(s: &str) -> Option<i64> {
    s.trim().parse().ok()
}

/// A finite `f64`. `NaN` and the infinities are `None`.
pub fn to_f64(s: &str) -> Option<f64> {
    s.trim().parse::<f64>().ok().filter(|v| v.is_finite())
}

/// A boolean in any spelling the office formats use: `1`, `true`, `on`, `t` and `0`, `false`,
/// `off`, `f`. Case matters, as in `xsd:boolean`.
pub fn to_bool(s: &str) -> Option<bool> {
    match s.trim() {
        "1" | "true" | "on" | "t" => Some(true),
        "0" | "false" | "off" | "f" => Some(false),
        _ => None,
    }
}

/// An integer that may be written as a decimal fraction or lie out of range: a finite fraction
/// is truncated toward zero and anything beyond ±9.0e18 is clamped.
pub fn to_i64_lossy(s: &str) -> Option<i64> {
    to_i64(s).or_else(|| to_f64(s).map(|f| f.clamp(-9.0e18, 9.0e18) as i64))
}

/// [`to_i64_lossy`], clamped to `i32`.
pub fn to_i32_lossy(s: &str) -> Option<i32> {
    to_i64_lossy(s).map(|v| v.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32)
}

/// [`to_i64_lossy`], clamped to `u32` (a negative value reads as 0).
pub fn to_u32_lossy(s: &str) -> Option<u32> {
    to_i64_lossy(s).map(|v| v.clamp(0, i64::from(u32::MAX)) as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_and_lossy_numbers() {
        assert_eq!(to_u32(" 42 "), Some(42));
        assert_eq!((to_u32("-1"), to_u32("1.5"), to_u32("4294967296"), to_u32("")), (None, None, None, None));
        assert_eq!((to_i32("-7"), to_i64("-9000000000")), (Some(-7), Some(-9_000_000_000)));
        assert_eq!((to_f64("2.5"), to_f64("NaN"), to_f64("inf"), to_f64("-inf"), to_f64("x")), (Some(2.5), None, None, None, None));
        assert_eq!(
            (to_i64_lossy("12.9"), to_i64_lossy("-12.9"), to_i64_lossy("1e30"), to_i64_lossy("NaN")),
            (Some(12), Some(-12), Some(9_000_000_000_000_000_000), None)
        );
        assert_eq!((to_i32_lossy("99999999999"), to_u32_lossy("-5"), to_u32_lossy("99999999999")), (Some(i32::MAX), Some(0), Some(u32::MAX)));
    }

    #[test]
    fn boolean_spellings() {
        for t in ["1", "true", "on", "t", " true "] {
            assert_eq!(to_bool(t), Some(true), "{t}");
        }
        for f in ["0", "false", "off", "f"] {
            assert_eq!(to_bool(f), Some(false), "{f}");
        }
        for other in ["", "TRUE", "yes", "2"] {
            assert_eq!(to_bool(other), None, "{other}");
        }
    }
}
