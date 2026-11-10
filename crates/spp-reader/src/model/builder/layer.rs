//! `DataStackLayers` → [`ProjectLayer`]s: layers, folders, per-channel blending and masks.

use super::{MAX_NESTING, ModelBuilder};
use crate::model::blending::{ChannelBlending, ProjectBlendMode};
use crate::model::channel::ChannelMask;
use crate::model::effect::ProjectEffect;
use crate::model::geometry_mask::{GeometryMaskKind, ProjectGeometryMask};
use crate::model::layer::{LayerKind, ProjectLayer};
use crate::model::mask::{MaskInitial, ProjectMask};
use crate::model::raw::RawObject;

impl ModelBuilder {
    /// The items of a `DataStackLayers`, bottom → top as stored.
    pub(super) fn layer_stack(&mut self, stack: &RawObject, depth: usize) -> Vec<ProjectLayer> {
        self.seen(stack);
        stack.objects("items").map(|item| self.layer(item, depth)).collect()
    }

    pub(super) fn layer(&mut self, layer: &RawObject, depth: usize) -> ProjectLayer {
        self.begin(layer);
        let mut built = self.layer_fields(layer, depth);
        built.unhandled = self.end(layer);
        built
    }

    fn layer_fields(&mut self, layer: &RawObject, depth: usize) -> ProjectLayer {
        let kind = match layer.class.as_str() {
            "DataLayerColor" => {
                self.seen(layer);
                None
            }
            "DataLayerGroup" if depth < MAX_NESTING => {
                self.seen(layer);
                let children = match layer.object("subStack") {
                    Some(sub) => self.layer_stack(sub, depth + 1),
                    None => Vec::new(),
                };
                Some(LayerKind::Group(children))
            }
            "DataLayerGroup" => Some(LayerKind::Other(self.too_deep(layer).class)),
            other => {
                self.raw(layer);
                Some(LayerKind::Other(other.to_string()))
            }
        };
        let content = match layer.object("actions") {
            Some(actions) => self.effect_stack(actions, depth),
            None => Vec::new(),
        };
        let kind = kind.unwrap_or_else(|| match content.first() {
            Some(ProjectEffect::Fill(_)) => LayerKind::Fill,
            _ => LayerKind::Paint,
        });
        ProjectLayer {
            uid: layer.uid(),
            name: layer.string("label").unwrap_or_default().to_string(),
            kind,
            visible: layer.bool("enabled").unwrap_or(true),
            blending: self.blending(layer),
            mask: self.mask(layer, depth),
            geometry: geometry_mask(layer),
            content,
            unhandled: Vec::new(),
        }
    }

    fn mask(&mut self, layer: &RawObject, depth: usize) -> Option<ProjectMask> {
        let actions = layer.object("maskActions")?;
        Some(ProjectMask {
            enabled: layer.bool("maskEnabled").unwrap_or(true),
            initial: MaskInitial::from_raw(layer.u64("maskInitial").unwrap_or(1)),
            effects: self.effect_stack(actions, depth),
        })
    }

    /// `perChannelBlending` of a layer, folder or effect.
    pub(super) fn blending(&mut self, holder: &RawObject) -> Vec<ChannelBlending> {
        let mut out = Vec::new();
        for entry in holder.objects("perChannelBlending") {
            if entry.class != "DataBlending" {
                self.raw(entry);
                continue;
            }
            self.seen(entry);
            out.push(ChannelBlending {
                channels: ChannelMask(entry.channel_mask("channelTypes").unwrap_or(0)),
                mode: ProjectBlendMode::from_raw(entry.u64("blendingMode").unwrap_or(2)),
                opacity: entry.f32("opacity").unwrap_or(1.0),
            });
        }
        out
    }
}

/// The layer's geometry mask (see [`crate::model::ProjectGeometryMask`]); `None` without a type.
fn geometry_mask(layer: &RawObject) -> Option<ProjectGeometryMask> {
    let kind = match layer.u64("geometryMaskType")? {
        0 => return None,
        1 => GeometryMaskKind::Meshes,
        2 => GeometryMaskKind::UvTiles,
        other => GeometryMaskKind::Other(other),
    };
    Some(ProjectGeometryMask {
        enabled: layer.bool("enabledGeometryMask").unwrap_or(true),
        kind,
        mesh_default: layer.bool("enabledMeshDefault").unwrap_or(true),
        mesh_list: layer.objects("enabledMeshList").filter_map(|o| o.string("value")).map(str::to_string).collect(),
        uv_tile_default: layer.bool("enabledUVTileDefault").unwrap_or(true),
        uv_tile_entries: layer.list("enabledUVTileList").len(),
    })
}
