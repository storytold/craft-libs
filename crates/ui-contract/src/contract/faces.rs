//! Document faces on the contexts a component sees.

use super::{Component, Cx, MeasureCx};
use crate::fonts::document::DocumentFaces;
use crate::fonts::{FaceBytes, FaceState};
use crate::primitives::FacesCx;

impl<C: Component> FacesCx for Cx<'_, C> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        DocumentFaces::face(&self.core.ctx, key, load)
    }
}

impl<C: Component> FacesCx for MeasureCx<'_, C> {
    fn document_face(&self, key: &str, load: impl FnOnce() -> Option<FaceBytes>) -> FaceState {
        DocumentFaces::face(self.ui.ctx(), key, load)
    }
}
