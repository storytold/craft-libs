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
    /// The legacy, unnamespaced EffectCraft layout, predating per-app OPFS namespacing. Kept
    /// as-is so existing EffectCraft data stays reachable; a coordinator for any other app, or
    /// for EffectCraft under the current namespaced layout, must come from
    /// [`for_app`](Self::for_app).
    fn default() -> Self {
        Self { warm_root: "effectcraft-store/v1".into() }
    }
}

impl DefaultTierCoordinator {
    /// A coordinator whose warm root matches `assets/opfs-flush.js`'s actual namespaced layout
    /// for `app_id`: `{sanitized_app_id}/store/v1`. CodeRabbit finding on coordinator.rs:20:
    /// [`default`](Default::default) hard-codes `effectcraft-store/v1` regardless of app id,
    /// which disagrees with the flush hook's per-app path for every app including EffectCraft
    /// under the namespaced layout. Use `sanitize_app_segment` here, the same byte-wise encoding
    /// `opfs-flush.js`'s `sanitizeAppSegment` uses, so the two sides always agree on the path for
    /// a given app id.
    pub fn for_app(app_id: &str) -> Self {
        Self { warm_root: format!("{}/store/v1", sanitize_app_segment(app_id)) }
    }
}

/// A single OPFS path segment may not be empty, `.`, `..`, or contain `/` (Chrome's
/// `getDirectoryHandle` throws "Name is not allowed" otherwise). Every byte outside
/// `[A-Za-z0-9_]` is hex-escaped as `~xx`, which makes the encoding injective: two different app
/// ids can never collide on the same segment (the lossy `replace(/[^A-Za-z0-9_-]+/g, '-')` this
/// replaces let `collab.photo` and `collab-photo` both become `collab-photo`). `~` escaping
/// itself guarantees the output never equals `.` or `..` and never contains `/`, so no separate
/// fallback case is needed for those; only a genuinely empty `app_id` falls back.
///
/// Must byte-for-byte match `sanitizeAppSegment` in `assets/opfs-flush.js`: the two run in
/// different processes (this crate natively, that one in the browser worker) and must still
/// compute the identical warm-tier path for the same app id.
pub fn sanitize_app_segment(app_id: &str) -> String {
    const FALLBACK: &str = "craft-app";
    if app_id.is_empty() {
        return FALLBACK.into();
    }
    let mut out = String::with_capacity(app_id.len());
    for byte in app_id.as_bytes() {
        if byte.is_ascii_alphanumeric() || *byte == b'_' {
            out.push(*byte as char);
        } else {
            out.push('~');
            out.push_str(&format!("{byte:02x}"));
        }
    }
    out
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
