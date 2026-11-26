//! The scene: everything the rasterizer needs, flattened into world-space triangles and plain
//! arrays, and the checks a scene passes before it is rendered.

use crate::shadow::cube_maps;
use crate::{Limits, RenderError};

/// One vertex in world space.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[repr(C)]
pub struct Vertex {
    pub pos: [f32; 3],
    pub normal: [f32; 3],
    pub uv: [f32; 2],
    /// Tangent and bitangent sign (`w`).
    pub tangent: [f32; 4],
    /// Index into [`Scene::materials`]. A triangle takes the material of its first vertex.
    pub material: u32,
}

/// A material, resolved for shading.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    /// Linear RGBA factor.
    pub base: [f32; 4],
    /// 0 to 1.
    pub metallic: f32,
    /// 0 to 1.
    pub roughness: f32,
    pub emissive: [f32; 3],
    pub normal_scale: f32,
    /// 0 to 1.
    pub occlusion_strength: f32,
    /// 0 opaque, 1 mask, 2 blend.
    pub alpha_mode: u32,
    pub alpha_cutoff: f32,
    /// Opacity of the whole surface (multiplies alpha).
    pub opacity: f32,
    /// Texture indices (negative = none).
    pub tex_base: i32,
    pub tex_mr: i32,
    pub tex_normal: i32,
    pub tex_occlusion: i32,
    pub tex_emissive: i32,
    pub double_sided: bool,
    pub unlit: bool,
    pub accepts_lights: bool,
    pub receives_shadows: bool,
    /// Draws only the shadows it receives, as black times their amount.
    pub shadows_only: bool,
    pub casts_shadows: bool,
    /// Drawn (false for a surface that only casts shadows).
    pub visible: bool,
    /// Weights of the diffuse lobe, the dielectric specular reflectance and the ambient light.
    pub diffuse_k: f32,
    pub specular_k: f32,
    pub ambient_k: f32,
}

impl Default for Material {
    fn default() -> Self {
        Material {
            base: [1.0; 4],
            metallic: 0.0,
            roughness: 0.5,
            emissive: [0.0; 3],
            normal_scale: 1.0,
            occlusion_strength: 1.0,
            alpha_mode: 0,
            alpha_cutoff: 0.5,
            opacity: 1.0,
            tex_base: -1,
            tex_mr: -1,
            tex_normal: -1,
            tex_occlusion: -1,
            tex_emissive: -1,
            double_sided: false,
            unlit: false,
            accepts_lights: true,
            receives_shadows: true,
            shadows_only: false,
            casts_shadows: false,
            visible: true,
            diffuse_k: 1.0,
            specular_k: 1.0,
            ambient_k: 1.0,
        }
    }
}

impl Material {
    /// Drawn in the transparent (sorted, blended) pass: a blended material, a shadows-only one, or
    /// one whose opacity is below 1.
    pub fn transparent(&self) -> bool {
        self.alpha_mode == 2 || self.opacity < 1.0 || self.shadows_only
    }
}

/// A texture in [`Scene::texels`]: linear straight RGBA.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TexInfo {
    pub offset: u32,
    pub width: u32,
    pub height: u32,
    /// 0 repeat, 1 clamp, 2 mirror (other values repeat).
    pub wrap_u: u32,
    pub wrap_v: u32,
}

/// A light (ambient light is summed into [`Scene::ambient`]).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Light {
    /// 0 parallel, 1 spot, 2 point.
    pub kind: u32,
    pub pos: [f32; 3],
    /// Unit direction the light travels, for a parallel or spot light; a length that differs from
    /// 1 by more than 0.001 is an error. A point light does not read it.
    pub dir: [f32; 3],
    /// Linear color times intensity.
    pub color: [f32; 3],
    /// Cosine of a spot light's full-intensity half angle, −1 to 1 and at least
    /// [`Light::cos_outer`].
    pub cos_inner: f32,
    /// Cosine of a spot light's cone half angle, −1 to 1.
    pub cos_outer: f32,
    /// 0 none, 1 smooth, 2 inverse square clamped.
    pub falloff: u32,
    pub radius: f32,
    pub falloff_distance: f32,
    /// First shadow map (point lights, and spot lights whose outer cone's half angle is over 1.40
    /// radians, use six), −1 = none. [`crate::try_shadow_maps`] reads any
    /// value of 0 or more as a request for maps.
    pub shadow: i32,
    /// Fraction of the light a full shadow removes, 0 to 1.
    pub shadow_darkness: f32,
    /// Penumbra widening in output pixels, 0 or more.
    pub shadow_diffusion: f32,
}

/// A depth map seen from a light: `view` maps world → light space (+z forward); pixels
/// `u = focal·x/z + size/2` (perspective) or `focal·x + size/2` (orthographic); texels store
/// the light-space depth of the nearest caster. A perspective map holds the parts of casters at a
/// light-space depth of at least 0.001, and a receiver at that depth or nearer is not shadowed by
/// it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ShadowMap {
    pub view: [[f32; 4]; 3],
    pub focal: f32,
    pub ortho: bool,
    pub size: u32,
    /// First texel in [`Scene::shadow_texels`].
    pub offset: u32,
    /// Depth bias in texels (scaled by the texel footprint at the receiver).
    pub bias: f32,
    /// Penumbra width in output pixels; the shader converts it to a tap reach through the
    /// receiver's texel footprint.
    pub radius: f32,
}

/// The environment image: radiance levels `radiance..radiance + mips` and an irradiance map,
/// in [`Scene::textures`], equirect with the vertical axis along −y.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EnvInfo {
    /// The texture index of the sharpest radiance level; levels `radiance..radiance + mips` go
    /// from smooth to rough.
    pub radiance: u32,
    /// The radiance level count, at least 1.
    pub mips: u32,
    /// The texture index of the irradiance map.
    pub irradiance: u32,
    pub intensity: f32,
    /// Radians about the vertical axis.
    pub rotation: f32,
    /// Unit direction from the scene toward the environment's dominant bright region, the
    /// direction its shadow is cast from. `[0; 3]` when the image has no dominant direction: no
    /// shadow. Any other length that differs from 1 by more than 0.001 is an error.
    pub sun: [f32; 3],
    /// The image's directional fraction, 0 to 1: 0 for a uniform image and near 1 for a single
    /// bright spot. The shadow occludes only this fraction of the environment light.
    pub key: f32,
    /// Shadow map index into [`Scene::shadows`]: [`crate::try_shadow_maps`] reads any value of 0
    /// or more as a request for a map; −1 casts no shadow.
    pub shadow: i32,
    /// Fraction of the occluded light a full shadow removes, 0 to 1.
    pub shadow_darkness: f32,
    /// Penumbra widening in output pixels, 0 or more.
    pub shadow_diffusion: f32,
}

/// A prepared scene.
#[derive(Clone, Debug, Default)]
pub struct Scene {
    /// Raster size (output × `ssaa`).
    pub width: u32,
    pub height: u32,
    /// Supersampling factor per side; 0 is read as 1.
    pub ssaa: u32,
    /// World → clip (row-major; reversed depth: ndc z = near / camera z).
    pub clip: [[f32; 4]; 4],
    /// World → camera (row-major; camera z is the depth written to [`crate::Target::depth`]).
    pub view: [[f32; 4]; 4],
    pub eye: [f32; 3],
    pub cam_fwd: [f32; 3],
    pub ortho: bool,
    pub vertices: Vec<Vertex>,
    /// Triangles: the first `opaque_count` indices are opaque, the rest transparent, sorted
    /// back to front.
    pub indices: Vec<u32>,
    pub opaque_count: u32,
    pub materials: Vec<Material>,
    pub textures: Vec<TexInfo>,
    pub texels: Vec<[f32; 4]>,
    pub lights: Vec<Light>,
    pub shadows: Vec<ShadowMap>,
    pub shadow_texels: Vec<f32>,
    pub env: Option<EnvInfo>,
    pub ambient: [f32; 3],
    /// Shading inputs and outputs are linear (no sRGB conversion needed).
    pub linear_io: bool,
}

fn finite<const N: usize>(v: &[f32; N]) -> bool {
    v.iter().all(|x| x.is_finite())
}

/// How far a direction's length may differ from 1.
const UNIT_TOLERANCE: f64 = 1e-3;

/// `v` is finite and its length is within [`UNIT_TOLERANCE`] of 1.
pub(crate) fn unit(v: &[f32; 3]) -> bool {
    let len = v.iter().map(|&x| f64::from(x) * f64::from(x)).sum::<f64>().sqrt();
    (len - 1.0).abs() <= UNIT_TOLERANCE
}

/// Checks a scene against `limits`: the raster size, the index list, every reference between the
/// arrays, texture and shadow spans, finite values and the triangle and light counts. Returns the
/// raster's pixel count.
pub(crate) fn validate(s: &Scene, limits: &Limits) -> Result<usize, RenderError> {
    let pixels = limits.check(s.width, s.height)?;
    if !s.clip.iter().all(finite) || !s.view.iter().all(finite) || !finite(&s.eye) || !finite(&s.cam_fwd) || !finite(&s.ambient) {
        return Err(RenderError::BadTransform);
    }
    validate_indices(s, limits)?;
    validate_materials(s)?;
    validate_textures(s)?;
    validate_lights(s, limits)?;
    validate_shadows(s)?;
    Ok(pixels)
}

fn validate_indices(s: &Scene, limits: &Limits) -> Result<(), RenderError> {
    let len = s.indices.len();
    if !len.is_multiple_of(3) {
        return Err(RenderError::BadIndexCount(len));
    }
    let opaque = s.opaque_count as usize;
    if opaque > len || !opaque.is_multiple_of(3) {
        return Err(RenderError::BadOpaqueCount { opaque: s.opaque_count, len });
    }
    let count = (len / 3) as u64;
    if count > limits.max_triangles {
        return Err(RenderError::TooManyTriangles { count, limit: limits.max_triangles });
    }
    let vertices = s.vertices.len() as u64;
    if vertices > limits.max_vertices {
        return Err(RenderError::TooManyVertices { count: vertices, limit: limits.max_vertices });
    }
    if let Some((position, &index)) = s.indices.iter().enumerate().find(|(_, i)| **i as usize >= s.vertices.len()) {
        return Err(RenderError::BadIndex { position, index });
    }
    let bad = |v: &Vertex| !finite(&v.pos) || !finite(&v.normal) || !finite(&v.uv) || !finite(&v.tangent) || v.material as usize >= s.materials.len();
    if let Some(i) = s.vertices.iter().position(bad) {
        return Err(RenderError::BadVertex(i));
    }
    Ok(())
}

fn validate_materials(s: &Scene) -> Result<(), RenderError> {
    let tex_ok = |i: i32| i < 0 || (i as usize) < s.textures.len();
    let ok = |m: &Material| {
        finite(&m.base)
            && finite(&m.emissive)
            && finite(&[
                m.metallic,
                m.roughness,
                m.normal_scale,
                m.occlusion_strength,
                m.alpha_cutoff,
                m.opacity,
                m.diffuse_k,
                m.specular_k,
                m.ambient_k,
            ])
            && (0.0..=1.0).contains(&m.base[3])
            && (0.0..=1.0).contains(&m.opacity)
            && (0.0..=1.0).contains(&m.metallic)
            && (0.0..=1.0).contains(&m.roughness)
            && (0.0..=1.0).contains(&m.occlusion_strength)
            && m.alpha_mode <= 2
            && [m.tex_base, m.tex_mr, m.tex_normal, m.tex_occlusion, m.tex_emissive].into_iter().all(tex_ok)
    };
    match s.materials.iter().position(|m| !ok(m)) {
        Some(i) => Err(RenderError::BadMaterial(i)),
        None => Ok(()),
    }
}

fn validate_textures(s: &Scene) -> Result<(), RenderError> {
    for (i, t) in s.textures.iter().enumerate() {
        let end = u64::from(t.width).checked_mul(u64::from(t.height)).and_then(|n| n.checked_add(u64::from(t.offset)));
        if t.width == 0 || t.height == 0 || end.is_none_or(|e| e > s.texels.len() as u64) {
            return Err(RenderError::BadTexture(i));
        }
    }
    match s.texels.iter().position(|t| !finite(t) || !(0.0..=1.0).contains(&t[3])) {
        Some(i) => Err(RenderError::BadTexel(i)),
        None => Ok(()),
    }
}

fn validate_lights(s: &Scene, limits: &Limits) -> Result<(), RenderError> {
    if s.lights.len() > limits.max_lights {
        return Err(RenderError::TooManyLights { count: s.lights.len(), limit: limits.max_lights });
    }
    let ok = |l: &Light| {
        l.kind <= 2
            && finite(&l.pos)
            && finite(&l.dir)
            && finite(&l.color)
            && finite(&[l.cos_inner, l.cos_outer, l.radius, l.falloff_distance, l.shadow_darkness, l.shadow_diffusion])
            && (-1.0..=1.0).contains(&l.cos_inner)
            && (-1.0..=1.0).contains(&l.cos_outer)
            && (l.kind != 1 || l.cos_inner >= l.cos_outer)
            && (0.0..=1.0).contains(&l.shadow_darkness)
            && l.shadow_diffusion >= 0.0
            && (l.kind == 2 || unit(&l.dir))
    };
    if let Some(i) = s.lights.iter().position(|l| !ok(l)) {
        return Err(RenderError::BadLight(i));
    }
    if let Some(e) = &s.env {
        let ok = finite(&e.sun)
            && (e.sun == [0.0; 3] || unit(&e.sun))
            && finite(&[e.intensity, e.rotation, e.key, e.shadow_darkness, e.shadow_diffusion])
            && (0.0..=1.0).contains(&e.key)
            && (0.0..=1.0).contains(&e.shadow_darkness)
            && e.shadow_diffusion >= 0.0
            && e.mips > 0
            && e.radiance.checked_add(e.mips).is_some_and(|end| end as usize <= s.textures.len())
            && (e.irradiance as usize) < s.textures.len();
        if !ok {
            return Err(RenderError::BadEnvironment);
        }
    }
    Ok(())
}

/// Every light and the environment light either has no shadow (a negative mark) or refers to
/// maps in [`Scene::shadows`]: six from its mark for a point light or a spot light whose cone
/// one map does not cover, one otherwise.
/// Checks that no drawn triangle of the opaque range has a shadows-only material (its first
/// vertex's material, as the raster reads it; a material with `visible` false is not drawn). Run
/// after [`validate`].
pub(crate) fn validate_passes(s: &Scene) -> Result<(), RenderError> {
    let shadows_only =
        |t: &[u32]| s.vertices.get(t[0] as usize).and_then(|v| s.materials.get(v.material as usize)).is_some_and(|m| m.shadows_only && m.visible);
    match s.indices.get(..s.opaque_count as usize).unwrap_or_default().chunks(3).position(shadows_only) {
        Some(t) => Err(RenderError::ShadowsOnlyOpaque(t)),
        None => Ok(()),
    }
}

pub(crate) fn validate_shadow_refs(s: &Scene) -> Result<(), RenderError> {
    let exists = |first: i32, count: usize| first < 0 || (first as usize).checked_add(count).is_some_and(|end| end <= s.shadows.len());
    if let Some(i) = s.lights.iter().position(|l| !exists(l.shadow, if cube_maps(l) { 6 } else { 1 })) {
        return Err(RenderError::MissingShadowMap(Some(i)));
    }
    if s.env.is_some_and(|e| !exists(e.shadow, 1)) {
        return Err(RenderError::MissingShadowMap(None));
    }
    Ok(())
}

fn validate_shadows(s: &Scene) -> Result<(), RenderError> {
    for (i, m) in s.shadows.iter().enumerate() {
        let end = u64::from(m.size).checked_mul(u64::from(m.size)).and_then(|n| n.checked_add(u64::from(m.offset)));
        let ok = m.size > 0
            && m.view.iter().all(finite)
            && m.focal.is_finite()
            && m.focal > 0.0
            && finite(&[m.bias, m.radius])
            && m.radius >= 0.0
            && end.is_some_and(|e| e <= s.shadow_texels.len() as u64);
        if !ok {
            return Err(RenderError::BadShadowMap(i));
        }
    }
    match s.shadow_texels.iter().position(|t| t.is_nan()) {
        Some(i) => Err(RenderError::BadShadowTexel(i)),
        None => Ok(()),
    }
}
