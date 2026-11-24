//! The random numbers of the compiled-graph engine, worked out from images the reference app renders:
//! each generator is a 24-bit linear congruential generator; a draw is the state's top 23 bits as a fraction, times the argument.
//! All of it: measurement "Random numbers: solved, from rendered images only"; the rule table `lcg_draws.tsv`.

/// Multiplier and increment of one generator, modulo 2^24.
pub type Lcg = (u32, u32);

/// `rand` in a pixel processor or a value processor.
pub const FUNCTION: Lcg = (0x37_573B, 0xC7C1);
/// `rand` in a node's parameter expression. Each expression starts again from the node's seed.
pub const PARAMETER: Lcg = (0xF8_A3ED, 0xA0_1D31);
/// `rand` in a fx-map's expressions, by what the expression computes. Each starts again from the quadrant's state. measurement
/// "Random numbers: solved" (the first table, fx-map rows).
pub const FX_SITES: [Lcg; 9] = [
    FUNCTION,               // luminance
    (0xD7_FCD1, 0xC72D),    // branch offset
    (0x1B_D4C1, 0xC613),    // pattern offset
    (0x58_B04D, 0xC4EB),    // size
    (0x34_F3F5, 0xC3D3),    // rotation
    (0xBC_F1A6, 0xC5D7),    // variation
    (0xDC_B7DC, 0xC3BF),    // a switch's condition
    (0x90_FB9D, 0xC59F),    // an iterate's count
    (0x19_660D, 0x6E_F35F), // which image a quadrant draws
];
/// From a fx-map iterate's state to the state of each of its iterations, one step per iteration. Measurement "Random numbers:
/// solved" ("Iterate: ... a=0x8232B9, c=0x38C5").
pub const ITERATION: Lcg = (0x82_32B9, 0x38C5);
/// From a quadrant's state to its children's at depth `k + 1`, one step per child (a child left out still takes its step).
/// Measurement "Random numbers: solved" (the depth table; measured to depth 16).
pub const QUADRANT: [Lcg; 16] = [
    (0x19_CF16, 0xCD93),
    (0x54_A406, 0xCDC1),
    (0x13_AC69, 0xCDE7),
    (0xA1_EBE2, 0xCE0B),
    (0x27_BAB1, 0xCE47),
    (0xC6_90C5, 0xCE7B),
    (0xDC_FAA7, 0xCC19),
    (0x49_D83B, 0xCC41),
    (0x3B_9CA8, 0xCC61),
    (0x3F_FD4B, 0xCC9B),
    (0x47_0DA9, 0xCCE5),
    (0x36_4C22, 0xCD1F),
    (0x76_F899, 0xCD3F),
    (0x34_48BE, 0xCBDD),
    (0xAF_DC7A, 0xCAD9),
    (0xDC_7A18, 0xCD5B),
];

/// States are 24 bits. Measurement "Random numbers: solved" ("an LCG modulo 2^24").
const MASK: u32 = 0xFF_FFFF;

/// The next state of a generator.
pub fn step((a, c): Lcg, state: u32) -> u32 {
    a.wrapping_mul(state).wrapping_add(c) & MASK
}

/// The generator's output for a state, in 0..1: its top 23 bits. Measurement "Random numbers: solved" (`rand(x) = x * (s >> 1) / 2^23`).
pub fn output(state: u32) -> f64 {
    f64::from((state & MASK) >> 1) / f64::from(1u32 << 23)
}

/// The state of a pixel: Jenkins' one-at-a-time hash of the seed and the bits of the pixel's position as floats, low 24 bits.
/// A value processor is the same with position (0, 0). Measurement "Random numbers: solved" ("H is Jenkins' one-at-a-time over
/// three 32-bit words").
pub fn pixel_state(seed: u32, u: f32, v: f32) -> u32 {
    let mut h = 0u32;
    for w in [seed, u.to_bits(), v.to_bits()] {
        h = h.wrapping_add(w);
        h = h.wrapping_add(h << 10);
        h ^= h >> 6;
    }
    h = h.wrapping_add(h << 3);
    h ^= h >> 11;
    h = h.wrapping_add(h << 15);
    h & MASK
}

/// A quadrant's state to the state of its child number `q` (0..4), `depth` the child's depth (the first quadrant has 0).
pub fn child_state(state: u32, q: usize, depth: usize) -> u32 {
    let lcg = QUADRANT.get(depth.saturating_sub(1)).or(QUADRANT.last()).copied().unwrap_or(ITERATION);
    (0..=q).fold(state, |s, _| step(lcg, s))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matches_reference_draws() {
        // seed 0, 256 x 256, pixel (0, 0): first draw; state from the export's 16 bits at both offsets
        let s = step(FUNCTION, pixel_state(0, 0.5 / 256.0, 0.5 / 256.0));
        assert_eq!(s >> 1, 0x41_C501);
        assert_eq!(step(FUNCTION, s) >> 1, 0x6E_CDB9);
        // a value processor with seed 1: first and second draw
        let s = step(FUNCTION, pixel_state(1, 0.0, 0.0));
        assert_eq!(s >> 1, 0x5C_EC02);
        assert_eq!(step(FUNCTION, s) >> 1, 0x29_21F4);
        // a parameter with seed 5, two draws
        let s = step(PARAMETER, 5);
        assert_eq!(s >> 1, 0x3D_A869);
        assert_eq!(step(PARAMETER, s) >> 1, 0x57_D2CD);
        // a quadrant's children, seed 0
        assert_eq!(child_state(0, 0, 1), 0xCD93);
        assert_eq!(child_state(0, 1, 1), 0xA7_5535);
        assert_eq!(child_state(0, 3, 1), 0x57_1169);
    }
}
