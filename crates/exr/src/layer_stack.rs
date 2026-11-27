//! The optional `craftLayerStack` string attribute describes an editable raster layer stack.
//! The EXR remains readable as an ordinary image without this attribute. Version 1 is an
//! ASCII-safe JSON object. Layer array order is bottom-to-top within each parent. The beauty
//! image and other passes can be stored in EXR parts or channels outside the manifest.

use std::collections::HashMap;
use std::fmt::{self, Write};

use serde::{Deserialize, Serialize};

pub const ATTRIBUTE_NAME: &str = "craftLayerStack";
pub const MAX_ATTRIBUTE_BYTES: usize = 1 << 20;
const VERSION: u64 = 1;
const MAX_LAYERS: usize = 4096;
const MAX_GROUP_DEPTH: usize = 100;

#[derive(Debug)]
pub enum Error {
    Limit(&'static str),
    Malformed(&'static str),
    Json(serde_json::Error),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{ATTRIBUTE_NAME}: ")?;
        match self {
            Self::Limit(message) | Self::Malformed(message) => f.write_str(message),
            Self::Json(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug, Clone, PartialEq)]
pub struct LayerStack {
    pub layers: Vec<Layer>,
    /// The producer's original RGB compositing encoding. A portable profile ID or
    /// `icc:<base64>` describing an ICC profile. EXR chromaticities describe the stored pixels.
    pub original_blend_profile: Option<String>,
    /// The producer's document sample depth before conversion to EXR channels.
    pub original_sample_depth: Option<SampleDepth>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SampleDepth {
    U8,
    U16,
    F32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Layer {
    pub id: u64,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<u64>,
    pub visible: bool,
    /// Portable blend mode identifier. Consumers may warn and use Normal for unsupported modes.
    pub blend: String,
    pub opacity: f32,
    pub fill_opacity: f32,
    pub clipped: bool,
    pub content: LayerContent,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum LayerContent {
    Group,
    Raster { pixels: PixelRef, sampling: PixelSampling },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase", deny_unknown_fields)]
pub enum PixelRef {
    /// A named, flat EXR part. Use part names that are unique within the file.
    Part { name: String },
    /// The exact prefix of the RGB(A) channels in the zero-based flat EXR part.
    /// An empty prefix selects unprefixed channels such as `R`, `G` and `B`.
    ChannelGroup { part: u32, prefix: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PixelSampling {
    /// Raster pixels from the source layer.
    Raw,
    /// Raster pixels derived by rendering the source layer. Transforms, effects or masks may
    /// be baked into the pixels. The manifest's stack compositing properties still apply.
    Baked,
}

#[derive(Deserialize)]
struct Version {
    version: u64,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Input {
    version: u64,
    layers: Vec<Layer>,
    original_blend_profile: Option<String>,
    original_sample_depth: Option<SampleDepth>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Output<'a> {
    version: u64,
    layers: &'a [Layer],
    #[serde(skip_serializing_if = "Option::is_none")]
    original_blend_profile: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    original_sample_depth: Option<SampleDepth>,
}

impl LayerStack {
    /// Parse the attribute, returning `None` for an unsupported version. A caller can still
    /// open the EXR as a flat image in that case.
    pub fn parse(text: &str) -> Result<Option<Self>, Error> {
        if text.len() > MAX_ATTRIBUTE_BYTES {
            return Err(Error::Limit("attribute is too large"));
        }
        if !text.trim_start().starts_with('{') {
            return Err(Error::Malformed("expected a JSON object"));
        }
        let version = serde_json::from_str::<Version>(text).map_err(Error::Json)?.version;
        if version != VERSION {
            return Ok(None);
        }
        let Input { version, layers, original_blend_profile, original_sample_depth } = serde_json::from_str(text).map_err(Error::Json)?;
        if version != VERSION {
            return Ok(None);
        }
        let stack = Self { layers, original_blend_profile, original_sample_depth };
        stack.validate()?;
        Ok(Some(stack))
    }

    /// Validate and serialize version 1. Unicode layer names are escaped to keep the EXR
    /// header ASCII-safe.
    pub fn to_attribute(&self) -> Result<String, Error> {
        self.validate()?;
        let json = serde_json::to_string(&Output {
            version: VERSION,
            layers: &self.layers,
            original_blend_profile: self.original_blend_profile.as_deref(),
            original_sample_depth: self.original_sample_depth,
        })
        .map_err(Error::Json)?;
        if json.len() > MAX_ATTRIBUTE_BYTES {
            return Err(Error::Limit("attribute is too large"));
        }
        let mut text = String::with_capacity(json.len());
        for c in json.chars() {
            if c.is_ascii() {
                text.push(c);
            } else {
                for unit in c.encode_utf16(&mut [0; 2]) {
                    write!(text, "\\u{unit:04x}").map_err(|_| Error::Limit("could not encode attribute text"))?;
                }
            }
            if text.len() > MAX_ATTRIBUTE_BYTES {
                return Err(Error::Limit("attribute is too large"));
            }
        }
        Ok(text)
    }

    /// Check the manifest's intrinsic constraints. Consumers check its pixel references
    /// against the file headers before decoding.
    pub fn validate(&self) -> Result<(), Error> {
        if self.layers.len() > MAX_LAYERS {
            return Err(Error::Limit("too many layers"));
        }
        if self.original_blend_profile.as_deref().is_some_and(|profile| !valid_profile(profile)) {
            return Err(Error::Malformed("invalid original RGB blend profile"));
        }
        let mut text_bytes = self.original_blend_profile.as_ref().map_or(0, String::len);
        let mut by_id = HashMap::with_capacity(self.layers.len());
        for layer in &self.layers {
            if by_id.insert(layer.id, layer).is_some() {
                return Err(Error::Malformed("duplicate layer ID"));
            }
            if layer.name.len() > 4096 {
                return Err(Error::Limit("layer name is too large"));
            }
            if !valid_label(&layer.blend) {
                return Err(Error::Malformed("invalid blend mode identifier"));
            }
            if !(0.0..=1.0).contains(&layer.opacity) || !(0.0..=1.0).contains(&layer.fill_opacity) {
                return Err(Error::Malformed("opacity and fill opacity must be between zero and one"));
            }
            text_bytes = text_bytes.saturating_add(layer.name.len()).saturating_add(layer.blend.len());
            if let LayerContent::Raster { pixels, .. } = &layer.content {
                let (name, min, max) = match pixels {
                    PixelRef::Part { name } => (name, 1, 255),
                    PixelRef::ChannelGroup { prefix, .. } => (prefix, 0, 253),
                };
                if !(min..=max).contains(&name.chars().count()) || name.chars().any(|c| c.is_control() || u32::from(c) > 255) {
                    return Err(Error::Malformed("invalid EXR part or channel group name"));
                }
                text_bytes = text_bytes.saturating_add(name.len());
            }
            if text_bytes > MAX_ATTRIBUTE_BYTES {
                return Err(Error::Limit("attribute is too large"));
            }
        }
        for layer in &self.layers {
            let mut at = layer;
            let mut depth = 0;
            while let Some(parent) = at.parent {
                let ancestor = by_id.get(&parent).ok_or(Error::Malformed("unknown parent layer"))?;
                if ancestor.content != LayerContent::Group {
                    return Err(Error::Malformed("a parent layer must be a group"));
                }
                if ancestor.id == layer.id {
                    return Err(Error::Malformed("cyclic layer hierarchy"));
                }
                depth += 1;
                if depth > MAX_GROUP_DEPTH {
                    return Err(Error::Limit("layer hierarchy is too deep"));
                }
                at = ancestor;
            }
        }
        Ok(())
    }
}

fn valid_label(label: &str) -> bool {
    !label.trim().is_empty() && label.len() <= 128 && label.bytes().all(|b| b == b' ' || b.is_ascii_graphic())
}

fn valid_profile(profile: &str) -> bool {
    if let Some(encoded) = profile.strip_prefix("icc:") {
        if encoded.is_empty() || encoded.len() > MAX_ATTRIBUTE_BYTES || !encoded.len().is_multiple_of(4) {
            return false;
        }
        let data = encoded.trim_end_matches('=');
        !data.is_empty() && encoded.len() - data.len() <= 2 && data.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'+' | b'/'))
    } else {
        valid_label(profile)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack() -> LayerStack {
        LayerStack {
            layers: vec![
                Layer {
                    id: u64::MAX,
                    name: "東京 🎞".into(),
                    parent: Some(8),
                    visible: false,
                    blend: "multiply".into(),
                    opacity: 0.4,
                    fill_opacity: 0.5,
                    clipped: true,
                    content: LayerContent::Raster { pixels: PixelRef::Part { name: "Layer".into() }, sampling: PixelSampling::Baked },
                },
                Layer {
                    id: 8,
                    name: "Group".into(),
                    parent: None,
                    visible: true,
                    blend: "passThrough".into(),
                    opacity: 1.0,
                    fill_opacity: 1.0,
                    clipped: false,
                    content: LayerContent::Group,
                },
            ],
            original_blend_profile: Some("icc:AAE=".into()),
            original_sample_depth: None,
        }
    }

    #[test]
    fn unicode_and_compositing_fields_round_trip_through_an_ascii_attribute() {
        let stack = stack();
        let encoded = stack.to_attribute().unwrap();
        assert!(encoded.is_ascii());
        assert_eq!(LayerStack::parse(&encoded).unwrap(), Some(stack));
    }

    #[test]
    fn original_sample_depth_is_optional_in_version_one() {
        let old = r#"{"version":1,"layers":[]}"#;
        let stack = LayerStack::parse(old).unwrap().unwrap();
        assert_eq!(stack.original_sample_depth, None);
        assert_eq!(stack.to_attribute().unwrap(), old);

        for (depth, wire) in [(SampleDepth::U8, "u8"), (SampleDepth::U16, "u16"), (SampleDepth::F32, "f32")] {
            let stack = LayerStack { original_sample_depth: Some(depth), ..stack.clone() };
            let text = format!(r#"{{"version":1,"layers":[],"originalSampleDepth":"{wire}"}}"#);
            assert_eq!(stack.to_attribute().unwrap(), text);
            assert_eq!(LayerStack::parse(&text).unwrap(), Some(stack));
        }
        assert!(LayerStack::parse(r#"{"version":1,"layers":[],"originalSampleDepth":"f16"}"#).is_err());
    }

    #[test]
    fn empty_channel_group_prefix_round_trips_but_empty_part_name_is_invalid() {
        let mut stack = stack();
        stack.layers[0].content =
            LayerContent::Raster { pixels: PixelRef::ChannelGroup { part: 0, prefix: String::new() }, sampling: PixelSampling::Raw };
        let text = stack.to_attribute().unwrap();
        assert_eq!(LayerStack::parse(&text).unwrap(), Some(stack.clone()));

        stack.layers[0].content = LayerContent::Raster { pixels: PixelRef::Part { name: String::new() }, sampling: PixelSampling::Raw };
        assert!(matches!(stack.to_attribute(), Err(Error::Malformed(_))));
    }

    #[test]
    fn supports_a_raster_inside_one_hundred_groups() {
        let mut stack = stack();
        let group = stack.layers[1].clone();
        stack.layers.truncate(1);
        stack.layers[0].parent = Some(100);
        for id in 1..=100 {
            stack.layers.push(Layer { id, parent: (id > 1).then_some(id - 1), ..group.clone() });
        }
        let text = stack.to_attribute().unwrap();
        assert_eq!(LayerStack::parse(&text).unwrap(), Some(stack.clone()));

        stack.layers.push(Layer { id: 101, parent: Some(100), ..group });
        stack.layers[0].parent = Some(101);
        assert!(matches!(stack.to_attribute(), Err(Error::Limit(_))));
    }

    #[test]
    fn supports_flat_image_fallback_for_unknown_versions_and_rejects_bad_v1() {
        assert!(LayerStack::parse(r#"{"version":2,"layout":{"different":true}}"#).unwrap().is_none());
        for text in [
            r#"{"version":1,"layers":[],"unexpected":null}"#,
            r#"{"version":1,"version":2,"layers":[]}"#,
            r#"{"version":1,"layers":[],"originalBlendProfile":"icc:abc?"}"#,
            r#"{"version":"1","layers":[]}"#,
        ] {
            assert!(LayerStack::parse(text).is_err(), "{text}");
        }
        assert!(matches!(LayerStack::parse(&" ".repeat(MAX_ATTRIBUTE_BYTES + 1)), Err(Error::Limit(_))));
    }

    #[test]
    fn rejects_ambiguous_identity_invalid_opacity_and_bad_hierarchy() {
        let mut s = stack();
        s.layers[0].id = 8;
        assert!(s.to_attribute().is_err());
        let mut s = stack();
        s.layers[0].fill_opacity = f32::NAN;
        assert!(s.to_attribute().is_err());
        let mut s = stack();
        s.layers[1].parent = Some(8);
        assert!(s.to_attribute().is_err());
        let mut s = stack();
        s.layers[1].parent = Some(u64::MAX);
        assert!(s.to_attribute().is_err());
        let mut s = stack();
        s.layers[0].parent = Some(9);
        assert!(s.to_attribute().is_err());
    }
}
