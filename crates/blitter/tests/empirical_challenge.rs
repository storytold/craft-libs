use craft_blitter::blend::blend_channel;
use craft_blitter::fixed::blend_channel_u8;
use craft_blitter::float::composite_fill_f32;
use craft_blitter::special::composite_special_eight_u8;
use craft_blitter::{BlendMode, blit_rgba8, blit_rgba32f};

/// 1. EMPIRICAL TEST: Integer Fixed-Point vs IEEE-754 Reference Rounding Parity
#[test]
fn test_empirical_fixed_point_vs_ieee754_all_separable_modes() {
    let separable_modes = [
        BlendMode::Normal,
        BlendMode::Darken,
        BlendMode::Multiply,
        BlendMode::ColorBurn,
        BlendMode::LinearBurn,
        BlendMode::Lighten,
        BlendMode::Screen,
        BlendMode::ColorDodge,
        BlendMode::LinearDodge,
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
    ];

    println!("\n=== EXHAUSTIVE 65,536 (Cb, Cs) PAIRS TEST PER SEPARABLE MODE ===");

    for mode in separable_modes {
        let mut exact_matches = 0usize;
        let mut off_by_1 = 0usize;
        let mut off_by_more = 0usize;
        let mut max_diff = 0i32;
        let mut sample_mismatch = None;

        for cb_int in 0..=255u8 {
            for cs_int in 0..=255u8 {
                let got = blend_channel_u8(mode, cb_int, cs_int);

                let cb_f = f32::from(cb_int) / 255.0;
                let cs_f = f32::from(cs_int) / 255.0;
                let ref_f = blend_channel(mode, cb_f, cs_f, 1.0);
                let expected = (ref_f.clamp(0.0, 1.0) * 255.0).round() as u8;

                let diff = (got as i32 - expected as i32).abs();
                if diff == 0 {
                    exact_matches += 1;
                } else if diff == 1 {
                    off_by_1 += 1;
                    if max_diff < 1 {
                        max_diff = 1;
                    }
                    if sample_mismatch.is_none() {
                        sample_mismatch = Some((cb_int, cs_int, got, expected, diff));
                    }
                } else {
                    off_by_more += 1;
                    if diff > max_diff {
                        max_diff = diff;
                        sample_mismatch = Some((cb_int, cs_int, got, expected, diff));
                    }
                }
            }
        }

        let total = 65536;
        let match_pct = (exact_matches as f64 / total as f64) * 100.0;
        println!("{mode:?}: {match_pct:.2}% exact ({exact_matches}/{total}), off-by-1: {off_by_1}, off-by->1: {off_by_more}, max_diff: {max_diff}");
        if let Some((cb, cs, got, exp, diff)) = sample_mismatch {
            println!("   -> Sample divergence for {mode:?}: Cb={cb}, Cs={cs} => got {got}, expected {exp} (diff: {diff})");
        }
    }
}

/// 2. EMPIRICAL TEST: Special Eight Fill Dynamics vs IEEE-754 Float Reference
#[test]
fn test_empirical_special_eight_fill_dynamics_parity() {
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

    println!("\n=== SPECIAL EIGHT FILL DYNAMICS PARITY TEST ===");

    let fills = [0.0f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
    let opacities = [0.25f32, 0.5, 0.8, 1.0];

    let test_colors: [[u8; 4]; 8] = [
        [200, 150, 100, 255], // Photoshop test bg
        [100, 180, 220, 255], // Photoshop test fg
        [60, 120, 180, 255],
        [210, 90, 30, 255],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [128, 128, 128, 200], // Semi-transparent
        [50, 100, 150, 128],  // Semi-transparent
    ];

    for mode in special_modes {
        let mut max_diff = 0i32;
        let mut total_compared = 0usize;
        let mut exact_matches = 0usize;
        let mut diff_dist = [0usize; 256];

        for &bg in &test_colors {
            for &fg in &test_colors {
                for &fill in &fills {
                    for &op in &opacities {
                        total_compared += 1;
                        let op8 = (op * 255.0 + 0.5) as u32;
                        let fill8 = (fill * 255.0 + 0.5) as u32;

                        let got_u8 = composite_special_eight_u8(mode, bg, fg, op8, fill8);

                        let bg_f = [f32::from(bg[0]) / 255.0, f32::from(bg[1]) / 255.0, f32::from(bg[2]) / 255.0, f32::from(bg[3]) / 255.0];
                        let fg_f = [f32::from(fg[0]) / 255.0, f32::from(fg[1]) / 255.0, f32::from(fg[2]) / 255.0, f32::from(fg[3]) / 255.0];
                        let exp_f = composite_fill_f32(mode, bg_f, fg_f, op, fill, 1.0, false);
                        let exp_u8 = [
                            (exp_f[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (exp_f[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (exp_f[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                            (exp_f[3].clamp(0.0, 1.0) * 255.0).round() as u8,
                        ];

                        let mut d = 0i32;
                        for i in 0..4 {
                            let diff = (got_u8[i] as i32 - exp_u8[i] as i32).abs();
                            d = d.max(diff);
                        }
                        diff_dist[d as usize] += 1;
                        if d == 0 {
                            exact_matches += 1;
                        }
                        if d > max_diff {
                            max_diff = d;
                        }
                    }
                }
            }
        }

        let match_pct = (exact_matches as f64 / total_compared as f64) * 100.0;
        println!(
            "{mode:?}: {match_pct:.2}% exact ({exact_matches}/{total_compared}), max_diff: {max_diff}, <=1: {}, <=2: {}",
            diff_dist[0] + diff_dist[1],
            diff_dist[0] + diff_dist[1] + diff_dist[2]
        );
    }
}

/// 3. EMPIRICAL TEST: Slice Blitting with Mismatches, Subnormals, and Extreme Values
#[test]
fn test_empirical_slice_robustness_and_extremes() {
    let mut empty_dst: [u8; 0] = [];
    let empty_src: [u8; 0] = [];
    blit_rgba8(&mut empty_dst, &empty_src, BlendMode::Normal, 1.0, 1.0);

    let mut dst = [128u8; 17];
    let src = [200u8; 11];
    blit_rgba8(&mut dst, &src, BlendMode::Multiply, 0.8, 0.7);
    assert_eq!(dst[8..17], [128u8; 9]);

    let subnormal = f32::from_bits(1);
    let dst32 = [[f32::NAN, f32::INFINITY, f32::NEG_INFINITY, 1.0], [subnormal, -0.0, 1e-30, 0.5], [1.0, 1.0, 1.0, 1.0]];
    let src32 = [[0.5, 0.5, 0.5, 0.5], [f32::NAN, 1.0, -1.0, 1.0], [subnormal, subnormal, subnormal, subnormal]];

    for mode in BlendMode::LAYER_MODES {
        let mut d = dst32;
        blit_rgba32f(&mut d, &src32, mode, 1.0, 1.0, false);
        blit_rgba32f(&mut d, &src32, mode, 1.0, 1.0, true);
    }
}

/// 4. EMPIRICAL TEST: HardLight & Overlay U8/U16 Overflow Bug Reproduction
#[test]
fn test_empirical_hard_light_u8_u16_overflow() {
    // In HardLight u8, cb = 255, cs = 128:
    // Expected IEEE-754: 255
    // Current fixed implementation produces 256 as u8 -> 0!
    let got_u8 = craft_blitter::fixed::hard_light_u8(255, 128);
    println!("hard_light_u8(255, 128): got {got_u8}, expected 255");

    let got_overlay_u8 = craft_blitter::fixed::blend_channel_u8(BlendMode::Overlay, 128, 255);
    println!("blend_channel_u8(Overlay, 128, 255): got {got_overlay_u8}, expected 255");

    // In HardLight u16, cb = 65535, cs = 32768:
    // Expected IEEE-754: 65535
    // Current fixed implementation produces 65536 as u16 -> 0!
    let got_u16 = craft_blitter::fixed::hard_light_u16(65535, 32768);
    println!("hard_light_u16(65535, 32768): got {got_u16}, expected 65535");

    let got_overlay_u16 = craft_blitter::fixed::blend_channel_u16(BlendMode::Overlay, 32768, 65535);
    println!("blend_channel_u16(Overlay, 32768, 65535): got {got_overlay_u16}, expected 65535");

    // Print vivid light discrepancy
    let got_vl = craft_blitter::fixed::vivid_light_u8(2, 254);
    let exp_vl = (craft_blitter::blend::vivid_light_ps(2.0 / 255.0, 254.0 / 255.0) * 255.0).round() as u8;
    println!("vivid_light_u8(2, 254): got {got_vl}, expected {exp_vl} (diff: {})", (got_vl as i32 - exp_vl as i32).abs());
}

/// 5. EMPIRICAL TEST: VividLight Special Eight Max Diff 255 Root Cause Analysis
#[test]
fn test_empirical_vivid_light_special_eight_divergence() {
    let fills = [0.0f32, 0.1, 0.25, 0.5, 0.75, 0.9, 1.0];
    let opacities = [0.25f32, 0.5, 0.8, 1.0];
    let test_colors: [[u8; 4]; 8] = [
        [200, 150, 100, 255],
        [100, 180, 220, 255],
        [60, 120, 180, 255],
        [210, 90, 30, 255],
        [0, 0, 0, 255],
        [255, 255, 255, 255],
        [128, 128, 128, 200],
        [50, 100, 150, 128],
    ];

    let mut worst = None;
    let mut max_diff = 0;

    for &bg in &test_colors {
        for &fg in &test_colors {
            for &fill in &fills {
                for &op in &opacities {
                    let op8 = (op * 255.0 + 0.5) as u32;
                    let fill8 = (fill * 255.0 + 0.5) as u32;
                    let got = composite_special_eight_u8(BlendMode::VividLight, bg, fg, op8, fill8);

                    let bg_f = [bg[0] as f32 / 255.0, bg[1] as f32 / 255.0, bg[2] as f32 / 255.0, bg[3] as f32 / 255.0];
                    let fg_f = [fg[0] as f32 / 255.0, fg[1] as f32 / 255.0, fg[2] as f32 / 255.0, fg[3] as f32 / 255.0];
                    let exp_f = composite_fill_f32(BlendMode::VividLight, bg_f, fg_f, op, fill, 1.0, false);
                    let exp = [
                        (exp_f[0] * 255.0).round() as u8,
                        (exp_f[1] * 255.0).round() as u8,
                        (exp_f[2] * 255.0).round() as u8,
                        (exp_f[3] * 255.0).round() as u8,
                    ];

                    for c in 0..4 {
                        let diff = (got[c] as i32 - exp[c] as i32).abs();
                        if diff > max_diff {
                            max_diff = diff;
                            worst = Some((bg, fg, fill, op, c, got, exp, diff));
                        }
                    }
                }
            }
        }
    }

    if let Some((bg, fg, fill, op, c, got, exp, diff)) = worst {
        println!("\nWorst VividLight Special Eight mismatch:");
        println!("  bg={bg:?}, fg={fg:?}, fill={fill}, op={op}");
        println!("  channel {c}: got={}, exp={} (diff={diff})", got[c], exp[c]);
        println!("  got full={got:?}, exp full={exp:?}");
    }
}

/// 6. EMPIRICAL TEST: Non-Separable Modes Parity
#[test]
fn test_empirical_non_separable_modes() {
    let non_separable =
        [BlendMode::DarkerColor, BlendMode::LighterColor, BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity];

    println!("\n=== NON-SEPARABLE MODES SAMPLING TEST ===");

    // Grid of colors
    let steps = [0u8, 32, 64, 128, 192, 255];
    for mode in non_separable {
        let mut total = 0usize;
        let mut exact = 0usize;
        let mut max_diff = 0i32;
        let mut diff_dist = [0usize; 256];

        for &r1 in &steps {
            for &g1 in &steps {
                for &b1 in &steps {
                    for &r2 in &steps {
                        for &g2 in &steps {
                            for &b2 in &steps {
                                total += 1;
                                let cb = [r1, g1, b1];
                                let cs = [r2, g2, b2];
                                let got = craft_blitter::fixed::blend_rgb_u8(mode, cb, cs);

                                let cb_f = [r1 as f32 / 255.0, g1 as f32 / 255.0, b1 as f32 / 255.0];
                                let cs_f = [r2 as f32 / 255.0, g2 as f32 / 255.0, b2 as f32 / 255.0];
                                let exp_f = craft_blitter::blend::blend_rgb(mode, cb_f, cs_f);
                                let exp = [
                                    (exp_f[0].clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (exp_f[1].clamp(0.0, 1.0) * 255.0).round() as u8,
                                    (exp_f[2].clamp(0.0, 1.0) * 255.0).round() as u8,
                                ];

                                let mut d = 0i32;
                                for c in 0..3 {
                                    d = d.max((got[c] as i32 - exp[c] as i32).abs());
                                }
                                diff_dist[d as usize] += 1;
                                if d == 0 {
                                    exact += 1;
                                }
                                if d > max_diff {
                                    max_diff = d;
                                }
                            }
                        }
                    }
                }
            }
        }

        let pct = (exact as f64 / total as f64) * 100.0;
        println!(
            "{mode:?}: {pct:.2}% exact ({exact}/{total}), max_diff: {max_diff}, <=1: {}, <=2: {}",
            diff_dist[0] + diff_dist[1],
            diff_dist[0] + diff_dist[1] + diff_dist[2]
        );
    }
}
