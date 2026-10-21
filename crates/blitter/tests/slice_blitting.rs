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
