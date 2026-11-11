use craft_blitter::float::blit_rgba32f_scalar;
use craft_blitter::simd::{composite_separable_rgb_span_serial, supports_lanes};
use craft_blitter::{
    AlphaMode, BlendMode, CompositeOp, blit_rgba8, blit_rgba8_atop, blit_rgba8_opts, blit_rgba8_premultiplied, blit_rgba16, blit_rgba16_atop,
    blit_rgba16_opts, blit_rgba16_premultiplied, blit_rgba32f, blit_rgba32f_atop, blit_rgba32f_opts, blit_rgba32f_premultiplied, composite,
    composite_atop,
};

fn assert_bits_eq_span(a: &[[f32; 4]], b: &[[f32; 4]], ctx: &str) {
    assert_eq!(a.len(), b.len(), "{ctx}: length mismatch");
    for (i, (pa, pb)) in a.iter().zip(b).enumerate() {
        for c in 0..4 {
            assert_eq!(pa[c].to_bits(), pb[c].to_bits(), "{ctx} pixel {i} ch {c}: {pa:?} != {pb:?}");
        }
    }
}

#[test]
fn simd_lanes_and_parallel_spans_are_bit_identical_to_scalar_fallback() {
    let lengths = [0usize, 1, 7, 8, 9, 15, 16, 17, 63, 64, 65, 4095, 4096, 4097, 5000];

    for &len in &lengths {
        let base_dst: Vec<[f32; 4]> = (0..len)
            .map(|i| match i % 5 {
                0 => [0.0, 0.0, 0.0, 0.0],
                1 => [0.25, 0.5, 0.75, 1.0],
                2 => [0.8, 0.1, 0.4, 0.4],
                3 => [1.5, 0.2, 2.25, 0.85],
                _ => [0.1, 0.9, 0.3, -0.0],
            })
            .collect();
        let src: Vec<[f32; 4]> = (0..len)
            .map(|i| match i % 6 {
                0 => [0.9, 0.2, 0.5, 0.0],
                1 => [0.6, 0.3, 0.8, 1.0],
                2 => [0.15, 0.85, 0.35, 0.55],
                3 => [2.0, 0.5, 1.25, 0.7],
                4 => [0.4, 0.4, 0.4, f32::NAN],
                _ => [0.7, 0.2, 0.1, -0.25],
            })
            .collect();

        for mode in BlendMode::ALL {
            for hdr in [false, true] {
                for opacity in [0.0f32, 0.37, 1.0] {
                    let mut scalar = base_dst.clone();
                    blit_rgba32f_scalar(&mut scalar, &src, mode, opacity, hdr, AlphaMode::Straight, CompositeOp::SourceOver);

                    if supports_lanes(mode) {
                        let mut simd_serial = base_dst.clone();
                        assert!(composite_separable_rgb_span_serial(&mut simd_serial, &src, mode, opacity, hdr,));
                        assert_bits_eq_span(&simd_serial, &scalar, &format!("serial {mode:?} len={len} hdr={hdr} op={opacity}"));
                    }

                    let mut public_out = base_dst.clone();
                    blit_rgba32f(&mut public_out, &src, mode, opacity, hdr);
                    assert_bits_eq_span(&public_out, &scalar, &format!("public {mode:?} len={len} hdr={hdr} op={opacity}"));
                }
            }
        }
    }
}

#[test]
fn source_atop_preserves_backdrop_alpha_across_depths() {
    // Straight RGBA8: transparent pixel stays transparent; translucent keeps alpha = 128.
    let mut dst8 = [0u8, 0, 0, 0, 50, 50, 50, 128, 100, 100, 100, 255];
    let src8 = [200u8, 200, 200, 255, 200, 200, 200, 255, 200, 200, 200, 255];
    blit_rgba8_atop(&mut dst8, &src8, BlendMode::Normal, 1.0);
    assert_eq!(&dst8[0..4], &[0, 0, 0, 0]);
    assert_eq!(&dst8[4..8], &[200, 200, 200, 128]);
    assert_eq!(&dst8[8..12], &[200, 200, 200, 255]);

    // Straight RGBA16:
    let mut dst16 = [0u16, 0, 0, 0, 10000, 10000, 10000, 32768, 20000, 20000, 20000, 65535];
    let src16 = [50000u16, 50000, 50000, 65535, 50000, 50000, 50000, 65535, 50000, 50000, 50000, 65535];
    blit_rgba16_atop(&mut dst16, &src16, BlendMode::Normal, 1.0);
    assert_eq!(&dst16[0..4], &[0, 0, 0, 0]);
    assert_eq!(&dst16[4..8], &[50000, 50000, 50000, 32768]);
    assert_eq!(&dst16[8..12], &[50000, 50000, 50000, 65535]);

    // Straight RGBA32F (`Co = (1 - as) * Cb + as * B(Cb, Cs)`, `ao = ab`):
    let mut dst32 = [[0.0f32, 0.0, 0.0, 0.0], [0.2, 0.4, 0.6, 0.5]];
    let src32 = [[1.0f32, 1.0, 1.0, 1.0], [0.5, 0.5, 0.5, 0.5]];
    blit_rgba32f_atop(&mut dst32, &src32, BlendMode::Multiply, 1.0, false);
    assert_eq!(dst32[0], [0.0, 0.0, 0.0, 0.0]);
    let want = composite_atop(BlendMode::Multiply, [0.2, 0.4, 0.6, 0.5], [0.5, 0.5, 0.5, 0.5], 1.0);
    for c in 0..4 {
        assert!((dst32[1][c] - want[c]).abs() < 1e-6);
    }
    assert_eq!(dst32[1][3], 0.5);
}

#[test]
fn premultiplied_source_over_and_source_atop_maintain_associated_alpha_invariant() {
    // SourceOver Normal:
    let mut dst8 = [50u8, 50, 50, 128];
    blit_rgba8_premultiplied(&mut dst8, &[100, 100, 100, 200], 1.0);
    assert!(dst8[0] <= dst8[3]);

    let mut dst16 = [10000u16, 10000, 10000, 32768];
    blit_rgba16_premultiplied(&mut dst16, &[20000, 20000, 20000, 50000], 1.0);
    assert!(dst16[0] <= dst16[3]);

    let mut dst32 = [[0.2f32, 0.2, 0.2, 0.5]];
    blit_rgba32f_premultiplied(&mut dst32, &[[0.4, 0.4, 0.4, 0.8]], 1.0);
    assert!((dst32[0][0] - 0.44).abs() < 1e-6);
    assert!((dst32[0][3] - 0.9).abs() < 1e-6);

    // Premultiplied SourceAtop Normal (`Dca' = Sca * Da + Dca * (1 - Sa)`, `Da' = Da`):
    // Opaque white [1, 1, 1, 1] atop half-transparent black [0, 0, 0, 0.5] -> [0.5, 0.5, 0.5, 0.5].
    let mut atop32 = [[0.0f32, 0.0, 0.0, 0.5]];
    blit_rgba32f_opts(&mut atop32, &[[1.0, 1.0, 1.0, 1.0]], BlendMode::Normal, 1.0, false, AlphaMode::Premultiplied, CompositeOp::SourceAtop);
    for c in 0..4 {
        assert!((atop32[0][c] - 0.5).abs() < 1e-6, "{atop32:?}");
    }

    let mut atop8 = [0u8, 0, 0, 128];
    blit_rgba8_opts(&mut atop8, &[255, 255, 255, 255], BlendMode::Normal, 1.0, AlphaMode::Premultiplied, CompositeOp::SourceAtop);
    assert_eq!(atop8, [128, 128, 128, 128]);

    let mut atop16 = [0u16, 0, 0, 32768];
    blit_rgba16_opts(&mut atop16, &[65535, 65535, 65535, 65535], BlendMode::Normal, 1.0, AlphaMode::Premultiplied, CompositeOp::SourceAtop);
    assert_eq!(atop16, [32768, 32768, 32768, 32768]);

    // Premultiplied Multiply SourceOver:
    let mut mul8 = [128u8, 128, 128, 128];
    blit_rgba8_opts(&mut mul8, &[64, 64, 64, 128], BlendMode::Multiply, 1.0, AlphaMode::Premultiplied, CompositeOp::SourceOver);
    assert!(mul8[0] <= mul8[3]);
    assert_eq!(mul8[3], 192);
}

#[test]
fn mismatched_slices_and_nan_inputs_do_not_panic_or_corrupt_backdrop() {
    // Unaligned & mismatched byte lengths: 10 pixels dst + 2 trailing bytes, 6 pixels src + 3 bytes.
    let mut dst = vec![7u8; 42];
    let src = vec![250u8; 27];
    blit_rgba8(&mut dst, &src, BlendMode::Normal, 1.0);
    assert!(dst[..24].iter().all(|&b| b == 250));
    assert!(dst[24..].iter().all(|&b| b == 7));

    // Empty slices:
    blit_rgba8(&mut [], &[1, 2, 3, 4], BlendMode::Normal, 1.0);
    blit_rgba16(&mut [], &[1, 2, 3, 4], BlendMode::Normal, 1.0);
    blit_rgba32f(&mut [], &[[1.0; 4]], BlendMode::Normal, 1.0, false);

    // NaN opacity is a no-op across all depths:
    let mut d8 = [10u8, 20, 30, 255];
    blit_rgba8(&mut d8, &[200, 200, 200, 255], BlendMode::Normal, f32::NAN);
    assert_eq!(d8, [10, 20, 30, 255]);

    let mut d16 = [100u16, 200, 300, 65535];
    blit_rgba16(&mut d16, &[5000, 5000, 5000, 65535], BlendMode::Normal, f32::NAN);
    assert_eq!(d16, [100, 200, 300, 65535]);

    let mut d32 = [[0.25f32, 0.5, 0.75, 1.0]];
    blit_rgba32f(&mut d32, &[[0.9, 0.9, 0.9, 1.0]], BlendMode::Normal, f32::NAN, false);
    assert_eq!(d32[0], [0.25, 0.5, 0.75, 1.0]);

    // NaN source alpha is treated as transparent in composite():
    let bg = [0.3f32, 0.4, 0.5, 1.0];
    assert_eq!(composite(BlendMode::Multiply, bg, [0.9, 0.9, 0.9, f32::NAN], 1.0), bg);
}
