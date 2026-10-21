use craft_blitter::BlendMode;
use craft_blitter::blend::*;

#[test]
fn test_all_27_modes_in_layer_modes() {
    assert_eq!(BlendMode::LAYER_MODES.len(), 27);
    for mode in BlendMode::LAYER_MODES {
        let cb = [0.4f32, 0.5, 0.6];
        let cs = [0.7f32, 0.2, 0.8];
        let r = blend_rgb(mode, cb, cs);
        for c in 0..3 {
            assert!(r[c] >= 0.0 && r[c] <= 1.0, "{mode:?} out of [0, 1]: {:?}", r);
        }
    }
}

#[test]
fn test_vivid_light_source_extremes_win() {
    // cs <= 0.0 -> 0.0
    assert_eq!(vivid_light_ps(1.0, 0.0), 0.0);
    assert_eq!(vivid_light_ps(0.5, 0.0), 0.0);
    assert_eq!(vivid_light_ps(0.0, 0.0), 0.0);

    // cs >= 1.0 -> 1.0
    assert_eq!(vivid_light_ps(0.0, 1.0), 1.0);
    assert_eq!(vivid_light_ps(0.5, 1.0), 1.0);
    assert_eq!(vivid_light_ps(1.0, 1.0), 1.0);

    // Interior
    assert!((vivid_light_ps(0.5, 0.75) - 1.0).abs() < 1e-6);
    assert!((vivid_light_ps(0.5, 0.25) - 0.0).abs() < 1e-6);
}

#[test]
fn test_hard_mix_edges_match_photoshop() {
    // Backdrop extremes win in Hard Mix
    assert_eq!(hard_mix_ps(1.0, 0.0), 1.0);
    assert_eq!(hard_mix_ps(0.0, 1.0), 0.0);

    // Interior threshold: cb + cs >= 1.0 -> 1.0
    assert_eq!(hard_mix_ps(0.6, 0.4), 1.0);
    assert_eq!(hard_mix_ps(0.6, 0.6), 1.0);
    assert_eq!(hard_mix_ps(0.3, 0.3), 0.0);
    assert_eq!(hard_mix_ps(0.6, 0.3), 0.0);
}

#[test]
fn test_color_burn_edge_guard() {
    // Near-white backdrop preserves white
    assert_eq!(color_burn(1.0 - 1e-6, 0.0), 1.0);
    assert_eq!(color_burn(1.0, 0.0), 1.0);
    assert_eq!(color_burn(0.99999, 0.0), 1.0);

    // Black source gives black
    assert_eq!(color_burn(0.5, 0.0), 0.0);
    assert!((color_burn(0.5, 0.5) - 0.0).abs() < 1e-6);
    assert!((color_burn(0.75, 0.5) - 0.5).abs() < 1e-6);
}

#[test]
fn test_color_dodge_edge_guard() {
    // Near-black backdrop preserves black
    assert_eq!(color_dodge(1e-6, 1.0), 0.0);
    assert_eq!(color_dodge(0.0, 1.0), 0.0);
    assert_eq!(color_dodge(0.00005, 1.0), 0.0);

    // White source gives white
    assert_eq!(color_dodge(0.5, 1.0), 1.0);
    assert!((color_dodge(0.25, 0.5) - 0.5).abs() < 1e-6);
}

#[test]
fn test_divide_guards() {
    // cs <= 0.0, cb <= 0.0 -> 0.0
    assert_eq!(divide(0.0, 0.0, 1.0), 0.0);
    assert_eq!(divide(-0.5, 0.0, 1.0), 0.0);

    // cs <= 0.0, cb > 0.0 -> 1.0
    assert_eq!(divide(0.5, 0.0, 1.0), 1.0);
    assert_eq!(divide(1.0, 0.0, 1.0), 1.0);
    assert_eq!(divide(3.0, 0.0, f32::MAX), 1.0);

    // Normal division
    assert!((divide(0.4, 0.8, 1.0) - 0.5).abs() < 1e-6);
    // HDR preservation beyond 1.0
    assert!((divide(2.0, 0.5, f32::MAX) - 4.0).abs() < 1e-6);
    // SDR clamping at 1.0
    assert_eq!(divide(2.0, 0.5, 1.0), 1.0);
}

#[test]
fn test_non_separable_modes() {
    let cb = [0.2f32, 0.5, 0.9];
    let cs = [0.8f32, 0.3, 0.1];

    let hue = blend_rgb(BlendMode::Hue, cb, cs);
    let sat = blend_rgb(BlendMode::Saturation, cb, cs);
    let col = blend_rgb(BlendMode::Color, cb, cs);
    let lum_res = blend_rgb(BlendMode::Luminosity, cb, cs);

    // Hue, Sat, Col preserve backdrop luminance
    assert!((lum(hue) - lum(cb)).abs() < 1e-4, "Hue lum mismatch");
    assert!((lum(sat) - lum(cb)).abs() < 1e-4, "Sat lum mismatch");
    assert!((lum(col) - lum(cb)).abs() < 1e-4, "Color lum mismatch");

    // Luminosity preserves source luminance
    assert!((lum(lum_res) - lum(cs)).abs() < 1e-4, "Lum lum mismatch");
}

#[test]
fn test_paint_net_modes() {
    let cb = 0.6f32;
    let cs = 0.4f32;

    let r = reflect(cb, cs);
    assert!((r - (0.36 / 0.6)).abs() < 1e-5);

    let g = glow(cb, cs);
    assert!((g - reflect(cs, cb)).abs() < 1e-6);

    let n = negation(cb, cs);
    assert!((n - (1.0 - 0.0)).abs() < 1e-6);

    let x = xor(cb, cs);
    let b = (0.6f32 * 255.0).round() as u8;
    let s = (0.4f32 * 255.0).round() as u8;
    assert!((x - f32::from(b ^ s) / 255.0).abs() < 1e-6);

    assert_eq!(paint_net_color_burn(1.0, 0.0), 0.0);
    assert_eq!(paint_net_color_dodge(0.0, 1.0), 1.0);
}
