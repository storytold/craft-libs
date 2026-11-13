//! DDS compatibility facade; implementation and BC block codecs live in craft-image-codecs.
use crate::{CodecError, Image, Limits, SampleType, bridge};
use craft_image_codecs::Format;

pub fn looks_like_dds(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Dds)
}
pub fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    craft_image_codecs::has_unretained_source_data(bytes).map_err(bridge::from_shared_error)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Dds, bytes, limits)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DdsCompression {
    #[default]
    None,
    Bc1,
    Bc2,
    Bc3,
}
impl DdsCompression {
    pub const ALL: [Self; 4] = [Self::None, Self::Bc1, Self::Bc2, Self::Bc3];
    pub fn name(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Bc1 => "bc1",
            Self::Bc2 => "bc2",
            Self::Bc3 => "bc3",
        }
    }
    pub fn from_name(name: &str) -> Option<Self> {
        match name.to_ascii_lowercase().as_str() {
            "none" => Some(Self::None),
            "bc1" | "dxt1" => Some(Self::Bc1),
            "bc2" | "dxt3" => Some(Self::Bc2),
            "bc3" | "dxt5" => Some(Self::Bc3),
            _ => None,
        }
    }
    pub fn is_block(self) -> bool {
        self != Self::None
    }
}
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DdsOptions {
    pub compression: DdsCompression,
    pub mipmaps: bool,
}
pub fn encode(image: &Image, options: DdsOptions) -> Result<Vec<u8>, CodecError> {
    if image.sample_type() != SampleType::U8 {
        return Err(CodecError::encode("DDS", "DDS is written at 8 bits per channel"));
    }
    let compression = match options.compression {
        DdsCompression::None => craft_image_codecs::DdsCompression::None,
        DdsCompression::Bc1 => craft_image_codecs::DdsCompression::Bc1,
        DdsCompression::Bc2 => craft_image_codecs::DdsCompression::Bc2,
        DdsCompression::Bc3 => craft_image_codecs::DdsCompression::Bc3,
    };
    let opts =
        craft_image_codecs::EncodeOptions { dds: craft_image_codecs::DdsOptions { compression, mipmaps: options.mipmaps }, ..Default::default() };
    bridge::encode(image, Format::Dds, &opts)
}
