//! Shared EXIF/XMP/Photoshop resolution parsing and edits.

pub use craft_metadata::edit::{
    exif_resolution, exif_with_pixel_size, exif_with_resolution, exif_with_resolution_adding_missing, export_exif,
    export_exif_adding_missing_resolution, export_xmp, photoshop_resolution, resolution, xmp_resolution, xmp_with_resolution,
};

/// Resolution used by raster decode, preferring XMP over EXIF as before.
pub(crate) fn from_metadata(exif: Option<&[u8]>, xmp: Option<&str>) -> Option<(f32, f32)> {
    craft_metadata::edit::resolution(exif, xmp)
}
