use crate::{ExternalVolume, StoreKey};

/// Maps logical [`StoreKey`] values to host storage paths (OPFS, FS Access). No IO.
pub trait TierCoordinator {
    /// Relative OPFS path segments joined by `/` (warm tier).
    fn warm_opfs_relative_path(&self, key: &StoreKey) -> String;

    /// Relative path under a cold [`ExternalVolume`], if this key may be stored there.
    fn cold_relative_path(&self, volume: &ExternalVolume, key: &StoreKey) -> Option<String>;
}

/// Default warm root matches EffectCraft persistent store layout.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DefaultTierCoordinator {
    pub warm_root: String,
}

impl Default for DefaultTierCoordinator {
    fn default() -> Self {
        Self { warm_root: "effectcraft-store/v1".into() }
    }
}

impl TierCoordinator for DefaultTierCoordinator {
    fn warm_opfs_relative_path(&self, key: &StoreKey) -> String {
        if key.0.is_empty() {
            return self.warm_root.clone();
        }
        format!("{}/{}", self.warm_root, key.0)
    }

    fn cold_relative_path(&self, volume: &ExternalVolume, key: &StoreKey) -> Option<String> {
        if key.0.starts_with("config/") {
            return None;
        }
        Some(format!("volumes/{}/{}", volume.handle_id, key.0))
    }
}
