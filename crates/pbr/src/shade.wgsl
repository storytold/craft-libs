// craft-pbr shading functions in WGSL: the metallic-roughness BRDF for direct light, the
// environment BRDF of the split sum, the equirect mapping, SH9 irradiance and the light of an
// environment. Each function computes the terms of the Rust function the crate README lists
// next to it. The text declares no bindings and no entry points. An app makes one module of
// its own declarations, this text and its entry points. Every name here starts with `pbr_`,
// `PBR_` or `Pbr`. Colors are linear, and nothing here encodes them for display. No function
// loops or recurses.

const PBR_PI: f32 = 3.14159265358979;
const PBR_TAU: f32 = 6.28318530717959;
// MIN_DIRECT_ROUGHNESS
const PBR_MIN_DIRECT_ROUGHNESS: f32 = 0.032;
// MAX_SPECULAR
const PBR_MAX_SPECULAR: f32 = 25.0;
// The smallest n·v the direct BRDF divides by.
const PBR_MIN_N_DOT_V: f32 = 1e-4;
// The α floor of ggx_distribution.
const PBR_MIN_ALPHA: f32 = 1e-3;
// f32::MIN_POSITIVE
const PBR_F32_MIN_POSITIVE: f32 = 1.17549435e-38;

// A material as GpuMaterial lays it out: 32 bytes, base_color at 0, metallic at 12, roughness
// at 16, specular at 20 and diffuse at 24.
struct PbrMaterial {
    base_color: vec3<f32>,
    metallic: f32,
    roughness: f32,
    specular: f32,
    diffuse: f32,
};

// Nine SH coefficients as GpuSh9 lays them out, in the order of pbr_sh_basis: red, green and
// blue in x, y and z, 16 bytes each.
struct PbrSh9 {
    c: array<vec4<f32>, 9>,
};

// The material with each value clamped to the range Material::try_new and try_with_weights
// accept: base color, metallic, roughness and diffuse from 0 to 1, specular from 0 to 25.
fn pbr_material_clamped(m: PbrMaterial) -> PbrMaterial {
    return PbrMaterial(
        clamp(m.base_color, vec3<f32>(0.0), vec3<f32>(1.0)),
        clamp(m.metallic, 0.0, 1.0),
        clamp(m.roughness, 0.0, 1.0),
        clamp(m.specular, 0.0, PBR_MAX_SPECULAR),
        clamp(m.diffuse, 0.0, 1.0),
    );
}

// `d` scaled to unit length, or the zero vector when `d` has zero length. `d` is divided by its
// largest component first, which keeps the squared length in range.
fn pbr_unit(d: vec3<f32>) -> vec3<f32> {
    let big = max(abs(d.x), max(abs(d.y), abs(d.z)));
    if (!(big > 0.0)) {
        return vec3<f32>(0.0);
    }
    let s = d / big;
    return s * inverseSqrt(dot(s, s));
}

// The reflectance at normal incidence: 0.04 × specular mixed toward the base color by metallic.
fn pbr_f0(m: PbrMaterial) -> vec3<f32> {
    let d = vec3<f32>(0.04 * m.specular);
    return d + (m.base_color - d) * m.metallic;
}

// The GGX distribution D at n_dot_h for perceptual roughness, with α = max(roughness², 0.001):
// α² / (π ((n·h)² (α² − 1) + 1)²). The inner term is computed as (1 − n·h)(1 + n·h) + (n·h)² α²,
// the same value, which keeps the peak 1 / (π α²) of small α in f32.
fn pbr_ggx_distribution(n_dot_h: f32, roughness: f32) -> f32 {
    let a = max(roughness * roughness, PBR_MIN_ALPHA);
    let a2 = a * a;
    let k = (1.0 - n_dot_h) * (1.0 + n_dot_h) + n_dot_h * n_dot_h * a2;
    return a2 / max(PBR_PI * k * k, PBR_F32_MIN_POSITIVE);
}

// Schlick's Fresnel f0 + (1 − f0) (1 − v·h)⁵, with 1 − v·h kept in 0..=1.
fn pbr_fresnel_schlick(f0: vec3<f32>, v_dot_h: f32) -> vec3<f32> {
    let x = clamp(1.0 - v_dot_h, 0.0, 1.0);
    let x2 = x * x;
    return f0 + (vec3<f32>(1.0) - f0) * (x2 * x2 * x);
}

// The Smith-Schlick-GGX geometry term for direct light, k = (roughness + 1)² / 8.
fn pbr_smith_schlick_ggx(n_dot_v: f32, n_dot_l: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let gv = n_dot_v / (n_dot_v * (1.0 - k) + k);
    let gl = n_dot_l / (n_dot_l * (1.0 - k) + k);
    return gv * gl;
}

// The light one light reflects toward the viewer: the BRDF times n·l times π, per channel, for
// a light whose color is the irradiance on a surface facing it. `n`, `v` and `l` are the
// normal, the direction to the viewer and the direction to the light; they are normalized here.
// The material is clamped with pbr_material_clamped and roughness is shaded as at least
// PBR_MIN_DIRECT_ROUGHNESS. A light at or below the surface, a view straight opposite the light
// and a zero-length direction give 0. n·v is taken as at least 1e-4, n·h as at most 1, and the
// specular denominator 4 (n·v) (n·l) as at least 1e-4.
fn pbr_direct(material: PbrMaterial, n_in: vec3<f32>, v_in: vec3<f32>, l_in: vec3<f32>) -> vec3<f32> {
    let m = pbr_material_clamped(material);
    let n = pbr_unit(n_in);
    let v = pbr_unit(v_in);
    let l = pbr_unit(l_in);
    let n_l = dot(n, l);
    if (n_l <= 0.0 || dot(v, v) == 0.0) {
        return vec3<f32>(0.0);
    }
    let h = pbr_unit(v + l);
    if (dot(h, h) == 0.0) {
        return vec3<f32>(0.0);
    }
    let n_v = max(dot(n, v), PBR_MIN_N_DOT_V);
    let n_h = clamp(dot(n, h), 0.0, 1.0);
    let v_h = max(dot(v, h), 0.0);
    let r = max(m.roughness, PBR_MIN_DIRECT_ROUGHNESS);
    let f = pbr_fresnel_schlick(pbr_f0(m), v_h);
    let spec = pbr_ggx_distribution(n_h, r) * pbr_smith_schlick_ggx(n_v, n_l, r) / max(4.0 * n_v * n_l, 1e-4);
    let kd = (vec3<f32>(1.0) - f) * (1.0 - m.metallic);
    return (kd * m.base_color / PBR_PI * m.diffuse + f * spec) * n_l * PBR_PI;
}

// The environment BRDF term of the split sum for n_dot_v and perceptual roughness, as
// (scale, bias): specular light is prefiltered radiance times f0 × scale + bias. Karis's analytic
// fit for mobile, with scale and bias at least 0 and their sum at most 1. n_dot_v and roughness
// are clamped to 0..=1.
fn pbr_environment_brdf(n_dot_v: f32, roughness: f32) -> vec2<f32> {
    let n_v = clamp(n_dot_v, 0.0, 1.0);
    let rough = clamp(roughness, 0.0, 1.0);
    let r = rough * vec4<f32>(-1.0, -0.0275, -0.572, 0.022) + vec4<f32>(1.0, 0.0425, 1.04, -0.04);
    let a004 = min(r.x * r.x, exp2(-9.28 * n_v)) * r.x + r.y;
    let scale = clamp(-1.04 * a004 + r.z, 0.0, 1.0);
    let bias = clamp(1.04 * a004 + r.w, 0.0, 1.0 - scale);
    return vec2<f32>(scale, bias);
}

// The direction of equirect coordinates (u, v) in the crate's frame: +Y up, row 0 straight up,
// u = 0 toward +X and u increasing toward +Z.
fn pbr_dir_of(u: f32, v: f32) -> vec3<f32> {
    let theta = v * PBR_PI;
    let phi = u * PBR_TAU;
    return vec3<f32>(sin(theta) * cos(phi), cos(theta), sin(theta) * sin(phi));
}

// The equirect coordinates (u, v) of a unit direction, u in 0..=1 and v in 0..=1. A direction
// straight up or down has u = 0.
fn pbr_uv_of(d: vec3<f32>) -> vec2<f32> {
    var phi = 0.0;
    if (d.x != 0.0 || d.z != 0.0) {
        phi = atan2(d.z, d.x);
    }
    let u = phi / PBR_TAU;
    return vec2<f32>(u - floor(u), acos(clamp(d.y, -1.0, 1.0)) / PBR_PI);
}

// The nine real SH basis functions at direction `d`, bands 0 to 2 in the order
// Y00, Y1-1 (y), Y10 (z), Y11 (x), Y2-2 (xy), Y2-1 (yz), Y20, Y21 (xz), Y22. `d` is normalized
// first; a zero-length direction gives nine zeros.
fn pbr_sh_basis(d_in: vec3<f32>) -> array<f32, 9> {
    let d = pbr_unit(d_in);
    if (dot(d, d) == 0.0) {
        return array<f32, 9>();
    }
    return array<f32, 9>(
        0.282095,
        0.488603 * d.y,
        0.488603 * d.z,
        0.488603 * d.x,
        1.092548 * d.x * d.y,
        1.092548 * d.y * d.z,
        0.315392 * (3.0 * d.z * d.z - 1.0),
        1.092548 * d.x * d.z,
        0.546274 * (d.x * d.x - d.y * d.y),
    );
}

// Σ c_k Y_k(n): the diffuse radiance a white surface with normal `n` reflects. Negative results,
// which ringing can give, are kept. A zero-length normal gives 0.
fn pbr_irradiance(sh: PbrSh9, n: vec3<f32>) -> vec3<f32> {
    let b = pbr_sh_basis(n);
    return sh.c[0].xyz * b[0] + sh.c[1].xyz * b[1] + sh.c[2].xyz * b[2] + sh.c[3].xyz * b[3] + sh.c[4].xyz * b[4]
        + sh.c[5].xyz * b[5] + sh.c[6].xyz * b[6] + sh.c[7].xyz * b[7] + sh.c[8].xyz * b[8];
}

// The two prefiltered levels to blend for perceptual roughness, and the weight of the second, as
// (low, high, weight). Level l of `levels` from try_prefilter is filtered for roughness
// l / (levels − 1). Roughness is clamped to 0..=1; 0 or 1 levels give (0, 0, 0).
fn pbr_specular_levels(roughness: f32, levels: u32) -> vec3<f32> {
    let last = f32(max(levels, 1u) - 1u);
    let lv = clamp(roughness, 0.0, 1.0) * last;
    let lo = floor(lv);
    let hi = min(lo + 1.0, last);
    return vec3<f32>(lo, hi, lv - lo);
}

// The light an environment reflects toward the viewer, by the split sum. `irradiance` is the
// diffuse radiance a white surface with this normal reflects (pbr_irradiance, or the app's own
// lookup), and `prefiltered` is the radiance of the prefiltered levels along the reflection of
// the view about the normal, blended as pbr_specular_levels gives. The diffuse part is weighted
// by 1 − (f0 × scale + bias) and 1 − metallic. The material is clamped with pbr_material_clamped.
fn pbr_environment_light(material: PbrMaterial, n_dot_v: f32, irradiance: vec3<f32>, prefiltered: vec3<f32>) -> vec3<f32> {
    let m = pbr_material_clamped(material);
    let ab = pbr_environment_brdf(n_dot_v, m.roughness);
    let fs = pbr_f0(m) * ab.x + vec3<f32>(ab.y);
    return irradiance * m.base_color * (vec3<f32>(1.0) - fs) * (1.0 - m.metallic) * m.diffuse + prefiltered * fs;
}
