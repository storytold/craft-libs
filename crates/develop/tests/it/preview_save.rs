//! Preview/save approximation checks, not CPU/GPU equivalence or pixel parity.
//!
//! All inputs are procedural, scene-linear Rec.2020. The save reference is the
//! full-size U8 sRGB output, decoded to linear light, Box reduced, then encoded
//! back to U8 sRGB. This models one explicit display reduction, not every export
//! codec/viewer's resampler. Averaging before nonlinear tone mapping cannot in
//! general equal averaging afterwards, especially on unresolved bright texture.

use craft_develop::raster::{
    Rgb32f, Rgba8,
    resample::{Filter, resize},
};
use craft_develop::{RenderRequest, SourceInfo, local, render};
use craft_develop_model::DevelopSettings;

fn scene(w: usize, h: usize) -> Rgb32f {
    Rgb32f::from_fn(w, h, |x, y| {
        let u = x as f32 / w as f32;
        // Smooth shadows through scene highlights, hard boundaries, and fine
        // twigs/checkers that straddle the shoulder after a strong exposure push.
        let ramp = 0.012 * 2.0_f32.powf(7.0 * u);
        let texture = if (x + 3 * y) % 7 < 2 { 0.35 } else { 1.15 };
        let v = ramp * texture * if y < h / 2 { 1.0 } else { 0.3 };
        [v * 1.05, v, v * 0.85]
    })
}

fn edits() -> DevelopSettings {
    let mut s = DevelopSettings::default();
    s.light.exposure = 1.5;
    s.light.highlights = -90.0;
    s.light.shadows = 75.0;
    s.light.contrast = 35.0;
    s.light.whites = 20.0;
    s.light.blacks = -25.0;
    s.effects.clarity = 40.0;
    s
}

#[derive(Debug)]
struct Error {
    mean: f64,
    p99: u8,
}

impl Error {
    fn acceptable(&self) -> bool {
        // Observed maxima on these fixtures: mean 6.057, pixel-max p99 37. Leave
        // headroom for numerical variation, while bounding both widespread
        // drift and localized tails. These are fixture bounds, not a promise
        // for arbitrary photographs, settings, resamplers, or output depths.
        self.mean < 8.0 && self.p99 <= 40
    }
}

fn error(a: &Rgba8, b: &Rgba8) -> Error {
    assert_eq!((a.width, a.height), (b.width, b.height));
    let mut sum = 0_u64;
    let mut pixel_errors: Vec<u8> = a
        .data
        .iter()
        .zip(&b.data)
        .map(|(a, b)| {
            let deltas = std::array::from_fn::<_, 3, _>(|c| a[c].abs_diff(b[c]));
            sum += deltas.iter().map(|&v| u64::from(v)).sum::<u64>();
            deltas.into_iter().max().unwrap_or(0)
        })
        .collect();
    // Mean remains over RGB samples; the tail uses each pixel's worst channel
    // so a thin artifact in just one channel is not diluted by the other two.
    let mean = sum as f64 / (pixel_errors.len() * 3) as f64;
    pixel_errors.sort_unstable();
    Error { mean, p99: pixel_errors[(pixel_errors.len() - 1) * 99 / 100] }
}

fn reduced_save(save: &Rgba8, w: usize, h: usize) -> Rgba8 {
    resize(&save.to_linear(), w, h, Filter::Box).to_srgb8()
}

fn measure(w: usize, h: usize, edges: &[usize], inject: bool) {
    let src = scene(w, h);
    let s = edits();
    let info = SourceInfo::default();
    let save = render(&src, &info, &s, &RenderRequest::fit(w, h)).image;
    for &edge in edges {
        let req = RenderRequest::fit(edge, edge);
        let preview = render(&src, &info, &s, &req).image;
        let reference = reduced_save(&save, preview.width, preview.height);
        let actual = error(&preview, &reference);
        eprintln!("{w}x{h} -> {edge}: direct {actual:?}");
        assert!(actual.acceptable(), "{w}x{h} -> {edge}: {actual:?}");

        // A separately reduced source models a proxy decode. Preserve native
        // size metadata and use the geometry stage's Mitchell prefilter here.
        let proxy = resize(&src, preview.width, preview.height, Filter::Mitchell);
        let proxy_info = SourceInfo { native_long: Some(w.max(h)), ..info.clone() };
        let from_proxy = render(&proxy, &proxy_info, &s, &req).image;
        let proxy_error = error(&from_proxy, &reference);
        eprintln!("{w}x{h} -> {edge}: proxy {proxy_error:?}");
        assert!(proxy_error.acceptable(), "{w}x{h} -> {edge} proxy: {proxy_error:?}");

        if inject {
            for control in ["exposure", "highlights"] {
                let mut broken = s.clone();
                match control {
                    "exposure" => broken.light.exposure = 0.0,
                    _ => broken.light.highlights = 0.0,
                }
                let omitted = render(&src, &info, &broken, &req).image;
                let omitted_error = error(&omitted, &reference);
                eprintln!("{w}x{h} -> {edge}: omitted {control}: {omitted_error:?}");
                // Exercise the very same acceptance predicate as the real
                // preview. A test-only omitted control must be rejected; this
                // guards against silently loosening the bounds into a no-op.
                assert!(!omitted_error.acceptable(), "omitted {control} escaped: {omitted_error:?}");
            }
        }
    }
}

#[test]
fn tail_bound_rejects_a_thin_single_channel_artifact() {
    let reference = Rgba8::filled(800, 48, [0, 0, 0, 255]);
    let mut corrupted = reference.clone();
    for pixel in &mut corrupted.data[..800] {
        pixel[0] = 255;
    }
    let actual = error(&corrupted, &reference);
    // One red row is only 0.69% of RGB samples (and mean < 2), but is 2.08%
    // of pixels. The tail must reject it using the preview's own predicate.
    assert!(actual.mean < 2.0);
    assert_eq!(actual.p99, 255);
    assert!(!actual.acceptable(), "single-channel row escaped: {actual:?}");
}

#[test]
fn preview_tracks_reduced_save_with_strong_edits() {
    // 3200 reaches the guided-filter subsample cap (sigma = 1.5% long edge),
    // unlike 800/1440. A narrow strip keeps this regression affordable in CI.
    assert_eq!(local::guided_fast_step(0.015 * 3200.0), 16);
    assert!(local::guided_fast_step(0.015 * 1440.0) < 16);
    measure(3200, 192, &[800, 1440], true);
}

#[test]
#[ignore = "larger preview/save stress measurement; run with --ignored --nocapture"]
fn large_preview_save_stress() {
    measure(6400, 768, &[800, 1440], true);
}
