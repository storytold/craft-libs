use craft_blitter::{BlendMode, blit_rgba8, blit_rgba16, blit_rgba32f, blit_rgba32f_gamma};

#[test]
fn test_zero_alloc_stack_slices_rgba8() {
    let mut dst = [128u8; 1024];
    let src = [200u8; 1024];

    for mode in BlendMode::LAYER_MODES {
        blit_rgba8(&mut dst, &src, mode, 0.75, 0.5);
    }
}

#[test]
fn test_zero_alloc_stack_slices_rgba16() {
    let mut dst = [32768u16; 512];
    let src = [50000u16; 512];

    for mode in BlendMode::LAYER_MODES {
        blit_rgba16(&mut dst, &src, mode, 0.75, 0.5);
    }
}

#[test]
fn test_zero_alloc_stack_slices_rgba32f() {
    let mut dst = [[0.5f32, 0.5, 0.5, 1.0]; 128];
    let src = [[0.8f32, 0.2, 0.4, 0.9]; 128];

    for mode in BlendMode::LAYER_MODES {
        blit_rgba32f(&mut dst, &src, mode, 0.75, 0.5, false);
        blit_rgba32f(&mut dst, &src, mode, 0.75, 0.5, true);
        blit_rgba32f_gamma(&mut dst, &src, mode, 0.75, 0.5, 1.45, false);
    }
}
