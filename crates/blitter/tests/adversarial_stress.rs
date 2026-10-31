use craft_blitter::{
    AlphaMode, BlendMode, blit_rgba8, blit_rgba8_clipped, blit_rgba8_opts, blit_rgba8_premultiplied, blit_rgba16, blit_rgba16_clipped,
    blit_rgba16_opts, blit_rgba16_premultiplied, blit_rgba32f, blit_rgba32f_clipped, blit_rgba32f_gamma, blit_rgba32f_opts,
    blit_rgba32f_premultiplied, composite_fill_f32, composite_normal_f32,
};

const SENTINEL_U8: u8 = 0xDE;
const SENTINEL_U16: u16 = 0xDEAD;
const SENTINEL_F32: [f32; 4] = [999.0, 888.0, 777.0, 666.0];

// ---------------------------------------------------------------------------
// 1. Mismatched Slice Lengths & Remainder Preservations
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_mismatched_slice_lengths_rgba8() {
    let lengths = [
        (0, 0),
        (0, 4),
        (4, 0),
        (0, 7),
        (7, 0),
        (1, 1),
        (1, 5),
        (5, 1),
        (3, 3),
        (3, 8),
        (8, 3),
        (7, 13),
        (13, 7),
        (16, 24),
        (24, 16),
        (100, 20),
        (20, 100),
        (4097 * 4, 100),
        (100, 4097 * 4),
    ];

    for &(dst_len, src_len) in &lengths {
        for &mode in &BlendMode::LAYER_MODES {
            let mut dst = vec![SENTINEL_U8; dst_len];
            let src = vec![128u8; src_len];

            blit_rgba8(&mut dst, &src, mode, 0.7, 0.6);

            let num_pixels = (dst_len / 4).min(src_len / 4);
            let blitted_bytes = num_pixels * 4;

            // Untouched tail in dst must preserve sentinel
            for (idx, &byte) in dst[blitted_bytes..].iter().enumerate() {
                assert_eq!(
                    byte,
                    SENTINEL_U8,
                    "dst byte at index {} (past blitted bytes {}) was modified for mode {:?}",
                    blitted_bytes + idx,
                    blitted_bytes,
                    mode
                );
            }
        }
    }
}

#[test]
fn test_adversarial_mismatched_slice_lengths_rgba16() {
    let lengths = [
        (0, 0),
        (0, 4),
        (4, 0),
        (0, 7),
        (7, 0),
        (1, 1),
        (1, 5),
        (5, 1),
        (3, 3),
        (3, 8),
        (8, 3),
        (7, 13),
        (13, 7),
        (16, 24),
        (24, 16),
        (100, 20),
        (20, 100),
        (4097 * 4, 100),
        (100, 4097 * 4),
    ];

    for &(dst_len, src_len) in &lengths {
        for &mode in &BlendMode::LAYER_MODES {
            let mut dst = vec![SENTINEL_U16; dst_len];
            let src = vec![32768u16; src_len];

            blit_rgba16(&mut dst, &src, mode, 0.7, 0.6);

            let num_pixels = (dst_len / 4).min(src_len / 4);
            let blitted_words = num_pixels * 4;

            for (idx, &word) in dst[blitted_words..].iter().enumerate() {
                assert_eq!(
                    word,
                    SENTINEL_U16,
                    "dst word at index {} (past blitted words {}) was modified for mode {:?}",
                    blitted_words + idx,
                    blitted_words,
                    mode
                );
            }
        }
    }
}

#[test]
fn test_adversarial_mismatched_slice_lengths_rgba32f() {
    let lengths =
        [(0, 0), (0, 1), (1, 0), (1, 1), (2, 5), (5, 2), (7, 13), (13, 7), (15, 33), (33, 15), (4095, 4097), (4097, 4095), (5000, 10), (10, 5000)];

    for &(dst_len, src_len) in &lengths {
        for &mode in &BlendMode::LAYER_MODES {
            let mut dst = vec![SENTINEL_F32; dst_len];
            let src = vec![[0.4f32, 0.5, 0.6, 0.8]; src_len];

            blit_rgba32f(&mut dst, &src, mode, 0.7, 0.6, false);

            let num_pixels = dst_len.min(src_len);

            for (idx, &px) in dst[num_pixels..].iter().enumerate() {
                assert_eq!(
                    px,
                    SENTINEL_F32,
                    "dst pixel at index {} (past blitted pixels {}) was modified for mode {:?}",
                    num_pixels + idx,
                    num_pixels,
                    mode
                );
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 2. Prime and Non-Multiple-of-LANES Chunks & Tail Parity Verification
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_prime_and_boundary_lengths_simd_vs_scalar_oracle() {
    let prime_and_boundary_sizes = [
        1, 2, 3, 5, 7, 8, 9, 11, 13, 15, 16, 17, 31, 33, 63, 64, 65, 127, 255, 256, 257, 511, 512, 513, 1023, 1024, 1025, 4095, 4096, 4097, 8191,
        8192, 8193,
    ];

    let opacity = 0.85f32;
    let fill = 0.65f32;

    for &len in &prime_and_boundary_sizes {
        // Prepare distinct input patterns so each pixel differs
        let initial_dst: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = (i % 256) as f32 / 255.0;
                [f * 0.8, (1.0 - f) * 0.7, (f * 0.5 + 0.2), 0.9]
            })
            .collect();

        let src: Vec<[f32; 4]> = (0..len)
            .map(|i| {
                let f = ((i * 7 + 13) % 256) as f32 / 255.0;
                [f * 0.6 + 0.1, f * 0.9, (1.0 - f) * 0.4, 0.75]
            })
            .collect();

        // 1. Normal Mode (exercises simd::composite_normal_rgb_span)
        {
            let mut dst = initial_dst.clone();
            blit_rgba32f(&mut dst, &src, BlendMode::Normal, opacity, 1.0, false);

            for i in 0..len {
                let oracle = composite_normal_f32(initial_dst[i], src[i], opacity);
                for c in 0..4 {
                    let diff = (dst[i][c] - oracle[c]).abs();
                    assert!(diff < 1e-5, "Normal mode mismatch at len {}, pixel {}, channel {}: got {}, want {}", len, i, c, dst[i][c], oracle[c]);
                }
            }
        }

        // 2. Special Eight Mode (e.g. VividLight with fill < 1.0)
        {
            let mut dst = initial_dst.clone();
            blit_rgba32f(&mut dst, &src, BlendMode::VividLight, opacity, fill, false);

            for i in 0..len {
                let oracle = composite_fill_f32(BlendMode::VividLight, initial_dst[i], src[i], opacity, fill, 1.0, false);
                for c in 0..4 {
                    let diff = (dst[i][c] - oracle[c]).abs();
                    assert!(
                        diff < 1e-5,
                        "VividLight mode mismatch at len {}, pixel {}, channel {}: got {}, want {}",
                        len,
                        i,
                        c,
                        dst[i][c],
                        oracle[c]
                    );
                }
            }
        }

        // 3. RGBA8 SIMD chunk boundaries with opaque source
        {
            let mut dst8 = Vec::with_capacity(len * 4);
            let mut src8 = Vec::with_capacity(len * 4);
            for _ in 0..len {
                dst8.extend_from_slice(&[100, 120, 140, 255]);
                src8.extend_from_slice(&[200, 210, 220, 255]);
            }
            blit_rgba8(&mut dst8, &src8, BlendMode::Normal, 1.0, 1.0);
            for px in dst8.as_chunks::<4>().0 {
                assert_eq!(px, &[200, 210, 220, 255], "RGBA8 Normal blit mismatch at len {}", len);
            }
        }

        // 4. RGBA8 SIMD chunk boundaries with translucent source: verify uniform blending across SIMD chunks and tail
        {
            let mut dst8 = Vec::with_capacity(len * 4);
            let mut src8 = Vec::with_capacity(len * 4);
            for _ in 0..len {
                dst8.extend_from_slice(&[50, 60, 70, 128]);
                src8.extend_from_slice(&[150, 160, 170, 200]);
            }
            blit_rgba8(&mut dst8, &src8, BlendMode::Multiply, 0.8, 0.7);
            let expected_px = [dst8[0], dst8[1], dst8[2], dst8[3]];
            for (idx, px) in dst8.as_chunks::<4>().0.iter().enumerate() {
                assert_eq!(px, &expected_px, "RGBA8 Multiply blit mismatch across chunks/tail at len {}, pixel {}", len, idx);
            }
        }

        // 5. RGBA16 SIMD chunk boundaries with opaque source
        {
            let mut dst16 = Vec::with_capacity(len * 4);
            let mut src16 = Vec::with_capacity(len * 4);
            for _ in 0..len {
                dst16.extend_from_slice(&[10000, 20000, 30000, 65535]);
                src16.extend_from_slice(&[40000, 50000, 60000, 65535]);
            }
            blit_rgba16(&mut dst16, &src16, BlendMode::Normal, 1.0, 1.0);
            for px in dst16.as_chunks::<4>().0 {
                assert_eq!(px, &[40000, 50000, 60000, 65535], "RGBA16 Normal blit mismatch at len {}", len);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 3. Extreme Floating Point Values (NaN, Infinities, Subnormals, Negatives, HDR)
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_extreme_floats_zero_panics() {
    let extreme_f32_values = [
        f32::NAN,
        f32::from_bits(0x7FC00001), // signaling NaN pattern
        f32::from_bits(0x7F800001),
        f32::INFINITY,
        f32::NEG_INFINITY,
        0.0f32,
        -0.0f32,
        f32::MIN_POSITIVE,
        f32::MIN_POSITIVE / 2.0, // subnormal
        1e-45f32,                // smallest subnormal
        -1e-45f32,
        -1.0f32,
        -100.0f32,
        -1e30f32,
        1.0f32,
        1.5f32,
        2.0f32,
        100.0f32,
        1e10f32,
        f32::MAX,
    ];

    for &extreme_val in &extreme_f32_values {
        for &mode in &BlendMode::LAYER_MODES {
            // Test extreme values in destination
            let mut dst = [[extreme_val, extreme_val, extreme_val, extreme_val]; 17];
            let src = [[0.5f32, 0.5, 0.5, 1.0]; 17];
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst, &src, mode, 1.0, 1.0, true);

            // Test extreme values in source
            let mut dst2 = [[0.5f32, 0.5, 0.5, 1.0]; 17];
            let src2 = [[extreme_val, extreme_val, extreme_val, extreme_val]; 17];
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, false);
            blit_rgba32f(&mut dst2, &src2, mode, 1.0, 1.0, true);

            // Test extreme opacity and fill
            let mut dst3 = [[0.5f32, 0.5, 0.5, 1.0]; 17];
            let src3 = [[0.8f32, 0.2, 0.4, 0.9]; 17];
            blit_rgba32f(&mut dst3, &src3, mode, extreme_val, extreme_val, false);
            blit_rgba32f(&mut dst3, &src3, mode, extreme_val, extreme_val, true);

            // Test gamma blitting with extreme gamma
            blit_rgba32f_gamma(&mut dst3, &src3, mode, 0.8, 0.8, extreme_val, false);
        }
    }
}

#[test]
fn test_adversarial_hdr_vs_sdr_clamping() {
    // Backdrop and source with values > 1.0
    let mut dst_sdr = [[1.5f32, 0.8, 2.0, 1.0]];
    let mut dst_hdr = [[1.5f32, 0.8, 2.0, 1.0]];
    let src = [[1.2f32, 0.5, 1.5, 1.0]];

    // LinearDodge (Add): 1.5 + 1.2 = 2.7
    blit_rgba32f(&mut dst_sdr, &src, BlendMode::LinearDodge, 1.0, 1.0, false);
    blit_rgba32f(&mut dst_hdr, &src, BlendMode::LinearDodge, 1.0, 1.0, true);

    // SDR should clamp at 1.0
    assert_eq!(dst_sdr[0][0], 1.0, "SDR should clamp channel to 1.0");

    // HDR should preserve sum > 1.0
    assert!(dst_hdr[0][0] > 2.6 && dst_hdr[0][0] < 2.8, "HDR should preserve unclamped channel value: got {}", dst_hdr[0][0]);

    // Divide in HDR vs SDR
    let mut div_sdr = [[0.9f32, 0.9, 0.9, 1.0]];
    let mut div_hdr = [[0.9f32, 0.9, 0.9, 1.0]];
    let div_src = [[0.3f32, 0.3, 0.3, 1.0]]; // 0.9 / 0.3 = 3.0

    blit_rgba32f(&mut div_sdr, &div_src, BlendMode::Divide, 1.0, 1.0, false);
    blit_rgba32f(&mut div_hdr, &div_src, BlendMode::Divide, 1.0, 1.0, true);

    assert_eq!(div_sdr[0][0], 1.0, "SDR Divide must clamp to 1.0");
    assert!((div_hdr[0][0] - 3.0).abs() < 1e-5, "HDR Divide must equal 3.0, got {}", div_hdr[0][0]);
}

// ---------------------------------------------------------------------------
// 4. Extreme Opacity and Fill Values
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_extreme_opacities_and_fills_all_modes() {
    let test_opacities = [-1000.0f32, -1.0, -0.001, -0.0, 0.0, 0.0001, 0.5, 1.0, 1.5, 1000.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];
    let test_fills = [-1000.0f32, -1.0, -0.001, -0.0, 0.0, 0.0001, 0.5, 1.0, 1.5, 1000.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN];

    for &op in &test_opacities {
        for &fill in &test_fills {
            for &mode in &BlendMode::LAYER_MODES {
                // RGBA8
                let mut dst8 = [128u8, 64, 200, 255];
                let src8 = [200u8, 100, 50, 255];
                let orig8 = dst8;
                blit_rgba8(&mut dst8, &src8, mode, op, fill);
                if op <= 0.0 {
                    assert_eq!(dst8, orig8, "Zero or negative opacity must be a no-op for RGBA8");
                }

                // RGBA16
                let mut dst16 = [32768u16, 16384, 50000, 65535];
                let src16 = [50000u16, 25000, 12500, 65535];
                let orig16 = dst16;
                blit_rgba16(&mut dst16, &src16, mode, op, fill);
                if op <= 0.0 {
                    assert_eq!(dst16, orig16, "Zero or negative opacity must be a no-op for RGBA16");
                }

                // RGBA32F
                let mut dst32 = [[0.5f32, 0.25, 0.75, 1.0]];
                let src32 = [[0.8f32, 0.4, 0.2, 1.0]];
                let orig32 = dst32;
                blit_rgba32f(&mut dst32, &src32, mode, op, fill, false);
                if op <= 0.0 {
                    assert_eq!(dst32, orig32, "Zero or negative opacity must be a no-op for RGBA32F");
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// 5. Fixed-Point Exclusion Underflow Stress Test
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_fixed_point_exclusion_exhaustive_u8() {
    // Test all 65,536 combinations of cb, cs for Exclusion
    for cb in 0..=255u8 {
        for cs in 0..=255u8 {
            let mut dst = [cb, cb, cb, 255];
            let src = [cs, cs, cs, 255];
            blit_rgba8(&mut dst, &src, BlendMode::Exclusion, 1.0, 1.0);
            let _ = dst[0];
        }
    }
}

#[test]
fn test_adversarial_fixed_point_exclusion_dense_u16() {
    let steps: Vec<u16> = (0..=65535u32).step_by(511).map(|x| x as u16).chain(std::iter::once(65535u16)).collect();

    for &cb in &steps {
        for &cs in &steps {
            let mut dst = [cb, cb, cb, 65535];
            let src = [cs, cs, cs, 65535];
            blit_rgba16(&mut dst, &src, BlendMode::Exclusion, 1.0, 1.0);
            let _ = dst[0];
        }
    }
}

// ---------------------------------------------------------------------------
// 6. Zero Allocations & Memory Safety Harness
// ---------------------------------------------------------------------------

#[test]
fn test_adversarial_stack_buffer_in_place_safety() {
    // Ensure all blit variants work safely directly on un-heapified stack buffers
    let mut stack_dst8 = [120u8; 64];
    let stack_src8 = [240u8; 64];
    blit_rgba8_opts(&mut stack_dst8, &stack_src8, BlendMode::HardMix, 0.9, 0.8, AlphaMode::Straight, false);
    blit_rgba8_clipped(&mut stack_dst8, &stack_src8, BlendMode::ColorBurn, 0.9, 0.8);
    blit_rgba8_premultiplied(&mut stack_dst8, &stack_src8, 0.9);

    let mut stack_dst16 = [12000u16; 64];
    let stack_src16 = [24000u16; 64];
    blit_rgba16_opts(&mut stack_dst16, &stack_src16, BlendMode::HardMix, 0.9, 0.8, AlphaMode::Straight, false);
    blit_rgba16_clipped(&mut stack_dst16, &stack_src16, BlendMode::ColorBurn, 0.9, 0.8);
    blit_rgba16_premultiplied(&mut stack_dst16, &stack_src16, 0.9);

    let mut stack_dst32 = [[0.3f32, 0.4, 0.5, 1.0]; 16];
    let stack_src32 = [[0.8f32, 0.7, 0.6, 1.0]; 16];
    blit_rgba32f_opts(&mut stack_dst32, &stack_src32, BlendMode::HardMix, 0.9, 0.8, 1.45, true, AlphaMode::Straight, false);
    blit_rgba32f_clipped(&mut stack_dst32, &stack_src32, BlendMode::ColorBurn, 0.9, 0.8, true);
    blit_rgba32f_premultiplied(&mut stack_dst32, &stack_src32, 0.9);
}
