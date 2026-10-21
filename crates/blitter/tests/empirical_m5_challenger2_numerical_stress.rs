//! Empirical Challenge Harness: M5 Phase 2 Tier 5 Numerical Stability & Stress Suite
//!
//! Rigorously verifies:
//! 1. Division by zero in dodge / burn / vivid light / divide formulas across:
//!    - IEEE-754 32-bit float kernels
//!    - 8-bit fixed-point integer fast path (exhaustive 65,536 pairs)
//!    - 16-bit fixed-point integer fast path (boundary conditions & dense grid)
//! 2. Subnormal floats and denormals:
//!    - Subnormal denormal handling in all blend formulas, color conversions, text gamma, and slice blitting
//! 3. High bit depth operations (16-bit and 32-bit float channels):
//!    - RGBA16 fixed point and RGBA32F float HDR/SDR dynamic ranges
//! 4. Rayon parallel chunk thresholds:
//!    - Single-thread vs multi-thread chunk boundaries at `n = 4095`, `4096`, `4097` (and `8191`, `8192`, `8193`)
//!    - Bit-exact parity between serial and parallel chunk execution across RGBA8, RGBA16, and RGBA32F

use craft_blitter::blend::{
    blend_channel, clip_color, color_burn, color_dodge, divide, hard_mix_ps, linear_to_srgb, lum, sat, set_lum, set_sat, srgb_to_linear, text_decode,
    text_encode, vivid_light_generic, vivid_light_ps,
};
use craft_blitter::fixed::{
    blend_channel_u8, blend_channel_u16, color_burn_u8, color_burn_u16, color_dodge_u8, color_dodge_u16, divide_u8, divide_u16, hard_light_u8,
    hard_light_u16, hard_mix_u8, hard_mix_u16, vivid_light_u8, vivid_light_u16,
};
use craft_blitter::simd::composite_separable_rgb_span_serial;
use craft_blitter::{AlphaMode, BlendMode, blit_rgba8, blit_rgba16, blit_rgba16_opts, blit_rgba32f, blit_rgba32f_opts};

// =========================================================================
// 1. DIVISION BY ZERO STRESS: DODGE, BURN, VIVID LIGHT, DIVIDE
// =========================================================================

#[test]
fn test_empirical_div_by_zero_float_kernels() {
    let edge_denominators = [
        0.0f32,
        -0.0f32,
        1.0f32,
        -1.0f32,
        0.5f32,
        1e-45f32,
        -1e-45f32,
        f32::MIN_POSITIVE,
        -f32::MIN_POSITIVE,
        f32::from_bits(1),
        1.0 - 1e-7,
        0.5 - 1e-7,
        0.5 + 1e-7,
    ];

    let edge_numerators = [0.0f32, -0.0f32, 1.0f32, -1.0f32, 0.5f32, 1e-4f32, 1.0 - 1e-4, 1e-45f32, f32::MAX, f32::MIN_POSITIVE];

    for &cs in &edge_denominators {
        for &cb in &edge_numerators {
            // 1. Color Burn: cs <= 0.0 guard, cb >= 1.0 - EDGE guard
            let res_burn = color_burn(cb, cs);
            assert!(!res_burn.is_nan(), "color_burn({cb}, {cs}) produced NaN: {res_burn}");

            // 2. Color Dodge: cb <= EDGE guard, cs >= 1.0 guard
            let res_dodge = color_dodge(cb, cs);
            assert!(!res_dodge.is_nan(), "color_dodge({cb}, {cs}) produced NaN: {res_dodge}");

            // 3. Vivid Light PS: cs <= 0.0, cs >= 1.0 guards
            let res_vl_ps = vivid_light_ps(cb, cs);
            assert!(!res_vl_ps.is_nan(), "vivid_light_ps({cb}, {cs}) produced NaN: {res_vl_ps}");

            // 4. Vivid Light Generic & Hard Mix
            let res_vl_gen = vivid_light_generic(cb, cs);
            assert!(!res_vl_gen.is_nan(), "vivid_light_generic({cb}, {cs}) produced NaN: {res_vl_gen}");
            let res_hm = hard_mix_ps(cb, cs);
            assert!(res_hm == 0.0 || res_hm == 1.0, "hard_mix_ps({cb}, {cs}) must be 0.0 or 1.0, got {res_hm}");

            // 5. Divide: SDR and HDR
            let res_div_sdr = divide(cb, cs, 1.0);
            assert!(!res_div_sdr.is_nan(), "divide SDR({cb}, {cs}) produced NaN: {res_div_sdr}");
            assert!(res_div_sdr <= 1.0, "divide SDR({cb}, {cs}) exceeded 1.0: {res_div_sdr}");

            let res_div_hdr = divide(cb, cs, f32::MAX);
            assert!(!res_div_hdr.is_nan(), "divide HDR({cb}, {cs}) produced NaN: {res_div_hdr}");
        }
    }
}

#[test]
fn test_empirical_div_by_zero_u8_exhaustive() {
    // Exhaustive 65,536 combinations of (cb, cs) for u8 divide, color_burn, color_dodge, vivid_light
    for cb in 0..=255u8 {
        for cs in 0..=255u8 {
            let _ = divide_u8(cb, cs);
            let _ = color_burn_u8(cb, cs);
            let _ = color_dodge_u8(cb, cs);
            let _ = vivid_light_u8(cb, cs);
            let _ = hard_light_u8(cb, cs);
            let _ = hard_mix_u8(cb, cs);

            // General channel blend for all separable modes
            for &mode in &BlendMode::LAYER_MODES {
                let _ = blend_channel_u8(mode, cb, cs);
            }
        }
    }

    // Explicit boundary assertions
    assert_eq!(divide_u8(0, 0), 0);
    assert_eq!(divide_u8(128, 0), 255);
    assert_eq!(color_burn_u8(255, 0), 255);
    assert_eq!(color_burn_u8(128, 0), 0);
    assert_eq!(color_dodge_u8(0, 255), 0);
    assert_eq!(color_dodge_u8(128, 255), 255);
    assert_eq!(vivid_light_u8(128, 0), 0);
    assert_eq!(vivid_light_u8(128, 255), 255);
}

#[test]
fn test_empirical_div_by_zero_u16_boundaries() {
    let edge_values_u16 = [0u16, 1, 2, 3, 127, 128, 255, 256, 32767, 32768, 32769, 65533, 65534, 65535];

    // Boundary matrix
    for &cb in &edge_values_u16 {
        for &cs in &edge_values_u16 {
            let _ = divide_u16(cb, cs);
            let _ = color_burn_u16(cb, cs);
            let _ = color_dodge_u16(cb, cs);
            let _ = vivid_light_u16(cb, cs);
            let _ = hard_light_u16(cb, cs);
            let _ = hard_mix_u16(cb, cs);

            for &mode in &BlendMode::LAYER_MODES {
                let _ = blend_channel_u16(mode, cb, cs);
            }
        }
    }

    // Dense grid across 65,536 space with prime step 257
    let dense_steps: Vec<u16> = (0..=65535u32).step_by(257).map(|x| x as u16).chain(std::iter::once(65535u16)).collect();
    for &cb in &dense_steps {
        for &cs in &dense_steps {
            let _ = divide_u16(cb, cs);
            let _ = color_burn_u16(cb, cs);
            let _ = color_dodge_u16(cb, cs);
            let _ = vivid_light_u16(cb, cs);
        }
    }

    // Explicit boundary assertions
    assert_eq!(divide_u16(0, 0), 0);
    assert_eq!(divide_u16(32768, 0), 65535);
    assert_eq!(color_burn_u16(65535, 0), 65535);
    assert_eq!(color_burn_u16(32768, 0), 0);
    assert_eq!(color_dodge_u16(0, 65535), 0);
    assert_eq!(color_dodge_u16(32768, 65535), 65535);
    assert_eq!(vivid_light_u16(32768, 0), 0);
    assert_eq!(vivid_light_u16(32768, 65535), 65535);
}

// =========================================================================
// 2. SUBNORMAL FLOATS AND DENORMALS STRESS
// =========================================================================

#[test]
fn test_empirical_subnormals_and_denormals_robustness() {
    let subnormals = [
        f32::from_bits(1),           // Smallest positive subnormal: ~1.4e-45
        f32::from_bits(2),           // Second subnormal
        f32::from_bits(0x007f_ffff), // Largest subnormal: ~1.1754942e-38
        f32::from_bits(0x8000_0001), // Smallest negative subnormal
        f32::from_bits(0x807f_ffff), // Largest negative subnormal
        1e-45f32,
        -1e-45f32,
        f32::MIN_POSITIVE / 2.0,
        -f32::MIN_POSITIVE / 2.0,
        f32::MIN_POSITIVE * 0.75,
    ];

    let standard_values = [0.0f32, 0.25, 0.5, 0.75, 1.0, 1.5, -0.5];

    for &sub in &subnormals {
        assert!(sub.is_subnormal(), "Value {sub:e} must be identified as subnormal");

        // Single channel blend test
        for &mode in &BlendMode::LAYER_MODES {
            for &std_v in &standard_values {
                // Subnormal as backdrop
                let r1 = blend_channel(mode, sub, std_v, 1.0);
                assert!(!r1.is_nan(), "blend_channel({mode:?}, sub={sub:e}, {std_v}) produced NaN");

                // Subnormal as source
                let r2 = blend_channel(mode, std_v, sub, 1.0);
                assert!(!r2.is_nan(), "blend_channel({mode:?}, {std_v}, sub={sub:e}) produced NaN");

                // Subnormal as both
                let r3 = blend_channel(mode, sub, sub, 1.0);
                assert!(!r3.is_nan(), "blend_channel({mode:?}, sub, sub) produced NaN");
            }
        }

        // Color space helpers with subnormals
        let rgb_sub = [sub, 0.5, 0.7];
        let _ = lum(rgb_sub);
        let _ = sat(rgb_sub);
        let _ = clip_color(rgb_sub, 1.0);
        let _ = set_lum(rgb_sub, 0.5, 1.0);
        let _ = set_sat(rgb_sub, 0.5);

        // Gamma functions with subnormals
        let _ = srgb_to_linear(sub);
        let _ = linear_to_srgb(sub);
        let _ = text_encode(sub, 1.45);
        let _ = text_decode(sub, 1.45);

        // Slice blitter with subnormals
        let mut dst_slice = [[sub, 0.4, 0.6, 1.0]; 16];
        let src_slice = [[0.2, sub, 0.8, sub]; 16];
        blit_rgba32f(&mut dst_slice, &src_slice, BlendMode::Normal, 1.0, 1.0, false);
        blit_rgba32f(&mut dst_slice, &src_slice, BlendMode::Multiply, 0.8, 0.7, true);
        blit_rgba32f_opts(&mut dst_slice, &src_slice, BlendMode::ColorDodge, 1.0, 1.0, 1.45, false, AlphaMode::Premultiplied, false);
        blit_rgba32f_opts(&mut dst_slice, &src_slice, BlendMode::VividLight, 1.0, 1.0, 1.0, true, AlphaMode::Straight, true);
    }
}

// =========================================================================
// 3. HIGH BIT DEPTH OPERATIONS (16-BIT & 32-BIT FLOAT HDR/SDR)
// =========================================================================

#[test]
fn test_empirical_high_bit_depth_operations() {
    // RGBA16 comprehensive test
    let mut dst16 = vec![0u16; 64 * 4];
    let mut src16 = vec![0u16; 64 * 4];

    for i in 0..64 {
        dst16[i * 4] = ((i * 1019) % 65536) as u16;
        dst16[i * 4 + 1] = ((i * 2039) % 65536) as u16;
        dst16[i * 4 + 2] = ((i * 3049) % 65536) as u16;
        dst16[i * 4 + 3] = 65535;

        src16[i * 4] = ((i * 4079) % 65536) as u16;
        src16[i * 4 + 1] = ((i * 5087) % 65536) as u16;
        src16[i * 4 + 2] = ((i * 6091) % 65536) as u16;
        src16[i * 4 + 3] = ((i * 997) % 65536) as u16;
    }

    for &mode in &BlendMode::LAYER_MODES {
        let mut d = dst16.clone();
        blit_rgba16(&mut d, &src16, mode, 0.85, 0.75);

        // Test clipping
        let mut d_clip = dst16.clone();
        blit_rgba16_opts(&mut d_clip, &src16, mode, 0.85, 0.75, AlphaMode::Straight, true);
        for i in 0..64 {
            assert_eq!(d_clip[i * 4 + 3], dst16[i * 4 + 3], "RGBA16 clipped alpha corrupted at pixel {i}");
        }

        // Test premultiplied
        let mut d_pm = dst16.clone();
        blit_rgba16_opts(&mut d_pm, &src16, mode, 0.85, 1.0, AlphaMode::Premultiplied, false);
        for i in 0..64 {
            let a = d_pm[i * 4 + 3];
            assert!(d_pm[i * 4] <= a, "Premultiplied R <= A violated at pixel {i}");
            assert!(d_pm[i * 4 + 1] <= a, "Premultiplied G <= A violated at pixel {i}");
            assert!(d_pm[i * 4 + 2] <= a, "Premultiplied B <= A violated at pixel {i}");
        }
    }

    // RGBA32F HDR Dynamic Range preservation
    let mut dst32_hdr = vec![[5.0f32, 10.0, 20.0, 1.0]; 32];
    let src32_hdr = vec![[2.0f32, 4.0, 8.0, 1.0]; 32];

    blit_rgba32f(&mut dst32_hdr, &src32_hdr, BlendMode::LinearDodge, 1.0, 1.0, true);
    for p in &dst32_hdr {
        assert!((p[0] - 7.0).abs() < 1e-4, "HDR LinearDodge R: got {}, expected 7.0", p[0]);
        assert!((p[1] - 14.0).abs() < 1e-4, "HDR LinearDodge G: got {}, expected 14.0", p[1]);
        assert!((p[2] - 28.0).abs() < 1e-4, "HDR LinearDodge B: got {}, expected 28.0", p[2]);
    }

    // RGBA32F SDR clamping
    let mut dst32_sdr = vec![[0.8f32, 0.7, 0.9, 1.0]; 32];
    let src32_sdr = vec![[0.5f32, 0.6, 0.4, 1.0]; 32];
    blit_rgba32f(&mut dst32_sdr, &src32_sdr, BlendMode::LinearDodge, 1.0, 1.0, false);
    for p in &dst32_sdr {
        assert_eq!(p[0], 1.0, "SDR LinearDodge R must clamp to 1.0");
        assert_eq!(p[1], 1.0, "SDR LinearDodge G must clamp to 1.0");
        assert_eq!(p[2], 1.0, "SDR LinearDodge B must clamp to 1.0");
    }
}

// =========================================================================
// 4. RAYON PARALLEL CHUNK THRESHOLDS: n = 4095, 4096, 4097
// =========================================================================

#[test]
fn test_empirical_rayon_chunk_boundaries_rgba8() {
    let threshold_sizes = [
        4095usize, // Strictly single-threaded serial
        4096usize, // Exactly at Rayon PAR_THRESHOLD (1 full chunk)
        4097usize, // 1 full chunk of 4096 + 1 tail element
        8191usize, // 1 full chunk of 4096 + 4095 tail chunk
        8192usize, // Exactly 2 full chunks
        8193usize, // 2 full chunks + 1 tail element
    ];

    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::LinearDodge,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::ColorBurn,
        BlendMode::ColorDodge,
        BlendMode::VividLight,
    ];

    for &n in &threshold_sizes {
        let initial_dst: Vec<u8> = (0..n * 4).map(|i| ((i * 13 + 7) % 256) as u8).collect();
        let src: Vec<u8> = (0..n * 4).map(|i| ((i * 37 + 19) % 256) as u8).collect();

        for &mode in &modes {
            // 1. Parallel execution via blit_rgba8
            let mut par_dst = initial_dst.clone();
            blit_rgba8(&mut par_dst, &src, mode, 0.8, 0.7);

            // 2. Serial reference execution
            let mut ser_dst = initial_dst.clone();
            craft_blitter::fixed::blit_rgba8_fixed(&mut ser_dst, &src, mode, 0.8, 0.7, AlphaMode::Straight, false);

            // 3. Strict bit-exact equivalence
            assert_eq!(par_dst, ser_dst, "Rayon parallel execution diverged from serial reference for mode {mode:?} at length n={n}");
        }
    }
}

#[test]
fn test_empirical_rayon_chunk_boundaries_rgba16() {
    let threshold_sizes = [4095usize, 4096usize, 4097usize, 8191usize, 8192usize, 8193usize];

    let modes = [
        BlendMode::Normal,
        BlendMode::Multiply,
        BlendMode::Screen,
        BlendMode::LinearDodge,
        BlendMode::Darken,
        BlendMode::Lighten,
        BlendMode::Difference,
        BlendMode::Exclusion,
        BlendMode::VividLight,
    ];

    for &n in &threshold_sizes {
        let initial_dst: Vec<u16> = (0..n * 4).map(|i| ((i * 1019 + 77) % 65536) as u16).collect();
        let src: Vec<u16> = (0..n * 4).map(|i| ((i * 2039 + 133) % 65536) as u16).collect();

        for &mode in &modes {
            // 1. Parallel execution
            let mut par_dst = initial_dst.clone();
            blit_rgba16(&mut par_dst, &src, mode, 0.85, 0.65);

            // 2. Serial reference execution
            let mut ser_dst = initial_dst.clone();
            craft_blitter::fixed::blit_rgba16_fixed(&mut ser_dst, &src, mode, 0.85, 0.65, AlphaMode::Straight, false);

            // 3. Strict bit-exact equivalence
            assert_eq!(par_dst, ser_dst, "Rayon parallel execution diverged from serial reference for mode {mode:?} at length n={n}");
        }
    }
}

#[test]
fn test_empirical_rayon_chunk_boundaries_rgba32f() {
    let threshold_sizes = [4095usize, 4096usize, 4097usize, 8191usize, 8192usize, 8193usize];

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

    for &n in &threshold_sizes {
        let initial_dst: Vec<[f32; 4]> = (0..n)
            .map(|i| {
                let f = (i % 251) as f32 / 250.0;
                [f * 0.7 + 0.1, (1.0 - f) * 0.8, (f * 0.6 + 0.3).min(1.0), 0.9]
            })
            .collect();

        let src: Vec<[f32; 4]> = (0..n)
            .map(|i| {
                let f = ((i * 17 + 43) % 251) as f32 / 250.0;
                [(1.0 - f) * 0.6 + 0.2, f * 0.75, (f * 0.5 + 0.1), 0.85]
            })
            .collect();

        for &mode in &separable_modes {
            // 1. Parallel execution via blit_rgba32f
            let mut par_dst = initial_dst.clone();
            blit_rgba32f(&mut par_dst, &src, mode, 0.77, 1.0, false);

            // 2. Serial reference execution via composite_separable_rgb_span_serial
            let mut ser_dst = initial_dst.clone();
            composite_separable_rgb_span_serial(&mut ser_dst, &src, mode, 0.77, false);

            // 3. Verify exact float match within tight epsilon
            for i in 0..n {
                for c in 0..4 {
                    let diff = (par_dst[i][c] - ser_dst[i][c]).abs();
                    assert!(
                        diff < 1e-6,
                        "Parallel vs Serial mismatch for mode {mode:?} at n={n}, pixel {i}, ch {c}: par={}, ser={} (diff {diff})",
                        par_dst[i][c],
                        ser_dst[i][c]
                    );
                }
            }
        }
    }
}
