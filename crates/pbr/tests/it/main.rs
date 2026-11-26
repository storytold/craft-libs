//! Integration tests: one test binary for the crate, one module per area.
// Unwrapping and panicking on failure is fine here, unlike in shipped code (AGENTS.md › Robustness).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

mod bench;
mod bounds;
mod brdf;
mod differential;
mod moved;
mod numeric;
mod wgsl;

/// Verbatim copies of the apps' previous code, run by `differential` and `brdf`.
mod reference {
    pub mod effectcraft;
    pub mod materialcraft;
    pub mod texturecraft;
}

use craft_pbr::{EnvImage, Limits, PrefilterOptions};

/// The default options with `f` applied.
pub fn opts(f: impl FnOnce(&mut PrefilterOptions)) -> PrefilterOptions {
    let mut o = PrefilterOptions::default();
    f(&mut o);
    o
}

/// The default limits with `f` applied.
pub fn limits(f: impl FnOnce(&mut Limits)) -> Limits {
    let mut l = Limits::default();
    f(&mut l);
    l
}

/// The largest difference between two pixel lists, relative to `max(1, |a|, |b|)` per value.
/// Lists of different lengths, and a NaN or infinite value on either side, differ by infinity.
pub fn max_rel_diff(a: &[[f32; 3]], b: &[[f32; 3]]) -> f32 {
    if a.len() != b.len() {
        return f32::INFINITY;
    }
    let rel = |x: f32, y: f32| {
        if x.is_finite() && y.is_finite() { (x - y).abs() / 1f32.max(x.abs()).max(y.abs()) } else { f32::INFINITY }
    };
    a.iter().zip(b).flat_map(|(p, q)| p.iter().zip(q).map(|(x, y)| rel(*x, *y))).fold(0.0, f32::max)
}

#[test]
fn max_rel_diff_counts_non_finite_values_as_infinitely_far() {
    for bad in [f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
        assert_eq!(max_rel_diff(&[[bad; 3]], &[[0.0; 3]]), f32::INFINITY, "{bad} against 0");
        assert_eq!(max_rel_diff(&[[0.0; 3]], &[[bad; 3]]), f32::INFINITY, "0 against {bad}");
        assert_eq!(max_rel_diff(&[[1.0, 1.0, 1.0]], &[[1.0, bad, 1.0]]), f32::INFINITY, "one channel {bad}");
    }
    assert_eq!(max_rel_diff(&[[f32::INFINITY; 3]], &[[f32::INFINITY; 3]]), f32::INFINITY);
    assert_eq!(max_rel_diff(&[[2.0, 0.5, 0.0]], &[[2.0, 0.5, 0.0]]), 0.0);
    assert!((max_rel_diff(&[[4.0, 0.0, 0.0]], &[[3.0, 0.0, 0.0]]) - 0.25).abs() < 1e-6);
}

/// An environment of linear pixels made by `f(x, y)`, with default limits.
pub fn env_from(w: u32, h: u32, f: impl Fn(u32, u32) -> [f32; 3]) -> EnvImage {
    let data = (0..h).flat_map(|y| (0..w).map(move |x| (x, y))).map(|(x, y)| f(x, y)).collect();
    EnvImage::try_from_linear(w, h, data, &Limits::default()).unwrap()
}

/// Radiance integrated over the sphere: bilinear lookups on a 512 × 256 grid, each times its
/// solid angle. The same grid for every level keeps coarse levels comparable with the source.
pub fn energy(env: &EnvImage) -> [f64; 3] {
    let (w, h) = (512usize, 256usize);
    let mut e = [0.0f64; 3];
    for y in 0..h {
        let v = (y as f64 + 0.5) / h as f64;
        let dw = (std::f64::consts::TAU / w as f64) * (std::f64::consts::PI / h as f64) * (std::f64::consts::PI * v).sin();
        for x in 0..w {
            let p = env.sample(craft_pbr::dir_of((x as f32 + 0.5) / w as f32, v as f32)).unwrap();
            for c in 0..3 {
                e[c] += f64::from(p[c]) * dw;
            }
        }
    }
    e
}
