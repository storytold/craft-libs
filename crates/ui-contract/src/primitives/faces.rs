//! Document faces: the chapter trait and its content-painter context.

use super::content::ContentPainterCx;
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};

/// Where a document face is asked for: a component's context, its measure or a content painter.
pub trait FacesCx {
    /// The face registered under `key` as a family of its own, read from a font catalog. `job` is called only for a
    /// key the registry does not know (a ready face costs nothing) and makes the load, which runs as a background job,
    /// once while the face is kept; the face is ready from the pass after it ends. A load that returns `None` marks the
    /// key unavailable for as long as the context lives. Every call marks the face drawn in this frame.
    fn document_face<L>(&self, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static;

    /// The bytes of the face `document_face` registered under `key`, once it is ready: for reading its tables without
    /// loading the file again.
    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes>;
}

impl FacesCx for ContentPainterCx<'_> {
    fn document_face<L>(&self, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static,
    {
        DocumentFaces::face(self.painter.ctx(), key, job)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(self.painter.ctx(), key)
    }
}
