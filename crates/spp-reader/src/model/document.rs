//! [`ProjectDocument`]: the whole `document.bin` as the reference app's own concepts, and its entry point.

use super::builder::ModelBuilder;
use super::fields::UnhandledSetting;
use super::raw::RawTree;
use super::report::ModelReport;
use super::texture_set::ProjectTextureSet;
use crate::decode::DecodeError;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectDocument {
    /// Header format version (51 in 2019 … 78 in version 9).
    pub format_version: u32,
    /// `DataDocument.materials`, in document order (includes stale sets no mesh uses any more).
    pub texture_sets: Vec<ProjectTextureSet>,
    /// Fields of the document object, and of objects outside any set, that the reader does not handle
    /// ([`super::fields`]). The document's own are also on every set.
    pub unhandled: Vec<UnhandledSetting>,
}

impl ProjectDocument {
    /// Builds the model from the bytes of `document.bin`. Fails only where the object serialisation
    /// itself is broken; unknown classes are kept raw and counted in the report.
    pub fn from_bytes(data: &[u8]) -> Result<(ProjectDocument, ModelReport), DecodeError> {
        let tree = RawTree::parse(data)?;
        Ok(ModelBuilder::build(&tree))
    }

    /// The model of an already parsed tree.
    pub fn from_tree(tree: &RawTree) -> (ProjectDocument, ModelReport) {
        ModelBuilder::build(tree)
    }

    /// The set with this `sceneMaterialUID`.
    pub fn texture_set(&self, material_uid: u64) -> Option<&ProjectTextureSet> {
        self.texture_sets.iter().find(|s| s.material_uid == Some(material_uid))
    }
}
