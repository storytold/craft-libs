//! The panel's previews from the font preview service: a small subset of a face instead of the whole family.
//!
//! The service's client is native only; on the web there is no service, and the panel keeps its whole-family previews.

use std::sync::Arc;

use craft_ui::fonts::FaceBytes;

use crate::index::Family;

#[cfg(target_arch = "wasm32")]
mod none;
#[cfg(not(target_arch = "wasm32"))]
mod service;

#[cfg(target_arch = "wasm32")]
pub(crate) use none::Subsets;
#[cfg(not(target_arch = "wasm32"))]
pub(crate) use service::Subsets;

/// A row of the list: the family, whose list face draws it, and the text it shows.
pub(crate) type Row<'a> = (&'a Family, String);

/// A subset that is here: its key among the document faces, and the font.
#[derive(Clone, Debug)]
pub(crate) struct SubsetFont {
    pub(crate) key: String,
    pub(crate) bytes: Arc<[u8]>,
}

impl SubsetFont {
    /// The face of the subset, at its default coordinates.
    pub(crate) fn face(&self) -> FaceBytes {
        FaceBytes::new(Arc::clone(&self.bytes), 0)
    }
}
