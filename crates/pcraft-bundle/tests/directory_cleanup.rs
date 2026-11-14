//! Publication succeeds independently of best-effort cleanup of the previous entry set.
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use craft_pcraft_bundle::{DirectoryBundle, DirectoryBundleWriter, Limits, ObjectKind, ZipBundleWriter};
use craft_pcraft_format::{COMPOSITE, FULL_COMPOSITE, MANIFEST, THUMB};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

const OLD: &[u8] = br#"{"revision":1}"#;
const NEW: &[u8] = br#"{"revision":2}"#;

struct Temporary(PathBuf);

impl Temporary {
    fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let path = std::env::temp_dir().join(format!("craft-bundle-cleanup-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)));
        fs::create_dir(&path).unwrap();
        Self(path)
    }

    fn bundle(&self) -> PathBuf {
        self.0.join("document.pcraft")
    }
}

impl Drop for Temporary {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn entries(manifest: &[u8]) -> ZipBundleWriter {
    let mut bundle = ZipBundleWriter::default();
    bundle.set_manifest(manifest.to_vec()).unwrap();
    bundle
}

fn assert_manifest(root: &Path, expected: &[u8]) {
    assert_eq!(fs::read(root.join(MANIFEST)).unwrap(), expected);
    assert_eq!(DirectoryBundle::open(root, Limits::default()).unwrap().manifest().unwrap(), expected);
}

#[test]
fn non_file_stale_object_does_not_fail_the_committed_save() {
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    let stuck = root.join("tiles").join(format!("{}.zst", "0".repeat(64)));
    fs::create_dir_all(stuck.join("inner")).unwrap();
    fs::write(stuck.join("inner/keep"), b"not an object file").unwrap();

    let stats = writer.save_directory(&root, entries(NEW)).expect("cleanup cannot fail after manifest publication");
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 0);
    assert_eq!(fs::read(stuck.join("inner/keep")).unwrap(), b"not an object file");
}

#[test]
fn cleanup_removes_only_canonical_stale_regular_objects_and_reuses_current_objects() {
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    let mut first = entries(OLD);
    let retained = first.add_object(ObjectKind::Tile, b"still referenced").unwrap();
    let stale = first.add_object(ObjectKind::Blob, b"old blob").unwrap();
    for preview in [THUMB, COMPOSITE, FULL_COMPOSITE] {
        first.set_preview(preview, b"old preview".to_vec()).unwrap();
    }
    writer.save_directory(&root, first).unwrap();
    let retained_bytes = fs::read(root.join(&retained)).unwrap();
    let unrelated = [
        "tiles/personal-notes.zst".to_string(),
        "blobs/012345.zst".to_string(),
        format!("blobs/{}.zst", "A".repeat(64)),
        format!("tiles/{}.zst.backup", "b".repeat(64)),
    ];
    for path in &unrelated {
        fs::write(root.join(path), b"unrelated").unwrap();
    }
    let nested = root.join("blobs/not-an-object-folder");
    fs::create_dir(&nested).unwrap();
    fs::write(nested.join(format!("{}.zst", "c".repeat(64))), b"unrelated nested file").unwrap();
    let mut second = entries(NEW);
    assert_eq!(second.add_object(ObjectKind::Tile, b"still referenced").unwrap(), retained);
    let stats = writer.save_directory(&root, second).unwrap();

    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 1);
    assert_eq!(stats.reused, 1);
    assert_eq!(stats.reused_objects.as_slice(), std::slice::from_ref(&retained));
    assert!(stats.written_objects.is_empty());
    assert!(!root.join(stale).exists());
    assert_eq!(fs::read(root.join(retained)).unwrap(), retained_bytes);
    for path in unrelated {
        assert_eq!(fs::read(root.join(path)).unwrap(), b"unrelated");
    }
    assert!(nested.is_dir());
    for preview in [THUMB, COMPOSITE, FULL_COMPOSITE] {
        assert!(!root.join(preview).exists());
    }
}

#[test]
fn stale_preview_directories_are_preserved_after_commit() {
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    for preview in [THUMB, COMPOSITE, FULL_COMPOSITE] {
        fs::create_dir_all(root.join(preview)).unwrap();
        fs::write(root.join(preview).join("keep"), b"directory contents").unwrap();
    }
    let stats = writer.save_directory(&root, entries(NEW)).unwrap();
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 0);
    for preview in [THUMB, COMPOSITE, FULL_COMPOSITE] {
        assert_eq!(fs::read(root.join(preview).join("keep")).unwrap(), b"directory contents");
    }
}

#[test]
fn required_non_file_objects_and_previews_fail_before_manifest_publication() {
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    let mut changed = entries(NEW);
    let object = changed.add_object(ObjectKind::Tile, b"required object").unwrap();
    fs::create_dir_all(root.join(&object)).unwrap();
    assert!(writer.save_directory(&root, changed).is_err());
    assert_manifest(&root, OLD);

    for preview in [THUMB, COMPOSITE] {
        fs::create_dir_all(root.join(preview)).unwrap();
        let mut changed = entries(NEW);
        changed.set_preview(preview, b"required preview".to_vec()).unwrap();
        assert!(writer.save_directory(&root, changed).is_err());
        assert_manifest(&root, OLD);
    }
}

#[cfg(windows)]
#[test]
fn locked_stale_object_and_preview_do_not_fail_commit_and_are_collected_on_retry() {
    use std::fs::OpenOptions;
    use std::os::windows::fs::OpenOptionsExt;
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    let mut first = entries(OLD);
    let stale = first.add_object(ObjectKind::Tile, b"locked stale tile").unwrap();
    first.set_preview(FULL_COMPOSITE, b"locked preview".to_vec()).unwrap();
    writer.save_directory(&root, first).unwrap();
    let tile_lock = OpenOptions::new().read(true).share_mode(0).open(root.join(&stale)).unwrap();
    let preview_lock = OpenOptions::new().read(true).share_mode(0).open(root.join(FULL_COMPOSITE)).unwrap();
    let stats = writer.save_directory(&root, entries(NEW)).unwrap();
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 0);
    assert!(root.join(&stale).exists());
    assert!(root.join(FULL_COMPOSITE).exists());
    drop(tile_lock);
    drop(preview_lock);
    let retry = writer.save_directory(&root, entries(NEW)).unwrap();
    assert_manifest(&root, NEW);
    assert_eq!(retry.removed, 1);
    assert!(!root.join(stale).exists());
    assert!(!root.join(FULL_COMPOSITE).exists());
}

#[cfg(unix)]
#[test]
fn cleanup_preserves_symlinks_and_does_not_traverse_linked_directories() {
    use std::os::unix::fs::symlink;
    let temp = Temporary::new();
    let root = temp.bundle();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let object_name = format!("{}.zst", "d".repeat(64));
    fs::write(outside.join(&object_name), b"outside object").unwrap();
    fs::write(outside.join("preview.png"), b"outside preview").unwrap();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    symlink(&outside, root.join("blobs")).unwrap();
    symlink(&outside, root.join("composite")).unwrap();
    symlink(outside.join("preview.png"), root.join(THUMB)).unwrap();
    fs::create_dir(root.join("tiles")).unwrap();
    let linked_object = root.join("tiles").join(&object_name);
    symlink(outside.join(&object_name), &linked_object).unwrap();
    let stats = writer.save_directory(&root, entries(NEW)).unwrap();
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 0);
    assert_eq!(fs::read(outside.join(object_name)).unwrap(), b"outside object");
    assert_eq!(fs::read(outside.join("preview.png")).unwrap(), b"outside preview");
    for link in [root.join("blobs"), root.join("composite"), root.join(THUMB), linked_object] {
        assert!(fs::symlink_metadata(link).unwrap().file_type().is_symlink());
    }
}

#[cfg(unix)]
#[test]
fn required_symlink_objects_and_previews_still_fail_before_commit() {
    use std::os::unix::fs::symlink;
    let temp = Temporary::new();
    let root = temp.bundle();
    let outside = temp.0.join("outside-file");
    fs::write(&outside, b"unchanged outside bytes").unwrap();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    let mut changed = entries(NEW);
    let object = changed.add_object(ObjectKind::Tile, b"required object").unwrap();
    fs::create_dir(root.join("tiles")).unwrap();
    symlink(&outside, root.join(object)).unwrap();
    assert!(writer.save_directory(&root, changed).is_err());
    assert_manifest(&root, OLD);
    symlink(&outside, root.join(THUMB)).unwrap();
    let mut changed = entries(NEW);
    changed.set_preview(THUMB, b"new preview".to_vec()).unwrap();
    assert!(writer.save_directory(&root, changed).is_err());
    assert_manifest(&root, OLD);
    assert_eq!(fs::read(outside).unwrap(), b"unchanged outside bytes");
}

#[cfg(windows)]
#[test]
fn cleanup_does_not_traverse_windows_directory_junctions() {
    use std::os::windows::process::CommandExt;
    struct Junction(PathBuf);
    impl Drop for Junction {
        fn drop(&mut self) {
            let _ = fs::remove_dir(&self.0);
        }
    }
    fn junction(link: PathBuf, target: &Path) -> Junction {
        // Fixture-owned absolute paths only; junctions require no symlink privilege.
        assert!(link.is_absolute() && target.is_absolute());
        let output = std::process::Command::new("cmd")
            .args(["/C", "mklink", "/J"])
            .arg(&link)
            .arg(target)
            .creation_flags(0x0800_0000) // CREATE_NO_WINDOW
            .output()
            .unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        Junction(link)
    }
    let temp = Temporary::new();
    let root = temp.bundle();
    let outside = temp.0.join("outside");
    fs::create_dir(&outside).unwrap();
    let object_name = format!("{}.zst", "e".repeat(64));
    fs::write(outside.join(&object_name), b"outside object").unwrap();
    fs::write(outside.join("preview.png"), b"outside preview").unwrap();
    fs::write(outside.join("full.png"), b"outside full preview").unwrap();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    let _objects = junction(root.join("blobs"), &outside);
    let _previews = junction(root.join("composite"), &outside);
    let stats = writer.save_directory(&root, entries(NEW)).unwrap();
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 0);
    assert_eq!(fs::read(outside.join(object_name)).unwrap(), b"outside object");
    assert_eq!(fs::read(outside.join("preview.png")).unwrap(), b"outside preview");
    assert_eq!(fs::read(outside.join("full.png")).unwrap(), b"outside full preview");
    assert!(root.join("blobs").exists());
    assert!(root.join("composite").exists());
}
