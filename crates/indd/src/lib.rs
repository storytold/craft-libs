//! Shared InDesign document conversion boundary.
//!
//! InDesign's binary document model is converted to IDML here. Applications can import the
//! resulting package through [`craft_idml`] while retaining only their own model and UI mapping.

pub use indd::header::SIGNATURE;
pub use indd::{Conversion, Error, Preview, Warning};

/// Whether `bytes` begins with the fixed signature of an InDesign document.
pub fn is_indd(bytes: &[u8]) -> bool {
    bytes.starts_with(&SIGNATURE)
}

/// Convert an InDesign document to IDML. `name` is the file name stored in the package.
pub fn convert(bytes: &[u8], name: &str) -> Result<Conversion, Error> {
    indd::convert(bytes, name)
}
