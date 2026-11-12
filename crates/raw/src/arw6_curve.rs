//! The ARW6 companding curve: 12-bit tile code to output value in 2 × 14-bit DN (black 1024, white 32766).
//!
//! Piecewise linear through knots (c_k, 128 k): c_k = 128 k for k <= 11, else 356.89 + 655.646 ln(k - 6); each code's
//! value is the interpolation rounded half up. Our own measurement from the CC0 lossless / ARW6 pairs of the ILCE-7RM6,
//! not stored in the file; see docs/arw6-compression.md (*Output range*).

use std::sync::OnceLock;

/// Code of knot `k`, whose output value is 128 k.
fn knot(k: u32) -> f64 {
    if k <= 11 { f64::from(128 * k) } else { 356.89 + 655.646 * f64::from(k - 6).ln() }
}

fn table() -> &'static [u16; 4096] {
    static TABLE: OnceLock<[u16; 4096]> = OnceLock::new();
    TABLE.get_or_init(|| {
        let mut t = [0u16; 4096];
        let mut k = 0u32;
        for (code, v) in t.iter_mut().enumerate() {
            let c = code as f64;
            while knot(k + 1) < c {
                k += 1;
            }
            let (c0, c1, u0) = (knot(k), knot(k + 1), f64::from(128 * k));
            *v = (u0 + 128.0 * (c - c0) / (c1 - c0) + 0.5).floor() as u16;
        }
        t
    })
}

/// Companding curve: 12-bit code (clamped to 0..=4095) to output value.
pub(crate) fn curve(code12: i32) -> u16 {
    table().get(code12.clamp(0, 4095) as usize).copied().unwrap_or(u16::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Pins the whole generated table (sum and position-weighted sum of all 4096 values, from the derivation's own
    /// table), so a platform whose `ln` rounds differently fails here.
    #[test]
    fn table_is_pinned() {
        let t = table();
        let sum: u64 = t.iter().map(|&v| u64::from(v)).sum();
        let weighted: u64 = t.iter().enumerate().map(|(i, &v)| (i as u64 + 1) * u64::from(v)).sum();
        assert_eq!((sum, weighted), (27776336, 92792980288));
        for (c, v) in [(0, 0), (1408, 1408), (1536, 1542), (2048, 2457), (3000, 7979), (3500, 16227), (3977, 32767), (4095, 39077)] {
            assert_eq!(curve(c), v, "code {c}");
        }
        assert!((0..=1408).all(|c| curve(c) == c as u16), "identity through 1408");
        assert!(t.windows(2).all(|w| w[0] < w[1]), "strictly increasing");
        assert_eq!((curve(-5), curve(5000)), (0, 39077));
    }
}
