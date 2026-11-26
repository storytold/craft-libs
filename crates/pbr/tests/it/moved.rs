//! The tests of TextureCraft's and MaterialCraft's `ibl.rs`, on this crate's API. The import test
//! runs TextureCraft's import sizing (halve above 2048, resample below 128 × 64) the way its
//! adapter does, since that sizing stays in the app.

use craft_pbr::{ColorEncoding, ENVIRONMENTS, EnvImage, Environment, Limits, PrefilterOptions, dir_of, irradiance, try_prefilter, try_sh9, uv_of};

use crate::{limits, opts};

#[test]
fn uv_and_direction_round_trip() {
    for i in 1..20 {
        for j in 1..10 {
            let (u, v) = (i as f32 / 20.0, j as f32 / 10.0);
            let back = uv_of(dir_of(u, v));
            assert!((back[0] - u).abs() < 1e-4 && (back[1] - v).abs() < 1e-4);
        }
    }
}

#[test]
fn sh_of_a_uniform_environment_is_constant_irradiance() {
    let env = EnvImage::try_from_linear(64, 32, vec![[1.0, 1.0, 1.0]; 64 * 32], &Limits::default()).unwrap();
    let c = try_sh9(&env, &Limits::default()).unwrap();
    for d in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.3, -0.8, 0.5]] {
        let e = irradiance(&c, d).unwrap()[0];
        assert!((e - 1.0).abs() < 0.03, "{e}");
    }
}

#[test]
fn prefilter_levels_shrink_and_stay_finite() {
    let limits = Limits::default();
    let env = Environment::StudioSoft.try_render(128, &limits).unwrap();
    let options = opts(|o| o.levels = 5);
    let mips = try_prefilter(&env, &options, &limits).unwrap();
    assert_eq!(mips.len(), 5);
    assert!(mips[4].width() < mips[1].width());
    for m in &mips {
        assert!(m.pixels().iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)));
    }
    for name in ENVIRONMENTS {
        let e = Environment::from_name(name).unwrap().try_render(64, &limits).unwrap();
        assert!(e.pixels().iter().any(|c| c[0] > 0.5), "{name} has no light");
    }
}

/// TextureCraft's import sizing, as its adapter applies it after `try_from_rgba`.
fn texturecraft_import(width: u32, height: u32, rgba: &[[f32; 4]], encoding: ColorEncoding) -> Option<EnvImage> {
    let limits = limits(|l| {
        l.max_side = 1 << 16;
        l.max_pixels = 1 << 26;
        l.max_bytes = 1 << 30;
    });
    let mut img = EnvImage::try_from_rgba(width, height, rgba, encoding, &limits).ok()?;
    while img.width() > 2048 || img.height() > 2048 {
        img = img.try_half(&limits).ok()?;
    }
    if img.width() < 128 || img.height() < 64 {
        img = img.try_resampled(img.width().max(128), img.height().max(64), &limits).ok()?;
    }
    Some(img)
}

#[test]
fn imported_images_become_safe_linear_environments() {
    assert!(texturecraft_import(4, 2, &[[0.0; 4]; 7], ColorEncoding::Linear).is_none(), "pixel count must match");
    // HDR values stay above 1; NaN, infinity and negatives become 0; tiny images grow to 128×64.
    let px = [[50.0, f32::NAN, -1.0, 1.0], [f32::INFINITY, 0.5, 0.25, 1.0]];
    let raw = EnvImage::try_from_rgba(2, 1, &px, ColorEncoding::Linear, &Limits::default()).unwrap();
    assert_eq!(raw.pixels(), &[[50.0, 0.0, 0.0], [0.0, 0.5, 0.25]]);
    let e = texturecraft_import(2, 1, &px, ColorEncoding::Linear).unwrap();
    assert_eq!((e.width(), e.height()), (128, 64));
    assert!(e.pixels().iter().all(|c| c.iter().all(|v| v.is_finite() && *v >= 0.0)));
    assert!(e.pixels().iter().any(|c| c[0] > 40.0), "HDR range is kept");
    // 8-bit images are sRGB-decoded; big ones are halved down to the import limit.
    let e = texturecraft_import(4096, 2048, &vec![[0.5, 0.5, 0.5, 1.0]; 4096 * 2048], ColorEncoding::Srgb).unwrap();
    assert_eq!((e.width(), e.height()), (2048, 1024));
    assert!((e.pixels()[0][0] - 0.214).abs() < 0.01, "{}", e.pixels()[0][0]);
    let mips = try_prefilter(&e, &PrefilterOptions::default(), &Limits::default()).unwrap();
    assert!(mips.iter().all(|m| m.pixels().iter().all(|c| c.iter().all(|v| v.is_finite()))));
}
