//! Document faces: the chapter trait and its content-painter context.

use super::content::ContentPainterCx;
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};

/// Where a document face is asked for: a component's context, its measure or a content painter.
pub trait FacesCx {
    /// The face registered under `key` as a family of its own. `load` runs only for a new key within the frame's
    /// budget of `DOCUMENT_FACES_PER_PASS`, and for a key whose family a replacement of the definitions removed. A
    /// `load` that returns `None` marks the key unavailable for as long as the context lives. Every call marks the
    /// face drawn in this frame.
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState;
}

impl FacesCx for ContentPainterCx<'_> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        DocumentFaces::face(self.painter.ctx(), key, load)
    }
}
