//! The release a person chose to skip at the startup prompt, per app.
//!
//! "Skip this version" on the startup prompt writes the release's tag here, and startup offers
//! nothing more for that release: the next one is offered as usual, and Help ▸ Check for Updates
//! still shows it. Each app has a file of its own, `ArtCraft/updates/<app id>.skipped` in the
//! per-user data folder, so apps skipping at the same time never write over each other. The files
//! are apart from the launcher's version choices ([`crate::pins`]), which apps only read.
//!
//! Invariants:
//! - A missing or unreadable file skips nothing.
//! - Skipping another release replaces the one skipped before.

use std::path::{Path, PathBuf};

/// `ArtCraft/updates/<app_id>.skipped` in the per-user data folder, if there is one.
pub fn default_path(app_id: &str) -> Option<PathBuf> {
    crate::folders::data_local_dir().map(|dir| dir.join("ArtCraft").join("updates").join(format!("{app_id}.skipped")))
}

/// Whether `tag` is the release skipped in the file at `path`.
pub fn is_skipped(path: &Path, tag: &str) -> bool {
    std::fs::read_to_string(path).is_ok_and(|skipped| skipped.trim() == tag)
}

/// Remembers `tag` as the release skipped, in place of any earlier one.
pub fn skip(path: &Path, tag: &str) -> Result<(), String> {
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    // Written beside and renamed over it, so a crash halfway leaves the old file, not half a new one.
    let partial = path.with_extension("partial");
    std::fs::write(&partial, tag).map_err(|e| format!("{}: {e}", partial.display()))?;
    std::fs::rename(&partial, path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_skipped_release_is_remembered_and_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("updates").join("photocraft.skipped");
        assert!(!is_skipped(&path, "v1.2.0"), "nothing is skipped without a file");
        skip(&path, "v1.2.0").unwrap();
        assert!(is_skipped(&path, "v1.2.0"));
        assert!(!is_skipped(&path, "v1.3.0"), "the next release is offered");
        skip(&path, "v1.3.0").unwrap();
        assert!(!is_skipped(&path, "v1.2.0"), "one release at a time");
        assert!(is_skipped(&path, "v1.3.0"));
    }

    #[test]
    fn each_app_has_its_own_file() {
        let (a, b) = (default_path("photocraft"), default_path("pdfcraft"));
        assert_ne!(a, b);
        assert!(a.is_none_or(|p| p.ends_with("ArtCraft/updates/photocraft.skipped")));
    }

    #[test]
    fn a_file_that_cant_be_written_says_so() {
        let dir = tempfile::tempdir().unwrap();
        // A folder where the file should be.
        let path = dir.path().join("photocraft.skipped");
        std::fs::create_dir_all(&path).unwrap();
        assert!(skip(&path, "v1.2.0").is_err());
    }
}
