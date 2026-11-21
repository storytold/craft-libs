//! Numeric properties of the filtering: constants stay constant, GGX is normalized, energy is
//! kept, and the SH projection matches analytic irradiance.

use craft_pbr::{EnvImage, Environment, Limits, PrefilterOptions, dir_of, ggx_distribution, irradiance, try_prefilter, try_sh9};

use crate::{energy, env_from, max_rel_diff, opts};

#[test]
fn a_constant_environment_prefilters_to_itself_at_every_roughness() {
    // 100 is above the cap of 64 that the apps' copies always applied; the default has no cap.
    for value in [0.25, 3.0, 100.0] {
        let env = env_from(64, 32, |_, _| [value, value * 0.5, value * 2.0]);
        let mips = try_prefilter(&env, &PrefilterOptions::default(), &Limits::default()).unwrap();
        assert_eq!(mips.len(), 6);
        for (l, m) in mips.iter().enumerate() {
            let want = vec![[value, value * 0.5, value * 2.0]; m.pixels().len()];
            let d = max_rel_diff(m.pixels(), &want);
            assert!(d < 1e-5, "level {l} of {value} differs by {d}");
        }
    }
}

#[test]
fn the_firefly_cap_applies_only_when_set() {
    let env = env_from(64, 32, |_, _| [100.0; 3]);
    let options = opts(|o| o.firefly_cap = Some(64.0));
    let mips = try_prefilter(&env, &options, &Limits::default()).unwrap();
    assert_eq!(mips[0].pixels()[0], [100.0; 3], "level 0 is never capped");
    assert!((mips[3].pixels()[0][0] - 64.0).abs() < 1e-3, "{:?}", mips[3].pixels()[0]);
}

#[test]
fn roughness_zero_is_the_source_bit_for_bit() {
    let limits = Limits::default();
    let env = Environment::Sunset.try_render(128, &limits).unwrap();
    let one = try_prefilter(&env, &opts(|o| o.levels = 1), &limits).unwrap();
    assert_eq!(one, vec![env.clone()]);
    let six = try_prefilter(&env, &opts(|o| o.firefly_cap = Some(64.0)), &limits).unwrap();
    assert_eq!(six[0], env);
}

#[test]
fn ggx_projected_area_integrates_to_one() {
    // ∫ D(h) (n·h) dω over the hemisphere = 2π ∫₀¹ D(μ) μ dμ, by the midpoint rule in μ = n·h.
    let steps = 1_000_000;
    for roughness in [0.1, 0.3, 0.5, 0.8, 1.0] {
        let sum: f64 = (0..steps)
            .map(|i| {
                let mu = (i as f64 + 0.5) / steps as f64;
                f64::from(ggx_distribution(mu as f32, roughness)) * mu
            })
            .sum();
        let integral = sum / steps as f64 * std::f64::consts::TAU;
        assert!((integral - 1.0).abs() < 1e-3, "roughness {roughness}: {integral}");
    }
    // The peak 1 / (π α²) at n·h = 1, with α at its floor of 0.001 for roughness 0.
    for (roughness, alpha) in [(0.0f32, 1e-3f64), (0.05, 0.0025), (0.2, 0.04)] {
        let peak = 1.0 / (std::f64::consts::PI * alpha * alpha);
        let d = f64::from(ggx_distribution(1.0, roughness));
        assert!((d / peak - 1.0).abs() < 1e-3, "roughness {roughness}: {d} for {peak}");
    }
}

/// The radiance of each level at least 32 pixels wide integrated over the sphere, relative to the
/// source's. Narrower levels are left out: bilinear lookups between their few pixel centers miss
/// the peak of a lobe, so the measurement, not the filtering, loses energy there.
fn energy_ratios(env: &EnvImage) -> Vec<f64> {
    let source = energy(env);
    let mips = try_prefilter(env, &PrefilterOptions::default(), &Limits::default()).unwrap();
    mips.iter().filter(|m| m.width() >= 32).map(|m| energy(m)[0] / source[0]).collect()
}

/// Largest change of the light energy of a level, relative to the source. The 128 samples read a
/// box pyramid, so small bright lights lose some energy as roughness grows: on 256-wide sources
/// the most measured is 5.3%, the Warehouse lights at roughness 0.6.
const ENERGY_TOLERANCE: f64 = 0.07;

#[test]
fn prefiltering_keeps_the_light_energy() {
    let limits = Limits::default();
    for e in Environment::ALL {
        let env = e.try_render(256, &limits).unwrap();
        let ratios = energy_ratios(&env);
        eprintln!("{}: energy ratios {ratios:?}", e.name());
        assert_eq!(ratios.len(), 4);
        for (l, r) in ratios.iter().enumerate() {
            assert!((r - 1.0).abs() < ENERGY_TOLERANCE, "{} level {l}: energy ratio {r}", e.name());
        }
    }
    // A small bright patch facing +Z on a dim background.
    let patch = env_from(256, 128, |x, y| if (60..68).contains(&x) && (60..68).contains(&y) { [500.0; 3] } else { [0.1; 3] });
    let ratios = energy_ratios(&patch);
    eprintln!("patch: energy ratios {ratios:?}");
    for (l, r) in ratios.iter().enumerate() {
        assert!((r - 1.0).abs() < ENERGY_TOLERANCE, "patch level {l}: energy ratio {r}");
    }
}

#[test]
fn a_bright_patch_lights_the_normals_facing_it() {
    let env = env_from(128, 64, |x, y| if (30..34).contains(&x) && (30..34).contains(&y) { [500.0; 3] } else { [0.1; 3] });
    let mips = try_prefilter(&env, &PrefilterOptions::default(), &Limits::default()).unwrap();
    for m in &mips[1..] {
        let toward = m.sample(dir_of(0.25, 0.5)).unwrap()[0];
        let away = m.sample(dir_of(0.75, 0.5)).unwrap()[0];
        assert!(toward > 4.0 * away, "{}x{}: {toward} toward, {away} away", m.width(), m.height());
    }
}

#[test]
fn sh9_matches_analytic_irradiance_of_linear_and_quadratic_fields() {
    let limits = Limits::default();
    let field = |f: fn([f32; 3]) -> f32| {
        env_from(128, 64, move |x, y| {
            let d = dir_of((x as f32 + 0.5) / 128.0, (y as f32 + 0.5) / 64.0);
            [f(d); 3]
        })
    };
    // L = 1 + y / 2 gives E / π = 1 + y / 3; L = x² gives E / π = 1/3 + (x² − 1/3) / 4.
    let linear = try_sh9(&field(|d| 1.0 + 0.5 * d[1]), &limits).unwrap();
    let quadratic = try_sh9(&field(|d| d[0] * d[0]), &limits).unwrap();
    for n in [[0.0, 1.0, 0.0], [0.0, -1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, 1.0], [0.6, 0.0, 0.8], [0.48, 0.6, -0.64]] {
        let e = irradiance(&linear, n).unwrap()[0];
        assert!((e - (1.0 + n[1] / 3.0)).abs() < 0.02, "linear at {n:?}: {e}");
        let e = irradiance(&quadratic, n).unwrap()[0];
        let want = 1.0 / 3.0 + (n[0] * n[0] - 1.0 / 3.0) / 4.0;
        assert!((e - want).abs() < 0.02, "quadratic at {n:?}: {e} for {want}");
    }
}

#[test]
fn sh9_weights_a_row_near_the_pole_by_its_solid_angle() {
    // Only the top row is lit. Its solid angle is 2π (1 − cos(π / h)), so the constant band of
    // the irradiance is L (1 − cos(π / h)) / 2, at every width, also above 128 pixels.
    for (w, h) in [(64, 32), (256, 128), (512, 256)] {
        let env = env_from(w, h, |_, y| if y == 0 { [1000.0; 3] } else { [0.0; 3] });
        let c = try_sh9(&env, &Limits::default()).unwrap();
        let expected = 1000.0 * (1.0 - (std::f64::consts::PI / h as f64).cos()) / 2.0;
        let dc = f64::from(c[0][0]) * 0.282_094_791_773_878_14;
        assert!((dc / expected - 1.0).abs() < 1e-4, "{w}x{h}: {dc} for {expected}");
    }
}

#[test]
fn sh9_of_a_uniform_environment_is_uniform_at_any_height() {
    // Each pixel is projected with the basis integrated over its area, so the directional terms
    // of a uniform image vanish even with one or two rows or columns.
    for (w, h) in [(1, 1), (4, 1), (1, 3), (4, 2), (3, 5), (16, 8), (64, 32)] {
        let c = try_sh9(&env_from(w, h, |_, _| [1.0; 3]), &Limits::default()).unwrap();
        for n in [[0.0, 1.0, 0.0], [1.0, 0.0, 0.0], [0.0, 0.0, -1.0], [0.3, -0.8, 0.5]] {
            let e = irradiance(&c, n).unwrap()[0];
            assert!((e - 1.0).abs() < 1e-4, "{w}x{h} at {n:?}: {e}");
        }
    }
}

#[test]
fn halving_an_odd_side_keeps_the_last_column_and_row() {
    let limits = Limits::default();
    // Only the last column of a 5 × 3 image is lit, then only the last row.
    for (w, h, lit) in [(5u32, 3u32, true), (3, 5, false)] {
        let env = env_from(w, h, |x, y| if (lit && x == w - 1) || (!lit && y == h - 1) { [9.0; 3] } else { [0.0; 3] });
        let half = env.try_half(&limits).unwrap();
        let mean = |e: &EnvImage| e.pixels().iter().map(|p| f64::from(p[0])).sum::<f64>() / e.pixels().len() as f64;
        assert!((mean(&half) - mean(&env)).abs() < 1e-5, "{w}x{h}: mean {} for {}", mean(&half), mean(&env));
    }
}

#[test]
fn halving_averages_the_source_area_each_pixel_covers() {
    let limits = Limits::default();
    let value = |x: u32, y: u32| f64::from((x * 7 + y * 13) % 11) + 0.5;
    // The share of source pixel `s` inside output pixel `i` when `src` pixels become `dst`.
    let cover = |src: u32, dst: u32, i: u32, s: u32| {
        let scale = f64::from(src) / f64::from(dst);
        let (lo, hi) = (f64::from(i) * scale, f64::from(i + 1) * scale);
        (hi.min(f64::from(s) + 1.0) - lo.max(f64::from(s))).max(0.0) / scale
    };
    for (w, h) in [(1u32, 1u32), (3, 1), (1, 3), (5, 3), (3, 5), (7, 4), (4, 7), (9, 9), (2047, 1)] {
        let env = env_from(w, h, |x, y| [value(x, y) as f32; 3]);
        let half = env.try_half(&limits).unwrap();
        let (hw, hh) = (half.width(), half.height());
        assert_eq!((hw, hh), ((w / 2).max(1), (h / 2).max(1)));
        for oy in 0..hh {
            for ox in 0..hw {
                let mut want = 0.0;
                for sy in 0..h {
                    for sx in 0..w {
                        want += value(sx, sy) * cover(w, hw, ox, sx) * cover(h, hh, oy, sy);
                    }
                }
                let got = f64::from(half.pixels()[(oy * hw + ox) as usize][0]);
                assert!((got - want).abs() < 1e-4 * want.max(1.0), "{w}x{h} at ({ox}, {oy}): {got} for {want}");
            }
        }
    }
}
