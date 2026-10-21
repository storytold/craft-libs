//! Regression gate test suite for all 7 defects reported by Reviewer 2 & Challenger 2,
//! plus SIMD chunk vectorization expansion verification.

use craft_blitter::blend::{blend_channel, vivid_light_ps};
use craft_blitter::fixed::{blend_channel_u8, blend_channel_u16, hard_light_u8, hard_light_u16, vivid_light_u8};
use craft_blitter::float::composite_f32;
use craft_blitter::special::{composite_special_eight_u8, composite_special_eight_u16};
use craft_blitter::{AlphaMode, BlendMode, blit_rgba8_clipped, blit_rgba8_opts, blit_rgba16_clipped, blit_rgba16_opts, blit_rgba32f};

/// DEFECT 1 REGRESSION: Premultiplied Alpha Opacity Scaling Omission in fixed.rs
#[test]
fn test_defect1_premultiplied_opacity_scaling_rgba8_rgba16() {
    // 1. RGBA8: Opacity 0.5 over transparent backdrop
    let mut dst8 = [0u8, 0, 0, 0];
    let src8 = [200u8, 200, 200, 200];
    blit_rgba8_opts(&mut dst8, &src8, BlendMode::Normal, 0.5, 1.0, AlphaMode::Premultiplied, false);
    // At opacity 0.5, effective alpha = 100, and premultiplied color must be scaled by 0.5 -> 100
    assert_eq!(dst8, [100, 100, 100, 100], "Defect 1 RGBA8: source color not scaled by opacity");

    // Invariant check: C <= A across all opacities
    for op_int in 0..=10 {
        let op = op_int as f32 / 10.0;
        let mut d = [0u8, 0, 0, 0];
        blit_rgba8_opts(&mut d, &src8, BlendMode::Normal, op, 1.0, AlphaMode::Premultiplied, false);
        assert!(d[0] <= d[3], "RGBA8 Premultiplied invariant C <= A violated at opacity {op}: {d:?}");
        assert!(d[1] <= d[3], "RGBA8 Premultiplied invariant C <= A violated at opacity {op}: {d:?}");
        assert!(d[2] <= d[3], "RGBA8 Premultiplied invariant C <= A violated at opacity {op}: {d:?}");
    }

    // 2. RGBA16: Opacity 0.5 over transparent backdrop
    let mut dst16 = [0u16, 0, 0, 0];
    let src16 = [40000u16, 40000, 40000, 40000];
    blit_rgba16_opts(&mut dst16, &src16, BlendMode::Normal, 0.5, 1.0, AlphaMode::Premultiplied, false);
    assert_eq!(dst16, [20000, 20000, 20000, 20000], "Defect 1 RGBA16: source color not scaled by opacity");

    for op_int in 0..=10 {
        let op = op_int as f32 / 10.0;
        let mut d = [0u16, 0, 0, 0];
        blit_rgba16_opts(&mut d, &src16, BlendMode::Normal, op, 1.0, AlphaMode::Premultiplied, false);
        assert!(d[0] <= d[3], "RGBA16 Premultiplied invariant C <= A violated at opacity {op}: {d:?}");
    }
}

/// DEFECT 2 REGRESSION: Clipping Mask Translucent Base Divergence in fixed.rs
#[test]
fn test_defect2_clipping_mask_translucent_base_rgba8_rgba16() {
    // 1. RGBA8 Multiply over translucent base [100, 100, 100, 128]
    let mut dst8 = [100u8, 100, 100, 128];
    let src8 = [200u8, 200, 200, 255];
    blit_rgba8_clipped(&mut dst8, &src8, BlendMode::Multiply, 1.0, 1.0);
    // Photoshop & float.rs reference: base treated as opaque during blend (200 * 100 / 255 = 78), base alpha retained (128)
    assert_eq!(dst8, [78, 78, 78, 128], "Defect 2 RGBA8: unblended source color bled onto translucent base");

    // 2. RGBA8 Screen over translucent base
    let mut dst_screen = [100u8, 100, 100, 128];
    blit_rgba8_clipped(&mut dst_screen, &src8, BlendMode::Screen, 1.0, 1.0);
    assert_eq!(dst_screen, [222, 222, 222, 128], "Defect 2 RGBA8 Screen: base not treated as opaque");

    // 3. Fully transparent base must remain completely untouched
    let mut dst_trans = [0u8, 0, 0, 0];
    blit_rgba8_clipped(&mut dst_trans, &src8, BlendMode::Multiply, 1.0, 1.0);
    assert_eq!(dst_trans, [0, 0, 0, 0], "Transparent base in clipping mask was modified");

    // 4. RGBA16 Multiply over translucent base
    let mut dst16 = [25700u16, 25700, 25700, 32768];
    let src16 = [51400u16, 51400, 51400, 65535];
    blit_rgba16_clipped(&mut dst16, &src16, BlendMode::Multiply, 1.0, 1.0);
    assert_eq!(dst16[3], 32768, "RGBA16 clipping alpha not preserved");
    let exp16 = craft_blitter::fixed::div65535(25700 * 51400) as u16;
    assert!((dst16[0] as i32 - exp16 as i32).abs() <= 1, "Defect 2 RGBA16 Multiply color mismatch");
}

/// DEFECT 3 REGRESSION: hard_light_u8 & Overlay Wrap-Around Modulo 256 Overflow
#[test]
fn test_defect3_hard_light_and_overlay_u8_overflow() {
    let got_hl = hard_light_u8(255, 128);
    assert_eq!(got_hl, 255, "Defect 3: hard_light_u8(255, 128) wrapped to 0 instead of 255");

    let got_overlay = blend_channel_u8(BlendMode::Overlay, 128, 255);
    assert_eq!(got_overlay, 255, "Defect 3: Overlay(128, 255) wrapped to 0 instead of 255");

    let got_hl_mode = blend_channel_u8(BlendMode::HardLight, 255, 128);
    assert_eq!(got_hl_mode, 255, "Defect 3: HardLight(255, 128) wrapped to 0 instead of 255");

    // Monotonicity check across all backdrop values at cs = 128
    let mut prev = 0u8;
    for cb in 0..=255u8 {
        let val = hard_light_u8(cb, 128);
        assert!(val >= prev, "Non-monotonic wrap in hard_light_u8 at cb={cb}: {val} < {prev}");
        prev = val;
    }
}

/// DEFECT 4 REGRESSION: hard_light_u16 & Overlay Wrap-Around Modulo 65536 Overflow
#[test]
fn test_defect4_hard_light_and_overlay_u16_overflow() {
    let got_hl16 = hard_light_u16(65535, 32768);
    assert_eq!(got_hl16, 65535, "Defect 4: hard_light_u16(65535, 32768) wrapped to 0 instead of 65535");

    let got_overlay16 = blend_channel_u16(BlendMode::Overlay, 32768, 65535);
    assert_eq!(got_overlay16, 65535, "Defect 4: Overlay u16 wrapped to 0 instead of 65535");

    let got_hl_mode16 = blend_channel_u16(BlendMode::HardLight, 65535, 32768);
    assert_eq!(got_hl_mode16, 65535, "Defect 4: HardLight u16 wrapped to 0 instead of 65535");
}

/// DEFECT 5 REGRESSION: Vivid Light Special Eight Integer Truncation Inversion
#[test]
fn test_defect5_vivid_light_special_eight_fill_truncation() {
    // Pure black over pure white at 100% fill and 100% opacity must yield pure black
    let bg = [255u8, 255, 255, 255];
    let fg = [0u8, 0, 0, 255];
    let got = composite_special_eight_u8(BlendMode::VividLight, bg, fg, 255, 255);
    assert_eq!(got, [0, 0, 0, 255], "Defect 5: Black over white in Vivid Light inverted to white!");

    // Pure white over pure black at 100% fill
    let got_white = composite_special_eight_u8(BlendMode::VividLight, fg, bg, 255, 255);
    assert_eq!(got_white, [255, 255, 255, 255], "White over black in Vivid Light failed");

    // 16-bit Special Eight Vivid Light parity
    let bg16 = [65535u16, 65535, 65535, 65535];
    let fg16 = [0u16, 0, 0, 65535];
    let got16 = composite_special_eight_u16(BlendMode::VividLight, bg16, fg16, 65535, 65535);
    assert_eq!(got16, [0, 0, 0, 65535], "Defect 5 RGBA16: Black over white inverted");

    // Parity with float reference across fills [0.0, 0.25, 0.5, 0.75, 1.0]
    for &fill in &[0.0f32, 0.25, 0.5, 0.75, 1.0] {
        let fill8 = (fill * 255.0 + 0.5) as u32;
        let res = composite_special_eight_u8(BlendMode::VividLight, bg, fg, 255, fill8);
        let exp_f =
            craft_blitter::float::composite_fill_f32(BlendMode::VividLight, [1.0, 1.0, 1.0, 1.0], [0.0, 0.0, 0.0, 1.0], 1.0, fill, 1.0, false);
        let exp = (exp_f[0].clamp(0.0, 1.0) * 255.0).round() as u8;
        let diff = (res[0] as i32 - exp as i32).abs();
        assert!(diff <= 1, "Vivid Light mismatch against float reference at fill={fill}: got {res:?}, exp={exp}");
    }
}

/// DEFECT 6 REGRESSION: vivid_light_u8 Divisor Accuracy
#[test]
fn test_defect6_vivid_light_u8_divisor_accuracy() {
    let got = vivid_light_u8(2, 254);
    assert_eq!(got, 255, "Defect 6: vivid_light_u8(2, 254) returned 170 instead of 255");

    let mut off_by_more = 0usize;
    let mut max_diff = 0i32;

    for cb in 0..=255u8 {
        for cs in 0..=255u8 {
            let got_val = vivid_light_u8(cb, cs);
            let cb_f = f32::from(cb) / 255.0;
            let cs_f = f32::from(cs) / 255.0;
            let exp_f = vivid_light_ps(cb_f, cs_f);
            let exp_val = (exp_f.clamp(0.0, 1.0) * 255.0).round() as u8;
            let diff = (got_val as i32 - exp_val as i32).abs();
            if diff > max_diff {
                max_diff = diff;
            }
            if diff > 1 {
                off_by_more += 1;
            }
        }
    }

    assert_eq!(off_by_more, 0, "Defect 6: {off_by_more} pairs deviated by > 1 level (max diff: {max_diff})");
    assert!(max_diff <= 1, "Vivid Light max difference {max_diff} exceeds 1 LSB");
}

/// DEFECT 7 REGRESSION: Exclusion Rounding Precision in fixed.rs
#[test]
fn test_defect7_exclusion_rounding_precision_u8_u16() {
    let mut exact_matches = 0usize;
    let mut max_diff = 0i32;

    for cb in 0..=255u8 {
        for cs in 0..=255u8 {
            let got = blend_channel_u8(BlendMode::Exclusion, cb, cs);
            let cb_f = f32::from(cb) / 255.0;
            let cs_f = f32::from(cs) / 255.0;
            let exp_f = blend_channel(BlendMode::Exclusion, cb_f, cs_f, 1.0);
            let exp = (exp_f.clamp(0.0, 1.0) * 255.0).round() as u8;
            let diff = (got as i32 - exp as i32).abs();
            if diff == 0 {
                exact_matches += 1;
            }
            if diff > max_diff {
                max_diff = diff;
            }
        }
    }

    assert!(max_diff <= 1, "Exclusion max diff exceeds 1 LSB");
    let match_pct = (exact_matches as f64 / 65536.0) * 100.0;
    // Before fix: 50.62% exact match. With div255(2 * cb * cs): >= 99.5% exact match.
    assert!(match_pct >= 99.5, "Exclusion precision too low: only {match_pct:.2}% exact matches ({exact_matches}/65536)");
}

/// SIMD EXPANSION VERIFICATION: Separable Modes Bit-Exact Equivalence with Scalar Reference
#[test]
fn test_simd_separable_chunk_vectorization_expansion() {
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

    let opacities = [0.0f32, 0.33, 0.5, 0.8, 1.0];

    for mode in separable_modes {
        for &op in &opacities {
            // Buffer size covering multiple 8-pixel chunks plus remainder tail (27 pixels)
            let len = 27;
            let mut dst_simd = vec![[0.2f32, 0.4, 0.6, 0.8]; len];
            let mut dst_scalar = dst_simd.clone();
            let src = vec![[0.7f32, 0.5, 0.3, 0.9]; len];

            // Run SIMD blit
            blit_rgba32f(&mut dst_simd, &src, mode, op, 1.0, false);

            // Run Scalar Reference
            for (d, s) in dst_scalar.iter_mut().zip(&src) {
                *d = composite_f32(mode, *d, *s, op, false);
            }

            for i in 0..len {
                for c in 0..4 {
                    let diff = (dst_simd[i][c] - dst_scalar[i][c]).abs();
                    assert!(
                        diff < 1e-6,
                        "SIMD divergence for mode {mode:?} at op {op}, pixel {i}, channel {c}: SIMD={}, Scalar={}, diff={diff}",
                        dst_simd[i][c],
                        dst_scalar[i][c]
                    );
                }
            }
        }
    }
}

/// SIMD BOUNDARY & CHUNK SIZES VERIFICATION
#[test]
fn test_simd_chunk_boundaries_and_tail_lengths() {
    let test_lengths = [1, 7, 8, 9, 15, 16, 17, 31, 32, 33, 101, 4095, 4096, 4097];

    for &len in &test_lengths {
        let mut dst = vec![[0.3f32, 0.5, 0.7, 1.0]; len];
        let src = vec![[0.8f32, 0.2, 0.4, 0.6]; len];

        for mode in [BlendMode::Multiply, BlendMode::Screen, BlendMode::Difference] {
            blit_rgba32f(&mut dst, &src, mode, 0.75, 1.0, false);
            // Verify all pixels are valid, finite, and within [0, 1]
            for px in &dst {
                for &ch in px {
                    assert!(ch.is_finite());
                    assert!((0.0..=1.0).contains(&ch));
                }
            }
        }
    }
}
