//! Shadow maps: per-light sizing within a scene-wide texel budget, the light-space depth
//! rasterizer and the environment light's orthographic map.

use std::cmp::Reverse;
use std::collections::BinaryHeap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use crate::raster::f32_crossing;
use crate::scene::{EnvInfo, Light, Material, Scene, ShadowMap, TexInfo, unit, validate};
use crate::shade::sample;
use crate::{Limits, Poly, RenderError, filled, for_each_item, map_collect, try_with_capacity};

/// Hard cap on the shadow depth texels (`f32`) of one scene, about 104 MB. It holds one point
/// light's full 2048-texel cube (6 × 2048² ≈ 25.2 M texels).
pub const MAX_SHADOW_TEXELS: u64 = 26_000_000;

fn dot3(a: [f64; 3], b: [f64; 3]) -> f64 {
    a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
}

fn cross3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

fn len3(a: [f64; 3]) -> f64 {
    dot3(a, a).sqrt()
}

/// `a / |a|`, or `a` unchanged when its length is 0.
fn normalize3(a: [f64; 3]) -> [f64; 3] {
    let l = len3(a);
    if l > 0.0 { a.map(|v| v / l) } else { a }
}

fn sub3(a: [f64; 3], b: [f64; 3]) -> [f64; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}

fn scale3(a: [f64; 3], k: f64) -> [f64; 3] {
    a.map(|v| v * k)
}

/// The first three rows of the world → light transform looking from `eye` along `fwd`, with
/// `down` as the +y axis's hint: rows right, down and forward (orthonormal), applied after a
/// translation by −`eye`. A zero `fwd` looks along +z; a `down` along `fwd` is replaced by +y,
/// or by −z when `fwd` is near vertical.
fn basis(eye: [f64; 3], fwd: [f64; 3], down: [f64; 3]) -> [[f32; 4]; 3] {
    let f = normalize3(fwd);
    let f = if len3(f) == 0.0 { [0.0, 0.0, 1.0] } else { f };
    let mut d = sub3(down, scale3(f, dot3(down, f)));
    if len3(d) < 1e-9 {
        d = if f[1].abs() < 0.99 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, -1.0] };
        d = sub3(d, scale3(f, dot3(d, f)));
    }
    let d = normalize3(d);
    let r = normalize3(cross3(d, f));
    let rot = [[r[0], r[1], r[2], 0.0], [d[0], d[1], d[2], 0.0], [f[0], f[1], f[2], 0.0], [0.0, 0.0, 0.0, 1.0]];
    let mut tr = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0], [0.0, 0.0, 0.0, 1.0]];
    tr[0][3] = -eye[0];
    tr[1][3] = -eye[1];
    tr[2][3] = -eye[2];
    let mut m = [[0.0f64; 4]; 4];
    for i in 0..4 {
        for j in 0..4 {
            m[i][j] = (0..4).map(|k| rot[i][k] * tr[k][j]).sum();
        }
    }
    [0, 1, 2].map(|r| m[r].map(|v| v as f32))
}

/// Light-space near plane of a perspective shadow map. Casters are clipped against it, and the
/// shader does not shadow a receiver at this depth or nearer.
pub(crate) const NEAR: f32 = 1e-3;

/// A caster's corner in light space with its texture coordinates.
type LightCorner = ([f32; 3], [f32; 2]);

/// A corner on the map: (x, y), 1/z (1 for an orthographic map), depth and texture coordinates.
type MapCorner = ([f64; 2], f32, f32, [f32; 2]);

/// A caster triangle clipped against one plane: at most four corners, held inline.
type CasterPoly<T> = Poly<T, 4>;

/// Triangle `t`'s corners in `m`'s light space, clipped against the near plane for a
/// perspective map.
fn light_poly(s: &Scene, m: &ShadowMap, ids: [usize; 3]) -> CasterPoly<LightCorner> {
    let q: CasterPoly<LightCorner> = ids
        .iter()
        .map(|&i| {
            let p = s.vertices[i].pos;
            let v = &m.view;
            ([0, 1, 2].map(|r| v[r][0] * p[0] + v[r][1] * p[1] + v[r][2] * p[2] + v[r][3]), s.vertices[i].uv)
        })
        .collect();
    let mut poly = q;
    if !m.ortho {
        let mut out = CasterPoly::new();
        for i in 0..poly.len() {
            let (a, b) = (poly[i], poly[(i + 1) % poly.len()]);
            let (da, db) = (a.0[2] - NEAR, b.0[2] - NEAR);
            if da >= 0.0 {
                out.push(a);
            }
            if (da >= 0.0) != (db >= 0.0) {
                let (from_b, k) = f32_crossing(da, db);
                let (p, q) = if from_b { (b, a) } else { (a, b) };
                let lerp = |x: f32, y: f32| {
                    let fast = x + (y - x) * k;
                    if fast.is_finite() || !x.is_finite() || !y.is_finite() {
                        fast
                    } else {
                        (f64::from(x) + (f64::from(y) - f64::from(x)) * f64::from(k)) as f32
                    }
                };
                // The crossing lies on the near plane; its depth is set to it, since the lerp of
                // far-apart depths can round to 0 or below.
                out.push(([lerp(p.0[0], q.0[0]), lerp(p.0[1], q.0[1]), NEAR], [lerp(p.1[0], q.1[0]), lerp(p.1[1], q.1[1])]));
            }
        }
        poly = out;
    }
    poly
}

/// The light-space polygon on `m`'s texels.
fn map_corners(m: &ShadowMap, poly: &[LightCorner]) -> CasterPoly<MapCorner> {
    let c = m.size as f32 * 0.5;
    // A finite coordinate whose product with the focal length overflows f32 is projected in f64.
    let at = |x: f32, z: f32, fast: f32| {
        if fast.is_finite() || !x.is_finite() { f64::from(fast) } else { f64::from(x) * f64::from(m.focal) / f64::from(z) + f64::from(c) }
    };
    poly.iter()
        .map(|(p, uv)| {
            if m.ortho {
                ([at(p[0], 1.0, p[0] * m.focal + c), at(p[1], 1.0, p[1] * m.focal + c)], 1.0, p[2], *uv)
            } else {
                let iz = 1.0 / p[2];
                ([at(p[0], p[2], p[0] * m.focal * iz + c), at(p[1], p[2], p[1] * m.focal * iz + c)], iz, p[2], *uv)
            }
        })
        .collect()
}

/// A caster's alpha test: the base texture and the factor its alpha is multiplied by (the
/// material's base alpha times the opacity, or the base alpha alone for a masked caster). A texel
/// is skipped when its alpha is below `cutoff`.
struct AlphaTest<'a> {
    tex: &'a TexInfo,
    factor: f32,
    cutoff: f32,
}

/// The opacity a caster's material alpha is multiplied by and the cutoff it is tested against: a
/// masked caster tests the material's alpha against its cutoff, as shading does; a blended one
/// tests its alpha times the opacity against 0.5.
fn caster_alpha(mat: &Material) -> (f32, f32) {
    if mat.alpha_mode == 1 { (1.0, mat.alpha_cutoff) } else { (mat.opacity, 0.5) }
}

/// Whether a caster of material `mat` casts nothing whatever its texels: a masked or blended
/// caster without a base texture has one alpha, tested as a texel's alpha is; a masked or blended
/// caster with a base texture and a factor of 0 fails a cutoff above 0 at every texel; and an
/// opaque-mode or masked caster draws with its opacity alone, tested as a blended caster's alpha
/// is.
fn casts_nothing(mat: &Material) -> bool {
    let (opacity, cutoff) = caster_alpha(mat);
    let factor = mat.base[3] * opacity;
    let alpha_fails = if mat.tex_base < 0 { factor < cutoff } else { factor == 0.0 && cutoff > 0.0 };
    (mat.alpha_mode != 0 && alpha_fails) || (mat.alpha_mode != 2 && mat.opacity < 0.5)
}

/// Texture coordinate `i` at the barycentric weights `l` of map triangle `tri`, perspective
/// correct with depth `z` on a perspective map. A sum that overflows `f32` (a large coordinate
/// times a corner's 1/depth) is taken in `f64` and kept within `f32`'s range, so the sample
/// reduces it by its wrap period.
fn caster_uv(tri: &[MapCorner; 3], l: [f32; 3], z: f32, i: usize, ortho: bool) -> f32 {
    let u = if ortho {
        l[0] * tri[0].3[i] + l[1] * tri[1].3[i] + l[2] * tri[2].3[i]
    } else {
        (l[0] * tri[0].3[i] * tri[0].1 + l[1] * tri[1].3[i] * tri[1].1 + l[2] * tri[2].3[i] * tri[2].1) * z
    };
    if u.is_finite() {
        return u;
    }
    let (iz, z) = if ortho { ([1.0; 3], 1.0) } else { ([tri[0].1, tri[1].1, tri[2].1].map(f64::from), f64::from(z)) };
    let sum: f64 = (0..3).map(|k| f64::from(l[k]) * f64::from(tri[k].3[i]) * iz[k]).sum();
    (sum * z).clamp(f64::from(f32::MIN), f64::from(f32::MAX)) as f32
}

impl AlphaTest<'_> {
    fn discards(&self, a: f32) -> bool {
        a < self.cutoff
    }
}

/// A map triangle's signed area and the texels of `m` inside its bounding box (`x0..x1`,
/// `y0..y1`), or `None` when its area is too small to rasterize.
fn texel_bounds(m: &ShadowMap, tri: &[MapCorner; 3]) -> Option<(f64, [usize; 4])> {
    let n = m.size as usize;
    let (a, b, cc) = (tri[0].0, tri[1].0, tri[2].0);
    let area = (b[0] - a[0]) * (cc[1] - a[1]) - (b[1] - a[1]) * (cc[0] - a[0]);
    if area.abs() < 1e-12 {
        return None;
    }
    let x0 = a[0].min(b[0]).min(cc[0]).floor().max(0.0) as usize;
    let x1 = (a[0].max(b[0]).max(cc[0]).ceil().max(0.0) as usize).min(n);
    let y0 = a[1].min(b[1]).min(cc[1]).floor().max(0.0) as usize;
    let y1 = (a[1].max(b[1]).max(cc[1]).ceil().max(0.0) as usize).min(n);
    Some((area, [x0, x1, y0, y1]))
}

/// Rasterize one map triangle's nearest depth into `d` (`n × n` texels).
fn raster_triangle(s: &Scene, m: &ShadowMap, tri: [MapCorner; 3], alpha: Option<&AlphaTest>, d: &mut [f32]) {
    let n = m.size as usize;
    let (a, b, cc) = (tri[0].0, tri[1].0, tri[2].0);
    let Some((area, [x0, x1, y0, y1])) = texel_bounds(m, &tri) else { return };
    for y in y0..y1 {
        for x in x0..x1 {
            let (px, py) = (x as f64 + 0.5, y as f64 + 0.5);
            let e0 = ((cc[0] - b[0]) * (py - b[1]) - (cc[1] - b[1]) * (px - b[0])) / area;
            let e1 = ((a[0] - cc[0]) * (py - cc[1]) - (a[1] - cc[1]) * (px - cc[0])) / area;
            let e2 = 1.0 - e0 - e1;
            if e0 < 0.0 || e1 < 0.0 || e2 < 0.0 {
                continue;
            }
            let l = [e0 as f32, e1 as f32, e2 as f32];
            let z = if m.ortho {
                l[0] * tri[0].2 + l[1] * tri[1].2 + l[2] * tri[2].2
            } else {
                let z = 1.0 / (l[0] * tri[0].1 + l[1] * tri[1].1 + l[2] * tri[2].1);
                // 1/z can round to a value whose reciprocal overflows; the depth is then taken in
                // f64 from the corners' depths.
                if z.is_finite() {
                    z
                } else {
                    let iz: f64 = (0..3).map(|k| f64::from(l[k]) / f64::from(tri[k].2)).sum();
                    (1.0 / iz) as f32
                }
            };
            let px = &mut d[y * n + x];
            if z >= *px {
                continue;
            }
            if let Some(t) = alpha {
                let uv = [0, 1].map(|i| caster_uv(&tri, l, z, i, m.ortho));
                if t.discards(sample(t.tex, &s.texels, uv[0], uv[1])[3] * t.factor) {
                    continue;
                }
            }
            *px = z;
        }
    }
}

/// The texels of `m` inside a map triangle's bounding box, as [`raster_triangle`] visits them.
fn triangle_texels(m: &ShadowMap, tri: [MapCorner; 3]) -> u64 {
    let Some((_, [x0, x1, y0, y1])) = texel_bounds(m, &tri) else { return 0 };
    (x1.saturating_sub(x0) as u64).saturating_mul(y1.saturating_sub(y0) as u64)
}

/// The depth tests rendering `maps` takes: one per caster and map, plus each map triangle's
/// bounding-box texels. When the casters times the maps already exceed `limit`, that product.
/// The maps are counted into one total, on several threads with the `parallel` feature, and each
/// stops at the first caster after the total passes `limit`. Returns
/// [`RenderError::BadShadowInputs`] when the total stays within `limit` and a caster's corners in
/// a map are not finite, since the rasterizer would skip that caster.
fn shadow_work(s: &Scene, maps: &[ShadowMap], casters: &[usize], limit: u64) -> Result<u64, RenderError> {
    let pairs = (maps.len() as u64).saturating_mul(casters.len() as u64);
    if pairs > limit {
        return Ok(pairs);
    }
    let finite = |c: &MapCorner| c.0.iter().all(|v| v.is_finite()) && c.1.is_finite() && c.2.is_finite();
    let total = AtomicU64::new(0);
    let not_finite = AtomicBool::new(false);
    for_each_item(maps, |m| {
        for &t in casters {
            if total.load(Ordering::Relaxed) > limit {
                return;
            }
            let ids = [s.indices[3 * t] as usize, s.indices[3 * t + 1] as usize, s.indices[3 * t + 2] as usize];
            // A caster that casts nothing is not projected, as the rasterizer skips it.
            let poly = if casts_nothing(&s.materials[s.vertices[ids[0]].material as usize]) { CasterPoly::new() } else { light_poly(s, m, ids) };
            let mut cost = 1u64;
            if poly.len() >= 3 {
                let scr = map_corners(m, &poly);
                if scr.iter().all(finite) {
                    cost = (1..scr.len() - 1).fold(cost, |acc, k| acc.saturating_add(triangle_texels(m, [scr[0], scr[k], scr[k + 1]])));
                } else {
                    not_finite.store(true, Ordering::Relaxed);
                }
            }
            let mut seen = total.load(Ordering::Relaxed);
            while let Err(now) = total.compare_exchange_weak(seen, seen.saturating_add(cost), Ordering::Relaxed, Ordering::Relaxed) {
                seen = now;
            }
        }
    });
    let total = total.into_inner();
    if total <= limit && not_finite.into_inner() { Err(RenderError::BadShadowInputs) } else { Ok(total) }
}

/// Rasterize the shadow-casting triangles' light-space depth into `m`'s texels.
fn shadow_raster(s: &Scene, m: &ShadowMap, casters: &[usize]) -> Result<Vec<f32>, RenderError> {
    let n = m.size as usize;
    let mut d = filled(n * n, f32::INFINITY)?;
    for &t in casters {
        let ids = [s.indices[3 * t] as usize, s.indices[3 * t + 1] as usize, s.indices[3 * t + 2] as usize];
        let mat = &s.materials[s.vertices[ids[0]].material as usize];
        if casts_nothing(mat) {
            continue;
        }
        let (opacity, cutoff) = caster_alpha(mat);
        let alpha = (mat.alpha_mode != 0 && mat.tex_base >= 0).then(|| AlphaTest {
            tex: &s.textures[mat.tex_base as usize],
            factor: mat.base[3] * opacity,
            cutoff,
        });
        let poly = light_poly(s, m, ids);
        if poly.len() < 3 {
            continue;
        }
        let scr = map_corners(m, &poly);
        for k in 1..scr.len() - 1 {
            raster_triangle(s, m, [scr[0], scr[k], scr[k + 1]], alpha.as_ref(), &mut d);
        }
    }
    Ok(d)
}

/// Long side of a quality shadow map: one map texel per pixel of the `width × height` raster
/// (the supersampled size when the scene is supersampled), to a power of two,
/// clamped to 512..4096. Draft renders keep a flat 512.
pub fn shadow_map_size(width: u32, height: u32, draft: bool) -> u32 {
    if draft {
        return 512;
    }
    let side = width.max(height).clamp(512, 4096);
    let mut size = 512u32;
    while size < side {
        size *= 2;
    }
    size
}

/// Per-face size of a point light's cube map: the full map size, capped at 2048 so one cube fits
/// [`MAX_SHADOW_TEXELS`]. Draft renders take half the map size.
pub fn shadow_point_face_size(size: u32, draft: bool) -> u32 {
    if draft { (size / 2).max(1) } else { size.min(2048) }
}

/// Per-light map sizes (`None` = this light gets no shadow map). `kinds`: 0 = no shadows (costs
/// nothing), 1 = single map (spot or parallel), 2 = cube (a point light, or a spot light wider
/// than one map; budgeted as its six faces).
/// Lights start at their full size; while the scene exceeds [`MAX_SHADOW_TEXELS`] the largest
/// map above its floor (512, or 128 for a cube face) is halved, the earliest light first on
/// ties, and once every light is at its floor the last lights lose their shadows. Deterministic,
/// and O(n log n) in the light count.
pub fn shadow_sizes(width: u32, height: u32, draft: bool, kinds: &[u32]) -> Vec<Option<u32>> {
    let base = shadow_map_size(width, height, draft);
    let faces = |k: u32| -> u64 { if k == 2 { 6 } else { 1 } };
    let floor = |k: u32| -> u32 { if k == 2 { 128 } else { 512 } };
    let mut sizes: Vec<u32> = kinds
        .iter()
        .map(|&k| match k {
            0 => 0,
            2 => shadow_point_face_size(base, draft),
            _ => base,
        })
        .collect();
    let cost = |z: u32, k: u32| -> u64 { (z as u64) * (z as u64) * faces(k) };
    let mut total: u64 = sizes.iter().zip(kinds).map(|(&z, &k)| cost(z, k)).sum();
    // Largest size first, then the lowest index.
    let mut heap: BinaryHeap<(u32, Reverse<usize>)> =
        sizes.iter().zip(kinds).enumerate().filter(|(_, (z, k))| **z > floor(**k)).map(|(i, (&z, _))| (z, Reverse(i))).collect();
    while total > MAX_SHADOW_TEXELS {
        let Some((z, Reverse(i))) = heap.pop() else { break };
        let k = kinds[i];
        total -= cost(z, k) - cost(z / 2, k);
        sizes[i] = z / 2;
        if z / 2 > floor(k) {
            heap.push((z / 2, Reverse(i)));
        }
    }
    for i in (0..sizes.len()).rev() {
        if total <= MAX_SHADOW_TEXELS {
            break;
        }
        total -= cost(sizes[i], kinds[i]);
        sizes[i] = 0;
    }
    sizes.into_iter().map(|z| (z > 0).then_some(z)).collect()
}

/// The environment light's shadow map: one orthographic map of `size` texels over the sphere at
/// `centre` with `radius`, seen from the environment's dominant direction `e.sun` (the light
/// travels from the bright region into the scene). Its penumbra is at least eight output
/// pixels, or four map texels where those are coarser, plus half the light's shadow diffusion
/// up to 24 pixels.
///
/// Returns [`RenderError::BadShadowInputs`] for a zero size, a radius that is not positive and
/// finite, a center that is not finite, a direction whose length differs from 1 by more than
/// 0.001, a shadow diffusion that is negative or not finite, or inputs whose map would hold a
/// value that is not finite in `f32`.
pub fn try_env_shadow_map(e: EnvInfo, centre: [f64; 3], radius: f64, size: u32) -> Result<ShadowMap, RenderError> {
    let ok = size > 0
        && radius.is_finite()
        && radius > 0.0
        && centre.iter().all(|v| v.is_finite())
        && unit(&e.sun)
        && e.shadow_diffusion.is_finite()
        && e.shadow_diffusion >= 0.0;
    if !ok {
        return Err(RenderError::BadShadowInputs);
    }
    let m = env_shadow_map(e, centre, radius, size);
    if !(m.focal.is_finite() && m.focal > 0.0 && m.radius.is_finite() && m.view.iter().flatten().all(|v| v.is_finite())) {
        return Err(RenderError::BadShadowInputs);
    }
    Ok(m)
}

fn env_shadow_map(e: EnvInfo, centre: [f64; 3], radius: f64, size: u32) -> ShadowMap {
    let texel = 2.0 * radius / size as f64;
    let soft = (4.0 * texel).max(8.0) + (e.shadow_diffusion as f64 * 0.5).min(24.0);
    let sun = e.sun.map(|v| v as f64);
    let dir = [-sun[0], -sun[1], -sun[2]];
    let eye = [centre[0] + sun[0] * radius * 2.0, centre[1] + sun[1] * radius * 2.0, centre[2] + sun[2] * radius * 2.0];
    let hint = if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
    let focal = (size as f64 * 0.5 / radius) as f32;
    ShadowMap { view: basis(eye, dir, hint), focal, ortho: true, size, offset: 0, bias: 1.5, radius: soft as f32 }
}

/// The triangles whose material casts shadows.
fn caster_list(s: &Scene) -> Result<Vec<usize>, RenderError> {
    let mut out = vec![];
    for t in 0..s.indices.len() / 3 {
        if s.materials.get(s.vertices[s.indices[3 * t] as usize].material as usize).is_some_and(|m| m.casts_shadows) {
            out.try_reserve(1).map_err(|_| RenderError::OutOfMemory)?;
            out.push(t);
        }
    }
    Ok(out)
}

/// The bounding sphere of the scene's vertices: (center, radius + 1).
fn bounding_sphere(s: &Scene) -> ([f64; 3], f64) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for v in &s.vertices {
        for k in 0..3 {
            lo[k] = lo[k].min(v.pos[k] as f64);
            hi[k] = hi[k].max(v.pos[k] as f64);
        }
    }
    let centre = [0, 1, 2].map(|k| (lo[k] + hi[k]) * 0.5);
    let radius = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt() * 0.5 + 1.0;
    (centre, radius)
}

/// The widest half angle, in radians, that one spot light map covers (about 83°).
const MAX_SPOT_HALF_ANGLE: f64 = 1.45;

/// The half angle a spot light's map covers: the outer cone's half angle plus 0.05 radians.
fn spot_half_angle(l: &Light) -> f64 {
    l.cos_outer.clamp(-1.0, 1.0).acos() as f64 + 0.05
}

/// Whether light `l` has a cube of six maps instead of one map: a point light, or a spot light
/// whose cone with its margin is wider than one map covers.
pub(crate) fn cube_maps(l: &Light) -> bool {
    l.kind == 2 || (l.kind == 1 && spot_half_angle(l) > MAX_SPOT_HALF_ANGLE)
}

/// The maps of light `li` at `size`, appended to `maps`.
fn light_maps(s: &Scene, li: usize, size: u32, sphere: ([f64; 3], f64), maps: &mut Vec<ShadowMap>) {
    let (centre, radius) = sphere;
    let l = s.lights[li];
    // Penumbra width in output pixels; the sampling bias is independent of it.
    let soft = 1.0 + (l.shadow_diffusion * 0.5).min(24.0);
    let pos = l.pos.map(|v| v as f64);
    let dir = l.dir.map(|v| v as f64);
    let hint = if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [0.0, 0.0, 1.0] };
    match l.kind {
        0 => {
            let eye = [centre[0] - dir[0] * radius * 2.0, centre[1] - dir[1] * radius * 2.0, centre[2] - dir[2] * radius * 2.0];
            let focal = (size as f64 * 0.5 / radius) as f32;
            maps.push(ShadowMap { view: basis(eye, dir, hint), focal, ortho: true, size, offset: 0, bias: 1.5, radius: soft });
        }
        1 if !cube_maps(&l) => {
            let half = spot_half_angle(&l);
            let focal = (size as f64 * 0.5 / half.tan()) as f32;
            maps.push(ShadowMap { view: basis(pos, dir, hint), focal, ortho: false, size, offset: 0, bias: 1.5, radius: soft });
        }
        _ => {
            let fs = size;
            for (f, d) in [
                ([1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                ([-1.0, 0.0, 0.0], [0.0, 1.0, 0.0]),
                ([0.0, 1.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, -1.0, 0.0], [0.0, 0.0, 1.0]),
                ([0.0, 0.0, 1.0], [0.0, 1.0, 0.0]),
                ([0.0, 0.0, -1.0], [0.0, 1.0, 0.0]),
            ] {
                maps.push(ShadowMap { view: basis(pos, f, d), focal: fs as f32 * 0.5, ortho: false, size: fs, offset: 0, bias: 1.5, radius: soft });
            }
        }
    }
}

/// Render the shadow maps of every shadow-casting light (lights whose `shadow` is 0 or more),
/// and of the environment light when it casts shadows and has a dominant direction
/// (`env.shadow` 0 or more, `env.sun != [0; 3]`): one orthographic map from that direction over
/// the scene's bounding sphere, after the lights' maps. Map sizes come from [`shadow_sizes`] at
/// the scene's raster size. Each light's `shadow` becomes the index of its first map, or −1 when
/// it gets none; with no shadow-casting triangle, every mark is cleared.
///
/// [`Scene::shadows`] and [`Scene::shadow_texels`] are replaced, and the indices a call writes
/// request the same maps again, so a second call gives the same maps and marks. The scene is
/// checked first, as for [`crate::try_render`], and the depth tests the maps take (one per
/// caster and map, plus each caster's bounding box in each map) are limited by
/// [`Limits::max_work`]. On an error the scene is left as it was.
pub fn try_shadow_maps(s: &mut Scene, draft: bool, limits: &Limits) -> Result<(), RenderError> {
    validate(s, limits)?;
    let marks = (s.lights.clone(), s.env);
    let result = shadow_maps_checked(s, draft, limits);
    if result.is_err() {
        (s.lights, s.env) = marks;
    }
    result
}

fn shadow_maps_checked(s: &mut Scene, draft: bool, limits: &Limits) -> Result<(), RenderError> {
    let casters = caster_list(s)?;
    // The environment light casts a shadow when marked and its image has a dominant direction;
    // with nothing to cast one, its mark is cleared either way.
    let env_shadow = !casters.is_empty() && s.env.is_some_and(|e| e.shadow >= 0 && e.sun != [0.0; 3]);
    if !env_shadow && let Some(e) = &mut s.env {
        e.shadow = -1;
    }
    if casters.is_empty() {
        for l in &mut s.lights {
            l.shadow = -1;
        }
        s.shadows.clear();
        s.shadow_texels.clear();
        return Ok(());
    }
    let sphere = bounding_sphere(s);
    let kinds: Vec<u32> = s
        .lights
        .iter()
        .map(|l| {
            if l.shadow < 0 {
                0
            } else if cube_maps(l) {
                2
            } else {
                1
            }
        })
        // The environment light's map is a single orthographic one.
        .chain(std::iter::once(if env_shadow { 1 } else { 0 }))
        .collect();
    let sizes = shadow_sizes(s.width, s.height, draft, &kinds);
    let mut maps: Vec<ShadowMap> = vec![];
    for li in 0..s.lights.len() {
        let Some(size) = sizes.get(li).copied().flatten() else {
            s.lights[li].shadow = -1;
            continue;
        };
        if s.lights[li].shadow < 0 {
            s.lights[li].shadow = -1;
            continue;
        }
        let first = maps.len() as i32;
        light_maps(s, li, size, sphere, &mut maps);
        s.lights[li].shadow = first;
    }
    if env_shadow && let Some(e) = s.env {
        match sizes.get(s.lights.len()).copied().flatten() {
            Some(size) => {
                let first = maps.len() as i32;
                maps.push(env_shadow_map(e, sphere.0, sphere.1, size));
                if let Some(e) = &mut s.env {
                    e.shadow = first;
                }
            }
            // The texel budget dropped it.
            None => {
                if let Some(e) = &mut s.env {
                    e.shadow = -1;
                }
            }
        }
    }
    // Finite vertices far from the origin can still give a view that does not fit f32.
    let usable = |m: &ShadowMap| m.focal.is_finite() && m.focal > 0.0 && m.radius.is_finite() && m.view.iter().flatten().all(|v| v.is_finite());
    if !maps.iter().all(usable) {
        return Err(RenderError::BadShadowInputs);
    }
    let needed = shadow_work(s, &maps, &casters, limits.max_work)?;
    if needed > limits.max_work {
        return Err(RenderError::TooMuchWork { needed, limit: limits.max_work });
    }
    let texels = map_collect(&maps, |m| shadow_raster(s, m, &casters))?;
    let total = texels.iter().map(|t| t.as_ref().map_or(0, Vec::len)).fold(0, usize::saturating_add);
    let mut all = try_with_capacity(total)?;
    for (m, t) in maps.iter_mut().zip(texels) {
        let t = t?;
        m.offset = u32::try_from(all.len()).map_err(|_| RenderError::TooManyShadowTexels)?;
        all.extend(t);
    }
    s.shadows = maps;
    s.shadow_texels = all;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{NEAR, light_poly, raster_triangle};
    use crate::{Scene, ShadowMap, Vertex};

    #[test]
    fn a_caster_at_the_largest_finite_depth_keeps_a_finite_depth() {
        let view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        let map = ShadowMap { view, focal: 1.0, ortho: false, size: 4, offset: 0, bias: 0.0, radius: 0.0 };
        let z = f32::MAX;
        let corner = |x: f64, y: f64| ([x, y], 1.0 / z, z, [0.0, 0.0]);
        let mut d = vec![f32::INFINITY; 16];
        raster_triangle(&Scene::default(), &map, [corner(0.0, 0.0), corner(4.0, 0.0), corner(0.0, 4.0)], None, &mut d);
        assert_eq!(d[0], z);
    }

    #[test]
    fn near_plane_clipping_preserves_finite_opposite_uvs() {
        let scene = Scene {
            vertices: vec![
                Vertex { pos: [0.0, 0.0, 0.0], uv: [3e38, 0.0], ..Vertex::default() },
                Vertex { pos: [0.0, 1.0, 2.0 * NEAR], uv: [-3e38, 0.0], ..Vertex::default() },
                Vertex { pos: [1.0, 0.0, 2.0 * NEAR], uv: [0.0, 0.0], ..Vertex::default() },
            ],
            ..Scene::default()
        };
        let view = [[1.0, 0.0, 0.0, 0.0], [0.0, 1.0, 0.0, 0.0], [0.0, 0.0, 1.0, 0.0]];
        let map = ShadowMap { view, focal: 1.0, ortho: false, size: 8, offset: 0, bias: 0.0, radius: 0.0 };
        let polygon = light_poly(&scene, &map, [0, 1, 2]);
        let crossings: Vec<_> = polygon.iter().filter(|corner| corner.0[2] == NEAR).collect();
        assert_eq!(crossings.len(), 2);
        assert!(crossings.iter().all(|corner| corner.1.iter().all(|value| value.is_finite())));
        assert!(crossings.iter().any(|corner| corner.1[0] == 0.0), "the midpoint of opposite finite texture coordinates is zero");
    }
}
