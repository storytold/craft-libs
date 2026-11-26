//! This crate against the apps' previous code (`crate::reference`), on the same generated
//! environments. With the apps' settings (a firefly cap of 64, 128 samples, a level bias of 1),
//! every level matches within `TOLERANCE` for sources with even sides. The intended differences:
//! halving an odd side keeps the last column or row, which the previous code dropped, so levels
//! read from the pyramid of an odd-sized source differ; and the SH projection integrates the
//! basis over each pixel of the full source and sums in `f64`, so the irradiance differs within
//! `SH_TOLERANCE`.

use craft_pbr::{ColorEncoding, ENVIRONMENTS, EnvImage, Environment, Limits, PrefilterOptions, irradiance, try_prefilter, try_sh9};

use crate::reference::{materialcraft as mc, texturecraft as tc};
use crate::{max_rel_diff, opts};

/// Largest difference allowed, relative to `max(1, |value|)`.
const TOLERANCE: f32 = 1e-5;

/// Largest difference allowed in the irradiance the SH coefficients give, relative to
/// `max(1, |value|)`. The previous code took each pixel's basis value and solid angle at its
/// center, and halved sources over 128 by plain averaging; the difference is about
/// `1.3 / rows²` at 128 rows or fewer: 3.2e-4 at 64 rows, 2.9e-3 at 19.
const SH_TOLERANCE: f32 = 4e-3;

fn apps_options(levels: u32) -> PrefilterOptions {
    opts(|o| {
        o.levels = levels;
        o.firefly_cap = Some(64.0);
    })
}

fn to_reference(env: &EnvImage) -> tc::EnvImage {
    tc::EnvImage { width: env.width(), height: env.height(), data: env.pixels().to_vec() }
}

/// The largest difference of each prefiltered level and of the SH coefficients.
fn compare(env: &EnvImage, levels: u32) -> (Vec<f32>, f32) {
    let ours = try_prefilter(env, &apps_options(levels), &Limits::default()).unwrap();
    let theirs = tc::prefilter(&to_reference(env), levels);
    assert_eq!(ours.len(), theirs.len());
    let diffs = ours
        .iter()
        .zip(&theirs)
        .map(|(a, b)| {
            assert_eq!((a.width(), a.height()), (b.width, b.height));
            max_rel_diff(a.pixels(), &b.data)
        })
        .collect();
    let sh = try_sh9(env, &Limits::default()).unwrap();
    let sh_ref = tc::sh9(&to_reference(env));
    (diffs, irradiance_diff(&sh, &sh_ref))
}

/// The largest difference of the irradiance two SH sets give, over 26 normals (the axes, the
/// face diagonals and the corner diagonals), relative to `max(1, |value|)`.
fn irradiance_diff(a: &[[f32; 3]; 9], b: &[[f32; 3]; 9]) -> f32 {
    let normals = (-1..=1).flat_map(|x| (-1..=1).flat_map(move |y| (-1..=1).map(move |z| [x as f32, y as f32, z as f32])));
    let (ea, eb): (Vec<[f32; 3]>, Vec<[f32; 3]>) =
        normals.filter(|n| *n != [0.0; 3]).map(|n| (irradiance(a, n).unwrap(), irradiance(b, n).unwrap())).unzip();
    max_rel_diff(&ea, &eb)
}

fn assert_matches(label: &str, env: &EnvImage, levels: u32) {
    let (levels, sh) = compare(env, levels);
    eprintln!("{label}: level differences {levels:?}, SH difference {sh}");
    assert!(levels.iter().all(|d| *d <= TOLERANCE), "{label}: {levels:?}");
    assert!(sh <= SH_TOLERANCE, "{label}: SH {sh}");
}

#[test]
fn the_procedural_environments_match_the_apps() {
    let limits = Limits::default();
    assert_eq!(ENVIRONMENTS, tc::ENVIRONMENTS);
    for e in Environment::ALL {
        for width in [64, 128, 256] {
            let ours = e.try_render(width, &limits).unwrap();
            let theirs = tc::environment(e.name(), width);
            let d = max_rel_diff(ours.pixels(), &theirs.data);
            assert!(d <= TOLERANCE, "{} at {width}: {d}", e.name());
            assert_matches(&format!("{} at {width}", e.name()), &ours, 6);
        }
    }
}

#[test]
fn materialcraft_names_map_onto_the_environments() {
    assert_eq!(ENVIRONMENTS, mc::ENVIRONMENTS);
    for id in ["studio", "sunset", "overcast", "night", "contrast", "warehouse", "unknown"] {
        assert!(Environment::from_name(mc::by_id(id)).is_some(), "{id}");
    }
}

#[test]
fn synthetic_environments_match_the_apps() {
    let (w, h) = (128u32, 64u32);
    let make = |f: &dyn Fn(u32, u32) -> [f32; 3]| crate::env_from(w, h, f);
    assert_matches("constant 2", &make(&|_, _| [2.0, 1.0, 0.5]), 6);
    assert_matches("constant 100, over the cap", &make(&|_, _| [100.0; 3]), 6);
    // One bright texel in each quadrant and at each pole.
    for (px, py) in [(10, 20), (70, 20), (10, 44), (70, 44), (5, 0), (90, 63)] {
        assert_matches(&format!("bright texel at {px},{py}"), &make(&move |x, y| if (x, y) == (px, py) { [5000.0; 3] } else { [0.05; 3] }), 6);
    }
    assert_matches("linear field", &make(&|x, y| [x as f32 / 128.0, y as f32 / 64.0, 1.0]), 8);
    assert_matches("quadratic field", &make(&|x, y| [(x as f32 / 64.0 - 1.0).powi(2), (y as f32 / 32.0 - 1.0).powi(2), 0.5]), 3);
}

#[test]
fn odd_sizes_differ_only_where_halving_drops_an_edge() {
    for (w, h) in [(37u32, 19u32), (129, 64), (128, 65)] {
        let env = crate::env_from(w, h, |x, y| [1.0 + x as f32 / w as f32, 1.0 + y as f32 / h as f32, 1.0]);
        let (levels, sh) = compare(&env, 6);
        eprintln!("{w}x{h}: level differences {levels:?}, SH difference {sh}");
        assert_eq!(levels[0], 0.0, "{w}x{h}: level 0 is the source");
        // The previous code halved sources over 128 for SH; a smaller one projects the same pixels.
        if w <= 128 && h <= 128 {
            assert!(sh <= SH_TOLERANCE, "{w}x{h}: SH {sh}");
        }
    }
}

#[test]
fn rgba_import_matches_the_apps_at_sizes_they_keep() {
    let (w, h) = (128u32, 64u32);
    let rgba: Vec<[f32; 4]> = (0..w * h)
        .map(|i| [(i % 97) as f32 / 40.0, f32::NAN, -0.5 + (i % 13) as f32 / 4.0, 1.0])
        .map(|mut p| {
            if p[0] > 2.0 {
                p[1] = f32::INFINITY;
            }
            p
        })
        .collect();
    for (encoding, srgb) in [(ColorEncoding::Linear, false), (ColorEncoding::Srgb, true)] {
        let ours = EnvImage::try_from_rgba(w, h, &rgba, encoding, &Limits::default()).unwrap();
        let theirs = tc::EnvImage::from_rgba(w, h, &rgba, srgb).unwrap();
        assert_eq!(ours.pixels(), &theirs.data[..], "{encoding:?}");
    }
}
