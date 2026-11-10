//! [`ProjectLayer`]: one entry of a layer stack (`DataStackLayers.items`): a layer (`DataLayerColor`) or a
//! folder (`DataLayerGroup`). Stacks are stored **bottom → top** (Verified: a full, unmasked uniform fill as
//! the last item is the final colour of a real project's sets; layer numbers grow along the list).

use super::blending::ChannelBlending;
use super::effect::ProjectEffect;
use super::fields::UnhandledSetting;
use super::geometry_mask::ProjectGeometryMask;
use super::mask::ProjectMask;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectLayer {
    pub uid: Option<u64>,
    /// `label`, the name shown in the reference app.
    pub name: String,
    pub kind: LayerKind,
    /// `enabled`: the eye toggle.
    pub visible: bool,
    /// `perChannelBlending`: blend mode and opacity per channel, as stored. The reference app has no separate
    /// layer opacity; the opacity slider writes these entries.
    pub blending: Vec<ChannelBlending>,
    /// `maskActions`; `None` if the layer has no mask.
    pub mask: Option<ProjectMask>,
    /// The geometry mask; `None` when the layer has no type set (`geometryMaskType` 0) or the fields are missing.
    pub geometry: Option<ProjectGeometryMask>,
    /// `actions`: the layer's content as an effect stack, bottom → top (empty for most folders).
    pub content: Vec<ProjectEffect>,
    /// Fields of the layer object the reader does not handle ([`super::fields`]), with their values.
    pub unhandled: Vec<UnhandledSetting>,
}

/// What kind of layer this is, as the reference app's UI shows it.
#[derive(Debug, Clone, PartialEq)]
pub enum LayerKind {
    /// A `DataLayerColor` whose content starts with a fill effect.
    Fill,
    /// A `DataLayerColor` whose content starts with an effect group (the reference app's container for strokes),
    /// or with nothing.
    Paint,
    /// A `DataLayerGroup` (folder) with its own stack, bottom → top.
    Group(Vec<ProjectLayer>),
    /// A layer class the model does not know (e.g. one from a newer version), by class name.
    Other(String),
}

impl LayerKind {
    pub fn name(&self) -> &str {
        match self {
            LayerKind::Fill => "fill",
            LayerKind::Paint => "paint",
            LayerKind::Group(_) => "folder",
            LayerKind::Other(class) => class,
        }
    }
}

impl ProjectLayer {
    /// Child layers of a folder (empty otherwise).
    pub fn children(&self) -> &[ProjectLayer] {
        match &self.kind {
            LayerKind::Group(children) => children,
            _ => &[],
        }
    }

    /// This layer and every layer below it in folders, depth first, top-level order kept.
    pub fn walk(&self) -> Vec<&ProjectLayer> {
        let mut out = Vec::new();
        let mut todo = vec![self];
        while let Some(layer) = todo.pop() {
            out.push(layer);
            todo.extend(layer.children().iter().rev());
        }
        out
    }
}
