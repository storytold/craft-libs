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

#[cfg(any(unix, windows))]
#[test]
fn manifest_symlink_is_refused_before_writing_any_objects_or_previews() {
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    writer.save_directory(&root, entries(OLD)).unwrap();
    assert_manifest(&root, OLD); // Ordinary publication remains supported.
    let external = temp.0.join("external-manifest.json");
    fs::write(&external, b"unchanged external manifest").unwrap();
    fs::remove_file(root.join(MANIFEST)).unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&external, root.join(MANIFEST)).unwrap();
    #[cfg(windows)]
    if let Err(error) = std::os::windows::fs::symlink_file(&external, root.join(MANIFEST)) {
        if error.raw_os_error() == Some(1314) || error.kind() == std::io::ErrorKind::Unsupported {
            eprintln!("SKIP manifest symlink fixture: {error}");
            return;
        }
        panic!("create manifest symlink fixture: {error}");
    }
    let mut changed = entries(NEW);
    changed.add_object(ObjectKind::Tile, b"must not write tile").unwrap();
    changed.add_object(ObjectKind::Blob, b"must not write blob").unwrap();
    changed.set_preview(THUMB, b"must not write thumbnail".to_vec()).unwrap();
    changed.set_preview(FULL_COMPOSITE, b"must not write composite".to_vec()).unwrap();
    assert!(writer.save_directory(&root, changed).is_err());
    assert_eq!(fs::read(&external).unwrap(), b"unchanged external manifest");
    assert!(fs::symlink_metadata(root.join(MANIFEST)).unwrap().file_type().is_symlink());
    for unwritten in ["tiles", "blobs", THUMB, "composite"] {
        assert!(!root.join(unwritten).exists(), "manifest refusal must precede {unwritten} publication");
    }
}

#[cfg(unix)]
#[test]
fn chosen_root_symlink_cleanup_works_with_search_only_root_and_composite_permissions() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    struct Restore(PathBuf, fs::Permissions);
    impl Restore {
        fn restrict(path: &Path) -> Self {
            let old = fs::metadata(path).unwrap().permissions();
            fs::set_permissions(path, fs::Permissions::from_mode(0o300)).unwrap();
            Self(path.to_path_buf(), old)
        }
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = fs::set_permissions(&self.0, self.1.clone());
        }
    }
    let temp = Temporary::new();
    let root = temp.bundle();
    let mut writer = DirectoryBundleWriter::default();
    let mut first = entries(OLD);
    let stale = first.add_object(ObjectKind::Blob, b"stale blob").unwrap();
    first.set_preview(THUMB, b"stale thumbnail".to_vec()).unwrap();
    first.set_preview(COMPOSITE, b"stale preview".to_vec()).unwrap();
    writer.save_directory(&root, first).unwrap();
    let chosen = temp.0.join("chosen.pcraft");
    symlink(&root, &chosen).unwrap();
    let composite_mode = Restore::restrict(&root.join("composite"));
    let root_mode = Restore::restrict(&root);
    let result = writer.save_directory(&chosen, entries(NEW));
    drop(root_mode);
    drop(composite_mode);
    let stats = result.expect("chosen root resolves before cleanup; traversal does not require directory read permission");
    assert_manifest(&root, NEW);
    assert_eq!(stats.removed, 1);
    assert!(!root.join(stale).exists());
    assert!(!root.join(THUMB).exists());
    assert!(!root.join(COMPOSITE).exists());
    assert!(fs::symlink_metadata(chosen).unwrap().file_type().is_symlink());
}
