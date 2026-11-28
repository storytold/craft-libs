//! Physically based shading of one fragment.
//!
//! - Direct light: Cook–Torrance microfacet specular with the GGX/Trowbridge–Reitz normal
//!   distribution, the Smith–Schlick-GGX geometry term (`k = (r + 1)² / 8`) and Schlick's
//!   Fresnel (`F0` = 0.04 for dielectrics, the base color for metals), plus a Lambertian
//!   diffuse lobe weighted by `(1 − F)(1 − metallic)` (Karis 2013, "Real Shading in Unreal
//!   Engine 4"; the glTF 2.0 metallic-roughness model). Light colors are the irradiance on a
//!   surface facing the light (a white 100% light shows a white diffuse surface at its base
//!   color), so π is folded into the light intensity.
//! - Image-based light: a cosine-convolved irradiance map for diffuse, a mip chain of the
//!   equirectangular environment sampled by roughness for specular, and the analytic split-sum
//!   environment BRDF fit (Karis 2014, "Physically Based Shading on Mobile").
//! - Shadows: depth maps per light, percentage-closer filtered.

use crate::scene::{EnvInfo, Light, Material, Scene, ShadowMap, TexInfo};
use crate::shadow::{NEAR, cube_maps};

pub(crate) const PI: f32 = std::f32::consts::PI;

#[inline]
pub(crate) fn dot(a: [f32; 3], b: [f32; 3]) -> f32 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}
#[inline]
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
#[inline]
fn norm(a: [f32; 3]) -> [f32; 3] {
    let l = dot(a, a).sqrt();
    // The squared length of a large finite vector overflows f32; such a vector is normalized in f64.
    if l.is_infinite() && a.iter().all(|c| c.is_finite()) {
        let a = a.map(f64::from);
        let l = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt();
        return a.map(|c| (c / l) as f32);
    }
    if l > 1e-20 { [a[0] / l, a[1] / l, a[2] / l] } else { [0.0, 0.0, -1.0] }
}
#[inline]
fn add(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] + b[0], a[1] + b[1], a[2] + b[2]]
}
#[inline]
fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
#[inline]
fn scale(a: [f32; 3], k: f32) -> [f32; 3] {
    [a[0] * k, a[1] * k, a[2] * k]
}
#[inline]
fn mix(a: f32, b: f32, t: f32) -> f32 {
    a + (b - a) * t
}

/// sRGB-encoded → linear light. The segment is chosen by magnitude and a negative value keeps
/// its sign, as in CSS Color 4's conversion code.
#[inline]
pub fn srgb_to_linear(c: f32) -> f32 {
    let a = c.abs();
    if a <= 0.04045 { c / 12.92 } else { ((a + 0.055) / 1.055).powf(2.4).copysign(c) }
}
/// Linear light → sRGB-encoded. Values above 1 continue the curve; the segment is chosen by
/// magnitude and a negative value keeps its sign, as in CSS Color 4's conversion code.
#[inline]
pub fn linear_to_srgb(c: f32) -> f32 {
    let a = c.abs();
    if a <= 0.0031308 { c * 12.92 } else { (1.055 * a.powf(1.0 / 2.4) - 0.055).copysign(c) }
}

/// GGX / Trowbridge–Reitz normal distribution, `a = roughness²`.
#[inline]
fn d_ggx(n_h: f32, a: f32) -> f32 {
    let a2 = a * a;
    let d = n_h * n_h * (a2 - 1.0) + 1.0;
    a2 / (PI * d * d).max(1e-12)
}

/// Smith–Schlick-GGX geometry term for direct light (`k = (r + 1)² / 8`).
#[inline]
fn g_smith(n_v: f32, n_l: f32, rough: f32) -> f32 {
    let k = (rough + 1.0) * (rough + 1.0) / 8.0;
    let g1 = |x: f32| x / (x * (1.0 - k) + k);
    g1(n_v) * g1(n_l)
}

/// Schlick's Fresnel approximation.
#[inline]
fn f_schlick(f0: [f32; 3], v_h: f32) -> [f32; 3] {
    let k = (1.0 - v_h).clamp(0.0, 1.0).powi(5);
    [f0[0] + (1.0 - f0[0]) * k, f0[1] + (1.0 - f0[1]) * k, f0[2] + (1.0 - f0[2]) * k]
}

/// Surface parameters used by the direct-light BRDF after texture sampling.
#[derive(Clone, Copy)]
struct BrdfSurface {
    base: [f32; 3],
    metallic: f32,
    roughness: f32,
    specular: f32,
    diffuse: f32,
}

/// Cook–Torrance BRDF × N·L × π for one light, the reflected fraction of the light's
/// irradiance per channel.
fn brdf(surface: BrdfSurface, n: [f32; 3], v: [f32; 3], l: [f32; 3]) -> [f32; 3] {
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
fn env_brdf(n_v: f32, rough: f32) -> (f32, f32) {
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r = [rough * c0[0] + c1[0], rough * c0[1] + c1[1], rough * c0[2] + c1[2], rough * c0[3] + c1[3]];
    let a004 = (r[0] * r[0]).min((-9.28 * n_v).exp2()) * r[0] + r[1];
    (-1.04 * a004 + r[2], 1.04 * a004 + r[3])
}

/// Wrap a texel index.
#[inline]
fn wrap(i: i64, n: i64, mode: u32) -> i64 {
    match mode {
        1 => i.clamp(0, n - 1),
        2 => {
            let m = i.rem_euclid(2 * n);
            if m >= n { 2 * n - 1 - m } else { m }
        }
        _ => i.rem_euclid(n),
    }
}

/// The smallest `f32` magnitude whose neighbors are 1 apart (2^23): a scaled coordinate this
/// large loses the half-texel shift.
const UNIT_STEP: f32 = 8_388_608.0;

/// Bilinear texture sample at (u, v): 0..1 across the texture, texel centers at +0.5, wrapped
/// by the texture's modes. A texture with a zero side samples as white, and a texel past the
/// end of `texels` as transparent black.
pub fn sample(t: &TexInfo, texels: &[[f32; 4]], u: f32, v: f32) -> [f32; 4] {
    let (w, h) = (t.width as i64, t.height as i64);
    if w == 0 || h == 0 {
        return [1.0; 4];
    }
    // The first texel index and the weight of the next one along an axis of `n` texels. A finite
    // coordinate whose scaled value reaches 2^23 is scaled in f64, after a repeated axis is reduced
    // by its period of 1 and a mirrored axis by its period of 2, so it keeps the half-texel shift
    // and the texel index does not saturate. A coordinate that is not finite is taken as the
    // largest finite one, so the weights below stay finite.
    let axis = |c: f32, n: i64, mode: u32| -> (i64, f32) {
        if c.is_finite() && (c * n as f32).abs() >= UNIT_STEP {
            let c = f64::from(c);
            let c = match mode {
                1 => c,
                2 => c.rem_euclid(2.0),
                _ => c.rem_euclid(1.0),
            };
            let x = c * n as f64 - 0.5;
            let x0 = x.floor();
            return (x0 as i64, (x - x0) as f32);
        }
        let x = c * n as f32 - 0.5;
        let x = if x.is_finite() { x } else { f32::MAX.copysign(x) };
        let x0 = x.floor();
        (x0 as i64, x - x0)
    };
    let ((x0, fx), (y0, fy)) = (axis(u, w, t.wrap_u), axis(v, h, t.wrap_v));
    let get = |xi: i64, yi: i64| -> [f32; 4] {
        let xi = wrap(xi, w, t.wrap_u);
        let yi = wrap(yi, h, t.wrap_v);
        // In i128, so a texture whose sides are near u32::MAX cannot overflow the offset.
        let i = i128::from(t.offset) + i128::from(yi) * i128::from(w) + i128::from(xi);
        usize::try_from(i).ok().and_then(|i| texels.get(i)).copied().unwrap_or([0.0; 4])
    };
    // A coordinate past i64 saturates; the next texel saturates too rather than overflowing.
    let (x1, y1) = (x0.saturating_add(1), y0.saturating_add(1));
    let (a, b, c, d) = (get(x0, y0), get(x1, y0), get(x0, y1), get(x1, y1));
    let mut o = [0.0; 4];
    for k in 0..4 {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bot = c[k] + (d[k] - c[k]) * fx;
        o[k] = top + (bot - top) * fy;
        // Large finite texels of opposite signs overflow the differences; such a channel is
        // interpolated in f64.
        if !o[k].is_finite() {
            let lerp = |p: f64, q: f64, t: f64| p + (q - p) * t;
            let (fx, fy) = (f64::from(fx), f64::from(fy));
            let top = lerp(f64::from(a[k]), f64::from(b[k]), fx);
            let bot = lerp(f64::from(c[k]), f64::from(d[k]), fx);
            o[k] = lerp(top, bot, fy).clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32;
        }
    }
    o
}

/// Equirectangular coordinates of a world direction (+Y down; the environment's up is −Y),
/// rotated about the vertical axis by `rot` radians.
#[inline]
fn equirect_uv(d: [f32; 3], rot: f32) -> (f32, f32) {
    let u = 0.5 + (d[0].atan2(d[2]) + rot) / (2.0 * PI);
    let v = (-d[1]).clamp(-1.0, 1.0).acos() / PI;
    (u - u.floor(), v)
}

/// Diffuse (irradiance) and roughness-filtered specular radiance of the environment.
fn env_light(s: &Scene, e: &EnvInfo, n: [f32; 3], r: [f32; 3], rough: f32) -> ([f32; 3], [f32; 3]) {
    let (u, v) = equirect_uv(n, e.rotation);
    let irr = s.textures.get(e.irradiance as usize).map_or([0.0; 4], |t| sample(t, &s.texels, u, v));
    let (u, v) = equirect_uv(r, e.rotation);
    let lv = rough.clamp(0.0, 1.0) * (e.mips.saturating_sub(1)) as f32;
    let l0 = lv.floor() as u32;
    let l1 = (l0 + 1).min(e.mips.saturating_sub(1));
    let f = lv - l0 as f32;
    let a = s.textures.get((e.radiance + l0) as usize).map_or([0.0; 4], |t| sample(t, &s.texels, u, v));
    let b = s.textures.get((e.radiance + l1) as usize).map_or([0.0; 4], |t| sample(t, &s.texels, u, v));
    // Finite levels of opposite signs near the top of the f32 range overflow the difference;
    // such a channel is blended in f64.
    let spec = [0, 1, 2].map(|k| {
        let m = mix(a[k], b[k], f);
        if m.is_finite() || !(a[k].is_finite() && b[k].is_finite()) {
            return m;
        }
        let (p, q) = (f64::from(a[k]), f64::from(b[k]));
        (p + (q - p) * f64::from(f)).clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
    });
    (scale([irr[0], irr[1], irr[2]], e.intensity), scale(spec, e.intensity))
}

/// Light-space position (u, v in shadow-map pixels, depth) of a world point.
#[inline]
fn shadow_coord(m: &ShadowMap, p: [f32; 3]) -> Option<(f32, f32, f32)> {
    let v = &m.view;
    let q = [
        v[0][0] * p[0] + v[0][1] * p[1] + v[0][2] * p[2] + v[0][3],
        v[1][0] * p[0] + v[1][1] * p[1] + v[1][2] * p[2] + v[1][3],
        v[2][0] * p[0] + v[2][1] * p[1] + v[2][2] * p[2] + v[2][3],
    ];
    let c = m.size as f32 * 0.5;
    // A finite coordinate whose product with the focal length overflows f32 is projected in f64.
    let at = |x: f32, z: f32, fast: f32| {
        if fast.is_finite() || !x.is_finite() { fast } else { (f64::from(x) * f64::from(m.focal) / f64::from(z) + f64::from(c)) as f32 }
    };
    if m.ortho {
        Some((at(q[0], 1.0, q[0] * m.focal + c), at(q[1], 1.0, q[1] * m.focal + c), q[2]))
    } else if q[2] > NEAR {
        Some((at(q[0], q[2], q[0] * m.focal / q[2] + c), at(q[1], q[2], q[1] * m.focal / q[2] + c), q[2]))
    } else {
        None
    }
}

/// Integer tap jitter hash: u32 bit math only, so a GPU backend can compute the same jitter
/// from the same inputs.
fn tap_hash(x: i32, y: i32) -> u32 {
    let mut h = (x as u32).wrapping_mul(0x9E37_79B9) ^ (y as u32).wrapping_mul(0x85EB_CA6B);
    h ^= h >> 16;
    h = h.wrapping_mul(0x2545_F491);
    h ^= h >> 15;
    h
}

/// The jitter of tap (`i`, `j`), each axis in [−0.5, 0.5): [`tap_hash`] values at the four
/// corners of the receiver's 1/8-texel lattice cell (`cx`, `cy`), blended bilinearly by the
/// receiver's position in the cell (`wx`, `wy`). The jitter is continuous in the receiver's
/// map position.
fn tap_jitter(cx: i32, cy: i32, wx: f32, wy: f32, i: i32, j: i32) -> (f32, f32) {
    let corner = |dx: i32, dy: i32| -> (f32, f32) {
        let h = tap_hash(cx.wrapping_add(dx).wrapping_add(i.wrapping_mul(7919)), cy.wrapping_add(dy).wrapping_add(j.wrapping_mul(7919)));
        (((h & 0xFFFF) as f32) * (1.0 / 65536.0) - 0.5, (((h >> 16) & 0xFFFF) as f32) * (1.0 / 65536.0) - 0.5)
    };
    let (c00, c10, c01, c11) = (corner(0, 0), corner(1, 0), corner(0, 1), corner(1, 1));
    let ax = c00.0 + (c10.0 - c00.0) * wx;
    let bx = c01.0 + (c11.0 - c01.0) * wx;
    let ay = c00.1 + (c10.1 - c00.1) * wx;
    let by = c01.1 + (c11.1 - c01.1) * wx;
    (ax + (bx - ax) * wy, ay + (by - ay) * wy)
}

/// Upper bound of the PCF grid's taps per axis.
const PCF_MAX_TAPS: i32 = 10;

/// Steepest receiver-plane slope the PCF extrapolates, in texel footprints of depth per
/// texel (about 89°). Steeper planes are clamped to this slope.
const PLANE_SLOPE_MAX: f32 = 64.0;

/// Fraction of light reaching `p` through shadow map `m` (1 = lit). `ng` is the receiver's
/// geometric (not normal-mapped) normal facing the viewer.
///
/// PCF over an N×N tap grid (5 ≤ N ≤ `PCF_MAX_TAPS`) spread over a reach derived from `radius`
/// (the penumbra in output pixels) divided by the receiver's texel footprint, each tap jittered
/// by [`tap_jitter`] and comparing against the four nearest texel centers with bilinear weights.
/// Every texel is compared against the depth the receiver's own plane has at that texel's
/// center, not the receiver point's depth.
fn shadow_factor(m: &ShadowMap, texels: &[f32], p: [f32; 3], ng: [f32; 3], n_l: f32) -> f32 {
    let Some((u, v, z)) = shadow_coord(m, p) else { return 1.0 };
    let size = m.size as i64;
    // Slope-scaled bias at texel scale (world units are pixels), steeper for grazing light.
    let texel = if m.ortho { 1.0 / m.focal } else { z.abs() / m.focal };
    let bias = m.bias * texel * (1.0 + 3.0 * (1.0 - n_l.clamp(0.0, 1.0)));
    let at = |xi: i64, yi: i64| -> f32 {
        if xi < 0 || yi < 0 || xi >= size || yi >= size {
            return f32::INFINITY; // outside the map: nothing blocks the light there
        }
        texels.get(m.offset as usize + (yi * size + xi) as usize).copied().unwrap_or(f32::INFINITY)
    };
    // The receiver plane in light space as the slope of its depth across the map, in texel
    // footprints per texel (`tx`, `ty`; depth grows along −t), clamped where the plane grazes the
    // light. `den0` is the plane normal against the light ray through the receiver (the depth
    // axis for an orthographic map), negative while it faces it.
    let w = &m.view;
    let nv = [0, 1, 2].map(|r| w[r][0] * ng[0] + w[r][1] * ng[1] + w[r][2] * ng[2]);
    let c = m.size as f32 * 0.5;
    let den0 = if m.ortho { nv[2] } else { nv[0] * ((u - c) / m.focal) + nv[1] * ((v - c) / m.focal) + nv[2] };
    let (tx, ty) = if den0 < -1e-6 {
        ((nv[0] / den0).clamp(-PLANE_SLOPE_MAX, PLANE_SLOPE_MAX), (nv[1] / den0).clamp(-PLANE_SLOPE_MAX, PLANE_SLOPE_MAX))
    } else {
        (0.0, 0.0)
    };
    // 1 where texel (xi, yi) does not occlude the receiver plane at the texel's center. The
    // perspective compare runs on 1/depth, which is linear across the map; a plane whose inverse
    // depth reaches 0 is behind the light and cannot be occluded.
    let (iz, iq) = (1.0 / z, 1.0 / (m.focal * z));
    let wide = z.is_finite() && !(m.focal * z).is_finite();
    let open = |xi: i64, yi: i64| -> f32 {
        let stored = at(xi, yi);
        if stored == f32::INFINITY {
            return 1.0;
        }
        let s = stored + bias;
        let g = tx * (xi as f32 + 0.5 - u) + ty * (yi as f32 + 0.5 - v);
        let fast = if m.ortho {
            (s.is_finite() && g.is_finite() && texel.is_finite()).then_some(z - g * texel <= s)
        } else {
            let inv = iz + g * iq;
            (!wide && s.is_finite() && inv.is_finite()).then_some(inv <= 0.0 || inv * s >= 1.0)
        };
        let lit = fast.unwrap_or_else(|| {
            let (z, focal) = (f64::from(z), f64::from(m.focal));
            let texel = if m.ortho { 1.0 / focal } else { z.abs() / focal };
            let bias = f64::from(m.bias) * texel * (1.0 + 3.0 * (1.0 - f64::from(n_l.clamp(0.0, 1.0))));
            let s = f64::from(stored) + bias;
            let g = f64::from(tx) * (xi as f64 + 0.5 - f64::from(u)) + f64::from(ty) * (yi as f64 + 0.5 - f64::from(v));
            if m.ortho {
                z - g * texel <= s
            } else {
                let inv = 1.0 / z + g / (focal * z);
                inv <= 0.0 || inv * s >= 1.0
            }
        });
        if lit { 1.0 } else { 0.0 }
    };
    // The penumbra in output pixels, converted through the receiver's texel footprint, and at
    // least two texels.
    let mut reach = (m.radius / texel).clamp(2.0, 256.0);
    if !reach.is_finite() {
        reach = 1.0;
    }
    // Every tap of a receiver more than `reach + 1` texels outside the map reads only texels
    // outside it, so the light is unblocked there. Returning here also keeps the jitter
    // lattice finite for map coordinates near `f32::MAX`.
    let (lo, hi) = (-(reach + 1.0), size as f32 + reach + 1.0);
    if !(u >= lo && u <= hi && v >= lo && v <= hi) {
        return 1.0;
    }
    let n = (reach.ceil() as i32 + 1).clamp(5, PCF_MAX_TAPS);
    let step = reach / (n - 1) as f32;
    let half = (n - 1) as f32 * 0.5;
    let (gu, gv) = (u * 8.0, v * 8.0);
    let (cx, cy) = (gu.floor() as i32, gv.floor() as i32);
    let (wx, wy) = (gu - gu.floor(), gv - gv.floor());
    let mut lit = 0.0;
    for j in 0..n {
        for i in 0..n {
            let (jx, jy) = tap_jitter(cx, cy, wx, wy, i, j);
            let (su, sv) = (u + ((i as f32 - half) + jx) * step, v + ((j as f32 - half) + jy) * step);
            // Texel (x, y) holds the depth at its center (x + 0.5, y + 0.5).
            let (tu, tv) = (su - 0.5, sv - 0.5);
            let (fx, fy) = (tu - tu.floor(), tv - tv.floor());
            let (x0, y0) = (tu.floor() as i64, tv.floor() as i64);
            let (x1, y1) = (x0.saturating_add(1), y0.saturating_add(1));
            let (h00, h10, h01, h11) = (open(x0, y0), open(x1, y0), open(x0, y1), open(x1, y1));
            let a = h00 + (h10 - h00) * fx;
            let b = h01 + (h11 - h01) * fx;
            lit += a + (b - a) * fy;
        }
    }
    lit / (n * n) as f32
}

/// The visibility of light `l` at `p` (1 = unshadowed), darkened by its shadow darkness.
fn light_shadow(s: &Scene, l: &Light, p: [f32; 3], ng: [f32; 3], n_l: f32) -> f32 {
    if l.shadow < 0 {
        return 1.0;
    }
    let first = l.shadow as usize;
    let m = if cube_maps(l) {
        // Point light, or a spot light wider than one map: the cube face of the major axis of
        // the light → point direction.
        let d = sub(p, l.pos);
        let (ax, ay, az) = (d[0].abs(), d[1].abs(), d[2].abs());
        let face = if ax >= ay && ax >= az {
            if d[0] > 0.0 { 0 } else { 1 }
        } else if ay >= az {
            if d[1] > 0.0 { 2 } else { 3 }
        } else if d[2] > 0.0 {
            4
        } else {
            5
        };
        s.shadows.get(first + face)
    } else {
        s.shadows.get(first)
    };
    let Some(m) = m else { return 1.0 };
    let f = shadow_factor(m, &s.shadow_texels, p, ng, n_l);
    1.0 - l.shadow_darkness * (1.0 - f)
}

/// The visibility of the environment light at `p` (1 = unshadowed), darkened by its shadow
/// darkness times the image's key fraction: the map occludes the dominant lobe, and the rest of
/// the environment keeps lighting the surface. An image without a dominant direction, or a
/// light without a map, shades unshadowed.
fn env_shadow(s: &Scene, e: &EnvInfo, p: [f32; 3], ng: [f32; 3], n: [f32; 3]) -> f32 {
    if e.shadow < 0 || e.sun == [0.0; 3] {
        return 1.0;
    }
    let Some(m) = s.shadows.get(e.shadow as usize) else { return 1.0 };
    let n_l = dot(n, e.sun).clamp(0.0, 1.0);
    let f = shadow_factor(m, &s.shadow_texels, p, ng, n_l);
    1.0 - e.shadow_darkness * e.key * (1.0 - f)
}

/// Unit direction from `p` to light `l`, and the distance (infinite for a parallel light).
/// For a point or spot light, two finite positions retain a finite direction when their difference
/// exceeds the `f32` range.
fn to_light(l: &Light, p: [f32; 3]) -> ([f32; 3], f64) {
    if l.kind == 0 {
        return (scale(l.dir, -1.0), f64::INFINITY);
    }
    let d = sub(l.pos, p);
    let len = dot(d, d).sqrt();
    // The squared distance to a far light overflows f32; such a direction is taken in f64.
    if len.is_infinite() {
        let d = if d.iter().all(|c| c.is_finite()) { d.map(f64::from) } else { [0, 1, 2].map(|i| f64::from(l.pos[i]) - f64::from(p[i])) };
        let len = (d[0] * d[0] + d[1] * d[1] + d[2] * d[2]).sqrt();
        return (d.map(|c| (c / len) as f32), len);
    }
    if len < 1e-6 { ([0.0, 0.0, -1.0], 0.0) } else { (scale(d, 1.0 / len), f64::from(len)) }
}

/// Distance attenuation × spot cone of a light at `p`.
fn attenuation(l: &Light, p: [f32; 3], to_l: [f32; 3], dist: f64) -> f32 {
    let narrow = dist as f32;
    let mut k = if dist.is_finite() && !narrow.is_finite() {
        falloff_wide(l, dist)
    } else {
        let dist = narrow;
        match l.falloff {
            1 if l.kind != 0 => {
                if dist <= l.radius {
                    1.0
                } else if l.falloff_distance <= 0.0 {
                    0.0
                } else {
                    let t = ((dist - l.radius) / l.falloff_distance).clamp(0.0, 1.0);
                    1.0 - t * t * (3.0 - 2.0 * t)
                }
            }
            2 if l.kind != 0 => {
                // The plateau and the inverse square use the same radius, at least 1, so the
                // factor never exceeds 1.
                let r = l.radius.max(1.0);
                if dist <= r { 1.0 } else { (r / dist).powi(2) }
            }
            _ => 1.0,
        }
    };
    let _ = p;
    if l.kind == 1 {
        let cos_a = dot(scale(to_l, -1.0), l.dir).clamp(-1.0, 1.0);
        k *= if cos_a >= l.cos_inner {
            1.0
        } else if cos_a <= l.cos_outer {
            0.0
        } else {
            // Smoothstep across the feather in angle.
            let a = cos_a.acos();
            let (ai, ao) = (l.cos_inner.acos(), l.cos_outer.acos());
            let t = ((a - ai) / (ao - ai).max(1e-6)).clamp(0.0, 1.0);
            1.0 - t * t * (3.0 - 2.0 * t)
        };
    }
    k
}

/// This function computes point and spot falloff for finite distances beyond the `f32` range.
fn falloff_wide(l: &Light, dist: f64) -> f32 {
    match l.falloff {
        1 if l.kind != 0 => {
            if l.falloff_distance <= 0.0 {
                0.0
            } else {
                let t = ((dist - f64::from(l.radius)) / f64::from(l.falloff_distance)).clamp(0.0, 1.0);
                (1.0 - t * t * (3.0 - 2.0 * t)) as f32
            }
        }
        2 if l.kind != 0 => (f64::from(l.radius.max(1.0)) / dist).powi(2) as f32,
        _ => 1.0,
    }
}

/// Inputs of one fragment.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Frag {
    pub pos: [f32; 3],
    /// Interpolated (unnormalized) vertex normal.
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    pub tangent: [f32; 4],
    pub material: u32,
    pub front: bool,
}

/// Shade a fragment: premultiplied linear RGBA, or `None` when discarded (alpha test, back
/// face of a single-sided material).
pub(crate) fn shade(s: &Scene, f: &Frag) -> Option<[f32; 4]> {
    let m: &Material = s.materials.get(f.material as usize)?;
    if !f.front && !m.double_sided {
        return None;
    }
    let tex = |i: i32| -> Option<[f32; 4]> { (i >= 0).then(|| s.textures.get(i as usize).map(|t| sample(t, &s.texels, f.uv[0], f.uv[1]))).flatten() };
    let mut base = m.base;
    if let Some(t) = tex(m.tex_base) {
        for k in 0..4 {
            base[k] *= t[k];
        }
    }
    // Mask mode tests the material's alpha (factor times texture) against the cutoff, and a
    // fragment that passes keeps only the layer's opacity. Blend mode multiplies the two.
    if m.alpha_mode == 1 && base[3] < m.alpha_cutoff {
        return None;
    }
    let blend = m.alpha_mode > 1;
    let alpha = if blend { base[3] * m.opacity } else { m.opacity };
    if blend && alpha <= 1e-5 {
        return None;
    }
    let rgb = [base[0], base[1], base[2]];
    let mut emissive = m.emissive;
    if let Some(t) = tex(m.tex_emissive) {
        emissive = [emissive[0] * t[0], emissive[1] * t[1], emissive[2] * t[2]];
    }
    if m.shadows_only {
        return Some(shadow_only(s, m, f, alpha));
    }
    if m.unlit || !m.accepts_lights || (s.lights.is_empty() && s.env.is_none() && s.ambient == [0.0; 3]) {
        let c = add(rgb, emissive);
        return Some([c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha]);
    }
    let (mut metallic, mut rough) = (m.metallic, m.roughness);
    if let Some(t) = tex(m.tex_mr) {
        rough *= t[1];
        metallic *= t[2];
    }
    let mut n = norm(f.normal);
    if !f.front {
        n = scale(n, -1.0);
    }
    // Shadow lookups follow the geometric surface, not the normal map.
    let ng = n;
    if let Some(t) = tex(m.tex_normal) {
        let tn = [(t[0] * 2.0 - 1.0) * m.normal_scale, (t[1] * 2.0 - 1.0) * m.normal_scale, t[2] * 2.0 - 1.0];
        let tg = [f.tangent[0], f.tangent[1], f.tangent[2]];
        // Gram-Schmidt against the shading normal.
        let tg = norm(sub(tg, scale(n, dot(n, tg))));
        let sign = if f.front { f.tangent[3] } else { -f.tangent[3] };
        let bt = scale(cross(n, tg), sign);
        n = norm(add(add(scale(tg, tn[0]), scale(bt, tn[1])), scale(n, tn[2])));
    }
    let ao = match tex(m.tex_occlusion) {
        Some(t) => 1.0 + m.occlusion_strength * (t[0] - 1.0),
        None => 1.0,
    };
    let v = if s.ortho { scale(s.cam_fwd, -1.0) } else { norm(sub(s.eye, f.pos)) };
    let mut out = [0.0f32; 3];
    for l in &s.lights {
        let (to_l, dist) = to_light(l, f.pos);
        let k = attenuation(l, f.pos, to_l, dist);
        if k <= 0.0 {
            continue;
        }
        let n_l = dot(n, to_l);
        if n_l <= 0.0 {
            continue;
        }
        let sh = if m.receives_shadows { light_shadow(s, l, f.pos, ng, n_l) } else { 1.0 };
        let b = brdf(BrdfSurface { base: rgb, metallic, roughness: rough, specular: m.specular_k, diffuse: m.diffuse_k }, n, v, to_l);
        for c in 0..3 {
            out[c] += b[c] * l.color[c] * k * sh;
        }
    }
    for c in 0..3 {
        out[c] += s.ambient[c] * rgb[c] * m.ambient_k * ao;
    }
    if let Some(e) = &s.env {
        let n_v = dot(n, v).clamp(1e-4, 1.0);
        let r = sub(scale(n, 2.0 * dot(n, v)), v);
        let (irr, spec) = env_light(s, e, n, r, rough);
        let f0 = [mix(0.04 * m.specular_k, rgb[0], metallic), mix(0.04 * m.specular_k, rgb[1], metallic), mix(0.04 * m.specular_k, rgb[2], metallic)];
        let (a, b) = env_brdf(n_v, rough);
        // The environment light's shadow darkens its own diffuse and specular.
        let sh = if m.receives_shadows { env_shadow(s, e, f.pos, ng, n) } else { 1.0 };
        for c in 0..3 {
            let fs = f0[c] * a + b;
            out[c] += (irr[c] * rgb[c] * (1.0 - fs) * (1.0 - metallic) * m.diffuse_k + spec[c] * fs) * ao * sh;
        }
    }
    let c = add(out, emissive);
    Some([c[0] * alpha, c[1] * alpha, c[2] * alpha, alpha])
}

/// A material that draws only the shadows it receives: black at the strongest shadow any light
/// casts on the fragment, nothing where it is unshadowed, takes no lights or receives no shadows.
fn shadow_only(s: &Scene, m: &Material, f: &Frag, alpha: f32) -> [f32; 4] {
    if m.unlit || !m.accepts_lights || !m.receives_shadows {
        return [0.0; 4];
    }
    let n = if f.front { norm(f.normal) } else { scale(norm(f.normal), -1.0) };
    let mut amount = 0.0f32;
    for l in &s.lights {
        let (to_l, dist) = to_light(l, f.pos);
        if attenuation(l, f.pos, to_l, dist) <= 0.0 {
            continue;
        }
        let n_l = dot(n, to_l);
        if n_l <= 0.0 {
            continue;
        }
        amount = amount.max(1.0 - light_shadow(s, l, f.pos, n, n_l));
    }
    // The environment light's shadow counts too (it reaches every surface: no falloff).
    if let Some(e) = &s.env {
        amount = amount.max(1.0 - env_shadow(s, e, f.pos, n, n));
    }
    [0.0, 0.0, 0.0, alpha * amount]
}

#[cfg(test)]
mod tests {
    use super::{attenuation, shadow_coord, shadow_factor, to_light};
    use crate::scene::{Light, ShadowMap};

    #[test]
    fn wide_light_distances_keep_smooth_and_inverse_square_falloff() {
        for falloff in [1, 2] {
            let radius = 3e38f32;
            let length = (2.0 * f64::from(radius).powi(2)).sqrt();
            let light = Light {
                kind: 2,
                pos: [radius, radius, 0.0],
                dir: [0.0, 0.0, 1.0],
                color: [1.0; 3],
                cos_inner: 1.0,
                cos_outer: 1.0,
                falloff,
                radius,
                falloff_distance: (2.0 * (length - f64::from(radius))) as f32,
                shadow: -1,
                shadow_darkness: 0.0,
                shadow_diffusion: 0.0,
            };
            let (direction, distance) = to_light(&light, [0.0; 3]);
            let amount = attenuation(&light, [0.0; 3], direction, distance);
            assert!((amount - 0.5).abs() < 1e-5, "falloff {falloff} retains half intensity, got {amount}");
        }
    }

    #[test]
    fn tiny_orthographic_focal_keeps_empty_maps_unoccluded() {
        let view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        let map = ShadowMap { view, focal: 1e-40, ortho: true, size: 8, offset: 0, bias: 0.0, radius: 0.0 };
        let at = [0.0, 0.0, 1.0];
        let normal = [0.0, 0.0, -1.0];
        assert_eq!(shadow_factor(&map, &[f32::INFINITY; 64], at, normal, 1.0), 1.0);
        assert_eq!(shadow_factor(&map, &[0.5; 64], at, normal, 1.0), 0.0);
        assert_eq!(shadow_factor(&map, &[2.0; 64], at, normal, 1.0), 1.0);
    }

    #[test]
    fn finite_opposite_light_positions_keep_a_finite_direction() {
        for kind in [1, 2] {
            let light = Light {
                kind,
                pos: [3e38, 0.0, 0.0],
                dir: [0.0, 0.0, 1.0],
                color: [1.0; 3],
                cos_inner: 0.9999,
                cos_outer: 0.9999,
                falloff: 0,
                radius: 0.0,
                falloff_distance: 0.0,
                shadow: -1,
                shadow_darkness: 0.0,
                shadow_diffusion: 0.0,
            };
            let (expected, _) = to_light(&light, [0.0; 3]);
            assert!(expected.iter().all(|value| value.is_finite()));
            let (direction, _) = to_light(&light, [-3e38, 0.0, 0.0]);
            assert_eq!(direction, expected, "light kind {kind} keeps the direction between collinear finite positions");
        }
    }

    #[test]
    fn a_receiver_whose_projection_overflows_f32_is_projected_in_f64() {
        // 1e37 × 400 overflows f32; divided by the depth of 1e38 it is 40 texels from the center.
        let view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        let m = ShadowMap { view, focal: 400.0, ortho: false, size: 512, offset: 0, bias: 1.5, radius: 1.0 };
        let (u, v, z) = shadow_coord(&m, [1e37, 0.0, 1e38]).unwrap_or((f32::NAN, f32::NAN, f32::NAN));
        assert!((u - 296.0).abs() < 1e-3 && v == 256.0 && z == 1e38, "{u} {v} {z}");
    }
}
