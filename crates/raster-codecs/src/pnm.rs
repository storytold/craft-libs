//! Netpbm compatibility facade; implementation and bounds live in craft-image-codecs.
use crate::{CodecError, Image, Limits, bridge};
use craft_image_codecs::Format;

pub fn looks_like_pnm(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Pnm)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Pnm, bytes, limits)
}
pub fn encode(image: &Image) -> Result<Vec<u8>, CodecError> {
    // PFM has only gray and RGB tuples. Preserve this facade's contract that
    // unsupported layouts are refused instead of silently dropping alpha.
    if image.sample_type() == crate::SampleType::F32 && !matches!(image.layout(), crate::ChannelLayout::Gray | crate::ChannelLayout::Rgb) {
        return Err(CodecError::encode("PNM", format!("PFM cannot store {:?}", image.layout())));
    }
    bridge::encode(image, Format::Pnm, &Default::default())
}
