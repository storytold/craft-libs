#![cfg(feature = "lenient")]
// From Craft Anything crates/formats/tests/zip_entry_detection.rs.
use craft_zip::lenient::has_entry as zip_has_entry;
#[path = "lenient_common/mod.rs"]
mod common;
use common::zip_stored;

/// A directory-shaped payload is ordinary file content, not an archive member.
#[test]
fn embedded_archive_does_not_supply_outer_entries() {
    let inner = zip_stored(&[("AndroidManifest.xml", b"manifest")]);
    let outer = zip_stored(&[("backup.zip", &inner)]);
    assert!(!zip_has_entry(&outer, "AndroidManifest.xml"));
    assert!(zip_has_entry(&outer, "backup.zip"));
}

/// A ZIP comment can contain another directory without changing the archive's entries.
#[test]
fn comment_does_not_supply_entries() {
    let mut outer = zip_stored(&[("readme.txt", b"notes")]);
    let inner = zip_stored(&[("AndroidManifest.xml", b"manifest")]);
    let len = outer.len();
    outer[len - 2..].copy_from_slice(&(inner.len() as u16).to_le_bytes());
    outer.extend(inner);
    assert!(!zip_has_entry(&outer, "AndroidManifest.xml"));
    assert!(zip_has_entry(&outer, "readme.txt"));
}

/// Actual manifests are found regardless of member order, including prefixed archives.
#[test]
fn actual_entries_are_found() {
    let data = zip_stored(&[("classes.dex", b"dex"), ("AndroidManifest.xml", b"manifest")]);
    assert!(zip_has_entry(&data, "AndroidManifest.xml"));
    let mut prefixed = b"stub".to_vec();
    prefixed.extend(data);
    assert!(zip_has_entry(&prefixed, "AndroidManifest.xml"));
    assert!(!zip_has_entry(&prefixed, "missing"));
}

/// A stray central-directory signature without an archive is not an entry.
#[test]
fn incomplete_archive_is_not_a_manifest() {
    let data = zip_stored(&[("AndroidManifest.xml", b"manifest")]);
    assert!(!zip_has_entry(&data[..data.len() - 22], "AndroidManifest.xml"));
}
