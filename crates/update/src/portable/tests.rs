//! Tests for the portable install.

use super::*;
use craft_ui::testing::trial;
use std::io::Write;

/// Validation of the installed copy is part of the transaction, while a backup still exists.
#[trial]
fn a_self_update_that_fails_validation_puts_the_previous_files_back() {
    let dir = tempfile::tempdir().unwrap();
    let staging = dir.path().join("launcher-update");
    let folder = dir.path().join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "old launcher").unwrap();
    let zip = archive(dir.path(), "update.zip", &[("launcher/artcraft-launcher.exe", b"new launcher"), ("launcher/README.md", b"new readme")]);
    let checked = std::cell::Cell::new(false);
    let error = unpack_over(&zip, &staging, &folder, std::ffi::OsStr::new("artcraft-launcher.exe"), &|file| {
        if file == exe {
            assert_eq!(std::fs::read(file).unwrap(), b"new launcher");
            assert!(folder.join("README.md").is_file());
            checked.set(true);
            Err("installed launcher failed validation".into())
        } else {
            Ok(())
        }
    })
    .unwrap_err();
    assert_eq!(error, "installed launcher failed validation");
    assert!(checked.get());
    assert_eq!(std::fs::read(&exe).unwrap(), b"old launcher");
    assert!(!folder.join("README.md").exists());
    assert!(!staging.exists());
}

fn archive(root: &Path, name: &str, files: &[(&str, &[u8])]) -> PathBuf {
    let path = root.join(name);
    let mut zip = zip::ZipWriter::new(std::fs::File::create(&path).unwrap());
    for (name, content) in files {
        if name.ends_with('/') {
            zip.add_directory(*name, zip::write::SimpleFileOptions::default()).unwrap();
        } else {
            zip.start_file(*name, zip::write::SimpleFileOptions::default()).unwrap();
            zip.write_all(content).unwrap();
        }
    }
    zip.finish().unwrap();
    path
}

#[trial]
fn stale_junction_record_cannot_remove_files_after_retargeting() {
    use std::process::Command;

    fn junction(link: &Path, target: &Path) {
        let status = Command::new("cmd.exe").args(["/C", "mklink", "/J"]).arg(link).arg(target).status().unwrap();
        assert!(status.success());
    }

    let dir = tempfile::tempdir().unwrap();
    let first = dir.path().join("first");
    let second = dir.path().join("second");
    let link = dir.path().join("Apps junction");
    std::fs::create_dir(&first).unwrap();
    std::fs::create_dir(&second).unwrap();
    junction(&link, &first);
    let alias_root = link.join("photocraft");
    std::fs::create_dir(&alias_root).unwrap();
    std::fs::write(alias_root.join("photocraft.exe"), b"old program").unwrap();
    let record = Record {
        version: "0.5.0".into(),
        kind: PackageKind::PortableZip,
        path: alias_root.join("photocraft.exe"),
        installed_at: 0,
        files: vec![alias_root.join("photocraft.exe")],
        needs_root: false,
    };
    std::process::Command::new("cmd.exe").args(["/C", "rmdir"]).arg(&link).status().unwrap();
    junction(&link, &second);
    let foreign_root = link.join("photocraft");
    std::fs::create_dir(&foreign_root).unwrap();
    std::fs::write(foreign_root.join("photocraft.exe"), b"foreign program").unwrap();
    assert!(remove(App::by_id("photocraft").unwrap(), &record).is_err());
    assert_eq!(std::fs::read(foreign_root.join("photocraft.exe")).unwrap(), b"foreign program");
}

/// ZIP 9 strips a drive prefix from enclosed paths; the original name must still be rejected.
#[trial]
fn rejects_drive_prefixed_names_before_writing_files() {
    let dir = tempfile::tempdir().unwrap();
    let zip = archive(dir.path(), "drive.zip", &[("c:escape.exe", b"bad".as_slice())]);
    let output = dir.path().join("output");
    std::fs::create_dir(&output).unwrap();
    assert!(extract(&zip, &output).is_err());
    assert_eq!(std::fs::read_dir(&output).unwrap().count(), 0);
}

#[trial]
fn rejects_traversal_and_case_collisions() {
    let dir = tempfile::tempdir().unwrap();
    for (name, files) in [
        ("traversal.zip", vec![("../escape.exe", b"bad".as_slice())]),
        ("colon.zip", vec![("data:stream", b"bad".as_slice())]),
        ("duplicate.zip", vec![("PhotoCraft.exe", b"one".as_slice()), ("photocraft.exe", b"two".as_slice())]),
    ] {
        let zip = archive(dir.path(), name, &files);
        let output = dir.path().join(name.replace('.', "-"));
        std::fs::create_dir(&output).unwrap();
        assert!(extract(&zip, &output).is_err(), "{name}");
    }
    assert!(!dir.path().join("escape.exe").exists());
    assert!(validate_directory(Path::new("relative/folder")).is_err());
    assert!(validate_directory(&dir.path().join("../escape")).is_err());
}
