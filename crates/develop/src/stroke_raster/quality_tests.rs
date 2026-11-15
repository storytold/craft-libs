//! How well Auto Mask strokes select on constructed scenes, [`AutoMaskModel::PerPixel`] (processes
//! 1 to 8) against [`AutoMaskModel::Smoothed`] (9): inside a noisy or textured object the paint
//! should be solid and smooth, and it should stay off the background beyond the edge.
//!
//! `cargo test -p craft-develop --lib quality_tests -- --nocapture` prints the table.

use super::*;
use crate::geometry::Frame;
use crate::masks::brush_dabs;
use craft_develop_model::{BrushStroke, DevelopSettings};

const W: usize = 400;
const H: usize = 260;
/// The object's right edge (x), the background beyond.
const EDGE: usize = 220;

/// A deterministic noise value in -1..1 per pixel and seed (sum of hashed uniforms: roughly
/// Gaussian, sd about 0.5).
fn noise(x: usize, y: usize, seed: u64) -> f32 {
    let mut acc = 0.0;
    for k in 0..4u64 {
        let mut v = (x as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
            ^ (y as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
            ^ (seed * 4 + k).wrapping_mul(0x1656_67B1_9E37_79F9);
        v ^= v >> 29;
        v = v.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        v ^= v >> 32;
        acc += (v % 10_000) as f32 / 10_000.0;
    }
    acc / 2.0 - 1.0
}

#[derive(Clone, Copy, Debug)]
enum Scene {
    /// High-ISO noise (about a third of a stop sd per pixel, and colour noise) on both sides.
    Noisy,
    /// Dark pits (a stop and a half darker, 2-3 px) over a fifth of the object, light noise.
    Textured,
    /// The edge a 12 px ramp instead of a step.
    SoftEdge,
    /// A 3 px line of the background's colour through the object at x = 150.
    ThinGap,
}

/// Object (x < EDGE, warm, darker) and background (bluish, a stop brighter).
fn scene(kind: Scene) -> (Rgb32f, Plane, Vec<bool>) {
    let obj = [0.11f32, 0.08, 0.05];
    let bg = [0.16f32, 0.2, 0.26];
    let mut inside = vec![false; W * H];
    let img = Rgb32f::from_fn(W, H, |x, y| {
        let t = match kind {
            Scene::SoftEdge => ((x as f32 - EDGE as f32) / 12.0 + 0.5).clamp(0.0, 1.0),
            _ => f32::from(u8::from(x >= EDGE)),
        };
        let gap = matches!(kind, Scene::ThinGap) && (150..153).contains(&x);
        let t = if gap { 1.0 } else { t };
        let mut c: [f32; 3] = std::array::from_fn(|i| obj[i] + (bg[i] - obj[i]) * t);
        let (lum_sd, chroma_sd) = match kind {
            Scene::Noisy => (1.0, 0.35),
            _ => (0.06, 0.02),
        };
        let g = (noise(x, y, 1) * lum_sd).exp2();
        for (i, v) in c.iter_mut().enumerate() {
            *v *= g * (1.0 + chroma_sd * noise(x, y, 2 + i as u64));
        }
        if matches!(kind, Scene::Textured) && t < 0.5 {
            // pits on a 7 px grid, jittered
            let (cx, cy) = (x / 7 * 7 + 3, y / 7 * 7 + 3);
            let pit = noise(cx, cy, 9) > -0.2;
            let (dx, dy) = (x as f32 - cx as f32, y as f32 - cy as f32);
            if pit && dx * dx + dy * dy <= 4.0 {
                c = c.map(|v| v * 0.35);
            }
        }
        c
    });
    for y in 0..H {
        for x in 0..W {
            let gap = matches!(kind, Scene::ThinGap) && (150..153).contains(&x);
            inside[y * W + x] = x < EDGE && !gap;
        }
    }
    let l = img.map(crate::local::log_lum);
    (img, l, inside)
}

fn frame() -> Frame {
    Frame::new(W, H, &DevelopSettings::default(), true)
}

fn stroke(points: Vec<Point>, auto_mask: bool) -> BrushStroke {
    BrushStroke { points, size: 0.04, feather: 50.0, flow: 100.0, auto_mask, ..BrushStroke::default() }
}

fn painted(s: &BrushStroke, img: &Rgb32f, l: &Plane, model: AutoMaskModel) -> Plane {
    let mut r = StrokeRaster::with_model(W, H, model);
    r.update(&brush_dabs(s, &frame(), W, H), img, l);
    r.to_plane()
}

/// Strokes: zigzag over the object's inside (uniformity), and down along its edge with the rim
/// over it (leakage; `drift`: every fourth sample's centre 4 px over the edge).
fn interior() -> Vec<Point> {
    // five rows across the object, each a radius below the last
    (0..5)
        .flat_map(|row| (0..40).map(move |i| (row, if row % 2 == 0 { i } else { 39 - i })))
        .map(|(row, i)| Point::new((40.0 + 3.5 * i as f64) / W as f64, (70.0 + 30.0 * row as f64) / H as f64))
        .collect()
}

fn along_edge(drift: bool) -> Vec<Point> {
    let r = 0.04 * W as f64;
    (0..40)
        .map(|i| {
            let x = if drift && i % 4 == 3 { EDGE as f64 + 4.0 } else { EDGE as f64 - 0.55 * r };
            Point::new(x / W as f64, (30.0 + 5.0 * i as f64) / H as f64)
        })
        .collect()
}

#[derive(Debug, Default)]
struct Stats {
    /// Inside the object where the plain brush paints fully, away from the edge: mean alpha, the
    /// fraction at 0.9 or more, the standard deviation and the mean absolute step between
    /// neighbours (total variation).
    mean: f32,
    full: f32,
    sd: f32,
    tv: f32,
    /// Outside the object (2 px or more beyond its edge) where the plain brush reaches: mean alpha
    /// and the fraction over 0.1.
    leak: f32,
    leak_frac: f32,
}

fn stats(a: &Plane, plain: &Plane, inside: &[bool]) -> Stats {
    let near_edge = |x: usize, y: usize| (x as i64 - EDGE as i64).abs() < 3 || (x as i64 - 151).abs() < 4 && inside.get(y * W + 140).is_some();
    let (mut n, mut sum, mut sq, mut full, mut tv) = (0f32, 0f32, 0f32, 0f32, 0f32);
    let (mut no, mut leak, mut leak_n) = (0f32, 0f32, 0f32);
    for y in 1..H - 1 {
        for x in 1..W - 1 {
            let i = y * W + x;
            if near_edge(x, y) {
                continue;
            }
            if inside[i] && plain.data[i] >= 0.95 && inside[i + 1] && inside[i + W] {
                let v = a.data[i];
                n += 1.0;
                sum += v;
                sq += v * v;
                full += f32::from(u8::from(v >= 0.9));
                tv += (a.data[i + 1] - v).abs() + (a.data[i + W] - v).abs();
            } else if !inside[i] && plain.data[i] > 0.05 {
                no += 1.0;
                leak += a.data[i];
                leak_n += f32::from(u8::from(a.data[i] > 0.1));
            }
        }
    }
    let mean = sum / n.max(1.0);
    Stats {
        mean,
        full: full / n.max(1.0),
        sd: (sq / n.max(1.0) - mean * mean).max(0.0).sqrt(),
        tv: tv / n.max(1.0),
        leak: leak / no.max(1.0),
        leak_frac: leak_n / no.max(1.0),
    }
}

fn measure(kind: Scene, points: Vec<Point>, model: AutoMaskModel) -> Stats {
    let (img, l, inside) = scene(kind);
    let plain = painted(&stroke(points.clone(), false), &img, &l, model);
    stats(&painted(&stroke(points, true), &img, &l, model), &plain, &inside)
}

#[test]
fn smoothed_auto_mask_fills_noisy_and_textured_objects_solidly_without_leaking_more() {
    let cases: [(&str, Scene, Vec<Point>); 7] = [
        ("noisy inside", Scene::Noisy, interior()),
        ("textured inside", Scene::Textured, interior()),
        ("noisy edge", Scene::Noisy, along_edge(false)),
        ("textured edge", Scene::Textured, along_edge(false)),
        ("soft edge", Scene::SoftEdge, along_edge(false)),
        ("drifting centre", Scene::Noisy, along_edge(true)),
        ("thin gap", Scene::ThinGap, interior()),
    ];
    eprintln!("{:<16} {:>8} | {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}", "case", "model", "mean", "≥0.9", "sd", "tv", "leak", "leak%");
    let mut got = Vec::new();
    for (name, kind, pts) in cases {
        for model in [AutoMaskModel::PerPixel, AutoMaskModel::Smoothed] {
            let s = measure(kind, pts.clone(), model);
            eprintln!(
                "{name:<16} {:>8} | {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3} {:>6.3}",
                if model == AutoMaskModel::PerPixel { "v1-8" } else { "v9" },
                s.mean,
                s.full,
                s.sd,
                s.tv,
                s.leak,
                s.leak_frac
            );
            got.push((name, model, s));
        }
    }
    let at = |name: &str, model| got.iter().find(|g| g.0 == name && g.1 == model).map(|g| &g.2).unwrap();
    for name in ["noisy inside", "textured inside"] {
        let (old, new) = (at(name, AutoMaskModel::PerPixel), at(name, AutoMaskModel::Smoothed));
        assert!(new.full >= 0.95 && new.full > old.full, "{name}: solid fill {} (was {})", new.full, old.full);
        assert!(new.tv < old.tv * 0.5 && new.sd < old.sd, "{name}: smooth fill tv {} sd {} (was {} {})", new.tv, new.sd, old.tv, old.sd);
    }
    // (a dab centred on the background samples the background, as in Lightroom: drifting over the
    // edge still paints there, under either model)
    for name in ["noisy edge", "textured edge", "soft edge", "drifting centre"] {
        let (old, new) = (at(name, AutoMaskModel::PerPixel), at(name, AutoMaskModel::Smoothed));
        assert!(new.leak <= old.leak + 0.02, "{name}: leakage {} (was {})", new.leak, old.leak);
    }
    assert!(at("drifting centre", AutoMaskModel::Smoothed).leak < 0.05, "a stray dab centre keeps the stroke's side");
}

/// The 3 px gap of background through the object: how much of it each model paints (closing small
/// holes also bridges a gap this thin), and a stroke that moves on into the background on purpose
/// paints it once it is a radius in.
#[test]
fn thin_gaps_and_strokes_that_move_over() {
    let (img, l, _) = scene(Scene::ThinGap);
    let pts = interior();
    for model in [AutoMaskModel::PerPixel, AutoMaskModel::Smoothed] {
        let plain = painted(&stroke(pts.clone(), false), &img, &l, model);
        let a = painted(&stroke(pts.clone(), true), &img, &l, model);
        let (mut n, mut sum) = (0.0f32, 0.0f32);
        for y in 0..H {
            for x in 150..153 {
                if plain.data[y * W + x] >= 0.95 {
                    n += 1.0;
                    sum += a.data[y * W + x];
                }
            }
        }
        eprintln!("thin gap {model:?}: mean alpha in the gap {:.3}", sum / n.max(1.0));
    }
    let (img, l, _) = scene(Scene::Noisy);
    let r = 0.04 * W as f64;
    let across: Vec<Point> = (0..40).map(|i| Point::new((EDGE as f64 - 2.0 * r + 0.25 * r * i as f64) / W as f64, 0.5)).collect();
    let end = EDGE as f64 - 2.0 * r + 0.25 * r * 39.0;
    for model in [AutoMaskModel::PerPixel, AutoMaskModel::Smoothed] {
        let a = painted(&stroke(across.clone(), true), &img, &l, model);
        let y = H / 2;
        let x0 = (EDGE as f64 + 1.5 * r) as usize;
        let row: Vec<f32> = (x0..end as usize).map(|x| a.data[y * W + x]).collect();
        let mean = row.iter().sum::<f32>() / row.len().max(1) as f32;
        eprintln!("moving over {model:?}: background a radius and a half past the edge {mean:.3}");
        if model == AutoMaskModel::Smoothed {
            assert!(mean > 0.9, "a stroke that moves over paints the other side: {mean}");
        }
    }
}
