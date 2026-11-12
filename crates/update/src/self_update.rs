//! ArtCraft Launcher updating itself.
//!
//! The new version is downloaded and verified like any Craft App, then swapped in the way this
//! copy was installed. A small helper process waits for this launcher to exit, finishes anything
//! that can't happen while it runs (Windows locks a running exe), and starts the new one. The
//! launcher has to exit first anyway: it holds the single-instance lock, and a new copy started
//! while it runs would only ask it to show its window. A macOS bundle, an AppImage or a Linux
//! standalone copy that can't be replaced gets the download link.
//!
//! | Installed as | Swap |
//! |---|---|
//! | macOS `.app` | copy the new bundle from the DMG over this one (fine while running), reopen it |
//! | Windows MSI (any folder, for one account or everyone) | after exit, run the new MSI in the copy's own context (everyone asks for permission), then start the exe it registers |
//! | Windows portable zip | after exit, unzip over this folder (settings in `ArtCraftLauncherData` stay) |
//! | Linux AppImage | rename the new AppImage over this one (fine while running), start it |
//! | Linux `.deb` / `.rpm` | the system package manager, with the password prompt; then restart |
//! | Linux Flatpak | reinstall the bundle on the same installation (per-user, or system-wide through `pkexec`); then restart |
//!
//! Anything else (FreeBSD, a development build, an unsigned Windows copy) gets the download link
//! instead.

use std::ffi::OsString;
use std::path::{Path, PathBuf};

use crate::PackageKind;
use crate::app::App;
use crate::platform::{Platform, pick_asset};
use crate::protocol::JobPhase;
use crate::release::Release;
use crate::version::{self, Version};
#[cfg(target_os = "macos")]
use crate::{InstallSource, Installed};

use crate::http::{Http, Stop};
use crate::install::{self, Job};
use crate::paths::Paths;
use crate::process;

/// How this running launcher was installed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Install {
    /// The `ArtCraft Launcher.app` bundle.
    MacApp(PathBuf),
    /// The per-user Windows Installer package; the exe.
    Msi(PathBuf),
    /// The Windows portable zip; the exe (with `portable.txt` beside it).
    Portable(PathBuf),
    /// The AppImage file.
    AppImage(PathBuf),
    /// A Debian or RPM package; the binary.
    Package(PackageKind, PathBuf),
    /// A Flatpak application.
    Flatpak,
    /// A standalone executable or unpacked tree.
    Standalone(PathBuf),
}

impl Install {
    fn kind(&self) -> PackageKind {
        match self {
            Self::MacApp(_) => PackageKind::Dmg,
            Self::Msi(_) => PackageKind::Msi,
            Self::Portable(_) => PackageKind::PortableZip,
            Self::AppImage(_) => PackageKind::AppImage,
            Self::Package(kind, _) => *kind,
            Self::Flatpak => PackageKind::Flatpak,
            Self::Standalone(_) => {
                #[cfg(windows)]
                {
                    PackageKind::PortableZip
                }
                #[cfg(not(windows))]
                {
                    PackageKind::TarGz
                }
            }
        }
    }
}

/// Makes an installed copy believe it's this version, to try the self-update against the current
/// release. The restart helper drops it, so it lasts one run.
pub const PRETEND_VAR: &str = "ARTCRAFT_LAUNCHER_PRETEND_VERSION";

/// Makes a development build believe it's an installed copy, to try the self-update.
pub const DEV_VAR: &str = "CRAFT_UPDATE_DEV";

/// The version this app counts as (see [`PRETEND_VAR`]).
pub fn current_version(app: &App, fallback_pkg_version: &str) -> Option<Version> {
    let app_pretend = pretend_var(app);
    std::env::var(&app_pretend)
        .or_else(|_| std::env::var(PRETEND_VAR))
        .ok()
        .and_then(|v| version::parse(&v))
        .or_else(|| version::parse(fallback_pkg_version))
}

fn pretend_var(app: &App) -> String {
    format!("{}_PRETEND_VERSION", app.id.replace('-', "_").to_uppercase())
}

/// A pretend version lasts one run, including when an update schedules a restart.
fn clear_pretend_version(cmd: &mut std::process::Command, app: &App) {
    cmd.env_remove(PRETEND_VAR).env_remove(pretend_var(app));
}

/// How long a just-installed version stays exempt from being offered again.
const JUST_UPDATED_FOR: i64 = 24 * 3600;

/// Whether `release` is the version this app just updated itself to. Offering it again would
/// loop: update, restart, "update available", … That happens if a release's binary reports a
/// different version from its tag, or while a pretend version is set.
pub fn just_updated_to(paths: &Paths, release: &Release) -> bool {
    let Ok(text) = std::fs::read_to_string(marker(paths)) else { return false };
    let mut lines = text.lines();
    let (Some(tag), Some(at)) = (lines.next(), lines.next().and_then(|a| a.parse::<i64>().ok())) else { return false };
    tag == release.tag && crate::now().checked_sub(at).is_some_and(|age| (0..JUST_UPDATED_FOR).contains(&age))
}

fn marker(paths: &Paths) -> PathBuf {
    paths.data.join("self-update.txt")
}

/// How this copy is installed, if it's one the app knows how to replace.
pub fn current(app: &App) -> Option<Install> {
    let exe = std::env::current_exe().ok()?;
    current_at(app, &exe, std::env::var_os(DEV_VAR).is_some())
}

/// [`current`] for a known executable and opt-in setting, without reading the ambient
/// environment. A copy the update can't replace doesn't count; [`DEV_VAR`] still counts it, to
/// try the self-update.
fn current_at(app: &App, exe: &Path, opted_in: bool) -> Option<Install> {
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    if is_development_build(&exe, opted_in) {
        return None;
    }
    detect(app, &exe).filter(|install| opted_in || can_replace_all(app, install))
}

/// Whether the update can replace `install` and, for a standalone copy, the `share` files it
/// replaces along with the program.
fn can_replace_all(app: &App, install: &Install) -> bool {
    can_replace(install) && !matches!(install, Install::Standalone(file) if !can_replace_share(app, file))
}

/// Cargo's cache-directory marker at the root of a target directory.
const CARGO_CACHE_MARKER: &str = "CACHEDIR.TAG";

/// Whether `exe` runs from a Cargo build tree, wherever `CARGO_TARGET_DIR` or
/// `build.target-dir` put it. Normally Cargo caches rustc information in `.rustc_info.json` at
/// the target root, and that marker alone identifies the tree; a folder that installers put the
/// app in doesn't carry it, so this doesn't depend on the directory being called `target`. But
/// the cache can be disabled (`CARGO_CACHE_RUSTC_INFO=0`) or a tree copied without the hidden
/// file, so a profile output directory is also accepted when it has Cargo's `.fingerprint`,
/// `deps` and `build` subdirectories and its target root carries `CACHEDIR.TAG`. With
/// `build.build-dir` set, `.rustc_info.json` and those subdirectories live elsewhere and the
/// profile directory has only Cargo's `.cargo-lock` file, which is accepted too.
/// `CACHEDIR.TAG` alone isn't Cargo-specific (the same signature marks any cache directory), so
/// it only corroborates that profile layout.
fn is_development_build(exe: &Path, opted_in: bool) -> bool {
    !opted_in && exe.ancestors().skip(1).any(is_cargo_build_dir)
}

/// Whether `dir` is a Cargo target root (by its rustc-info cache marker) or a profile output
/// directory inside one (by the cache-disabled fallback).
fn is_cargo_build_dir(dir: &Path) -> bool {
    if dir.join(".rustc_info.json").is_file() {
        return true;
    }
    let Some(profile_parent) = dir.parent() else { return false };
    let is_profile = dir.join(".cargo-lock").is_file() || [".fingerprint", "deps", "build"].into_iter().all(|name| dir.join(name).is_dir());
    if !is_profile {
        return false;
    }
    // The target root is the profile's parent, or its grandparent for cross-compiled output
    // (`<target>/<triple>/<profile>`). It must carry Cargo's marker, as a file rather than a
    // directory of that name.
    if profile_parent.join(CARGO_CACHE_MARKER).is_file() {
        return true;
    }
    profile_parent.parent().is_some_and(|root| root.join(CARGO_CACHE_MARKER).is_file())
}

/// Whether this installed copy can update itself. [`release_installs`] says whether a given
/// release has a file to update it from.
pub fn is_self_updating(app: &App) -> bool {
    std::env::current_exe().is_ok_and(|exe| can_self_update_at(app, &exe, std::env::var_os(DEV_VAR).is_some()))
}

/// [`is_self_updating`] for a known executable and opt-in setting. On Windows the update helper
/// installs nothing for an unsigned copy, so only a signed one is offered the update; [`DEV_VAR`]
/// still offers it, to try the self-update.
fn can_self_update_at(app: &App, exe: &Path, opted_in: bool) -> bool {
    current_at(app, exe, opted_in).is_some() && (opted_in || is_signed(exe))
}

/// Whether `exe` carries a valid Authenticode signature.
#[cfg(windows)]
fn is_signed(exe: &Path) -> bool {
    verifysign::verify_file(exe).is_ok()
}

/// Only Windows refuses an unsigned copy's update.
#[cfg(not(windows))]
fn is_signed(_exe: &Path) -> bool {
    true
}

/// Whether `release` has a file the self-update can install this copy from. Only a standalone
/// program on macOS is checked, against the release's macOS `.tar.gz`, with or without
/// [`DEV_VAR`]. Call it together with [`is_self_updating`], which answers for every other reason
/// a copy can't update itself.
pub fn release_installs(app: &App, release: &Release) -> bool {
    if !cfg!(target_os = "macos") {
        return true;
    }
    let Ok(exe) = std::env::current_exe() else { return true };
    release_installs_at(app, &exe, std::env::var_os(DEV_VAR).is_some(), release, Platform::current())
}

/// [`release_installs`] for a known executable, opt-in setting and platform, without reading the
/// ambient environment.
fn release_installs_at(app: &App, exe: &Path, opted_in: bool, release: &Release, platform: Option<Platform>) -> bool {
    current_at(app, exe, opted_in).is_none_or(|install| installs_from(app, &install, release, platform))
}

/// Whether the update can install `install` from `release` on `platform`. Only a standalone
/// program on macOS is checked against the release's files. A release without its file list
/// passes: `updater::install` fetches the full release before it picks the file.
fn installs_from(app: &App, install: &Install, release: &Release, platform: Option<Platform>) -> bool {
    match (install, platform) {
        (Install::Standalone(_), Some(platform)) if platform.os == crate::Os::MacOs => {
            release.assets.is_empty() || pick_asset(release, &[app.id], platform, &[install.kind()]).is_some()
        }
        _ => true,
    }
}

/// Whether `exe` sits in a folder a macOS package manager owns (Homebrew, MacPorts, Nix).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn macos_package_manager_owns(exe: &Path) -> bool {
    // Homebrew (both prefixes), MacPorts and the Nix store; component-wise, so `/opt/homebrewed` is not one
    ["/opt/homebrew", "/usr/local/Cellar", "/usr/local/Homebrew", "/usr/local/opt", "/opt/local", "/nix"].iter().any(|root| exe.starts_with(root))
}

/// Whether the update can replace `install`, for the kinds it replaces in place from this
/// process.
#[cfg(target_os = "macos")]
fn can_replace(install: &Install) -> bool {
    match install {
        Install::MacApp(bundle) => can_replace_bundle(bundle),
        Install::Standalone(file) => can_replace_file(file),
        _ => true,
    }
}

/// Whether the update can replace `install`, for the kinds it replaces in place from this
/// process.
#[cfg(target_os = "linux")]
fn can_replace(install: &Install) -> bool {
    match install {
        Install::AppImage(file) | Install::Standalone(file) => can_replace_file(file),
        _ => true,
    }
}

/// Other platforms install from the helper or a package manager, or have no self-update.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn can_replace(_install: &Install) -> bool {
    true
}

/// Whether `access(2)` refuses `wanted` on `path`. Only a clear refusal counts: a check that
/// fails for another reason leaves the update on offer, and the install reports any failure.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn refused(path: &Path, wanted: rustix::fs::Access) -> bool {
    use rustix::io::Errno;

    matches!(rustix::fs::access(path, wanted), Err(Errno::ACCESS | Errno::PERM | Errno::ROFS))
}

/// Whether the update can replace the file at `file`. It writes the new file beside it and
/// renames that over it, which needs permission to write and search the folder. In a folder with
/// the sticky bit, the rename also needs the user to own the folder or the file.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn can_replace_file(file: &Path) -> bool {
    use rustix::fs::Access;
    use std::os::unix::fs::MetadataExt;

    file.parent().is_none_or(|folder| {
        if refused(folder, Access::WRITE_OK | Access::EXEC_OK) {
            return false;
        }
        // A folder or file whose details can't be read doesn't refuse.
        let (Ok(folder), Ok(file)) = (std::fs::metadata(folder), std::fs::symlink_metadata(file)) else { return true };
        !sticky_refuses(folder.mode(), folder.uid(), file.uid(), rustix::process::geteuid().as_raw())
    })
}

/// Whether a folder's sticky bit stops `user` renaming over a file in it: the folder (`mode`,
/// owned by `folder_owner`) has the bit, and `user` owns neither the folder nor the file. Root is
/// never stopped.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn sticky_refuses(mode: u32, folder_owner: u32, file_owner: u32, user: u32) -> bool {
    mode & 0o1000 != 0 && user != 0 && folder_owner != user && file_owner != user
}

/// The prefix of a program in `bin`, when `bin` is a folder named `bin` with the app's
/// `share/<id>` folder next to it. The update then also replaces the release's `share` files
/// under that prefix.
#[cfg_attr(not(unix), allow(dead_code))]
fn prefix_with_share<'a>(app: &App, bin: &'a Path) -> Option<&'a Path> {
    bin.parent().filter(|prefix| bin.file_name().is_some_and(|n| n == "bin") && prefix.join("share").join(app.id).is_dir())
}

/// Whether the update can replace the other files of the standalone copy at `file`: the app's
/// other programs beside it, and its `share` files in the folders a release writes to. A copy
/// with no `share/<id>` folder has no such `share` files.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn can_replace_share(app: &App, file: &Path) -> bool {
    use rustix::fs::Access;

    let Some(bin) = file.parent() else { return true };
    if sticky_folder_refuses(bin, |path| is_app_program(app, path)) {
        return false;
    }
    let Some(prefix) = prefix_with_share(app, bin) else { return true };
    let share = prefix.join("share");
    let replaced = |path: &Path| is_share_file(app, &share, path);
    share_folders(app, &share).iter().all(|folder| !refused(folder, Access::WRITE_OK | Access::EXEC_OK) && !sticky_folder_refuses(folder, replaced))
}

/// Whether `path` names one of the app's programs: `<id>`, or `<id>-<name>` with no extension,
/// such as `<id>-cli`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn is_app_program(app: &App, path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name == app.id || name.strip_prefix(app.id).is_some_and(|rest| rest.starts_with('-') && !rest.contains('.'))
}

/// Whether `path` under `share` is one of the app's files: anything in `<id>` or `doc/<id>`, and
/// elsewhere a file named `<id>.<ext>` or `<bundle id>.<ext>`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn is_share_file(app: &App, share: &Path, path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.starts_with(share.join(app.id))
        || path.starts_with(share.join("doc").join(app.id))
        || [app.id.to_string(), app.bundle_id()].iter().any(|stem| name.strip_prefix(stem.as_str()).is_some_and(|rest| rest.starts_with('.')))
}

/// Whether `folder` has the sticky bit and holds a file that `replaced` accepts and that the
/// user can't rename over (see [`sticky_refuses`]). At most [`SHARE_ENTRY_LIMIT`] entries are
/// read. A folder or entry that can't be read doesn't refuse.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn sticky_folder_refuses(folder: &Path, replaced: impl Fn(&Path) -> bool) -> bool {
    sticky_folder_refuses_for(folder, replaced, rustix::process::geteuid().as_raw())
}

/// [`sticky_folder_refuses`] for the user with id `user`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn sticky_folder_refuses_for(folder: &Path, replaced: impl Fn(&Path) -> bool, user: u32) -> bool {
    use std::os::unix::fs::MetadataExt;

    let Ok(meta) = std::fs::metadata(folder) else { return false };
    if meta.mode() & 0o1000 == 0 {
        return false;
    }
    let Ok(entries) = std::fs::read_dir(folder) else { return false };
    entries.flatten().take(SHARE_ENTRY_LIMIT).any(|entry| {
        let path = entry.path();
        replaced(&path)
            && std::fs::symlink_metadata(&path).is_ok_and(|file| !file.is_dir() && sticky_refuses(meta.mode(), meta.uid(), file.uid(), user))
    })
}

/// The most entries, files and folders together, that [`share_folders`] reads in each of the
/// app's folder, `doc/<id>` and `icons/hicolor`. A folder past the limit is not checked, and the
/// install reports it if it can't write there.
const SHARE_ENTRY_LIMIT: usize = 4096;

/// The existing folders under `share` that a release's files can go in:
/// - the app's own folder and every folder inside it;
/// - `doc/<id>` and every folder inside it;
/// - `icons/hicolor`, and each `<size>/apps` folder in it that holds the app's icon;
/// - `applications`, `metainfo`, `mime/packages` and `pixmaps`.
///
/// When the app's folder, `doc/<id>`, `icons/hicolor`, `applications`, `metainfo` or
/// `mime/packages` doesn't exist, the check takes its nearest existing parent instead, `share`
/// included, since the update creates the folder there. Only the old flat release layout writes
/// to `pixmaps`, so for a missing `pixmaps` the check takes its nearest existing parent inside
/// `share`, and never `share` itself. The walk follows links, as the update's writes do, and
/// reads a folder it reaches twice only once. An icon size that the installed copy doesn't have
/// yet is not checked, whether or not its `<size>` folder exists.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn share_folders(app: &App, share: &Path) -> Vec<PathBuf> {
    share_folders_within(app, share, SHARE_ENTRY_LIMIT)
}

/// [`share_folders`], reading at most `limit` entries in each of the app's folder, `doc/<id>`
/// and `icons/hicolor`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn share_folders_within(app: &App, share: &Path, limit: usize) -> Vec<PathBuf> {
    let nearest = |folder: &Path| folder.ancestors().find(|p| p.is_dir()).map(Path::to_path_buf);
    let nearest_inside = |folder: &Path| folder.ancestors().take_while(|p| *p != share).find(|p| p.is_dir()).map(Path::to_path_buf);
    let mut folders = Vec::new();
    let own = share.join(app.id);
    if own.is_dir() {
        walk_folders(own, limit, &mut folders);
    } else {
        folders.extend(nearest(&own));
    }
    let doc = share.join("doc").join(app.id);
    if doc.is_dir() {
        walk_folders(doc, limit, &mut folders);
    } else {
        folders.extend(nearest(&doc));
    }
    let hicolor = share.join("icons/hicolor");
    if hicolor.is_dir() {
        folders.push(hicolor.clone());
        let icons = [app.bundle_id(), app.id.to_string()].map(|name| [format!("{name}.png"), format!("{name}.svg")]);
        if let Ok(sizes) = std::fs::read_dir(&hicolor) {
            for size in sizes.flatten().take(limit) {
                let apps = size.path().join("apps");
                if icons.iter().flatten().any(|icon| apps.join(icon).symlink_metadata().is_ok()) {
                    folders.push(apps);
                }
            }
        }
    } else {
        folders.extend(nearest(&hicolor));
    }
    folders.extend(["applications", "metainfo", "mime/packages"].iter().filter_map(|folder| nearest(&share.join(folder))));
    folders.extend(nearest_inside(&share.join("pixmaps")));
    folders.sort();
    folders.dedup();
    folders
}

/// Adds `root` and every folder inside it to `folders`, reading at most `limit` entries. Links
/// are followed, and a folder reached a second time, through a link or a loop, is listed but not
/// read again. A folder that can't be read is listed, and the folders inside it are not. An
/// entry that can't be read is skipped.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
fn walk_folders(root: PathBuf, limit: usize, folders: &mut Vec<PathBuf>) {
    let mut read = std::collections::HashSet::new();
    let mut budget = limit;
    let mut pending = vec![root];
    while let Some(dir) = pending.pop() {
        folders.push(dir.clone());
        if budget == 0 || !std::fs::canonicalize(&dir).is_ok_and(|real| read.insert(real)) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if budget == 0 {
                break;
            }
            budget -= 1;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
            }
        }
    }
}

/// Other platforms make no check here, as in [`can_replace`].
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn can_replace_share(_app: &App, _file: &Path) -> bool {
    true
}

/// `access(2)`'s `_APPEND_OK`: on a directory, permission to add a subdirectory.
#[cfg(target_os = "macos")]
const ADD_SUBDIRECTORY_OK: rustix::fs::Access = rustix::fs::Access::from_bits_retain(1 << 13);

/// `access(2)`'s `_DELETE_OK`: permission to remove the entry from its folder.
#[cfg(target_os = "macos")]
const DELETE_OK: rustix::fs::Access = rustix::fs::Access::from_bits_retain(1 << 12);

/// Whether the update can replace the bundle at `bundle`. It creates a staging folder beside the
/// bundle and renames the bundle into another folder, which needs permission to add a
/// subdirectory to the parent and permission to move the bundle itself.
#[cfg(target_os = "macos")]
fn can_replace_bundle(bundle: &Path) -> bool {
    let Some(parent) = bundle.parent() else { return true };
    !is_translocated(bundle) && !refused(parent, ADD_SUBDIRECTORY_OK) && !refused(bundle, ADD_SUBDIRECTORY_OK | DELETE_OK)
}

/// Whether `bundle` sits in an App Translocation folder under the system's per-user folders.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn is_translocated(bundle: &Path) -> bool {
    bundle.starts_with("/private/var/folders") && bundle.components().any(|part| part.as_os_str() == "AppTranslocation")
}

/// Whether `exe` sits in a folder a Windows package manager owns (scoop, winget, Chocolatey, the
/// Store).
#[cfg_attr(not(windows), allow(dead_code))]
fn windows_package_manager_owns(exe: &Path) -> bool {
    // Compared as text so the rule reads the same wherever it is tested: either separator, any case.
    let text = exe.to_string_lossy().to_lowercase();
    let parts: Vec<&str> = text.split(['\\', '/']).collect();
    parts.contains(&"windowsapps")
        || parts.windows(2).any(|w| matches!((w[0], w[1]), ("scoop", "apps" | "shims") | ("winget", "packages") | ("chocolatey", "lib" | "bin")))
}

#[cfg(target_os = "macos")]
fn detect(app: &App, exe: &Path) -> Option<Install> {
    if let Some(bundle) = exe.ancestors().find(|p| p.extension().is_some_and(|e| e == "app"))
        && let Ok(plist) = plist::Value::from_file(bundle.join("Contents/Info.plist"))
        && let Some(id) = plist.as_dictionary().and_then(|d| d.get("CFBundleIdentifier")).and_then(|v| v.as_string())
        && id == app.bundle_id()
    {
        return Some(Install::MacApp(bundle.to_path_buf()));
    }
    // A package manager's copy is its to update (replacing it in place leaves its record stale), and
    // a program inside another app's bundle is that app's file.
    if macos_package_manager_owns(exe) || exe.ancestors().any(|p| p.extension().is_some_and(|e| e == "app")) {
        return None;
    }
    if exe.is_file() {
        return Some(Install::Standalone(exe.to_path_buf()));
    }
    None
}

/// How the Windows copy at `exe` was installed: portable (a `portable.txt` beside it), by its MSI
/// (registered in either context, or the launcher's own per-user package), else a loose folder.
#[cfg(windows)]
fn detect(app: &App, exe: &Path) -> Option<Install> {
    let dir = exe.parent()?;
    if dir.join("portable.txt").is_file() {
        return Some(Install::Portable(exe.to_path_buf()));
    }
    // Any folder, either context: the MSI's own registration says so (see `msi_copy`). A copy
    // installed for everyone must never be unpacked over as if it were a loose folder.
    if crate::msi_copy::windows::find(app, exe).is_some() {
        return Some(Install::Msi(exe.to_path_buf()));
    }
    // scoop, winget, Chocolatey and the Store own their copies; an in-place replace leaves their record stale
    if windows_package_manager_owns(exe) {
        return None;
    }
    if exe.is_file() {
        return Some(Install::Standalone(exe.to_path_buf()));
    }
    None
}

/// What `exe`'s MSI update does (update in place, or move a per-user copy to everyone), or why it
/// stops. Asked while the app still runs (so it can show the error) and again by the helper.
#[cfg(windows)]
fn msi_step(app: &App, exe: &Path) -> Result<crate::msi_migrate::Step, String> {
    crate::msi_migrate::windows::step(app, exe)
}

/// Whether the helper starts the app after the update step: never while the old app is still
/// running (a second copy of it would open), always after a successful step, and after a failed
/// one only while its exe is still there.
#[cfg_attr(not(windows), allow(dead_code))]
fn restarts(app_exited: bool, step_succeeded: bool, exe_exists: bool) -> bool {
    app_exited && (step_succeeded || exe_exists)
}

/// The exe to start after an MSI update of a registered copy: the one its registrations name now
/// (the package may have moved an exact old default folder), else `exe`.
#[cfg(windows)]
fn exe_after_msi(app: &App, registered: bool, exe: &Path) -> PathBuf {
    registered
        .then(|| crate::msi_copy::registered_exe(&crate::msi_copy::windows::registered_exes(app)))
        .flatten()
        .filter(|moved| is_own_executable(app, moved))
        .unwrap_or_else(|| exe.to_path_buf())
}

/// How the running copy at `exe` was installed on Linux, or `None` when it isn't this app's to
/// update: a Flatpak release bundle (per user or system-wide), an AppImage, the deb/rpm/Arch
/// package at `/usr/bin/<id>`, or a standalone copy outside the package managers' folders
/// ([`linux_location`]). A `/usr/bin/<id>` that none of those package databases claims is also
/// `None`: it sits in a system folder, so it isn't treated as a standalone copy.
#[cfg(target_os = "linux")]
fn detect(app: &App, exe: &Path) -> Option<Install> {
    if process::in_sandbox() {
        // A release bundle updates itself in place. Per-user uses `flatpak install --user`.
        // System-wide uses `pkexec flatpak install --system`: `--user` would leave a second copy
        // beside the system one, and Flatpak's own install-bundle polkit action is denied on
        // current polkit when its message cannot fill in `$(path)`. A copy from a repository is
        // that repository's to update, and an unsigned bundle can't go into a signed one.
        let level = flatpak_level()?;
        return from_bundle(app, level).then_some(Install::Flatpak);
    }
    if let Some(appimage) = std::env::var_os("APPIMAGE") {
        return Some(Install::AppImage(PathBuf::from(appimage)));
    }
    match linux_location(app, exe) {
        LinuxLocation::PackagePath => {}
        LinuxLocation::System => return None,
        LinuxLocation::Elsewhere => return exe.is_file().then(|| Install::Standalone(exe.to_path_buf())),
    }
    let t = std::time::Duration::from_secs(10);
    if process::capture("dpkg-query", &["-W", "-f", "${db:Status-Status}", app.id], t).is_some_and(|s| s.trim() == "installed") {
        return Some(Install::Package(PackageKind::Deb, exe.to_path_buf()));
    }
    if process::capture("rpm", &["-q", app.id], t).is_some() {
        return Some(Install::Package(PackageKind::Rpm, exe.to_path_buf()));
    }
    // A pacman install of the app. We detect the release's own Arch package (by bundle id).
    // AUR builds (`<id>`, `<id>-bin`, `<id>-git`) are left to pacman and the AUR helper.
    if crate::detect::linux::has_pacman_db() && process::capture("pacman", &["-Q", &app.bundle_id()], t).is_some() {
        return Some(Install::Package(PackageKind::ArchPkg, exe.to_path_buf()));
    }
    // `/usr/bin/<id>` that no package database claims, e.g. copied there by hand or put there by a
    // package whose name isn't the app's id. Replacing it would write over a system folder without
    // a package to do it, so it doesn't update itself.
    log::info!("{}: {} belongs to no deb, rpm or Arch package, so it doesn't update itself", app.name, exe.display());
    None
}

/// Where a Linux executable outside a Flatpak or AppImage runs from, for [`detect`].
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum LinuxLocation {
    /// `/usr/bin/<id>`, where the release's deb, rpm and Arch packages put it. Only this path is
    /// checked against the package databases: a copy elsewhere isn't the package's even when the
    /// package is installed too.
    PackagePath,
    /// Anywhere else under a folder a package manager owns (`/usr` but not `/usr/local`, `/opt`,
    /// `/bin`, `/sbin`, `/nix`, `/snap` or `/var/lib/snapd/snap` (Snap on Fedora and Arch), Guix's
    /// `/gnu`, or a Homebrew `.linuxbrew` prefix): a
    /// distribution, AUR, Nix, Snap, Guix or Homebrew copy. It's theirs to update, and replacing
    /// it in place would write over a file the package manager owns (or fail on a read-only
    /// store), as on macOS ([`macos_package_manager_owns`]).
    System,
    /// Anywhere else, such as a folder in the user's home: a standalone copy that replaces itself.
    Elsewhere,
}

/// Which [`LinuxLocation`] `exe` is in, from its path alone.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn linux_location(app: &App, exe: &Path) -> LinuxLocation {
    if exe == Path::new("/usr/bin").join(app.id) {
        LinuxLocation::PackagePath
    } else if (!exe.starts_with("/usr/local") && ["/usr", "/opt", "/bin", "/sbin", "/nix", "/snap", "/var/lib/snapd/snap", "/gnu"].iter().any(|dir| exe.starts_with(dir)))
        // Homebrew on Linux: /home/linuxbrew/.linuxbrew, or ~/.linuxbrew for one user
        || exe.components().any(|c| c.as_os_str() == ".linuxbrew")
    {
        LinuxLocation::System
    } else {
        LinuxLocation::Elsewhere
    }
}

/// Where this sandbox's Flatpak copy is installed, from `app-path` in `/.flatpak-info`.
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FlatpakLevel {
    User,
    System,
}

#[cfg(target_os = "linux")]
fn flatpak_level() -> Option<FlatpakLevel> {
    let info = std::fs::read_to_string("/.flatpak-info").ok()?;
    parse_flatpak_level(&info)
}

#[cfg(target_os = "linux")]
fn parse_flatpak_level(info: &str) -> Option<FlatpakLevel> {
    let mut in_instance = false;
    for line in info.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_instance = line == "[Instance]";
        } else if in_instance && let Some(path) = line.strip_prefix("app-path=") {
            // The host's path. The per-user installation is ~/.local/share/flatpak there; it can't
            // be looked up from in here, where XDG_DATA_HOME points into ~/.var/app instead.
            let path = path.trim();
            if path.contains("/.local/share/flatpak/") {
                return Some(FlatpakLevel::User);
            }
            if path.contains("/var/lib/flatpak/") {
                return Some(FlatpakLevel::System);
            }
            return None;
        }
    }
    None
}

/// Whether this copy was installed from a `.flatpak` bundle. `/.flatpak-info` doesn't say where
/// it came from, so the host's Flatpak is asked on the same installation the copy runs from.
#[cfg(target_os = "linux")]
fn from_bundle(app: &App, level: FlatpakLevel) -> bool {
    let scope = match level {
        FlatpakLevel::User => "--user",
        FlatpakLevel::System => "--system",
    };
    process::capture_flatpak(&["info", scope, "--show-origin", &app.bundle_id()], std::time::Duration::from_secs(3))
        .is_some_and(|origin| is_bundle_origin(app, origin.trim()))
}

/// Flatpak names a bundle's origin after the last part of the app id, numbered when that name is
/// taken: `<id>-origin`, `<id>1-origin`, …
#[cfg(target_os = "linux")]
fn is_bundle_origin(app: &App, origin: &str) -> bool {
    origin.strip_prefix(app.id).and_then(|rest| rest.strip_suffix("-origin")).is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
fn detect(_app: &App, _exe: &Path) -> Option<Install> {
    None
}

/// Downloads `release` for `platform`, puts it in place and starts the helper that relaunches
/// it once this process has exited. On success the caller must quit promptly. `last` is called
/// once the download is ready and returns when the app may replace itself and restart.
#[allow(clippy::too_many_arguments)]
pub fn update(
    app: &App,
    install: &Install,
    release: &Release,
    platform: Platform,
    http: &Http,
    paths: &Paths,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
    last: &dyn Fn() -> Result<(), String>,
    running_version: Option<&str>,
) -> Result<(), String> {
    prepare_update(app, install, release, platform, http, paths, stop, phase, last, running_version)?.start()
}

/// What remains of an update after [`prepare_update`]: starting the helper, which waits for this
/// process to exit. On Windows the helper then installs the update and starts the new version;
/// on macOS and Linux the update is already in place and the helper starts the new version.
#[must_use = "the update only installs or restarts once the restart is started"]
pub struct Restart {
    helper: Option<Box<dyn FnOnce() -> Result<(), String> + Send>>,
}

impl Restart {
    /// Nothing is left to start.
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn scheduled() -> Self {
        Self { helper: None }
    }

    /// `start` runs when the app closes.
    #[cfg_attr(not(any(windows, unix, test)), allow(dead_code))]
    pub(crate) fn later(start: impl FnOnce() -> Result<(), String> + Send + 'static) -> Self {
        Self { helper: Some(Box::new(start)) }
    }

    /// Starts the helper. Call it when the app's exit is certain: the helper waits only a minute
    /// for the app to exit. On Windows it installs nothing when the app is still running after
    /// that; on macOS and Linux the Flatpak helper starts nothing then, and the other helpers
    /// start the new version anyway.
    pub fn start(self) -> Result<(), String> {
        self.helper.map_or(Ok(()), |start| start())
    }
}

impl std::fmt::Debug for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Restart").field("pending", &self.helper.is_some()).finish()
    }
}

/// [`update`] without starting the helper: the caller starts the returned [`Restart`] when the
/// app's exit is certain. On macOS and Linux the update is in place when this returns.
#[allow(clippy::too_many_arguments)]
pub fn prepare_update(
    app: &App,
    install: &Install,
    release: &Release,
    platform: Platform,
    http: &Http,
    paths: &Paths,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
    last: &dyn Fn() -> Result<(), String>,
    running_version: Option<&str>,
) -> Result<Restart, String> {
    let cur_v = running_version.and_then(|v| current_version(app, v));
    if let (Some(rel_v), Some(cur_v)) = (release.version.as_ref(), cur_v.as_ref())
        && !version::offers_update(rel_v, cur_v)
    {
        // Compare the effective version: an explicit pretend version also exercises installation.
        // Without that opt-in, a build from source requires a strictly higher release.
        return Err(format!("{} {} is not newer than currently running version {}", app.name, release.version_label(), cur_v));
    }
    let (asset, _) = pick_asset(release, &[app.id], platform, &[install.kind()])
        .ok_or_else(|| format!("{} {} has no {} for {}", app.name, release.version_label(), install.kind().label(), platform.label()))?;
    let asset = asset.clone();
    crate::install::bind_artifact_version(app, release, &asset)?;
    crate::install::check_download_link(app, release, &asset)?;
    // Nothing is downloaded for a copy that can't install it.
    #[cfg(windows)]
    windows_precheck(app, install, paths)?;
    let mut job = Job { http, paths, stop, phase, install_locations: &Default::default(), replace: false };
    #[cfg(not(windows))]
    let recorded = keep_download(paths, &asset);
    let file = install::download(release, &asset, &mut job)?;
    #[cfg(windows)]
    windows_check_download(app, install, &file)?;
    // The restart ends this process: nothing else may be under way.
    last()?;
    phase(JobPhase::Installing);
    // Written before the swap starts. On Windows the helper does the swap, so the record is
    // written when it starts (see `windows_helper`): it clears the record when it fails, which a
    // write after it started could undo.
    #[cfg(not(windows))]
    mark_installed(paths, release);
    let pid = std::process::id();
    let scheduled = (|| -> Result<Restart, String> {
        match install {
            #[cfg(target_os = "macos")]
            Install::MacApp(bundle) => {
                let existing = Installed { version: None, source: InstallSource::MacApp, location: bundle.display().to_string(), package: None };
                let wait = install::macos::CheckTurn { stop, phase: &mut *phase };
                let placed =
                    install::macos::install_dmg(app, &file, Some(&existing), None, false, paths, &install::macos::Expect::self_update(release), wait);
                after_install(paths, &file, recorded, &placed);
                let placed = placed?;
                Ok(relaunch_after_exit(app, pid, &[Path::new("/usr/bin/open").as_os_str(), placed.as_os_str()]))
            }
            #[cfg(target_os = "linux")]
            Install::AppImage(path) => {
                let result = replace_appimage(&file, path);
                after_install(paths, &file, recorded, &result);
                result?;
                Ok(relaunch_after_exit(app, pid, &[path.as_os_str()]))
            }
            #[cfg(target_os = "linux")]
            Install::Package(kind, exe) => {
                let result = install::linux::install_package(*kind, &file, None, stop, phase);
                after_install(paths, &file, recorded, &result);
                result?;
                Ok(relaunch_after_exit(app, pid, &[exe.as_os_str()]))
            }
            #[cfg(target_os = "linux")]
            Install::Flatpak => {
                let system = match flatpak_level() {
                    Some(FlatpakLevel::System) => true,
                    Some(FlatpakLevel::User) => false,
                    // Re-read rather than trusting a stale guess: `--user` on a system copy
                    // would install a second one beside it.
                    None => return Err(format!("this copy of {} can't update itself; download the new version instead", app.name)),
                };
                let result = install::linux::update_flatpak_bundle(&file, &app.bundle_id(), system, stop, phase);
                after_install(paths, &file, recorded, &result);
                result?;
                relaunch_flatpak_after_exit(app)
            }
            #[cfg(windows)]
            Install::Msi(exe) => windows_helper(app, paths, release, pid, exe, &msi_finish(&file)?),
            #[cfg(windows)]
            Install::Portable(exe) => {
                exe.parent().ok_or_else(|| format!("{} isn't in a folder", app.name))?;
                windows_helper(app, paths, release, pid, exe, &Finish::Portable { zip: file, tmp: paths.cache.join(PORTABLE_STAGING) })
            }
            #[cfg(windows)]
            Install::Standalone(exe) => {
                exe.parent().ok_or_else(|| format!("{} isn't in a folder", app.name))?;
                windows_helper(app, paths, release, pid, exe, &Finish::Portable { zip: file, tmp: paths.cache.join(PORTABLE_STAGING) })
            }
            #[cfg(unix)]
            Install::Standalone(exe) => {
                let result = replace_standalone(app, paths, &file, exe);
                after_install(paths, &file, recorded, &result);
                result?;
                Ok(relaunch_after_exit(app, pid, &[exe.as_os_str()]))
            }
            #[allow(unreachable_patterns)]
            _ => {
                let _ = (pid, &file);
                Err(format!("this copy of {} can't update itself; download the new version instead", app.name))
            }
        }
    })();
    let scheduled = unmark_unless_scheduled(paths, scheduled);
    // On macOS and Linux the update is in place once it is prepared.
    #[cfg(not(windows))]
    if scheduled.is_ok() {
        record_done(paths, &release.version_label());
    }
    scheduled
}

/// Records `release` as just installed, so it isn't offered again for [`JUST_UPDATED_FOR`]: its
/// tag, the time, and the version it installs. The outcome records of an earlier update are
/// removed: this update writes its own.
fn mark_installed(paths: &Paths, release: &Release) {
    let _ = std::fs::remove_file(failure_file(paths));
    let _ = std::fs::remove_file(done_file(paths));
    let text = format!("{}\n{}\n{}\n", release.tag, crate::now(), release.version_label());
    if let Err(e) = crate::paths::write_atomic(&marker(paths), text.as_bytes()) {
        log::warn!("couldn't record the self-update: {e}");
    }
}

/// Where a finished update records the version it installed, for the new version's first start.
fn done_file(paths: &Paths) -> PathBuf {
    paths.data.join("self-update-done.txt")
}

/// Records that the update to `version` has been installed.
fn record_done(paths: &Paths, version: &str) {
    if let Err(e) = crate::paths::write_atomic(&done_file(paths), version.as_bytes()) {
        log::warn!("couldn't record the finished self-update: {e}");
    }
}

/// Where a self-update records the name and size of the release file it downloads. The file stays
/// in the download folder after a failed install, and the next try checks it against its
/// published checksum and skips the transfer. On Windows the update helper removes the download
/// itself, so nothing is recorded there.
#[cfg(not(windows))]
fn kept_download_record(paths: &Paths) -> PathBuf {
    paths.cache.join("self-update-download.txt")
}

/// Records `asset` as this app's self-update download. A finished or partial download of another
/// release file that the record named is removed first, so the download folder holds the files of
/// one self-update download at most. Returns whether the record was written.
#[cfg(not(windows))]
fn keep_download(paths: &Paths, asset: &crate::release::Asset) -> bool {
    let record = kept_download_record(paths);
    if let Some(text) = read_record(&record) {
        let mut lines = text.lines();
        let (name, size) = (lines.next().unwrap_or_default(), lines.next().and_then(|s| s.parse::<u64>().ok()));
        if (name, size) != (asset.name.as_str(), Some(asset.size)) && crate::release::valid_file_name(name) {
            let dest = paths.downloads().join(name);
            match size {
                Some(size) => crate::http::discard(&dest, size),
                // A record without a size still names a finished download.
                None => {
                    let _ = std::fs::remove_file(dest);
                }
            }
        }
    }
    let text = format!("{}\n{}\n", asset.name, asset.size);
    match crate::paths::write_atomic(&record, text.as_bytes()) {
        Ok(()) => true,
        Err(e) => {
            log::warn!("couldn't record the self-update download: {e}");
            false
        }
    }
}

/// Keeps the download `file` for the next try when the install's `result` is a failure that
/// [`keeps_download`] allows and [`keep_download`] `recorded` it; otherwise removes the file and
/// its record. A download without a record is removed, since nothing would remove it later.
#[cfg(not(windows))]
fn after_install<T>(paths: &Paths, file: &Path, recorded: bool, result: &Result<T, String>) {
    if !(recorded && keeps_download(result)) {
        let _ = std::fs::remove_file(file);
        let _ = std::fs::remove_file(kept_download_record(paths));
    }
}

/// Whether the download stays for the next try after the install's `result`. A failed install
/// keeps it, except on macOS when the disk image could not be opened: an image still attached
/// from an install that ended part way makes hdiutil refuse every attach of the same file with
/// "Resource busy", while a new download of it attaches.
#[cfg(not(windows))]
fn keeps_download<T>(result: &Result<T, String>) -> bool {
    match result {
        Ok(_) => false,
        #[cfg(target_os = "macos")]
        Err(e) if e.starts_with(install::macos::IMAGE_NOT_OPENED) => false,
        Err(_) => true,
    }
}

/// The version the scheduled update installs, from the record [`mark_installed`] wrote: its third
/// line, or the tag on its first line in a record without one.
#[cfg_attr(not(windows), allow(dead_code))]
fn scheduled_version(paths: &Paths) -> Option<String> {
    let text = read_record(&marker(paths))?;
    let mut lines = text.lines();
    let tag = lines.next();
    let version = lines.nth(1).filter(|version| !version.trim().is_empty()).or(tag);
    version.map(str::trim).filter(|version| !version.is_empty()).map(str::to_owned)
}

/// The most of an outcome record that is read: a reason longer than this is cut short.
const RECORD_LIMIT: usize = 4096;

/// The text of the record at `path`, at most [`RECORD_LIMIT`] bytes of it. `None` when it is
/// missing, unreadable or not a plain file (a link is not followed).
fn read_record(path: &Path) -> Option<String> {
    use std::io::Read;
    if !is_plain_file(path) {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).ok()?.take(RECORD_LIMIT as u64 + 1).read_to_end(&mut bytes).ok()?;
    let cut = bytes.len() > RECORD_LIMIT;
    bytes.truncate(RECORD_LIMIT);
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        // A multi-byte character split at the limit decodes as U+FFFD; drop it.
        text = text.trim_end_matches('\u{FFFD}').to_owned();
        text.push('…');
    }
    Some(text)
}

/// What the last self-update came to, read once by the next start ([`take_outcome`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// The update installed this version, which is the one running now.
    Installed(Version),
    /// The update could not be installed, for this reason.
    Failed(String),
}

/// What the last self-update came to, once: the records are removed as they are read. A
/// recorded failure wins over a record of a finished update, unless this program was put in place
/// after the failure: the copy that failed has been replaced, and the failure is dropped. A
/// finished update counts only when the version it installed is `running`, the version this copy
/// counts as.
pub fn take_outcome(paths: &Paths, running: &str) -> Option<Outcome> {
    let program = std::env::current_exe().ok();
    take_outcome_of(paths, running, program.as_deref())
}

/// [`take_outcome`] for a known program, without reading the ambient executable.
fn take_outcome_of(paths: &Paths, running: &str, program: Option<&Path>) -> Option<Outcome> {
    let stale = program.is_some_and(|program| failure_predates(paths, program));
    let failure = take_failure(paths).filter(|_| !stale);
    let done = read_record(&done_file(paths));
    let _ = std::fs::remove_file(done_file(paths));
    if let Some(reason) = failure {
        let reason = reason.trim();
        let reason = if reason.is_empty() { "no reason was recorded" } else { reason };
        return Some(Outcome::Failed(reason.to_owned()));
    }
    let installed = version::parse(done?.trim())?;
    (Some(&installed) == version::parse(running).as_ref()).then_some(Outcome::Installed(installed))
}

/// Whether the recorded failure is older than `program`, by its modified or created time. A
/// program installed from a package carries the package's build time, so one built before the
/// failure is not detected and the failure is reported once as usual. `false` when either time
/// can't be read.
fn failure_predates(paths: &Paths, program: &Path) -> bool {
    let (Ok(record), Ok(program)) = (std::fs::metadata(failure_file(paths)), std::fs::metadata(program)) else { return false };
    let Ok(recorded) = record.modified() else { return false };
    [program.modified(), program.created()].into_iter().flatten().any(|at| at > recorded)
}

/// Removes the record of [`mark_installed`] when the swap couldn't be scheduled, so the update
/// stays on offer.
fn unmark_unless_scheduled<T>(paths: &Paths, scheduled: Result<T, String>) -> Result<T, String> {
    if scheduled.is_err() {
        let _ = std::fs::remove_file(marker(paths));
    }
    scheduled
}

/// Copies the new AppImage beside the running one and renames it over it. The running copy is
/// mounted from its own (now unlinked) file, so it keeps working until it exits.
#[cfg(target_os = "linux")]
fn replace_appimage(new: &Path, path: &Path) -> Result<(), String> {
    path.parent().ok_or("the AppImage isn't in a folder")?;
    // The replacement AppImage must be executable; 0o755 is not group/world-writable.
    let staged = stage_copy(new, path, 0o755)?; // NOSONAR
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("couldn't replace {} ({e})", path.display())
    })
}

/// Replaces a standalone copy from a release tarball, along with what the tarball install put
/// beside it: the release's other programs that are already in the executable's folder, and,
/// when that folder is a prefix's `bin` with the app's `share/<id>` folder next to it, everything
/// under the tarball's `share` (data, desktop entry, icons). A copy on its own in some folder gets
/// only its executable. Files the release doesn't ship are never touched. Every file is copied
/// next to its destination before any is renamed over it, so a copy that fails (full disk,
/// unwritable folder) changes nothing; the executable is renamed last.
#[cfg(unix)]
fn replace_standalone(app: &App, paths: &Paths, tarball: &Path, exe: &Path) -> Result<(), String> {
    // Per-process names, so two updates running at once can't remove each other's files.
    let staging = paths.cache.join(format!("{}.standalone.{}", app.id, std::process::id()));
    let result = install::tree::unpack(app, tarball, &staging).and_then(|tree| replace_from_tree(app, &tree, exe));
    remove_staging(&staging);
    result
}

/// Removes an unpacked release. The archive sets each folder's mode, and a folder its owner can't
/// list or write can't be emptied, so those folders get owner access back first.
#[cfg(unix)]
fn remove_staging(staging: &Path) {
    use std::os::unix::fs::PermissionsExt;
    if std::fs::remove_dir_all(staging).is_ok() || std::fs::symlink_metadata(staging).is_err() {
        return;
    }
    let mut pending = vec![staging.to_path_buf()];
    while let Some(dir) = pending.pop() {
        if !std::fs::symlink_metadata(&dir).is_ok_and(|m| m.file_type().is_dir()) {
            continue;
        }
        let _ = std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700));
        let Ok(entries) = std::fs::read_dir(&dir) else { continue };
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir()) {
                pending.push(entry.path());
            }
        }
    }
    let _ = std::fs::remove_dir_all(staging);
}

#[cfg(unix)]
fn replace_from_tree(app: &App, tree: &Path, exe: &Path) -> Result<(), String> {
    let new_exe = tree.join("bin").join(app.id);
    if !is_plain_file(&new_exe) {
        return Err(format!("release archive did not contain binary for {}", app.id));
    }
    // A `bin` that is a link would bring in programs from outside the release.
    let release_bin = tree.join("bin");
    if !std::fs::symlink_metadata(&release_bin).is_ok_and(|m| m.file_type().is_dir()) {
        return Err(format!("{} isn't a folder", release_bin.display()));
    }
    let bin = exe.parent().ok_or_else(|| "executable isn't in a folder".to_string())?;
    let mut moves = Vec::new();
    for entry in std::fs::read_dir(&release_bin).map_err(|e| format!("couldn't read the release's programs ({e})"))? {
        let entry = entry.map_err(|e| format!("couldn't read {} ({e})", release_bin.display()))?;
        let name = entry.file_name();
        if name != app.id && is_plain_file(&entry.path()) && bin.join(&name).is_file() {
            moves.push((entry.path(), bin.join(&name)));
        }
    }
    if let Some(prefix) = prefix_with_share(app, bin) {
        for file in plain_files(&tree.join("share"))? {
            let relative = file.strip_prefix(tree).map_err(|_| "staged file escapes the release".to_string())?;
            moves.push((file.clone(), prefix.join(relative)));
        }
    }
    moves.push((new_exe, exe.to_path_buf()));
    // A folder where a file goes would fail its rename after earlier files were replaced.
    if let Some((_, to)) = moves.iter().find(|(_, to)| to.symlink_metadata().is_ok_and(|m| m.is_dir())) {
        return Err(format!("couldn't replace {} (it's a folder)", to.display()));
    }
    // Copy everything next to its destination first, so a full disk or an unwritable folder
    // stops the update before any installed file changes. Renames within a folder don't fail
    // for those reasons, and the executable is renamed last.
    let mut staged = Vec::new();
    for (i, (from, to)) in moves.iter().enumerate() {
        match stage(from, to, i == moves.len() - 1) {
            Ok(s) => staged.push((s, to)),
            Err(e) => {
                for (s, _) in &staged {
                    let _ = std::fs::remove_file(s);
                }
                return Err(e);
            }
        }
    }
    let mut result = Ok(());
    for (s, to) in &staged {
        if result.is_ok() {
            result = std::fs::rename(s, to).map_err(|e| format!("couldn't replace {} ({e})", to.display()));
        }
        if result.is_err() {
            let _ = std::fs::remove_file(s);
        }
    }
    result
}

/// Copies `from` to a hidden file next to `to` and returns its path. Group and world write and
/// the set-id bits are never carried over from the archive; the executable is always 0755.
#[cfg(unix)]
fn stage(from: &Path, to: &Path, executable: bool) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = to.parent().ok_or_else(|| format!("{} isn't in a folder", to.display()))?;
    std::fs::create_dir_all(dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
    let mode = std::fs::metadata(from).map(|m| m.permissions().mode()).unwrap_or(0o644);
    stage_copy(from, to, if executable { 0o755 } else { mode & 0o755 })
}

/// Copies `from` to a new file beside `to` and returns its path. The file is created exclusively
/// under an unpredictable name, so a link someone left at a guessable name is never followed,
/// and its bytes and `mode` go through the open handle. Removed again when any step fails.
#[cfg(unix)]
fn stage_copy(from: &Path, to: &Path, mode: u32) -> Result<PathBuf, String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = to.parent().ok_or_else(|| format!("{} isn't in a folder", to.display()))?;
    let write_error = |e: std::io::Error| format!("couldn't write next to {} ({e})", to.display());
    // fixed prefix: the temp name's length must not grow with the destination's basename
    let mut staged = tempfile::Builder::new().prefix(".staged-").suffix(".update").tempfile_in(dir).map_err(write_error)?;
    let mut source = std::fs::File::open(from).map_err(|e| format!("couldn't read {} ({e})", from.display()))?;
    std::io::copy(&mut source, staged.as_file_mut()).map_err(write_error)?;
    staged.as_file().set_permissions(std::fs::Permissions::from_mode(mode)).map_err(|e| e.to_string())?;
    staged.into_temp_path().keep().map_err(|e| e.to_string())
}

/// A regular file, not a link to one.
fn is_plain_file(path: &Path) -> bool {
    std::fs::symlink_metadata(path).is_ok_and(|m| m.file_type().is_file())
}

/// The regular files under `dir`, without following links; none when `dir` doesn't exist. Links
/// and other entries that aren't files or folders are skipped. A `dir` that is a link or a file, and
/// a folder that can't be read, are errors, so an update never goes ahead with its files left out.
#[cfg(unix)]
fn plain_files(dir: &Path) -> Result<Vec<PathBuf>, String> {
    match std::fs::symlink_metadata(dir) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("couldn't read {} ({e})", dir.display())),
        Ok(m) if !m.file_type().is_dir() => return Err(format!("{} isn't a folder", dir.display())),
        Ok(_) => {}
    }
    let mut files = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(dir) = pending.pop() {
        let entries = std::fs::read_dir(&dir).map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("couldn't read {} ({e})", dir.display()))?;
            let kind = entry.file_type().map_err(|e| format!("couldn't read {} ({e})", entry.path().display()))?;
            if kind.is_dir() {
                pending.push(entry.path());
            } else if kind.is_file() {
                files.push(entry.path());
            }
        }
    }
    Ok(files)
}

/// A [`Restart`] that starts `sh`, which waits for `pid` to exit and then runs `command`.
/// Detached, so it outlives us. Nothing starts before the app's exit is certain: a close the user
/// cancels leaves no helper counting down its wait.
#[cfg(unix)]
fn relaunch_after_exit(app: &App, pid: u32, command: &[&std::ffi::OsStr]) -> Restart {
    relaunch_with(app, pid, command, process::spawn_detached)
}

/// [`relaunch_after_exit`], with `spawn` starting the helper.
#[cfg(unix)]
fn relaunch_with(
    app: &App,
    pid: u32,
    command: &[&std::ffi::OsStr],
    spawn: impl FnOnce(std::process::Command) -> Result<(), String> + Send + 'static,
) -> Restart {
    let cmd = helper_command(app, pid, command);
    Restart::later(move || spawn(cmd).map_err(|e| format!("couldn't schedule the restart ({e})")))
}

#[cfg(unix)]
fn helper_command(app: &App, pid: u32, command: &[&std::ffi::OsStr]) -> std::process::Command {
    let mut cmd = process::command("/bin/sh");
    cmd.arg("-c")
        .arg(r#"pid=$1; shift; i=0; while kill -0 "$pid" 2>/dev/null && [ $i -lt 600 ]; do sleep 0.1; i=$((i+1)); done; exec "$@""#)
        .arg("sh")
        .arg(pid.to_string())
        .args(command);
    clear_pretend_version(&mut cmd, app);
    cmd
}

#[cfg(target_os = "linux")]
fn parse_flatpak_instance_id(info: &str) -> Option<String> {
    for line in info.lines() {
        if let Some(id) = line.trim().strip_prefix("instance-id=") {
            let trimmed = id.trim();
            if !trimmed.is_empty() {
                return Some(trimmed.to_string());
            }
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn flatpak_instance_id() -> Option<String> {
    let info = std::fs::read_to_string("/.flatpak-info").ok()?;
    parse_flatpak_instance_id(&info)
}

#[cfg(target_os = "linux")]
fn parse_flatpak_host_pid(ps_output: &str, target_instance: &str) -> Option<u32> {
    for line in ps_output.lines() {
        let mut parts = line.split_whitespace();
        if let (Some(inst), Some(pid)) = (parts.next(), parts.next())
            && inst == target_instance
        {
            return pid.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
fn flatpak_host_pid() -> Option<u32> {
    let instance_id = flatpak_instance_id()?;
    let out = process::capture_flatpak(&["ps", "--columns=instance,pid"], std::time::Duration::from_secs(3))?;
    parse_flatpak_host_pid(&out, &instance_id)
}

/// A [`Restart`] that starts a host helper, which waits for this Flatpak instance to exit, then
/// relaunches it via Flatpak on the host. Nothing starts before the app's exit is certain.
#[cfg(target_os = "linux")]
fn relaunch_flatpak_after_exit(app: &App) -> Result<Restart, String> {
    let host_pid = flatpak_host_pid();
    let instance_id = flatpak_instance_id();
    // Each wait clause polls until the old process is gone, then execs the relaunch command.
    // If the old process is still alive after 60 s the helper gives up WITHOUT launching a second
    // copy, which is the safe choice: the user can relaunch manually once the old one exits.
    let (wait_clause, target_arg) = match (host_pid, instance_id) {
        (Some(pid), _) => (
            r#"pid=$1; shift; i=0; while kill -0 "$pid" 2>/dev/null; do [ $i -ge 600 ] && exit 1; sleep 0.1; i=$((i+1)); done; exec "$@""#,
            pid.to_string(),
        ),
        (None, Some(id)) => (
            r#"id=$1; shift; i=0; while flatpak ps --columns=instance 2>/dev/null | grep -q "^$id$"; do [ $i -ge 600 ] && exit 1; sleep 0.1; i=$((i+1)); done; exec "$@""#,
            id,
        ),
        // Without either, starting it again could only meet the old copy still running.
        (None, None) => return Err(format!("{} is updated; quit and reopen it to start the new version", app.name)),
    };
    let script = format!("({wait_clause}) &");
    let cmd = flatpak_restart_command(app, &script, &target_arg);
    Ok(Restart::later(move || process::spawn_detached(cmd).map_err(|e| format!("couldn't schedule the Flatpak restart ({e})"))))
}

#[cfg(target_os = "linux")]
fn flatpak_restart_command(app: &App, script: &str, target: &str) -> std::process::Command {
    let app_id = app.bundle_id();
    // flatpak run reapplies persistent overrides; removing inherited variables alone is insufficient.
    let global_unset = format!("--unset-env={PRETEND_VAR}");
    let app_unset = format!("--unset-env={}", pretend_var(app));
    let mut cmd = process::command("flatpak-spawn");
    cmd.args(["--host", "sh", "-c", script, "sh", target, "flatpak", "run", &global_unset, &app_unset, &app_id]);
    clear_pretend_version(&mut cmd, app);
    cmd
}

/// The first argument of the Windows update helper: a copy of this program that finishes an
/// update once the launcher has exited (see [`finish`]).
pub const FINISH_FLAG: &str = "--finish-update";

/// What the update helper does once the old launcher has exited.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
enum Finish {
    /// Run the new MSI, which reinstalls into the same folder.
    Msi(PathBuf),
    /// Unpack the new portable zip into `tmp` and copy it over the launcher's folder.
    Portable { zip: PathBuf, tmp: PathBuf },
}

/// The helper's arguments: [`FINISH_FLAG`], the process id to wait for, the exe to start
/// afterwards, then the step.
#[cfg_attr(not(windows), allow(dead_code))]
fn helper_args(pid: u32, exe: &Path, finish: &Finish) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![FINISH_FLAG.into(), pid.to_string().into(), exe.into()];
    match finish {
        Finish::Msi(msi) => args.extend(["msi".into(), msi.into()]),
        Finish::Portable { zip, tmp } => args.extend(["portable".into(), zip.into(), tmp.into()]),
    }
    args
}

/// Reads [`helper_args`] back.
#[cfg_attr(not(windows), allow(dead_code))]
fn parse_helper_args(args: &[OsString]) -> Option<(u32, PathBuf, Finish)> {
    let [flag, pid, exe, step @ ..] = args else { return None };
    if flag != FINISH_FLAG {
        return None;
    }
    let pid = pid.to_str()?.parse().ok()?;
    let finish = match step {
        [kind, msi] if kind == "msi" => Finish::Msi(msi.into()),
        [kind, zip, tmp] if kind == "portable" => Finish::Portable { zip: zip.into(), tmp: tmp.into() },
        _ => return None,
    };
    Some((pid, exe.into(), finish))
}

#[cfg_attr(not(windows), allow(dead_code))]
fn msi_finish(file: &Path) -> Result<Finish, String> {
    install::msi_path(file).map(Finish::Msi)
}

/// The cache folder the update helper is copied into.
#[cfg_attr(not(windows), allow(dead_code))]
const HELPER_DIR: &str = "launcher-update-helper";
/// The cache folder a portable update is unpacked into.
#[cfg_attr(not(windows), allow(dead_code))]
const PORTABLE_STAGING: &str = "launcher-update";

/// Copies this program into the cache as the update helper, which waits for `pid` to exit, does
/// `finish` and starts `exe`. Run from the cache, the helper doesn't hold the installed exe that
/// the step replaces. The returned [`Restart`] records `release` as installed and starts the
/// helper.
#[cfg(windows)]
fn windows_helper(app: &App, paths: &Paths, release: &Release, pid: u32, exe: &Path, finish: &Finish) -> Result<Restart, String> {
    let program = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
    let dir = paths.cache.join(HELPER_DIR);
    // The previous update's copy, when it's still there.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't schedule the update ({e})"))?;
    let helper = dir.join(program.file_name().ok_or("this program has no file name")?);
    std::fs::copy(&program, &helper).map_err(|e| format!("couldn't schedule the update ({e})"))?;
    let mut cmd = process::command(&helper);
    cmd.args(helper_args(pid, exe, finish));
    clear_pretend_version(&mut cmd, app);
    let (paths, release) = (paths.clone(), release.clone());
    Ok(Restart::later(move || {
        mark_installed(&paths, &release);
        let started = process::spawn_detached(cmd).map_err(|e| format!("couldn't schedule the update ({e})"));
        unmark_unless_scheduled(&paths, started)
    }))
}

/// Finishes an update when this program was started as the update helper; `args` are its
/// arguments without the program name. `None` when they don't start with [`FINISH_FLAG`]. The
/// app is started again even when the step fails: the old copy is then still in place.
#[cfg(windows)]
pub fn finish(app: &App, args: &[OsString]) -> Option<Result<(), String>> {
    if args.first().is_none_or(|a| a != FINISH_FLAG) {
        return None;
    }
    let Some((pid, exe, step)) = parse_helper_args(args) else { return Some(Err("the update helper's arguments are incomplete".into())) };
    let Ok(me) = std::env::current_exe() else { return Some(Err("the update helper couldn't find itself".into())) };
    let paths = match Paths::for_exe(app, &exe) {
        Ok(paths) => paths,
        Err(e) => return Some(Err(e)),
    };
    // Nothing runs unless this is the app's own update, signed like this helper.
    if let Err(e) = check_helper(app, &me, &paths, &exe, &step).and_then(|()| signed_like(app, &me, &exe)) {
        record_failure(&paths, &e);
        return Some(Err(e));
    }
    // Read before the step: the update may move or replace the old exe.
    let registered =
        matches!(step, Finish::Msi(_)) && matches!(crate::msi_copy::windows::find(app, &exe), Some(crate::msi_copy::MsiCopy::Registered { .. }));
    let wait = |pid| process::wait_for_exit(pid, std::time::Duration::from_secs(60));
    let start = |exe: &Path| {
        signed_like(app, &me, exe)
            .and_then(|()| process::spawn_detached(std::process::Command::new(exe)).map_err(|e| format!("couldn't start {} ({e})", exe.display())))
    };
    Some(finish_step(app, &me, &paths, pid, exe, &step, registered, &wait, &start))
}

/// The helper's work once its checks have passed: waits for `pid` with `wait`, does `step`, and
/// starts the app with `start` when [`restarts`] allows it. Every failure is recorded for the
/// app's next start.
#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
fn finish_step(
    app: &App,
    reference: &Path,
    paths: &Paths,
    pid: u32,
    exe: PathBuf,
    step: &Finish,
    registered: bool,
    wait: &dyn Fn(u32) -> bool,
    start: &dyn Fn(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let portable_before = has_portable_marker(&exe);
    let stepped = step_after_wait(app, reference, pid, &exe, step, wait);
    let done = stepped.result;
    if let Err(e) = &done {
        record_failure(paths, e);
    }
    // The new app, or the old one when the step failed.
    let exe = if done.is_ok() { exe_after_msi(app, registered, &exe) } else { exe };
    // A standalone copy updated from the portable zip gains `portable.txt` and keeps its folders
    // beside it from now on: the records the new app reads go there.
    let version = scheduled_version(paths);
    let moved = (done.is_ok() && has_portable_marker(&exe) != portable_before).then(|| Paths::for_exe(app, &exe).ok()).flatten();
    if let Some(after) = &moved {
        move_marker(paths, after);
    }
    let paths = moved.as_ref().unwrap_or(paths);
    // An app that is still running gets no second copy. A failed move can leave no program to
    // start; its recorded error already says how to get the app back, and a restart error must
    // not replace it.
    if !restarts(stepped.exited, done.is_ok(), exe.is_file()) {
        return done;
    }
    // Written before the new app starts, so its first start finds it. A failed start removes it.
    if done.is_ok()
        && let Some(version) = &version
    {
        record_done(paths, version);
    }
    let started = start(&exe);
    let restarted = started.is_ok();
    let result = with_restart(done, started);
    if !restarted && let Err(e) = &result {
        record_failure(paths, e);
    }
    result
}

/// The helper's result once it has tried to start the app: when both the update step and the
/// start fail, the error names both, the step's first.
#[cfg_attr(not(windows), allow(dead_code))]
fn with_restart(done: Result<(), String>, started: Result<(), String>) -> Result<(), String> {
    match (done, started) {
        (done, Ok(())) => done,
        (Ok(()), Err(restart)) => Err(restart),
        (Err(update), Err(restart)) => Err(format!("{update}; {restart}")),
    }
}

/// Whether `exe` has `portable.txt` beside it, which makes [`Paths::for_exe`] keep its folders
/// there.
#[cfg(windows)]
fn has_portable_marker(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| dir.join("portable.txt").is_file())
}

/// Moves the record of the scheduled update from `from` to `to`, the folders the updated app
/// reads, and removes outcome records left in `to` by an earlier update.
#[cfg(windows)]
fn move_marker(from: &Paths, to: &Paths) {
    let _ = std::fs::remove_file(failure_file(to));
    let _ = std::fs::remove_file(done_file(to));
    if let Some(text) = read_record(&marker(from))
        && crate::paths::write_atomic(&marker(to), text.as_bytes()).is_ok()
    {
        let _ = std::fs::remove_file(marker(from));
    }
}

/// Finishes an update on non-Windows platforms (no-op helper).
#[cfg(not(windows))]
pub fn finish(_app: &App, _args: &[OsString]) -> Option<Result<(), String>> {
    None
}

/// Where a failed update's error waits for the next start.
fn failure_file(paths: &Paths) -> PathBuf {
    paths.logs().join("self-update-error.txt")
}

/// Records a failed update: the next check offers the update again, and the next start shows
/// `error`. A record of the update as finished is removed.
#[cfg_attr(not(any(windows, feature = "egui")), allow(dead_code))]
pub(crate) fn record_failure(paths: &Paths, error: &str) {
    let _ = std::fs::remove_file(marker(paths));
    let _ = std::fs::remove_file(done_file(paths));
    let _ = std::fs::create_dir_all(paths.logs());
    let _ = crate::paths::write_atomic(&failure_file(paths), error.as_bytes());
}

/// The error of the last update, if it failed, once. A long error is cut short at
/// [`RECORD_LIMIT`] bytes.
pub fn take_failure(paths: &Paths) -> Option<String> {
    let file = failure_file(paths);
    let error = read_record(&file);
    if is_plain_file(&file) {
        let _ = std::fs::remove_file(&file);
    }
    error
}

/// Whether `exe` is named like this app's own executable (case-insensitive, any of its ids).
#[cfg(windows)]
fn is_own_executable(app: &App, exe: &Path) -> bool {
    exe.file_stem().and_then(|stem| stem.to_str()).is_some_and(|stem| app.stems().any(|id| id.eq_ignore_ascii_case(stem)))
}

/// Refuses an executable that isn't this app's own, as the helper would.
#[cfg(windows)]
fn own_executable(app: &App, exe: &Path) -> Result<(), String> {
    if is_own_executable(app, exe) { Ok(()) } else { Err(format!("the update helper only finishes {}'s own update", app.name)) }
}

/// Checks, before anything is downloaded, what the helper checks before it installs and what
/// does not depend on the download: this program and the installed exe are signed by one
/// publisher, and the helper finds the cache in `paths` that holds its copy, the download and the
/// staging folder.
#[cfg(windows)]
fn windows_precheck(app: &App, install: &Install, paths: &Paths) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
    match install {
        Install::Msi(exe) => signed_like(app, &me, exe)
            .and_then(|()| msi_step(app, exe).map(drop))
            .and_then(|()| own_executable(app, exe))
            .and_then(|()| same_cache_as_helper(app, paths, exe)),
        Install::Portable(exe) | Install::Standalone(exe) => {
            signed_like(app, &me, exe).and_then(|()| own_executable(app, exe)).and_then(|()| same_cache_as_helper(app, paths, exe))
        }
        _ => Ok(()),
    }
}

/// Checks the finished download, while the app still runs and can show the error: a new MSI is
/// signed by this program's publisher.
#[cfg(windows)]
fn windows_check_download(app: &App, install: &Install, download: &Path) -> Result<(), String> {
    match install {
        Install::Msi(_) => {
            let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
            signed_like(app, &me, download)
        }
        _ => Ok(()),
    }
}

/// Accepts `paths` when the update helper, which finds the app's folders again from `exe`
/// ([`Paths::for_exe`]), finds the same cache. Refuses an app that keeps its files in the
/// temporary folder because its own folders couldn't be found. The cache is created first, so a
/// first update finds it before the download does.
#[cfg(windows)]
fn same_cache_as_helper(app: &App, paths: &Paths, exe: &Path) -> Result<(), String> {
    std::fs::create_dir_all(&paths.cache).map_err(|e| format!("couldn't create {} ({e})", paths.cache.display()))?;
    let canonical = |p: &Path| p.canonicalize().ok();
    let theirs = Paths::for_exe(app, exe).ok().and_then(|helper| canonical(&helper.cache));
    if theirs.is_some() && theirs == canonical(&paths.cache) {
        Ok(())
    } else {
        Err(format!("{} can't update itself while it keeps its files in {}; download the new version instead", app.name, paths.cache.display()))
    }
}

/// Accepts `file` when it carries a valid Authenticode signature from the same publisher as
/// `reference`, this program. Refuses an unsigned file, another publisher's, and every file when
/// `reference` itself isn't signed.
#[cfg(windows)]
fn signed_like(app: &App, reference: &Path, file: &Path) -> Result<(), String> {
    let publisher = |path: &Path| verifysign::verify_file(path).map(|signature| signature.subject_name());
    let ours = publisher(reference).map_err(|e| format!("this copy of {} isn't signed ({e}), so it doesn't install updates", app.name))?;
    let theirs = publisher(file).map_err(|e| format!("{} isn't signed ({e})", file.display()))?;
    if ours == theirs { Ok(()) } else { Err(format!("{} isn't signed by {}'s publisher", file.display(), app.name)) }
}

/// Whether `file` is a Windows program or library (it starts with the `MZ` header), whatever its
/// name.
#[cfg(windows)]
fn is_program(file: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 2];
    std::fs::File::open(file).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"MZ"
}

/// Allows only the app's own update: `exe` is this app's MSI or portable installation,
/// of the kind `step` updates, and the helper (`me`), the download and the staging folder are in
/// that installation's cache. Anything else is refused before the helper runs or starts a file.
#[cfg(windows)]
fn check_helper(app: &App, me: &Path, paths: &Paths, exe: &Path, step: &Finish) -> Result<(), String> {
    let refused = || format!("the update helper only finishes {}'s own update", app.name);
    let canonical = |p: &Path| p.canonicalize().map_err(|_| refused());
    let cache = canonical(&paths.cache)?;
    if canonical(me.parent().ok_or_else(refused)?)? != cache.join(HELPER_DIR) {
        return Err(refused());
    }
    let exe = canonical(exe)?;
    // Whatever the folder, the program to restart is this app's own, never another executable.
    if !is_own_executable(app, &exe) {
        return Err(refused());
    }
    let downloads = canonical(&paths.downloads())?;
    let in_downloads = |file: &Path| canonical(file).map(|f| f.parent() == Some(downloads.as_path()));
    let allowed = match (detect(app, &exe), step) {
        (Some(Install::Msi(installed)), Finish::Msi(msi)) => installed == exe && in_downloads(msi)?,
        (Some(Install::Portable(installed) | Install::Standalone(installed)), Finish::Portable { zip, tmp }) => {
            installed == exe
                && in_downloads(zip)?
                && tmp.file_name().is_some_and(|n| n == PORTABLE_STAGING)
                && canonical(tmp.parent().ok_or_else(refused)?)? == cache
        }
        _ => false,
    };
    if allowed { Ok(()) } else { Err(refused()) }
}

/// [`step_after_wait`], waiting up to `wait` for `pid` to exit.
#[cfg(all(windows, test))]
fn run_step(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: std::time::Duration) -> Result<(), String> {
    step_after_wait(app, reference, pid, exe, step, &|pid| process::wait_for_exit(pid, wait)).result
}

/// What [`step_after_wait`] came to.
#[cfg(windows)]
struct Stepped {
    /// The old app had exited when the wait returned.
    exited: bool,
    result: Result<(), String>,
}

/// Calls `wait`, which says whether `pid` exited in time, does `step` once it has, and removes
/// the download either way. The MSI, and every program in the portable zip, must be signed like
/// `reference`.
#[cfg(windows)]
fn step_after_wait(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: &dyn Fn(u32) -> bool) -> Stepped {
    let waited = wait(pid);
    let result = if waited {
        match step {
            Finish::Msi(msi) => {
                signed_like(app, reference, msi).and_then(|()| msi_step(app, exe)).and_then(|step| crate::msi_migrate::windows::run(msi, &step))
            }
            Finish::Portable { zip, tmp } => match (exe.parent(), exe.file_name()) {
                (Some(dir), Some(executable)) => crate::portable::unpack_over(zip, tmp, dir, executable, &|file| {
                    // The expected executable must be signed even when its contents aren't a PE file.
                    if file.file_name() == Some(executable) || is_program(file) { signed_like(app, reference, file) } else { Ok(()) }
                }),
                _ => Err(format!("{} isn't in a folder", app.name)),
            },
        }
    } else {
        Err(format!("{} was still running after a minute, so the update wasn't installed", app.name))
    };
    match step {
        Finish::Msi(msi) => {
            let _ = std::fs::remove_file(msi);
        }
        // `unpack_over` removes the staging folder, unless it holds files it couldn't put back.
        Finish::Portable { zip, .. } => {
            let _ = std::fs::remove_file(zip);
        }
    }
    Stepped { exited: waited, result }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    #[test]
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
    #[test]
    fn macos_package_manager_folders_are_recognised() {
        for owned in [
            "/opt/homebrew/bin/artcraft-launcher",
            "/opt/homebrew/Cellar/artcraft/1.0/bin/artcraft-launcher",
            "/usr/local/Cellar/artcraft/1.0/bin/artcraft-launcher",
            "/usr/local/Homebrew/bin/artcraft-launcher",
            "/usr/local/opt/artcraft/bin/artcraft-launcher",
            "/opt/local/bin/artcraft-launcher",
            "/nix/store/abc123-artcraft/bin/artcraft-launcher",
        ] {
            assert!(macos_package_manager_owns(Path::new(owned)), "{owned}");
        }
        for loose in [
            "/Users/ada/bin/artcraft-launcher",
            "/usr/local/bin/artcraft-launcher",
            "/Users/ada/opt/homebrew/bin/artcraft-launcher",
            "/opt/homebrewed/artcraft-launcher",
            "/Applications/Tools/artcraft-launcher",
        ] {
            assert!(!macos_package_manager_owns(Path::new(loose)), "{loose}");
        }
    }

    #[test]
    fn windows_package_manager_folders_are_recognised() {
        for owned in [
            r"C:\Users\Ada\scoop\apps\artcraft-launcher\current\artcraft-launcher.exe",
            r"C:\Users\Ada\scoop\shims\artcraft-launcher.exe",
            r"C:\ProgramData\scoop\apps\artcraft-launcher\current\artcraft-launcher.exe",
            r"C:\Users\Ada\AppData\Local\Microsoft\WinGet\Packages\Storytold.ArtCraftLauncher_x\artcraft-launcher.exe",
            r"C:\ProgramData\chocolatey\lib\artcraft-launcher\tools\artcraft-launcher.exe",
            r"C:\Program Files\WindowsApps\Storytold.ArtCraftLauncher_1.0_x64\artcraft-launcher.exe",
        ] {
            assert!(windows_package_manager_owns(Path::new(owned)), "{owned}");
        }
        for loose in [
            r"C:\Tools\ArtCraft Launcher\artcraft-launcher.exe",
            r"D:\myscoopy\apps\artcraft-launcher.exe",
            r"C:\Users\Ada\Downloads\artcraft-launcher.exe",
        ] {
            assert!(!windows_package_manager_owns(Path::new(loose)), "{loose}");
        }
    }

    /// A program inside an `.app` bundle that is not this app's own (a different bundle id) is
    /// another app's file, not a loose copy of ours to replace.
    #[cfg(target_os = "macos")]
    #[test]
    fn an_executable_in_another_apps_bundle_is_not_self_updating() {
        let dir = tempfile::tempdir().unwrap();
        let macos = dir.path().join("Other.app/Contents/MacOS");
        std::fs::create_dir_all(&macos).unwrap();
        let mut info = plist::Dictionary::new();
        info.insert("CFBundleIdentifier".into(), plist::Value::String("com.example.other".into()));
        plist::Value::Dictionary(info).to_file_xml(dir.path().join("Other.app/Contents/Info.plist")).unwrap();
        let exe = macos.join("other");
        std::fs::write(&exe, b"binary").unwrap();
        assert_eq!(detect(&crate::suite::LAUNCHER, &exe), None);
        // a loose copy outside any bundle is still a standalone one
        let loose = dir.path().join("artcraft-launcher");
        std::fs::write(&loose, b"binary").unwrap();
        assert_eq!(detect(&crate::suite::LAUNCHER, &loose), Some(Install::Standalone(loose.clone())));
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

    /// A copy in a folder the user can write, as in `~/Applications`, keeps the update.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_copy_it_can_write_is_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let exe = launcher_bundle(&dir.path().join("Applications"));
        let bundle = exe.ancestors().nth(3).unwrap().canonicalize().unwrap();
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), Some(Install::MacApp(bundle)));
    }

    /// The staging folder can't be created in a folder the user can't write. [`DEV_VAR`] still
    /// offers the update, to try it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_copy_in_a_folder_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("Applications");
        let exe = launcher_bundle(&folder);
        let guard = ModeGuard::set(&folder, 0o555);
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, true).is_some(), "the opt-in still offers it");
        drop(guard);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, false).is_some(), "the answer follows the folder's permissions");
    }

    /// A standalone copy is replaced by writing beside it, so the folder has to be writable.
    /// [`DEV_VAR`] still offers the update, to try it.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_standalone_copy_in_a_folder_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("bin");
        std::fs::create_dir(&folder).unwrap();
        let exe = folder.join("artcraft-launcher");
        std::fs::write(&exe, b"binary").unwrap();
        let guard = ModeGuard::set(&folder, 0o555);
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, true).is_some(), "the opt-in still offers it");
        drop(guard);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, false).is_some(), "the answer follows the folder's permissions");
    }

    /// The update replaces a standalone copy's `share` files too, so that folder has to be
    /// writable as well. [`DEV_VAR`] still offers the update, to try it.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_standalone_copy_whose_share_folder_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let share = dir.path().join("share").join(crate::suite::LAUNCHER.id);
        std::fs::create_dir_all(&bin).unwrap();
        std::fs::create_dir_all(&share).unwrap();
        let exe = bin.join(crate::suite::LAUNCHER.id);
        std::fs::write(&exe, b"binary").unwrap();
        let guard = ModeGuard::set(&share, 0o555);
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, true).is_some(), "the opt-in still offers it");
        drop(guard);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, false).is_some(), "the answer follows the folder's permissions");
    }

    #[cfg(unix)]
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
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

    /// A folder outside the ones a release writes to doesn't count, such as another theme or
    /// another package's icon folder in a shared prefix.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_share_folder_a_release_does_not_write_to_does_not_refuse() {
        if running_as_root() {
            return;
        }
        let app = &crate::suite::LAUNCHER;
        for other in ["icons/SomeTheme", "icons/hicolor/48x48/apps"] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            let share = dir.path().join("share");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::create_dir_all(share.join(app.id)).unwrap();
            std::fs::create_dir_all(share.join(other)).unwrap();
            std::fs::write(share.join(other).join("other-app.png"), b"icon").unwrap();
            let exe = bin.join(app.id);
            std::fs::write(&exe, b"binary").unwrap();
            let _guard = ModeGuard::set(&share.join(other), 0o555);
            assert!(current_at(app, &exe, false).is_some(), "{other}");
        }
    }

    /// When every folder a release writes to exists, a `share` folder the user can't write doesn't
    /// refuse, even with `pixmaps` missing: only the old flat layout writes there.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_share_folder_it_cannot_write_does_not_refuse_when_the_release_folders_exist() {
        if running_as_root() {
            return;
        }
        let app = &crate::suite::LAUNCHER;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        let share = dir.path().join("share");
        std::fs::create_dir_all(&bin).unwrap();
        let doc = format!("doc/{}", app.id);
        for folder in [app.id, "applications", "icons/hicolor", &doc, "metainfo", "mime/packages"] {
            std::fs::create_dir_all(share.join(folder)).unwrap();
        }
        let exe = bin.join(app.id);
        std::fs::write(&exe, b"binary").unwrap();
        let _guard = ModeGuard::set(&share, 0o555);
        assert!(current_at(app, &exe, false).is_some());
    }

    /// A release writes into `doc/<id>`, `metainfo` and `mime/packages`, and the update creates a
    /// missing one in its nearest existing parent, `share` included, so a copy that lacks one of
    /// them and whose such parent the user can't write gets the download link.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_missing_release_folder_in_a_share_folder_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let app = &crate::suite::LAUNCHER;
        let doc = format!("doc/{}", app.id);
        for (missing, parent) in [("doc", ""), ("metainfo", ""), ("mime", ""), (doc.as_str(), "doc"), ("mime/packages", "mime")] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            let share = dir.path().join("share");
            std::fs::create_dir_all(&bin).unwrap();
            for folder in [app.id, "applications", "icons/hicolor", "pixmaps", &doc, "metainfo", "mime/packages"] {
                if !folder.starts_with(missing) {
                    std::fs::create_dir_all(share.join(folder)).unwrap();
                }
            }
            std::fs::create_dir_all(share.join(parent)).unwrap();
            let exe = bin.join(app.id);
            std::fs::write(&exe, b"binary").unwrap();
            let guard = ModeGuard::set(&share.join(parent), 0o555);
            assert_eq!(current_at(app, &exe, false), None, "{missing}");
            drop(guard);
            assert!(current_at(app, &exe, false).is_some(), "{missing}: the answer follows the folder's permissions");
        }
    }

    /// A folder in the app's folder that the user can't read is still checked itself, but the
    /// folders inside it are not, even one the user can't write.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_folder_in_the_app_folder_it_cannot_read_counts_only_by_its_own_permissions() {
        if running_as_root() {
            return;
        }
        let app = &crate::suite::LAUNCHER;
        for (mode, refused) in [(0o000, true), (0o111, true), (0o333, false)] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            let data = dir.path().join("share").join(app.id).join("data");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::create_dir_all(data.join("inner")).unwrap();
            let exe = bin.join(app.id);
            std::fs::write(&exe, b"binary").unwrap();
            let inner = ModeGuard::set(&data.join("inner"), 0o555);
            let guard = ModeGuard::set(&data, mode);
            assert_eq!(current_at(app, &exe, false).is_none(), refused, "{mode:o}");
            drop(guard);
            drop(inner);
            assert!(current_at(app, &exe, false).is_some(), "{mode:o}: the answer follows the folder's permissions");
        }
    }

    /// The update writes through a linked folder, so a folder inside the link's target that the
    /// user can't write gets the download link, also when the app's folder is the link.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_folder_behind_a_link_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let app = &crate::suite::LAUNCHER;
        for linked_app_folder in [false, true] {
            let dir = tempfile::tempdir().unwrap();
            let bin = dir.path().join("bin");
            let share = dir.path().join("share");
            let elsewhere = dir.path().join("elsewhere");
            std::fs::create_dir_all(&bin).unwrap();
            std::fs::create_dir_all(elsewhere.join("models")).unwrap();
            std::fs::create_dir_all(&share).unwrap();
            if linked_app_folder {
                std::os::unix::fs::symlink(&elsewhere, share.join(app.id)).unwrap();
            } else {
                std::fs::create_dir_all(share.join(app.id)).unwrap();
                std::os::unix::fs::symlink(&elsewhere, share.join(app.id).join("data")).unwrap();
            }
            let exe = bin.join(app.id);
            std::fs::write(&exe, b"binary").unwrap();
            let guard = ModeGuard::set(&elsewhere.join("models"), 0o555);
            assert_eq!(current_at(app, &exe, false), None, "linked app folder: {linked_app_folder}");
            drop(guard);
            assert!(current_at(app, &exe, false).is_some(), "linked app folder: {linked_app_folder}");
        }
    }

    /// In a sticky folder such as `/tmp`, only the owner of the folder or of the file, or root,
    /// can rename over the file.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn a_sticky_folder_refuses_a_file_another_user_owns() {
        let (me, other) = (501, 502);
        assert!(sticky_refuses(0o1777, 0, other, me), "neither the folder nor the file is mine");
        assert!(!sticky_refuses(0o1777, 0, me, me), "my file");
        assert!(!sticky_refuses(0o1777, me, other, me), "my folder");
        assert!(!sticky_refuses(0o1777, other, other, 0), "root");
        assert!(!sticky_refuses(0o777, 0, other, me), "no sticky bit");

        // A sticky folder of the user's own keeps the update.
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher");
        std::fs::write(&exe, b"binary").unwrap();
        let _guard = ModeGuard::set(dir.path(), 0o1777);
        assert!(can_replace_file(&exe));
    }

    /// The update also renames over the app's other programs and its `share` files, so a sticky
    /// folder holding one that another user owns refuses. The test reads the folder as another
    /// user, to whom the temporary folder and its files belong to someone else.
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
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
    #[test]
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

    #[test]
    fn a_prefix_counts_only_with_a_bin_folder_and_the_apps_share_folder() {
        let app = &crate::suite::LAUNCHER;
        let dir = tempfile::tempdir().unwrap();
        let bin = dir.path().join("bin");
        std::fs::create_dir_all(&bin).unwrap();
        assert_eq!(prefix_with_share(app, &bin), None, "no share folder");
        std::fs::create_dir_all(dir.path().join("share").join(app.id)).unwrap();
        assert_eq!(prefix_with_share(app, &bin), Some(dir.path()));
        let other = dir.path().join("tools");
        std::fs::create_dir_all(&other).unwrap();
        assert_eq!(prefix_with_share(app, &other), None, "not a bin folder");
    }

    /// Renaming a directory into another folder needs write permission on the directory, so a
    /// writable parent is not enough. Mode 555 stands in for a bundle another account owns.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_bundle_it_cannot_write_gets_the_download_link() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let exe = launcher_bundle(dir.path());
        let _guard = ModeGuard::set(exe.ancestors().nth(3).unwrap(), 0o555);
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
    }

    /// A copy opened from a mounted disk image is on a read-only volume.
    #[cfg(target_os = "macos")]
    #[test]
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

    /// The update replaces the bundle a symlink leads to, so that bundle's folder decides.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_symlinked_copy_is_judged_by_where_it_really_is() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let real = dir.path().join("real");
        launcher_bundle(&real);
        let links = dir.path().join("links");
        std::fs::create_dir(&links).unwrap();
        std::os::unix::fs::symlink(real.join("Launcher.app"), links.join("Launcher.app")).unwrap();
        let exe = links.join("Launcher.app/Contents/MacOS/artcraft-launcher");
        {
            let _guard = ModeGuard::set(&links, 0o555);
            let found = current_at(&crate::suite::LAUNCHER, &exe, false);
            assert_eq!(found, Some(Install::MacApp(real.canonicalize().unwrap().join("Launcher.app"))));
        }
        let _guard = ModeGuard::set(&real, 0o555);
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
    }

    /// A check that can't answer does not take the update away.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_check_that_cannot_answer_keeps_the_update() {
        let dir = tempfile::tempdir().unwrap();
        assert!(can_replace_bundle(&dir.path().join("missing/Launcher.app")));
        assert!(can_replace_bundle(Path::new("Launcher.app")));
    }

    /// A copy under an App Translocation folder gets the download link even where the folder
    /// allows the update.
    #[cfg(target_os = "macos")]
    #[test]
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
    #[test]
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

    /// Only the App Translocation folder under the system's per-user folders counts.
    #[test]
    fn only_the_system_translocation_folder_counts() {
        assert!(is_translocated(Path::new("/private/var/folders/ab/cd/T/AppTranslocation/0A1B/d/Launcher.app")));
        for other in ["/Applications/Launcher.app", "/Users/ada/AppTranslocation/Launcher.app", "/private/var/folders/ab/cd/T/Launcher.app"] {
            assert!(!is_translocated(Path::new(other)), "{other}");
        }
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
    #[test]
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

    /// A folder with the sticky bit: its owner may still move the bundle.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_sticky_folder_keeps_the_update_for_the_bundles_owner() {
        let dir = tempfile::tempdir().unwrap();
        let exe = launcher_bundle(dir.path());
        let _guard = ModeGuard::set(dir.path(), 0o1777);
        assert!(current_at(&crate::suite::LAUNCHER, &exe, false).is_some());
    }

    /// The update writes beside an AppImage or a standalone program, so a folder the user can't
    /// write can't take either.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_linux_copy_in_a_folder_it_cannot_write_cannot_be_replaced() {
        if running_as_root() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("launcher.AppImage");
        std::fs::write(&file, b"binary").unwrap();
        let kinds = [Install::AppImage(file.clone()), Install::Standalone(file.clone())];
        assert!(kinds.iter().all(can_replace), "a folder it can write keeps the update");
        let guard = ModeGuard::set(dir.path(), 0o555);
        assert!(!kinds.iter().any(can_replace));
        drop(guard);
        assert!(kinds.iter().all(can_replace), "the answer follows the folder's permissions");
    }

    /// A standalone copy in a folder the user can't write gets the download link; [`DEV_VAR`]
    /// still offers the update, to try it.
    #[cfg(target_os = "linux")]
    #[test]
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

    /// A check that can't answer does not take the update away.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_linux_check_that_cannot_answer_keeps_the_update() {
        let dir = tempfile::tempdir().unwrap();
        assert!(can_replace(&Install::AppImage(dir.path().join("missing/launcher.AppImage"))));
        assert!(can_replace(&Install::AppImage(PathBuf::from("launcher.AppImage"))));
    }

    #[cfg(unix)]
    #[test]
    fn msi_self_update_uses_the_target_of_a_symlinked_download() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("Launcher O'Brien.msi");
        std::fs::write(&file, b"test package").unwrap();
        let link = dir.path().join("cached.msi");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        assert_eq!(msi_finish(&link).unwrap(), Finish::Msi(file.canonicalize().unwrap()));
    }

    #[test]
    fn a_missing_msi_cannot_schedule_a_self_update() {
        let dir = tempfile::tempdir().unwrap();
        assert!(msi_finish(&dir.path().join("missing.msi")).is_err());
    }

    /// The helper reads back exactly what the launcher passed it, paths with spaces and
    /// apostrophes included, and nothing that is incomplete or unknown.
    #[test]
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

    /// Arguments that aren't the helper's leave the launcher to start as usual.
    #[cfg(windows)]
    #[test]
    fn only_the_helper_flag_starts_the_update_helper() {
        assert_eq!(finish(&crate::suite::LAUNCHER, &[]), None);
        assert_eq!(finish(&crate::suite::LAUNCHER, &[OsString::from("--hidden")]), None);
        assert!(finish(&crate::suite::LAUNCHER, &[OsString::from(FINISH_FLAG)]).is_some_and(|done| done.is_err()));
    }

    /// The portable update waits for the old launcher to exit, then replaces its files with the
    /// zip's folder, keeps the settings beside it, and removes the download and the staging folder.
    #[cfg(windows)]
    #[test]
    fn a_portable_update_replaces_the_launcher_after_it_exits() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ArtCraft Launcher");
        std::fs::create_dir_all(folder.join("ArtCraftLauncherData")).unwrap();
        std::fs::write(folder.join("artcraft-launcher.exe"), "0.1.0").unwrap();
        std::fs::write(folder.join("portable.txt"), "").unwrap();
        std::fs::write(folder.join("ArtCraftLauncherData").join("settings.json"), "{}").unwrap();
        let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        let signed_exe = std::fs::read(system_program("cmd.exe")).unwrap();
        for (name, body) in [
            ("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", signed_exe.as_slice()),
            ("artcraft-launcher-0.2.0-windows-x64-portable/portable.txt", b"".as_slice()),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        let tmp = dir.path().join("cache").join("launcher-update");
        let step = Finish::Portable { zip: zip.clone(), tmp: tmp.clone() };

        let mut old = BlockedLauncher::start();
        let exe = folder.join("artcraft-launcher.exe");
        std::thread::scope(|scope| {
            // Inside the scope, so a failed assertion drops the gate and releases the helper
            // before the scope joins it.
            let (gate, wait) = WaitGate::new();
            let (helper_exe, pid) = (exe.clone(), old.id());
            let helper = scope.spawn(move || step_after_wait(&crate::suite::LAUNCHER, &system_program("cmd.exe"), pid, &helper_exe, &step, &wait));
            gate.entered();
            old.assert_running("while the helper was waiting");
            assert_eq!(std::fs::read(&exe).unwrap(), b"0.1.0", "replaced while the old launcher was still running");
            old.finish().expect("the stand-in launcher couldn't be reaped");
            gate.release();
            let stepped = helper.join().unwrap();
            assert!(stepped.exited, "the wait reported that the launcher was still running although it had exited");
            stepped.result.unwrap();
        });
        assert_eq!(std::fs::read(&exe).unwrap(), signed_exe);
        assert_eq!(std::fs::read_to_string(folder.join("ArtCraftLauncherData").join("settings.json")).unwrap(), "{}");
        assert!(!folder.join("artcraft-launcher-0.2.0-windows-x64-portable").exists(), "the zip's folder must not be nested in the launcher's");
        assert!(!zip.exists() && !tmp.exists(), "the download and the staging folder are removed");
    }

    /// Stands in for the old launcher: `sort.exe` keeps running until the test closes its stdin
    /// pipe, so the helper's wait cannot succeed early.
    #[cfg(windows)]
    struct BlockedLauncher {
        child: Option<std::process::Child>,
    }

    #[cfg(windows)]
    impl BlockedLauncher {
        fn start() -> Self {
            let child = std::process::Command::new(system_program("sort.exe"))
                .stdin(std::process::Stdio::piped())
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap();
            Self { child: Some(child) }
        }

        fn id(&self) -> u32 {
            self.child.as_ref().unwrap().id()
        }

        /// Fails unless the stand-in is still running. `try_wait` keeps the handle owned, so
        /// `finish` and `Drop` can still reap it.
        fn assert_running(&mut self, when: &str) {
            let child = self.child.as_mut().unwrap();
            assert!(child.try_wait().unwrap().is_none(), "the stand-in launcher had already exited {when}");
        }

        /// Closes the pipe so `sort.exe` exits, and reaps it. If reaping fails the child stays
        /// owned by `self`, so `Drop` can still kill and reap it.
        fn finish(&mut self) -> std::io::Result<std::process::ExitStatus> {
            let child = self.child.as_mut().unwrap();
            drop(child.stdin.take());
            child.wait()
        }
    }

    #[cfg(windows)]
    impl Drop for BlockedLauncher {
        fn drop(&mut self) {
            if let Some(mut child) = self.child.take() {
                // Closing the pipe normally lets sort exit; killing is the fallback after a failed
                // assertion, and the wait reaps it either way.
                drop(child.stdin.take());
                let _ = child.kill();
                let _ = child.wait();
            }
        }
    }

    /// A wait for [`step_after_wait`] that the test controls: it reports when the helper has
    /// reached its wait, blocks until the test releases it, and then asks the real
    /// [`process::wait_for_exit`] whether the launcher has exited, without waiting.
    #[cfg(windows)]
    struct WaitGate {
        entered: std::sync::mpsc::Receiver<()>,
        release: std::sync::mpsc::Sender<()>,
    }

    #[cfg(windows)]
    impl WaitGate {
        /// The gate and the wait to hand to the helper. The wait holds the only sender of
        /// `entered`, so a helper that ends without reaching its wait fails [`WaitGate::entered`]
        /// instead of blocking it, and dropping the gate releases the wait.
        fn new() -> (Self, impl Fn(u32) -> bool + Send) {
            let (entered_tx, entered) = std::sync::mpsc::channel();
            let (release, released) = std::sync::mpsc::channel::<()>();
            let released = std::sync::Mutex::new(released);
            let wait = move |pid| {
                let _ = entered_tx.send(());
                let _ = released.lock().map(|released| released.recv());
                process::wait_for_exit(pid, std::time::Duration::ZERO)
            };
            (Self { entered, release }, wait)
        }

        /// Blocks until the helper has reached its wait.
        fn entered(&self) {
            self.entered.recv().expect("the helper never reached its wait");
        }

        /// Lets the helper's wait return.
        fn release(&self) {
            self.release.send(()).expect("the helper stopped before its wait was released");
        }
    }

    /// A launcher without `portable.txt` is a standalone installation, and the helper still
    /// replaces its own executable after it exits. The test holds a stand-in launcher alive
    /// through a pipe and holds the helper inside its wait until the stand-in has exited, so the
    /// executable must still be the old one while the helper waits, and the real
    /// [`process::wait_for_exit`] must report the live stand-in as running and the exited one as gone.
    #[cfg(windows)]
    #[test]
    fn a_standalone_update_replaces_the_launcher_after_it_exits() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ArtCraft Launcher");
        std::fs::create_dir_all(folder.join("ArtCraftLauncherData")).unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        std::fs::write(folder.join("ArtCraftLauncherData").join("settings.json"), "{}").unwrap();
        let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        let signed_exe = std::fs::read(system_program("cmd.exe")).unwrap();
        for (name, body) in [
            ("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", signed_exe.as_slice()),
            ("artcraft-launcher-0.2.0-windows-x64-portable/portable.txt", b"".as_slice()),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(body).unwrap();
        }
        writer.finish().unwrap();
        let tmp = dir.path().join("cache").join(PORTABLE_STAGING);
        let step = Finish::Portable { zip: zip.clone(), tmp: tmp.clone() };

        let mut old = BlockedLauncher::start();
        let pid = old.id();
        std::thread::scope(|scope| {
            // Inside the scope, so a failed assertion drops the gate and releases the helper
            // before the scope joins it.
            let (gate, wait) = WaitGate::new();
            let helper_exe = exe.clone();
            let helper = scope.spawn(move || step_after_wait(&crate::suite::LAUNCHER, &system_program("cmd.exe"), pid, &helper_exe, &step, &wait));
            gate.entered();
            old.assert_running("while the helper was waiting");
            assert_eq!(std::fs::read(&exe).unwrap(), b"0.1.0", "replaced while the old launcher was still running");
            assert!(!process::wait_for_exit(pid, std::time::Duration::ZERO), "the wait reported the running launcher as exited");
            old.finish().expect("the stand-in launcher couldn't be reaped");
            gate.release();
            let stepped = helper.join().unwrap();
            assert!(stepped.exited, "the wait reported that the launcher was still running although it had exited");
            stepped.result.unwrap();
        });
        assert_eq!(std::fs::read(&exe).unwrap(), signed_exe);
        assert_eq!(std::fs::read_to_string(folder.join("ArtCraftLauncherData").join("settings.json")).unwrap(), "{}");
        // The update's zip carries `portable.txt`, so the replaced copy is detected as portable
        // and keeps its data in the folder beside the executable.
        assert_eq!(detect(&crate::suite::LAUNCHER, &exe), Some(Install::Portable(exe.clone())));
        assert_eq!(Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap().data, folder.join("ArtCraftLauncherData").join("data"));
        assert!(!folder.join("artcraft-launcher-0.2.0-windows-x64-portable").exists(), "the zip's folder must not be nested in the launcher's");
        assert!(!zip.exists() && !tmp.exists(), "the download and the staging folder are removed");
    }

    /// The helper finishes only the launcher's own update: its own installation, with the
    /// download from its own cache. A signed launcher must not run or start anything else.
    #[cfg(windows)]
    #[test]
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

    /// The helper finds a portable launcher's folders beside it, as that launcher does.
    #[test]
    fn the_helper_uses_the_portable_launchers_own_folders() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(dir.path().join("portable.txt"), "").unwrap();
        let paths = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
        assert_eq!(paths.cache, dir.path().join("ArtCraftLauncherData").join("cache"));
        assert_eq!(paths.data, dir.path().join("ArtCraftLauncherData").join("data"));
    }

    /// The launcher quits for an update only when the helper finds the cache the update was
    /// downloaded into. One that keeps its files in the temporary folder, because its own folders
    /// couldn't be found, refuses before it quits: the helper would look in the per-user folders.
    #[cfg(windows)]
    #[test]
    fn a_launcher_whose_helper_looks_in_other_folders_does_not_quit_for_an_update() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ArtCraft Launcher");
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        let own = Paths::under(&folder.join("ArtCraftLauncherData"));
        let temporary = Paths::under(&dir.path().join("Temp").join("artcraft-launcher"));
        for paths in [&own, &temporary] {
            std::fs::create_dir_all(paths.downloads()).unwrap();
        }
        // Installed with the MSI: the helper calls `Paths::new` again.
        assert!(same_cache_as_helper(&crate::suite::LAUNCHER, &temporary, &exe).is_err_and(|e| e.contains("can't update itself")));
        std::fs::write(folder.join("portable.txt"), "").unwrap();
        assert_eq!(same_cache_as_helper(&crate::suite::LAUNCHER, &own, &exe), Ok(()));
        assert!(same_cache_as_helper(&crate::suite::LAUNCHER, &temporary, &exe).is_err());
    }

    /// An update whose swap couldn't be scheduled stays on offer, and a scheduled one is not
    /// offered again unless the helper, running alongside, records that it failed.
    #[test]
    fn only_a_scheduled_update_is_recorded_as_installed() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(&paths.data).unwrap();
        let release = Release { tag: "v0.2.0".into(), ..Release::default() };
        mark_installed(&paths, &release);
        assert!(unmark_unless_scheduled(&paths, Err::<(), _>("couldn't schedule the update".into())).is_err());
        assert!(!just_updated_to(&paths, &release));
        mark_installed(&paths, &release);
        assert!(unmark_unless_scheduled(&paths, Ok(())).is_ok());
        assert!(just_updated_to(&paths, &release));
        // The helper refuses before this process has finished scheduling it.
        mark_installed(&paths, &release);
        record_failure(&paths, "the update helper only finishes ArtCraft Launcher's own update");
        assert!(unmark_unless_scheduled(&paths, Ok(())).is_ok());
        assert!(!just_updated_to(&paths, &release), "a failure the helper recorded must stay recorded");
    }

    /// A failed update is offered again and its error is logged at the next start, once.
    #[test]
    fn a_failed_update_is_offered_again_and_reported_once() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let release = Release { tag: "v0.2.0".into(), ..Release::default() };
        std::fs::create_dir_all(&paths.data).unwrap();
        crate::paths::write_atomic(&marker(&paths), format!("v0.2.0\n{}\n", crate::now()).as_bytes()).unwrap();
        assert!(just_updated_to(&paths, &release));
        record_failure(&paths, "the Windows Installer stopped with error 1603");
        assert!(!just_updated_to(&paths, &release));
        assert_eq!(take_failure(&paths).as_deref(), Some("the Windows Installer stopped with error 1603"));
        assert_eq!(take_failure(&paths), None);
    }

    /// The outcome is read once; a failure wins over a finished update, which counts only for the
    /// version it installed.
    #[test]
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
    #[test]
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
    #[test]
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

    /// Scheduling a new update clears the outcome records of an earlier one, so a stale failure
    /// is not reported after it.
    #[test]
    fn a_new_update_clears_the_last_ones_outcome() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        record_failure(&paths, "the Windows Installer stopped with error 1603");
        mark_installed(&paths, &Release { tag: "v0.3.0".into(), ..Release::default() });
        assert_eq!(take_failure(&paths), None);
        assert_eq!(scheduled_version(&paths).as_deref(), Some("v0.3.0"));
    }

    /// A release whose tag is not its version (an app may build its own `Release`) is recorded
    /// by its version, so the new version's first start still reports the update.
    #[test]
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

    /// A launcher that doesn't quit keeps its files, and the download is still removed.
    #[cfg(windows)]
    #[test]
    fn a_launcher_that_keeps_running_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        let msi = dir.path().join("artcraft-launcher-0.2.0-windows-x64.msi");
        std::fs::write(&msi, "not a package").unwrap();
        let running = std::process::id();
        // The test process itself is still running when the wait asks, so the package is never run.
        let error =
            run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), running, &exe, &Finish::Msi(msi.clone()), std::time::Duration::ZERO)
                .unwrap_err();
        assert!(error.contains("still running"), "{error}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
        assert!(!msi.exists());
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

    /// A catalog-signed program matches itself; an unsigned one never does, on either side.
    #[cfg(windows)]
    #[test]
    fn a_file_must_be_signed_by_the_launchers_own_publisher() {
        let cmd = system_program("cmd.exe");
        // `cargo test` binaries are unsigned.
        let unsigned = std::env::current_exe().unwrap();
        assert_eq!(signed_like(&crate::suite::LAUNCHER, &cmd, &cmd), Ok(()));
        assert!(signed_like(&crate::suite::LAUNCHER, &cmd, &unsigned).is_err_and(|e| e.contains("isn't signed")));
        assert!(signed_like(&crate::suite::LAUNCHER, &unsigned, &cmd).is_err_and(|e| e.contains("doesn't install updates")));
    }

    /// An unsigned copy gets the download link instead of an update it cannot install, unless
    /// [`DEV_VAR`] asks to try the self-update.
    #[cfg(windows)]
    #[test]
    fn only_a_signed_copy_is_offered_the_self_update() {
        let dir = tempfile::tempdir().unwrap();
        let unsigned = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&unsigned, "MZ").unwrap();
        std::fs::write(dir.path().join("portable.txt"), "").unwrap();
        assert!(!can_self_update_at(&crate::suite::LAUNCHER, &unsigned, false));
        assert!(can_self_update_at(&crate::suite::LAUNCHER, &unsigned, true));
        assert!(can_self_update_at(&crate::suite::LAUNCHER, &system_program("cmd.exe"), false));
    }

    /// An unsigned launcher says so before it quits for an update. `cargo test` binaries are
    /// unsigned, like a package built without the signing secrets.
    #[cfg(windows)]
    #[test]
    fn an_unsigned_launcher_refuses_to_update_before_it_quits() {
        let dir = tempfile::tempdir().unwrap();
        for install in [Install::Msi(system_program("msiexec.exe")), Install::Portable(system_program("msiexec.exe"))] {
            let error = windows_precheck(&crate::suite::LAUNCHER, &install, &Paths::under(dir.path())).unwrap_err();
            assert!(error.contains("isn't signed"), "{error}");
        }
    }

    /// A copy that can't install an update stops before the download: the address here can't be
    /// reached, so a download attempt would give a different error.
    #[cfg(windows)]
    #[test]
    fn a_copy_that_cannot_install_downloads_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let name = "artcraft-launcher-0.2.0-windows-x64-portable.zip";
        let asset = crate::release::Asset { name: name.into(), size: 1, url: format!("http://127.0.0.1:1/{name}"), sha256: Some("00".repeat(32)) };
        let release = Release { tag: "v0.2.0".into(), version: version::parse("0.2.0"), assets: vec![asset], ..Release::default() };
        let mut phases = 0;
        let error = prepare_update(
            &crate::suite::LAUNCHER,
            &Install::Portable(system_program("msiexec.exe")),
            &release,
            Platform { os: crate::Os::Windows, arch: crate::Arch::X64 },
            &Http::new(crate::TEST_AGENT).unwrap(),
            &Paths::under(dir.path()),
            &Stop::default(),
            &mut |_| phases += 1,
            &|| Ok(()),
            None,
        )
        .unwrap_err();
        assert!(error.contains("isn't signed"), "{error}");
        assert_eq!(phases, 0, "no download phase was reported");
    }

    /// The cache may not exist yet at the first update: the check runs before the download
    /// creates it.
    #[cfg(windows)]
    #[test]
    fn the_cache_check_passes_before_the_cache_exists() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&exe, "MZ").unwrap();
        std::fs::write(dir.path().join("portable.txt"), "").unwrap();
        let paths = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
        assert!(!paths.cache.exists());
        assert_eq!(same_cache_as_helper(&crate::suite::LAUNCHER, &paths, &exe), Ok(()));
    }

    /// A cache folder that can't be created is reported with its path and error, not as a cache
    /// in the wrong place.
    #[cfg(windows)]
    #[test]
    fn a_cache_that_cannot_be_created_is_named_with_its_error() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&exe, "MZ").unwrap();
        std::fs::write(dir.path().join("portable.txt"), "").unwrap();
        let file = dir.path().join("a-file");
        std::fs::write(&file, "").unwrap();
        let mut paths = Paths::for_exe(&crate::suite::LAUNCHER, &exe).unwrap();
        paths.cache = file.join("cache");
        let error = same_cache_as_helper(&crate::suite::LAUNCHER, &paths, &exe).unwrap_err();
        assert!(error.contains("couldn't create") && error.contains(&paths.cache.display().to_string()), "{error}");
    }

    /// An MSI that isn't signed by the launcher's publisher is never handed to the Windows Installer.
    #[cfg(windows)]
    #[test]
    fn an_unsigned_msi_is_never_run() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        let msi = dir.path().join("artcraft-launcher-0.2.0-windows-x64.msi");
        std::fs::write(&msi, "not a package").unwrap();
        let error = run_step(
            &crate::suite::LAUNCHER,
            &system_program("cmd.exe"),
            exited_pid(),
            &exe,
            &Finish::Msi(msi.clone()),
            std::time::Duration::from_secs(5),
        )
        .unwrap_err();
        assert!(error.contains("isn't signed"), "{error}");
        assert!(!msi.exists());
    }

    /// A program in the portable zip that isn't signed by the launcher's publisher stops the
    /// update before any file is replaced.
    #[cfg(windows)]
    #[test]
    fn an_unsigned_program_in_the_portable_zip_is_not_installed() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("ArtCraft Launcher");
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
        let mut writer = zip::ZipWriter::new(std::fs::File::create(&zip).unwrap());
        writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/README.md", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(b"# ArtCraft Launcher").unwrap();
        writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(system_program("cmd.exe")).unwrap()).unwrap();
        // An unsigned program under a harmless name.
        writer.start_file("artcraft-launcher-0.2.0-windows-x64-portable/notes.txt", zip::write::SimpleFileOptions::default()).unwrap();
        writer.write_all(&std::fs::read(std::env::current_exe().unwrap()).unwrap()).unwrap();
        writer.finish().unwrap();
        let tmp = dir.path().join("cache").join(PORTABLE_STAGING);
        let step = Finish::Portable { zip: zip.clone(), tmp: tmp.clone() };
        let error =
            run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), exited_pid(), &exe, &step, std::time::Duration::from_secs(5)).unwrap_err();
        assert!(error.contains("isn't signed"), "{error}");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
        assert!(!folder.join("README.md").exists() && !folder.join("notes.txt").exists(), "nothing is copied");
        assert!(!zip.exists() && !tmp.exists());
    }

    /// A missing or malformed executable must be rejected before any installed file is changed.
    #[cfg(windows)]
    #[test]
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
            let error = run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), exited_pid(), &exe, &step, std::time::Duration::from_secs(5))
                .unwrap_err();
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
    #[test]
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

    #[test]
    fn a_corrupt_self_update_timestamp_does_not_overflow() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let release = Release { tag: "v0.2.0".into(), ..Release::default() };
        crate::paths::write_atomic(&marker(&paths), format!("v0.2.0\n{}\n", i64::MIN).as_bytes()).unwrap();
        assert!(!just_updated_to(&paths, &release));
    }

    #[test]
    fn a_future_self_update_timestamp_does_not_suppress_updates() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let release = Release { tag: "v0.2.0".into(), ..Release::default() };
        for at in [crate::now() + JUST_UPDATED_FOR, i64::MAX] {
            crate::paths::write_atomic(&marker(&paths), format!("v0.2.0\n{at}\n").as_bytes()).unwrap();
            assert!(!just_updated_to(&paths, &release), "future timestamp {at}");
        }
    }

    #[test]
    fn a_version_just_installed_is_not_offered_again() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let r = Release { tag: "v0.2.0".into(), ..Release::default() };
        assert!(!just_updated_to(&paths, &r));
        crate::paths::write_atomic(&marker(&paths), format!("v0.2.0\n{}\n", crate::now()).as_bytes()).unwrap();
        assert!(just_updated_to(&paths, &r));
        assert!(!just_updated_to(&paths, &Release { tag: "v0.3.0".into(), ..Release::default() }));
        // A day later the guard lapses.
        crate::paths::write_atomic(&marker(&paths), format!("v0.2.0\n{}\n", crate::now() - JUST_UPDATED_FOR - 1).as_bytes()).unwrap();
        assert!(!just_updated_to(&paths, &r));
    }

    /// The restart helper doesn't pass a pretend version on to the new launcher.
    #[cfg(unix)]
    #[test]
    fn the_restarted_launcher_does_not_inherit_a_pretend_version() {
        let cmd = helper_command(&crate::suite::LAUNCHER, 1, &[std::ffi::OsStr::new("/usr/bin/true")]);
        assert!(cmd.get_envs().any(|(k, v)| k == PRETEND_VAR && v.is_none()), "the helper must remove {PRETEND_VAR}");
    }

    #[test]
    fn restarted_apps_do_not_inherit_their_pretend_versions() {
        for app in crate::suite::SUITE.iter().chain(std::iter::once(&crate::suite::LAUNCHER)) {
            let mut cmd = std::process::Command::new("unused");
            clear_pretend_version(&mut cmd, app);
            for key in [PRETEND_VAR.to_string(), pretend_var(app)] {
                assert!(cmd.get_envs().any(|(k, v)| k == key.as_str() && v.is_none()), "{} retained {key}", app.id);
            }
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn a_custom_flatpak_identity_is_used_for_the_restart() {
        let app = App { bundle_id_override: Some("ai.artcraft.app"), ..App::new("artcraft", "ArtCraft", "Create", "storytold/artcraft") };
        let cmd = flatpak_restart_command(&app, "exec \"$@\"", "123");
        let args: Vec<_> = cmd.get_args().map(|v| v.to_string_lossy().into_owned()).collect();
        assert_eq!(args.last().map(String::as_str), Some("ai.artcraft.app"));
        assert!(!args.iter().any(|arg| arg == "ai.storyteller.artcraft"));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn flatpak_restart_unsets_persistent_pretend_version_overrides() {
        for app in &crate::suite::SUITE {
            let cmd = flatpak_restart_command(app, "exec \"$@\"", "123");
            let args: Vec<_> = cmd.get_args().map(|v| v.to_string_lossy().into_owned()).collect();
            let expected = [
                "flatpak".to_string(),
                "run".to_string(),
                format!("--unset-env={PRETEND_VAR}"),
                format!("--unset-env={}", pretend_var(app)),
                app.bundle_id(),
            ];
            assert!(args.windows(expected.len()).any(|w| w == expected), "{}: {args:?}", app.id);
        }
    }

    /// The restart helper starts only when the restart is started, at the app's exit: a close the
    /// user cancels leaves no helper counting down its one-minute wait. The test starts the helper
    /// with its standard streams set to null, as `spawn_detached` does, and a thread that waits
    /// for the helper to exit writes `exited`, so a helper that exits without writing `started`
    /// fails the test.
    #[cfg(unix)]
    #[test]
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

    /// The helper waits for the old launcher to exit before it starts the new one. The helper's
    /// `sleep` is a script on `PATH` that reports each pause through a named pipe and pauses until
    /// the test releases it, and the started command reports whether the old launcher was still
    /// running. The test holds both pipes open for reading and writing, so no open blocks, and a
    /// thread that waits for the helper to exit writes `exited`, so every read gets a line.
    #[cfg(unix)]
    #[test]
    fn the_restart_waits_for_the_old_process_to_exit() {
        use std::io::{BufRead, Write};
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let (events, release) = (dir.path().join("events"), dir.path().join("release"));
        for fifo in [&events, &release] {
            assert!(std::process::Command::new("mkfifo").arg(fifo).status().unwrap().success());
        }
        let bin = dir.path().join("bin");
        std::fs::create_dir(&bin).unwrap();
        let sleep = bin.join("sleep");
        std::fs::write(&sleep, "#!/bin/sh\nprintf 'waiting\\n' > \"$CRAFT_TEST_EVENTS\"\nread _ < \"$CRAFT_TEST_RELEASE\"\n").unwrap();
        std::fs::set_permissions(&sleep, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut path = std::ffi::OsString::from(bin.as_os_str());
        if let Some(rest) = std::env::var_os("PATH") {
            path.push(":");
            path.push(rest);
        }

        let mut old = std::process::Command::new("cat").stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).spawn().unwrap();
        let pid = old.id().to_string();
        let started = r#"if kill -0 "$1" 2>/dev/null; then s=alive; else s=gone; fi; printf 'started %s\n' "$s" > "$CRAFT_TEST_EVENTS""#;
        let command = [std::ffi::OsStr::new("/bin/sh"), "-c".as_ref(), started.as_ref(), "sh".as_ref(), pid.as_ref()];
        let open = |fifo: &Path| std::fs::OpenOptions::new().read(true).write(true).open(fifo).unwrap();
        let (events_pipe, mut release_pipe) = (open(&events), open(&release));
        let mut exited = events_pipe.try_clone().unwrap();
        let mut lines = std::io::BufReader::new(events_pipe).lines();
        let mut next = move || lines.next().unwrap().unwrap();

        let mut helper = helper_command(&crate::suite::LAUNCHER, old.id(), &command);
        helper.env("PATH", path).env("CRAFT_TEST_EVENTS", &events).env("CRAFT_TEST_RELEASE", &release);
        let mut helper = helper.stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null()).spawn().unwrap();
        std::thread::spawn(move || {
            let _ = helper.wait();
            let _ = exited.write_all(b"exited\n");
        });

        assert_eq!(next(), "waiting", "the helper didn't wait while the old launcher was still running");
        drop(old.stdin.take());
        old.wait().unwrap();
        release_pipe.write_all(b"go\n").unwrap();
        assert_eq!(next(), "started gone", "didn't start once the old launcher had exited");
        assert_eq!(next(), "exited");
    }

    /// A real disk image: the new `ArtCraft Launcher.app` replaces the old one in place.
    #[cfg(target_os = "macos")]
    #[test]
    fn the_new_bundle_replaces_the_running_one() {
        fn bundle(at: &Path, version: &str) {
            let contents = at.join("ArtCraft Launcher.app/Contents");
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
        }
        let dir = tempfile::tempdir().unwrap();
        let apps = dir.path().join("Applications");
        bundle(&apps, "0.1.0");
        let src = dir.path().join("src");
        bundle(&src, "0.2.0");
        let dmg = dir.path().join("artcraft-launcher-0.2.0-macos-universal.dmg");
        let one_at_a_time = install::macos::DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let ok = std::process::Command::new("hdiutil")
            .args(["create", "-quiet", "-format", "UDZO", "-volname", "ArtCraft Launcher 0.2.0", "-srcfolder"])
            .arg(&src)
            .arg(&dmg)
            .status()
            .unwrap()
            .success();
        assert!(ok, "hdiutil create failed");
        drop(one_at_a_time);
        let current = apps.join("ArtCraft Launcher.app");
        let existing = Installed { version: None, source: InstallSource::MacApp, location: current.display().to_string(), package: None };
        // An unsigned test bundle: the signature check has its own tests in `install`.
        let expect = install::macos::Expect { team: None, version: None, quit_first: false };
        let stop = Stop::default();
        let wait = install::macos::CheckTurn { stop: &stop, phase: &mut |_| {} };
        let placed =
            install::macos::install_dmg(&crate::suite::LAUNCHER, &dmg, Some(&existing), None, false, &Paths::under(dir.path()), &expect, wait)
                .unwrap();
        assert_eq!(placed, current);
        assert_eq!(std::fs::read_to_string(current.join("Contents/MacOS/ArtCraft Launcher")).unwrap(), "0.2.0");
        assert_eq!(
            std::fs::read_dir(&apps).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(),
            [std::ffi::OsString::from("ArtCraft Launcher.app")]
        );
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
    #[test]
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
            let error = prepare_update(&crate::suite::LAUNCHER, &install, &release, platform, &http, &paths, &stop, &mut phase, &|| Ok(()), None)
                .unwrap_err();
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

    /// An install that succeeds removes the download and its record; a failed one keeps both, and
    /// so does an update that stops before installing.
    #[cfg(not(windows))]
    #[test]
    fn only_a_failed_install_keeps_the_download() {
        assert!(!keeps_download(&Ok(())));
        assert!(keeps_download::<()>(&Err("the app isn't signed by its developer, so it wasn't installed".into())));
        assert!(keeps_download::<()>(&Err("couldn't copy the app: No space left on device".into())));

        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(paths.downloads()).unwrap();
        let asset = update_asset("artcraft-launcher-0.2.0-linux-x64.tar.gz", 5);
        let file = paths.downloads().join(&asset.name);
        std::fs::write(&file, b"image").unwrap();
        assert!(keep_download(&paths, &asset));
        after_install::<()>(&paths, &file, true, &Err("couldn't replace it".into()));
        assert!(file.exists() && kept_download_record(&paths).exists());
        after_install(&paths, &file, true, &Ok(()));
        assert!(!file.exists() && !kept_download_record(&paths).exists());
    }

    /// When hdiutil can't attach the disk image, the download is removed: an image left attached
    /// by an install that ended part way makes every attach of the same file fail.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_disk_image_that_does_not_attach_is_not_kept() {
        assert!(!keeps_download::<()>(&Err(format!("{}: hdiutil: attach failed - Resource busy", install::macos::IMAGE_NOT_OPENED))));
    }

    /// A download that could not be recorded is removed after a failed install, since nothing
    /// would remove it later.
    #[cfg(not(windows))]
    #[test]
    fn a_download_without_a_record_is_not_kept() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(paths.downloads()).unwrap();
        // A folder where the record goes makes writing it fail.
        std::fs::create_dir_all(kept_download_record(&paths).join("in-the-way")).unwrap();
        let asset = update_asset("artcraft-launcher-0.2.0-linux-x64.tar.gz", 5);
        let file = paths.downloads().join(&asset.name);
        std::fs::write(&file, b"image").unwrap();
        let recorded = keep_download(&paths, &asset);
        assert!(!recorded);
        after_install::<()>(&paths, &file, recorded, &Err("couldn't replace it".into()));
        assert!(!file.exists());
    }

    /// A self-update for another release file removes the finished and the partial download of the
    /// one the record names, and leaves other files alone. A download of the same file keeps its
    /// partial file, so it resumes.
    #[cfg(not(windows))]
    #[test]
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
    #[test]
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

    /// A directory laid out like Cargo's target tree: `.rustc_info.json` at the root and the
    /// executable below `profile` (with or without a target triple in between).
    fn cargo_build_tree(dir: &Path, root: &str, relative_exe: &str) -> PathBuf {
        let root = dir.join(root);
        let exe = root.join(relative_exe);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(root.join(".rustc_info.json"), "{}").unwrap();
        std::fs::write(&exe, "ELF").unwrap();
        exe
    }

    /// A directory laid out like a Cargo target tree whose rustc-info cache was disabled: the
    /// target root has `CACHEDIR.TAG`, but no `.rustc_info.json`. `profile` is relative to the
    /// target root, so it may include a target triple.
    fn cargo_build_tree_without_rustc_info(dir: &Path, profile: &str, exe: &str) -> PathBuf {
        let root = dir.join("target");
        let profile = root.join(profile);
        for sub in [".fingerprint", "deps", "build"] {
            std::fs::create_dir_all(profile.join(sub)).unwrap();
        }
        let exe = root.join(exe);
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(root.join(CARGO_CACHE_MARKER), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
        std::fs::write(&exe, "ELF").unwrap();
        exe
    }

    /// `CARGO_TARGET_DIR` moves the build tree; the executable in it is still a development build,
    /// whatever the profile or target triple is called.
    #[test]
    fn a_development_build_under_a_custom_target_dir_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        for exe in [
            "debug/deps/craft_update-e8899812898c870d",
            "release/artcraft-launcher",
            "x86_64-pc-windows-gnu/debug/deps/craft_update-0123456789abcdef",
            "x86_64-pc-windows-gnu/my-profile/deps/artcraft-launcher-0123456789abcdef",
        ] {
            let exe = cargo_build_tree(dir.path(), "builds/egui037-craft-libs", exe);
            assert!(is_development_build(&exe, false), "{}", exe.display());
        }
    }

    /// A Cargo build tree is not recognised by names: an installed copy in a folder literally
    /// called `target` stays an installed copy.
    #[test]
    fn an_installed_copy_in_a_folder_called_target_is_not_a_development_build() {
        let dir = tempfile::tempdir().unwrap();
        let folder = dir.path().join("target").join("ArtCraft Launcher");
        std::fs::create_dir_all(&folder).unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        std::fs::write(&exe, "MZ").unwrap();
        assert!(!is_development_build(&exe, false), "a folder named target is not a build tree");
    }

    /// `CACHEDIR.TAG` has a generic signature, used by every cache directory; without Cargo's
    /// profile layout it does not identify a development build.
    #[test]
    fn a_generic_cache_marker_is_not_a_cargo_target_dir() {
        let dir = tempfile::tempdir().unwrap();
        let cache = dir.path().join("cache");
        let exe = cache.join("ArtCraft Launcher").join("artcraft-launcher");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(cache.join("CACHEDIR.TAG"), "Signature: 8a477f597d28d172789f06886806bc55\n").unwrap();
        std::fs::write(&exe, "ELF").unwrap();
        assert!(!is_development_build(&exe, false), "a generic cache is not a Cargo build tree");
    }

    /// `CARGO_CACHE_RUSTC_INFO=0` leaves no `.rustc_info.json`; the profile layout plus the
    /// target root's `CACHEDIR.TAG` still identify a development build, whatever the profile or
    /// target triple is called.
    #[test]
    fn a_cargo_build_tree_without_rustc_info_is_detected() {
        let dir = tempfile::tempdir().unwrap();
        for (profile, exe) in [
            ("debug", "debug/deps/craft_update-e8899812898c870d"),
            ("release", "release/artcraft-launcher"),
            ("my-profile", "my-profile/artcraft-launcher"),
            ("x86_64-pc-windows-gnu/my-profile", "x86_64-pc-windows-gnu/my-profile/deps/artcraft-launcher-0123456789abcdef"),
        ] {
            let exe = cargo_build_tree_without_rustc_info(dir.path(), profile, exe);
            assert!(is_development_build(&exe, false), "{}", exe.display());
        }
    }

    /// The fallback needs the complete profile layout and the target root's cache marker; a
    /// partial layout or a missing marker isn't enough.
    #[test]
    fn an_incomplete_cargo_layout_is_not_a_development_build() {
        let dir = tempfile::tempdir().unwrap();
        for missing in [".fingerprint", "deps", "build"] {
            let exe = cargo_build_tree_without_rustc_info(dir.path(), "debug", "debug/deps/artcraft-launcher");
            std::fs::remove_dir_all(dir.path().join("target/debug").join(missing)).unwrap();
            assert!(!is_development_build(&exe, false), "missing {missing}");
        }

        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree_without_rustc_info(dir.path(), "debug", "debug/deps/artcraft-launcher");
        std::fs::remove_file(dir.path().join("target").join(CARGO_CACHE_MARKER)).unwrap();
        assert!(!is_development_build(&exe, false), "a profile layout without the target-root marker");
    }

    /// An installed copy that merely lives below a Cargo project isn't in that project's target
    /// tree, even when the project has a build tree of its own.
    #[test]
    fn an_installed_copy_beneath_a_cargo_project_is_not_a_development_build() {
        let dir = tempfile::tempdir().unwrap();
        let project = dir.path().join("project");
        std::fs::create_dir_all(project.join("src")).unwrap();
        std::fs::write(project.join("Cargo.toml"), "[package]\nname = \"installer\"\n").unwrap();
        cargo_build_tree_without_rustc_info(&project, "debug", "debug/deps/installer-0123456789abcdef");
        let exe = project.join("dist").join("artcraft-launcher");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::write(&exe, "ELF").unwrap();
        assert!(!is_development_build(&exe, false), "{}", exe.display());
    }

    /// Both markers must be files: a directory of the same name doesn't identify or corroborate
    /// a build tree.
    #[test]
    fn marker_directories_are_not_development_builds() {
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("builds").join("debug").join("deps").join("artcraft-launcher");
        std::fs::create_dir_all(exe.parent().unwrap()).unwrap();
        std::fs::create_dir_all(dir.path().join("builds").join(".rustc_info.json")).unwrap();
        std::fs::write(&exe, "ELF").unwrap();
        assert!(!is_development_build(&exe, false), "a directory named .rustc_info.json");

        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree_without_rustc_info(dir.path(), "debug", "debug/deps/artcraft-launcher");
        let root = dir.path().join("target");
        std::fs::remove_file(root.join(CARGO_CACHE_MARKER)).unwrap();
        std::fs::create_dir(root.join(CARGO_CACHE_MARKER)).unwrap();
        assert!(!is_development_build(&exe, false), "a directory named CACHEDIR.TAG");
    }

    /// With `build.build-dir` set, the profile directory holds the program and `.cargo-lock`; the
    /// lock file and the target root's `CACHEDIR.TAG` identify a development build, and neither
    /// does alone.
    #[test]
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

    /// `CRAFT_UPDATE_DEV` still lets a development build try the self-update.
    #[test]
    fn a_development_build_can_opt_in_to_updating_itself() {
        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree(dir.path(), "builds/egui037-craft-libs", "release/artcraft-launcher");
        assert!(!is_development_build(&exe, true));
    }

    /// The opt-in also covers a build only recognised through the cache-disabled fallback.
    #[test]
    fn a_development_build_without_rustc_info_can_opt_in_to_updating_itself() {
        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree_without_rustc_info(dir.path(), "release", "release/artcraft-launcher");
        assert!(!is_development_build(&exe, true));
    }

    #[test]
    fn a_development_build_does_not_update_itself() {
        // An archived test runner may relocate its executable without Cargo's metadata.
        // Supply a complete cache-disabled build tree so this checks the updater guard,
        // independent of the runner's location and the ambient `CRAFT_UPDATE_DEV` value.
        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree_without_rustc_info(dir.path(), "debug", "debug/deps/artcraft-launcher");
        assert!(is_development_build(&exe, false), "{} should be inside a Cargo target directory", exe.display());
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
    }

    /// The guard in `current_at` runs before platform detection, so a development executable is
    /// rejected without probing the ambient host.
    #[test]
    fn current_at_rejects_a_development_build_before_detection() {
        let dir = tempfile::tempdir().unwrap();
        let exe = cargo_build_tree(dir.path(), "builds/egui037-craft-libs", "debug/artcraft-launcher");
        assert_eq!(current_at(&crate::suite::LAUNCHER, &exe, false), None);
    }

    #[test]
    fn the_launcher_picks_its_own_release_files() {
        let names = [
            "artcraft-launcher-0.2.0-macos-universal.dmg",
            "artcraft-launcher-0.2.0-windows-x64.msi",
            "artcraft-launcher-0.2.0-windows-x64-portable.zip",
            "artcraft-launcher-0.2.0-linux-x86_64.AppImage",
            "artcraft-launcher-0.2.0-linux-x86_64.deb",
            "artcraft-launcher-0.2.0-linux-x86_64.flatpak",
            "artcraft-launcher-0.2.0-linux-x86_64.pkg.tar.zst",
            "artcraft-launcher-web-0.2.0.zip",
        ];
        let release = Release {
            assets: names.iter().map(|n| crate::release::Asset { name: (*n).into(), size: 1, url: String::new(), sha256: None }).collect(),
            ..Release::default()
        };
        use crate::{Arch, Os};
        let pick =
            |os, kind| pick_asset(&release, &[crate::suite::LAUNCHER.id], Platform { os, arch: Arch::X64 }, &[kind]).map(|(a, _)| a.name.clone());
        assert_eq!(pick(Os::MacOs, Install::MacApp(PathBuf::new()).kind()).as_deref(), Some(names[0]));
        assert_eq!(pick(Os::Windows, Install::Msi(PathBuf::new()).kind()).as_deref(), Some(names[1]));
        assert_eq!(pick(Os::Windows, Install::Portable(PathBuf::new()).kind()).as_deref(), Some(names[2]));
        assert_eq!(pick(Os::Linux, Install::AppImage(PathBuf::new()).kind()).as_deref(), Some(names[3]));
        assert_eq!(pick(Os::Linux, Install::Package(PackageKind::Deb, PathBuf::new()).kind()).as_deref(), Some(names[4]));
        assert_eq!(pick(Os::Linux, Install::Flatpak.kind()).as_deref(), Some(names[5]));
        assert_eq!(pick(Os::Linux, Install::Package(PackageKind::ArchPkg, PathBuf::new()).kind()).as_deref(), Some(names[6]));
    }

    fn release_with(names: &[&str]) -> Release {
        Release {
            assets: names.iter().map(|n| crate::release::Asset { name: (*n).into(), size: 1, url: String::new(), sha256: None }).collect(),
            ..Release::default()
        }
    }

    /// A macOS release carries a disk image and a command-line zip, and no `.tar.gz` that a
    /// standalone program could be replaced from, so that copy gets the download link.
    #[cfg(not(windows))]
    #[test]
    fn a_standalone_macos_program_without_a_tarball_gets_the_download_link() {
        let app = &crate::suite::SOUNDCRAFT;
        let release = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-cli-0.19.0-macos-universal.zip"]);
        let standalone = Install::Standalone(PathBuf::from("/Users/someone/bin/soundcraft"));
        for arch in [crate::Arch::Arm64, crate::Arch::X64] {
            let mac = Some(Platform { os: crate::Os::MacOs, arch });
            assert!(!installs_from(app, &standalone, &release, mac), "{arch:?}");
            // the bundle installs from the disk image
            assert!(installs_from(app, &Install::MacApp(PathBuf::from("/Applications/SoundCraft.app")), &release, mac), "{arch:?}");
        }
    }

    /// A release that adds a macOS `.tar.gz` is offered to the standalone program again.
    #[cfg(not(windows))]
    #[test]
    fn a_standalone_macos_program_is_offered_a_release_with_a_tarball() {
        let app = &crate::suite::SOUNDCRAFT;
        let standalone = Install::Standalone(PathBuf::from("/Users/someone/bin/soundcraft"));
        let mac = Some(Platform { os: crate::Os::MacOs, arch: crate::Arch::Arm64 });
        let with_tarball = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-0.19.0-macos-universal.tar.gz"]);
        assert!(installs_from(app, &standalone, &with_tarball, mac));
    }

    /// A release whose file list wasn't loaded keeps the offer: the install fetches the full
    /// release before it picks the file.
    #[test]
    fn a_standalone_macos_program_is_offered_a_release_without_its_file_list() {
        let standalone = Install::Standalone(PathBuf::from("/Users/someone/bin/soundcraft"));
        let mac = Some(Platform { os: crate::Os::MacOs, arch: crate::Arch::Arm64 });
        assert!(installs_from(&crate::suite::SOUNDCRAFT, &standalone, &release_with(&[]), mac));
    }

    /// Other platforms keep offering the update to a standalone copy, whatever the release holds.
    #[test]
    fn a_standalone_copy_elsewhere_is_not_checked_against_the_release() {
        let app = &crate::suite::SOUNDCRAFT;
        let standalone = Install::Standalone(PathBuf::from("soundcraft"));
        let empty = release_with(&[]);
        for os in [crate::Os::Linux, crate::Os::Windows, crate::Os::FreeBsd] {
            assert!(installs_from(app, &standalone, &empty, Some(Platform { os, arch: crate::Arch::X64 })), "{os:?}");
        }
        assert!(installs_from(app, &standalone, &empty, None));
    }

    /// A loose program on macOS is checked against the release from its executable's path, with
    /// or without `CRAFT_UPDATE_DEV`.
    #[cfg(target_os = "macos")]
    #[test]
    fn a_loose_macos_program_is_checked_against_the_release() {
        let app = &crate::suite::SOUNDCRAFT;
        let dir = tempfile::tempdir().unwrap();
        let exe = dir.path().join("soundcraft");
        std::fs::write(&exe, b"").unwrap();
        let mac = Some(Platform { os: crate::Os::MacOs, arch: crate::Arch::Arm64 });
        let without = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-cli-0.19.0-macos-universal.zip"]);
        let with_tarball = release_with(&["soundcraft-0.19.0-macos-universal.dmg", "soundcraft-0.19.0-macos-universal.tar.gz"]);
        for opted_in in [false, true] {
            assert!(!release_installs_at(app, &exe, opted_in, &without, mac), "opted_in {opted_in}");
            assert!(release_installs_at(app, &exe, opted_in, &with_tarball, mac), "opted_in {opted_in}");
        }
    }

    #[test]
    fn a_failed_step_that_removed_the_app_starts_nothing_and_keeps_its_message() {
        assert!(restarts(true, true, true));
        assert!(restarts(true, true, false));
        assert!(restarts(true, false, true));
        assert!(!restarts(true, false, false));
    }

    /// An app still running after the helper's wait never gets a second copy, whatever the step
    /// and the exe would allow.
    #[test]
    fn an_app_still_running_is_never_started_again() {
        for (step_succeeded, exe_exists) in [(true, true), (true, false), (false, true), (false, false)] {
            assert!(!restarts(false, step_succeeded, exe_exists), "{step_succeeded} {exe_exists}");
        }
    }

    /// A helper whose wait ends with the old app still running installs nothing, starts no
    /// program, records why, and removes the download.
    #[cfg(windows)]
    #[test]
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
        let error = finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &never_exits, &start)
            .unwrap_err();
        assert!(error.contains("still running"), "{error}");
        assert_eq!(*started.lock().unwrap(), Vec::<PathBuf>::new(), "no copy of the app may start while it still runs");
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0");
        assert!(!zip.exists(), "the download is removed");
        assert_eq!(take_failure(&paths).as_deref(), Some(error.as_str()));
    }

    /// A step that fails after the old app exited starts the old app again, as it was.
    #[cfg(windows)]
    #[test]
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

    /// When the update step fails and the old app can't be started again either, the error kept
    /// for the next start names both.
    #[cfg(windows)]
    #[test]
    fn a_failed_step_and_a_failed_restart_are_both_recorded() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let exe = dir.path().join("artcraft-launcher.exe");
        std::fs::write(&exe, "0.1.0").unwrap();
        // Not a zip, so the step fails.
        let zip = dir.path().join("artcraft-launcher-0.2.0-windows-x64-portable.zip");
        std::fs::write(&zip, "not a zip").unwrap();
        let step = Finish::Portable { zip, tmp: dir.path().join(PORTABLE_STAGING) };
        let start = |_exe: &Path| Err("couldn't start artcraft-launcher.exe (denied)".to_string());
        let exited = |_pid| true;
        let error =
            finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start).unwrap_err();
        assert!(error.ends_with("; couldn't start artcraft-launcher.exe (denied)"), "{error}");
        assert!(error.len() > "; couldn't start artcraft-launcher.exe (denied)".len(), "the step's error comes first: {error}");
        assert_eq!(take_failure(&paths).as_deref(), Some(error.as_str()));
    }

    /// The helper's result keeps the update step's error when the start fails too.
    #[test]
    fn a_failed_restart_keeps_the_step_error() {
        assert_eq!(with_restart(Ok(()), Ok(())), Ok(()));
        assert_eq!(with_restart(Err("step".into()), Ok(())), Err("step".into()));
        assert_eq!(with_restart(Ok(()), Err("start".into())), Err("start".into()));
        assert_eq!(with_restart(Err("step".into()), Err("start".into())), Err("step; start".into()));
    }

    /// A successful step starts the new app once.
    #[cfg(windows)]
    #[test]
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
        assert_eq!(
            finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start),
            Ok(())
        );
        assert_eq!(*started.lock().unwrap(), vec![(exe, signed_exe)], "the new app starts once");
        assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Installed(Version::new(0, 2, 0))), "the new version's first start says so");
    }

    /// A standalone copy updated from the portable zip becomes portable: the outcome and the
    /// record of the update go to the folders beside it, which the new app reads.
    #[cfg(windows)]
    #[test]
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
        assert_eq!(
            finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe.clone(), &step, false, &exited, &start),
            Ok(())
        );
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
            writer
                .start_file("artcraft-launcher-0.2.0-windows-x64-portable/artcraft-launcher.exe", zip::write::SimpleFileOptions::default())
                .unwrap();
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

    /// A finished step whose new app can't be started is a failure at the next start, never a
    /// finished update.
    #[cfg(windows)]
    #[test]
    fn a_failed_relaunch_is_not_reported_as_a_finished_update() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let (exe, step) = portable_update(dir.path());
        mark_installed(&paths, &Release { tag: "v0.2.0".into(), ..Release::default() });
        let start = |_: &Path| Err("couldn't start the launcher (denied)".to_owned());
        let exited = |_pid| true;
        let result = finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe, &step, false, &exited, &start);
        assert_eq!(result, Err("couldn't start the launcher (denied)".into()));
        assert_eq!(take_outcome(&paths, "0.2.0"), Some(Outcome::Failed("couldn't start the launcher (denied)".into())));
    }

    /// A helper that times out or whose step fails records no finished update.
    #[cfg(windows)]
    #[test]
    fn a_timed_out_or_failed_step_is_not_reported_as_a_finished_update() {
        for (exits, zip_ok) in [(false, true), (true, false)] {
            let dir = tempfile::tempdir().unwrap();
            let paths = Paths::under(dir.path());
            let (exe, step) = portable_update(dir.path());
            if !zip_ok && let Finish::Portable { zip, .. } = &step {
                std::fs::write(zip, "not a zip").unwrap();
            }
            mark_installed(&paths, &Release { tag: "v0.2.0".into(), ..Release::default() });
            let start = |_: &Path| Ok(());
            let wait = |_pid| exits;
            assert!(finish_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), &paths, 4242, exe, &step, false, &wait, &start).is_err());
            assert!(!done_file(&paths).exists(), "exits {exits}, zip {zip_ok}");
            assert!(matches!(take_outcome(&paths, "0.2.0"), Some(Outcome::Failed(_))), "exits {exits}, zip {zip_ok}");
        }
    }

    /// A prepared Windows restart does nothing until it is started, and runs once when it is.
    #[test]
    fn a_prepared_restart_runs_only_when_started() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicUsize, Ordering};
        let runs = Arc::new(AtomicUsize::new(0));
        let counter = Arc::clone(&runs);
        let restart = Restart::later(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(())
        });
        assert_eq!(runs.load(Ordering::SeqCst), 0, "nothing starts before the app closes");
        assert_eq!(restart.start(), Ok(()));
        assert_eq!(runs.load(Ordering::SeqCst), 1);
        assert_eq!(Restart::scheduled().start(), Ok(()));
    }

    #[test]
    fn update_refuses_to_downgrade_when_release_is_older() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let http = Http::new(crate::TEST_AGENT).unwrap();
        let mut phase = |_| ();
        let last = || Ok(());
        let stop = Stop::default();
        let release = Release {
            version: version::parse("0.1.0"),
            tag: "v0.1.0".into(),
            assets: vec![crate::release::Asset {
                name: "artcraft-launcher-0.1.0-linux-x86_64.AppImage".into(),
                size: 1,
                url: String::new(),
                sha256: None,
            }],
            ..Release::default()
        };
        let install = Install::AppImage(dir.path().join("launcher.AppImage"));
        use crate::{Arch, Os};
        let platform = Platform { os: Os::Linux, arch: Arch::X64 };
        let result = update(&crate::suite::LAUNCHER, &install, &release, platform, &http, &paths, &stop, &mut phase, &last, Some("0.2.0"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("is not newer than currently running version"));
    }

    #[test]
    fn a_build_from_main_refuses_to_install_the_release_it_already_contains() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let http = Http::new(crate::TEST_AGENT).unwrap();
        let mut phase = |_| ();
        let last = || Ok(());
        let stop = Stop::default();
        let release = Release { version: version::parse("0.2.0"), tag: "v0.2.0".into(), ..Release::default() };
        let install = Install::AppImage(dir.path().join("launcher.AppImage"));
        use crate::{Arch, Os};
        let platform = Platform { os: Os::Linux, arch: Arch::X64 };
        let running = Some("0.2.0-integration.20261029.f5b3f123");
        let result = update(&crate::suite::LAUNCHER, &install, &release, platform, &http, &paths, &stop, &mut phase, &last, running);
        assert!(result.unwrap_err().contains("is not newer than currently running version"));
    }

    /// Only `/usr/bin/<id>` is checked against the package databases, as before the launcher's
    /// self-update moved here; another copy under a system folder (an AUR build, say) is left to
    /// its package manager, and only a copy outside them (`/usr/local` included, which no package
    /// manager owns) replaces itself.
    #[test]
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
    #[test]
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

    /// A launcher release whose tag has a `+` (semver build metadata) has `%2B` in its links.
    /// The self-update accepts them and gets as far as the download, which the cancelled stop ends
    /// before anything is fetched.
    #[test]
    fn update_accepts_a_plus_release_up_to_the_download() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let http = Http::new(crate::TEST_AGENT).unwrap();
        let mut phase = |_| ();
        let last = || Ok(());
        let stop = Stop::default();
        stop.cancel();
        let app = &crate::suite::LAUNCHER;
        let name = "artcraft-launcher-0.3.0+build.1-linux-x86_64.AppImage";
        let release = crate::release::release_from_sums(app.repo, "v0.3.0+build.1", &[("a".repeat(64), name.to_string())]);
        let install = Install::AppImage(dir.path().join("launcher.AppImage"));
        use crate::{Arch, Os};
        let platform = Platform { os: Os::Linux, arch: Arch::X64 };
        let result = update(app, &install, &release, platform, &http, &paths, &stop, &mut phase, &last, Some("0.2.0"));
        assert_eq!(result.unwrap_err(), crate::protocol::CANCELLED);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_flatpak_metadata_for_restart() {
        let info = "[Application]\nname=ai.storyteller.artcraft-launcher\n\n[Instance]\ninstance-id=155661136\n";
        assert_eq!(parse_flatpak_instance_id(info), Some("155661136".to_string()));
        assert_eq!(parse_flatpak_instance_id("no instance id here"), None);

        let ps = "Instance   PID    Application\n155661136  842791 ai.storyteller.artcraft-launcher\n";
        assert_eq!(parse_flatpak_host_pid(ps, "155661136"), Some(842791));
        assert_eq!(parse_flatpak_host_pid(ps, "999999999"), None);
    }

    /// A per-user bundle and a system-wide bundle are told apart. Anything else is left alone,
    /// including `flatpak run --app-path=`, which mounts an empty /app.
    #[cfg(target_os = "linux")]
    #[test]
    fn a_flatpak_copy_is_updated_on_the_installation_it_runs_from() {
        let info = |path: &str| format!("[Application]\nname=ai.storyteller.artcraft-launcher\n\n[Instance]\ninstance-id=1\napp-path={path}\n");
        assert_eq!(
            parse_flatpak_level(&info("/home/alice/.local/share/flatpak/app/ai.storyteller.artcraft-launcher/x86_64/stable/0a3de1c/files")),
            Some(FlatpakLevel::User)
        );
        assert_eq!(
            parse_flatpak_level(&info("/var/lib/flatpak/app/ai.storyteller.artcraft-launcher/x86_64/stable/0a3de1c/files")),
            Some(FlatpakLevel::System)
        );
        assert_eq!(parse_flatpak_level(&info("")), None);
        assert_eq!(parse_flatpak_level(&info("/opt/flatpak/app/ai.storyteller.artcraft-launcher/files")), None);
        // The key belongs to [Instance] only.
        assert_eq!(parse_flatpak_level("[Application]\napp-path=/home/alice/.local/share/flatpak/app/x/files\n"), None);
        assert_eq!(parse_flatpak_level("[Instance]\ninstance-id=123\n"), None);
        assert_eq!(parse_flatpak_level(""), None);
    }

    /// A copy from a repository (Flathub or any other) isn't the bundle's to replace.
    #[cfg(target_os = "linux")]
    #[test]
    fn only_a_bundle_origin_counts_as_the_release_bundle() {
        assert!(is_bundle_origin(&crate::suite::LAUNCHER, "artcraft-launcher-origin"));
        assert!(is_bundle_origin(&crate::suite::LAUNCHER, "artcraft-launcher2-origin"));
        assert!(!is_bundle_origin(&crate::suite::LAUNCHER, "flathub"));
        assert!(!is_bundle_origin(&crate::suite::LAUNCHER, "flathub-beta"));
        assert!(!is_bundle_origin(&crate::suite::LAUNCHER, "artcraft-launcher-beta-origin"));
        assert!(!is_bundle_origin(&crate::suite::LAUNCHER, "photocraft-origin"));
        assert!(is_bundle_origin(&crate::suite::PHOTOCRAFT, "photocraft-origin"));
        assert!(!is_bundle_origin(&crate::suite::LAUNCHER, ""));
    }

    /// A release tarball like `package.sh` makes: everything under one `<app>-<ver>-<platform>/`.
    #[cfg(unix)]
    fn release_tarball(path: &Path, files: &[(&str, &[u8])]) {
        release_tarball_with(path, files, |_| {});
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
    #[test]
    fn a_standalone_update_in_a_prefix_replaces_the_cli_and_shared_files_too() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("local");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("share/pdfcraft/models")).unwrap();
        let exe = prefix.join("bin/pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        std::fs::write(prefix.join("bin/pdfcraft-cli"), b"old cli").unwrap();
        std::fs::write(prefix.join("bin/unrelated"), b"mine").unwrap();
        std::fs::write(prefix.join("share/pdfcraft/models/a.rten"), b"old model").unwrap();
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(
            &tarball,
            &[
                ("bin/pdfcraft", b"new app"),
                ("bin/pdfcraft-cli", b"new cli"),
                ("share/pdfcraft/models/a.rten", b"new model"),
                ("share/pdfcraft/models/b.rten", b"added model"),
                ("share/applications/ai.storyteller.pdfcraft.desktop", b"desktop"),
            ],
        );
        let paths = Paths::under(&dir.path().join("data"));
        replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap();
        let read = |p: &str| std::fs::read(prefix.join(p)).unwrap();
        assert_eq!(read("bin/pdfcraft"), b"new app");
        assert_eq!(read("bin/pdfcraft-cli"), b"new cli");
        assert_eq!(read("bin/unrelated"), b"mine", "files the release doesn't ship are left alone");
        assert_eq!(read("share/pdfcraft/models/a.rten"), b"new model");
        assert_eq!(read("share/pdfcraft/models/b.rten"), b"added model");
        assert_eq!(read("share/applications/ai.storyteller.pdfcraft.desktop"), b"desktop");
        let leftovers: Vec<_> = walk(&prefix).into_iter().filter(|p| p.to_string_lossy().contains(".update")).collect();
        assert!(leftovers.is_empty(), "no staged files left behind: {leftovers:?}");
        use std::os::unix::fs::PermissionsExt;
        let mode = |p: &str| std::fs::metadata(prefix.join(p)).unwrap().permissions().mode() & 0o7777;
        assert_eq!(mode("bin/pdfcraft"), 0o755);
        assert_eq!(mode("share/pdfcraft/models/a.rten") & 0o022, 0, "no group or world write from the archive");
    }

    #[cfg(unix)]
    #[test]
    fn an_unwritable_folder_stops_the_update_before_anything_changes() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("local");
        let models = prefix.join("share/pdfcraft/models");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(&models).unwrap();
        let exe = prefix.join("bin/pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        std::fs::write(prefix.join("bin/pdfcraft-cli"), b"old cli").unwrap();
        std::fs::write(prefix.join("share/pdfcraft/a.txt"), b"old data").unwrap();
        std::fs::write(models.join("a.rten"), b"old model").unwrap();
        std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(models.join("probe"), b"").is_ok() {
            return; // running as root: permissions don't stop writes, so there's nothing to test
        }
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(
            &tarball,
            &[
                ("bin/pdfcraft", b"new app"),
                ("bin/pdfcraft-cli", b"new cli"),
                ("share/pdfcraft/a.txt", b"new data"),
                ("share/pdfcraft/models/a.rten", b"new model"),
            ],
        );
        let paths = Paths::under(&dir.path().join("data"));
        let result = replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe);
        std::fs::set_permissions(&models, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(result.is_err());
        let read = |p: &str| std::fs::read(prefix.join(p)).unwrap();
        assert_eq!(read("bin/pdfcraft"), b"old app");
        assert_eq!(read("bin/pdfcraft-cli"), b"old cli");
        assert_eq!(read("share/pdfcraft/a.txt"), b"old data", "files staged before the failure aren't put in place");
        assert_eq!(read("share/pdfcraft/models/a.rten"), b"old model");
        let leftovers: Vec<_> = walk(&prefix).into_iter().filter(|p| p.to_string_lossy().contains(".update")).collect();
        assert!(leftovers.is_empty(), "staged files are removed: {leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
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
    #[test]
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
    #[test]
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
    #[test]
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

    #[cfg(unix)]
    #[test]
    fn a_folder_where_a_shipped_file_goes_stops_the_update_before_anything_changes() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("local");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("share/pdfcraft/b.txt")).unwrap();
        let exe = prefix.join("bin/pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        std::fs::write(prefix.join("share/pdfcraft/a.txt"), b"old a").unwrap();
        std::fs::write(prefix.join("share/pdfcraft/c.txt"), b"old c").unwrap();
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(
            &tarball,
            &[
                ("bin/pdfcraft", b"new app"),
                ("share/pdfcraft/a.txt", b"new a"),
                ("share/pdfcraft/b.txt", b"new b"),
                ("share/pdfcraft/c.txt", b"new c"),
            ],
        );
        let paths = Paths::under(&dir.path().join("data"));
        let result = replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe);
        assert!(result.is_err());
        let read = |p: &str| std::fs::read(prefix.join(p)).unwrap();
        assert_eq!(read("bin/pdfcraft"), b"old app");
        assert_eq!(read("share/pdfcraft/a.txt"), b"old a", "no file is replaced when one can't be");
        assert_eq!(read("share/pdfcraft/c.txt"), b"old c", "no file is replaced when one can't be");
        assert!(prefix.join("share/pdfcraft/b.txt").is_dir());
        let leftovers: Vec<_> = walk(&prefix).into_iter().filter(|p| p.to_string_lossy().contains(".update")).collect();
        assert!(leftovers.is_empty(), "staged files are removed: {leftovers:?}");
    }

    #[cfg(unix)]
    #[test]
    fn a_standalone_update_leaves_another_updates_staged_files_alone() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("local");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        let exe = prefix.join("bin/pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        // What a concurrent update would have staged under the shared names.
        let other_file = prefix.join("bin/.pdfcraft.update");
        std::fs::write(&other_file, b"other update").unwrap();
        let paths = Paths::under(&dir.path().join("data"));
        let other_dir = paths.cache.join("pdfcraft.standalone");
        std::fs::create_dir_all(&other_dir).unwrap();
        std::fs::write(other_dir.join("marker"), b"other update").unwrap();
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(&tarball, &[("bin/pdfcraft", b"new app")]);
        replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new app");
        assert_eq!(std::fs::read(&other_file).unwrap(), b"other update");
        assert_eq!(std::fs::read(other_dir.join("marker")).unwrap(), b"other update");
    }

    #[cfg(unix)]
    #[test]
    fn a_lone_standalone_binary_gets_only_what_sits_beside_it() {
        let dir = tempfile::tempdir().unwrap();
        let tools = dir.path().join("tools");
        std::fs::create_dir_all(&tools).unwrap();
        let exe = tools.join("pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(&tarball, &[("bin/pdfcraft", b"new app"), ("bin/pdfcraft-cli", b"new cli"), ("share/pdfcraft/models/a.rten", b"model")]);
        let paths = Paths::under(&dir.path().join("data"));
        replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).unwrap();
        assert_eq!(std::fs::read(&exe).unwrap(), b"new app");
        assert!(!tools.join("pdfcraft-cli").exists(), "a program that wasn't installed isn't added");
        assert!(!dir.path().join("share").exists(), "no prefix, so no shared files");
    }

    #[cfg(unix)]
    #[test]
    fn a_standalone_update_without_the_app_binary_changes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let prefix = dir.path().join("local");
        std::fs::create_dir_all(prefix.join("bin")).unwrap();
        std::fs::create_dir_all(prefix.join("share/pdfcraft")).unwrap();
        let exe = prefix.join("bin/pdfcraft");
        std::fs::write(&exe, b"old app").unwrap();
        std::fs::write(prefix.join("bin/pdfcraft-cli"), b"old cli").unwrap();
        let tarball = dir.path().join("new.tar.gz");
        release_tarball(&tarball, &[("bin/pdfcraft-cli", b"new cli")]);
        let paths = Paths::under(&dir.path().join("data"));
        assert!(replace_standalone(&crate::suite::PDFCRAFT, &paths, &tarball, &exe).is_err());
        assert_eq!(std::fs::read(&exe).unwrap(), b"old app");
        assert_eq!(std::fs::read(prefix.join("bin/pdfcraft-cli")).unwrap(), b"old cli");
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
}
