use craft_blitter::{
    BlendMode, blit_rgba8, blit_rgba8_clipped, blit_rgba8_premultiplied, blit_rgba16, blit_rgba16_clipped, blit_rgba16_premultiplied, blit_rgba32f,
    blit_rgba32f_clipped, blit_rgba32f_gamma, blit_rgba32f_premultiplied,
};

#[test]
fn test_mismatched_slice_lengths_no_panic() {
    // 10 pixels destination, 6 pixels source
    let mut dst = vec![0u8; 40];
    let src = vec![255u8; 24];
    blit_rgba8(&mut dst, &src, BlendMode::Normal, 1.0, 1.0);

    // First 6 pixels blitted
    for &val in &dst[..24] {
        assert_eq!(val, 255);
    }
    // Remaining 4 pixels untouched
    for &val in &dst[24..40] {
        assert_eq!(val, 0);
    }

    // 5 pixels destination, 10 pixels source
    let mut dst2 = vec![100u8; 20];
    let mut src2 = Vec::new();
    for _ in 0..10 {
        src2.extend_from_slice(&[200, 200, 200, 255]);
    }
    blit_rgba8(&mut dst2, &src2, BlendMode::Normal, 1.0, 1.0);
    for chunk in dst2.as_chunks::<4>().0 {
        assert_eq!(chunk, &[200, 200, 200, 255]);
    }
}

#[test]
fn test_clipping_group_isolation_rgba8() {
    // Base layer: pixel 0 transparent, pixel 1 alpha 128, pixel 2 alpha 255
    let mut dst = [
        0u8, 0, 0, 0, // Transparent pixel
        50, 50, 50, 128, // Translucent pixel
        100, 100, 100, 255, // Opaque pixel
    ];
    let src = [200u8, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255];

    blit_rgba8_clipped(&mut dst, &src, BlendMode::Normal, 1.0, 1.0);

    // Pixel 0 must remain completely untouched [0, 0, 0, 0]
    assert_eq!(&dst[0..4], &[0, 0, 0, 0]);

    // Pixel 1: alpha must remain locked to 128
    assert_eq!(dst[7], 128);

    // Pixel 2: alpha must remain locked to 255
    assert_eq!(dst[11], 255);
}

#[test]
fn test_clipping_group_isolation_rgba16() {
    let mut dst = [0u16, 0, 0, 0, 10000, 10000, 10000, 32768, 20000, 20000, 20000, 65535];
    let src = [50000u16, 50000, 50000, 65535, 50000, 50000, 50000, 65535, 50000, 50000, 50000, 65535];
    blit_rgba16_clipped(&mut dst, &src, BlendMode::Normal, 1.0, 1.0);
    assert_eq!(&dst[0..4], &[0, 0, 0, 0]);
    assert_eq!(dst[7], 32768);
    assert_eq!(dst[11], 65535);

    let mut dst_plain = [10000u16, 10000, 10000, 65535];
    blit_rgba16(&mut dst_plain, &src[0..4], BlendMode::Normal, 1.0, 1.0);
    assert_eq!(dst_plain, src[0..4]);
}

#[test]
fn test_clipping_group_isolation_rgba32f() {
    let mut dst = [[0.0f32, 0.0, 0.0, 0.0], [0.2, 0.2, 0.2, 0.5], [0.5, 0.5, 0.5, 1.0]];
    let src = [[1.0f32, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0], [1.0, 1.0, 1.0, 1.0]];

    blit_rgba32f_clipped(&mut dst, &src, BlendMode::Normal, 1.0, 1.0, false);

    assert_eq!(dst[0], [0.0, 0.0, 0.0, 0.0]);
    assert_eq!(dst[1][3], 0.5);
    assert_eq!(dst[2][3], 1.0);
}

#[test]
fn test_premultiplied_alpha_paths() {
    // RGBA8 premultiplied
    let mut dst8 = [50u8, 50, 50, 128];
    let src8 = [100u8, 100, 100, 200];
    blit_rgba8_premultiplied(&mut dst8, &src8, 1.0);
    assert!(dst8[3] >= 200);

    // RGBA16 premultiplied
    let mut dst16 = [10000u16, 10000, 10000, 32768];
    let src16 = [20000u16, 20000, 20000, 50000];
    blit_rgba16_premultiplied(&mut dst16, &src16, 1.0);
    assert!(dst16[3] >= 50000);

    // RGBA32F premultiplied
    let mut dst32 = [[0.2f32, 0.2, 0.2, 0.5]];
    let src32 = [[0.4f32, 0.4, 0.4, 0.8]];
    blit_rgba32f_premultiplied(&mut dst32, &src32, 1.0);
    assert!((dst32[0][3] - (0.8 + 0.5 * 0.2)).abs() < 1e-6);
}

#[test]
fn test_various_span_sizes_simd_and_tail() {
    for len in [1, 7, 8, 9, 15, 16, 17, 33, 100, 4096, 5000] {
        let mut dst = vec![[0.2f32, 0.4, 0.6, 1.0]; len];
        let src = vec![[0.8f32, 0.1, 0.3, 0.5]; len];

        blit_rgba32f(&mut dst, &src, BlendMode::Normal, 1.0, 1.0, false);

        // Every pixel should be identical
        for (i, px) in dst.iter().enumerate() {
            assert!((px[0] - dst[0][0]).abs() < 1e-6, "pixel {i} differs");
            assert!((px[1] - dst[0][1]).abs() < 1e-6, "pixel {i} differs");
            assert!((px[2] - dst[0][2]).abs() < 1e-6, "pixel {i} differs");
            assert!((px[3] - dst[0][3]).abs() < 1e-6, "pixel {i} differs");
        }
    }
}

#[test]
fn test_rgba32f_gamma_text_blit() {
    let mut dst = [[129.0 / 255.0, 129.0 / 255.0, 129.0 / 255.0, 1.0]];
    let src = [[0.0f32, 0.2, 0.6, 0.686]];
    blit_rgba32f_gamma(&mut dst, &src, BlendMode::Normal, 1.0, 1.0, 1.45, false);

    assert!((dst[0][0] * 255.0 - 57.0).abs() < 1.0);
    assert!((dst[0][1] * 255.0 - 80.0).abs() < 1.0);
    assert!((dst[0][2] * 255.0 - 146.0).abs() < 1.0);
}

#[test]
fn test_premultiplied_non_normal_and_fill_modes() {
    use craft_blitter::{AlphaMode, blit_rgba8_opts, blit_rgba16_opts, blit_rgba32f_opts};

    // 1. Premultiplied Multiply (RGBA8)
    let mut dst8 = [128u8, 128, 128, 128]; // straight color: [255, 255, 255]
    let src8 = [64u8, 64, 64, 128]; // straight color: [128, 128, 128]
    blit_rgba8_opts(&mut dst8, &src8, BlendMode::Multiply, 1.0, 1.0, AlphaMode::Premultiplied, false);
    // Multiply of 255 and 128 is 128. Output alpha = 128 + 128*(1 - 0.5) = 192.
    // Premultiplied RGB channel should be <= Alpha channel
    assert!(dst8[0] <= dst8[3]);
    assert!(dst8[1] <= dst8[3]);
    assert!(dst8[2] <= dst8[3]);
    assert_eq!(dst8[3], 192);

    // 2. Premultiplied Normal with partial fill (RGBA8)
    let mut dst_norm = [0u8, 0, 0, 0];
    let src_norm = [200u8, 200, 200, 200];
    blit_rgba8_opts(&mut dst_norm, &src_norm, BlendMode::Normal, 1.0, 0.5, AlphaMode::Premultiplied, false);
    assert_eq!(dst_norm[3], 100);
    assert_eq!(dst_norm[0], 100);

    // 3. Premultiplied non-normal (RGBA16)
    let mut dst16 = [32768u16, 32768, 32768, 32768];
    let src16 = [16384u16, 16384, 16384, 32768];
    blit_rgba16_opts(&mut dst16, &src16, BlendMode::Multiply, 1.0, 1.0, AlphaMode::Premultiplied, false);
    assert!(dst16[0] <= dst16[3]);
    assert!(dst16[1] <= dst16[3]);
    assert!(dst16[2] <= dst16[3]);

    // 4. Premultiplied non-normal (RGBA32F)
    let mut dst32 = [[0.5f32, 0.5, 0.5, 0.5]];
    let src32 = [[0.25f32, 0.25, 0.25, 0.5]];
    blit_rgba32f_opts(&mut dst32, &src32, BlendMode::Multiply, 1.0, 1.0, 1.0, false, AlphaMode::Premultiplied, false);
    assert!(dst32[0][0] <= dst32[0][3]);
    assert!((dst32[0][3] - 0.75).abs() < 1e-5);
}

#[test]
fn test_special_eight_clamping_robustness() {
    use craft_blitter::special::{composite_special_eight_u8, composite_special_eight_u16};

    // op8 / fill8 larger than 255 should not panic with overflow
    let bg8 = [100u8, 150, 200, 255];
    let src8 = [50u8, 120, 180, 255];
    let res8 = composite_special_eight_u8(BlendMode::VividLight, bg8, src8, 999, 888);
    assert_eq!(res8[3], 255);

    // op16 / fill16 larger than 65535 should not panic with overflow
    let bg16 = [25000u16, 35000, 45000, 65535];
    let src16 = [15000u16, 25000, 35000, 65535];
    let res16 = composite_special_eight_u16(BlendMode::VividLight, bg16, src16, 999_999, 888_888);
    assert_eq!(res16[3], 65535);
}

#[test]
fn test_nan_opacity_and_fill_sanitization() {
    use craft_blitter::{AlphaMode, blit_rgba32f_opts};

    let mut dst = [[0.5f32, 0.5, 0.5, 1.0]];
    let src = [[0.8f32, 0.8, 0.8, 1.0]];

    // NaN opacity should be treated as 0 (no-op)
    blit_rgba32f_opts(&mut dst, &src, BlendMode::Normal, f32::NAN, 1.0, 1.0, false, AlphaMode::Straight, false);
    assert_eq!(dst[0], [0.5, 0.5, 0.5, 1.0]);

    // NaN fill should be treated as 1.0
    blit_rgba32f_opts(&mut dst, &src, BlendMode::Normal, 1.0, f32::NAN, 1.0, false, AlphaMode::Straight, false);
    assert_eq!(dst[0], [0.8, 0.8, 0.8, 1.0]);
}
