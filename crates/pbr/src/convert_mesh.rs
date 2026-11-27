//! The conversion from a `craft_mesh::Material`, which holds a glTF 2.0 metallic-roughness
//! material.

use craft_mesh::{AlphaMode as MeshAlpha, Material as MeshMaterial, TextureRef, Wrap as MeshWrap};

use crate::Material;
use crate::surface::{
    AlphaMode, Clearcoat, Conversion, ConversionReport, ConvertError, FINITE, NON_NEGATIVE, ReportItem, Sheen, Surface, TextureBinding,
    TextureEncoding, TextureInput, TextureSource, UNIT, UvTransform, Wrap, checked,
};

/// The material extensions whose values `craft_mesh` reads into `craft_mesh::Material`'s fields.
/// It reads only part of `KHR_materials_pbrSpecularGlossiness` and of `KHR_materials_clearcoat`.
const CONVERTED_EXTENSIONS: &[&str] = &[
    "KHR_materials_unlit",
    "KHR_materials_emissive_strength",
    "KHR_materials_pbrSpecularGlossiness",
    "KHR_materials_clearcoat",
    "KHR_materials_sheen",
];
const SPECULAR_GLOSSINESS: &str = "KHR_materials_pbrSpecularGlossiness";

/// Converts a `craft_mesh::Material` as glTF 2.0 defines its metallic-roughness material
/// (<https://registry.khronos.org/glTF/specs/2.0/glTF-2.0.html#materials>).
///
/// - Mapped as they are: the linear base color RGB, metallic and roughness into the core with
///   specular and diffuse weights of 1, base color alpha into [`Surface::alpha`], the alpha mode
///   and cutoff, the linear emissive color, the double-sided flag, the unlit flag of
///   `KHR_materials_unlit` and the name. Nothing is converted between color spaces: the source
///   values are linear.
/// - Kept and reported as [`ReportItem::Unshaded`]: every texture, with its image index, UV set,
///   wrap modes and `KHR_texture_transform`, and the normal scale and occlusion strength as the
///   binding's scale; the clearcoat layer of `KHR_materials_clearcoat`; the sheen layer of
///   `KHR_materials_sheen`; and the unlit flag. Base color, emissive and sheen color textures
///   are sRGB-encoded and the others are linear, as glTF and its extensions define them.
/// - Dropped and reported: the clearcoat textures when the clearcoat factor is 0, and the sheen
///   textures when the sheen color factor is black. Each extension turns its layer off then.
///   Every other material extension in `craft_mesh::Material::extensions`, such as
///   `KHR_materials_transmission`, is dropped and reported by its name, and so is the
///   `clearcoatNormalTexture` of `KHR_materials_clearcoat`, which `craft_mesh` does not read.
/// - Reported as [`ReportItem::Approximated`]: `KHR_materials_pbrSpecularGlossiness`, which
///   `craft_mesh` reads as its diffuse color and texture with roughness from the glossiness,
///   without its specular color or specular-glossiness texture.
///
/// Every value is checked and none is clamped. Base color channels, alpha, metallic,
/// roughness, occlusion strength and the clearcoat and sheen values must be numbers from 0 to 1;
/// emissive channels and the alpha cutoff must be finite and at least 0; the normal scale and
/// every UV transform value must be finite. Otherwise the result is
/// [`ConvertError::BadValue`] naming the `craft_mesh::Material` field.
pub fn try_from_mesh(m: &MeshMaterial) -> Result<Conversion, ConvertError> {
    let [r, g, b, a] = m.base_color;
    let base = [checked(r, 0.0, 1.0, "base_color", UNIT)?, checked(g, 0.0, 1.0, "base_color", UNIT)?, checked(b, 0.0, 1.0, "base_color", UNIT)?];
    let alpha = checked(a, 0.0, 1.0, "base_color alpha", UNIT)?;
    let metallic = checked(m.metallic, 0.0, 1.0, "metallic", UNIT)?;
    let roughness = checked(m.roughness, 0.0, 1.0, "roughness", UNIT)?;
    let material = Material::try_new(base, metallic, roughness)?;
    let mut emissive = [0.0; 3];
    for (out, v) in emissive.iter_mut().zip(m.emissive) {
        *out = checked(v, 0.0, f32::MAX, "emissive", NON_NEGATIVE)?;
    }
    let alpha_mode = match m.alpha_mode {
        MeshAlpha::Opaque => AlphaMode::Opaque,
        MeshAlpha::Mask(cutoff) => AlphaMode::Mask(checked(cutoff, 0.0, f32::MAX, "alpha cutoff", NON_NEGATIVE)?),
        MeshAlpha::Blend => AlphaMode::Blend,
    };
    let normal_scale = checked(m.normal_scale, f32::MIN, f32::MAX, "normal_scale", FINITE)?;
    let occlusion_strength = checked(m.occlusion_strength, 0.0, 1.0, "occlusion_strength", UNIT)?;
    let clearcoat_weight = checked(m.clearcoat, 0.0, 1.0, "clearcoat", UNIT)?;
    let clearcoat_roughness = checked(m.clearcoat_roughness, 0.0, 1.0, "clearcoat_roughness", UNIT)?;
    let mut sheen_color = [0.0; 3];
    for (out, v) in sheen_color.iter_mut().zip(m.sheen_color) {
        *out = checked(v, 0.0, 1.0, "sheen_color", UNIT)?;
    }
    let sheen_roughness = checked(m.sheen_roughness, 0.0, 1.0, "sheen_roughness", UNIT)?;

    let mut report = ConversionReport::default();
    let mut textures = Vec::new();
    let mut bind =
        |name: &str, slot: &Option<TextureRef>, input: TextureInput, encoding: TextureEncoding, scale: f32, dropped: Option<&'static str>| {
            let Some(t) = slot else {
                if scale != 1.0 {
                    let factor = match input {
                        TextureInput::Normal => "normal_scale",
                        TextureInput::Occlusion => "occlusion_strength",
                        _ => name,
                    };
                    report.items.push(ReportItem::Dropped { input: factor.to_owned(), why: "the corresponding texture is absent" });
                }
                return Ok(());
            };
            // The texture is checked before a layer that is off drops it.
            let b = binding(name, t, input, encoding, scale)?;
            if let Some(why) = dropped {
                report.items.push(ReportItem::Dropped { input: name.to_owned(), why });
                return Ok(());
            }
            textures.push(b);
            report.items.push(ReportItem::Unshaded { input: name.to_owned() });
            Ok::<(), ConvertError>(())
        };
    use TextureEncoding::{Linear, Srgb};
    bind("base_color_texture", &m.base_color_texture, TextureInput::BaseColor, Srgb, 1.0, None)?;
    bind("metallic_roughness_texture", &m.metallic_roughness_texture, TextureInput::MetallicRoughness, Linear, 1.0, None)?;
    bind("normal_texture", &m.normal_texture, TextureInput::Normal, Linear, normal_scale, None)?;
    bind("occlusion_texture", &m.occlusion_texture, TextureInput::Occlusion, Linear, occlusion_strength, None)?;
    bind("emissive_texture", &m.emissive_texture, TextureInput::Emissive, Srgb, 1.0, None)?;
    let coat_off = (clearcoat_weight == 0.0).then_some("the clearcoat factor is 0, which turns the layer off");
    bind("clearcoat_texture", &m.clearcoat_texture, TextureInput::Clearcoat, Linear, 1.0, coat_off)?;
    bind("clearcoat_roughness_texture", &m.clearcoat_roughness_texture, TextureInput::ClearcoatRoughness, Linear, 1.0, coat_off)?;
    let sheen_off = (sheen_color == [0.0; 3]).then_some("the sheen color factor is black, which turns the layer off");
    bind("sheen_color_texture", &m.sheen_color_texture, TextureInput::SheenColor, Srgb, 1.0, sheen_off)?;
    bind("sheen_roughness_texture", &m.sheen_roughness_texture, TextureInput::SheenRoughness, Linear, 1.0, sheen_off)?;

    let clearcoat = coat_off.is_none().then_some(Clearcoat { weight: clearcoat_weight, roughness: clearcoat_roughness });
    if clearcoat.is_some() {
        report.items.push(ReportItem::Unshaded { input: "clearcoat".into() });
    }
    let sheen = sheen_off.is_none().then_some(Sheen { color: sheen_color, roughness: sheen_roughness });
    if sheen.is_some() {
        report.items.push(ReportItem::Unshaded { input: "sheen".into() });
    }
    for (inactive, roughness, input) in
        [(clearcoat.is_none(), clearcoat_roughness, "clearcoat_roughness"), (sheen.is_none(), sheen_roughness, "sheen_roughness")]
    {
        if inactive && roughness != 0.0 {
            report.items.push(ReportItem::Dropped { input: input.into(), why: "the layer is off" });
        }
    }
    if m.unlit {
        report.items.push(ReportItem::Unshaded { input: "unlit".into() });
    }
    for name in m.extensions.keys().filter(|k| !CONVERTED_EXTENSIONS.contains(&k.as_str())) {
        report.items.push(ReportItem::Dropped { input: name.clone(), why: "the extension is not converted" });
    }
    if m.extensions.get("KHR_materials_clearcoat").is_some_and(|c| c.get("clearcoatNormalTexture").is_some()) {
        report.items.push(ReportItem::Dropped { input: "clearcoatNormalTexture".into(), why: "craft-mesh does not read it" });
    }
    if m.extensions.contains_key(SPECULAR_GLOSSINESS) {
        report.items.push(ReportItem::Approximated {
            input: SPECULAR_GLOSSINESS.into(),
            how: "read as its diffuse color and texture with roughness from the glossiness; its specular color and texture are not converted",
        });
    }
    let surface = Surface {
        name: m.name.clone(),
        material,
        alpha,
        alpha_mode,
        emissive,
        double_sided: m.double_sided,
        unlit: m.unlit,
        clearcoat,
        sheen,
        textures,
        other_inputs: Vec::new(),
    };
    Ok(Conversion { surface, report })
}

/// The binding of the texture `t` in the field `name`.
fn binding(name: &str, t: &TextureRef, input: TextureInput, encoding: TextureEncoding, scale: f32) -> Result<TextureBinding, ConvertError> {
    let transform = match t.transform {
        Some(x) => {
            for v in [x.offset[0], x.offset[1], x.rotation, x.scale[0], x.scale[1]] {
                checked(v, f32::MIN, f32::MAX, &format!("{name} transform"), FINITE)?;
            }
            Some(UvTransform { offset: x.offset, rotation: x.rotation, scale: x.scale })
        }
        None => None,
    };
    let wrap = t.wrap.map(|w| match w {
        MeshWrap::Repeat => Wrap::Repeat,
        MeshWrap::Clamp => Wrap::Clamp,
        MeshWrap::Mirror => Wrap::Mirror,
    });
    Ok(TextureBinding { input, source: TextureSource::Image(t.image), encoding, uv_set: t.uv_set, wrap, transform, scale })
}
