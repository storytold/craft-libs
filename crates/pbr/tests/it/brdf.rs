//! The direct-light BRDF, the environment BRDF and the default environment: EffectCraft's tests on
//! this crate's API, a comparison with EffectCraft's code, glTF 2.0's closed forms and numeric
//! properties.

use std::f32::consts::PI;

use craft_pbr::{
    EnvImage, MIN_DIRECT_ROUGHNESS, Material, PbrError, Visibility, default_environment, ggx_distribution, ggx_distribution_between, hammersley,
    try_environment_brdf,
};

const VISIBILITIES: [Visibility; 3] = [Visibility::SmithSchlick, Visibility::SmithSeparable, Visibility::SmithHeightCorrelated];

use crate::reference::effectcraft as ec;

fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    [a[0] / l, a[1] / l, a[2] / l]
}

fn material(base: [f32; 3], metallic: f32, roughness: f32) -> Material {
    Material::try_new(base, metallic, roughness).unwrap()
}

/// Unit directions spread over the upper hemisphere of +Z, from Hammersley points, with `n·d` at
/// least `min_cos`.
fn hemisphere(count: u32, min_cos: f32) -> Vec<[f32; 3]> {
    (0..count)
        .filter_map(|i| {
            let [a, b] = hammersley(i, count)?;
            let z = min_cos + (1.0 - min_cos) * b;
            let s = (1.0 - z * z).max(0.0).sqrt();
            Some([s * (2.0 * PI * a).cos(), s * (2.0 * PI * a).sin(), z])
        })
        .collect()
}

// ------------------------------------------------------------------ moved from EffectCraft

#[test]
fn brdf_matches_cook_torrance_formula() {
    // Reference: f = kd·c/π + D·G·F/(4·NV·NL), returned × NL × π.
    let n = [0.0, 0.0, 1.0];
    let v = norm([0.3, 0.1, 1.0]);
    let l = norm([-0.4, 0.2, 1.0]);
    let (base, metal, rough) = ([0.8, 0.4, 0.2], 0.3f32, 0.45f32);
    let got = material(base, metal, rough).try_direct(n, v, l).unwrap();
    let h = norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    let (nv, nl, nh, vh) = (dot(n, v), dot(n, l), dot(n, h), dot(v, h));
    let a2 = (rough * rough).powi(2);
    let d = a2 / (PI * ((nh * nh) * (a2 - 1.0) + 1.0).powi(2));
    let k = (rough + 1.0).powi(2) / 8.0;
    let g = nv / (nv * (1.0 - k) + k) * nl / (nl * (1.0 - k) + k);
    for c in 0..3 {
        let f0 = 0.04 + (base[c] - 0.04) * metal;
        let f = f0 + (1.0 - f0) * (1.0 - vh).powi(5);
        let expect = ((1.0 - f) * (1.0 - metal) * base[c] / PI + d * g * f / (4.0 * nv * nl)) * nl * PI;
        assert!((got[c] - expect).abs() < 1e-5, "channel {c}: {} vs {expect}", got[c]);
    }
    // No light from behind the surface.
    assert_eq!(material(base, metal, rough).try_direct(n, v, [0.0, 0.0, -1.0]).unwrap(), [0.0; 3]);
    // A white rough dielectric lit head-on reflects about its base color (π folded in).
    let w = material([1.0; 3], 0.0, 1.0).try_direct(n, n, n).unwrap();
    assert!(w[0] > 0.95 && w[0] < 1.05, "{w:?}");
}

#[test]
fn env_brdf_endpoints() {
    let [a, b] = try_environment_brdf(1.0, 0.0).unwrap();
    assert!(a + b > 0.9 && a + b <= 1.05);
    let [a, b] = try_environment_brdf(0.1, 1.0).unwrap();
    assert!(a >= 0.0 && b > -0.01 && a + b < 1.0);
}

/// The default environment is a dim studio with a mean of 0.3, lit from up and toward +X, and
/// symmetric about the XY plane.
#[test]
fn default_environment_keeps_the_level_and_points_up() {
    let env = default_environment().unwrap();
    let (w, h) = (env.width(), env.height());
    assert_eq!((w, h), (8, 4));
    let (mut mean, mut weight) = (0.0f32, 0.0f32);
    let mut toward = [0.0f32; 3];
    for (i, p) in env.pixels().iter().enumerate() {
        let (x, y) = (i % w as usize, i / w as usize);
        let v = (y as f32 + 0.5) / h as f32;
        let sa = (v * PI).sin();
        let lum = (p[0] + p[1] + p[2]) / 3.0;
        mean += lum * sa;
        weight += sa;
        let d = craft_pbr::dir_of((x as f32 + 0.5) / w as f32, v);
        for c in 0..3 {
            toward[c] += d[c] * lum * sa;
        }
    }
    assert!((mean / weight - 0.3).abs() < 0.01, "dim studio mean: {}", mean / weight);
    let t = norm(toward);
    assert!(t[1] > 0.5, "the key is up: {t:?}");
    assert!(t[0] > 0.3, "the key is toward +X: {t:?}");
    assert!(t[2].abs() < 0.1, "symmetric about the XY plane: {t:?}");
    // No texel clips past white at the default intensity.
    assert!(env.pixels().iter().all(|p| p[0] <= 1.0 && p[0] > 0.0), "0 < radiance <= 1");
}

// ------------------------------------------------------------------ against EffectCraft's code

/// EffectCraft's environment direction `(x, y, z)`, with +Y down and the image center at +Z, is
/// `(−z, −y, −x)` in this crate's frame, and the same pixels hold the same light.
#[test]
fn default_environment_matches_effectcraft_in_this_crate_frame() {
    let ours = default_environment().unwrap();
    let (w, h, theirs) = ec::default_environment();
    assert_eq!((ours.width(), ours.height()), (w, h));
    for (a, b) in ours.pixels().iter().zip(&theirs) {
        for c in 0..3 {
            assert!((a[c] - b[c]).abs() < 1e-6, "{a:?} vs {b:?}");
        }
    }
    for (u, v) in [(0.1f32, 0.2f32), (0.5, 0.5), (0.8, 0.9)] {
        let e = ec::equirect_dir(u, v, 0.0);
        let d = craft_pbr::dir_of(u, v);
        for (x, y) in [(d[0], -e[2]), (d[1], -e[1]), (d[2], -e[0])] {
            assert!((x - y).abs() < 1e-5, "{d:?} vs {e:?}");
        }
    }
}

/// From roughness 0.25 up, the direct light matches EffectCraft's `brdf` within `1e-4` of
/// `max(1, value)`. Below it, this crate's distribution is evaluated in `f64` and EffectCraft's
/// in `f32`, which loses the narrow peak, and roughness is shaded as at least 0.032 where
/// EffectCraft used 0.03.
#[test]
fn direct_light_matches_effectcraft_for_valid_inputs() {
    let n = [0.0, 0.0, 1.0];
    let dirs = hemisphere(24, 0.05);
    for base in [[0.8, 0.4, 0.2], [1.0, 1.0, 1.0], [0.0, 0.1, 0.9]] {
        for metallic in [0.0, 0.3, 1.0] {
            for roughness in [0.25, 0.5, 0.8, 1.0] {
                for (specular, diffuse) in [(1.0, 1.0), (2.0, 0.5), (0.0, 0.0)] {
                    let m = material(base, metallic, roughness).try_with_weights(specular, diffuse).unwrap();
                    let s = ec::BrdfSurface { base, metallic, roughness, specular, diffuse };
                    for &v in &dirs {
                        for &l in &dirs {
                            let a = m.try_direct(n, v, l).unwrap();
                            let b = ec::brdf(s, n, v, l);
                            for c in 0..3 {
                                assert!((a[c] - b[c]).abs() <= 1e-4 * 1f32.max(b[c].abs()), "{a:?} vs {b:?} at {v:?} {l:?}");
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The environment BRDF matches EffectCraft's `env_brdf` wherever that gives a scale and bias of
/// at least 0 with a sum of at most 1.
#[test]
fn environment_brdf_matches_effectcraft_inside_its_valid_range() {
    for i in 0..=50 {
        for j in 0..=50 {
            let (n_v, rough) = (i as f32 / 50.0, j as f32 / 50.0);
            let (a, b) = ec::env_brdf(n_v, rough);
            if a >= 0.0 && b >= 0.0 && a + b <= 1.0 {
                let [x, y] = try_environment_brdf(n_v, rough).unwrap();
                assert!((x - a).abs() <= 1e-6 && (y - b).abs() <= 1e-6, "{x}, {y} vs {a}, {b} at {n_v}, {rough}");
            }
        }
    }
}

// ------------------------------------------------------------------ glTF 2.0 and numeric properties

/// glTF 2.0 section B.3.5's metal and dielectric BRDFs, with `vd` the specular BRDF `V D`.
fn gltf_metal(base: f32, vh: f32, vd: f32) -> f32 {
    vd * (base + (1.0 - base) * (1.0 - vh.abs()).powi(5))
}

fn gltf_dielectric(base: f32, vh: f32, vd: f32) -> f32 {
    let fr = 0.04 + (1.0 - 0.04) * (1.0 - vh.abs()).powi(5);
    base / PI + (vd - base / PI) * fr
}

/// At metallic 0 and 1 the BRDF is glTF 2.0's dielectric and metal BRDF of section B.3.5, with
/// the specular BRDF `V D` taken from this crate's geometry term and distribution.
#[test]
fn metallic_endpoints_are_the_gltf_closed_forms() {
    let n = [0.0, 0.0, 1.0];
    let dirs = hemisphere(16, 0.05);
    for base in [[0.9, 0.5, 0.1], [1.0, 1.0, 1.0], [0.0, 0.0, 0.0]] {
        for roughness in [0.1f32, 0.4, 1.0] {
            let r = roughness.max(MIN_DIRECT_ROUGHNESS);
            let k = (r + 1.0) * (r + 1.0) / 8.0;
            for &v in &dirs {
                for &l in &dirs {
                    let h = norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
                    let (nv, nl, nh, vh) = (dot(n, v), dot(n, l), dot(n, h), dot(v, h));
                    let g = nv / (nv * (1.0 - k) + k) * nl / (nl * (1.0 - k) + k);
                    let vd = g / (4.0 * nv * nl) * ggx_distribution(nh, r);
                    let metal = material(base, 1.0, roughness).try_direct(n, v, l).unwrap();
                    let dielectric = material(base, 0.0, roughness).try_direct(n, v, l).unwrap();
                    for c in 0..3 {
                        let (want_m, want_d) = (gltf_metal(base[c], vh, vd), gltf_dielectric(base[c], vh, vd));
                        let (got_m, got_d) = (metal[c] / (nl * PI), dielectric[c] / (nl * PI));
                        // `try_direct` normalizes the directions again, which moves the half vector
                        // of the narrow lobes by a few units in the last place.
                        assert!((got_m - want_m).abs() <= 1e-3 * 1f32.max(want_m), "metal {got_m} vs {want_m}");
                        assert!((got_d - want_d).abs() <= 1e-3 * 1f32.max(want_d), "dielectric {got_d} vs {want_d}");
                    }
                }
            }
        }
    }
}

/// glTF 2.0 section B.3.2's separable Smith masking for one direction.
fn gltf_g1(x: f32, a: f32) -> f32 {
    2.0 * x / (x + (a * a + (1.0 - a * a) * x * x).sqrt())
}

/// The Smith-Schlick geometry term with `k = (r + 1)² / 8` is glTF's separable Smith term at
/// roughness 1, and darker than it at grazing angles on smoother surfaces.
#[test]
fn the_geometry_term_against_the_gltf_separable_smith_form() {
    let n = [0.0, 0.0, 1.0];
    let spec = |roughness: f32, v: [f32; 3], l: [f32; 3]| {
        // A white metal reflects only its specular lobe, with F = 1.
        material([1.0; 3], 1.0, roughness).try_direct(n, v, l).unwrap()[0]
    };
    for &v in &hemisphere(16, 0.05) {
        for &l in &hemisphere(16, 0.05) {
            let h = norm([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
            let (nv, nl) = (dot(n, v), dot(n, l));
            let want = gltf_g1(nv, 1.0) * gltf_g1(nl, 1.0) * ggx_distribution(dot(n, h), 1.0) / (4.0 * nv * nl) * nl * PI;
            let got = spec(1.0, v, l);
            assert!((got - want).abs() <= 1e-4 * want.max(1.0), "{got} vs {want}");
        }
    }
    let r = 0.25f32;
    let (v, l) = (norm([0.995, 0.0, 0.1]), norm([-0.995, 0.0, 0.1]));
    let karis = spec(r, v, l);
    let ggx = gltf_g1(0.1, r * r) * gltf_g1(0.1, r * r) * ggx_distribution(1.0, r) / (4.0 * 0.1 * 0.1) * 0.1 * PI;
    assert!(karis < 0.5 * ggx, "{karis} vs {ggx}");
}

/// `n·v`, `n·l` and `n·h` in `f64` for the normal +Z.
fn cosines64(v: [f32; 3], l: [f32; 3]) -> (f64, f64, f64) {
    let unit = |d: [f64; 3]| {
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        [d[0] / len, d[1] / len, d[2] / len]
    };
    let (v, l) = (unit(v.map(f64::from)), unit(l.map(f64::from)));
    let h = unit([v[0] + l[0], v[1] + l[1], v[2] + l[2]]);
    (v[2], l[2], h[2])
}

/// glTF 2.0 section B.3.2's GGX `D` in `f64`, with `α = roughness²`.
fn gltf_d64(nh: f64, roughness: f64) -> f64 {
    let a2 = roughness.powi(4);
    a2 / (std::f64::consts::PI * (nh * nh * (a2 - 1.0) + 1.0).powi(2))
}

/// Views and lights for the visibility tests: a spread over the hemisphere and grazing pairs at
/// 89 and 89.9 degrees from the normal, mirrored and side by side.
fn visibility_pairs() -> Vec<([f32; 3], [f32; 3])> {
    let mut pairs = Vec::new();
    for &v in &hemisphere(12, 0.05) {
        for &l in &hemisphere(12, 0.05) {
            pairs.push((v, l));
        }
    }
    for deg in [89.0f32, 89.9] {
        let (s, c) = (deg.to_radians().sin(), deg.to_radians().cos());
        let head_on = [0.0, 0.0, 1.0];
        pairs.extend([([s, 0.0, c], [-s, 0.0, c]), ([s, 0.0, c], [0.0, s, c]), ([s, 0.0, c], head_on), (head_on, [s, 0.0, c])]);
    }
    pairs
}

/// The specular light of a white metal, whose `F` is 1 and whose diffuse lobe is 0, against
/// `want(n·v, n·l, α)`, a visibility computed in `f64` from a published formula.
fn check_visibility(kind: Visibility, want: impl Fn(f64, f64, f64) -> f64) {
    let n = [0.0, 0.0, 1.0];
    for roughness in [0.0f32, 0.1, 0.25, 0.5, 1.0] {
        let r = f64::from(roughness.max(MIN_DIRECT_ROUGHNESS));
        let m = material([1.0; 3], 1.0, roughness).with_visibility(kind);
        for (v, l) in visibility_pairs() {
            let (nv, nl, nh) = cosines64(v, l);
            let expect = want(nv, nl, r * r) * gltf_d64(nh, r) * nl * std::f64::consts::PI;
            let got = f64::from(m.try_direct(n, v, l).unwrap()[0]);
            assert!((got - expect).abs() <= 1e-3 * expect + 1e-6, "{kind:?} roughness {roughness} at {v:?} {l:?}: {got} vs {expect}");
        }
    }
}

/// `Visibility::SmithSeparable` is the `V` of glTF 2.0 section B.3.2, written here as the
/// section writes it, at roughness 0 (shaded at the floor) to 1 and at grazing angles.
#[test]
fn the_separable_visibility_is_gltf_section_b_3_2() {
    check_visibility(Visibility::SmithSeparable, |nv, nl, a| {
        let a2 = a * a;
        1.0 / (nl.abs() + (a2 + (1.0 - a2) * nl * nl).sqrt()) / (nv.abs() + (a2 + (1.0 - a2) * nv * nv).sqrt())
    });
}

/// `Visibility::SmithHeightCorrelated` is Heitz's height-correlated masking-shadowing
/// `G₂ = 1 / (1 + Λ(v) + Λ(l))` with the GGX `Λ = (√(1 + α² tan² θ) − 1) / 2`, divided by
/// `4 (n·v) (n·l)`, computed here from the tangents rather than in the crate's form.
#[test]
fn the_height_correlated_visibility_is_heitzs_smith_form() {
    check_visibility(Visibility::SmithHeightCorrelated, |nv, nl, a| {
        let lambda = |c: f64| ((1.0 + a * a * (1.0 - c * c) / (c * c)).sqrt() - 1.0) / 2.0;
        1.0 / (1.0 + lambda(nv) + lambda(nl)) / (4.0 * nv * nl)
    });
}

/// A material uses EffectCraft's Smith-Schlick term unless a host selects another, and the two
/// Smith forms are brighter than it at grazing angles on a smooth surface.
#[test]
fn the_default_visibility_is_effectcrafts_smith_schlick() {
    let n = [0.0, 0.0, 1.0];
    assert_eq!(Visibility::default(), Visibility::SmithSchlick);
    let m = material([0.8, 0.4, 0.2], 0.3, 0.45);
    assert_eq!(m.visibility(), Visibility::SmithSchlick);
    assert_eq!(m.try_with_weights(2.0, 0.5).unwrap().visibility(), Visibility::SmithSchlick);
    let schlick = m.with_visibility(Visibility::SmithSchlick);
    for (v, l) in visibility_pairs() {
        assert_eq!(m.try_direct(n, v, l).unwrap(), schlick.try_direct(n, v, l).unwrap());
    }
    let (v, l) = (norm([0.995, 0.0, 0.1]), norm([-0.995, 0.0, 0.1]));
    let at = |kind| material([1.0; 3], 1.0, 0.25).with_visibility(kind).try_direct(n, v, l).unwrap()[0];
    assert!(at(Visibility::SmithSeparable) > 2.0 * at(Visibility::SmithSchlick));
    assert!(at(Visibility::SmithHeightCorrelated) > at(Visibility::SmithSeparable));
}

/// A unit vector along `d`, computed in the same steps and order as the crate normalizes
/// directions: divided by its largest component, then multiplied by the reciprocal length.
fn crate_unit(d: [f32; 3]) -> [f32; 3] {
    let big = d[0].abs().max(d[1].abs()).max(d[2].abs());
    let s = [d[0] / big, d[1] / big, d[2] / big];
    let r = 1.0 / ((s[0] * s[0]) + (s[1] * s[1]) + (s[2] * s[2])).sqrt();
    [s[0] * r, s[1] * r, s[2] * r]
}

/// The light a material reflects from a uniform white sky onto a surface seen at `theta`: the
/// specular lobe by GGX importance sampling of the half vector, and the diffuse lobe by cosine
/// sampling of the light.
fn albedo(m: &Material, theta: f32, steps: u32) -> f64 {
    let n = [0.0, 0.0, 1.0];
    let v = [theta.sin(), 0.0, theta.cos()];
    let spec_only = m.try_with_weights(m.specular(), 0.0).unwrap();
    let a = m.roughness().max(MIN_DIRECT_ROUGHNESS).powi(2);
    let (mut spec, mut diff) = (0.0f64, 0.0f64);
    for i in 0..steps {
        let xi = (i as f32 + 0.5) / steps as f32;
        for j in 0..steps {
            let phi = 2.0 * PI * (j as f32 + 0.5) / steps as f32;
            // Specular: a half vector drawn from D (n·h), a light reflected about it.
            let ct = ((1.0 - xi) / (1.0 + (a * a - 1.0) * xi)).sqrt();
            let st = (1.0 - ct * ct).max(0.0).sqrt();
            let hv = [st * phi.cos(), st * phi.sin(), ct];
            let vh = dot(v, hv);
            if vh > 0.0 {
                let l = [2.0 * vh * hv[0] - v[0], 2.0 * vh * hv[1] - v[1], 2.0 * vh * hv[2] - v[2]];
                if l[2] > 0.0 {
                    // The density at the half vector `try_direct` finds again from `v` and `l`,
                    // rounded the same way and with `D` taken from the same vectors.
                    let (uv, ul) = (crate_unit(v), crate_unit(l));
                    let h = crate_unit([uv[0] + ul[0], uv[1] + ul[1], uv[2] + ul[2]]);
                    let d = ggx_distribution_between(n, h, m.roughness().max(MIN_DIRECT_ROUGHNESS)).unwrap();
                    let pdf = d * h[2] / (4.0 * dot(uv, h));
                    spec += f64::from(spec_only.try_direct(n, v, l).unwrap()[0] / PI / pdf);
                }
            }
            // Diffuse: a light drawn from n·l / π.
            let (cl, sl) = ((1.0 - xi).sqrt(), xi.sqrt());
            let l = [sl * phi.cos(), sl * phi.sin(), cl];
            let d = m.try_direct(n, v, l).unwrap()[0] - spec_only.try_direct(n, v, l).unwrap()[0];
            diff += f64::from(d / cl);
        }
    }
    (spec + diff) / f64::from(steps * steps)
}

const ANGLES: [f32; 8] = [0.0, 30.0, 60.0, 75.0, 80.0, 85.0, 89.0, 89.9];
const ROUGHNESS: [f32; 9] = [0.0, 0.01, 0.032, 0.05, 0.1, 0.25, 0.5, 0.75, 1.0];

/// A white furnace: a white metal, whose BRDF is the specular lobe alone with `F = 1`, reflects
/// at most the light it receives at every roughness and view angle, with each geometry term.
#[test]
fn a_white_metal_reflects_at_most_the_light_it_receives() {
    for kind in VISIBILITIES {
        for roughness in ROUGHNESS {
            for deg in ANGLES {
                let e = albedo(&material([1.0; 3], 1.0, roughness).with_visibility(kind), deg.to_radians(), 96);
                assert!(e <= 1.0 + 1e-3, "{kind:?} roughness {roughness} at {deg}°: {e}");
                assert!(e > 0.0, "{kind:?} roughness {roughness} at {deg}°: {e}");
            }
        }
    }
}

/// glTF 2.0 section B.3.6.3 notes that the Fresnel coupling of the diffuse and specular lobes in
/// section B.3.5 does not conserve energy. For a white dielectric the excess is largest on smooth
/// surfaces seen about 80 degrees from the normal, where it is about 9.8%.
#[test]
fn a_white_dielectric_reflects_at_most_ten_percent_more_than_it_receives() {
    for roughness in ROUGHNESS {
        for deg in ANGLES {
            let e = albedo(&material([1.0; 3], 0.0, roughness), deg.to_radians(), 96);
            assert!(e <= 1.1, "roughness {roughness} at {deg}°: {e}");
        }
    }
    assert!(albedo(&material([1.0; 3], 0.0, 0.5), 0.0, 96) <= 1.0, "head-on the lobes conserve energy");
}

#[test]
fn the_brdf_is_reciprocal() {
    let n = norm([0.1, -0.2, 1.0]);
    let dirs: Vec<[f32; 3]> = hemisphere(20, 0.2).into_iter().map(|d| norm([d[0] + 0.1, d[1] - 0.2, d[2]])).collect();
    for (base, metallic) in [([0.7, 0.3, 0.1], 0.0), ([0.9, 0.8, 0.2], 1.0), ([0.5; 3], 0.4)] {
        for (roughness, kind) in [0.05, 0.3, 0.9].into_iter().flat_map(|r| VISIBILITIES.map(|k| (r, k))) {
            let m = material(base, metallic, roughness).with_visibility(kind);
            for &v in &dirs {
                for &l in &dirs {
                    let (nv, nl) = (dot(n, v), dot(n, l));
                    if nv < 1e-2 || nl < 1e-2 {
                        continue;
                    }
                    let a = m.try_direct(n, v, l).unwrap();
                    let b = m.try_direct(n, l, v).unwrap();
                    for c in 0..3 {
                        let (x, y) = (a[c] / nl, b[c] / nv);
                        assert!((x - y).abs() <= 1e-4 * x.abs().max(1.0), "{x} vs {y}");
                    }
                }
            }
        }
    }
}

/// Roughness below `MIN_DIRECT_ROUGHNESS` is shaded as that roughness, and its highlight peak is
/// finite.
#[test]
fn roughness_is_clamped_at_the_documented_floor() {
    let n = [0.0, 0.0, 1.0];
    let (v, l) = (norm([0.2, 0.0, 1.0]), norm([-0.2, 0.0, 1.0]));
    let at = |r: f32| material([0.8; 3], 1.0, r).try_direct(n, v, l).unwrap();
    let floor = at(MIN_DIRECT_ROUGHNESS);
    assert!(floor.iter().all(|c| c.is_finite() && *c > 0.0), "{floor:?}");
    for r in [0.0, 1e-6, 0.01, 0.03, MIN_DIRECT_ROUGHNESS] {
        assert_eq!(at(r), floor, "roughness {r}");
    }
    assert!(at(MIN_DIRECT_ROUGHNESS + 0.01)[0] < floor[0], "a rougher surface has a lower peak");
}

/// When rounding after normalization gives a dot product slightly over 1, the highlight is not
/// above the head-on peak.
#[test]
fn the_highlight_is_never_above_the_head_on_peak() {
    let m = material([1.0; 3], 1.0, 0.0);
    let head_on = m.try_direct([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]).unwrap()[0];
    for d in [[1.0, 1.0, 0.5], [0.3, -0.7, 0.2], [2.0, 3.0, 7.0], [-1.0, 0.1, 0.9], [0.6, 0.6, 0.6]] {
        let got = m.try_direct(d, d, d).unwrap()[0];
        assert!(got <= head_on * (1.0 + 1e-4), "{d:?}: {got} vs {head_on}");
    }
}

/// With `n`, `v` and `l` all along one direction off the axes, the highlight at the roughness
/// floor reaches the head-on peak. An `n·h` rounded to `f32` can be one step below 1 there, which
/// gives a `D` about 20% low.
#[test]
fn the_highlight_reaches_the_head_on_peak_off_the_axes() {
    let m = material([1.0; 3], 1.0, MIN_DIRECT_ROUGHNESS);
    let head_on = m.try_direct([0.0, 0.0, 1.0], [0.0, 0.0, 1.0], [0.0, 0.0, 1.0]).unwrap()[0];
    let d = norm([0.6, 0.6, 0.6]);
    let got = m.try_direct(d, d, d).unwrap()[0];
    assert!((got - head_on).abs() <= 1e-4 * head_on, "{got} vs {head_on}");
}

/// A view at least as far below the surface as the light is above it has `n·h` of 0 or less,
/// where `D` is 0: a white metal, which reflects only its specular lobe, reflects nothing.
/// EffectCraft took `n·h` as at least 0 and gave `D` its value at 0.
#[test]
fn a_view_with_no_half_vector_above_the_surface_sees_no_specular_lobe() {
    let n = [0.0, 0.0, 1.0];
    let m = material([1.0; 3], 1.0, 0.5);
    let s = ec::BrdfSurface { base: [1.0; 3], metallic: 1.0, roughness: 0.5, specular: 1.0, diffuse: 1.0 };
    for (v, l) in [([-1.0, 0.0, -0.5], [1.0, 0.0, 0.2]), ([0.0, 1.0, -0.4], [0.0, 1.0, 0.3]), ([0.2, -1.0, -0.9], [0.5, 0.5, 0.1])] {
        let (v, l) = (norm(v), norm(l));
        assert!(dot(n, v) + dot(n, l) <= 0.0, "{v:?} {l:?}");
        assert_eq!(m.try_direct(n, v, l).unwrap(), [0.0; 3], "{v:?} {l:?}");
        assert!(ec::brdf(s, n, v, l)[0] > 0.0, "{v:?} {l:?}");
    }
}

/// The specular lobe divides by `4 (n·v) (n·l)` taken as at least `1e-4`, as EffectCraft does,
/// so at grazing angles it is lower than the expression without that floor.
#[test]
fn the_specular_denominator_is_floored_at_grazing_angles() {
    let n = [0.0, 0.0, 1.0];
    let v = norm([1.0, 0.0, 0.001]);
    let r = 0.5f32;
    // A white metal: F is 1 and the diffuse lobe is 0.
    let got = material([1.0; 3], 1.0, r).try_direct(n, v, v).unwrap()[0];
    let nv = dot(n, v);
    let k = (r + 1.0).powi(2) / 8.0;
    let g = (nv / (nv * (1.0 - k) + k)).powi(2);
    let d = ggx_distribution(nv, r);
    let floored = d * g / (4.0 * nv * nv).max(1e-4) * nv * PI;
    let unfloored = d * g / (4.0 * nv * nv) * nv * PI;
    assert!((got - floored).abs() <= 1e-3 * floored, "{got} vs {floored}");
    assert!(got < 0.05 * unfloored, "{got} vs {unfloored}");
}

#[test]
fn the_environment_brdf_keeps_reflectance_between_0_and_1() {
    for i in 0..=60 {
        for j in 0..=40 {
            let (n_v, rough) = (i as f32 / 40.0 - 0.25, j as f32 / 40.0);
            let [scale, bias] = try_environment_brdf(n_v, rough).unwrap();
            assert!(scale >= 0.0 && bias >= 0.0 && scale + bias <= 1.0, "{scale}, {bias} at {n_v}, {rough}");
        }
    }
    // A smooth surface seen at a grazing angle reflects all the light, whatever its f0.
    assert_eq!(try_environment_brdf(0.0, 0.0).unwrap(), [0.0, 1.0]);
}

#[test]
fn valid_inputs_give_finite_non_negative_light() {
    let n = [0.0, 0.0, 1.0];
    let all: Vec<[f32; 3]> = hemisphere(32, -1.0);
    for base in [[0.0; 3], [1.0; 3]] {
        for metallic in [0.0, 1.0] {
            for roughness in [0.0, 1.0] {
                for (specular, diffuse, kind) in
                    [(0.0, 0.0), (craft_pbr::MAX_SPECULAR, 1.0)].into_iter().flat_map(|(s, d)| VISIBILITIES.map(|k| (s, d, k)))
                {
                    let m = material(base, metallic, roughness).try_with_weights(specular, diffuse).unwrap().with_visibility(kind);
                    for &v in &all {
                        for &l in &all {
                            let out = m.try_direct(n, v, l).unwrap();
                            assert!(out.iter().all(|c| c.is_finite() && *c >= 0.0), "{out:?} at {v:?} {l:?}");
                        }
                        let huge = m.try_direct([0.0, 0.0, 1e30], v, [1e-30, 0.0, 1e-30]).unwrap();
                        assert!(huge.iter().all(|c| c.is_finite() && *c >= 0.0), "{huge:?}");
                    }
                    // A view straight opposite the light has no half vector.
                    assert_eq!(m.try_direct(n, [0.0, 0.6, -0.8], [0.0, -0.6, 0.8]).unwrap(), [0.0; 3]);
                }
            }
        }
    }
}

#[test]
fn values_that_are_not_finite_or_out_of_range_are_refused() {
    let bad = [f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.01, 1.01];
    for x in bad {
        assert!(matches!(Material::try_new([x, 0.5, 0.5], 0.0, 0.5), Err(PbrError::BadMaterial(_))), "base {x}");
        assert!(matches!(Material::try_new([0.5; 3], x, 0.5), Err(PbrError::BadMaterial(_))), "metallic {x}");
        assert!(matches!(Material::try_new([0.5; 3], 0.0, x), Err(PbrError::BadMaterial(_))), "roughness {x}");
        assert!(matches!(material([0.5; 3], 0.0, 0.5).try_with_weights(1.0, x), Err(PbrError::BadMaterial(_))), "diffuse {x}");
        assert!(matches!(try_environment_brdf(0.5, x), Err(PbrError::BadMaterial(_))), "environment roughness {x}");
    }
    for x in [f32::NAN, f32::INFINITY, -0.01, craft_pbr::MAX_SPECULAR + 0.01] {
        assert!(matches!(material([0.5; 3], 0.0, 0.5).try_with_weights(x, 1.0), Err(PbrError::BadMaterial(_))), "specular {x}");
    }
    for x in [f32::NAN, f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(try_environment_brdf(x, 0.5), Err(PbrError::BadDirection), "n·v {x}");
    }
    let m = material([0.5; 3], 0.0, 0.5);
    let ok = [0.0, 0.0, 1.0];
    for d in [[0.0; 3], [f32::NAN, 0.0, 1.0], [0.0, f32::INFINITY, 1.0], [f32::NEG_INFINITY, 0.0, 0.0]] {
        assert_eq!(m.try_direct(d, ok, ok), Err(PbrError::BadDirection), "normal {d:?}");
        assert_eq!(m.try_direct(ok, d, ok), Err(PbrError::BadDirection), "view {d:?}");
        assert_eq!(m.try_direct(ok, ok, d), Err(PbrError::BadDirection), "light {d:?}");
    }
}

/// The default environment is a valid environment for the rest of the crate.
#[test]
fn the_default_environment_prefilters_and_projects() {
    let env: EnvImage = default_environment().unwrap();
    let limits = craft_pbr::Limits::default();
    let mips = craft_pbr::try_prefilter(&env, &craft_pbr::PrefilterOptions::default(), &limits).unwrap();
    assert!(mips.iter().all(|m| m.pixels().iter().all(|p| p.iter().all(|c| c.is_finite() && *c >= 0.0))));
    let sh = craft_pbr::try_sh9(&env, &limits).unwrap();
    let up = craft_pbr::irradiance(&sh, [0.66, 0.75, 0.0]).unwrap()[0];
    let down = craft_pbr::irradiance(&sh, [-0.66, -0.75, 0.0]).unwrap()[0];
    assert!(up > down, "the key side is brighter: {up} vs {down}");
}
