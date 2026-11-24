//! Flat OpenDocument text (`.fodt`): taken apart into a package ([`crate::flat`]) and opened as
//! one; saved by putting a package's files together.

use craft_wordmodel::{Document, SectionProps};

use crate::{Limits, flat, mime};

/// Open a flat OpenDocument text document.
pub fn import_flat(bytes: &[u8]) -> Result<Document, String> {
    import_flat_on(bytes, &SectionProps::default())
}

/// [`import_flat`] onto `page` (see [`super::import_on`]).
pub fn import_flat_on(bytes: &[u8], page: &SectionProps) -> Result<Document, String> {
    let package = flat::to_package(bytes, mime::TEXT, &Limits::DEFAULT).map_err(|e| format!("FODT: {e}"))?;
    super::import_on(&package, page)
}

/// Save `doc` as flat OpenDocument text: its package's files in one `office:document`, pictures and
/// formulas inline.
pub fn export_flat(doc: &Document) -> Result<Vec<u8>, String> {
    let package = super::export(doc)?;
    flat::from_package(&package, &Limits::DEFAULT).map_err(|e| format!("FODT: {e}"))
}
