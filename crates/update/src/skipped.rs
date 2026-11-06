//! The release a person chose to skip at the startup prompt, per app.
//!
//! "Skip this version" on the startup prompt writes the release's tag here, and the check at start
//! offers nothing more for that release: the next one is offered as usual, and Help ▸ Check for
//! Updates still shows it. The file is the app's own, in the per-user data folder, apart from the
//! launcher's version choices ([`crate::pins`]), which apps only read.
//!
//! Invariants:
//! - A missing, unreadable or damaged file skips nothing.
//! - Each app has one entry at most; skipping another release replaces it, and other apps' entries
//!   are kept.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Default, Serialize, Deserialize)]
struct Skipped {
    /// App id to the tag of the release skipped.
    #[serde(default)]
    apps: BTreeMap<String, String>,
}

/// `ArtCraft/updates/skipped.json` in the per-user data folder, if there is one.
pub fn default_path() -> Option<PathBuf> {
    crate::folders::data_local_dir().map(|dir| dir.join("ArtCraft").join("updates").join("skipped.json"))
}

fn load(path: &Path) -> Skipped {
    std::fs::read(path).ok().and_then(|bytes| serde_json::from_slice(&bytes).ok()).unwrap_or_default()
}

/// Whether `tag` is the release skipped for `app_id`.
pub fn is_skipped(path: &Path, app_id: &str, tag: &str) -> bool {
    load(path).apps.get(app_id).is_some_and(|skipped| skipped == tag)
}

/// Remembers `tag` as the release skipped for `app_id`, in place of any earlier one.
pub fn skip(path: &Path, app_id: &str, tag: &str) -> Result<(), String> {
    let mut skipped = load(path);
    skipped.apps.insert(app_id.to_string(), tag.to_string());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let json = serde_json::to_vec_pretty(&skipped).map_err(|e| e.to_string())?;
    // Written beside and renamed over it, so a crash halfway leaves the old file, not half a new one.
    let partial = path.with_extension("json.partial");
    std::fs::write(&partial, json).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_release_is_remembered_per_app() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updates").join("skipped.json");
        assert!(!is_skipped(&path, "photocraft", "v1.2.0"), "nothing is skipped without a file");
        skip(&path, "photocraft", "v1.2.0").unwrap();
        skip(&path, "pdfcraft", "v0.3.0").unwrap();
        assert!(is_skipped(&path, "photocraft", "v1.2.0"));
        assert!(!is_skipped(&path, "photocraft", "v1.3.0"), "the next release is offered");
        assert!(is_skipped(&path, "pdfcraft", "v0.3.0"), "another app's entry is kept");
        skip(&path, "photocraft", "v1.3.0").unwrap();
        assert!(!is_skipped(&path, "photocraft", "v1.2.0"), "one entry per app");
        assert!(is_skipped(&path, "photocraft", "v1.3.0"));
    }

    #[test]
    fn a_damaged_file_skips_nothing_and_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("skipped.json");
        std::fs::write(&path, b"{not json").unwrap();
        assert!(!is_skipped(&path, "photocraft", "v1.2.0"));
        skip(&path, "photocraft", "v1.2.0").unwrap();
        assert!(is_skipped(&path, "photocraft", "v1.2.0"));
    }
}
