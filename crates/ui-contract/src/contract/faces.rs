//! Document faces on the contexts a component sees.

use super::{Component, Cx, MeasureCx};
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};
use crate::primitives::FacesCx;

impl<C: Component> FacesCx for Cx<'_, C> {
    fn document_face<L>(&self, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static,
    {
        DocumentFaces::face(&self.core.ctx, key, job)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(&self.core.ctx, key)
    }
}

impl<C: Component> FacesCx for MeasureCx<'_, C> {
    fn document_face<L>(&self, key: &str, job: impl FnOnce() -> L) -> FaceState
    where
        L: FnOnce() -> Option<FaceBytes> + Send + 'static,
    {
        DocumentFaces::face(self.ui.ctx(), key, job)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(self.ui.ctx(), key)
    }
}
