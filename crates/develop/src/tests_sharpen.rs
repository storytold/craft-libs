//! Sharpening: an unsharp mask at the pixel scale, with Radius (source pixels), Detail (halo
//! damping) and Masking (edge mask), the same at every preview and export size.

use crate::raster::{Rgb32f, Rgba8};
use craft_develop_model::DevelopSettings;

use crate::{RenderRequest, SourceInfo, render};

fn info() -> SourceInfo {
    SourceInfo { raw: true, ..Default::default() }
}

fn render_at(src: &Rgb32f, s: &DevelopSettings, w: usize) -> Rgba8 {
    render(src, &info(), s, &RenderRequest::fit(w, w)).image
}

/// RMS difference of the green channel.
fn rms(a: &Rgba8, b: &Rgba8) -> f64 {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let sum: f64 = a.data.iter().zip(&b.data).map(|(p, q)| (p[1] as f64 - q[1] as f64).powi(2)).sum();
    (sum / a.data.len() as f64).sqrt()
}

/// A grating with the given period (px at 512 wide), in scene-linear light.
fn grating(w: usize, period: f32) -> Rgb32f {
    let h = w / 8;
    Rgb32f::from_fn(w, h, |x, _| {
        let k = w as f32 / 512.0;
        let v = 0.18 * (1.0 + 0.4 * (std::f32::consts::TAU * x as f32 / (period * k)).sin());
        [v; 3]
    })
}

/// A scene with a hard edge, fine texture and a gradient, defined in normalized coordinates so it
/// can be sampled at any size.
fn scene(w: usize) -> Rgb32f {
    let h = w * 3 / 4;
    Rgb32f::from_fn(w, h, |x, y| {
        let (u, v) = (x as f32 / w as f32, y as f32 / w as f32);
        let edge = if u > 0.5 { 0.3 } else { 0.08 };
        let tex = 0.03 * (u * 150.0).sin() * (v * 130.0).cos();
        let g = 0.1 * v;
        let l = (edge + tex + g).max(0.001);
        [l, l * 0.9, l * 0.8]
    })
}

fn sharp(amount: f64, radius: f64, detail: f64, masking: f64) -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.detail.sharpen_amount = amount;
    s.detail.sharpen_radius = radius;
    s.detail.sharpen_detail = detail;
    s.detail.sharpen_masking = masking;
    s
}

#[test]
fn amount_zero_is_the_identity_whatever_radius_and_detail() {
    let src = scene(256);
    let plain = render_at(&src, &DevelopSettings::default(), 256);
    for (r, d) in [(0.5, 0.0), (1.0, 25.0), (3.0, 100.0)] {
        assert_eq!(render_at(&src, &sharp(0.0, r, d, 0.0), 256).data, plain.data, "radius {r} detail {d}");
    }
}

#[test]
fn radius_changes_the_output() {
    let src = scene(256);
    let a = render_at(&src, &sharp(60.0, 0.8, 25.0, 0.0), 256);
    let b = render_at(&src, &sharp(60.0, 2.5, 25.0, 0.0), 256);
    assert_ne!(a.data, b.data);
    assert!(rms(&a, &b) > 0.3, "{}", rms(&a, &b));
}

#[test]
fn larger_radius_boosts_coarser_detail() {
    // gain on a fine (4 px) and a coarse (24 px) grating, for a small and a large radius
    let gain = |period: f32, radius: f64| {
        let src = grating(512, period);
        rms(&render_at(&src, &sharp(80.0, radius, 100.0, 0.0), 512), &render_at(&src, &sharp(0.0, radius, 100.0, 0.0), 512))
    };
    let (fine_small, fine_large) = (gain(4.0, 0.6), gain(4.0, 3.0));
    let (coarse_small, coarse_large) = (gain(24.0, 0.6), gain(24.0, 3.0));
    eprintln!("fine {fine_small} {fine_large}, coarse {coarse_small} {coarse_large}");
    assert!(coarse_large > 2.0 * coarse_small, "coarse: small {coarse_small}, large {coarse_large}");
    assert!(fine_small > coarse_small, "a small radius favours fine detail: {fine_small} vs {coarse_small}");
    // the large radius moves its emphasis to coarser detail
    assert!(coarse_large / fine_large > 2.0 * coarse_small / fine_small, "{coarse_large}/{fine_large} vs {coarse_small}/{fine_small}");
}

#[test]
fn detail_changes_the_output_and_damps_halos() {
    let src = scene(256);
    let lo = render_at(&src, &sharp(80.0, 1.5, 0.0, 0.0), 256);
    let hi = render_at(&src, &sharp(80.0, 1.5, 100.0, 0.0), 256);
    let plain = render_at(&src, &sharp(0.0, 1.5, 0.0, 0.0), 256);
    assert_ne!(lo.data, hi.data);
    // the overshoot next to the hard edge (u = 0.5) shrinks with Detail 0
    let row = 96 * 256;
    let at = |img: &Rgba8, x: usize| img.data[row + x][1] as i32;
    let over = |img: &Rgba8| (118..128).map(|x| (at(img, x) - at(&plain, x)).abs()).max().unwrap_or(0);
    assert!(over(&lo) < over(&hi), "halo {} (detail 0) vs {} (detail 100)", over(&lo), over(&hi));
}

#[test]
fn masking_leaves_flat_areas_alone() {
    // gentle noise on a flat field: unmasked sharpening changes it, a high Masking does not
    let src = Rgb32f::from_fn(128, 96, |x, y| {
        let n = ((x * 7 + y * 13) % 5) as f32 * 0.0015;
        [0.18 + n; 3]
    });
    let plain = render_at(&src, &sharp(0.0, 1.0, 25.0, 0.0), 128);
    let open = render_at(&src, &sharp(100.0, 1.0, 25.0, 0.0), 128);
    let masked = render_at(&src, &sharp(100.0, 1.0, 25.0, 100.0), 128);
    assert!(rms(&open, &plain) > rms(&masked, &plain), "{} vs {}", rms(&open, &plain), rms(&masked, &plain));
}

#[test]
fn sharpening_is_the_same_at_every_size() {
    // the same photo previewed at half size and rendered in full: what sharpening adds (sharpened
    // minus plain) agrees between the preview and the shrunk full render far better than with zero
    let src = scene(512);
    let shrink = |img: &Rgba8| -> Vec<f64> {
        (0..256 * 192)
            .map(|i| {
                let (x, y) = (i % 256 * 2, i / 256 * 2);
                let p = |dx: usize, dy: usize| img.data[(y + dy) * 512 + x + dx][1] as f64;
                (p(0, 0) + p(1, 0) + p(0, 1) + p(1, 1)) / 4.0
            })
            .collect()
    };
    let added = |a: &[f64], b: &[f64]| -> Vec<f64> { a.iter().zip(b).map(|(p, q)| p - q).collect() };
    let norm = |v: &[f64]| (v.iter().map(|x| x * x).sum::<f64>() / v.len() as f64).sqrt();
    let (s, off) = (sharp(60.0, 2.0, 25.0, 0.0), sharp(0.0, 2.0, 25.0, 0.0));
    let full = added(&shrink(&render_at(&src, &s, 512)), &shrink(&render_at(&src, &off, 512)));
    let half_on = render_at(&src, &s, 256);
    let half_off = render_at(&src, &off, 256);
    let half: Vec<f64> = half_on.data.iter().zip(&half_off.data).map(|(p, q)| p[1] as f64 - q[1] as f64).collect();
    let miss = norm(&added(&full, &half));
    eprintln!("size invariance: effect {} (half) {} (full, shrunk), disagreement {miss}", norm(&half), norm(&full));
    assert!(norm(&half) > 0.5, "the sharpening shows at half size");
    assert!(miss < 0.6 * norm(&half), "preview and export disagree: {miss} vs an effect of {}", norm(&half));
}

#[test]
fn subpixel_blur_response_is_continuous_and_does_not_vanish() {
    use crate::local::{sharpen_gain, sharpen_sigma};
    use crate::raster::{
        Plane,
        blur::{gauss_taps, gaussian_taps},
    };
    // An alternating grating measures the actual FIR response at Nyquist, away from edges.
    let src = Plane::from_fn(256, 16, |x, _| if x % 2 == 0 { 1.0 } else { -1.0 });
    let response = |sigma| {
        let blur = gaussian_taps(&src, &gauss_taps(sharpen_sigma(sigma)));
        (16..240).map(|x| (src.data[x] - blur.data[x]) * src.data[x]).sum::<f32>() / 224.0 * sharpen_gain(sigma)
    };
    for sigma in [0.1, 0.25, 0.34, 0.4, 0.4999, 0.5001] {
        let desired = -(-0.5 * std::f32::consts::PI.powi(2) * sigma * sigma).exp_m1();
        assert!((response(sigma) - desired).abs() < 0.001, "sigma {sigma}: {} vs {desired}", response(sigma));
    }
    assert!((response(0.4999) - response(0.5001)).abs() < 0.001);
    assert!(response(0.75) > response(0.5));
    assert_eq!(sharpen_gain(1.0), 1.0);
    assert_eq!(sharpen_gain(2.0), 1.0);
}

#[test]
fn reduced_source_uses_the_original_pixel_radius() {
    let original = scene(512);
    let told = SourceInfo { native_long: Some(512), ..info() };
    let req = RenderRequest::fit(128, 128);
    let s = sharp(100.0, 2.0, 100.0, 0.0);
    // Use the pipeline's own resampling for the smaller source level. Mixing resampling
    // kernels adds edge ringing unrelated to whether the original pixel radius is retained.
    let p = crate::plan(&original, &info(), &s, &req);
    let reduced = p.frame.sample(&original, p.w, p.h);
    let mut off = s.clone();
    off.detail.sharpen_amount = 0.0;
    let from_original = render(&original, &info(), &s, &req).image;
    let from_reduced = render(&reduced, &told, &s, &req).image;
    let unknown = render(&reduced, &info(), &s, &req).image;
    let resampling = rms(&render(&original, &info(), &off, &req).image, &render(&reduced, &told, &off, &req).image);
    let (a, b) = (rms(&from_original, &from_reduced), rms(&from_original, &unknown));
    assert!(a < resampling + 0.5, "original-size radius: {a}, resampling {resampling}");
    assert!(b > a + 0.3, "copy-size radius: {b}, original-size radius: {a}");
}

#[test]
fn old_processes_keep_their_sharpening_and_cache_switches_are_exact() {
    use crate::{StageCache, render_cached};
    use craft_develop_model::Process;
    use std::sync::Arc;
    let src = Arc::new(scene(256));
    let req = RenderRequest::fit(256, 256);
    let cache = StageCache::default();
    for process in [Process::V1, Process::V2, Process::V3, Process::V2, Process::V3] {
        let mut s = sharp(80.0, 0.6, 0.0, 30.0);
        s.process = process.version();
        let plain = render(&src, &info(), &s, &req).image;
        assert_eq!(render_cached(&src, &info(), &s, &req, &cache).image, plain);
        s.detail.sharpen_radius = 3.0;
        s.detail.sharpen_detail = 100.0;
        let changed = render(&src, &info(), &s, &req).image;
        if process < Process::V3 {
            assert_eq!(changed, plain, "{process:?}");
        } else {
            assert_ne!(changed, plain);
        }
        for nr in [60.0, 20.0] {
            s.detail.nr_luminance = nr;
            assert_eq!(render_cached(&src, &info(), &s, &req, &cache).image, render(&src, &info(), &s, &req).image, "{process:?}, NR {nr}");
        }
    }
}

#[test]
fn texture_does_not_depend_on_sharpening_settings() {
    let src = scene(256);
    let mut a = DevelopSettings::default();
    a.effects.texture = 50.0;
    let mut b = a.clone();
    b.detail.sharpen_radius = 3.0;
    b.detail.sharpen_detail = 100.0;
    assert_eq!(render_at(&src, &a, 256).data, render_at(&src, &b, 256).data);
}

#[test]
fn local_sharpness_uses_the_sharpen_band() {
    use craft_develop_model::{LocalAdjustments, Mask, MaskComponent, MaskOp, MaskShape};
    use craft_geom::Point;
    let src = scene(256);
    let mut s = DevelopSettings {
        masks: vec![Mask {
            components: vec![MaskComponent {
                name: None,
                op: MaskOp::Add,
                invert: false,
                shape: MaskShape::Linear { start: Point::new(0.5, 0.0), end: Point::new(0.5, 0.01) },
            }],
            adjust: LocalAdjustments { sharpness: 100.0, ..Default::default() },
            ..Default::default()
        }],
        ..Default::default()
    };
    let plain = render_at(&src, &DevelopSettings::default(), 256);
    let with = render_at(&src, &s, 256);
    assert!(rms(&with, &plain) > 0.05, "{}", rms(&with, &plain));
    s.detail.sharpen_radius = 3.0;
    assert_ne!(render_at(&src, &s, 256).data, with.data, "Radius reaches the local sharpness band");
}

/// An enlarging export (2× the original) scales Radius 2 and 3 to 4 and 6 px: they stay distinct
/// rather than meeting at a 4 px cap.
#[test]
fn enlarged_export_keeps_radius_steps_apart() {
    use crate::Quality;
    use crate::local::plane_sigmas;
    let sigma = |r: f64| plane_sigmas(&sharp(100.0, r, 0.0, 0.0), 1024.0, 512.0, Quality::Full).sharpen.unwrap();
    assert_eq!((sigma(2.0), sigma(3.0)), (4.0, 6.0));
    let src = scene(128);
    let enlarged = SourceInfo { native_long: Some(64), ..info() };
    let at = |r: f64| render(&src, &enlarged, &sharp(100.0, r, 0.0, 0.0), &RenderRequest::fit(128, 128)).image;
    let d = rms(&at(2.0), &at(3.0));
    assert!(d > 0.1, "Radius 2 and 3 render alike at 2x: {d}");
}
