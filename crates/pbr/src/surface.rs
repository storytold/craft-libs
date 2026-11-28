//! A surface material: the BRDF core with the surface state around it, and the report and errors
//! of the conversions into it.

use crate::{Material, PbrError};

/// A material as a renderer needs it: the metallic-roughness [`Material`] that
/// [`Material::try_direct`] shades, and the state a renderer applies around it. Alpha, emission
/// and sidedness are applied by the renderer, not by the BRDF. The clearcoat and sheen layers,
/// the textures and [`Surface::other_inputs`] are kept but not shaded by
/// [`Material::try_direct`]. When [`Surface::unlit`] is set, the surface shows its base color
/// without lighting, and [`Material::try_direct`] does not give its appearance.
///
/// Made by `try_from_mesh` (the `mesh` feature) and `try_from_materialx` (the `materialx`
/// feature), which check every value.
/// `#[non_exhaustive]`: read and change the fields, but get a value from a conversion.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Surface {
    /// The source material's name.
    pub name: String,
    /// The BRDF core. Where a texture is bound to one of its values, the core holds the factor the
    /// texture's samples are multiplied by.
    pub material: Material,
    /// Coverage from 0 (transparent) to 1 (opaque).
    pub alpha: f32,
    /// How [`Surface::alpha`] is used.
    pub alpha_mode: AlphaMode,
    /// Linear RGB emission, each channel at least 0.
    pub emissive: [f32; 3],
    /// Both faces are drawn and lit.
    pub double_sided: bool,
    /// The surface is shown as its base color and alpha, without lighting.
    pub unlit: bool,
    /// A clear layer over the base, when its weight is above 0 or a texture sets it.
    pub clearcoat: Option<Clearcoat>,
    /// A sheen layer, when its color is not black or a texture sets it.
    pub sheen: Option<Sheen>,
    /// Textures bound to the material's values, not loaded or sampled.
    pub textures: Vec<TextureBinding>,
    /// Source inputs with no field here, by their source name, as authored. Color values are
    /// linear.
    pub other_inputs: Vec<(String, Vec<f32>)>,
}

/// How a surface's alpha is used.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub enum AlphaMode {
    /// Alpha is ignored and the surface is opaque.
    #[default]
    Opaque,
    /// The surface is drawn where alpha is at least the cutoff, which is at least 0.
    Mask(f32),
    /// The surface is blended by alpha.
    Blend,
}

/// A clear layer over the base: weight and perceptual roughness, each from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clearcoat {
    pub weight: f32,
    pub roughness: f32,
}

/// A sheen layer: linear RGB color and perceptual roughness, each from 0 to 1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Sheen {
    pub color: [f32; 3],
    pub roughness: f32,
}

/// The value a texture sets, and which of its channels it reads.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureInput {
    /// Base color in RGB; from glTF, alpha in A.
    BaseColor,
    /// glTF's packed texture: roughness in G and metallic in B.
    MetallicRoughness,
    /// Metallic in the first channel.
    Metallic,
    /// Roughness in the first channel.
    Roughness,
    /// A tangent-space normal map; [`TextureBinding::scale`] scales its X and Y.
    Normal,
    /// Ambient occlusion in R; [`TextureBinding::scale`] is its strength.
    Occlusion,
    /// Emission color in RGB.
    Emissive,
    /// Opacity: alpha in the first channel.
    Opacity,
    /// A colored opacity in RGB, such as Standard Surface's `opacity`; the alpha is its Rec.709
    /// luminance.
    OpacityRgb,
    /// Clearcoat weight in R.
    Clearcoat,
    /// glTF's clearcoat roughness texture: roughness in G.
    ClearcoatRoughness,
    /// Clearcoat roughness in the first channel.
    ClearcoatRoughnessFirst,
    /// Sheen color in RGB.
    SheenColor,
    /// glTF's sheen roughness texture: roughness in A.
    SheenRoughness,
    /// Sheen roughness in the first channel.
    SheenRoughnessFirst,
    /// Displacement height in the first channel.
    Displacement,
    /// A source input with no slot above, by its source name.
    Other(String),
}

/// Where a texture's image is.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureSource {
    /// An index into the source scene's images (`craft_mesh::Scene::images`).
    Image(usize),
    /// A file path, as the source document wrote it with any file prefix applied.
    File(String),
}

/// How a texture's samples are encoded.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum TextureEncoding {
    /// Linear values, or data that is not a color.
    Linear,
    /// sRGB-encoded color.
    Srgb,
    /// A color space by the name the source gives it, which the caller converts.
    Named(String),
}

/// How a texture continues outside 0..1.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum Wrap {
    #[default]
    Repeat,
    Clamp,
    Mirror,
}

/// A UV transform: scale, then rotate by `rotation` radians, then add `offset`, in the order and
/// with the signs of glTF's `KHR_texture_transform`. Every value is finite.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct UvTransform {
    pub offset: [f32; 2],
    pub rotation: f32,
    pub scale: [f32; 2],
}

/// A texture bound to a value of a [`Surface`].
#[derive(Clone, Debug, PartialEq)]
pub struct TextureBinding {
    pub input: TextureInput,
    pub source: TextureSource,
    pub encoding: TextureEncoding,
    /// The UV set the texture reads.
    pub uv_set: u32,
    /// How the texture continues outside 0..1 along u and v.
    pub wrap: [Wrap; 2],
    pub transform: Option<UvTransform>,
    /// A finite scale on the samples: the normal scale of a normal map, the strength of an
    /// occlusion map, and 1 otherwise.
    pub scale: f32,
}

/// A converted surface and what the conversion did not carry over exactly.
#[derive(Clone, Debug, PartialEq)]
pub struct Conversion {
    pub surface: Surface,
    pub report: ConversionReport,
}

/// What a conversion kept without shading, approximated or dropped, in the order it found them.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ConversionReport {
    pub items: Vec<ReportItem>,
}

impl ConversionReport {
    /// Whether every source value went into a field that [`Material::try_direct`] shades or that
    /// a renderer applies (alpha, emission, sidedness), with nothing approximated or dropped.
    pub fn is_empty(&self) -> bool {
        self.items.is_empty()
    }
}

/// One entry of a [`ConversionReport`]. `input` is the source's name for the value.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ReportItem {
    /// Kept on the [`Surface`] (a layer, a texture, an unlit flag or an other input), and not
    /// shaded by [`Material::try_direct`].
    Unshaded { input: String },
    /// Mapped with a change of meaning; `how` says what was done.
    Approximated { input: String, how: &'static str },
    /// Not kept; `why` says why.
    Dropped { input: String, why: &'static str },
    /// A warning the source reader gave.
    Warning(String),
}

/// Why a material could not be converted. Nothing is clamped or replaced: a value out of its
/// range is an error.
#[derive(Clone, Debug, PartialEq, thiserror::Error)]
#[non_exhaustive]
pub enum ConvertError {
    /// A value is not a finite number, or is out of its range; `reason` says which.
    #[error("{input} {reason}")]
    BadValue { input: String, reason: &'static str },
    /// A value has the wrong number of components.
    #[error("{input} has {found} values where {expected} are needed")]
    Components { input: String, expected: usize, found: usize },
    /// A color value is in a color space the conversion does not decode.
    #[error("{input} is in the color space {colorspace}, which is not converted")]
    ColorSpace { input: String, colorspace: String },
    /// The source names an input more than once.
    #[error("the material has more than one {0} input")]
    Duplicate(String),
    /// The BRDF core rejected the converted values.
    #[error(transparent)]
    Material(#[from] PbrError),
}

/// `v` when it is finite and in `lo..=hi`, else [`ConvertError::BadValue`] naming `input`.
#[cfg(feature = "mesh")]
pub(crate) fn checked(v: f32, lo: f32, hi: f32, input: &str, reason: &'static str) -> Result<f32, ConvertError> {
    if v.is_finite() && (lo..=hi).contains(&v) { Ok(v) } else { Err(ConvertError::BadValue { input: input.to_owned(), reason }) }
}

/// The reason text for a value that must be from 0 to 1.
#[cfg(any(feature = "mesh", feature = "materialx"))]
pub(crate) const UNIT: &str = "is not a number from 0 to 1";
/// The reason text for a value that must be finite and at least 0.
#[cfg(any(feature = "mesh", feature = "materialx"))]
pub(crate) const NON_NEGATIVE: &str = "is not a finite number of at least 0";
/// The reason text for a value that must be finite.
#[cfg(any(feature = "mesh", feature = "materialx"))]
pub(crate) const FINITE: &str = "is not a finite number";
