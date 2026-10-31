//! ArtCraft Launcher updating itself.
//!
//! The new version is downloaded and verified like any Craft App, then swapped in the way this
//! copy was installed. A small helper process waits for this launcher to exit, finishes anything
//! that can't happen while it runs (Windows locks a running exe), and starts the new one. The
//! launcher has to exit first anyway: it holds the single-instance lock, and a new copy started
//! while it runs would only ask it to show its window.
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
//! Anything else (FreeBSD, a development build) gets the download link instead.

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
/// environment.
fn current_at(app: &App, exe: &Path, opted_in: bool) -> Option<Install> {
    let exe = exe.canonicalize().unwrap_or_else(|_| exe.to_path_buf());
    if is_development_build(&exe, opted_in) {
        return None;
    }
    detect(app, &exe)
}

/// Cargo's cache-directory marker at the root of a target directory.
const CARGO_CACHE_MARKER: &str = "CACHEDIR.TAG";

/// Whether `exe` runs from a Cargo build tree, wherever `CARGO_TARGET_DIR` or
/// `build.target-dir` put it. Normally Cargo caches rustc information in `.rustc_info.json` at
/// the target root, and that marker alone identifies the tree; a folder that installers put the
/// app in doesn't carry it, so this doesn't depend on the directory being called `target`. But
/// the cache can be disabled (`CARGO_CACHE_RUSTC_INFO=0`) or a tree copied without the hidden
/// file, so a profile output directory is also accepted when it has Cargo's `.fingerprint`,
/// `deps` and `build` subdirectories and its target root carries `CACHEDIR.TAG`.
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
    let is_profile = [".fingerprint", "deps", "build"].into_iter().all(|name| dir.join(name).is_dir());
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

/// Whether this installed copy can update itself.
pub fn is_self_updating(app: &App) -> bool {
    current(app).is_some()
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

/// Whether the helper starts the app after the update step: always after a successful one, and
/// after a failed one only while its exe is still there.
#[cfg_attr(not(windows), allow(dead_code))]
fn restarts(step_succeeded: bool, exe_exists: bool) -> bool {
    step_succeeded || exe_exists
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
    if exe.is_file() {
        return Some(Install::Standalone(exe.to_path_buf()));
    }
    None
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
    let mut job = Job { http, paths, stop, phase, install_locations: &Default::default(), replace: false };
    let file = install::download(release, &asset, &mut job)?;
    #[cfg(windows)]
    windows_precheck(app, install, paths, &file)?;
    // The restart ends this process: nothing else may be under way.
    last()?;
    phase(JobPhase::Installing);
    // Written before the swap starts: the Windows helper runs alongside this process and clears
    // it when it fails, which a write after scheduling could undo.
    mark_installed(paths, release);
    let pid = std::process::id();
    let scheduled = (|| -> Result<(), String> {
        match install {
            #[cfg(target_os = "macos")]
            Install::MacApp(bundle) => {
                let existing = Installed { version: None, source: InstallSource::MacApp, location: bundle.display().to_string(), package: None };
                let placed = install::macos::install_dmg(app, &file, Some(&existing), None, false, paths, &install::macos::Expect::release(release));
                let _ = std::fs::remove_file(&file);
                let placed = placed?;
                relaunch_after_exit(app, pid, &[Path::new("/usr/bin/open").as_os_str(), placed.as_os_str()])
            }
            #[cfg(target_os = "linux")]
            Install::AppImage(path) => {
                replace_appimage(&file, path)?;
                let _ = std::fs::remove_file(&file);
                relaunch_after_exit(app, pid, &[path.as_os_str()])
            }
            #[cfg(target_os = "linux")]
            Install::Package(kind, exe) => {
                let result = install::linux::install_package(*kind, &file, stop, phase);
                let _ = std::fs::remove_file(&file);
                result?;
                relaunch_after_exit(app, pid, &[exe.as_os_str()])
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
                let result = install::linux::install_flatpak_bundle(&file, system, stop, phase);
                let _ = std::fs::remove_file(&file);
                result?;
                relaunch_flatpak_after_exit(app)
            }
            #[cfg(windows)]
            Install::Msi(exe) => windows_helper(app, paths, pid, exe, &msi_finish(&file)?),
            #[cfg(windows)]
            Install::Portable(exe) => {
                exe.parent().ok_or_else(|| format!("{} isn't in a folder", app.name))?;
                windows_helper(app, paths, pid, exe, &Finish::Portable { zip: file, tmp: paths.cache.join(PORTABLE_STAGING) })
            }
            #[cfg(windows)]
            Install::Standalone(exe) => {
                exe.parent().ok_or_else(|| format!("{} isn't in a folder", app.name))?;
                windows_helper(app, paths, pid, exe, &Finish::Portable { zip: file, tmp: paths.cache.join(PORTABLE_STAGING) })
            }
            #[cfg(unix)]
            Install::Standalone(exe) => {
                let result = replace_standalone(app, &file, exe);
                let _ = std::fs::remove_file(&file);
                result?;
                relaunch_after_exit(app, pid, &[exe.as_os_str()])
            }
            #[allow(unreachable_patterns)]
            _ => {
                let _ = (pid, &file);
                Err(format!("this copy of {} can't update itself; download the new version instead", app.name))
            }
        }
    })();
    unmark_unless_scheduled(paths, scheduled)
}

/// Records `release` as just installed, so it isn't offered again for [`JUST_UPDATED_FOR`].
fn mark_installed(paths: &Paths, release: &Release) {
    if let Err(e) = crate::paths::write_atomic(&marker(paths), format!("{}\n{}\n", release.tag, crate::now()).as_bytes()) {
        log::warn!("couldn't record the self-update: {e}");
    }
}

/// Removes the record of [`mark_installed`] when the swap couldn't be scheduled, so the update
/// stays on offer.
fn unmark_unless_scheduled(paths: &Paths, scheduled: Result<(), String>) -> Result<(), String> {
    if scheduled.is_err() {
        let _ = std::fs::remove_file(marker(paths));
    }
    scheduled
}

/// Copies the new AppImage beside the running one and renames it over it. The running copy is
/// mounted from its own (now unlinked) file, so it keeps working until it exits.
#[cfg(target_os = "linux")]
fn replace_appimage(new: &Path, path: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = path.parent().ok_or("the AppImage isn't in a folder")?;
    let staged = dir.join(format!(".{}.update", path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()));
    std::fs::copy(new, &staged).map_err(|e| format!("couldn't write next to {} ({e})", path.display()))?;
    // The replacement AppImage must be executable; 0o755 is not group/world-writable.
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?; // NOSONAR
    std::fs::rename(&staged, path).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("couldn't replace {} ({e})", path.display())
    })
}

/// Extracts the app's binary from a release tarball and atomically renames it over the running
/// standalone executable.
#[cfg(unix)]
fn replace_standalone(app: &App, tarball: &Path, exe: &Path) -> Result<(), String> {
    use std::os::unix::fs::PermissionsExt;
    let dir = exe.parent().ok_or_else(|| "executable isn't in a folder".to_string())?;
    let f = std::fs::File::open(tarball).map_err(|e| format!("could not open {}: {e}", tarball.display()))?;
    let gz = flate2::read::GzDecoder::new(f);
    let mut archive = tar::Archive::new(gz);
    let staged = dir.join(format!(".{}.update", exe.file_name().and_then(|n| n.to_str()).unwrap_or("app")));
    let bin_suffix = format!("/{}", app.id);
    let bin_rel = format!("bin/{}", app.id);
    let mut found = false;
    for entry in archive.entries().map_err(|e| format!("could not read archive entries: {e}"))? {
        let mut entry = entry.map_err(|e| format!("archive entry error: {e}"))?;
        if !entry.header().entry_type().is_file() {
            continue;
        }
        let path = entry.path().map_err(|e| format!("archive path error: {e}"))?;
        let path_str = path.to_string_lossy();
        if path_str == bin_rel || path_str.ends_with(&bin_suffix) || path.file_name().is_some_and(|n| n == app.id) {
            let mut out = std::fs::File::create(&staged).map_err(|e| format!("could not create {}: {e}", staged.display()))?;
            std::io::copy(&mut entry, &mut out).map_err(|e| format!("could not extract binary: {e}"))?;
            found = true;
            break;
        }
    }
    if !found {
        let _ = std::fs::remove_file(&staged);
        return Err(format!("release archive did not contain binary for {}", app.id));
    }
    std::fs::set_permissions(&staged, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?;
    std::fs::rename(&staged, exe).map_err(|e| {
        let _ = std::fs::remove_file(&staged);
        format!("could not replace {}: {e}", exe.display())
    })
}

/// Starts `sh`, which waits for `pid` to exit and then runs `command`. Detached, so it outlives us.
#[cfg(unix)]
fn relaunch_after_exit(app: &App, pid: u32, command: &[&std::ffi::OsStr]) -> Result<(), String> {
    process::spawn_detached(helper_command(app, pid, command)).map_err(|e| format!("couldn't schedule the restart ({e})"))
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

/// Starts a host helper that waits for this Flatpak instance to exit, then relaunches it via Flatpak on the host.
#[cfg(target_os = "linux")]
fn relaunch_flatpak_after_exit(app: &App) -> Result<(), String> {
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
    process::spawn_detached(flatpak_restart_command(app, &script, &target_arg)).map_err(|e| format!("couldn't schedule the Flatpak restart ({e})"))
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

/// Copies this program into the cache and starts the copy as the update helper, which waits for
/// `pid` to exit, does `finish` and starts `exe`. Run from the cache, the helper doesn't hold the
/// installed exe that the step replaces.
#[cfg(windows)]
fn windows_helper(app: &App, paths: &Paths, pid: u32, exe: &Path, finish: &Finish) -> Result<(), String> {
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
    process::spawn_detached(cmd).map_err(|e| format!("couldn't schedule the update ({e})"))
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
    let done = run_step(app, &me, pid, &exe, &step, std::time::Duration::from_secs(60));
    if let Err(e) = &done {
        record_failure(&paths, e);
    }
    // The new app, or the old one when the step failed.
    let exe = if done.is_ok() { exe_after_msi(app, registered, &exe) } else { exe };
    // A failed move can leave no program to start; its recorded error already says how to get
    // the app back, and a restart error must not replace it.
    if !restarts(done.is_ok(), exe.is_file()) {
        return Some(done);
    }
    let started = signed_like(app, &me, &exe)
        .and_then(|()| process::spawn_detached(std::process::Command::new(&exe)).map_err(|e| format!("couldn't start {} ({e})", exe.display())));
    if let Err(e) = &started {
        record_failure(&paths, e);
    }
    Some(done.and(started))
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

/// Records a failed update: the next check offers the update again, and the next start logs `error`.
#[cfg_attr(not(windows), allow(dead_code))]
fn record_failure(paths: &Paths, error: &str) {
    let _ = std::fs::remove_file(marker(paths));
    let _ = std::fs::create_dir_all(paths.logs());
    let _ = crate::paths::write_atomic(&failure_file(paths), error.as_bytes());
}

/// The error of the last update, if it failed, once.
pub fn take_failure(paths: &Paths) -> Option<String> {
    let file = failure_file(paths);
    let error = std::fs::read_to_string(&file).ok()?;
    let _ = std::fs::remove_file(&file);
    Some(error)
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

/// Checks, while the app still runs and can show the error, what the helper checks before
/// it installs: this program, the installed exe and the new MSI are signed by one publisher, and
/// the helper finds the cache in `paths` that holds its copy, the download and the staging folder.
#[cfg(windows)]
fn windows_precheck(app: &App, install: &Install, paths: &Paths, download: &Path) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| format!("couldn't find this program ({e})"))?;
    match install {
        Install::Msi(exe) => signed_like(app, &me, exe)
            .and_then(|()| msi_step(app, exe).map(drop))
            .and_then(|()| signed_like(app, &me, download))
            .and_then(|()| own_executable(app, exe))
            .and_then(|()| same_cache_as_helper(app, paths, exe)),
        Install::Portable(exe) | Install::Standalone(exe) => {
            signed_like(app, &me, exe).and_then(|()| own_executable(app, exe)).and_then(|()| same_cache_as_helper(app, paths, exe))
        }
        _ => Ok(()),
    }
}

/// Accepts `paths` when the update helper, which finds the app's folders again from `exe`
/// ([`Paths::for_exe`]), finds the same cache. Refuses an app that keeps its files in the
/// temporary folder because its own folders couldn't be found.
#[cfg(windows)]
fn same_cache_as_helper(app: &App, paths: &Paths, exe: &Path) -> Result<(), String> {
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

/// Lets a Windows test observe both sides of `run_step`'s wait for the old launcher. Compiled only
/// for tests, so production behavior is unchanged.
#[cfg(all(test, windows))]
mod wait_watch {
    use std::sync::mpsc::{self, Receiver, Sender};
    use std::sync::{Mutex, PoisonError};

    /// A test's two observations of one wait, keyed by the launcher's pid.
    pub(super) struct Channels {
        /// Sent just before `run_step` calls the wait.
        pub(super) entered: Receiver<()>,
        /// Sent right after the wait returns, before any extraction, and carries the wait's result.
        pub(super) returned: Receiver<bool>,
    }

    struct Watch {
        pid: u32,
        entered: Option<Sender<()>>,
        returned: Option<Sender<bool>>,
    }

    static WATCH: Mutex<Option<Watch>> = Mutex::new(None);

    pub(super) fn watch(pid: u32) -> Channels {
        let (entered, entered_rx) = mpsc::channel();
        let (returned, returned_rx) = mpsc::channel();
        *WATCH.lock().unwrap_or_else(PoisonError::into_inner) = Some(Watch { pid, entered: Some(entered), returned: Some(returned) });
        Channels { entered: entered_rx, returned: returned_rx }
    }

    /// Notes that `run_step` is about to wait. Only the first call for a watched pid is kept.
    pub(super) fn entered(pid: u32) {
        let mut watch = WATCH.lock().unwrap_or_else(PoisonError::into_inner);
        let sender = watch.as_mut().filter(|watch| watch.pid == pid).and_then(|watch| watch.entered.take());
        if let Some(sender) = sender {
            let _ = sender.send(());
        }
    }

    /// Notes the wait's result as soon as it returns; the step hasn't run yet.
    pub(super) fn returned(pid: u32, waited: bool) {
        let mut watch = WATCH.lock().unwrap_or_else(PoisonError::into_inner);
        let sender = watch.as_mut().filter(|watch| watch.pid == pid).and_then(|watch| watch.returned.take());
        if let Some(sender) = sender {
            let _ = sender.send(waited);
        }
    }
}

/// Waits up to `wait` for `pid` to exit, does `step` once it has, and removes the download either
/// way. The MSI, and every program in the portable zip, must be signed like `reference`.
#[cfg(windows)]
fn run_step(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: std::time::Duration) -> Result<(), String> {
    #[cfg(test)]
    wait_watch::entered(pid);
    // The test hook needs the result, so it is stored rather than consumed by the branch.
    let waited = process::wait_for_exit(pid, wait);
    #[cfg(test)]
    wait_watch::returned(pid, waited);
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
    result
}

#[cfg(test)]
mod tests {
    use super::*;

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

        let mut old = std::process::Command::new("ping").args(["-n", "3", "127.0.0.1"]).stdout(std::process::Stdio::null()).spawn().unwrap();
        let exe = folder.join("artcraft-launcher.exe");
        let helper = {
            let (exe, step, pid) = (exe.clone(), step.clone(), old.id());
            std::thread::spawn(move || {
                run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), pid, &exe, &step, std::time::Duration::from_secs(30))
            })
        };
        std::thread::sleep(std::time::Duration::from_millis(500));
        assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0", "replaced while the old launcher was still running");
        old.wait().unwrap();
        helper.join().unwrap().unwrap();
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

    /// A launcher without `portable.txt` is a standalone installation, and the helper still
    /// replaces its own executable after it exits. The test holds a stand-in launcher alive
    /// through a pipe and observes both sides of the helper's wait: the first assertion runs only
    /// after the helper announces it is about to wait. A wait that returns while the stand-in is alive
    /// fails the test before any extraction can replace the executable.
    ///
    /// The "still blocked" check is a bounded observation, not proof: a helper descheduled for the
    /// whole window could hide a missing wait until the release below. A premature timeout longer
    /// than this observation window is also outside this test's coverage.
    #[cfg(windows)]
    #[test]
    fn a_standalone_update_replaces_the_launcher_after_it_exits() {
        use std::io::Write;
        use std::sync::mpsc::RecvTimeoutError;
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

        // Timeouts only bound the test; each failure names the observation that never arrived.
        let entered_timeout = std::time::Duration::from_secs(10);
        let blocked_window = std::time::Duration::from_secs(2);
        let returned_timeout = std::time::Duration::from_secs(30);

        std::thread::scope(|scope| {
            let mut old = BlockedLauncher::start();
            let pid = old.id();
            let watch = wait_watch::watch(pid);
            let helper_exe = exe.clone();
            let step = step.clone();
            let helper = scope.spawn(move || {
                run_step(&crate::suite::LAUNCHER, &system_program("cmd.exe"), pid, &helper_exe, &step, std::time::Duration::from_secs(30))
            });
            match watch.entered.recv_timeout(entered_timeout) {
                Ok(()) => {}
                Err(RecvTimeoutError::Timeout) => {
                    panic!("the helper didn't reach its wait within {entered_timeout:?} (helper already finished: {})", helper.is_finished())
                }
                Err(RecvTimeoutError::Disconnected) => panic!("the entered channel disconnected before the helper reached its wait"),
            }
            old.assert_running("before the first replacement check");
            assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0", "replaced while the old launcher was still running");

            // A correct wait can't return while the stand-in holds the pipe.
            match watch.returned.recv_timeout(blocked_window) {
                Err(RecvTimeoutError::Timeout) => {}
                Ok(waited) => panic!("run_step left its wait while the old launcher was still running (waited: {waited})"),
                Err(RecvTimeoutError::Disconnected) => panic!("the returned channel disconnected while the old launcher was still running"),
            }
            old.assert_running("after the wait was expected to stay blocked");
            assert_eq!(std::fs::read_to_string(&exe).unwrap(), "0.1.0", "replaced while the old launcher was still running");

            old.finish().expect("the stand-in launcher couldn't be reaped");
            match watch.returned.recv_timeout(returned_timeout) {
                Ok(waited) => assert!(waited, "the wait reported that the launcher was still running although it had exited"),
                Err(RecvTimeoutError::Timeout) => panic!(
                    "the helper didn't leave its wait within {returned_timeout:?} after the launcher exited (helper already finished: {})",
                    helper.is_finished()
                ),
                Err(RecvTimeoutError::Disconnected) => panic!("the returned channel disconnected before reporting the wait's result"),
            }
            helper.join().unwrap().unwrap();
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
        assert!(unmark_unless_scheduled(&paths, Err("couldn't schedule the update".into())).is_err());
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
        let started = std::time::Instant::now();
        // The test process itself never exits within the wait, so the package is never run.
        let error = run_step(
            &crate::suite::LAUNCHER,
            &system_program("cmd.exe"),
            running,
            &exe,
            &Finish::Msi(msi.clone()),
            std::time::Duration::from_millis(300),
        )
        .unwrap_err();
        assert!(error.contains("still running"), "{error}");
        assert!(started.elapsed() < std::time::Duration::from_secs(30));
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

    /// An unsigned launcher says so before it quits for an update. `cargo test` binaries are
    /// unsigned, like a package built without the signing secrets.
    #[cfg(windows)]
    #[test]
    fn an_unsigned_launcher_refuses_to_update_before_it_quits() {
        let dir = tempfile::tempdir().unwrap();
        let msi = dir.path().join("artcraft-launcher-0.2.0-windows-x64.msi");
        std::fs::write(&msi, "not a package").unwrap();
        for install in [Install::Msi(system_program("msiexec.exe")), Install::Portable(system_program("msiexec.exe"))] {
            let error = windows_precheck(&crate::suite::LAUNCHER, &install, &Paths::under(dir.path()), &msi).unwrap_err();
            assert!(error.contains("isn't signed"), "{error}");
        }
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

    /// The helper waits for the old launcher to exit before it starts the new one.
    #[cfg(unix)]
    #[test]
    fn the_restart_waits_for_the_old_process_to_exit() {
        let dir = tempfile::tempdir().unwrap();
        let marker = dir.path().join("started");
        let mut old = std::process::Command::new("sleep").arg("1").spawn().unwrap();
        relaunch_after_exit(&crate::suite::LAUNCHER, old.id(), &[std::ffi::OsStr::new("touch"), marker.as_os_str()]).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(400));
        assert!(!marker.exists(), "started while the old launcher was still running");
        old.wait().unwrap();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while !marker.exists() && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert!(marker.exists(), "never started after the old launcher exited");
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
        let expect = install::macos::Expect { team: None, version: None };
        let placed =
            install::macos::install_dmg(&crate::suite::LAUNCHER, &dmg, Some(&existing), None, false, &Paths::under(dir.path()), &expect).unwrap();
        assert_eq!(placed, current);
        assert_eq!(std::fs::read_to_string(current.join("Contents/MacOS/ArtCraft Launcher")).unwrap(), "0.2.0");
        assert_eq!(
            std::fs::read_dir(&apps).unwrap().map(|e| e.unwrap().file_name()).collect::<Vec<_>>(),
            [std::ffi::OsString::from("ArtCraft Launcher.app")]
        );
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

    #[test]
    fn a_failed_step_that_removed_the_app_starts_nothing_and_keeps_its_message() {
        assert!(restarts(true, true));
        assert!(restarts(true, false));
        assert!(restarts(false, true));
        assert!(!restarts(false, false));
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
}
