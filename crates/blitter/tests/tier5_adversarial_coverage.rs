#![allow(clippy::needless_range_loop, clippy::manual_memcpy)]
//!
//! White-box stress testing and boundary verification across:
//! 1. Fixed-point SIMD arithmetic: overflow boundaries, zero-alpha vs pure opaque, extreme fill values.
//! 2. Float SIMD chunking: edge buffer remnants (n % 8 != 0), NaN/Infinity inputs, optical text gamma values, HDR modes.

use craft_blitter::{BlendMode, blit_rgba8, blit_rgba8_clipped, blit_rgba16, blit_rgba16_clipped, blit_rgba32f, blit_rgba32f_gamma};

const CANARY_32F: [f32; 4] = [123.456, 234.567, 345.678, 456.789];

// =========================================================================
// AREA 1: FIXED-POINT SIMD ARITHMETIC & BOUNDARY STRESS
// =========================================================================

#[test]
fn test_tier5_fixed_point_exhaustive_overflow_boundaries_u8() {
    let boundary_values_u8 = [0u8, 1, 2, 127, 128, 129, 253, 254, 255];

    for &mode in &BlendMode::LAYER_MODES {
        for &cb in &boundary_values_u8 {
            for &cs in &boundary_values_u8 {
                let res = craft_blitter::fixed::blend_channel_u8(mode, cb, cs);
                // In non-separable or float-fallback modes, res is clamped to [0, 255]
                let _ = res;

                // Test via slice blit with various alphas
                for &b_a in &[0u8, 128, 255] {
                    for &s_a in &[0u8, 128, 255] {
                        let mut dst = [cb, cb, cb, b_a];
                        let src = [cs, cs, cs, s_a];
                        blit_rgba8(&mut dst, &src, mode, 1.0, 1.0);

                        // If source alpha is 0, destination must be untouched
                        if s_a == 0 {
                            assert_eq!(dst, [cb, cb, cb, b_a], "Zero source alpha modified dst in mode {mode:?}");
                        }

                        // If backdrop alpha is 0 in straight mode and s_a is 255, dst must equal src
                        if b_a == 0 && s_a == 255 {
                            assert_eq!(dst, [cs, cs, cs, 255], "Zero backdrop alpha did not take pure source in mode {mode:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_tier5_fixed_point_exhaustive_overflow_boundaries_u16() {
    let boundary_values_u16 = [0u16, 1, 2, 32767, 32768, 32769, 65533, 65534, 65535];

    for &mode in &BlendMode::LAYER_MODES {
        for &cb in &boundary_values_u16 {
            for &cs in &boundary_values_u16 {
                let res = craft_blitter::fixed::blend_channel_u16(mode, cb, cs);
                let _ = res;

                for &b_a in &[0u16, 32768, 65535] {
                    for &s_a in &[0u16, 32768, 65535] {
                        let mut dst = [cb, cb, cb, b_a];
                        let src = [cs, cs, cs, s_a];
                        blit_rgba16(&mut dst, &src, mode, 1.0, 1.0);

                        if s_a == 0 {
                            assert_eq!(dst, [cb, cb, cb, b_a], "Zero source alpha modified dst16 in mode {mode:?}");
                        }

                        if b_a == 0 && s_a == 65535 {
                            assert_eq!(dst, [cs, cs, cs, 65535], "Zero backdrop alpha did not take pure source in mode {mode:?}");
                        }
                    }
                }
            }
        }
    }
}

#[test]
fn test_tier5_fixed_point_extreme_fill_dynamics() {
    let special_modes = [
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
        BlendMode::VividLight,
        BlendMode::LinearLight,
        BlendMode::HardMix,
        BlendMode::Difference,
    ];

    let extreme_fills = [-100.0f32, -1.0, -0.0001, 0.0, 1e-6, 0.5, 0.9999, 1.0, 1.0001, 10.0, 100.0, f32::NAN, f32::INFINITY];

    for &mode in &special_modes {
        for &fill in &extreme_fills {
            let mut dst8 = [120u8, 140, 160, 255];
            let src8 = [200u8, 80, 40, 255];
            let orig8 = dst8;

            blit_rgba8(&mut dst8, &src8, mode, 1.0, fill);

            if fill <= 0.0 {
                // At fill 0, Special Eight pulls source to neutral color, producing backdrop
                for c in 0..3 {
                    assert!(
                        (dst8[c] as i32 - orig8[c] as i32).abs() <= 1,
                        "Special Eight mode {mode:?} at fill {fill} did not resolve to backdrop: got {dst8:?}, orig {orig8:?}"
                    );
                }
            }

            let mut dst16 = [30000u16, 35000, 40000, 65535];
            let src16 = [50000u16, 20000, 10000, 65535];
            let orig16 = dst16;

            blit_rgba16(&mut dst16, &src16, mode, 1.0, fill);

            if fill <= 0.0 {
                for c in 0..3 {
                    assert!(
                        (dst16[c] as i64 - orig16[c] as i64).abs() <= 2,
                        "Special Eight mode {mode:?} at fill {fill} u16 did not resolve to backdrop: got {dst16:?}, orig {orig16:?}"
                    );
                }
            }
        }
    }
}

#[test]
fn test_tier5_fixed_point_clipping_isolation_and_alpha_preservation() {
    for &mode in &BlendMode::LAYER_MODES {
        // 1. Transparent base must remain completely untouched in RGBA8 and RGBA16
        let mut trans8 = [0u8, 0, 0, 0];
        let src8 = [255u8, 255, 255, 255];
        blit_rgba8_clipped(&mut trans8, &src8, mode, 1.0, 1.0);
        assert_eq!(trans8, [0, 0, 0, 0], "Transparent base modified under clipping in RGBA8 for {mode:?}");

        let mut trans16 = [0u16, 0, 0, 0];
        let src16 = [65535u16, 65535, 65535, 65535];
        blit_rgba16_clipped(&mut trans16, &src16, mode, 1.0, 1.0);
        assert_eq!(trans16, [0, 0, 0, 0], "Transparent base modified under clipping in RGBA16 for {mode:?}");

        // 2. Translucent base alpha must be strictly preserved
        for &base_a in &[1u8, 50, 128, 200, 254] {
            let mut semi8 = [100u8, 120, 140, base_a];
            blit_rgba8_clipped(&mut semi8, &src8, mode, 0.8, 0.7);
            assert_eq!(semi8[3], base_a, "Translucent base alpha {base_a} modified in RGBA8 for {mode:?}");
        }

        for &base_a in &[1u16, 10000, 32768, 50000, 65534] {
            let mut semi16 = [25000u16, 30000, 35000, base_a];
            blit_rgba16_clipped(&mut semi16, &src16, mode, 0.8, 0.7);
            assert_eq!(semi16[3], base_a, "Translucent base alpha {base_a} modified in RGBA16 for {mode:?}");
        }
    }
}

// =========================================================================
// AREA 2: FLOAT SIMD CHUNKING & HDR / TEXT GAMMA STRESS
// =========================================================================

#[test]
fn test_tier5_float_simd_exhaustive_edge_buffer_remnants_1_to_65() {
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

    const PAD: usize = 8;

    // Test every buffer size from 1 to 65 (covers n % 8 = 0, 1, 2, 3, 4, 5, 6, 7)
    for len in 1..=65 {
        let initial_dst: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = (i % 31) as f32 / 30.0;
                [f * 0.7 + 0.1, (1.0 - f) * 0.8, (f * 0.5 + 0.2), 0.85]
            })
            .collect();

        let src: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = ((i * 7 + 11) % 31) as f32 / 30.0;
                [(1.0 - f) * 0.6 + 0.2, f * 0.75, (f * 0.4 + 0.3), 0.75]
            })
            .collect();

        for &mode in &separable_modes {
            // Buffer with sentinel canaries before and after
            let mut dst_with_canaries = vec![CANARY_32F; len + 2 * PAD];
            for i in 0..len {
                dst_with_canaries[PAD + i] = initial_dst[i];
            }

            let slice = &mut dst_with_canaries[PAD..PAD + len];
            blit_rgba32f(slice, &src, mode, 0.8, 1.0, false);

            // 1. Verify leading and trailing canaries remain untouched
            for i in 0..PAD {
                assert_eq!(dst_with_canaries[i], CANARY_32F, "Leading canary corrupted at len {len}, mode {mode:?}");
            }
            for i in PAD + len..dst_with_canaries.len() {
                assert_eq!(dst_with_canaries[i], CANARY_32F, "Trailing canary corrupted at len {len}, mode {mode:?}");
            }

            // 2. Verify all blitted pixels match scalar reference oracle within 1e-5
            for i in 0..len {
                let oracle = craft_blitter::float::composite_f32(mode, initial_dst[i], src[i], 0.8, false);
                for c in 0..4 {
                    let diff = (dst_with_canaries[PAD + i][c] - oracle[c]).abs();
                    assert!(
                        diff < 1e-5,
                        "SIMD chunk vs scalar mismatch at len {len}, pixel {i}, ch {c}, mode {mode:?}: got {}, want {}",
                        dst_with_canaries[PAD + i][c],
                        oracle[c]
                    );
                }
            }
        }
    }
}

#[test]
fn test_tier5_float_simd_nan_and_infinity_resilience() {
    let adversarial_floats = [
        f32::NAN,
        -f32::NAN,
        f32::INFINITY,
        f32::NEG_INFINITY,
        f32::MIN_POSITIVE,
        f32::MIN_POSITIVE / 4.0, // denormal
        f32::from_bits(1),       // subnormal
        0.0f32,
        -0.0f32,
        -1.0f32,
        1e38f32,
        -1e38f32,
    ];

    for &adv_val in &adversarial_floats {
        for &mode in &BlendMode::LAYER_MODES {
            // 1. Destination contains adversarial values
            let mut dst = [[adv_val; 4]; 11]; // 11 = 8 SIMD chunk + 3 tail
            let src = [[0.5f32, 0.5, 0.5, 1.0]; 11];
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, true);

            // 2. Source contains adversarial values
            let mut dst2 = [[0.5f32, 0.5, 0.5, 1.0]; 11];
            let src2 = [[adv_val; 4]; 11];
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, true);

            // 3. Opacity and fill adversarial values
            let mut dst3 = [[0.4f32, 0.5, 0.6, 0.8]; 11];
            let src3 = [[0.7f32, 0.3, 0.2, 0.9]; 11];
            blit_rgba32f(&mut dst3, &src3, mode, adv_val, adv_val, false);

            // 4. Gamma adversarial values
            blit_rgba32f_gamma(&mut dst3, &src3, mode, 0.8, 0.8, adv_val, false);
        }
    }
}

#[test]
fn test_tier5_float_optical_text_gamma_sweep() {
    let gammas = [0.1f32, 0.5, 1.0, 1.45, 1.8, 2.2, 3.0, 10.0, -1.0, 0.0, f32::NAN, f32::INFINITY];

    let backdrop = [0.5f32, 0.5, 0.5, 1.0];
    let source = [0.0f32, 0.2, 0.6, 0.7];

    for &mode in &BlendMode::LAYER_MODES {
        for &gamma in &gammas {
            let res = craft_blitter::float::composite_gamma_f32(mode, backdrop, source, 1.0, gamma, false);
            // Verify no crash, and output alpha is valid
            assert!(!res[3].is_nan());

            // When gamma == 1.0, it must exactly equal composite_f32
            if (gamma - 1.0).abs() < 1e-6 {
                let exp = craft_blitter::float::composite_f32(mode, backdrop, source, 1.0, false);
                assert_eq!(res, exp, "gamma = 1.0 must match composite_f32 exactly for {mode:?}");
            }
        }
    }
}

#[test]
fn test_tier5_float_hdr_unclamped_vs_clamped_behavior() {
    // Backdrop and source with values > 1.0
    let dst_val = [2.5f32, 1.8, 3.0, 1.0];
    let src_val = [1.5f32, 2.2, 0.5, 1.0];

    // 1. LinearDodge: 2.5 + 1.5 = 4.0
    let mut dst_sdr = [dst_val];
    let mut dst_hdr = [dst_val];
    blit_rgba32f(&mut dst_sdr, &[src_val], BlendMode::LinearDodge, 1.0, 1.0, false);
    blit_rgba32f(&mut dst_hdr, &[src_val], BlendMode::LinearDodge, 1.0, 1.0, true);

    assert_eq!(dst_sdr[0][0], 1.0, "SDR LinearDodge must clamp to 1.0");
    assert!((dst_hdr[0][0] - 4.0).abs() < 1e-5, "HDR LinearDodge must equal 4.0, got {}", dst_hdr[0][0]);

    // 2. Divide: 2.5 / 0.5 = 5.0
    let div_dst = [2.5f32, 2.5, 2.5, 1.0];
    let div_src = [0.5f32, 0.5, 0.5, 1.0];

    let mut div_sdr = [div_dst];
    let mut div_hdr = [div_dst];
    blit_rgba32f(&mut div_sdr, &[div_src], BlendMode::Divide, 1.0, 1.0, false);
    blit_rgba32f(&mut div_hdr, &[div_src], BlendMode::Divide, 1.0, 1.0, true);

    assert_eq!(div_sdr[0][0], 1.0, "SDR Divide must clamp to 1.0");
    assert!((div_hdr[0][0] - 5.0).abs() < 1e-5, "HDR Divide must equal 5.0, got {}", div_hdr[0][0]);
}
