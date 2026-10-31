#![allow(clippy::needless_range_loop)]

use craft_blitter::{
    AlphaMode, BlendMode, blit_rgba8, blit_rgba8_clipped, blit_rgba8_opts, blit_rgba8_premultiplied, blit_rgba16, blit_rgba16_clipped,
    blit_rgba16_opts, blit_rgba16_premultiplied, blit_rgba32f, blit_rgba32f_clipped, blit_rgba32f_gamma, blit_rgba32f_opts,
    blit_rgba32f_premultiplied,
};

const CANARY_U8: u8 = 0xAA;
const CANARY_U16: u16 = 0xAAAA;
const CANARY_F32: [f32; 4] = [777.125, 888.25, 999.5, 1111.75];

const ALL_MODES: [BlendMode; 33] = [
    BlendMode::Normal,
    BlendMode::Dissolve,
    BlendMode::Darken,
    BlendMode::Multiply,
    BlendMode::ColorBurn,
    BlendMode::LinearBurn,
    BlendMode::DarkerColor,
    BlendMode::Lighten,
    BlendMode::Screen,
    BlendMode::ColorDodge,
    BlendMode::LinearDodge,
    BlendMode::LighterColor,
    BlendMode::Overlay,
    BlendMode::SoftLight,
    BlendMode::HardLight,
    BlendMode::VividLight,
    BlendMode::LinearLight,
    BlendMode::PinLight,
    BlendMode::HardMix,
    BlendMode::Difference,
    BlendMode::Exclusion,
    BlendMode::Subtract,
    BlendMode::Divide,
    BlendMode::Hue,
    BlendMode::Saturation,
    BlendMode::Color,
    BlendMode::Luminosity,
    BlendMode::Reflect,
    BlendMode::Glow,
    BlendMode::Negation,
    BlendMode::Xor,
    BlendMode::PaintNetColorBurn,
    BlendMode::PaintNetColorDodge,
];

// =========================================================================
// 1. MISMATCHED SLICE LENGTHS, UNALIGNED TAILS & MEMORY SAFETY CANARIES
// =========================================================================

#[test]
fn test_challenger_mismatched_slice_lengths_and_canaries_rgba8() {
    let test_lengths = [
        (0, 0),
        (0, 4),
        (4, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (2, 3),
        (3, 2),
        (4, 7),
        (7, 4),
        (5, 11),
        (11, 5),
        (8, 12),
        (12, 8),
        (15, 33),
        (33, 15),
        (16, 24),
        (24, 16),
        (63, 65),
        (65, 63),
        (100, 20),
        (20, 100),
        (4095 * 4, 17),
        (17, 4095 * 4),
        (4097 * 4, 33),
        (33, 4097 * 4),
        (8193 * 4, 5),
        (5, 8193 * 4),
    ];

    const PAD: usize = 16;

    for &(dst_len, src_len) in &test_lengths {
        for &mode in &ALL_MODES {
            // Buffer with leading and trailing padding canaries
            let mut dst_buffer = vec![CANARY_U8; dst_len + 2 * PAD];
            let src_buffer = vec![137u8; src_len + 2 * PAD];

            let dst_slice = &mut dst_buffer[PAD..PAD + dst_len];
            let src_slice = &src_buffer[PAD..PAD + src_len];

            blit_rgba8(dst_slice, src_slice, mode, 0.8, 0.7);

            let num_pixels = (dst_len / 4).min(src_len / 4);
            let blitted_bytes = num_pixels * 4;

            // 1. Verify leading padding is untouched
            for i in 0..PAD {
                assert_eq!(dst_buffer[i], CANARY_U8, "Leading canary corrupted at {i} for mode {mode:?}");
            }

            // 2. Verify unblended remainder bytes in dst are untouched
            for i in blitted_bytes..dst_len {
                assert_eq!(dst_buffer[PAD + i], CANARY_U8, "Dst remainder byte at {i} (past {blitted_bytes} blitted) corrupted for mode {mode:?}");
            }

            // 3. Verify trailing padding is untouched
            for i in PAD + dst_len..dst_buffer.len() {
                assert_eq!(dst_buffer[i], CANARY_U8, "Trailing canary corrupted at {i} for mode {mode:?}");
            }
        }
    }
}

#[test]
fn test_challenger_mismatched_slice_lengths_and_canaries_rgba16() {
    let test_lengths = [
        (0, 0),
        (0, 4),
        (4, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (2, 3),
        (3, 2),
        (4, 7),
        (7, 4),
        (5, 11),
        (11, 5),
        (8, 12),
        (12, 8),
        (15, 33),
        (33, 15),
        (16, 24),
        (24, 16),
        (63, 65),
        (65, 63),
        (100, 20),
        (20, 100),
        (4095 * 4, 17),
        (17, 4095 * 4),
        (4097 * 4, 33),
        (33, 4097 * 4),
    ];

    const PAD: usize = 16;

    for &(dst_len, src_len) in &test_lengths {
        for &mode in &ALL_MODES {
            let mut dst_buffer = vec![CANARY_U16; dst_len + 2 * PAD];
            let src_buffer = vec![33333u16; src_len + 2 * PAD];

            let dst_slice = &mut dst_buffer[PAD..PAD + dst_len];
            let src_slice = &src_buffer[PAD..PAD + src_len];

            blit_rgba16(dst_slice, src_slice, mode, 0.8, 0.7);

            let num_pixels = (dst_len / 4).min(src_len / 4);
            let blitted_words = num_pixels * 4;

            for i in 0..PAD {
                assert_eq!(dst_buffer[i], CANARY_U16, "Leading canary corrupted at {i} for mode {mode:?}");
            }
            for i in blitted_words..dst_len {
                assert_eq!(dst_buffer[PAD + i], CANARY_U16, "Dst remainder word at {i} (past {blitted_words} blitted) corrupted for mode {mode:?}");
            }
            for i in PAD + dst_len..dst_buffer.len() {
                assert_eq!(dst_buffer[i], CANARY_U16, "Trailing canary corrupted at {i} for mode {mode:?}");
            }
        }
    }
}

#[test]
fn test_challenger_mismatched_slice_lengths_and_canaries_rgba32f() {
    let test_lengths = [
        (0, 0),
        (0, 1),
        (1, 0),
        (1, 1),
        (2, 5),
        (5, 2),
        (7, 13),
        (13, 7),
        (15, 33),
        (33, 15),
        (63, 65),
        (65, 63),
        (4095, 4097),
        (4097, 4095),
        (5000, 17),
        (17, 5000),
    ];

    const PAD: usize = 16;

    for &(dst_len, src_len) in &test_lengths {
        for &mode in &ALL_MODES {
            let mut dst_buffer = vec![CANARY_F32; dst_len + 2 * PAD];
            let src_buffer = vec![[0.45f32, 0.55, 0.65, 0.85]; src_len + 2 * PAD];

            let dst_slice = &mut dst_buffer[PAD..PAD + dst_len];
            let src_slice = &src_buffer[PAD..PAD + src_len];

            blit_rgba32f(dst_slice, src_slice, mode, 0.85, 0.65, false);

            let num_pixels = dst_len.min(src_len);

            for i in 0..PAD {
                assert_eq!(dst_buffer[i], CANARY_F32, "Leading canary corrupted at {i} for mode {mode:?}");
            }
            for i in num_pixels..dst_len {
                assert_eq!(dst_buffer[PAD + i], CANARY_F32, "Dst remainder pixel at {i} (past {num_pixels} blitted) corrupted for mode {mode:?}");
            }
            for i in PAD + dst_len..dst_buffer.len() {
                assert_eq!(dst_buffer[i], CANARY_F32, "Trailing canary corrupted at {i} for mode {mode:?}");
            }
        }
    }
}

// =========================================================================
// 2. PRIME LENGTHS & SIMD REMAINDER TAIL EXACT BIT PARITY (LANES = 8)
// =========================================================================

#[test]
fn test_challenger_simd_chunks_and_tails_oracle_parity() {
    let test_pixel_counts = [
        // Small primes and non-multiples
        1, 2, 3, 5, 7, 8, 9, 11, 13, 15, 16, 17, 19, 23, 29, 31, 33, 37, 41, 47, 53, 59, 61, 63, 64, 65, 127, 128, 129,
        // Larger sizes around thresholds
        255, 256, 257, 511, 512, 513, 1023, 1024, 1025, 2047, 2048, 2049, // Around Rayon PAR_THRESHOLD = 4096
        4095, 4096, 4097, 8191, 8192, 8193,
    ];

    let separable_modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::LinearDodge,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::LinearBurn,
        BlendMode::Exclusion,
        BlendMode::Subtract,
    ];

    let opacity = 0.77f32;

    for &len in &test_pixel_counts {
        let initial_dst: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = (i % 251) as f32 / 250.0;
                [f * 0.7 + 0.1, (1.0 - f) * 0.8, (f * 0.6 + 0.3).min(1.0), 0.88]
            })
            .collect();

        let src: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = ((i * 13 + 37) % 251) as f32 / 250.0;
                [(1.0 - f) * 0.6 + 0.2, f * 0.75, (f * 0.5 + 0.1), 0.72]
            })
            .collect();

        for &mode in &separable_modes {
            // SDR pass
            {
                let mut dst = initial_dst.clone();
                blit_rgba32f(&mut dst, &src, mode, opacity, 1.0, false);

                for i in 0..len {
                    let oracle = craft_blitter::float::composite_f32(mode, initial_dst[i], src[i], opacity, false);
                    for c in 0..4 {
                        let diff = (dst[i][c] - oracle[c]).abs();
                        assert!(
                            diff < 1e-5,
                            "SIMD SDR mismatch for {mode:?} at len {len}, pixel {i}, ch {c}: got {}, want {} (diff {diff})",
                            dst[i][c],
                            oracle[c]
                        );
                    }
                }
            }

            // HDR pass
            {
                let mut dst = initial_dst.clone();
                blit_rgba32f(&mut dst, &src, mode, opacity, 1.0, true);

                for i in 0..len {
                    let oracle = craft_blitter::float::composite_f32(mode, initial_dst[i], src[i], opacity, true);
                    for c in 0..4 {
                        let diff = (dst[i][c] - oracle[c]).abs();
                        assert!(
                            diff < 1e-5,
                            "SIMD HDR mismatch for {mode:?} at len {len}, pixel {i}, ch {c}: got {}, want {} (diff {diff})",
                            dst[i][c],
                            oracle[c]
                        );
                    }
                }
            }
        }
    }
}

// =========================================================================
// 3. EXTREME FLOATING POINT VALUES (SUBNORMALS, NANS, INFS, NEGATIVES)
// =========================================================================

#[test]
fn test_challenger_extreme_floats_zero_panics_all_entrypoints() {
    let extreme_floats = [
        f32::NAN,
        -f32::NAN,
        f32::from_bits(0x7fc00001), // quiet NaN with payload
        f32::from_bits(0x7f800001), // signaling NaN pattern
        f32::from_bits(0xffc00001),
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0f32,
        -0.0f32,
        f32::MIN_POSITIVE,
        f32::MIN_POSITIVE / 2.0, // subnormal
        1e-45f32,                // min subnormal
        -1e-45f32,
        f32::from_bits(1), // raw integer 1 bit as float subnormal
        -1.0f32,
        -100.0f32,
        -1e30f32,
        f32::MIN,
        1.0f32,
        1.0001f32,
        2.5f32,
        100.0f32,
        1e30f32,
        f32::MAX,
    ];

    for &val in &extreme_floats {
        for &mode in &ALL_MODES {
            let mut dst = [[val, val, val, val]; 9]; // 9 pixels forces 1 chunk of 8 + 1 tail
            let src = [[0.5f32, 0.4, 0.3, 0.9]; 9];

            // 1. blit_rgba32f (SDR & HDR)
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, true);

            // 2. Extreme src
            let mut dst2 = [[0.5f32, 0.5, 0.5, 1.0]; 9];
            let src2 = [[val, val, val, val]; 9];
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, true);

            // 3. Extreme opacity & fill
            let mut dst3 = [[0.6f32, 0.3, 0.7, 1.0]; 9];
            let src3 = [[0.2f32, 0.8, 0.4, 0.8]; 9];
            blit_rgba32f(&mut dst3, &src3, mode, val, val, false);

            // 4. Extreme gamma
            blit_rgba32f_gamma(&mut dst3, &src3, mode, 0.9, 0.9, val, false);

            // 5. Clipped & premultiplied
            blit_rgba32f_clipped(&mut dst3, &src3, mode, 0.9, 0.9, false);
            blit_rgba32f_premultiplied(&mut dst3, &src3, val);

            // 6. Generic opts
            blit_rgba32f_opts(&mut dst3, &src3, mode, 0.8, 0.7, 1.0, true, AlphaMode::Straight, false);
            blit_rgba32f_opts(&mut dst3, &src3, mode, 0.8, 0.7, 1.0, true, AlphaMode::Premultiplied, false);
        }
    }
}

#[test]
fn test_challenger_hdr_vs_sdr_exact_preservation() {
    // 1. LinearDodge (Add): cb = 2.0, cs = 3.0 => sum = 5.0
    let mut dst_sdr = [[2.0f32, 1.5, 0.8, 1.0]; 8];
    let mut dst_hdr = [[2.0f32, 1.5, 0.8, 1.0]; 8];
    let src = [[3.0f32, 2.0, 0.5, 1.0]; 8];

    blit_rgba32f(&mut dst_sdr, &src, BlendMode::LinearDodge, 1.0, 1.0, false);
    blit_rgba32f(&mut dst_hdr, &src, BlendMode::LinearDodge, 1.0, 1.0, true);

    for i in 0..8 {
        assert_eq!(dst_sdr[i][0], 1.0, "SDR LinearDodge must clamp to 1.0 at pixel {i}");
        assert_eq!(dst_sdr[i][1], 1.0, "SDR LinearDodge must clamp to 1.0 at pixel {i}");
        assert!((dst_hdr[i][0] - 5.0).abs() < 1e-5, "HDR LinearDodge must equal 5.0, got {}", dst_hdr[i][0]);
        assert!((dst_hdr[i][1] - 3.5).abs() < 1e-5, "HDR LinearDodge must equal 3.5, got {}", dst_hdr[i][1]);
    }

    // 2. Divide: cb = 1.0, cs = 0.25 => 4.0
    let mut div_sdr = [[1.0f32, 0.5, 0.8, 1.0]; 8];
    let mut div_hdr = [[1.0f32, 0.5, 0.8, 1.0]; 8];
    let div_src = [[0.25f32, 0.1, 0.2, 1.0]; 8];

    blit_rgba32f(&mut div_sdr, &div_src, BlendMode::Divide, 1.0, 1.0, false);
    blit_rgba32f(&mut div_hdr, &div_src, BlendMode::Divide, 1.0, 1.0, true);

    for i in 0..8 {
        assert_eq!(div_sdr[i][0], 1.0, "SDR Divide must clamp to 1.0 at pixel {i}");
        assert!((div_hdr[i][0] - 4.0).abs() < 1e-5, "HDR Divide must equal 4.0, got {}", div_hdr[i][0]);
        assert!((div_hdr[i][1] - 5.0).abs() < 1e-5, "HDR Divide must equal 5.0, got {}", div_hdr[i][1]);
    }
}

// =========================================================================
// 4. EXTREME OPACITY AND FILL DYNAMICS (NEGATIVE, ZERO, EXCESS, NAN)
// =========================================================================

#[test]
fn test_challenger_zero_and_negative_opacity_strict_no_op() {
    let non_positive_opacities = [-1000.0f32, -1.0, -0.001, -0.0, 0.0];

    for &op in &non_positive_opacities {
        for &mode in &ALL_MODES {
            // RGBA8
            let mut dst8 = [42u8, 99, 150, 255, 30, 60, 90, 128];
            let orig8 = dst8;
            let src8 = [200u8, 210, 220, 255, 10, 20, 30, 255];
            blit_rgba8(&mut dst8, &src8, mode, op, 0.5);
            assert_eq!(dst8, orig8, "RGBA8 must be unmodified for op {op} in mode {mode:?}");

            // RGBA8 opts
            blit_rgba8_opts(&mut dst8, &src8, mode, op, 0.5, AlphaMode::Premultiplied, false);
            assert_eq!(dst8, orig8, "RGBA8 opts must be unmodified for op {op} in mode {mode:?}");

            // RGBA16
            let mut dst16 = [4200u16, 9900, 15000, 65535, 3000, 6000, 9000, 32768];
            let orig16 = dst16;
            let src16 = [20000u16, 21000, 22000, 65535, 1000, 2000, 3000, 65535];
            blit_rgba16(&mut dst16, &src16, mode, op, 0.5);
            assert_eq!(dst16, orig16, "RGBA16 must be unmodified for op {op} in mode {mode:?}");

            // RGBA16 opts
            blit_rgba16_opts(&mut dst16, &src16, mode, op, 0.5, AlphaMode::Premultiplied, false);
            assert_eq!(dst16, orig16, "RGBA16 opts must be unmodified for op {op} in mode {mode:?}");

            // RGBA32F
            let mut dst32 = [[0.4f32, 0.5, 0.6, 1.0], [0.1, 0.2, 0.3, 0.5]];
            let orig32 = dst32;
            let src32 = [[0.9f32, 0.8, 0.7, 1.0], [0.5, 0.6, 0.7, 0.8]];
            blit_rgba32f(&mut dst32, &src32, mode, op, 0.5, false);
            assert_eq!(dst32, orig32, "RGBA32F must be unmodified for op {op} in mode {mode:?}");

            // RGBA32F opts
            blit_rgba32f_opts(&mut dst32, &src32, mode, op, 0.5, 1.0, false, AlphaMode::Premultiplied, false);
            assert_eq!(dst32, orig32, "RGBA32F opts must be unmodified for op {op} in mode {mode:?}");
        }
    }
}

#[test]
fn test_challenger_excess_opacity_and_fill_clamping() {
    let excess_values = [1.5f32, 2.0, 10.0, 1000.0, f32::INFINITY];

    for &val in &excess_values {
        for &mode in &ALL_MODES {
            let mut dst8_1 = [100u8, 120, 140, 255];
            let mut dst8_excess = [100u8, 120, 140, 255];
            let src8 = [180u8, 190, 200, 255];

            // Opacity = 1.0 vs Opacity = val (should clamp to 1.0)
            blit_rgba8(&mut dst8_1, &src8, mode, 1.0, 1.0);
            blit_rgba8(&mut dst8_excess, &src8, mode, val, val);
            assert_eq!(dst8_excess, dst8_1, "Excess opacity/fill {val} in RGBA8 must clamp to 1.0 for {mode:?}");

            let mut dst16_1 = [10000u16, 12000, 14000, 65535];
            let mut dst16_excess = [10000u16, 12000, 14000, 65535];
            let src16 = [18000u16, 19000, 20000, 65535];

            blit_rgba16(&mut dst16_1, &src16, mode, 1.0, 1.0);
            blit_rgba16(&mut dst16_excess, &src16, mode, val, val);
            assert_eq!(dst16_excess, dst16_1, "Excess opacity/fill {val} in RGBA16 must clamp to 1.0 for {mode:?}");
        }
    }
}

// =========================================================================
// 5. CLIPPING GROUP ISOLATION AND TRANSPARENT BASE INTEGRITY
// =========================================================================

#[test]
fn test_challenger_clipping_group_alpha_preservation_and_transparent_base() {
    for &mode in &ALL_MODES {
        // 1. Transparent base in RGBA8: must be untouched
        let mut dst8_trans = [0u8, 0, 0, 0];
        let src8 = [255u8, 255, 255, 255];
        blit_rgba8_clipped(&mut dst8_trans, &src8, mode, 1.0, 1.0);
        assert_eq!(dst8_trans, [0, 0, 0, 0], "Transparent base must remain transparent under clipping for {mode:?}");

        // 2. Translucent base in RGBA8: dst alpha must be preserved exactly
        let mut dst8_semi = [100u8, 150, 200, 128];
        blit_rgba8_clipped(&mut dst8_semi, &src8, mode, 0.8, 0.6);
        assert_eq!(dst8_semi[3], 128, "Translucent base alpha (128) must be preserved under clipping for {mode:?}");

        // 3. Transparent base in RGBA16
        let mut dst16_trans = [0u16, 0, 0, 0];
        let src16 = [65535u16, 65535, 65535, 65535];
        blit_rgba16_clipped(&mut dst16_trans, &src16, mode, 1.0, 1.0);
        assert_eq!(dst16_trans, [0, 0, 0, 0], "Transparent base must remain transparent under clipping for {mode:?}");

        // 4. Translucent base in RGBA16
        let mut dst16_semi = [20000u16, 30000, 40000, 32768];
        blit_rgba16_clipped(&mut dst16_semi, &src16, mode, 0.8, 0.6);
        assert_eq!(dst16_semi[3], 32768, "Translucent base alpha (32768) must be preserved under clipping for {mode:?}");

        // 5. Transparent base in RGBA32F
        let mut dst32_trans = [[0.0f32, 0.0, 0.0, 0.0]];
        let src32 = [[1.0f32, 1.0, 1.0, 1.0]];
        blit_rgba32f_clipped(&mut dst32_trans, &src32, mode, 1.0, 1.0, false);
        assert_eq!(dst32_trans[0], [0.0, 0.0, 0.0, 0.0], "Transparent base in RGBA32F must remain transparent for {mode:?}");

        // 6. Translucent base in RGBA32F
        let mut dst32_semi = [[0.3f32, 0.4, 0.5, 0.6]];
        blit_rgba32f_clipped(&mut dst32_semi, &src32, mode, 0.8, 0.6, false);
        assert_eq!(dst32_semi[0][3], 0.6, "Translucent base alpha (0.6) must be preserved for {mode:?}");
    }
}

// =========================================================================
// 6. PREMULTIPLIED ALPHA PATH PARITY & OPACITY SCALING
// =========================================================================

#[test]
fn test_challenger_premultiplied_alpha_scaling() {
    // Test that when opacity = 0.5, premultiplied source is scaled correctly
    // src = (100, 50, 25, 200). With opacity 0.5:
    // as_ = 100, factor = 155.
    // dst = (0, 0, 0, 0).
    // Result: color channels must be scaled, not raw!
    let mut dst8 = [0u8, 0, 0, 0];
    let src8 = [100u8, 50, 25, 200];
    blit_rgba8_premultiplied(&mut dst8, &src8, 0.5);

    // Expected: s[c] * 0.5 => 50, 25, 12 (approx div255)
    assert!(dst8[0] >= 49 && dst8[0] <= 51, "Premultiplied red channel not scaled: got {}", dst8[0]);
    assert!(dst8[1] >= 24 && dst8[1] <= 26, "Premultiplied green channel not scaled: got {}", dst8[1]);
    assert!(dst8[2] >= 11 && dst8[2] <= 13, "Premultiplied blue channel not scaled: got {}", dst8[2]);
    assert!(dst8[3] >= 99 && dst8[3] <= 101, "Premultiplied alpha channel not scaled: got {}", dst8[3]);

    let mut dst16 = [0u16, 0, 0, 0];
    let src16 = [10000u16, 5000, 2500, 20000];
    blit_rgba16_premultiplied(&mut dst16, &src16, 0.5);
    assert!(dst16[0] >= 4990 && dst16[0] <= 5010, "Premultiplied u16 red not scaled: got {}", dst16[0]);
    assert!(dst16[1] >= 2490 && dst16[1] <= 2510, "Premultiplied u16 green not scaled: got {}", dst16[1]);
    assert!(dst16[2] >= 1240 && dst16[2] <= 1260, "Premultiplied u16 blue not scaled: got {}", dst16[2]);
    assert!(dst16[3] >= 9990 && dst16[3] <= 10010, "Premultiplied u16 alpha not scaled: got {}", dst16[3]);
}
