//! DICOM compatibility facade; implementation and detection live in craft-image-codecs.
use crate::{CodecError, Image, Limits, bridge};
use craft_image_codecs::Format;

pub fn is_dicom(bytes: &[u8]) -> bool {
    craft_image_codecs::detect(bytes) == Some(Format::Dicom)
}
pub fn decode(bytes: &[u8], limits: &Limits) -> Result<Image, CodecError> {
    bridge::format(Format::Dicom, bytes, limits)
}
