//! [`ProjectPreset`]: a smart material (`.spsm`) or smart mask (`.spmsk`). Both are HDF5 like a project, with the
//! object serialisation of `document.bin` in `/preset.bin`: a smart material's root is `DataLayersPreset` holding
//! one folder (`layerGroup`, a `DataLayerGroup` as in a project's stack), a smart mask's root `DataSmartMask`
//! holding a mask's effect stack (`actionsStack`). The same builder reads them, so their layers and effects are a
//! project's.

use super::builder::ModelBuilder;
use super::effect::ProjectEffect;
use super::layer::ProjectLayer;
use super::raw::RawTree;
use super::report::ModelReport;
use crate::decode::DecodeError;

/// Where a preset's serialised objects are.
pub const PRESET_DATASET: &str = "/preset.bin";

#[allow(clippy::large_enum_variant, reason = "a plain data model matched by value across the importer; boxing would only move the bytes")]
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectPreset {
    /// A smart material: its folder of layers.
    SmartMaterial(ProjectLayer),
    /// A smart mask: the effects it puts in a mask, bottom → top.
    SmartMask(Vec<ProjectEffect>),
    /// A root class this reader doesn't know.
    Other(String),
}

impl ProjectPreset {
    /// The preset in the bytes of `/preset.bin`; fails only where the serialisation itself is broken.
    pub fn from_bytes(data: &[u8]) -> Result<(ProjectPreset, ModelReport), DecodeError> {
        let tree = RawTree::parse(data)?;
        Ok(ModelBuilder::build_preset(&tree))
    }
}
