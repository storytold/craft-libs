fn decode(value: u32) -> f32 {
    if value == 0u || value == 65535u {
        return select(0.0, 1.0, value != 0u);
    }
    // The normalized significand of q / 65535 is (q << (23 - leading_bit)) * 65536 / 65535.
    let leading = firstLeadingBit(value);
    let base = value << (23u - leading);
    let significand = base + (base + 32767u) / 65535u;
    return bitcast<f32>(((leading + 111u) << 23u) | (significand & 0x7fffffu));
}

fn rounded_read(input: f32) -> u32 {
    // Match the CPU's half-up read using integer high and low words for the exact product of the
    // f32 significand and 65535. A direct f32 multiply would round twice around the midpoint.
    let bits = bitcast<u32>(clamp(input, 0.0, 1.0));
    let exponent = bits >> 23u;
    if exponent < 110u {
        return 0u;
    }
    let shift = 150u - exponent;
    let significand = (bits & 0x7fffffu) | 0x800000u;
    let left = significand << 16u;
    let low = left - significand;
    let high = (significand >> 16u) - select(0u, 1u, left < significand);
    if shift > 32u {
        return (high + (1u << (shift - 33u))) >> (shift - 32u);
    }
    let rounded = low + (1u << (shift - 1u));
    let carry = high + select(0u, 1u, rounded < low);
    if shift == 32u {
        return carry;
    }
    return (carry << (32u - shift)) | (rounded >> shift);
}

// Quantize the output of compiled graph nodes to the 16-bit storage steps.
fn compiled_16bit(value: vec2<f32>) -> vec2<u32> {
    let fine = vec2<u32>(clamp(value, vec2(0.0), vec2(1.0)) * 1048576.0);
    // (65535 * fine + 524287) >> 20, split so no intermediate needs more than 32 bits.
    let low = vec2<i32>((fine & vec2<u32>(15u)) << vec2<u32>(16u)) + 524287 - vec2<i32>(fine);
    return vec2<u32>(vec2<i32>(fine >> vec2<u32>(4u)) + (low >> vec2<u32>(20u)));
}
