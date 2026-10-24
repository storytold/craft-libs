use craft_blitter::blend::{TEXT_GAMMA, color_burn, color_dodge, hard_mix_ps, vivid_light_ps};
use craft_blitter::{BlendMode, composite, composite_fill, composite_gamma};

#[test]
fn special_eight_fill_matches_photoshop() {
    // Photoshop 27.11, measured: Background #c89664, a layer #64b4dc at Fill 50% (128/255),
    // Opacity 100%, in each of the special eight. Values are the composite at 8 bits.
    let measured: [(BlendMode, [u8; 3]); 8] = [
        (BlendMode::ColorBurn, [176, 132, 88]),
        (BlendMode::LinearBurn, [122, 112, 82]),
        (BlendMode::ColorDodge, [249, 232, 176]),
        (BlendMode::LinearDodge, [250, 240, 210]),
        (BlendMode::VividLight, [193, 188, 157]),
        (BlendMode::LinearLight, [171, 202, 192]),
        (BlendMode::HardMix, [245, 225, 165]),
        (BlendMode::Difference, [150, 60, 10]),
    ];
    let px = |c: [u8; 3]| [f32::from(c[0]) / 255.0, f32::from(c[1]) / 255.0, f32::from(c[2]) / 255.0, 1.0];
    let (b, t) = (px([200, 150, 100]), px([100, 180, 220]));
    for (mode, want) in measured {
        let got = composite_fill(mode, b, t, 1.0, 128.0 / 255.0, 1.0);
        for i in 0..3 {
            assert!((got[i] * 255.0 - f32::from(want[i])).abs() <= 1.5, "{mode:?} channel {i}: {} vs Photoshop {}", got[i] * 255.0, want[i]);
        }
        // At 100% Fill it is the plain blend; at 0% the backdrop.
        assert_eq!(composite_fill(mode, b, t, 1.0, 1.0, 1.0), composite(mode, b, t, 1.0), "{mode:?}");
        let none = composite_fill(mode, b, t, 1.0, 0.0, 1.0);
        assert!((0..3).all(|i| (none[i] - b[i]).abs() < 1e-5), "{mode:?} at Fill 0: {none:?}");
    }

    // A second measurement: Background #3c78b4, layer #d25a1e at Fill 25% (64/255) and 75%
    // (191/255). Hard Mix at 75% is 3 levels off in red: dividing by 1 − f = 0.25 magnifies
    // Photoshop's 8-bit rounding.
    let confirm: [(BlendMode, [u8; 3], [u8; 3]); 8] = [
        (BlendMode::ColorBurn, [51, 94, 159], [30, 0, 33]),
        (BlendMode::LinearBurn, [49, 79, 124], [26, 0, 11]),
        (BlendMode::ColorDodge, [76, 132, 186], [156, 163, 197]),
        (BlendMode::LinearDodge, [113, 143, 188], [217, 187, 202]),
        (BlendMode::VividLight, [71, 109, 162], [117, 81, 80]),
        (BlendMode::LinearLight, [100, 100, 130], [183, 63, 33]),
        (BlendMode::HardMix, [65, 105, 166], [102, 0, 47]),
        (BlendMode::Difference, [7, 97, 172], [97, 53, 158]),
    ];
    let (b2, t2) = (px([60, 120, 180]), px([210, 90, 30]));
    for (mode, q, tq) in confirm {
        for (fill, want) in [(64.0 / 255.0, q), (191.0 / 255.0, tq)] {
            let got = composite_fill(mode, b2, t2, 1.0, fill, 1.0);
            let tol = if mode == BlendMode::HardMix { 3.5 } else { 1.5 };
            for i in 0..3 {
                assert!(
                    (got[i] * 255.0 - f32::from(want[i])).abs() <= tol,
                    "{mode:?} fill {fill} channel {i}: {} vs Photoshop {}",
                    got[i] * 255.0,
                    want[i]
                );
            }
        }
    }
    // Other modes keep Fill as coverage, like Opacity.
    assert_eq!(composite_fill(BlendMode::Multiply, b, t, 0.8, 0.5, 1.0), composite(BlendMode::Multiply, b, t, 0.4));
    // Over transparency Fill is still coverage for the special eight.
    let r = composite_fill(BlendMode::LinearDodge, [0.0; 4], t, 1.0, 0.5, 1.0);
    assert!((r[3] - 0.5).abs() < 1e-6 && (r[0] - t[0]).abs() < 1e-6, "{r:?}");
}

#[test]
fn generic_vivid_light_and_hard_mix_composites_match_photoshop() {
    let cases = [
        (BlendMode::HardMix, 0.0, 1.0),
        (BlendMode::HardMix, 1.0, 0.0),
        (BlendMode::HardMix, 0.6, 0.4),
        (BlendMode::HardMix, 0.6, 0.3),
        (BlendMode::VividLight, 1.0, 0.0),
        (BlendMode::VividLight, 0.0, 1.0),
        (BlendMode::VividLight, 0.75, 0.25),
        (BlendMode::VividLight, 0.25, 0.75),
    ];

    for (mode, cb, cs) in cases {
        let backdrop = [cb, cb, cb, 1.0];
        let source = [cs, cs, cs, 1.0];
        let res = composite(mode, backdrop, source, 1.0);
        let expected_val = match mode {
            BlendMode::HardMix => hard_mix_ps(cb, cs),
            BlendMode::VividLight => vivid_light_ps(cb, cs),
            _ => 0.0,
        };
        for (channel, &ch_val) in res.iter().take(3).enumerate() {
            assert!((ch_val - expected_val).abs() < 1e-6, "{mode:?} Cb={cb} Cs={cs}: channel {channel} {ch_val} != {expected_val}");
        }
    }
}

#[test]
fn text_gamma_lightens_dark_edges() {
    // 69 % blue (0, 51, 153) over grey 129: Photoshop shows (57, 80, 146).
    let r = composite_gamma(BlendMode::Normal, [129.0 / 255.0, 129.0 / 255.0, 129.0 / 255.0, 1.0], [0.0, 0.2, 0.6, 0.686], 1.0, TEXT_GAMMA);
    assert!((r[0] * 255.0 - 57.0).abs() < 1.0 && (r[1] * 255.0 - 80.0).abs() < 1.0 && (r[2] * 255.0 - 146.0).abs() < 1.0, "{r:?}");

    // Near-opaque edges stay dark: 94 % coverage shows (13, 57, 152)
    let r2 = composite_gamma(BlendMode::Normal, [129.0 / 255.0, 129.0 / 255.0, 129.0 / 255.0, 1.0], [0.0, 0.2, 0.6, 0.937], 1.0, TEXT_GAMMA);
    assert!((r2[0] * 255.0 - 13.0).abs() < 1.0 && (r2[1] * 255.0 - 57.0).abs() < 1.0 && (r2[2] * 255.0 - 152.0).abs() < 1.0, "{r2:?}");

    // gamma == 1.0 matches standard composite
    assert_eq!(
        composite_gamma(BlendMode::Multiply, [0.2, 0.4, 0.6, 0.7], [0.5, 0.1, 0.9, 0.5], 0.8, 1.0),
        composite(BlendMode::Multiply, [0.2, 0.4, 0.6, 0.7], [0.5, 0.1, 0.9, 0.5], 0.8)
    );
}

#[test]
fn burn_and_dodge_treat_near_extremes_as_exact() {
    assert_eq!(color_burn(1.0 - 1e-6, 0.0), 1.0);
    assert_eq!(color_dodge(1e-6, 1.0), 0.0);
    assert!((color_burn(0.5, 0.5) - 0.0).abs() < 1e-6);
    assert!((color_dodge(0.25, 0.5) - 0.5).abs() < 1e-6);
}
