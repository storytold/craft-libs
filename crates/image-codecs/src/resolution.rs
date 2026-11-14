//! Resolution metadata edits delegated to the shared [`craft_metadata::edit`] implementation.
//!
//! Image codecs keep these wrappers so their public API and format-specific callers remain
//! stable while EXIF, XMP, and Photoshop ResolutionInfo parsing and editing live in one library.

use std::borrow::Cow;

/// The resolution (pixels per inch, horizontal and vertical) recorded in IFD0 of an EXIF block
/// or TIFF structure.
pub fn exif_resolution(exif: &[u8]) -> Option<(f32, f32)> {
    craft_metadata::edit::exif_resolution(exif)
}

/// `exif` with its IFD0 resolution set to `ppi`, borrowed when no edit is needed.
pub fn exif_with_resolution(exif: &[u8], ppi: (f32, f32)) -> Cow<'_, [u8]> {
    craft_metadata::edit::exif_with_resolution(exif, ppi)
}

/// The resolution (pixels per inch) recorded by Photoshop's ResolutionInfo image resource.
pub fn photoshop_resolution(resources: &[u8]) -> Option<(f32, f32)> {
    craft_metadata::edit::photoshop_resolution(resources)
}

/// The resolution (pixels per inch, horizontal and vertical) recorded in an XMP packet.
pub fn xmp_resolution(xmp: &str) -> Option<(f32, f32)> {
    craft_metadata::edit::xmp_resolution(xmp)
}

/// `xmp` with its resolution set to `ppi`, borrowed when no edit is needed.
pub fn xmp_with_resolution(xmp: &str, ppi: (f32, f32)) -> Cow<'_, str> {
    craft_metadata::edit::xmp_with_resolution(xmp, ppi)
}

/// The EXIF to write next to upright pixels at resolution `ppi`.
pub fn export_exif(exif: &[u8], ppi: Option<(f32, f32)>) -> Cow<'_, [u8]> {
    craft_metadata::edit::export_exif(exif, ppi)
}

/// Export EXIF, adding missing resolution entries when `ppi` is supplied.
pub(crate) fn export_exif_adding_missing_resolution(exif: &[u8], ppi: Option<(f32, f32)>) -> Cow<'_, [u8]> {
    craft_metadata::edit::export_exif_adding_missing_resolution(exif, ppi)
}

/// The XMP to write next to upright pixels at resolution `ppi`.
pub fn export_xmp(xmp: &str, ppi: Option<(f32, f32)>) -> Cow<'_, str> {
    craft_metadata::edit::export_xmp(xmp, ppi)
}

/// Resolution recorded by XMP or EXIF when a format-specific resolution field is absent.
pub fn resolution(exif: Option<&[u8]>, xmp: Option<&str>) -> Option<(f32, f32)> {
    craft_metadata::edit::resolution(exif, xmp)
}

/// Resolution a decoded file's EXIF and XMP blocks record, in precedence order.
pub(crate) fn from_metadata(exif: Option<&[u8]>, xmp: Option<&str>) -> Option<(f32, f32)> {
    craft_metadata::edit::resolution(exif, xmp)
}
