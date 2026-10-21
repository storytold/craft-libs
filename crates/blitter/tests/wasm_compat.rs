use craft_blitter::{BlendMode, blit_rgba8, blit_rgba16, blit_rgba32f, composite, composite_fill, composite_gamma};

#[test]
fn test_wasm_compat_sanity() {
    let b = [0.2f32, 0.4, 0.6, 1.0];
    let s = [0.8f32, 0.2, 0.4, 0.5];
    let out = composite(BlendMode::Multiply, b, s, 1.0);
    assert!(out[0] >= 0.0 && out[0] <= 1.0);

    let out_fill = composite_fill(BlendMode::HardMix, b, s, 1.0, 0.5, 1.0);
    assert!(out_fill[0] >= 0.0 && out_fill[0] <= 1.0);

    let out_gamma = composite_gamma(BlendMode::Normal, b, s, 1.0, 1.45);
    assert!(out_gamma[0] >= 0.0 && out_gamma[0] <= 1.0);

    let mut dst8 = [100u8; 16];
    let src8 = [200u8; 16];
    blit_rgba8(&mut dst8, &src8, BlendMode::Screen, 0.8, 0.5);

    let mut dst16 = [10000u16; 16];
    let src16 = [20000u16; 16];
    blit_rgba16(&mut dst16, &src16, BlendMode::Darken, 0.8, 0.5);

    let mut dst32 = [[0.1f32, 0.2, 0.3, 1.0]; 4];
    let src32 = [[0.4f32, 0.5, 0.6, 0.8]; 4];
    blit_rgba32f(&mut dst32, &src32, BlendMode::Overlay, 0.8, 0.5, false);
}
