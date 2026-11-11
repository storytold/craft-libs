//! Which install this launcher is, its version and whether it can update itself.

#[cfg(windows)]
use super::helper::is_own_executable;
use super::*;

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

pub(super) fn pretend_var(app: &App) -> String {
    format!("{}_PRETEND_VERSION", app.id.replace('-', "_").to_uppercase())
}

/// A pretend version lasts one run, including when an update schedules a restart.
pub(super) fn clear_pretend_version(cmd: &mut std::process::Command, app: &App) {
    cmd.env_remove(PRETEND_VAR).env_remove(pretend_var(app));
}

/// How long a just-installed version stays exempt from being offered again.
pub(super) const JUST_UPDATED_FOR: i64 = 24 * 3600;

/// Whether `release` is the version this app just updated itself to. Offering it again would
/// loop: update, restart, "update available", … That happens if a release's binary reports a
/// different version from its tag, or while a pretend version is set.
pub fn just_updated_to(paths: &Paths, release: &Release) -> bool {
    let Ok(text) = std::fs::read_to_string(marker(paths)) else { return false };
    let mut lines = text.lines();
    let (Some(tag), Some(at)) = (lines.next(), lines.next().and_then(|a| a.parse::<i64>().ok())) else { return false };
    tag == release.tag && crate::now().checked_sub(at).is_some_and(|age| (0..JUST_UPDATED_FOR).contains(&age))
}

pub(super) fn marker(paths: &Paths) -> PathBuf {
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
pub(super) fn current_at(app: &App, exe: &Path, opted_in: bool) -> Option<Install> {
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    if is_development_build(&exe, opted_in) {
        return None;
    }
    detect(app, &exe).filter(|install| opted_in || can_replace_all(app, install))
}

/// Whether the update can replace `install` and, for a standalone copy, the `share` files it
/// replaces along with the program.
pub(super) fn can_replace_all(app: &App, install: &Install) -> bool {
    can_replace(install) && !matches!(install, Install::Standalone(file) if !can_replace_share(app, file))
}

/// Cargo's cache-directory marker at the root of a target directory.
pub(super) const CARGO_CACHE_MARKER: &str = "CACHEDIR.TAG";

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
pub(super) fn is_development_build(exe: &Path, opted_in: bool) -> bool {
    !opted_in && exe.ancestors().skip(1).any(is_cargo_build_dir)
}

/// Whether `dir` is a Cargo target root (by its rustc-info cache marker) or a profile output
/// directory inside one (by the cache-disabled fallback).
pub(super) fn is_cargo_build_dir(dir: &Path) -> bool {
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
pub(super) fn can_self_update_at(app: &App, exe: &Path, opted_in: bool) -> bool {
    current_at(app, exe, opted_in).is_some() && (opted_in || is_signed(exe))
}

/// Whether `exe` carries a valid Authenticode signature.
#[cfg(windows)]
pub(super) fn is_signed(exe: &Path) -> bool {
    verifysign::verify_file(exe).is_ok()
}

/// Only Windows refuses an unsigned copy's update.
#[cfg(not(windows))]
pub(super) fn is_signed(_exe: &Path) -> bool {
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
pub(super) fn release_installs_at(app: &App, exe: &Path, opted_in: bool, release: &Release, platform: Option<Platform>) -> bool {
    current_at(app, exe, opted_in).is_none_or(|install| installs_from(app, &install, release, platform))
}

/// Whether the update can install `install` from `release` on `platform`. Only a standalone
/// program on macOS is checked against the release's files. A release without its file list
/// passes: `updater::install` fetches the full release before it picks the file.
pub(super) fn installs_from(app: &App, install: &Install, release: &Release, platform: Option<Platform>) -> bool {
    match (install, platform) {
        (Install::Standalone(_), Some(platform)) if platform.os == crate::Os::MacOs => {
            release.assets.is_empty() || pick_asset(release, &[app.id], platform, &[install.kind()]).is_some()
        }
        _ => true,
    }
}

/// Whether `exe` sits in a folder a macOS package manager owns (Homebrew, MacPorts, Nix).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn macos_package_manager_owns(exe: &Path) -> bool {
    // Homebrew (both prefixes), MacPorts and the Nix store; component-wise, so `/opt/homebrewed` is not one
    ["/opt/homebrew", "/usr/local/Cellar", "/usr/local/Homebrew", "/usr/local/opt", "/opt/local", "/nix"].iter().any(|root| exe.starts_with(root))
}

/// Whether the update can replace `install`, for the kinds it replaces in place from this
/// process.
#[cfg(target_os = "macos")]
pub(super) fn can_replace(install: &Install) -> bool {
    match install {
        Install::MacApp(bundle) => can_replace_bundle(bundle),
        Install::Standalone(file) => can_replace_file(file),
        _ => true,
    }
}

/// Whether the update can replace `install`, for the kinds it replaces in place from this
/// process.
#[cfg(target_os = "linux")]
pub(super) fn can_replace(install: &Install) -> bool {
    match install {
        Install::AppImage(file) | Install::Standalone(file) => can_replace_file(file),
        _ => true,
    }
}

/// Other platforms install from the helper or a package manager, or have no self-update.
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
pub(super) fn can_replace(_install: &Install) -> bool {
    true
}

/// Whether `access(2)` refuses `wanted` on `path`. Only a clear refusal counts: a check that
/// fails for another reason leaves the update on offer, and the install reports any failure.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn refused(path: &Path, wanted: rustix::fs::Access) -> bool {
    use rustix::io::Errno;

    matches!(rustix::fs::access(path, wanted), Err(Errno::ACCESS | Errno::PERM | Errno::ROFS))
}

/// Whether the update can replace the file at `file`. It writes the new file beside it and
/// renames that over it, which needs permission to write and search the folder. In a folder with
/// the sticky bit, the rename also needs the user to own the folder or the file.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn can_replace_file(file: &Path) -> bool {
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
pub(super) fn sticky_refuses(mode: u32, folder_owner: u32, file_owner: u32, user: u32) -> bool {
    mode & 0o1000 != 0 && user != 0 && folder_owner != user && file_owner != user
}

/// The prefix of a program in `bin`, when `bin` is a folder named `bin` with the app's
/// `share/<id>` folder next to it. The update then also replaces the release's `share` files
/// under that prefix.
#[cfg_attr(not(unix), allow(dead_code))]
pub(super) fn prefix_with_share<'a>(app: &App, bin: &'a Path) -> Option<&'a Path> {
    bin.parent().filter(|prefix| bin.file_name().is_some_and(|n| n == "bin") && prefix.join("share").join(app.id).is_dir())
}

/// Whether the update can replace the other files of the standalone copy at `file`: the app's
/// other programs beside it, and its `share` files in the folders a release writes to. A copy
/// with no `share/<id>` folder has no such `share` files.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn can_replace_share(app: &App, file: &Path) -> bool {
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
pub(super) fn is_app_program(app: &App, path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    name == app.id || name.strip_prefix(app.id).is_some_and(|rest| rest.starts_with('-') && !rest.contains('.'))
}

/// Whether `path` under `share` is one of the app's files: anything in `<id>` or `doc/<id>`, and
/// elsewhere a file named `<id>.<ext>` or `<bundle id>.<ext>`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub(super) fn is_share_file(app: &App, share: &Path, path: &Path) -> bool {
    let name = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    path.starts_with(share.join(app.id))
        || path.starts_with(share.join("doc").join(app.id))
        || [app.id.to_string(), app.bundle_id()].iter().any(|stem| name.strip_prefix(stem.as_str()).is_some_and(|rest| rest.starts_with('.')))
}

/// Whether `folder` has the sticky bit and holds a file that `replaced` accepts and that the
/// user can't rename over (see [`sticky_refuses`]). At most [`SHARE_ENTRY_LIMIT`] entries are
/// read. A folder or entry that can't be read doesn't refuse.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn sticky_folder_refuses(folder: &Path, replaced: impl Fn(&Path) -> bool) -> bool {
    sticky_folder_refuses_for(folder, replaced, rustix::process::geteuid().as_raw())
}

/// [`sticky_folder_refuses`] for the user with id `user`.
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub(super) fn sticky_folder_refuses_for(folder: &Path, replaced: impl Fn(&Path) -> bool, user: u32) -> bool {
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
pub(super) const SHARE_ENTRY_LIMIT: usize = 4096;

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
pub(super) fn share_folders(app: &App, share: &Path) -> Vec<PathBuf> {
    share_folders_within(app, share, SHARE_ENTRY_LIMIT)
}

/// [`share_folders`], reading at most `limit` entries in each of the app's folder, `doc/<id>`
/// and `icons/hicolor`.
#[cfg_attr(not(any(target_os = "macos", target_os = "linux")), allow(dead_code))]
pub(super) fn share_folders_within(app: &App, share: &Path, limit: usize) -> Vec<PathBuf> {
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
pub(super) fn walk_folders(root: PathBuf, limit: usize, folders: &mut Vec<PathBuf>) {
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
pub(super) fn can_replace_share(_app: &App, _file: &Path) -> bool {
    true
}

/// `access(2)`'s `_APPEND_OK`: on a directory, permission to add a subdirectory.
#[cfg(target_os = "macos")]
pub(super) const ADD_SUBDIRECTORY_OK: rustix::fs::Access = rustix::fs::Access::from_bits_retain(1 << 13);

/// `access(2)`'s `_DELETE_OK`: permission to remove the entry from its folder.
#[cfg(target_os = "macos")]
pub(super) const DELETE_OK: rustix::fs::Access = rustix::fs::Access::from_bits_retain(1 << 12);

/// Whether the update can replace the bundle at `bundle`. It creates a staging folder beside the
/// bundle and renames the bundle into another folder, which needs permission to add a
/// subdirectory to the parent and permission to move the bundle itself.
#[cfg(target_os = "macos")]
pub(super) fn can_replace_bundle(bundle: &Path) -> bool {
    let Some(parent) = bundle.parent() else { return true };
    !is_translocated(bundle) && !refused(parent, ADD_SUBDIRECTORY_OK) && !refused(bundle, ADD_SUBDIRECTORY_OK | DELETE_OK)
}

/// Whether `bundle` sits in an App Translocation folder under the system's per-user folders.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub(super) fn is_translocated(bundle: &Path) -> bool {
    bundle.starts_with("/private/var/folders") && bundle.components().any(|part| part.as_os_str() == "AppTranslocation")
}

/// Whether `exe` sits in a folder a Windows package manager owns (scoop, winget, Chocolatey, the
/// Store).
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn windows_package_manager_owns(exe: &Path) -> bool {
    // Compared as text so the rule reads the same wherever it is tested: either separator, any case.
    let text = exe.to_string_lossy().to_lowercase();
    let parts: Vec<&str> = text.split(['\\', '/']).collect();
    parts.contains(&"windowsapps")
        || parts.windows(2).any(|w| matches!((w[0], w[1]), ("scoop", "apps" | "shims") | ("winget", "packages") | ("chocolatey", "lib" | "bin")))
}

#[cfg(target_os = "macos")]
pub(super) fn detect(app: &App, exe: &Path) -> Option<Install> {
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
pub(super) fn detect(app: &App, exe: &Path) -> Option<Install> {
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
pub(super) fn msi_step(app: &App, exe: &Path) -> Result<crate::msi_migrate::Step, String> {
    crate::msi_migrate::windows::step(app, exe)
}

/// Whether the helper starts the app after the update step: never while the old app is still
/// running (a second copy of it would open), always after a successful step, and after a failed
/// one only while its exe is still there.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn restarts(app_exited: bool, step_succeeded: bool, exe_exists: bool) -> bool {
    app_exited && (step_succeeded || exe_exists)
}

/// The exe to start after an MSI update of a registered copy: the one its registrations name now
/// (the package may have moved an exact old default folder), else `exe`.
#[cfg(windows)]
pub(super) fn exe_after_msi(app: &App, registered: bool, exe: &Path) -> PathBuf {
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
pub(super) fn detect(app: &App, exe: &Path) -> Option<Install> {
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
pub(super) enum LinuxLocation {
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
pub(super) fn linux_location(app: &App, exe: &Path) -> LinuxLocation {
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
pub(super) enum FlatpakLevel {
    User,
    System,
}

#[cfg(target_os = "linux")]
pub(super) fn flatpak_level() -> Option<FlatpakLevel> {
    let info = std::fs::read_to_string("/.flatpak-info").ok()?;
    parse_flatpak_level(&info)
}

#[cfg(target_os = "linux")]
pub(super) fn parse_flatpak_level(info: &str) -> Option<FlatpakLevel> {
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
pub(super) fn from_bundle(app: &App, level: FlatpakLevel) -> bool {
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
pub(super) fn is_bundle_origin(app: &App, origin: &str) -> bool {
    origin.strip_prefix(app.id).and_then(|rest| rest.strip_suffix("-origin")).is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
}

#[cfg(not(any(target_os = "macos", windows, target_os = "linux")))]
pub(super) fn detect(_app: &App, _exe: &Path) -> Option<Install> {
    None
}
