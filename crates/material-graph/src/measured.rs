// SPDX-License-Identifier: MIT OR Apache-2.0
//! Constants of the compiled format's engine that more than one module uses, each measured against the reference app.
//! Every one names the measurement it comes from; the rule tables in
//! `tests/rules/` hold the rules built on them as plain numbers.
//!
//! Constants used in one place only are named where they are used, with their own pointers.

/// Steps of a 16-bit value: a node holding 16 bits, a rounded read, a pyramid level and an export hold `k / 65535`.
/// Measurement "Random numbers: solved" (export `round(x * 65535)`) and "One-effect projects" › "What nodes hold and how
/// they read" (16-bit nodes, reads rounded half up to a 16-bit step).
pub const STEPS_16: f64 = 65535.0;

/// Steps of an 8-bit value (`k / 255`). Measurement "Node formats" (8-bit conversion).
pub const STEPS_8: u32 = 255;

/// A bilinear read takes its weights in steps of 1/256, rounded. Measurement "Transforms" ("Bilinear reads take weights in
/// steps of 1/256") and "What nodes hold and how they read" (`(a (256 - w) + b w + 128) >> 8`); the directional blur's
/// shear too: "Directional blur, exact form" ("Lerp weights in steps of 1/256").
pub const WEIGHT_STEPS: f64 = 256.0;

/// Intensities and distances of compiled nodes count in pixels of a 256-pixel image: a node `W` wide moves, blurs or
/// reaches `intensity W / 256` pixels. Measurement "Operations identified with it" (directional and gradient warp,
/// `intensity/256`), "Operations identified, by fitting" (distance, normal, emboss: `W / 256`) and "Directional blur,
/// exact form" (`rx = intensity W/256 aspect_x`).
pub const INTENSITY_SPAN: f32 = 256.0;

/// A node holding 8 bits: `fine = round(v 65535) >> 4` (16 bits cut to 12), then `(255 fine + 2047) >> 12`, a tie going
/// down. Measurement "Node formats" (`fine = floor(v 4096); out = (255 fine + 2047) >> 12`, 0.5 gives 127) and "Fx-map
/// frames, draw order and small rules" › "Images" (8-bit nodes: to 16 bits to the nearest, cut to 12 bits, then to 8).
pub const CUT_16_TO_12: u32 = 4;
pub const FINE_8: u32 = 1 << 12;
pub const FINE_8_TIE_DOWN: u32 = FINE_8 / 2 - 1;

/// A node holding 16 bits: `fine = floor(v 2^20)`, then `(65535 fine + 2^19 - 1) >> 20`, a tie going down. Measurement "What
/// nodes hold and how they read" (16 bits: `fine = floor(v 2^20); (65535 fine + 2^19 - 1) >> 20`).
pub const FINE_16_BITS: u32 = 20;
pub const FINE_16: u64 = 1 << FINE_16_BITS;
pub const FINE_16_TIE_DOWN: u64 = FINE_16 / 2 - 1;

/// What a node holds, as `ops::apply_format` takes it. 1 to 3 are this engine's own per-node output formats (8- and
/// 16-bit integers rounded to the nearest, half floats rounded to the nearest), a choice of ours and not measured. 5 to
/// 7 are the compiled format's node formats, flags bits 4-5 of the node (0x09, 0x19, 0x29), each held as measured:
/// Measurement "What nodes hold and how they read" (0x09 8 bits, 0x19 16 bits, 0x29 half float cut towards zero).
pub const FORMAT_8_BIT: u32 = 1;
pub const FORMAT_16_BIT: u32 = 2;
pub const FORMAT_HALF: u32 = 3;
pub const COMPILED_8_BIT: u32 = 5;
pub const COMPILED_16_BIT: u32 = 6;
pub const COMPILED_HALF: u32 = 7;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_named_values_are_the_numbers_they_replace() {
        assert_eq!((FINE_8, FINE_8_TIE_DOWN, CUT_16_TO_12), (4096, 2047, 4));
        assert_eq!((FINE_16, FINE_16_TIE_DOWN), (1_048_576, 524_287));
        assert_eq!((FINE_16 as f64, FINE_16_TIE_DOWN as f64), (1_048_576.0, 524_287.0));
        assert_eq!((STEPS_16, STEPS_16 as f32, STEPS_8 as f32), (65535.0, 65535.0f32, 255.0f32));
    }
}
