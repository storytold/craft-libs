//! IFF/ILBM compatibility facade; implementation lives in craft-image-codecs.
use crate::{CodecError, Image, Limits, bridge};
use craft_image_codecs::Format;

pub fn looks_like_iff(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Iff)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Iff, bytes, limits)
}
pub fn encode(image: &Image) -> Result<Vec<u8>, CodecError> {
    if !matches!(image.layout(), crate::ChannelLayout::Rgb | crate::ChannelLayout::Rgba) || image.sample_type() != crate::SampleType::U8 {
        return Err(CodecError::encode("IFF", "ILBM writer stores 8-bit RGB pixels with optional alpha"));
    }
    bridge::encode(image, Format::Iff, &Default::default())
}
