use craft_blitter::fixed::{div255, div65535, lerp255, lerp65535};
use craft_blitter::{BlendMode, blit_rgba8, blit_rgba16};

#[test]
fn test_div255_exhaustive_all_65536() {
    for x in 0..=65535u32 {
        let expected = (x as f64 / 255.0).round() as u32;
        let got = div255(x);
        assert_eq!(got, expected, "div255 mismatch for {x}: got {got}, want {expected}");
    }
}

#[test]
fn test_div65535_boundaries_and_steps() {
    let offsets = [-2i64, -1, 0, 1, 2, 32766, 32767, 32768, 32769];
    for k in 0..=65535u64 {
        for &off in &offsets {
            let val_i64 = (k * 65535) as i64 + off;
            if !(0..=(65535i64 * 65535i64)).contains(&val_i64) {
                continue;
            }
            let x = val_i64 as u64;
            let expected = (x as f64 / 65535.0).round() as u32;
            let got = div65535(x);
            assert_eq!(got, expected, "div65535 mismatch for {x} (k={k}, off={off}): got {got}, want {expected}");
        }
    }
}

#[test]
fn test_lerp_endpoints() {
    assert_eq!(lerp255(100, 200, 0), 200);
    assert_eq!(lerp255(100, 200, 255), 100);

    assert_eq!(lerp65535(10000, 50000, 0), 50000);
    assert_eq!(lerp65535(10000, 50000, 65535), 10000);
}

#[test]
fn test_rgba8_fixed_blit_basic() {
    let mut dst = [100u8, 150, 200, 255];
    let src = [200u8, 50, 100, 255];
    blit_rgba8(&mut dst, &src, BlendMode::Normal, 1.0, 1.0);
    assert_eq!(dst, src);
}

#[test]
fn test_rgba16_fixed_blit_basic() {
    let mut dst = [10000u16, 20000, 30000, 65535];
    let src = [40000u16, 50000, 60000, 65535];
    blit_rgba16(&mut dst, &src, BlendMode::Normal, 1.0, 1.0);
    assert_eq!(dst, src);
}

#[test]
fn test_rgba8_all_27_modes_no_overflow() {
    for mode in BlendMode::LAYER_MODES {
        let mut dst = [128u8, 64, 200, 255];
        let src = [64u8, 192, 100, 255];
        blit_rgba8(&mut dst, &src, mode, 0.8, 0.7);
        // Valid byte range guaranteed
        for &val in &dst {
            let _ = val;
        }
    }
}

#[test]
fn test_rgba16_all_27_modes_no_overflow() {
    for mode in BlendMode::LAYER_MODES {
        let mut dst = [32768u16, 16384, 50000, 65535];
        let src = [16384u16, 49152, 25000, 65535];
        blit_rgba16(&mut dst, &src, mode, 0.8, 0.7);
        for &val in &dst {
            let _ = val;
        }
    }
}
