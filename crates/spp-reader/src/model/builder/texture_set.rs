//! `DataMaterial` → [`ProjectTextureSet`]: identity, resolution, declared channels and the first stack.

use super::ModelBuilder;
use crate::model::channel::ProjectChannel;
use crate::model::raw::RawObject;
use crate::model::texture_set::{ChannelDecl, MeshMapRef, ProjectTextureSet};

impl ModelBuilder {
    pub(super) fn texture_set(&mut self, material: &RawObject) -> ProjectTextureSet {
        self.begin(material);
        let mut set = self.texture_set_fields(material);
        set.unhandled = self.end(material);
        set
    }

    fn texture_set_fields(&mut self, material: &RawObject) -> ProjectTextureSet {
        self.seen(material);
        let mut stacks = material.objects("stacks");
        let first = stacks.next();
        self.report.extra_stacks += stacks.count();
        let mut channels = Vec::new();
        let mut layers = Vec::new();
        let mut shader_model = None;
        if let Some(stack) = first {
            self.seen(stack);
            shader_model = stack.u64("shaderModel");
            channels = stack.objects("channels").map(|c| self.channel(c)).collect();
            if let Some(items) = stack.object("stack") {
                layers = self.layer_stack(items, 0);
            }
        }
        ProjectTextureSet {
            uid: material.uid(),
            material_uid: material.u64("sceneMaterialUID"),
            name: material.string("sceneMaterialName").unwrap_or_default().to_string(),
            resolution_log2: material.ivec2("resolutionLog2"),
            channels,
            shader_model,
            layers,
            mesh_maps: mesh_maps(material),
            unhandled: Vec::new(),
        }
    }

    fn channel(&mut self, channel: &RawObject) -> ChannelDecl {
        self.seen(channel);
        ChannelDecl {
            channel: ProjectChannel::from_bit(self.channel_bit(channel, "type").unwrap_or(0)),
            format: channel.u64("format").unwrap_or(0),
            user_name: channel.string("userName").unwrap_or_default().to_string(),
        }
    }
}

/// The set's mesh map slots, in the file's order: `additionalMaps`, then `normalMapBitmap`.
fn mesh_maps(material: &RawObject) -> Vec<MeshMapRef> {
    let extra = material.objects("additionalMaps").filter_map(|m| Some(MeshMapRef { usage: m.u64("usage"), url: m.string("urlToBitmapRes")?.to_string() }));
    let normal = material.object("normalMapBitmap").and_then(|n| Some(MeshMapRef { usage: None, url: n.string("urlToBitmapRes")?.to_string() }));
    extra.chain(normal).collect()
}
