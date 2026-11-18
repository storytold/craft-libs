//! PCX compatibility facade; implementation and header probing live in craft-image-codecs.
use crate::{CodecError, Image, Limits, bridge};
use craft_image_codecs::Format;

pub fn looks_like_pcx(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Pcx)
}
pub fn dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    craft_image_codecs::pcx_dimensions(bytes)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Pcx, bytes, limits)
}
