//! Document faces: the chapter trait and its content-painter context.

use super::content::ContentPainterCx;
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};

/// Where a document face is asked for: a component's context, its measure or a content painter.
pub trait FacesCx {
    /// The face registered under `key` as a family of its own, read from a font catalog. `load` runs as a background
    /// job, once while the face is kept, and the face is ready from the pass after it ends. A `load` that returns
    /// `None` marks the key unavailable for as long as the context lives. Every call marks the face drawn in this frame.
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes> + Send + 'static) -> FaceState;

    /// The bytes of the face `document_face` registered under `key`, once it is ready: for reading its tables without
    /// loading the file again.
    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes>;
}

impl FacesCx for ContentPainterCx<'_> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes> + Send + 'static) -> FaceState {
        DocumentFaces::face(self.painter.ctx(), key, load)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(self.painter.ctx(), key)
    }
}
