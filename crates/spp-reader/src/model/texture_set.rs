//! [`ProjectTextureSet`]: one texture set (`DataMaterial`) with its declared channels and layer stack.

use super::channel::ProjectChannel;
use super::fields::UnhandledSetting;
use super::layer::ProjectLayer;

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectTextureSet {
    /// The `DataMaterial`'s own uid.
    pub uid: Option<u64>,
    /// `sceneMaterialUID`: links to the mesh material and the texture-cache folder.
    pub material_uid: Option<u64>,
    /// `sceneMaterialName`: the set name the reference app shows.
    pub name: String,
    /// `resolutionLog2` (11 = 2048).
    pub resolution_log2: Option<[i32; 2]>,
    /// `stacks[0].channels`.
    pub channels: Vec<ChannelDecl>,
    /// `stacks[0].shaderModel`, stored from format 92 on: 2 in a project the reference app brought over from an older
    /// format, 4 in one it created (Measured on one of each). It decides what uncovered channels show.
    pub shader_model: Option<u64>,
    /// `stacks[0].stack.items`, **bottom → top**.
    pub layers: Vec<ProjectLayer>,
    /// The mesh maps assigned to the set (`additionalMaps`, then `normalMapBitmap`): a resource URL, or
    /// `resourcealias:<Map>` when the slot is empty.
    pub mesh_maps: Vec<MeshMapRef>,
    /// `useUVTiles`: the set is painted per UV tile (one texture per tile) rather than over UV 0..1.
    pub uses_uv_tiles: bool,
    /// The number of `uvTiles` entries (per-tile settings; their layout has not been seen yet).
    pub uv_tile_entries: usize,
    /// Fields of the set's own objects (`DataMaterial`, its stack and channel declarations) and of the document
    /// (they apply to every set) that the reader does not handle ([`super::fields`]), with their values.
    pub unhandled: Vec<UnhandledSetting>,
}

/// One mesh map slot of a set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeshMapRef {
    /// `DataAdditionalMap.usage` (0 ID, 1 ambient occlusion, 2 curvature, 3 world space normals, 4 position,
    /// 5 thickness); `None` for the baked normal map.
    pub usage: Option<u64>,
    /// `urlToBitmapRes`.
    pub url: String,
}

/// A channel the set declares (`DataChannel`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChannelDecl {
    pub channel: ProjectChannel,
    /// Storage format enum: 0 sRGB8, 1 L8, 5 L16F, 6 RGB16F (Verified for the channels in use).
    pub format: u64,
    /// Name of a user channel; empty for standard ones.
    pub user_name: String,
}

impl ProjectTextureSet {
    /// Edge length in texels, if the resolution is known and sane.
    pub fn resolution(&self) -> Option<[u32; 2]> {
        let [w, h] = self.resolution_log2?;
        let side = |l: i32| u32::try_from(l).ok().filter(|l| *l < 32).map(|l| 1u32 << l);
        Some([side(w)?, side(h)?])
    }

    /// Every layer including those inside folders, depth first.
    pub fn all_layers(&self) -> Vec<&ProjectLayer> {
        self.layers.iter().flat_map(|l| l.walk()).collect()
    }
}
