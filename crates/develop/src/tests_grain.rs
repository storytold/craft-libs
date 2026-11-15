//! Film grain through the whole pipeline: which model each process renders with, and the crystal
//! grain's resolution independence (`crate::crystal`).

use crate::raster::Rgb32f;
use craft_develop_model::{DevelopSettings, Process};

use crate::finish::GrainModel;
use crate::{OutputDepth, RenderRequest, SourceInfo, render};

fn on(process: Process, amount: f64) -> DevelopSettings {
    let mut s = DevelopSettings { process: process.version(), ..Default::default() };
    s.grain.amount = amount;
    s
}

/// Display-encoded luminance (0..255, 16-bit precision) of a render of `src` at `w × h`.
fn luma(src: &Rgb32f, s: &DevelopSettings, w: usize, h: usize) -> Vec<f64> {
    let mut req = RenderRequest::fit(w, h);
    req.depth = OutputDepth::U16;
    let r = render(src, &SourceInfo::default(), s, &req);
    let Some(crate::DeepSamples::U16(v)) = r.deep.map(|d| d.samples) else { panic!("no 16-bit render") };
    v.chunks(3).map(|p| (0.2126 * p[0] as f64 + 0.7152 * p[1] as f64 + 0.0722 * p[2] as f64) / 257.0).collect()
}

fn std(v: &[f64]) -> f64 {
    let m = v.iter().sum::<f64>() / v.len() as f64;
    (v.iter().map(|x| (x - m).powi(2)).sum::<f64>() / v.len() as f64).sqrt()
}

#[test]
fn processes_before_v7_keep_the_value_noise() {
    for p in Process::ALL {
        let model = GrainModel::new(&on(p, 40.0), 2000.0, false);
        match p {
            Process::V1 | Process::V2 | Process::V3 | Process::V4 | Process::V5 | Process::V6 => {
                assert!(matches!(model, Some(GrainModel::Value(_))), "{p:?}")
            }
            Process::V7 | Process::V8 | Process::V9 => assert!(matches!(model, Some(GrainModel::Crystal(_))), "{p:?}"),
        }
        assert!(GrainModel::new(&on(p, 0.0), 2000.0, false).is_none(), "{p:?}: no grain at Amount 0");
    }
    let src = Rgb32f::filled(160, 120, [0.18; 3]);
    let (v6, v7) = (luma(&src, &on(Process::V6, 50.0), 160, 120), luma(&src, &on(Process::V7, 50.0), 160, 120));
    assert_ne!(v6, v7, "V7 renders the crystal grain");
}

/// Grain is all that V7 changes: without it, a photo renders as under V6.
#[test]
fn without_grain_v7_renders_as_v6() {
    let src = Rgb32f::filled(160, 120, [0.18, 0.4, 0.7]);
    assert_eq!(luma(&src, &on(Process::V6, 0.0), 160, 120), luma(&src, &on(Process::V7, 0.0), 160, 120));
}

/// A preview shows what the export shows, box-downscaled to its size (whole pipeline).
#[test]
fn small_render_matches_the_downscaled_large_render() {
    let src = Rgb32f::filled(300, 200, [0.18; 3]);
    let (w, h, k) = (1200, 800, 4);
    for size in [0.0, 25.0, 100.0] {
        let mut s = on(Process::V7, 60.0);
        s.grain.size = size;
        let base = luma(&src, &on(Process::V7, 0.0), w, h);
        let big: Vec<f64> = luma(&src, &s, w, h).iter().zip(&base).map(|(a, b)| a - b).collect();
        let down: Vec<f64> = (0..(w / k) * (h / k))
            .map(|q| {
                let (x, y) = (q % (w / k) * k, q / (w / k) * k);
                (0..k * k).map(|i| big[(y + i / k) * w + x + i % k]).sum::<f64>() / (k * k) as f64
            })
            .collect();
        let small = luma(&src, &s, w / k, h / k);
        let ratio = std(&small) / std(&down);
        assert!((0.95..1.05).contains(&ratio), "size {size}: preview / export grain {ratio:.3}");
        // and the same grain pixel by pixel, not just as strong: the preview's grain (less its
        // no-grain render) against the downscaled export's
        let small_base = luma(&src, &on(Process::V7, 0.0), w / k, h / k);
        let delta: Vec<f64> = small.iter().zip(&small_base).map(|(a, b)| a - b).collect();
        // (measured: at most 0.003 of 255 levels apart, the 16-bit output's rounding; the grain's
        // standard deviation is 1.4–2.9 levels, so a shifted or reseeded pattern is far outside)
        let worst = delta.iter().zip(&down).map(|(a, b)| (a - b).abs()).fold(0.0, f64::max);
        assert!(worst < 0.01, "size {size}: preview grain {worst:.4} levels from the downscaled export's");
    }
}

/// Degenerate and hostile grain settings render finite pixels under V7.
#[test]
fn hostile_grain_settings_render() {
    let src = Rgb32f::filled(40, 30, [0.5, 0.2, 0.05]);
    for (amount, size, rough) in
        [(f64::NAN, 25.0, 50.0), (1e9, -1e9, f64::INFINITY), (50.0, f64::NAN, f64::NAN), (100.0, 100.0, 100.0), (-5.0, 0.0, 0.0)]
    {
        let mut s = on(Process::V7, amount);
        s.grain.size = size;
        s.grain.roughness = rough;
        s.grain.seed = u32::MAX;
        for (w, h) in [(40, 30), (1, 1), (3, 400)] {
            assert!(luma(&src, &s, w, h).iter().all(|v| v.is_finite()), "{amount}/{size}/{rough} at {w}×{h}");
        }
    }
}
