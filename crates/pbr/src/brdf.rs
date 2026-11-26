//! The metallic-roughness BRDF for direct light and the environment BRDF term of the split sum.

use std::f32::consts::PI;

use crate::PbrError;
use crate::prefilter::ggx_distribution_between;
use crate::vec3::unit;

/// The smallest roughness direct light is shaded with. Smaller values, 0 included, are shaded as
/// this one, so a point light keeps a highlight of finite height. Its `α = roughness²` of
/// 0.001024 is above the `α` floor of [`ggx_distribution`](crate::ggx_distribution).
pub const MIN_DIRECT_ROUGHNESS: f32 = 0.032;

/// Largest [`Material::specular`] weight: at 25 the dielectric reflectance at normal incidence,
/// `0.04 × specular`, is 1.
pub const MAX_SPECULAR: f32 = 25.0;

/// The smallest `n·v` the direct BRDF divides by. A view at or below the surface is shaded as
/// a view at this cosine.
const MIN_N_DOT_V: f32 = 1e-4;

/// The geometry term of the specular lobe in [`Material::try_direct`]. Each form uses
/// `α = roughness²` with roughness at least [`MIN_DIRECT_ROUGHNESS`]. The environment BRDF of
/// [`try_environment_brdf`] does not depend on it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Visibility {
    /// EffectCraft's term, the default: the Smith-Schlick `G` with `k = (roughness + 1)² / 8`
    /// that Karis gives for analytic lights in *Real Shading in Unreal Engine 4*
    /// (<https://de45xmedrsdbp.cloudfront.net/Resources/files/2013SiggraphPresentationsNotes-26915738.pdf>),
    /// divided by `4 (n·v) (n·l)` taken as at least `1e-4`. It equals
    /// [`SmithSeparable`](Visibility::SmithSeparable) at roughness 1 and is darker at grazing
    /// angles on smoother surfaces.
    #[default]
    SmithSchlick,
    /// The visibility of glTF 2.0 section B.3.2
    /// (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#specular-brdf>), from the
    /// separable form of the Smith masking-shadowing function:
    /// `V = 1 / ((n·l + √(α² + (1 − α²) (n·l)²)) (n·v + √(α² + (1 − α²) (n·v)²)))`.
    SmithSeparable,
    /// The height-correlated Smith masking-shadowing function for GGX of Heitz, *Understanding
    /// the Masking-Shadowing Function in Microfacet-Based BRDFs* (JCGT 2014,
    /// <https://jcgt.org/published/0003/02/03/>), which section B.3.2 cites for the derivation:
    /// `V = 0.5 / (n·l √((n·v)² (1 − α²) + α²) + n·v √((n·l)² (1 − α²) + α²))`. TextureCraft's
    /// mesh shader uses this form.
    SmithHeightCorrelated,
}

/// A metallic-roughness material: linear base color, metallic and perceptual roughness, each in
/// `0..=1` as glTF 2.0 defines them, and two weights for direct light.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Material {
    base_color: [f32; 3],
    metallic: f32,
    roughness: f32,
    specular: f32,
    diffuse: f32,
    visibility: Visibility,
}

/// Checks that `v` is finite and in `lo..=hi`.
fn in_range(v: f32, lo: f32, hi: f32, what: &'static str) -> Result<f32, PbrError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(v) } else { Err(PbrError::BadMaterial(what)) }
}

impl Material {
    /// A material with specular and diffuse weights of 1 and the default [`Visibility`],
    /// [`Visibility::SmithSchlick`]. Returns [`PbrError::BadMaterial`] when a base color channel,
    /// `metallic` or `roughness` is not a number in `0..=1`.
    pub fn try_new(base_color: [f32; 3], metallic: f32, roughness: f32) -> Result<Material, PbrError> {
        for c in base_color {
            in_range(c, 0.0, 1.0, "a base color channel is not a number from 0 to 1")?;
        }
        Ok(Material {
            base_color,
            metallic: in_range(metallic, 0.0, 1.0, "the metallic value is not a number from 0 to 1")?,
            roughness: in_range(roughness, 0.0, 1.0, "the roughness is not a number from 0 to 1")?,
            specular: 1.0,
            diffuse: 1.0,
            visibility: Visibility::default(),
        })
    }

    /// The material with this geometry term for direct light.
    pub fn with_visibility(self, visibility: Visibility) -> Material {
        Material { visibility, ..self }
    }

    /// The material with these weights. `specular` scales the dielectric reflectance at normal
    /// incidence, 0.04 at 1, and is a number from 0 to [`MAX_SPECULAR`]. `diffuse` scales the
    /// diffuse lobe and is a number from 0 to 1. Returns [`PbrError::BadMaterial`] otherwise.
    /// glTF 2.0 has neither weight; both are 1 for a glTF material.
    pub fn try_with_weights(self, specular: f32, diffuse: f32) -> Result<Material, PbrError> {
        Ok(Material {
            specular: in_range(specular, 0.0, MAX_SPECULAR, "the specular weight is not a number from 0 to 25")?,
            diffuse: in_range(diffuse, 0.0, 1.0, "the diffuse weight is not a number from 0 to 1")?,
            ..self
        })
    }

    /// The linear base color.
    pub fn base_color(&self) -> [f32; 3] {
        self.base_color
    }

    /// The metallic value.
    pub fn metallic(&self) -> f32 {
        self.metallic
    }

    /// The perceptual roughness.
    pub fn roughness(&self) -> f32 {
        self.roughness
    }

    /// The specular weight.
    pub fn specular(&self) -> f32 {
        self.specular
    }

    /// The diffuse weight.
    pub fn diffuse(&self) -> f32 {
        self.diffuse
    }

    /// The geometry term for direct light.
    pub fn visibility(&self) -> Visibility {
        self.visibility
    }

    /// The reflectance at normal incidence: `0.04 × specular` mixed toward the base color by
    /// `metallic`. Each channel is in `0..=1`.
    pub fn f0(&self) -> [f32; 3] {
        let d = 0.04 * self.specular;
        self.base_color.map(|c| d + (c - d) * self.metallic)
    }

    /// The light one light reflects toward the viewer: the BRDF times `n·l` times π, per RGB
    /// channel, for a light whose color is the irradiance on a surface facing it. A white light
    /// on a white rough dielectric lit and seen head-on gives about 1.
    ///
    /// `n`, `v` and `l` are the surface normal, the direction to the viewer and the direction to
    /// the light; they are normalized here. The BRDF is the final form of glTF 2.0 section
    /// B.3.5: a Lambert lobe `(1 − F) (1 − metallic) base / π` plus `F D V`, with Schlick's `F`
    /// (section B.3.4) at `v·h`, the GGX `D` of [`ggx_distribution_between`] from `n` and the
    /// half vector, and the visibility `V = G / (4 (n·v) (n·l))` that [`Material::visibility`]
    /// selects. By default `G` is the Smith-Schlick term with `k = (roughness + 1)² / 8` that
    /// Karis gives for analytic lights, where section B.3.2 writes the separable Smith form
    /// ([`Visibility::SmithSeparable`]). Roughness is shaded as at least
    /// [`MIN_DIRECT_ROUGHNESS`]. A light at or below the surface gives 0, and so does a view
    /// straight opposite the light, which has no half vector. `n·v` is taken as at least `1e-4`,
    /// so a view at or below the surface sees the surface as at a grazing angle. `D` is exact
    /// where `n·h` is close to 1, so the highlight peaks at its head-on value. `D` is 0 where
    /// `n·h` is 0 or less, which happens when the view is at least as far below the surface as
    /// the light is above it. With [`Visibility::SmithSchlick`] the specular denominator
    /// `4 (n·v) (n·l)` is taken as at least `1e-4`, as in EffectCraft, so where it is smaller the
    /// specular lobe is below `F D G / (4 (n·v) (n·l))`. The other two forms divide by no such
    /// product and have no floor.
    ///
    /// Returns [`PbrError::BadDirection`] when a direction is not finite or has zero length.
    pub fn try_direct(&self, n: [f32; 3], v: [f32; 3], l: [f32; 3]) -> Result<[f32; 3], PbrError> {
        let n = unit(n).ok_or(PbrError::BadDirection)?;
        let v = unit(v).ok_or(PbrError::BadDirection)?;
        let l = unit(l).ok_or(PbrError::BadDirection)?;
        let n_l = n.dot(l);
        if n_l <= 0.0 {
            return Ok([0.0; 3]);
        }
        let Some(h) = unit((v + l).to_array()) else { return Ok([0.0; 3]) };
        let n_v = n.dot(v).max(MIN_N_DOT_V);
        let v_h = v.dot(h).max(0.0);
        let r = self.roughness.max(MIN_DIRECT_ROUGHNESS);
        let f = fresnel_schlick(self.f0(), v_h);
        let d = ggx_distribution_between(n.to_array(), h.to_array(), r)?;
        let spec = d * visibility(self.visibility, n_v, n_l, r);
        let mut out = [0.0; 3];
        for c in 0..3 {
            let kd = (1.0 - f[c]) * (1.0 - self.metallic);
            out[c] = (kd * self.base_color[c] / PI * self.diffuse + f[c] * spec) * n_l * PI;
        }
        Ok(out)
    }
}

/// Schlick's Fresnel `f0 + (1 − f0) (1 − v·h)⁵`, with `1 − v·h` kept in `0..=1`.
fn fresnel_schlick(f0: [f32; 3], v_h: f32) -> [f32; 3] {
    let k = (1.0 - v_h).clamp(0.0, 1.0).powi(5);
    f0.map(|c| c + (1.0 - c) * k)
}

/// The Smith-Schlick-GGX geometry term for direct light, `k = (roughness + 1)² / 8`.
fn smith_schlick_ggx(n_v: f32, n_l: f32, roughness: f32) -> f32 {
    let k = (roughness + 1.0) * (roughness + 1.0) / 8.0;
    let g1 = |x: f32| x / (x * (1.0 - k) + k);
    g1(n_v) * g1(n_l)
}

/// The visibility `G / (4 (n·v) (n·l))` of `kind` for `n_v` and `n_l` in `(0, 1]` and perceptual
/// `roughness`. `χ⁺(h·l)` and `χ⁺(h·v)` of section B.3.2 are 1: with `h` the normalized `v + l`,
/// `h·l` and `h·v` are equal and positive whenever `h` exists.
fn visibility(kind: Visibility, n_v: f32, n_l: f32, roughness: f32) -> f32 {
    let a2 = (roughness * roughness) * (roughness * roughness);
    let root = |x: f32| (a2 + (1.0 - a2) * x * x).sqrt();
    match kind {
        Visibility::SmithSchlick => smith_schlick_ggx(n_v, n_l, roughness) / (4.0 * n_v * n_l).max(1e-4),
        Visibility::SmithSeparable => 1.0 / ((n_l + root(n_l)) * (n_v + root(n_v))),
        Visibility::SmithHeightCorrelated => 0.5 / (n_l * root(n_v) + n_v * root(n_l)),
    }
}

/// The environment BRDF term of the split sum for `n_dot_v` and perceptual `roughness`, as
/// `[scale, bias]`: the specular light of a prefiltered environment is its radiance times
/// `f0 × scale + bias`. The values are Karis's analytic fit for mobile
/// (<https://www.unrealengine.com/en-US/blog/physically-based-shading-on-mobile>), kept so that
/// `scale` and `bias` are at least 0 and their sum is at most 1, which keeps `f0 × scale + bias`
/// in `0..=1` for every `f0` in `0..=1`. `n_dot_v` is clamped to `0..=1`.
///
/// Returns [`PbrError::BadDirection`] when `n_dot_v` is not finite, and
/// [`PbrError::BadMaterial`] when `roughness` is not a number in `0..=1`.
pub fn try_environment_brdf(n_dot_v: f32, roughness: f32) -> Result<[f32; 2], PbrError> {
    if !n_dot_v.is_finite() {
        return Err(PbrError::BadDirection);
    }
    let rough = in_range(roughness, 0.0, 1.0, "the roughness is not a number from 0 to 1")?;
    let n_v = n_dot_v.clamp(0.0, 1.0);
    let c0 = [-1.0, -0.0275, -0.572, 0.022];
    let c1 = [1.0, 0.0425, 1.04, -0.04];
    let r = [rough * c0[0] + c1[0], rough * c0[1] + c1[1], rough * c0[2] + c1[2], rough * c0[3] + c1[3]];
    let a004 = (r[0] * r[0]).min((-9.28 * n_v).exp2()) * r[0] + r[1];
    let scale = (-1.04 * a004 + r[2]).clamp(0.0, 1.0);
    let bias = (1.04 * a004 + r[3]).clamp(0.0, 1.0 - scale);
    Ok([scale, bias])
}
