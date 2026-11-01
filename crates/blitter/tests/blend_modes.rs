use craft_blitter::BlendMode;
use craft_blitter::blend::{
    blend_channel, blend_channel_hdr, blend_rgb, blend_rgb_hdr, blend_rgb_within, classic_color_burn, classic_color_dodge, color_burn, color_dodge,
    divide, glow, hard_light, hard_mix, lum, negation, pin_light, reflect, sat, soft_light, vivid_light, xor,
};

const EPS: f32 = 1e-5;

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < EPS
}

#[test]
fn all_modes_stay_finite_and_in_unit_gamut_for_sdr() {
    assert_eq!(BlendMode::W3C_MODES.len(), 16);
    assert_eq!(BlendMode::LAYER_MODES.len(), 26);
    assert_eq!(BlendMode::ALL.len(), 32);

    let samples = [[0.0f32, 0.0, 0.0], [1.0, 1.0, 1.0], [0.5, 0.5, 0.5], [0.2, 0.5, 0.9], [0.8, 0.25, 0.1]];
    for mode in BlendMode::ALL {
        for cb in samples {
            for cs in samples {
                let out = blend_rgb(mode, cb, cs);
                for ch in out {
                    assert!(ch.is_finite() && (-EPS..=1.0 + EPS).contains(&ch), "{mode:?}({cb:?}, {cs:?}) -> {out:?}");
                }
            }
        }
    }
}

#[test]
fn w3c_separable_formulas_and_identities() {
    // W3C Compositing and Blending Level 1, §10.1:
    // https://www.w3.org/TR/compositing-1/#blendingseparable
    for cb in [0.0f32, 0.15, 0.25, 0.5, 0.75, 1.0] {
        assert!(close(blend_channel(BlendMode::Normal, cb, 0.35, 1.0), 0.35));
        assert!(close(blend_channel(BlendMode::Multiply, cb, 1.0, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Multiply, cb, 0.0, 1.0), 0.0));
        assert!(close(blend_channel(BlendMode::Screen, cb, 0.0, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Screen, cb, 1.0, 1.0), 1.0));
        assert!(close(blend_channel(BlendMode::Darken, cb, 1.0, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Lighten, cb, 0.0, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Difference, cb, 0.0, 1.0), cb));
        assert!(close(blend_channel(BlendMode::Exclusion, cb, 0.0, 1.0), cb));
        assert!(close(hard_light(cb, 0.5), cb));
        assert!(close(soft_light(cb, 0.5), cb));
        assert!(close(blend_channel(BlendMode::Overlay, 0.5, cb, 1.0), cb));
    }

    assert!(close(blend_channel(BlendMode::Multiply, 0.4, 0.5, 1.0), 0.2));
    assert!(close(blend_channel(BlendMode::Screen, 0.4, 0.5, 1.0), 0.7));
    assert!(close(blend_channel(BlendMode::Overlay, 0.2, 0.6, 1.0), 0.24));
    assert!(close(blend_channel(BlendMode::HardLight, 0.2, 0.6, 1.0), 0.36));
    assert!(close(blend_channel(BlendMode::Difference, 0.2, 0.7, 1.0), 0.5));
    assert!(close(blend_channel(BlendMode::Exclusion, 0.5, 0.5, 1.0), 0.5));

    // SoftLight cubic D(cb) for cb <= 0.25 vs sqrt(cb) for cb > 0.25 (W3C §10.1.10).
    let d_cubic = ((16.0 * 0.2 - 12.0) * 0.2 + 4.0) * 0.2;
    assert!(close(soft_light(0.2, 0.75), 0.2 + 0.5 * (d_cubic - 0.2)));
    assert!(close(soft_light(0.64, 0.75), 0.64 + 0.5 * (0.8 - 0.64)));
    // Continuity at cb = 0.25: cubic equals sqrt(0.25) = 0.5.
    assert!(close(soft_light(0.25, 0.8), 0.25 + 0.6 * (0.5 - 0.25)));
}

#[test]
fn w3c_vs_classic_color_burn_and_dodge_corners() {
    // W3C §10.1.7 & §10.1.8 vs ISO 32000-1 Table 136 / VG_KHR_advanced_blending Table 13
    // (documented in Khronos GL_NV_blend_equation_advanced Issue 34).
    assert_eq!(color_burn(1.0, 0.0), 1.0);
    assert_eq!(classic_color_burn(1.0, 0.0), 0.0);
    assert_eq!(color_burn(0.5, 0.0), 0.0);
    assert!(close(color_burn(0.75, 0.5), 0.5));
    // No un-documented threshold below 1.0: cb = 0.9999 with cs = 0.0 is 0.0 per W3C §10.1.8.
    assert_eq!(color_burn(0.9999, 0.0), 0.0);

    assert_eq!(color_dodge(0.0, 1.0), 0.0);
    assert_eq!(classic_color_dodge(0.0, 1.0), 1.0);
    assert_eq!(color_dodge(0.5, 1.0), 1.0);
    assert!(close(color_dodge(0.25, 0.5), 0.5));
    // No un-documented threshold above 0.0: cb = 0.0001 with cs = 1.0 is 1.0 per W3C §10.1.7.
    assert_eq!(color_dodge(0.0001, 1.0), 1.0);
}

#[test]
fn khronos_extended_separable_modes() {
    // GL_NV_blend_equation_advanced Table X.2 & VG_KHR_advanced_blending Table 14.
    // VividLight (VIVIDLIGHT_NV):
    assert_eq!(vivid_light(1.0, 0.0), 0.0);
    assert_eq!(vivid_light(0.0, 1.0), 1.0);
    assert!(close(vivid_light(0.37, 0.5), 0.37));
    assert!(close(vivid_light(0.75, 0.25), 0.5));
    assert!(close(vivid_light(0.25, 0.75), 0.5));

    // HardMix (HARDMIX_NV: 0 if cb + cs < 1, else 1):
    assert_eq!(hard_mix(0.0, 1.0), 1.0);
    assert_eq!(hard_mix(1.0, 0.0), 1.0);
    assert_eq!(hard_mix(0.6, 0.4), 1.0);
    assert_eq!(hard_mix(0.6, 0.39), 0.0);

    // LinearBurn, LinearDodge, LinearLight, PinLight, Subtract:
    assert!(close(blend_channel(BlendMode::LinearBurn, 0.7, 0.6, 1.0), 0.3));
    assert_eq!(blend_channel(BlendMode::LinearBurn, 0.3, 0.4, 1.0), 0.0);
    assert_eq!(blend_channel(BlendMode::LinearDodge, 0.7, 0.6, 1.0), 1.0);
    assert!(close(blend_channel_hdr(BlendMode::LinearDodge, 1.5, 0.75), 2.25));
    assert!(close(blend_channel(BlendMode::LinearLight, 0.4, 0.6, 1.0), 0.6));
    assert!(close(pin_light(0.6, 0.2), 0.4));
    assert!(close(pin_light(0.3, 0.8), 0.6));
    assert!(close(blend_channel(BlendMode::Subtract, 0.7, 0.25, 1.0), 0.45));
    assert_eq!(blend_channel(BlendMode::Subtract, 0.2, 0.7, 1.0), 0.0);
}

#[test]
fn adobe_divide_and_darker_lighter_color() {
    // Divide (https://helpx.adobe.com/photoshop/using/blending-modes.html):
    assert_eq!(divide(0.0, 0.0, 1.0), 0.0);
    assert_eq!(divide(-0.5, 0.0, 1.0), 0.0);
    assert_eq!(divide(0.5, 0.0, 1.0), 1.0);
    assert_eq!(divide(3.0, 0.0, f32::MAX), 1.0);
    assert!(close(divide(0.4, 0.8, 1.0), 0.5));
    assert_eq!(divide(2.0, 0.5, 1.0), 1.0);
    assert!(close(divide(2.0, 0.5, f32::MAX), 4.0));

    // DarkerColor & LighterColor compare total channel sum R + G + B:
    // a = [0.9, 0.0, 0.0] (sum = 0.9), b = [0.0, 0.5, 0.5] (sum = 1.0).
    let a = [0.9f32, 0.0, 0.0];
    let b = [0.0f32, 0.5, 0.5];
    assert_eq!(blend_rgb(BlendMode::DarkerColor, a, b), a);
    assert_eq!(blend_rgb(BlendMode::DarkerColor, b, a), a);
    assert_eq!(blend_rgb(BlendMode::LighterColor, a, b), b);
    assert_eq!(blend_rgb(BlendMode::LighterColor, b, a), b);
}

#[test]
fn w3c_non_separable_modes_and_hdr_gamut() {
    // W3C §10.2: https://www.w3.org/TR/compositing-1/#blendingnonseparable
    let cb = [0.2f32, 0.5, 0.9];
    let cs = [0.8f32, 0.3, 0.1];

    for mode in [BlendMode::Hue, BlendMode::Saturation, BlendMode::Color] {
        let out = blend_rgb(mode, cb, cs);
        assert!((lum(out) - lum(cb)).abs() < 1e-4, "{mode:?}: {out:?}");
    }
    let lum_out = blend_rgb(BlendMode::Luminosity, cb, cs);
    assert!((lum(lum_out) - lum(cs)).abs() < 1e-4);

    // Grey source desaturates backdrop in Saturation mode.
    let desat = blend_rgb(BlendMode::Saturation, cb, [0.5; 3]);
    assert!(sat(desat) < 1e-5);
    assert!((desat[0] - lum(cb)).abs() < 1e-5);

    // HDR: values above 1.0 are preserved when top = f32::MAX.
    let hdr_out = blend_rgb_hdr(BlendMode::Luminosity, [0.5, 0.25, 0.125], [2.0; 3]);
    assert!(hdr_out[0] > 1.0);
    assert!((lum(hdr_out) - 2.0).abs() < 1e-4);

    // Pure greys at HDR boundaries do not produce NaN.
    for mode in [BlendMode::Hue, BlendMode::Saturation, BlendMode::Color, BlendMode::Luminosity] {
        let g = blend_rgb_within(mode, [2.0; 3], [4.0; 3], f32::MAX, f32::MAX);
        assert!(g.iter().all(|v| v.is_finite()));
    }
}

#[test]
fn pegtop_and_paint_net_modes() {
    assert!(close(reflect(0.6, 0.4), 0.36 / 0.6));
    assert_eq!(reflect(0.0, 1.0), 1.0);
    assert!(close(glow(0.6, 0.4), reflect(0.4, 0.6)));
    assert_eq!(glow(1.0, 0.0), 1.0);
    assert!(close(negation(0.2, 0.7), 0.9));

    for b in [0u8, 1, 85, 128, 170, 255] {
        for s in [0u8, 1, 85, 128, 170, 255] {
            let got = xor(f32::from(b) / 255.0, f32::from(s) / 255.0);
            let want = f32::from(b ^ s) / 255.0;
            assert!(close(got, want));
        }
    }
}
