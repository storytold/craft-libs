//! [`ProjectGeometryMask`]: a layer's or folder's geometry mask, which limits it to some meshes or UV tiles of the
//! texture set. The reference app stores it on `DataLayerColor` and `DataLayerGroup`: `enabledGeometryMask`,
//! `geometryMaskType`, `enabledMeshDefault` and `enabledMeshList` (mesh names as `DataByteArrayBox.value`),
//! `enabledUVTileDefault` and `enabledUVTileList`.
//!
//! Inferred from the corpus (lists of mesh names): type 1 selects by mesh. Every entry in the list
//! has the opposite state from the default, so with the default on (the only case seen so far) the list names the
//! meshes the layer is switched off on. Names of meshes that aren't in the scene also appear in the list. Type 0 is
//! no geometry mask; any other type is kept as [`GeometryMaskKind::Other`].

/// What the geometry mask selects by.
#[derive(Debug, Clone, PartialEq)]
pub enum GeometryMaskKind {
    /// `geometryMaskType` 1: by mesh name.
    Meshes,
    /// `geometryMaskType` 2 (assumed): by UV tile.
    UvTiles,
    /// Another type, by its number.
    Other(u64),
}

#[derive(Debug, Clone, PartialEq)]
pub struct ProjectGeometryMask {
    /// `enabledGeometryMask`.
    pub enabled: bool,
    pub kind: GeometryMaskKind,
    /// `enabledMeshDefault`: the state of meshes the list doesn't name.
    pub mesh_default: bool,
    /// `enabledMeshList`: the meshes whose state is the opposite of the default.
    pub mesh_list: Vec<String>,
    /// `enabledUVTileDefault`.
    pub uv_tile_default: bool,
    /// The number of entries in `enabledUVTileList` (their layout hasn't been seen yet).
    pub uv_tile_entries: usize,
}

impl ProjectGeometryMask {
    /// True if the mask limits the layer at all: it is enabled, it has a type, and its default or its list switches
    /// something off.
    pub fn restricts(&self) -> bool {
        if !self.enabled {
            return false;
        }
        match self.kind {
            GeometryMaskKind::Meshes => !self.mesh_default || !self.mesh_list.is_empty(),
            GeometryMaskKind::UvTiles => !self.uv_tile_default || self.uv_tile_entries > 0,
            GeometryMaskKind::Other(_) => true,
        }
    }

    /// Whether the mesh named `name` is included: its state is the default unless the list names it.
    pub fn includes_mesh(&self, name: &str) -> bool {
        self.mesh_default != self.mesh_list.iter().any(|m| m == name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn meshes(default: bool, list: &[&str]) -> ProjectGeometryMask {
        ProjectGeometryMask {
            enabled: true,
            kind: GeometryMaskKind::Meshes,
            mesh_default: default,
            mesh_list: list.iter().map(|s| s.to_string()).collect(),
            uv_tile_default: true,
            uv_tile_entries: 0,
        }
    }

    #[test]
    fn the_list_flips_the_default() {
        let eyes = meshes(true, &["Body_low", "Fin_low", "11"]);
        assert!(eyes.restricts());
        assert!(eyes.includes_mesh("Eyes_low"));
        assert!(!eyes.includes_mesh("Body_low"));
        let only = meshes(false, &["Eyes_low"]);
        assert!(only.includes_mesh("Eyes_low"));
        assert!(!only.includes_mesh("Body_low"));
    }

    #[test]
    fn an_empty_mesh_mask_or_type_0_does_not_restrict() {
        assert!(!meshes(true, &[]).restricts());
        let off = ProjectGeometryMask { enabled: false, ..meshes(true, &["Body_low"]) };
        assert!(!off.restricts());
    }
}
