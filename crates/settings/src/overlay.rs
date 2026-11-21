//! A read-only settings overlay for declarative configuration.
//!
//! Applications load their normal writable preferences and then merge
//! `settings.override.json` over them. The file is written by a package manager or by hand and is
//! never written by the application, so declarative values survive a normal save.
//!
//! The overlay is plain JSON. Objects merge recursively; arrays and scalars replace the base
//! value. `CRAFT_SETTINGS_FILE` names another file, for tests or unusual layouts.

use serde_json::Value;
use std::path::{Path, PathBuf};

/// File name beside the application's preferences.
pub const OVERLAY_FILE: &str = "settings.override.json";
/// Environment variable naming an explicit overlay file.
pub const OVERLAY_ENV: &str = "CRAFT_SETTINGS_FILE";
/// Larger files are not settings.
pub const MAX_BYTES: u64 = 4 << 20;

/// The overlay path for a preferences directory.
pub fn overlay_path(config_dir: &Path) -> PathBuf {
    config_dir.join(OVERLAY_FILE)
}

/// Read the overlay for `config_dir`, or from `CRAFT_SETTINGS_FILE` when it names a readable file.
///
/// A missing file, an unreadable file, an oversized file or invalid JSON returns `None`. The
/// caller keeps its normal preferences.
pub fn load_overlay(config_dir: &Path) -> Option<Value> {
    if let Some(path) = env_path()
        && let Some(value) = load_overlay_from_path(&path)
    {
        return Some(value);
    }
    load_overlay_from_path(&overlay_path(config_dir))
}

/// Read one overlay file.
pub fn load_overlay_from_path(path: &Path) -> Option<Value> {
    let metadata = std::fs::metadata(path).ok()?;
    if !metadata.is_file() {
        return None;
    }
    if metadata.len() > MAX_BYTES {
        log::warn!("settings overlay {}: larger than {MAX_BYTES} bytes", path.display());
        return None;
    }
    let text = std::fs::read_to_string(path).ok()?;
    match serde_json::from_str(&text) {
        Ok(value) => Some(value),
        Err(error) => {
            log::warn!("settings overlay {}: {error}", path.display());
            None
        }
    }
}

/// Merge `overlay` over `base`.
///
/// Objects merge by key, recursively. Any other value replaces the base value. `null` replaces a
/// value with `null`; it does not delete a key.
pub fn merge(base: &mut Value, overlay: Value) {
    match (base, overlay) {
        (Value::Object(base), Value::Object(overlay)) => {
            for (key, value) in overlay {
                match base.entry(key) {
                    serde_json::map::Entry::Occupied(mut entry) => merge(entry.get_mut(), value),
                    serde_json::map::Entry::Vacant(entry) => {
                        entry.insert(value);
                    }
                }
            }
        }
        (base, overlay) => *base = overlay,
    }
}

fn env_path() -> Option<PathBuf> {
    std::env::var_os(OVERLAY_ENV).filter(|value| !value.is_empty()).map(PathBuf::from)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::fs;

    #[test]
    fn objects_merge_recursively() {
        let mut base = json!({
            "appearance": { "theme": "light", "scale": 1.0 },
            "recent": ["a"]
        });
        merge(&mut base, json!({ "appearance": { "theme": "dark" } }));
        assert_eq!(
            base,
            json!({
                "appearance": { "theme": "dark", "scale": 1.0 },
                "recent": ["a"]
            })
        );
    }

    #[test]
    fn arrays_and_scalars_replace() {
        let mut base = json!({ "recent": ["a", "b"], "scale": 1.0, "enabled": true });
        merge(&mut base, json!({ "recent": ["c"], "scale": 2, "enabled": false }));
        assert_eq!(base, json!({ "recent": ["c"], "scale": 2, "enabled": false }));
    }

    #[test]
    fn null_replaces_and_does_not_delete() {
        let mut base = json!({ "value": 1 });
        merge(&mut base, json!({ "value": null }));
        assert_eq!(base, json!({ "value": null }));
    }

    #[test]
    fn overlay_file_is_read() {
        let dir = tempfile::tempdir().unwrap();
        let path = overlay_path(dir.path());
        fs::write(&path, r#"{"appearance":{"theme":"dark"}}"#).unwrap();
        assert_eq!(load_overlay_from_path(&path), Some(json!({ "appearance": { "theme": "dark" } })));
    }

    #[test]
    fn missing_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(load_overlay_from_path(&overlay_path(dir.path())), None);
    }

    #[test]
    fn invalid_json_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = overlay_path(dir.path());
        fs::write(&path, "{ not json").unwrap();
        assert_eq!(load_overlay_from_path(&path), None);
    }

    #[test]
    fn oversized_file_is_ignored() {
        let dir = tempfile::tempdir().unwrap();
        let path = overlay_path(dir.path());
        let text = format!("{{\"value\":\"{}\"}}", "x".repeat(MAX_BYTES as usize));
        fs::write(&path, text).unwrap();
        assert_eq!(load_overlay_from_path(&path), None);
    }
}
