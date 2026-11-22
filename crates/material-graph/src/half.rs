//! IEEE 754 half-precision (f16) conversion, stored as `u16` bits.

/// Exact conversion: every f16 value is representable as f32.
pub fn f16_to_f32(h: u16) -> f32 {
    let sign = ((h >> 15) as u32) << 31;
    let exp = ((h >> 10) & 0x1f) as u32;
    let man = (h & 0x3ff) as u32;
    let bits = match (exp, man) {
        (0, 0) => sign,
        (0, _) => {
            // subnormal: normalise the mantissa
            let shift = man.leading_zeros() - 21;
            let man = (man << shift) & 0x3ff;
            sign | ((113 - shift) << 23) | (man << 13)
        }
        (0x1f, 0) => sign | 0x7f80_0000,
        (0x1f, _) => sign | 0x7fc0_0000 | (man << 13),
        _ => sign | ((exp + 112) << 23) | (man << 13),
    };
    f32::from_bits(bits)
}

/// Rounds to nearest, ties to even. Values too large become ±infinity; NaN stays NaN.
pub fn f32_to_f16(f: f32) -> u16 {
    let bits = f.to_bits();
    let sign = ((bits >> 16) & 0x8000) as u16;
    let exp = ((bits >> 23) & 0xff) as i32;
    let man = bits & 0x7f_ffff;
    if exp == 0xff {
        return sign | 0x7c00 | if man != 0 { 0x200 } else { 0 };
    }
    let e = exp - 127 + 15;
    if e >= 0x1f {
        return sign | 0x7c00;
    }
    if e <= 0 {
        if e < -10 {
            return sign;
        }
        // subnormal half: shift the full mantissa (with implicit 1) into place, then round
        let m = man | 0x80_0000;
        let shift = (14 - e) as u32;
        let half = 1u32 << (shift - 1);
        let rest = m & ((1 << shift) - 1);
        let mut out = m >> shift;
        if rest > half || (rest == half && out & 1 == 1) {
            out += 1;
        }
        return sign | out as u16;
    }
    let mut out = ((e as u32) << 10) | (man >> 13);
    let rest = man & 0x1fff;
    if rest > 0x1000 || (rest == 0x1000 && out & 1 == 1) {
        out += 1; // may carry into the exponent, which is still correct (up to infinity)
    }
    sign | out as u16
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_finite_half_round_trips() {
        for h in 0..=u16::MAX {
            let f = f16_to_f32(h);
            if f.is_nan() {
                assert!(f32_to_f16(f) & 0x7c00 == 0x7c00 && f32_to_f16(f) & 0x3ff != 0);
            } else {
                assert_eq!(f32_to_f16(f), h, "half bits {h:#06x} -> {f}");
            }
        }
    }

    #[test]
    fn rounds_and_saturates() {
        assert_eq!(f32_to_f16(1.0), 0x3c00);
        assert_eq!(f32_to_f16(65520.0), 0x7c00);
        assert_eq!(f32_to_f16(-1e-9), 0x8000);
        assert_eq!(f16_to_f32(f32_to_f16(0.1)), 0.099975586);
    }
}
