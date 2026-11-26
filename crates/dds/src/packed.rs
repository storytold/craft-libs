//! Four-byte packed RGB layouts. Words are little-endian, with red in the low bits.
//! Unsigned floats: <https://registry.khronos.org/DataFormat/specs/1.4/dataformat.1.4.html>, sections 10.2 and 10.3.

use crate::bptc;

pub(crate) fn rgba1010102(bytes: &[u8; 4]) -> [u8; 4] {
    let word = u32::from_le_bytes(*bytes);
    let channel = |shift: u32| (((word >> shift) & 1023) * 255 + 511) / 1023;
    [channel(0) as u8, channel(10) as u8, channel(20) as u8, (word >> 30) as u8 * 85]
}

// These unsigned formats share binary16's five-bit exponent and bias. Padding the
// six- or five-bit fraction with zeroes produces the same value in a positive half.
fn unsigned_float(bits: u16, padding: u32) -> f32 {
    bptc::half(bits << padding)
}

pub(crate) fn rgb111110(bytes: &[u8; 4]) -> [u8; 4] {
    let word = u32::from_le_bytes(*bytes);
    let channel = |shift: u32, mask: u32, padding: u32| bptc::srgb8(unsigned_float(((word >> shift) & mask) as u16, padding));
    [channel(0, 0x7ff, 4), channel(11, 0x7ff, 4), channel(22, 0x3ff, 5), 255]
}

pub(crate) fn rgb9e5(bytes: &[u8; 4]) -> [u8; 4] {
    let packed = u32::from_le_bytes(*bytes);
    let scale = 2.0f32.powi((packed >> 27) as i32 - 24);
    let channel = |shift: u32| bptc::srgb8(((packed >> shift) & 0x1ff) as f32 * scale);
    [channel(0), channel(9), channel(18), 255]
}

#[cfg(test)]
mod tests {
    use super::unsigned_float;

    #[test]
    fn unsigned_float_boundaries_are_preserved_before_display_quantization() {
        for (fraction_bits, padding, smallest, largest_subnormal, maximum) in
            [(6, 4, 0x3580_0000, 0x387c_0000, 65024.0), (5, 5, 0x3600_0000, 0x3878_0000, 64512.0)]
        {
            let fractions = 1u16 << fraction_bits;
            assert_eq!(unsigned_float(0, padding).to_bits(), 0);
            assert_eq!(unsigned_float(1, padding).to_bits(), smallest);
            assert_eq!(unsigned_float(fractions - 1, padding).to_bits(), largest_subnormal);
            assert_eq!(unsigned_float(fractions, padding).to_bits(), 0x3880_0000);
            assert_eq!(unsigned_float(31 * fractions - 1, padding), maximum);
            assert_eq!(unsigned_float(31 * fractions, padding), f32::INFINITY);
            assert!(unsigned_float(31 * fractions + 1, padding).is_nan());
            assert!(unsigned_float(32 * fractions - 1, padding).is_nan());
        }
    }
}
