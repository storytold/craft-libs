//! FITS compatibility facade; implementation lives in craft-image-codecs.
use crate::{CodecError, Image, Limits, bridge};
use craft_image_codecs::Format;

pub fn looks_like_fits(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Fits)
}
pub fn has_additional_images(bytes: &[u8]) -> Result<bool, CodecError> {
    craft_image_codecs::has_unretained_source_data(bytes).map_err(bridge::from_shared_error)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Fits, bytes, limits)
}
pub fn encode(image: &Image) -> Result<Vec<u8>, CodecError> {
    bridge::encode(image, Format::Fits, &Default::default())
}
