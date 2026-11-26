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
