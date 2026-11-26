//! The shading functions of EffectCraft's `crates/render/src/three_d/adv/shade.rs` and its
//! `default_environment` from `adv/scene.rs`, at 41da144555, copied as they are apart from
//! visibility and the scaling loop of `default_environment`, which iterates over the channels
//! instead of indexing them.

pub const PI: f32 = std::f32::consts::PI;

#[inline]
pub fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
pub fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    if l > 1e-20 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, -1.0] }
}
#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// GGX / Trowbridge–Reitz normal distribution, `a = roughness²`.
#[inline]
pub fn d_ggx(n_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_h * n_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d).max(1e-12)
}

/// Smith–Schlick-GGX geometry term for direct light (`k = (r + 1)² / 8`).
#[inline]
pub fn g_smith(n_v: f32, n_l: f32, rough: f32) -> f32 {
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g1 = |x: f32| x / (x * (1.0 - k) + k);
    g1(n_v) * g1(n_l)
}

/// Schlick's Fresnel approximation.
#[inline]
pub fn f_schlick(f0: [f32; 3], v_h: f32) -> [f32; 3] {
    let k = (1.0 - v_h).clamp(0.0, 1.0).powi(5);
    [f0[0] + (1.0 - f0[0]) * k, f0[1] + (1.0 - f0[1]) * k, f0[2] + (1.0 - f0[2]) * k]
}

/// Surface parameters used by the direct-light BRDF after texture sampling.
#[derive(Clone, Copy)]
pub struct BrdfSurface {
    pub base: [f32; 3],
    pub metallic: f32,
    pub roughness: f32,
    pub specular: f32,
    pub diffuse: f32,
}

/// Cook–Torrance BRDF × N·L × π for one light: (diffuse colour factor, specular colour).
/// Returned as the reflected fraction of the light's irradiance.
pub fn brdf(surface: BrdfSurface, n: [f32; 3], v: [f32; 3], l: [f32; 3]) -> [f32; 3] {
    let BrdfSurface { base, metallic, roughness: rough, specular: spec_k, diffuse: diff_k } = surface;
    let n_l = dot(n, l);
    if n_l <= 0.0 {
        return [0.0; 3];
    }
    let n_v = dot(n, v).max(1e-4);
    let h = norm(add(v, l));
    let n_h = dot(n, h).max(0.0);
    let v_h = dot(v, h).max(0.0);
    let r = rough.clamp(0.03, 1.0);
    let f0 = [mix(0.04 * spec_k, base[0], metallic), mix(0.04 * spec_k, base[1], metallic), mix(0.04 * spec_k, base[2], metallic)];
    let f = f_schlick(f0, v_h);
    let d = d_ggx(n_h, r * r);
    let g = g_smith(n_v, n_l, r);
    let spec = d * g / (4.0 * n_v * n_l).max(1e-4);
    let mut out = [0.0; 3];
    for c in 0..3 {
        let kd = (1.0 - f[c]) * (1.0 - metallic);
        out[c] = (kd * base[c] / PI * diff_k + f[c] * spec) * n_l * PI;
    }
    out
}

/// Split-sum environment BRDF (scale, bias) for `F0·A + B` (analytic fit).
#[inline]
pub fn env_brdf(n_v: f32, rough: f32) -> (f32, f32) {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r = [rough * c0[0] + c1[0], rough * c0[1] + c1[1], rough * c0[2] + c1[2], rough * c0[3] + c1[3]];
    let a004 = (r[0] * r[0]).min((-9.28 * n_v).exp2()) * r[0] + r[1];
    (-1.04 * a004 + r[2], 1.04 * a004 + r[3])
}

/// Inverse of `equirect_uv` (texel centre → direction).
pub fn equirect_dir(u: f32, v: f32, rot: f32) -> [f32; 3] {
    let phi = (u - 0.5) * 2.0 * PI - rot;
    let th = v * PI;
    [th.sin() * phi.sin(), -th.cos(), th.sin() * phi.cos()]
}

pub fn default_environment() -> (u32, u32, Vec<[f32; 4]>) {
    // The key region's centre: three quarters up, two thirds toward the viewer (the default
    // camera looks +z from -z).
    let key = [0.0f32, -0.75, -0.66];
    let (w, h) = (8u32, 4u32);
    let mut data = Vec::with_capacity((w * h) as usize);
    let (mut mean, mut weight) = (0.0f32, 0.0f32);
    for y in 0..h {
        let v = (y as f32 + 0.5) / h as f32;
        let sa = (v * std::f32::consts::PI).sin();
        for x in 0..w {
            let u = (x as f32 + 0.5) / w as f32;
            let l = 0.15 + 1.2 * dot(equirect_dir(u, v, 0.0), key).max(0.0);
            data.push([l, l, l, 1.0]);
            mean += l * sa;
            weight += sa;
        }
    }
    // A dim studio: scale the gradient to a weighted mean of exactly 0.3.
    let k = if mean.is_finite() && mean > 1e-6 { 0.3 * weight / mean } else { 1.0 };
    for p in &mut data {
        for c in p.iter_mut().take(3) {
            *c *= k;
        }
    }
    (w, h, data)
}
