use craft_blitter::BlendMode;
use craft_blitter::blend::blend_rgb;
use craft_blitter::fixed::{blend_channel_u8, blend_channel_u16, blend_rgb_u8, blend_rgb_u16, div255, div65535, lerp255, lerp65535};

#[test]
fn div255_is_exact_nearest_integer_for_all_inputs() {
    for x in 0..=65535u32 {
        let want = (x + 127) / 255;
        assert_eq!(div255(x), want, "div255({x})");
    }
}

#[test]
fn div65535_is_exact_nearest_integer_across_boundaries() {
    let offsets = [-2i64, -1, 0, 1, 2, 32766, 32767, 32768, 32769];
    for k in 0..=65535u64 {
        for off in offsets {
            let v = (k * 65535) as i64 + off;
            if !(0..=(65535i64 * 65535i64)).contains(&v) {
                continue;
            }
            let x = v as u64;
            let want = ((x + 32767) / 65535) as u32;
            assert_eq!(div65535(x), want, "div65535({x})");
        }
    }
}

#[test]
fn lerp_endpoints_and_midpoints() {
    assert_eq!(lerp255(100, 200, 0), 200);
    assert_eq!(lerp255(100, 200, 255), 100);
    assert_eq!(lerp65535(10000, 50000, 0), 50000);
    assert_eq!(lerp65535(10000, 50000, 65535), 10000);
}

#[test]
fn exhaustive_u8_separable_kernels_match_float_within_one_lsb() {
    for mode in BlendMode::ALL {
        if mode.is_non_separable() {
            continue;
        }
        let mut max_diff = 0u8;
        for cb in 0..=255u8 {
            for cs in 0..=255u8 {
                let got = blend_channel_u8(mode, cb, cs);
                let f = craft_blitter::blend_channel(mode, f32::from(cb) / 255.0, f32::from(cs) / 255.0, 1.0);
                let want = (f.clamp(0.0, 1.0) * 255.0).round() as u8;
                let diff = got.abs_diff(want);
                if diff > max_diff {
                    max_diff = diff;
                }
                assert!(diff <= 1, "{mode:?} u8 mismatch at ({cb}, {cs}): got {got}, want {want} (float {f})");
            }
        }
        assert!(max_diff <= 1, "{mode:?} max_diff={max_diff}");
    }
}

#[test]
fn sampled_u16_kernels_match_float_within_one_lsb() {
    // Note: in u16, Xor is bitwise `cb ^ cs`, whereas float `xor()` quantizes to 8-bit per Paint.NET.
    let samples = [0u16, 1, 2, 255, 16384, 32767, 32768, 32769, 49152, 65534, 65535];
    for mode in BlendMode::ALL {
        if mode.is_non_separable() || mode == BlendMode::Xor {
            continue;
        }
        for &cb in &samples {
            for &cs in &samples {
                let got = blend_channel_u16(mode, cb, cs);
                let f = craft_blitter::blend_channel(mode, f32::from(cb) / 65535.0, f32::from(cs) / 65535.0, 1.0);
                let want = (f.clamp(0.0, 1.0) * 65535.0).round() as u16;
                let diff = got.abs_diff(want);
                assert!(diff <= 1, "{mode:?} u16 mismatch at ({cb}, {cs}): got {got}, want {want} (float {f})");
            }
        }
    }
    assert_eq!(blend_channel_u16(BlendMode::Xor, 0xAAAA, 0x5555), 0xFFFF);
}

#[test]
fn non_separable_rgb_u8_and_u16_match_float_reference() {
    let pairs_u8 = [([200u8, 40, 20], [30u8, 150, 220]), ([10, 200, 100], [180, 20, 90]), ([128, 128, 128], [64, 192, 32])];
    for mode in BlendMode::ALL {
        if !mode.is_non_separable() {
            continue;
        }
        for (cb, cs) in pairs_u8 {
            let got8 = blend_rgb_u8(mode, cb, cs);
            let fb = cb.map(|v| f32::from(v) / 255.0);
            let fs = cs.map(|v| f32::from(v) / 255.0);
            let want_f = blend_rgb(mode, fb, fs);
            for c in 0..3 {
                let want8 = (want_f[c].clamp(0.0, 1.0) * 255.0).round() as u8;
                assert!(got8[c].abs_diff(want8) <= 1, "{mode:?} u8 {got8:?} vs {want8}");
            }

            let cb16 = cb.map(|v| u16::from(v) * 257);
            let cs16 = cs.map(|v| u16::from(v) * 257);
            let got16 = blend_rgb_u16(mode, cb16, cs16);
            for c in 0..3 {
                let want16 = (want_f[c].clamp(0.0, 1.0) * 65535.0).round() as u16;
                assert!(got16[c].abs_diff(want16) <= 1, "{mode:?} u16 {got16:?} vs {want16}");
            }
        }
    }
}
