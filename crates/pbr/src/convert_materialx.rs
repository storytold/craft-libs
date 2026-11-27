//! The conversion from a `craft_materialx::StrictMaterial`: an OpenPBR Surface, Standard Surface
//! or glTF PBR shader with its resolved inputs.

use craft_materialx::{Resolved, Shader, StrictMaterial};
use craft_texture::srgb_to_linear;
use std::collections::HashSet;

use crate::surface::{
    AlphaMode, Clearcoat, Conversion, ConversionReport, ConvertError, FINITE, NON_NEGATIVE, ReportItem, Sheen, Surface, TextureBinding,
    TextureEncoding, TextureInput, TextureSource, UNIT, UvTransform, Wrap,
};
use crate::{MAX_SPECULAR, Material};

/// What a shader input holds and the range its values must be in.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Kind {
    /// One number from `lo` to `hi`.
    Scalar(f32, f32),
    /// Three color components from 0 to the bound, in the color space in effect.
    Color(f32),
    /// Three finite numbers, such as a normal.
    Vector,
    /// glTF PBR's alpha mode: 0, 1 or 2.
    Mode,
}

const UNIT_KIND: Kind = Kind::Scalar(0.0, 1.0);
const NON_NEGATIVE_KIND: Kind = Kind::Scalar(0.0, f32::MAX);
const COLOR: Kind = Kind::Color(1.0);

const COAT_OFF: &str = "the coat weight is 0, which turns the layer off";
const SHEEN_OFF: &str = "the sheen color is black or its weight is 0, which turns the layer off";

/// What an input maps to on the [`Surface`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Role {
    BaseColor,
    BaseWeight,
    Metallic,
    Roughness,
    SpecularWeight,
    Ior,
    CoatWeight,
    CoatRoughness,
    SheenWeight,
    SheenColor,
    SheenRoughness,
    EmissionStrength,
    EmissionColor,
    Opacity,
    AlphaMode,
    AlphaCutoff,
    Normal,
    Occlusion,
    /// Kept as an other input or a texture, and not shaded.
    Kept,
    /// Kept like [`Role::Kept`] while the coat is on, and dropped when it is off.
    CoatKept,
}

/// A shader input the conversion reads, with its default from the shader's node definition.
struct Field {
    name: &'static str,
    kind: Kind,
    role: Role,
    default: [f32; 3],
}

const fn field(name: &'static str, kind: Kind, role: Role, default: f32) -> Field {
    Field { name, kind, role, default: [default; 3] }
}

/// OpenPBR Surface inputs and defaults, from its node definition
/// (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/bxdf/open_pbr_surface.mtlx>).
const OPEN_PBR: &[Field] = &[
    field("base_color", COLOR, Role::BaseColor, 0.8),
    field("base_weight", UNIT_KIND, Role::BaseWeight, 1.0),
    field("base_metalness", UNIT_KIND, Role::Metallic, 0.0),
    field("specular_roughness", UNIT_KIND, Role::Roughness, 0.3),
    field("specular_weight", NON_NEGATIVE_KIND, Role::SpecularWeight, 1.0),
    field("specular_ior", NON_NEGATIVE_KIND, Role::Ior, 1.5),
    field("coat_weight", UNIT_KIND, Role::CoatWeight, 0.0),
    field("coat_roughness", UNIT_KIND, Role::CoatRoughness, 0.0),
    field("fuzz_weight", UNIT_KIND, Role::SheenWeight, 0.0),
    field("fuzz_color", COLOR, Role::SheenColor, 1.0),
    field("fuzz_roughness", UNIT_KIND, Role::SheenRoughness, 0.5),
    field("emission_luminance", NON_NEGATIVE_KIND, Role::EmissionStrength, 0.0),
    field("emission_color", COLOR, Role::EmissionColor, 1.0),
    field("geometry_opacity", UNIT_KIND, Role::Opacity, 1.0),
    field("geometry_normal", Kind::Vector, Role::Normal, 0.0),
    field("geometry_coat_normal", Kind::Vector, Role::CoatKept, 0.0),
    field("specular_color", COLOR, Role::Kept, 1.0),
    field("specular_roughness_anisotropy", UNIT_KIND, Role::Kept, 0.0),
    field("transmission_weight", UNIT_KIND, Role::Kept, 0.0),
    field("subsurface_weight", UNIT_KIND, Role::Kept, 0.0),
    field("coat_color", COLOR, Role::CoatKept, 1.0),
    field("coat_ior", NON_NEGATIVE_KIND, Role::CoatKept, 1.6),
];

/// Standard Surface 1.0.1 inputs and defaults, from its node definition
/// (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/bxdf/standard_surface.mtlx>).
const STANDARD_SURFACE: &[Field] = &[
    field("base_color", COLOR, Role::BaseColor, 0.8),
    field("base", UNIT_KIND, Role::BaseWeight, 1.0),
    field("metalness", UNIT_KIND, Role::Metallic, 0.0),
    field("specular_roughness", UNIT_KIND, Role::Roughness, 0.2),
    field("specular", UNIT_KIND, Role::SpecularWeight, 1.0),
    field("specular_IOR", NON_NEGATIVE_KIND, Role::Ior, 1.5),
    field("coat", UNIT_KIND, Role::CoatWeight, 0.0),
    field("coat_roughness", UNIT_KIND, Role::CoatRoughness, 0.1),
    field("sheen", UNIT_KIND, Role::SheenWeight, 0.0),
    field("sheen_color", COLOR, Role::SheenColor, 1.0),
    field("sheen_roughness", UNIT_KIND, Role::SheenRoughness, 0.3),
    field("emission", NON_NEGATIVE_KIND, Role::EmissionStrength, 0.0),
    field("emission_color", COLOR, Role::EmissionColor, 1.0),
    field("opacity", COLOR, Role::Opacity, 1.0),
    field("normal", Kind::Vector, Role::Normal, 0.0),
    field("coat_normal", Kind::Vector, Role::CoatKept, 0.0),
    field("specular_color", COLOR, Role::Kept, 1.0),
    field("specular_anisotropy", UNIT_KIND, Role::Kept, 0.0),
    field("transmission", UNIT_KIND, Role::Kept, 0.0),
    field("subsurface", UNIT_KIND, Role::Kept, 0.0),
    field("coat_color", COLOR, Role::CoatKept, 1.0),
    field("coat_IOR", NON_NEGATIVE_KIND, Role::CoatKept, 1.5),
];

/// glTF PBR inputs and defaults, from its node definition
/// (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/bxdf/gltf_pbr.mtlx>).
const GLTF_PBR: &[Field] = &[
    field("base_color", COLOR, Role::BaseColor, 1.0),
    field("metallic", UNIT_KIND, Role::Metallic, 1.0),
    field("roughness", UNIT_KIND, Role::Roughness, 1.0),
    field("specular", UNIT_KIND, Role::SpecularWeight, 1.0),
    field("ior", Kind::Scalar(1.0, f32::MAX), Role::Ior, 1.5),
    field("clearcoat", UNIT_KIND, Role::CoatWeight, 0.0),
    field("clearcoat_roughness", UNIT_KIND, Role::CoatRoughness, 0.0),
    field("sheen_color", COLOR, Role::SheenColor, 0.0),
    field("sheen_roughness", UNIT_KIND, Role::SheenRoughness, 0.0),
    field("emissive_strength", NON_NEGATIVE_KIND, Role::EmissionStrength, 1.0),
    field("emissive", COLOR, Role::EmissionColor, 0.0),
    field("alpha", UNIT_KIND, Role::Opacity, 1.0),
    field("alpha_mode", Kind::Mode, Role::AlphaMode, 0.0),
    field("alpha_cutoff", UNIT_KIND, Role::AlphaCutoff, 0.5),
    field("normal", Kind::Vector, Role::Normal, 0.0),
    field("occlusion", UNIT_KIND, Role::Occlusion, 1.0),
    field("clearcoat_normal", Kind::Vector, Role::CoatKept, 0.0),
    field("transmission", UNIT_KIND, Role::Kept, 0.0),
    field("specular_color", Kind::Color(f32::MAX), Role::Kept, 1.0),
];

/// What a conversion found for one field of the table.
#[derive(Clone, Debug, PartialEq)]
enum Got {
    Missing,
    /// Checked values, linear for a color.
    Constant(Vec<f32>),
    Texture,
}

/// The table of the material's shader and what each field holds.
struct Inputs {
    table: &'static [Field],
    got: Vec<Got>,
}

impl Inputs {
    fn find(&self, role: Role) -> Option<(&Field, &Got)> {
        self.table.iter().zip(&self.got).find(|(f, _)| f.role == role)
    }

    /// The field's value: the authored one, 1 for a texture (the factor its samples multiply),
    /// or the default when the input is absent. `None` when the shader has no such field.
    fn value(&self, role: Role) -> Option<[f32; 3]> {
        let (f, got) = self.find(role)?;
        Some(match got {
            Got::Missing => f.default,
            Got::Texture => [1.0; 3],
            Got::Constant(v) if v.len() == 1 => [v[0]; 3],
            Got::Constant(v) => [v.first().copied().unwrap_or(0.0), v.get(1).copied().unwrap_or(0.0), v.get(2).copied().unwrap_or(0.0)],
        })
    }

    fn scalar(&self, role: Role, absent: f32) -> f32 {
        self.value(role).map_or(absent, |v| v[0])
    }

    fn textured(&self, role: Role) -> bool {
        self.find(role).is_some_and(|(_, g)| *g == Got::Texture)
    }

    fn name(&self, role: Role) -> &'static str {
        self.table.iter().find(|f| f.role == role).map_or("", |f| f.name)
    }
}

/// Converts a `craft_materialx::StrictMaterial`, as `Document::read_material_strict` reads it, by
/// the input tables of its shader's node definition
/// (<https://github.com/AcademySoftwareFoundation/MaterialX/tree/main/libraries/bxdf>). An input
/// that is absent takes the definition's default, so an empty input list gives the shader's
/// default material.
///
/// - The core's base color is the base color times the base weight (`base_weight`, or `base` in
///   Standard Surface), which both shaders apply to the diffuse and the metal color. Metallic
///   and roughness map as they are. The core's specular weight is the specular weight times
///   Schlick's reflectance at normal incidence for the index of refraction,
///   `((ior − 1) / (ior + 1))²`, divided by 0.04; it is reported as approximated when it is not 1,
///   and a value over [`MAX_SPECULAR`] is an error.
/// - The coat (`coat_*`, or `clearcoat*` in glTF PBR) becomes [`Surface::clearcoat`] when its
///   weight is above 0 or a texture sets it. The fuzz of OpenPBR and the sheen of Standard
///   Surface become [`Surface::sheen`] with the weight times the color as its color, reported as
///   approximated; glTF PBR's sheen maps as it is. A layer is kept only when it is on.
/// - Emission is the emission color times its strength: `emission_luminance` (reported as
///   approximated, since it is in nits), `emission`, or `emissive_strength`. A color whose
///   strength is 0 gives no emission.
///   An authored color at strength 0 is validated and reported as dropped.
/// - Alpha is the opacity. Standard Surface's opacity is a color: equal channels give that value,
///   and different channels give their luminance by the Rec.709 luma coefficients, reported as
///   approximated, as the shader's own node graph reduces it with a `luminance` node. OpenPBR's
///   `geometry_opacity` and glTF PBR's `alpha` are floats in their node definitions, so a color
///   there is [`ConvertError::Components`]. glTF PBR's `alpha_mode` 0, 1 and 2 give opaque,
///   masked at `alpha_cutoff`, and blended; the other shaders blend when the opacity is below 1
///   or a texture sets it.
/// - Every other input of the table (normals, specular color, anisotropy, transmission,
///   subsurface, coat color and IOR, occlusion) is kept in [`Surface::other_inputs`], or as a
///   texture, and reported as unshaded. The displacement is kept the same way.
/// - An image becomes a [`TextureBinding`] with the file path, the UV tiling as its transform's
///   scale, and an encoding from its color space for a color input (linear for data); the core
///   holds 1 times any weight in its place; Standard Surface's color `opacity` binds as
///   [`TextureInput::OpacityRgb`] and the float opacities as [`TextureInput::Opacity`]; an IOR
///   texture leaves the default IOR, reported
///   as approximated. The inputs and textures of a layer that is off (the coat's roughness,
///   color, IOR and normal among them) are dropped and reported.
/// - Color constants are decoded to linear Rec.709 from the color space in effect, as the
///   MaterialX specification asks
///   (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/documents/Specification/MaterialX.Specification.md#color-spaces-and-color-management-systems>):
///   `srgb_texture` and `srgb_rec709_scene` by the sRGB transfer function, the Rec.709 gamma
///   spaces by their power, and `lin_rec709`, `lin_rec709_scene`, `data`, `none` or no color
///   space as they are. Another color space is [`ConvertError::ColorSpace`].
/// - The reader's warnings are carried into the report. Inputs outside the table are dropped and
///   reported.
///
/// Every value is checked and none is clamped: a value that is not finite (a color also after it
/// is decoded to linear), is outside the definition's range (`0..=1` for weights, colors and
/// roughness, at least 0 for strengths and IOR, at least 1 for glTF PBR's `ior`), has the wrong
/// number of components, or is an
/// `alpha_mode` other than 0, 1 or 2 is an error, and so is a texture on `alpha_mode` or
/// `alpha_cutoff`, a UV tiling that is not two positive numbers, or an input named twice.
pub fn try_from_materialx(m: &StrictMaterial) -> Result<Conversion, ConvertError> {
    let table = match m.shader {
        Shader::OpenPbr => OPEN_PBR,
        Shader::StandardSurface => STANDARD_SURFACE,
        Shader::GltfPbr => GLTF_PBR,
    };
    let mut report = ConversionReport::default();
    let mut inputs = Inputs { table, got: table.iter().map(|_| Got::Missing).collect() };
    let mut textures: Vec<(Role, TextureBinding)> = Vec::new();
    let mut others: Vec<(Role, String, Vec<f32>)> = Vec::new();
    let mut seen = HashSet::new();
    for (name, value) in &m.inputs {
        if !seen.insert(name.as_str()) {
            return Err(ConvertError::Duplicate(name.clone()));
        }
        let Some(idx) = table.iter().position(|f| f.name == name) else {
            report.items.push(ReportItem::Dropped { input: name.clone(), why: "is not an input of the shader that the conversion reads" });
            continue;
        };
        let f = &table[idx];
        match value {
            Resolved::Constant { values, colorspace } => {
                let v = checked_values(f, values, colorspace.as_deref())?;
                if matches!(f.role, Role::Kept | Role::CoatKept | Role::Normal | Role::Occlusion) {
                    others.push((f.role, name.clone(), v));
                } else {
                    inputs.got[idx] = Got::Constant(v);
                }
            }
            Resolved::Image { file, colorspace, uv_tiling } | Resolved::NormalMap { file, colorspace, uv_tiling } => {
                if matches!(f.role, Role::AlphaMode | Role::AlphaCutoff) {
                    return Err(ConvertError::BadValue { input: name.clone(), reason: "cannot be a texture" });
                }
                let binding = image_binding(f, file, colorspace.as_deref(), *uv_tiling)?;
                let tangent = matches!(value, Resolved::NormalMap { .. });
                let normal = matches!(f.role, Role::Normal) || (matches!(f.role, Role::CoatKept) && matches!(f.kind, Kind::Vector));
                if normal && !tangent {
                    report.items.push(ReportItem::Dropped { input: name.clone(), why: "world-space normal images are not supported" });
                    continue;
                }
                if tangent && !normal {
                    report.items.push(ReportItem::Dropped { input: name.clone(), why: "a normalmap image is not supported on this input" });
                    continue;
                }
                textures.push((f.role, binding));
                inputs.got[idx] = Got::Texture;
            }
        }
    }
    let material = core(&inputs, &mut report)?;
    let clearcoat = clearcoat(&inputs);
    let sheen = sheen(&inputs, m.shader, &mut report);
    let emissive = emission(&inputs, m.shader, &mut report);
    let (alpha, alpha_mode) = alpha(&inputs, m.shader, &mut report)?;
    let off = |role: Role| match role {
        Role::CoatWeight | Role::CoatRoughness | Role::CoatKept if clearcoat.is_none() => Some(COAT_OFF),
        Role::SheenWeight | Role::SheenColor | Role::SheenRoughness if sheen.is_none() => Some(SHEEN_OFF),
        Role::EmissionColor if inputs.scalar(Role::EmissionStrength, 1.0) == 0.0 => Some("the emission strength is 0"),
        _ => None,
    };
    // Authored constants of a layer that is off, except the weight or color that turns it off.
    for (f, got) in table.iter().zip(&inputs.got) {
        let Got::Constant(v) = got else { continue };
        let turns_it_off = match f.role {
            Role::CoatWeight => true,
            Role::SheenWeight | Role::SheenColor => v.iter().all(|c| *c == 0.0),
            _ => false,
        };
        if let Some(why) = off(f.role).filter(|_| !turns_it_off) {
            report.items.push(ReportItem::Dropped { input: f.name.to_owned(), why });
        }
    }
    let mut other_inputs = Vec::new();
    for (role, input, values) in others {
        match off(role) {
            Some(why) => report.items.push(ReportItem::Dropped { input, why }),
            None => {
                report.items.push(ReportItem::Unshaded { input: input.clone() });
                other_inputs.push((input, values));
            }
        }
    }
    let mut kept = Vec::new();
    for (role, binding) in textures {
        let off = off(role);
        let input = texture_name(&inputs, role, &binding);
        match off {
            Some(why) => report.items.push(ReportItem::Dropped { input, why }),
            None => {
                report.items.push(ReportItem::Unshaded { input });
                kept.push(binding);
            }
        }
    }
    for (layer, on) in [("clearcoat", clearcoat.is_some()), ("sheen", sheen.is_some())] {
        if on {
            report.items.push(ReportItem::Unshaded { input: layer.into() });
        }
    }
    displacement(m.displacement.as_ref(), &mut kept, &mut other_inputs, &mut report)?;
    report.items.extend(m.warnings.iter().cloned().map(ReportItem::Warning));
    let surface = Surface {
        name: m.name.clone(),
        material,
        alpha,
        alpha_mode,
        emissive,
        double_sided: false,
        unlit: false,
        clearcoat,
        sheen,
        textures: kept,
        other_inputs,
    };
    Ok(Conversion { surface, report })
}

/// The source name of a texture for the report.
fn texture_name(inputs: &Inputs, role: Role, binding: &TextureBinding) -> String {
    match &binding.input {
        TextureInput::Other(name) => name.clone(),
        _ => inputs.name(role).to_owned(),
    }
}

/// The reason text for a value outside `lo..=hi`.
fn reason(lo: f32, hi: f32) -> &'static str {
    match (lo, hi) {
        (0.0, 1.0) => UNIT,
        (0.0, _) => NON_NEGATIVE,
        (1.0, _) => "is not a finite number of at least 1",
        _ => FINITE,
    }
}

/// The values of a constant for field `f`, checked against its kind and, for a color, decoded
/// to linear from `space`.
fn checked_values(f: &Field, values: &[f32], space: Option<&str>) -> Result<Vec<f32>, ConvertError> {
    let expected = match f.kind {
        Kind::Scalar(..) | Kind::Mode => 1,
        Kind::Color(_) | Kind::Vector => 3,
    };
    if values.len() != expected {
        return Err(ConvertError::Components { input: f.name.to_owned(), expected, found: values.len() });
    }
    let bad = |reason| ConvertError::BadValue { input: f.name.to_owned(), reason };
    let (lo, hi) = match f.kind {
        Kind::Scalar(lo, hi) => (lo, hi),
        Kind::Color(hi) => (0.0, hi),
        Kind::Vector => (f32::MIN, f32::MAX),
        Kind::Mode => (0.0, 2.0),
    };
    let in_range = |v: f32| v.is_finite() && (lo..=hi).contains(&v);
    for &v in values {
        if !in_range(v) {
            return Err(bad(reason(lo, hi)));
        }
        if f.kind == Kind::Mode && v.fract() != 0.0 {
            return Err(bad("is not 0, 1 or 2"));
        }
    }
    match f.kind {
        Kind::Color(_) => {
            let linear = decode(values, space, f.name)?;
            if linear.iter().all(|&v| in_range(v)) { Ok(linear) } else { Err(bad("is not a finite color once decoded to linear")) }
        }
        _ => Ok(values.to_vec()),
    }
}

/// `values` decoded to linear Rec.709 from the color space `space`.
fn decode(values: &[f32], space: Option<&str>, input: &str) -> Result<Vec<f32>, ConvertError> {
    let power = |p: f32| values.iter().map(|v| v.powf(p)).collect();
    Ok(match space {
        None | Some("data" | "none" | "lin_rec709" | "lin_rec709_scene") => values.to_vec(),
        Some("srgb_texture" | "srgb_rec709_scene") => values.iter().map(|v| srgb_to_linear(*v)).collect(),
        Some("g22_rec709" | "g22_rec709_scene") => power(2.2),
        Some("g18_rec709" | "g18_rec709_scene") => power(1.8),
        Some("g24_rec709_scene") => power(2.4),
        Some(other) => return Err(ConvertError::ColorSpace { input: input.to_owned(), colorspace: other.to_owned() }),
    })
}

/// The binding of an image on field `f`.
fn image_binding(f: &Field, file: &str, space: Option<&str>, tiling: Option<[f32; 2]>) -> Result<TextureBinding, ConvertError> {
    let input = match f.role {
        Role::BaseColor => TextureInput::BaseColor,
        Role::Metallic => TextureInput::Metallic,
        Role::Roughness => TextureInput::Roughness,
        Role::Normal => TextureInput::Normal,
        Role::Occlusion => TextureInput::Occlusion,
        Role::EmissionColor => TextureInput::Emissive,
        Role::Opacity if matches!(f.kind, Kind::Color(_)) => TextureInput::OpacityRgb,
        Role::Opacity => TextureInput::Opacity,
        Role::CoatWeight => TextureInput::Clearcoat,
        Role::CoatRoughness => TextureInput::ClearcoatRoughnessFirst,
        Role::SheenColor => TextureInput::SheenColor,
        Role::SheenRoughness => TextureInput::SheenRoughnessFirst,
        _ => TextureInput::Other(f.name.to_owned()),
    };
    let encoding = if matches!(f.kind, Kind::Color(_)) { encoding(space) } else { TextureEncoding::Linear };
    let transform = match tiling {
        Some(scale) if scale.iter().all(|v| v.is_finite() && *v > 0.0) => Some(UvTransform { offset: [0.0; 2], rotation: 0.0, scale }),
        Some(_) => return Err(ConvertError::BadValue { input: f.name.to_owned(), reason: "has a UV tiling that is not two positive numbers" }),
        None => None,
    };
    Ok(TextureBinding { input, source: TextureSource::File(file.to_owned()), encoding, uv_set: 0, wrap: [Wrap::Repeat; 2], transform, scale: 1.0 })
}

/// The encoding of a color image in the color space `space`.
fn encoding(space: Option<&str>) -> TextureEncoding {
    match space {
        None | Some("data" | "none" | "lin_rec709" | "lin_rec709_scene") => TextureEncoding::Linear,
        Some("srgb_texture" | "srgb_rec709_scene") => TextureEncoding::Srgb,
        Some(other) => TextureEncoding::Named(other.to_owned()),
    }
}

/// The BRDF core.
fn core(inputs: &Inputs, report: &mut ConversionReport) -> Result<Material, ConvertError> {
    let color = inputs.value(Role::BaseColor).unwrap_or([1.0; 3]);
    let weight = inputs.scalar(Role::BaseWeight, 1.0);
    let base = color.map(|c| c * weight);
    let metallic = inputs.scalar(Role::Metallic, 0.0);
    let roughness = inputs.scalar(Role::Roughness, 0.0);
    if inputs.textured(Role::Ior) {
        report
            .items
            .push(ReportItem::Approximated { input: inputs.name(Role::Ior).into(), how: "is a texture; the default index of refraction was used" });
    }
    let ior = match inputs.find(Role::Ior) {
        Some((f, Got::Texture)) => f64::from(f.default[0]),
        _ => f64::from(inputs.scalar(Role::Ior, 1.5)),
    };
    // The reflectance relative to 0.04, which is 1 within rounding at the default IOR of 1.5.
    let mut ratio = ((ior - 1.0) / (ior + 1.0)).powi(2) / 0.04;
    if (ratio - 1.0).abs() < 1e-9 {
        ratio = 1.0;
    }
    let specular = f64::from(inputs.scalar(Role::SpecularWeight, 1.0)) * ratio;
    let name = inputs.name(Role::SpecularWeight);
    if specular > f64::from(MAX_SPECULAR) {
        return Err(ConvertError::BadValue {
            input: name.into(),
            reason: "times the reflectance of the index of refraction is over the 25 the BRDF supports",
        });
    }
    if specular != 1.0 {
        report.items.push(ReportItem::Approximated {
            input: name.into(),
            how: "times Schlick's reflectance at normal incidence for the index of refraction, over 0.04, is the core's specular weight",
        });
    }
    Ok(Material::try_new(base, metallic, roughness)?.try_with_weights(specular as f32, 1.0)?)
}

/// The coat, when its weight is above 0 or a texture sets it.
fn clearcoat(inputs: &Inputs) -> Option<Clearcoat> {
    let weight = inputs.scalar(Role::CoatWeight, 0.0);
    (weight > 0.0).then(|| Clearcoat { weight, roughness: inputs.scalar(Role::CoatRoughness, 0.0) })
}

/// The sheen, when its color times its weight is not black.
fn sheen(inputs: &Inputs, shader: Shader, report: &mut ConversionReport) -> Option<Sheen> {
    let weight = inputs.scalar(Role::SheenWeight, 1.0);
    let color = inputs.value(Role::SheenColor)?.map(|c| c * weight);
    if color == [0.0; 3] {
        return None;
    }
    if shader != Shader::GltfPbr {
        report.items.push(ReportItem::Approximated { input: inputs.name(Role::SheenWeight).into(), how: "times the color is the sheen color" });
    }
    Some(Sheen { color, roughness: inputs.scalar(Role::SheenRoughness, 0.0) })
}

/// The emission color times its strength.
fn emission(inputs: &Inputs, shader: Shader, report: &mut ConversionReport) -> [f32; 3] {
    let strength = inputs.scalar(Role::EmissionStrength, 1.0);
    let color = inputs.value(Role::EmissionColor).unwrap_or([0.0; 3]);
    let emissive = color.map(|c| c * strength);
    if shader == Shader::OpenPbr && emissive != [0.0; 3] {
        report.items.push(ReportItem::Approximated {
            input: inputs.name(Role::EmissionStrength).into(),
            how: "is in nits and multiplies the emission color",
        });
    }
    emissive
}

/// Alpha and its mode.
fn alpha(inputs: &Inputs, shader: Shader, report: &mut ConversionReport) -> Result<(f32, AlphaMode), ConvertError> {
    let opacity = inputs.value(Role::Opacity).unwrap_or([1.0; 3]);
    let alpha = if shader == Shader::StandardSurface && (opacity[0] != opacity[1] || opacity[1] != opacity[2]) {
        report
            .items
            .push(ReportItem::Approximated { input: inputs.name(Role::Opacity).into(), how: "is a color; its Rec.709 luminance is the alpha" });
        luminance(opacity)
    } else {
        opacity[0]
    };
    let mode = if shader == Shader::GltfPbr {
        match inputs.scalar(Role::AlphaMode, 0.0) {
            0.0 => AlphaMode::Opaque,
            1.0 => AlphaMode::Mask(inputs.scalar(Role::AlphaCutoff, 0.5)),
            2.0 => AlphaMode::Blend,
            _ => return Err(ConvertError::BadValue { input: "alpha_mode".into(), reason: "is not 0, 1 or 2" }),
        }
    } else if alpha < 1.0 || inputs.textured(Role::Opacity) {
        AlphaMode::Blend
    } else {
        AlphaMode::Opaque
    };
    if shader == Shader::GltfPbr
        && !matches!(mode, AlphaMode::Mask(_))
        && inputs.find(Role::AlphaCutoff).is_some_and(|(_, value)| matches!(value, Got::Constant(_)))
    {
        report.items.push(ReportItem::Dropped { input: inputs.name(Role::AlphaCutoff).into(), why: "the alpha mode does not use a cutoff" });
    }
    Ok((alpha, mode))
}

/// The luminance of a linear Rec.709 color, by the `rec709` luma coefficients of the MaterialX
/// `luminance` node
/// (<https://github.com/AcademySoftwareFoundation/MaterialX/blob/main/libraries/stdlib/stdlib_defs.mtlx>),
/// kept between the smallest and largest channel so that rounding cannot leave `0..=1`.
fn luminance(c: [f32; 3]) -> f32 {
    let y = 0.2126 * f64::from(c[0]) + 0.7152 * f64::from(c[1]) + 0.0722 * f64::from(c[2]);
    (y as f32).clamp(c[0].min(c[1]).min(c[2]), c[0].max(c[1]).max(c[2]))
}

/// Keeps the displacement as a texture or an other input.
fn displacement(
    value: Option<&Resolved>,
    textures: &mut Vec<TextureBinding>,
    other_inputs: &mut Vec<(String, Vec<f32>)>,
    report: &mut ConversionReport,
) -> Result<(), ConvertError> {
    const DISPLACEMENT: Field = field("displacement", Kind::Scalar(f32::MIN, f32::MAX), Role::Kept, 0.0);
    match value {
        None => return Ok(()),
        Some(Resolved::Constant { values, .. }) => other_inputs.push(("displacement".into(), checked_values(&DISPLACEMENT, values, None)?)),
        Some(Resolved::Image { file, colorspace, uv_tiling }) => {
            let mut binding = image_binding(&DISPLACEMENT, file, colorspace.as_deref(), *uv_tiling)?;
            binding.input = TextureInput::Displacement;
            textures.push(binding);
        }
        Some(Resolved::NormalMap { .. }) => {
            return Err(ConvertError::BadValue { input: "displacement".into(), reason: "cannot be a normalmap image" });
        }
    }
    report.items.push(ReportItem::Unshaded { input: "displacement".into() });
    Ok(())
}
