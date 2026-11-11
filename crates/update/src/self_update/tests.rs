//! Tests for launcher self-update.

use super::detect::*;
use super::helper::*;
#[cfg(unix)]
use super::relaunch::*;
use super::replace::*;
use super::*;
use craft_ui::testing::trial;

/// The helper reads back exactly what the launcher passed it, paths with spaces and
/// apostrophes included, and nothing that is incomplete or unknown.
#[trial]
fn the_update_helper_reads_back_its_arguments() {
    let exe = Path::new(r"C:\Users\O'Brien\AppData\Local\Programs\ArtCraft Launcher\artcraft-launcher.exe");
    let msi = Finish::Msi(PathBuf::from(r"C:\Users\O'Brien\AppData\Local\ArtCraft Launcher\cache\artcraft-launcher-0.7.0-windows-x64.msi"));
    let portable =
        Finish::Portable { zip: PathBuf::from(r"D:\Tools\x 1.zip"), tmp: PathBuf::from(r"D:\Tools\ArtCraftLauncherData\cache\launcher-update") };
    for finish in [msi, portable] {
        let args = helper_args(4242, exe, &finish);
        assert_eq!(args.first().map(OsString::as_os_str), Some(std::ffi::OsStr::new(FINISH_FLAG)));
        assert_eq!(parse_helper_args(&args), Some((4242, exe.to_path_buf(), finish)));
    }
    let os = |args: &[&str]| args.iter().map(OsString::from).collect::<Vec<_>>();
    for bad in [
        os(&[]),
        os(&["--hidden"]),
        os(&[FINISH_FLAG, "4242", "launcher.exe"]),
        os(&[FINISH_FLAG, "4242", "launcher.exe", "msi"]),
        os(&[FINISH_FLAG, "4242", "launcher.exe", "msi", "a.msi", "extra"]),
        os(&[FINISH_FLAG, "4242", "launcher.exe", "portable", "a.zip"]),
        os(&[FINISH_FLAG, "not a pid", "launcher.exe", "msi", "a.msi"]),
        os(&[FINISH_FLAG, "4242", "launcher.exe", "script", "Remove-Item C:\\"]),
        os(&["--finish", "4242", "launcher.exe", "msi", "a.msi"]),
    ] {
        assert_eq!(parse_helper_args(&bad), None, "{bad:?}");
    }
}

/// The helper finishes only the launcher's own update: its own installation, with the
/// download from its own cache. A signed launcher must not run or start anything else.
#[cfg(windows)]
#[trial]
fn the_update_helper_refuses_anything_but_the_launchers_own_update() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    std::fs::write(folder.join("portable.txt"), "").unwrap();
    let paths = Paths::under(dir.path());
    let cache = paths.cache.clone();
    std::fs::create_dir_all(paths.downloads()).unwrap();
    std::fs::create_dir_all(cache.join(HELPER_DIR)).unwrap();
    let me = cache.join(HELPER_DIR).join("artcraft-launcher.exe");
    std::fs::write(&me, "helper").unwrap();
    let zip = paths.downloads().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    std::fs::write(&zip, "zip").unwrap();
    let elsewhere = dir.path().join("elsewhere.zip");
    std::fs::write(&elsewhere, "zip").unwrap();
    let other = dir.path().join("other.exe");
    std::fs::write(&other, "other").unwrap();
    let other_in_folder = folder.join("other.exe");
    std::fs::write(&other_in_folder, "other").unwrap();
    let other_app = folder.join(format!("{}.exe", crate::suite::PHOTOCRAFT.id));
    std::fs::write(&other_app, "another Craft app").unwrap();
    let staging = cache.join(PORTABLE_STAGING);
    let portable = |zip: &Path, tmp: &Path| Finish::Portable { zip: zip.to_path_buf(), tmp: tmp.to_path_buf() };

    assert_eq!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &portable(&zip, &staging)), Ok(()));
    // A helper that isn't in the installation's cache.
    assert!(check_helper(&crate::suite::LAUNCHER, &other, &paths, &exe, &portable(&zip, &staging)).is_err());
    // Not the launcher's installation.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &other, &portable(&zip, &staging)).is_err());
    // A portable marker doesn't make a foreign executable the launcher.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &other_in_folder, &portable(&zip, &staging)).is_err());
    // Another Craft App's executable is not the launcher, even in the same folder.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &other_app, &portable(&zip, &staging)).is_err());
    // A download from outside the cache.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &portable(&elsewhere, &staging)).is_err());
    // Unpacking anywhere but the cache's staging folder.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &portable(&zip, &dir.path().join(PORTABLE_STAGING))).is_err());
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &portable(&zip, &paths.downloads())).is_err());
    // An MSI for a portable installation.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &Finish::Msi(zip.clone())).is_err());
    // Without portable.txt it is a standalone executable, which updates from the same zip.
    std::fs::remove_file(folder.join("portable.txt")).unwrap();
    assert_eq!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &portable(&zip, &staging)), Ok(()));
    // A foreign executable is not a standalone launcher either.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &other_in_folder, &portable(&zip, &staging)).is_err());
    // An MSI is for an MSI installation only.
    assert!(check_helper(&crate::suite::LAUNCHER, &me, &paths, &exe, &Finish::Msi(zip.clone())).is_err());
}

#[cfg(windows)]
fn system_program(name: &str) -> PathBuf {
    PathBuf::from(std::env::var_os("SystemRoot").unwrap()).join("System32").join(name)
}

/// The process id of a process that has already exited.
#[cfg(windows)]
fn exited_pid() -> u32 {
    let mut gone = std::process::Command::new("cmd").args(["/C", "exit"]).spawn().unwrap();
    gone.wait().unwrap();
    gone.id()
}

/// A missing or malformed executable must be rejected before any installed file is changed.
#[cfg(windows)]
#[trial]
fn a_portable_update_requires_a_signed_launcher_even_without_an_mz_header() {
    use std::io::Write;
    for launcher in [None, Some(b"not a Windows program".as_slice()), Some(b"".as_slice())] {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ArtCraft Launcher");
        std::fs::create_dir_all(folder.join("ArtCraftLauncherData")).unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        std::fs::write(&exe, "old launcher").unwrap();
        let settings = folder.join("ArtCraftLauncherData/settings.json");
        std::fs::write(&settings, "settings").unwrap();
        let zip = dir.path().join("update.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        writer.start_file("launcher/README.md", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"new readme").unwrap();
        if let Some(body) = launcher {
            writer.start_file("launcher/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        let tmp = dir.path().join(PORTABLE_STAGING);
        let step = Finish::Portable { zip: zip.clone(), tmp: tmp.clone() };
        let error =
            run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), exited_pid(), &exe, &step, std::time::Duration::from_secs(5)).unwrap_err();
        assert!(error.contains(if launcher.is_some() { "isn't signed" } else { "doesn't contain" }), "{error}");
        assert_eq!(std::fs::read(&exe).unwrap(), b"old launcher");
        assert_eq!(std::fs::read(&settings).unwrap(), b"settings");
        assert!(!folder.join("README.md").exists());
        assert!(!zip.exists() && !tmp.exists());
    }
}

/// A copy that fails part way through puts the previous files back and removes the new ones,
/// so the launcher that starts again is the old one, whole.
#[cfg(windows)]
#[trial]
fn a_portable_update_that_fails_part_way_puts_the_old_files_back() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("ArtCraft Launcher");
    // A folder where the zip has a file: copying it there fails after the exe was replaced.
    std::fs::create_dir_all(folder.join("NOTICE")).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    let signed_exe = std::fs::read(system_program("cmd.exe")).unwrap();
    for (name, body) in [
        ("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", signed_exe.as_slice()),
        ("artcraft-launcher-0.2.0-windows-x64-portable/README.md", b"# ArtCraft Launcher".as_slice()),
        ("artcraft-launcher-0.2.0-windows-x64-portable/NOTICE", b"notice".as_slice()),
    ] {
        writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(body).unwrap();
    }
    writer.finish().unwrap();
    let tmp = dir.path().join("cache").join(PORTABLE_STAGING);
    let step = Finish::Portable { zip: zip.clone(), tmp: tmp.clone() };
    let error =
        run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), exited_pid(), &exe, &step, std::time::Duration::from_secs(5)).unwrap_err();
    assert!(error.contains("NOTICE"), "{error}");
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0", "the previous exe is put back");
    assert!(!folder.join("README.md").exists(), "a file the update added is removed");
    assert!(!zip.exists() && !tmp.exists());
}

/// A Cargo build tree is not recognised by names: an installed copy in a folder literally
/// called `target` stays an installed copy.
#[trial]
fn an_installed_copy_in_a_folder_called_target_is_not_a_development_build() {
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("target").join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "MZ").unwrap();
    assert!(!is_development_build(&exe, false), "a folder named target is not a build tree");
}

#[cfg(unix)]
#[trial]
fn staging_never_writes_through_a_link_at_the_old_staging_name() {
    let dir = tempfile::tempdir().unwrap();
    let (new, exe, victim) = (dir.path().join("new"), dir.path().join("app"), dir.path().join("victim"));
    std::fs::write(&new, b"replacement").unwrap();
    std::fs::write(&exe, b"old").unwrap();
    std::fs::write(&victim, b"keep me").unwrap();
    // The names staging used to take: `.app.update` and `.app.<pid>.update`.
    std::os::unix::fs::symlink(&victim, dir.path().join(".app.update")).unwrap();
    std::os::unix::fs::symlink(&victim, dir.path().join(format!(".app.{}.update", std::process::id()))).unwrap();
    let staged = stage_copy(&new, &exe, 0o755).unwrap();
    assert_eq!(std::fs::read(&staged).unwrap(), b"replacement");
    assert!(std::fs::symlink_metadata(&staged).unwrap().file_type().is_file());
    assert_eq!(std::fs::read(&victim).unwrap(), b"keep me");
}

/// A minimal launcher bundle in `folder`; returns its executable.
#[cfg(target_os = "macos")]
fn launcher_bundle(folder: &Path) -> PathBuf {
    let contents = folder.join("Launcher.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    let mut info = plist::Dictionary::new();
    info.insert("CFBundleIdentifier".into(), plist::Value::String(crate::suite::LAUNCHER.bundle_id()));
    plist::Value::Dictionary(info).to_file_xml(contents.join("Info.plist")).unwrap();
    let exe = contents.join("MacOS/artcraft-launcher");
    std::fs::write(&exe, b"binary").unwrap();
    exe
}

/// Sets a folder's mode and gives it back to `0o755` when dropped, so the temporary folder
/// can be removed however the test ends.
#[cfg(any(target_os = "macos", target_os = "linux"))]
struct ModeGuard(PathBuf);

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl ModeGuard {
    fn set(path: &Path, mode: u32) -> Self {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode)).unwrap();
        Self(path.to_path_buf())
    }
}

#[cfg(any(target_os = "macos", target_os = "linux"))]
impl Drop for ModeGuard {
    fn drop(&mut self) {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&self.0, std::fs::Permissions::from_mode(0o755));
    }
}

/// Root passes the permission checks these tests rely on.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn running_as_root() -> bool {
    let dir = tempfile::tempdir().unwrap();
    let _guard = ModeGuard::set(dir.path(), 0o555);
    std::fs::write(dir.path().join("probe"), b"").is_ok()
}

#[cfg(unix)]
#[trial]
fn share_folders_are_the_ones_a_release_writes_to() {
    let app = &crate::suite::LAUNCHER;
    let dir = tempfile::tempdir().unwrap();
    let share = dir.path().join("share");
    std::fs::create_dir_all(share.join(app.id).join("models/small")).unwrap();
    std::fs::create_dir_all(share.join("icons/SomeTheme/apps")).unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(outside.join("inner")).unwrap();
    std::os::unix::fs::symlink(&outside, share.join(app.id).join("linked")).unwrap();
    let found = share_folders(app, &share);
    // `applications`, `doc`, `metainfo` and `mime` don't exist, so `share` is checked for them;
    // `hicolor` doesn't, so `icons` is. `SomeTheme` isn't a folder a release writes to. The walk
    // follows the link. `pixmaps` doesn't exist, and nothing inside `share` is checked for it.
    let mut expected = vec![
        share.clone(),
        share.join("icons"),
        share.join(app.id),
        share.join(app.id).join("linked"),
        share.join(app.id).join("linked/inner"),
        share.join(app.id).join("models"),
        share.join(app.id).join("models/small"),
    ];
    expected.sort();
    assert_eq!(found, expected);

    let apps = share.join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(apps.join(format!("{}.png", app.bundle_id())), b"icon").unwrap();
    std::fs::create_dir_all(share.join("icons/hicolor/48x48/apps")).unwrap();
    for folder in ["applications", "pixmaps", "metainfo", "mime"] {
        std::fs::create_dir_all(share.join(folder)).unwrap();
    }
    std::fs::create_dir_all(share.join("doc").join(app.id).join("html")).unwrap();
    let found = share_folders(app, &share);
    let doc = format!("doc/{}", app.id);
    let html = format!("doc/{}/html", app.id);
    for folder in ["icons/hicolor", "icons/hicolor/256x256/apps", "applications", "pixmaps", "metainfo", "mime", &doc, &html] {
        assert!(found.contains(&share.join(folder)), "{folder} in {found:?}");
    }
    // Another package's icon folder isn't one the release writes to.
    assert!(!found.contains(&share.join("icons/hicolor/48x48/apps")), "{found:?}");
    assert!(!found.contains(&share), "every folder exists, so `share` is checked for none: {found:?}");
}

/// When the app's folder is a link, the walk goes through it.
#[cfg(unix)]
#[trial]
fn share_folders_walk_a_linked_app_folder() {
    let app = &crate::suite::LAUNCHER;
    let dir = tempfile::tempdir().unwrap();
    let share = dir.path().join("share");
    let elsewhere = dir.path().join("elsewhere");
    std::fs::create_dir_all(elsewhere.join("models")).unwrap();
    std::fs::create_dir_all(&share).unwrap();
    std::os::unix::fs::symlink(&elsewhere, share.join(app.id)).unwrap();
    std::os::unix::fs::symlink(&elsewhere, elsewhere.join("models/loop")).unwrap();
    let found = share_folders(app, &share);
    assert!(found.contains(&share.join(app.id).join("models")), "{found:?}");
    // The loop is listed once and not read again.
    assert!(found.contains(&share.join(app.id).join("models/loop")), "{found:?}");
    assert!(!found.contains(&share.join(app.id).join("models/loop/models")), "{found:?}");
}

/// The walk reads at most the limit's number of entries in each of the app's folder,
/// `doc/<id>` and `icons/hicolor`.
#[trial]
fn share_folders_stop_at_the_limit() {
    let app = &crate::suite::LAUNCHER;
    let dir = tempfile::tempdir().unwrap();
    let share = dir.path().join("share");
    for name in ["a", "b", "c", "d"] {
        std::fs::create_dir_all(share.join(app.id).join(name)).unwrap();
    }
    let found = share_folders_within(app, &share, 2);
    let walked = found.iter().filter(|folder| folder.starts_with(share.join(app.id))).count();
    // The app's folder and the two entries read in it.
    assert_eq!(walked, 3, "{found:?}");
    // `share` is checked for `icons/hicolor`, `applications`, `doc/<id>`, `metainfo` and
    // `mime/packages`.
    assert_eq!(found.len(), 4, "{found:?}");
    assert_eq!(share_folders(app, &share).len(), 6, "the default limit takes in the whole tree");

    // A full app folder doesn't use up the icon folders' entries.
    let apps = share.join("icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&apps).unwrap();
    std::fs::write(apps.join(format!("{}.png", app.bundle_id())), b"icon").unwrap();
    let found = share_folders_within(app, &share, 2);
    assert!(found.contains(&apps), "{found:?}");
}

/// A release writes into the folders inside the app's folder, `doc/<id>` and `icons/hicolor`,
/// and into `metainfo`, `mime/packages` and `pixmaps`, so a copy whose such folder the user
/// can't write gets the download link.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[trial]
fn a_standalone_copy_with_a_nested_share_folder_it_cannot_write_gets_the_download_link() {
    if running_as_root() {
        return;
    }
    let app = &crate::suite::LAUNCHER;
    for nested in [
        format!("{}/models", app.id),
        format!("doc/{}/html", app.id),
        "icons/hicolor/256x256/apps".to_string(),
        "metainfo".to_string(),
        "mime/packages".to_string(),
        "pixmaps".to_string(),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let share = dir.path().join("share");
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(share.join(app.id)).unwrap();
        std::fs::create_dir_all(share.join(&nested)).unwrap();
        if nested.starts_with("icons") {
            std::fs::write(share.join(&nested).join(format!("{}.png", app.bundle_id())), b"icon").unwrap();
        }
        let exe = bin.join(app.id);
        std::fs::write(&exe, b"binary").unwrap();
        let guard = ModeGuard::set(&share.join(&nested), 0o555);
        assert_eq!(current_at(app, &exe, false), None, "{nested}");
        drop(guard);
        assert!(current_at(app, &exe, false).is_some(), "{nested}: the answer follows the folder's permissions");
    }
}

/// The update also renames over the app's other programs and its `share` files, so a sticky
/// folder holding one that another user owns refuses. The test reads the folder as another
/// user, to whom the temporary folder and its files belong to someone else.
#[cfg(any(target_os = "macos", target_os = "linux"))]
#[trial]
fn a_sticky_folder_holding_a_file_the_update_replaces_refuses() {
    use std::os::unix::fs::MetadataExt;

    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("artcraft-launcher-cli"), b"binary").unwrap();
    std::fs::write(dir.path().join("notes.txt"), b"").unwrap();
    let me = std::fs::metadata(dir.path()).unwrap().uid();
    let someone_else = me + 1;
    let cli = |path: &Path| path.ends_with("artcraft-launcher-cli");
    let notes = |path: &Path| path.ends_with("notes.txt");
    let none = |_: &Path| false;
    assert!(!sticky_folder_refuses_for(dir.path(), cli, someone_else), "no sticky bit");
    let _guard = ModeGuard::set(dir.path(), 0o1777);
    assert!(sticky_folder_refuses_for(dir.path(), cli, someone_else));
    assert!(sticky_folder_refuses_for(dir.path(), notes, someone_else));
    assert!(!sticky_folder_refuses_for(dir.path(), none, someone_else), "a file the update doesn't replace");
    assert!(!sticky_folder_refuses_for(dir.path(), cli, me), "the user's own folder");
    assert!(!sticky_folder_refuses(dir.path(), cli));
}

/// The sticky check counts only files the update renames over: the app's programs, and its
/// own files under `share`.
#[trial]
fn the_sticky_check_counts_only_the_apps_files() {
    let app = &crate::suite::LAUNCHER;
    let bin = Path::new("/p/bin");
    assert!(is_app_program(app, &bin.join(app.id)));
    assert!(is_app_program(app, &bin.join(format!("{}-cli", app.id))));
    assert!(!is_app_program(app, &bin.join(format!("{}.log", app.id))));
    assert!(!is_app_program(app, &bin.join(format!("{}-cli.log", app.id))));
    assert!(!is_app_program(app, &bin.join(format!("{}x", app.id))));
    assert!(!is_app_program(app, &bin.join("other")));

    let share = Path::new("/p/share");
    assert!(is_share_file(app, share, &share.join(app.id).join("models/weights.bin")));
    assert!(is_share_file(app, share, &share.join("doc").join(app.id).join("README.md")));
    assert!(is_share_file(app, share, &share.join("applications").join(format!("{}.desktop", app.bundle_id()))));
    assert!(is_share_file(app, share, &share.join("pixmaps").join(format!("{}.png", app.id))));
    assert!(!is_share_file(app, share, &share.join("applications/other.desktop")));
    assert!(!is_share_file(app, share, &share.join("pixmaps").join(format!("{}-old.png", app.id))));
}

/// A copy opened from a mounted disk image is on a read-only volume.
#[cfg(target_os = "macos")]
#[trial]
fn a_copy_on_a_read_only_image_gets_the_download_link() {
    struct Detach(PathBuf);
    impl Drop for Detach {
        fn drop(&mut self) {
            let _ = std::process::Command::new("hdiutil").arg("detach").arg("-force").arg(&self.0).output();
        }
    }
    /// Tries a few times, because the disk-image helper turns requests away while it is busy,
    /// and removes what a failed `create` left at `partial` first. Returns what it last said.
    fn hdiutil(args: &[&std::ffi::OsStr], partial: Option<&Path>) -> Result<(), String> {
        let mut said = String::new();
        for _ in 0..5 {
            if let Some(partial) = partial {
                let _ = std::fs::remove_file(partial);
            }
            let output = std::process::Command::new("hdiutil").args(args).output().unwrap();
            if output.status.success() {
                return Ok(());
            }
            said = String::from_utf8_lossy(&output.stderr).trim().to_string();
        }
        Err(said)
    }
    let dir = tempfile::tempdir().unwrap();
    let (src, image, mount) = (dir.path().join("src"), dir.path().join("test.dmg"), dir.path().join("mount"));
    launcher_bundle(&src);
    std::fs::create_dir(&mount).unwrap();
    let _one_at_a_time = crate::install::macos::DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let text = std::ffi::OsStr::new;
    let created = hdiutil(
        &[text("create"), text("-quiet"), text("-format"), text("UDZO"), text("-srcfolder"), src.as_os_str(), image.as_os_str()],
        Some(&image),
    );
    let attached = created.and_then(|()| {
        let attach = [
            text("attach"),
            text("-nobrowse"),
            text("-readonly"),
            text("-noautoopen"),
            text("-noverify"),
            text("-mountpoint"),
            mount.as_os_str(),
            image.as_os_str(),
        ];
        hdiutil(&attach, None)
    });
    if let Err(said) = attached {
        eprintln!("skipped: the disk-image helper stayed busy: {said}");
        return;
    }
    let _detach = Detach(mount.clone());
    let exe = mount.join("Launcher.app/Contents/MacOS/artcraft-launcher");
    assert!(exe.is_file(), "the image holds the bundle");
    assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
}

/// A copy under an App Translocation folder gets the download link even where the folder
/// allows the update.
#[cfg(target_os = "macos")]
#[trial]
fn a_translocated_copy_gets_the_download_link() {
    let output = std::process::Command::new("/usr/bin/getconf").arg("DARWIN_USER_TEMP_DIR").output().unwrap();
    let system_temp = PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()).canonicalize().unwrap();
    let dir = tempfile::tempdir_in(system_temp).unwrap();
    let exe = launcher_bundle(&dir.path().join("AppTranslocation/0A1B/d"));
    assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
}

/// A locked bundle answers with a different refusal from a bundle without permission, and
/// can't be moved either.
#[cfg(target_os = "macos")]
#[trial]
fn a_locked_bundle_gets_the_download_link() {
    struct Unlock(PathBuf);
    impl Drop for Unlock {
        fn drop(&mut self) {
            let _ = std::process::Command::new("/usr/bin/chflags").arg("nouchg").arg(&self.0).status();
        }
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = launcher_bundle(dir.path());
    let bundle = dir.path().join("Launcher.app");
    assert!(std::process::Command::new("/usr/bin/chflags").arg("uchg").arg(&bundle).status().unwrap().success());
    let _unlock = Unlock(bundle);
    assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
}

/// Adds a deny entry to `path`'s access control list; the list is cleared when dropped, so
/// the temporary folder can be removed however the test ends.
#[cfg(target_os = "macos")]
struct DenyGuard(PathBuf);

#[cfg(target_os = "macos")]
impl DenyGuard {
    fn add(path: &Path, right: &str) -> Self {
        let status = std::process::Command::new("/bin/chmod").arg("+a").arg(format!("everyone deny {right}")).arg(path).status().unwrap();
        assert!(status.success(), "chmod +a {right}");
        Self(path.to_path_buf())
    }
}

#[cfg(target_os = "macos")]
impl Drop for DenyGuard {
    fn drop(&mut self) {
        let _ = std::process::Command::new("/bin/chmod").arg("-N").arg(&self.0).status();
    }
}

/// The update adds a subdirectory to the folder and moves the bundle, so a list that denies
/// only adding files keeps it, and one that denies either step does not.
#[cfg(target_os = "macos")]
#[trial]
fn access_control_lists_are_judged_by_the_rights_the_update_uses() {
    if running_as_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let folder = dir.path().join("Applications");
    let exe = launcher_bundle(&folder);
    let bundle = folder.join("Launcher.app");
    let found = || current_at(&crate::suite::LAUNCHER, &exe, false);
    for (path, right, keeps_the_update) in [
        (&folder, "add_file", true),
        (&bundle, "add_file", true),
        (&folder, "add_subdirectory", false),
        (&bundle, "add_subdirectory", false),
        (&bundle, "delete", false),
        (&folder, "delete_child", false),
    ] {
        let _guard = DenyGuard::add(path, right);
        assert_eq!(found().is_some(), keeps_the_update, "{right} denied on {}", path.display());
    }
    assert!(found().is_some(), "the answer follows the list");
}

/// A standalone copy in a folder the user can't write gets the download link; [`DEV_VAR`]
/// still offers the update, to try it.
#[cfg(target_os = "linux")]
#[trial]
fn a_standalone_linux_copy_it_cannot_replace_gets_the_download_link() {
    if running_as_root() {
        return;
    }
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("artcraft-launcher");
    std::fs::write(&exe, b"binary").unwrap();
    let _guard = ModeGuard::set(dir.path(), 0o555);
    assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
    assert!(current_at(&crate::suite::LAUNCHER, &exe, true).is_some(), "the opt-in still offers it");
}

/// The outcome is read once; a failure wins over a finished update, which counts only for the
/// version it installed.
#[trial]
fn the_outcome_of_an_update_is_read_once() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    assert_eq!(take_outcome(&paths, "0.2.0"), None);
    record_done(&paths, "v0.2.0");
    assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Installed(Version::new(0, 2, 0))));
    assert_eq!(take_outcome(&paths, "0.2.0"), None, "read once");

    record_done(&paths, "v0.2.0");
    assert_eq!(take_outcome(&paths, "0.1.0"), None, "the old version still runs");
    assert!(!done_file(&paths).exists(), "a record that doesn't match is removed too");

    record_done(&paths, "v0.2.0");
    record_failure(&paths, "the Windows Installer stopped with error 1603");
    assert!(!done_file(&paths).exists(), "a failure removes the record of a finished update");
    record_done(&paths, "v0.2.0");
    assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Failed("the Windows Installer stopped with error 1603".into())));
    assert_eq!(take_outcome(&paths, "0.2.0"), None);

    // Damaged records: no version, an empty reason, a reason longer than the limit.
    crate::paths::write_atomic(&done_file(&paths), b"\xff\xfe not a version").unwrap();
    assert_eq!(take_outcome(&paths, "0.2.0"), None);
    record_failure(&paths, "  \n");
    assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Failed("no reason was recorded".into())));
    record_failure(&paths, &"é".repeat(RECORD_LIMIT));
    let Some(Outcome::Failed(reason)) = take_outcome(&paths, "0.2.0") else { panic!("a long reason is still a failure") };
    assert!(reason.len() <= RECORD_LIMIT + '…'.len_utf8() && reason.ends_with("é…"), "{}", reason.len());
    // A folder where a record goes is not read, and not removed.
    std::fs::create_dir_all(failure_file(&paths)).unwrap();
    std::fs::create_dir_all(done_file(&paths)).unwrap();
    assert_eq!(take_outcome(&paths, "0.2.0"), None);
    assert!(failure_file(&paths).is_dir());
}

/// A failure recorded before this program was put in place is removed without being
/// reported; one recorded after it, like a failed helper's, is reported.
#[trial]
fn a_failure_older_than_the_running_program_is_not_reported() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let program = dir.path().join("artcraft-launcher");
    std::fs::write(&program, "program").unwrap();
    let hour = std::time::Duration::from_secs(3600);
    let set_program_time = |at| std::fs::OpenOptions::new().write(true).open(&program).unwrap().set_modified(at).unwrap();

    record_failure(&paths, "the Windows Installer stopped with error 1603");
    let recorded = std::fs::metadata(failure_file(&paths)).unwrap().modified().unwrap();
    set_program_time(recorded - hour);
    assert_eq!(take_outcome_of(&paths, "0.2.0", Some(&program)), Some(Outcome::Failed("the Windows Installer stopped with error 1603".into())));

    record_failure(&paths, "this copy isn't signed");
    let recorded = std::fs::metadata(failure_file(&paths)).unwrap().modified().unwrap();
    set_program_time(recorded + hour);
    assert_eq!(take_outcome_of(&paths, "0.2.0", Some(&program)), None);
    assert!(!failure_file(&paths).exists(), "the old failure is removed");
}

/// A link where a record goes is not followed: neither record is read through it, and the
/// file it points to stays. Skipped where the system refuses to create the link (Windows
/// without the privilege for symbolic links).
#[trial]
fn a_record_that_is_a_link_is_not_read() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    std::fs::create_dir_all(paths.logs()).unwrap();
    std::fs::create_dir_all(&paths.data).unwrap();
    let elsewhere = dir.path().join("elsewhere.txt");
    std::fs::write(&elsewhere, "v0.2.0").unwrap();
    #[cfg(unix)]
    let link = |to: &Path, at: &Path| std::os::unix::fs::symlink(to, at);
    #[cfg(windows)]
    let link = |to: &Path, at: &Path| std::os::windows::fs::symlink_file(to, at);
    if let Err(e) = link(&elsewhere, &failure_file(&paths)).and_then(|()| link(&elsewhere, &done_file(&paths))) {
        eprintln!("skipped: couldn't create a symbolic link ({e})");
        return;
    }
    assert_eq!(take_failure(&paths), None);
    assert_eq!(take_outcome(&paths, "0.2.0"), None);
    assert_eq!(std::fs::read_to_string(&elsewhere).unwrap(), "v0.2.0");
}

/// A release whose tag is not its version (an app may build its own `Release`) is recorded
/// by its version, so the new version's first start still reports the update.
#[trial]
fn a_finished_update_is_recorded_by_its_version_not_its_tag() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let release = Release { tag: "stable".into(), version: version::parse("0.2.0"), ..Release::default() };
    mark_installed(&paths, &release);
    assert!(just_updated_to(&paths, &release), "the tag still guards against offering it again");
    assert_eq!(scheduled_version(&paths).as_deref(), Some("0.2.0"));
    record_done(&paths, &scheduled_version(&paths).unwrap());
    assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Installed(Version::new(0, 2, 0))));
    // A record without the version line, from before it was written, names the tag.
    crate::paths::write_atomic(&marker(&paths), format!("v0.3.0\n{}\n", crate::now()).as_bytes()).unwrap();
    assert_eq!(scheduled_version(&paths).as_deref(), Some("v0.3.0"));
}

/// The cache may not exist yet at the first update: the check runs before the download
/// creates it.
#[cfg(windows)]
#[trial]
fn the_cache_check_passes_before_the_cache_exists() {
    let dir = tempfile::tempdir().unwrap();
    let exe = dir.path().join("artcraft-launcher.exe");
    std::fs::write(&exe, "MZ").unwrap();
    std::fs::write(dir.path().join("portable.txt"), "").unwrap();
    let paths = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
    assert!(!paths.cache.exists());
    assert_eq!(same_cache_as_helper(&crate::suite::LAUNCHER, &paths, &exe), Ok(()));
}

/// The restart helper starts only when the restart is started, at the app's exit: a close the
/// user cancels leaves no helper counting down its one-minute wait. The test starts the helper
/// with its standard streams set to null, as `spawn_detached` does, and a thread that waits
/// for the helper to exit writes `exited`, so a helper that exits without writing `started`
/// fails the test.
#[cfg(unix)]
#[trial]
fn the_restart_helper_starts_only_when_the_restart_is_started() {
    use std::io::{BufRead, Write};
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    let dir = tempfile::tempdir().unwrap();
    let events = dir.path().join("events");
    assert!(std::process::Command::new("mkfifo").arg(&events).status().unwrap().success());
    // Open for reading and writing, so neither this open nor the helper's blocks.
    let pipe = std::fs::OpenOptions::new().read(true).write(true).open(&events).unwrap();
    let mut exited = pipe.try_clone().unwrap();
    let mut lines = std::io::BufReader::new(pipe).lines();
    let mut gone = std::process::Command::new("true").spawn().unwrap();
    gone.wait().unwrap();
    let started = r#"printf 'started\n' > "$1""#;
    let command = [std::ffi::OsStr::new("/bin/sh"), "-c".as_ref(), started.as_ref(), "sh".as_ref(), events.as_os_str()];
    let spawned = Arc::new(AtomicBool::new(false));
    let flag = Arc::clone(&spawned);
    let spawn = move |mut cmd: std::process::Command| -> Result<(), String> {
        flag.store(true, Ordering::SeqCst);
        let mut helper = cmd
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| e.to_string())?;
        std::thread::spawn(move || {
            let _ = helper.wait();
            let _ = exited.write_all(b"exited\n");
        });
        Ok(())
    };
    let restart = relaunch_with(&crate::suite::LAUNCHER, gone.id(), &command, spawn);
    assert!(!spawned.load(Ordering::SeqCst), "the helper must wait for the restart to be started");
    assert_eq!(format!("{restart:?}"), "Restart { pending: true }", "the helper must wait for the restart to be started");
    assert_eq!(restart.start(), Ok(()));
    assert_eq!(lines.next().unwrap().unwrap(), "started", "the helper exited without starting the command");
    assert_eq!(lines.next().unwrap().unwrap(), "exited");
}

/// A disk image holding an unsigned `ArtCraft Launcher.app` of `version`, which a self-update
/// downloads and then refuses at the signature check.
#[cfg(target_os = "macos")]
fn unsigned_launcher_dmg(dir: &Path, version: &str) -> Vec<u8> {
    let contents = dir.join(format!("src-{version}")).join("ArtCraft Launcher.app/Contents");
    std::fs::create_dir_all(contents.join("MacOS")).unwrap();
    std::fs::write(
        contents.join("Info.plist"),
        format!(
            "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
             <key>CFBundleIdentifier</key><string>ai.storyteller.artcraft-launcher</string>\
             <key>CFBundleShortVersionString</key><string>{version}</string></dict></plist>"
        ),
    )
    .unwrap();
    std::fs::write(contents.join("MacOS/ArtCraft Launcher"), version).unwrap();
    let dmg = dir.join(format!("built-{version}.dmg"));
    let one_at_a_time = install::macos::DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
    let ok = std::process::Command::new("hdiutil")
        .args(["create", "-quiet", "-format", "UDZO", "-volname", "ArtCraft Launcher", "-srcfolder"])
        .arg(dir.join(format!("src-{version}")))
        .arg(&dmg)
        .status()
        .unwrap()
        .success();
    assert!(ok, "hdiutil create failed");
    drop(one_at_a_time);
    std::fs::read(dmg).unwrap()
}

/// A launcher release of `version` whose disk image `body` is served at `url`.
#[cfg(target_os = "macos")]
fn launcher_release(version: &str, body: &[u8], url: String, dir: &Path) -> Release {
    let file = dir.join(format!("sha-{version}"));
    std::fs::write(&file, body).unwrap();
    let sha = crate::http::sha256_file(&file).unwrap();
    Release {
        version: version::parse(version),
        tag: format!("v{version}"),
        assets: vec![crate::release::Asset {
            name: format!("artcraft-launcher-{version}-macos-universal.dmg"),
            size: body.len() as u64,
            url,
            sha256: Some(sha),
        }],
        ..Release::default()
    }
}

/// A self-update that fails after its download keeps the disk image, and the next try reuses
/// it without a second transfer. A newer release replaces the kept image.
#[cfg(target_os = "macos")]
#[trial]
fn a_failed_self_update_keeps_its_download_for_the_next_try() {
    use crate::http::tests::{Reply, serve};
    use crate::{Arch, Os};
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(&dir.path().join("home"));
    let apps = dir.path().join("Applications");
    let current = apps.join("ArtCraft Launcher.app/Contents");
    std::fs::create_dir_all(current.join("MacOS")).unwrap();
    std::fs::write(
        current.join("Info.plist"),
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?><plist version=\"1.0\"><dict>\
         <key>CFBundleIdentifier</key><string>ai.storyteller.artcraft-launcher</string>\
         <key>CFBundleShortVersionString</key><string>0.1.0</string></dict></plist>",
    )
    .unwrap();
    std::fs::write(current.join("MacOS/ArtCraft Launcher"), "0.1.0").unwrap();
    let install = Install::MacApp(apps.join("ArtCraft Launcher.app"));
    let platform = Platform { os: Os::MacOs, arch: Arch::X64 };
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let stop = Stop::default();
    let mut phase = |_| ();

    let body = unsigned_launcher_dmg(dir.path(), "0.2.0");
    let (url, requests) = serve(body.clone(), vec![Reply::Full, Reply::Full]);
    let release = launcher_release("0.2.0", &body, url, dir.path());
    let kept = install::download_path(&paths, &release.assets[0]);
    for _ in 0..2 {
        let error =
            prepare_update(&crate::suite::LAUNCHER, &install, &release, platform, &http, &paths, &stop, &mut phase, &|| Ok(()), None).unwrap_err();
        // An unsigned bundle is refused as "isn't signed" or, where codesign reports it, as a
        // failed signature check; either way the install stops after the download.
        assert!(error.contains("signed") && error.ends_with("so it wasn't installed"), "{error}");
        assert_eq!(std::fs::read(&kept).unwrap(), body, "the download stays after a failed install");
    }
    assert_eq!(requests.lock().unwrap().len(), 1, "the second try reuses the kept download");

    // A newer release's download replaces the kept one, even when that update stops before
    // installing.
    let newer = b"the 0.3.0 disk image".to_vec();
    let (url, _) = serve(newer.clone(), vec![Reply::Full]);
    let release = launcher_release("0.3.0", &newer, url, dir.path());
    let error = prepare_update(
        &crate::suite::LAUNCHER,
        &install,
        &release,
        platform,
        &http,
        &paths,
        &stop,
        &mut phase,
        &|| Err("another job is running".into()),
        None,
    )
    .unwrap_err();
    assert_eq!(error, "another job is running");
    assert!(!kept.exists(), "the older kept download is removed");
    assert_eq!(std::fs::read(install::download_path(&paths, &release.assets[0])).unwrap(), newer);
    assert_eq!(std::fs::read_dir(paths.downloads()).unwrap().count(), 1);
}

/// A self-update download asset of `name` and `size`.
#[cfg(not(windows))]
fn update_asset(name: &str, size: u64) -> crate::release::Asset {
    crate::release::Asset { name: name.into(), size, url: String::new(), sha256: None }
}

/// A self-update for another release file removes the finished and the partial download of the
/// one the record names, and leaves other files alone. A download of the same file keeps its
/// partial file, so it resumes.
#[cfg(not(windows))]
#[trial]
fn a_new_release_removes_the_kept_and_partial_download() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let downloads = paths.downloads();
    std::fs::create_dir_all(&downloads).unwrap();
    let old = update_asset("artcraft-launcher-0.2.0-linux-x64.tar.gz", 100);
    let finished = downloads.join(&old.name);
    let partial = downloads.join(format!("{}.100.part", old.name));
    let other = downloads.join("photocraft-0.5.0-linux-x64.tar.gz");
    for file in [&finished, &partial, &other] {
        std::fs::write(file, b"bytes").unwrap();
    }
    assert!(keep_download(&paths, &old));
    assert!(keep_download(&paths, &old));
    assert!(finished.exists() && partial.exists(), "the same release keeps its files");

    let new = update_asset("artcraft-launcher-0.3.0-linux-x64.tar.gz", 200);
    assert!(keep_download(&paths, &new));
    assert!(!finished.exists() && !partial.exists());
    assert!(other.exists());
    assert_eq!(std::fs::read_to_string(kept_download_record(&paths)).unwrap(), format!("{}\n200\n", new.name));
}

/// The record names a file in the download folder only: a name with a path in it removes
/// nothing, and a kept file the user already deleted is no error.
#[cfg(not(windows))]
#[trial]
fn a_kept_download_record_removes_only_its_own_file() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    std::fs::create_dir_all(paths.downloads()).unwrap();
    let outside = paths.cache.join("escape.dmg");
    std::fs::write(&outside, b"not ours").unwrap();
    std::fs::write(kept_download_record(&paths), "../escape.dmg\n8\n").unwrap();
    let new = update_asset("artcraft-launcher-0.3.0-macos-universal.dmg", 10);
    assert!(keep_download(&paths, &new));
    assert!(outside.exists());

    let newer = update_asset("artcraft-launcher-0.4.0-macos-universal.dmg", 10);
    assert!(keep_download(&paths, &newer));
    assert!(std::fs::read_to_string(kept_download_record(&paths)).unwrap().starts_with(&newer.name));

    // A record without a size still removes the file it names.
    let kept = paths.downloads().join(&newer.name);
    std::fs::write(&kept, b"image").unwrap();
    std::fs::write(kept_download_record(&paths), &newer.name).unwrap();
    assert!(keep_download(&paths, &update_asset("artcraft-launcher-0.5.0-macos-universal.dmg", 10)));
    assert!(!kept.exists());
}

/// With `build.build-dir` set, the profile directory holds the program and `.cargo-lock`; the
/// lock file and the target root's `CACHEDIR.TAG` identify a development build, and neither
/// does alone.
#[trial]
fn a_cargo_build_with_a_separate_build_dir_is_detected() {
    let tree = |profile: &str, lock: bool, tag: bool| {
        let dir = tempfile::tempdir().unwrap();
        let profile = dir.path().join("target").join(profile);
        std::fs::create_dir_all(&profile).unwrap();
        let exe = profile.join("artcraft-launcher");
        std::fs::write(&exe, "ELF").unwrap();
        if lock {
            std::fs::write(profile.join(".cargo-lock"), "").unwrap();
        }
        if tag {
            std::fs::write(dir.path().join("target").join(CARGO_CACHE_MARKER), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
        }
        (dir, exe)
    };
    for profile in ["debug", "release", "x86_64-pc-windows-gnu/release"] {
        let (_dir, exe) = tree(profile, true, true);
        assert!(is_development_build(&exe, false), "{profile}");
    }
    let (_dir, exe) = tree("release", true, false);
    assert!(!is_development_build(&exe, false), "a lock file alone");
    let (_dir, exe) = tree("release", false, true);
    assert!(!is_development_build(&exe, false), "a cache marker alone");
}

#[cfg(not(windows))]
fn release_with(names: &[&str]) -> Release {
    Release {
        assets: names.iter().map(|n| crate::release::Asset { name: (*n).into(), size: 1, url: String::new(), sha256: None }).collect(),
        ..Release::default()
    }
}

/// A macOS release carries a disk image and a command-line zip, and no `.tar.gz` that a
/// standalone program could be replaced from, so that copy gets the download link. A release
/// that adds a macOS `.tar.gz` is offered to it again.
#[cfg(not(windows))]
#[trial]
fn a_standalone_macos_program_needs_a_release_with_a_tarball() {
    let app = &crate::suite::SOUNDCRAFT;
    let release = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-cli-0.19.0-macos-universal.zip"]);
    let with_tarball = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-0.19.0-macos-universal.tar.gz"]);
    let standalone = Install::Standalone(PathBuf::from("/Users/someone/bin/soundcraft"));
    for arch in [crate::Arch::Arm64, crate::Arch::X64] {
        let mac = Some(Platform { os: crate::Os::MacOs, arch });
        assert!(!installs_from(app, &standalone, &release, mac), "{arch:?}");
        assert!(installs_from(app, &standalone, &with_tarball, mac), "{arch:?}");
        // the bundle installs from the disk image
        assert!(installs_from(app, &Install::MacApp(PathBuf::from("/Applications/SoundCraft.app")), &release, mac), "{arch:?}");
    }
}

/// A helper whose wait ends with the old app still running installs nothing, starts no
/// program, records why, and removes the download.
#[cfg(windows)]
#[trial]
fn the_helper_starts_nothing_while_the_old_app_runs() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let exe = dir.path().join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    std::fs::write(&zip, "zip").unwrap();
    let step = Finish::Portable { zip: zip.clone(), tmp: dir.path().join(PORTABLE_STAGING) };
    let started = std::sync::Mutex::new(Vec::<PathBuf>::new());
    let start = |exe: &Path| {
        started.lock().unwrap().push(exe.to_path_buf());
        Ok(())
    };
    let never_exits = |_pid| false;
    let error =
        finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &never_exits, &start).unwrap_err();
    assert!(error.contains("still running"), "{error}");
    assert_eq!(*started.lock().unwrap(), Vec::<PathBuf>::new(), "no copy of the app may start while it still runs");
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
    assert!(!zip.exists(), "the download is removed");
    assert_eq!(take_failure(&paths).as_deref(), Some(error.as_str()));
}

/// A step that fails after the old app exited starts the old app again, as it was.
#[cfg(windows)]
#[trial]
fn the_helper_starts_the_old_app_after_a_failed_step() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let exe = dir.path().join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    // Not a zip, so the step fails.
    let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    std::fs::write(&zip, "not a zip").unwrap();
    let step = Finish::Portable { zip, tmp: dir.path().join(PORTABLE_STAGING) };
    let started = std::sync::Mutex::new(Vec::<PathBuf>::new());
    let start = |exe: &Path| {
        started.lock().unwrap().push(exe.to_path_buf());
        Ok(())
    };
    let exited = |_pid| true;
    assert!(finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start).is_err());
    assert_eq!(*started.lock().unwrap(), vec![exe.clone()]);
    assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
}

/// A successful step starts the new app once.
#[cfg(windows)]
#[trial]
fn the_helper_starts_the_new_app_after_the_update() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let folder = dir.path().join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    let signed_exe = std::fs::read(system_program("cmd.exe")).unwrap();
    writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(&signed_exe).unwrap();
    writer.finish().unwrap();
    let step = Finish::Portable { zip, tmp: dir.path().join(PORTABLE_STAGING) };
    let started = std::sync::Mutex::new(Vec::<(PathBuf, Vec<u8>)>::new());
    let start = |exe: &Path| {
        started.lock().unwrap().push((exe.to_path_buf(), std::fs::read(exe).unwrap()));
        Ok(())
    };
    let exited = |_pid| true;
    mark_installed(&paths, &Release { tag: "v0.2.0".into(), ..Release::default() });
    assert_eq!(finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start), Ok(()));
    assert_eq!(*started.lock().unwrap(), vec![(exe, signed_exe)], "the new app starts once");
    assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Installed(Version::new(0, 2, 0))), "the new version's first start says so");
}

/// A standalone copy updated from the portable zip becomes portable: the outcome and the
/// record of the update go to the folders beside it, which the new app reads.
#[cfg(windows)]
#[trial]
fn a_standalone_copy_that_becomes_portable_finds_the_outcome_beside_it() {
    use std::io::Write;
    let dir = tempfile::tempdir().unwrap();
    // The per-user folders the old, standalone copy used.
    let paths = Paths::under(&dir.path().join("per-user"));
    let folder = dir.path().join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(&std::fs::read(system_program("cmd.exe")).unwrap()).unwrap();
    writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/portable.txt", zip::write::SimpleFileOptions::default()).unwrap();
    writer.finish().unwrap();
    let step = Finish::Portable { zip, tmp: dir.path().join(PORTABLE_STAGING) };
    let release = Release { tag: "v0.2.0".into(), version: version::parse("0.2.0"), ..Release::default() };
    mark_installed(&paths, &release);
    let exited = |_pid| true;
    let start = |_: &Path| Ok(());
    assert_eq!(finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start), Ok(()));
    let after = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
    assert_eq!(after.data, folder.join("ArtCraftLauncherData").join("data"));
    assert!(just_updated_to(&after, &release), "the new app does not offer the same update again");
    assert_eq!(take_outcome(&after, "0.2.0"), Some(Outcome::Installed(Version::new(0, 2, 0))));
    assert_eq!(take_outcome(&paths, "0.2.0"), None, "nothing is left where the new app no longer looks");

    // A new app that cannot start is reported there too.
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(&dir.path().join("per-user"));
    let (exe, step) = portable_update(dir.path());
    if let Finish::Portable { zip, .. } = &step {
        let mut writer = zip::ZipWriter::new(std::fs::File::create(zip).unwrap());
        writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(system_program("cmd.exe")).unwrap()).unwrap();
        writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/portable.txt", zip::write::SimpleFileOptions::default()).unwrap();
        writer.finish().unwrap();
    }
    mark_installed(&paths, &release);
    let denied = |_: &Path| Err("couldn't start the launcher (denied)".to_owned());
    assert!(finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &denied).is_err());
    let after = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
    assert_eq!(take_outcome(&after, "0.2.0"), Some(Outcome::Failed("couldn't start the launcher (denied)".into())));
}

/// A portable update of the launcher to 0.2.0 in `dir`: its exe and the step.
#[cfg(windows)]
fn portable_update(dir: &Path) -> (PathBuf, Finish) {
    use std::io::Write;
    let folder = dir.join("ArtCraft Launcher");
    std::fs::create_dir_all(&folder).unwrap();
    let exe = folder.join("artcraft-launcher.exe");
    std::fs::write(&exe, "0.1.0").unwrap();
    let zip = dir.join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
    let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
    writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
    writer.write_all(&std::fs::read(system_program("cmd.exe")).unwrap()).unwrap();
    writer.finish().unwrap();
    (exe, Finish::Portable { zip, tmp: dir.join(PORTABLE_STAGING) })
}

/// Only `/usr/bin/<id>` is checked against the package databases, as before the launcher's
/// self-update moved here; another copy under a system folder (an AUR build, say) is left to
/// its package manager, and only a copy outside them (`/usr/local` included, which no package
/// manager owns) replaces itself.
#[trial]
fn only_the_packages_own_path_is_a_package_and_system_folders_are_left_alone() {
    let app = &crate::suite::LAUNCHER;
    assert_eq!(linux_location(app, Path::new("/usr/bin/artcraft-launcher")), LinuxLocation::PackagePath);
    for system in [
        "/usr/bin/artcraft-launcher-bin",
        "/nix/store/abc-artcraft-launcher/bin/artcraft-launcher",
        "/opt/artcraft-launcher/artcraft-launcher",
        "/bin/artcraft-launcher",
        "/gnu/store/abc-artcraft-launcher/bin/artcraft-launcher",
        "/snap/artcraft-launcher/12/bin/artcraft-launcher",
        "/var/lib/snapd/snap/artcraft-launcher/12/bin/artcraft-launcher",
        "/home/linuxbrew/.linuxbrew/Cellar/artcraft-launcher/1.0/bin/artcraft-launcher",
        "/home/me/.linuxbrew/bin/artcraft-launcher",
    ] {
        assert_eq!(linux_location(app, Path::new(system)), LinuxLocation::System, "{system}");
    }
    for own in [
        "/usr/local/bin/artcraft-launcher",
        "/home/me/bin/artcraft-launcher",
        "/home/me/.local/bin/artcraft-launcher",
        "/usrlocal/artcraft-launcher",
        "/var/lib/snapdx/artcraft-launcher",
    ] {
        assert_eq!(linux_location(app, Path::new(own)), LinuxLocation::Elsewhere, "{own}");
    }
}

/// storytold/craft-launcher#764: a self-update downloads only its own release's link. One into
/// another app's releases, with the right tag and name, is refused before anything is fetched.
#[trial]
fn update_refuses_a_link_that_is_not_its_releases_own() {
    let dir = tempfile::tempdir().unwrap();
    let paths = Paths::under(dir.path());
    let http = Http::new(crate::TEST_AGENT).unwrap();
    let mut phase = |_| ();
    let last = || Ok(());
    let stop = Stop::default();
    let name = "artcraft-launcher-0.3.0-linux-x86_64.AppImage";
    let release = Release {
        version: version::parse("0.3.0"),
        tag: "v0.3.0".into(),
        assets: vec![crate::release::Asset {
            name: name.into(),
            size: 1,
            url: format!("https://github.com/storytold/photocraft/releases/download/v0.3.0/{name}"),
            sha256: None,
        }],
        ..Release::default()
    };
    let install = Install::AppImage(dir.path().join("launcher.AppImage"));
    use crate::{Arch, Os};
    let platform = Platform { os: Os::Linux, arch: Arch::X64 };
    let result = update(&crate::suite::LAUNCHER, &install, &release, platform, &http, &paths, &stop, &mut phase, &last, Some("0.2.0"));
    assert!(result.unwrap_err().contains("isn't the one its release publishes"));
}

/// [`release_tarball`], with entries that `more` appends after the files.
#[cfg(unix)]
fn release_tarball_with(path: &Path, files: &[(&str, &[u8])], more: impl FnOnce(&mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>)) {
    let f = std::fs::File::create(path).unwrap();
    let mut b = tar::Builder::new(flate2::write::GzEncoder::new(f, flate2::Compression::fast()));
    for (name, bytes) in files {
        let mut h = tar::Header::new_gnu();
        h.set_size(bytes.len() as u64);
        h.set_mode(0o755);
        h.set_cksum();
        b.append_data(&mut h, format!("pdfcraft-9.9.9-linux-x86_64/{name}"), *bytes).unwrap();
    }
    more(&mut b);
    b.into_inner().unwrap().finish().unwrap();
}

/// Appends a link at `name` in the release, pointing at `target`.
#[cfg(unix)]
fn append_release_link(b: &mut tar::Builder<flate2::write::GzEncoder<std::fs::File>>, name: &str, target: &Path) {
    let mut h = tar::Header::new_gnu();
    h.set_entry_type(tar::EntryType::Symlink);
    h.set_size(0);
    h.set_cksum();
    b.append_link(&mut h, format!("pdfcraft-9.9.9-linux-x86_64/{name}"), target).unwrap();
}

#[cfg(unix)]
#[trial]
fn an_unreadable_folder_in_the_release_stops_the_update_before_anything_changes() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempfile::tempdir().unwrap();
    let probe = dir.path().join("probe");
    std::fs::create_dir(&probe).unwrap();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o300)).unwrap();
    let readable = std::fs::read_dir(&probe).is_ok();
    std::fs::set_permissions(&probe, std::fs::Permissions::from_mode(0o755)).unwrap();
    if readable {
        return; // running as root: permissions don't stop reads, so there's nothing to test
    }
    let prefix = dir.path().join("local");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::create_dir_all(prefix.join("share/pdfcraft/models")).unwrap();
    let exe = prefix.join("bin/pdfcraft");
    std::fs::write(&exe, b"old app").unwrap();
    std::fs::write(prefix.join("share/pdfcraft/a.txt"), b"old data").unwrap();
    std::fs::write(prefix.join("share/pdfcraft/models/a.rten"), b"old model").unwrap();
    let tarball = dir.path().join("new.tar.gz");
    let files: [(&str, &[u8]); 3] =
        [("bin/pdfcraft", b"new app"), ("share/pdfcraft/a.txt", b"new data"), ("share/pdfcraft/models/a.rten", b"new model")];
    release_tarball_with(&tarball, &files, |b| {
        // The archive keeps this folder's mode, which lets nobody list it.
        let mut h = tar::Header::new_gnu();
        h.set_entry_type(tar::EntryType::Directory);
        h.set_size(0);
        h.set_mode(0o300);
        h.set_cksum();
        b.append_data(&mut h, "pdfcraft-9.9.9-linux-x86_64/share/pdfcraft/models/", &[][..]).unwrap();
    });
    let paths = Paths::under(&dir.path().join("data"));
    let error = replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap_err();
    assert!(
        error.starts_with("couldn't read ") && error.contains("pdfcraft-9.9.9-linux-x86_64/share/pdfcraft/models ("),
        "the error names the folder: {error}"
    );
    let staged = paths.cache.join(format!("pdfcraft.standalone.{}", std::process::id()));
    assert!(!staged.exists(), "the unpacked release is removed");
    let read = |p: &str| std::fs::read(prefix.join(p)).unwrap();
    assert_eq!(read("bin/pdfcraft"), b"old app");
    assert_eq!(read("share/pdfcraft/a.txt"), b"old data");
    assert_eq!(read("share/pdfcraft/models/a.rten"), b"old model");
    let leftovers: Vec<_> = walk(&prefix).into_iter().filter(|p| p.to_string_lossy().contains(".update")).collect();
    assert!(leftovers.is_empty(), "no staged files left behind: {leftovers:?}");
}

#[cfg(unix)]
#[trial]
fn a_release_without_shared_files_still_updates_a_prefix() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("local");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::create_dir_all(prefix.join("share/pdfcraft")).unwrap();
    let exe = prefix.join("bin/pdfcraft");
    std::fs::write(&exe, b"old app").unwrap();
    std::fs::write(prefix.join("share/pdfcraft/a.txt"), b"old data").unwrap();
    let tarball = dir.path().join("new.tar.gz");
    release_tarball(&tarball, &[("bin/pdfcraft", b"new app")]);
    let paths = Paths::under(&dir.path().join("data"));
    replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap();
    assert_eq!(std::fs::read(&exe).unwrap(), b"new app");
    assert_eq!(std::fs::read(prefix.join("share/pdfcraft/a.txt")).unwrap(), b"old data");
}

#[cfg(unix)]
#[trial]
fn a_release_with_a_link_for_its_shared_files_stops_the_update() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("local");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    std::fs::create_dir_all(prefix.join("share/pdfcraft")).unwrap();
    let exe = prefix.join("bin/pdfcraft");
    std::fs::write(&exe, b"old app").unwrap();
    let outside = dir.path().join("outside/pdfcraft");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("a.txt"), b"not from the release").unwrap();
    let tarball = dir.path().join("new.tar.gz");
    let outside = dir.path().join("outside");
    release_tarball_with(&tarball, &[("bin/pdfcraft", b"new app")], |b| append_release_link(b, "share", &outside));
    let paths = Paths::under(&dir.path().join("data"));
    let error = replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap_err();
    assert!(error.ends_with("pdfcraft-9.9.9-linux-x86_64/share isn't a folder"), "{error}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old app");
    assert!(!prefix.join("share/pdfcraft/a.txt").exists(), "nothing is copied in through the link");
}

#[cfg(unix)]
#[trial]
fn a_release_with_a_link_for_its_programs_stops_the_update() {
    let dir = tempfile::tempdir().unwrap();
    let prefix = dir.path().join("local");
    std::fs::create_dir_all(prefix.join("bin")).unwrap();
    let exe = prefix.join("bin/pdfcraft");
    std::fs::write(&exe, b"old app").unwrap();
    std::fs::write(prefix.join("bin/pdfcraft-cli"), b"old cli").unwrap();
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("pdfcraft"), b"not from the release").unwrap();
    std::fs::write(outside.join("pdfcraft-cli"), b"not from the release").unwrap();
    let tarball = dir.path().join("new.tar.gz");
    release_tarball_with(&tarball, &[("share/pdfcraft/a.txt", b"new data")], |b| append_release_link(b, "bin", &outside));
    let paths = Paths::under(&dir.path().join("data"));
    let error = replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap_err();
    assert!(error.ends_with("pdfcraft-9.9.9-linux-x86_64/bin isn't a folder"), "{error}");
    assert_eq!(std::fs::read(&exe).unwrap(), b"old app");
    assert_eq!(std::fs::read(prefix.join("bin/pdfcraft-cli")).unwrap(), b"old cli");
}

/// A release tarball like `package.sh` makes: everything under one `<app>-<ver>-<platform>/`.
#[cfg(unix)]
fn release_tarball(path: &Path, files: &[(&str, &[u8])]) {
    release_tarball_with(path, files, |_| {});
}

#[cfg(unix)]
fn walk(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(dir).unwrap().flatten() {
        let p = e.path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            out.push(p);
        }
    }
    out
}

#[cfg(target_os = "linux")]
#[trial]
fn a_custom_flatpak_identity_is_used_for_the_restart() {
    let app = App { bundle_id_override: Some("ai.artcraft.app"), ..App::new("artcraft", "ArtCraft", "Create", "storytold/artcraft") };
    let cmd = flatpak_restart_command(&app, "exec \"$@\"", "123");
    let args: Vec<_> = cmd.get_args().map(|v| v.to_string_lossy().into_owned()).collect();
    assert_eq!(args.last().map(String::as_str), Some("ai.artcraft.app"));
    assert!(!args.iter().any(|arg| arg == "ai.storyteller.artcraft"));
}
