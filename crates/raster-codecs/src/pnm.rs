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
    bridge::encode(image, Format::Pnm, &Default::default())
}
