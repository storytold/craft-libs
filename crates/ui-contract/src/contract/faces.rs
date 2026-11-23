//! Document faces on the contexts a component sees.

use super::{Component, Cx, MeasureCx};
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};
use crate::primitives::FacesCx;

impl<C: Component> FacesCx for Cx<'_, C> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes> + Send + 'static) -> FaceState {
        DocumentFaces::face(&self.core.ctx, key, load)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(&self.core.ctx, key)
    }
}

impl<C: Component> FacesCx for MeasureCx<'_, C> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes> + Send + 'static) -> FaceState {
        DocumentFaces::face(self.ui.ctx(), key, load)
    }

    fn document_face_bytes(&self, key: &str) -> Option<FaceBytes> {
        DocumentFaces::bytes(self.ui.ctx(), key)
    }
}
