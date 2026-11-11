//! Installing, updating, removing and starting Craft Apps.
//!
//! Apps go where their platform keeps applications, installed the platform's own way:
//!
//! - macOS: the signed `.app` from the release DMG into `/Applications`.
//! - Windows: the release MSI (Windows Installer) into `C:\Program Files\<App>`.
//! - Linux: the distro's own package (`.deb` through apt on the Debian family, `.rpm` through
//!   dnf/zypper/rpm on the Fedora, Red Hat and SUSE families, read from `os-release`) with the
//!   system's polkit password prompt; otherwise a Flatpak, otherwise an AppImage in
//!   `~/Applications` (the AppImage convention), with a menu entry and icon in
//!   `~/.local/share` so it shows up like any other app. A Flatpak or AppImage launcher prefers
//!   its own format.
//! - FreeBSD: the release's `/usr/local` tree, with a polkit prompt (or `~/.local` without one).
//! - Linux, inside the Flatpak launcher with no usable host Flatpak (none installed, or
//!   `flatpak-spawn` not allowed): the release tarball, unpacked into the launcher's own folder
//!   (`<data>/apps/<app>`) and run inside the same sandbox with `HOME` pointed at a `home/`
//!   folder beside it, so the app's settings stay in the sandbox too.
//!
//! Every download is checked against its published SHA-256 before anything is unpacked, a
//! running app is never replaced, and a new copy is fully in place before the old one goes.

use std::path::{Path, PathBuf};

use crate::protocol::{CANCELLED, INSTALL_CANCELLED, InstallLocations, JobPhase};
use crate::release::{self, Asset, Release};
use crate::{App, InstallSource, Installed, PackageKind, version};

use crate::http::{Http, Stop, Transfer};
use crate::manifest::Record;
use crate::paths::Paths;
use crate::process;

/// What an install needs from the backend.
pub struct Job<'a> {
    #[allow(missing_docs)]
    pub http: &'a Http,
    #[allow(missing_docs)]
    pub paths: &'a Paths,
    /// Checked during the install; set it to pause or cancel.
    pub stop: &'a Stop,
    /// Called each time the job enters a new phase.
    pub phase: &'a mut dyn FnMut(JobPhase),
    #[allow(missing_docs)]
    pub install_locations: &'a InstallLocations,
    /// Write over whatever [`occupied`] found.
    pub replace: bool,
}

/// Downloads and installs `asset` of `release`. Returns a record when the launcher owns the
/// result (portable ZIP, AppImage, macOS bundle, tarball); MSI installs are found in the registry.
/// `previous` is the launcher's record of the copy being replaced, if it installed it.
pub fn install(
    app: &App,
    release: &Release,
    asset: &Asset,
    kind: PackageKind,
    existing: Option<&Installed>,
    previous: Option<&Record>,
    job: &mut Job<'_>,
) -> Result<Option<Record>, String> {
    install_at(app, release, asset, kind, existing, previous, job, None)
}

type RecordCommit<'a> = Option<&'a mut dyn FnMut(&Record) -> Result<(), String>>;

/// Refuses to change an app that is running, or one whose state couldn't be checked.
fn not_running(app: &App, answer: Result<bool, String>) -> Result<(), String> {
    match answer {
        Ok(false) => Ok(()),
        Ok(true) => Err(format!("{} is running. Quit it, then try again.", app.name)),
        Err(reason) => Err(format!("couldn't check whether {} is running ({reason}), so nothing was changed", app.name)),
    }
}

/// [`install`], with `commit` called once a portable ZIP's new files are in place and its version
/// checked, so the caller can save its record; an error from it rolls the previous copy back
/// (Windows only; ignored elsewhere). Refuses before downloading when the app is running, the
/// asset isn't the release's version (`bind_artifact_version`), a newer copy is installed, or the
/// link isn't the release's own (`check_download_link`).
#[allow(clippy::too_many_arguments)]
pub fn install_at(
    app: &App,
    release: &Release,
    asset: &Asset,
    kind: PackageKind,
    existing: Option<&Installed>,
    previous: Option<&Record>,
    job: &mut Job<'_>,
    commit: RecordCommit<'_>,
) -> Result<Option<Record>, String> {
    #[cfg(windows)]
    let mut commit = commit;
    #[cfg(not(windows))]
    let _ = commit;
    #[allow(unused_variables)]
    let destination = destination(app, kind, existing, previous, job.install_locations)?;
    if let Some(inst) = existing
        && inst.source.launcher_can_update()
    {
        not_running(app, process::running(Path::new(&inst.location)))?;
    }
    bind_artifact_version(app, release, asset)?;
    if !job.replace
        && let Some(inst) = existing
        && let (Some(rel_v), Some(inst_v)) = (release.version.as_ref(), inst.version.as_ref())
        && version::is_newer(inst_v, rel_v)
    {
        return Err(format!("Cannot update {} to {}: installed version {} is newer", app.name, release.version_label(), inst_v));
    }
    check_download_link(app, release, asset)?;
    log::info!("{}: starting {} to {} ({})", app.name, if existing.is_some() { "update" } else { "install" }, release.version_label(), kind.label());
    let file = download(release, asset, job)?;
    log::info!("{}: download ready ({}), running installer...", app.name, file.display());
    // Past this point the install can't be interrupted halfway.
    (job.phase)(JobPhase::Installing);
    #[allow(unused_variables)]
    let version = release.version_label();
    let result = match kind {
        #[cfg(target_os = "macos")]
        PackageKind::Dmg => {
            let wait = macos::CheckTurn { stop: job.stop, phase: &mut *job.phase };
            macos::install_dmg(app, &file, existing, destination.as_deref(), job.replace, job.paths, &macos::Expect::release(release), wait)
                .map(|path| Some(record(&version, kind, path, Vec::new(), false)))
        }
        #[cfg(windows)]
        PackageKind::Msi => windows::run_msi(&file, destination.as_deref(), job.stop, &mut *job.phase).map(|_| None),
        #[cfg(windows)]
        PackageKind::PortableZip => {
            crate::portable::install(app, &file, &version, previous, job.paths, destination.as_deref(), job.replace, &mut |record| {
                if release.version.is_some() && crate::detect::windows::executable_version(&record.path).as_ref() != release.version.as_ref() {
                    return Err("The portable executable does not match the selected version. The installation was rolled back.".into());
                }
                match commit.as_deref_mut() {
                    Some(commit) => commit(record),
                    None => Ok(()),
                }
            })
            .map(Some)
        }
        #[cfg(target_os = "linux")]
        PackageKind::Deb | PackageKind::Rpm | PackageKind::ArchPkg | PackageKind::Flatpak => {
            let id = app.bundle_id();
            linux::install_package(kind, &file, Some(&id), job.stop, &mut *job.phase).map(|_| None)
        }
        #[cfg(target_os = "linux")]
        PackageKind::AppImage => {
            linux::install_appimage(&file, &asset.name, destination.as_deref(), previous, job.replace).map(|(path, replaced)| {
                let files = linux::integrate(app, &path, &replaced, previous);
                Some(record(&version, kind, path, files, false))
            })
        }
        #[cfg(target_os = "freebsd")]
        PackageKind::TarGz => freebsd::install_tree(app, &file, job.paths).map(|(bin, files, root)| Some(record(&version, kind, bin, files, root))),
        #[cfg(target_os = "linux")]
        PackageKind::TarGz => linux::install_tree(app, &file, job.paths).map(|path| Some(record(&version, kind, path, Vec::new(), false))),
        other => Err(format!("{} isn't supported on this system", other.label())),
    };
    match &result {
        Ok(_) => log::info!("{}: successfully installed/updated to {}", app.name, release.version_label()),
        Err(e) if e == CANCELLED => log::info!("{}: installation cancelled", app.name),
        Err(e) => log::error!("{}: installation failed: {e}", app.name),
    }
    // A failed install keeps the download: the next try checks it against its published checksum
    // again and skips the transfer.
    if result.is_ok() {
        let _ = std::fs::remove_file(&file);
    }
    // The system's own prompt was dismissed: the download itself wasn't cancelled.
    result.map_err(|e| if e == CANCELLED { INSTALL_CANCELLED.to_string() } else { e })
}

/// Rejects an artifact whose own identity disagrees with the release that authorized it.
///
/// A feed or GitHub converter supplies the release version, the asset filename, and the download
/// URL independently. The downgrade guard and the persisted record both trust `release.version`,
/// so a controller could advertise a new version while pointing at an older official artifact —
/// bypassing the guard and recording a false installed version (finding: version not bound to the
/// release). We require the version encoded in the asset filename and the release tag embedded in
/// the download URL to agree with the release version before any bytes are fetched.
pub(crate) fn bind_artifact_version(app: &App, release: &Release, asset: &Asset) -> Result<(), String> {
    let mismatch = || format!("{}'s download does not match the advertised version {}.", app.name, release.version_label());
    let release_version = release.version.as_ref();

    // The version in the asset filename (`<stem>-<version>-<os>-<arch>.<ext>`), when it carries one.
    let stems: Vec<&str> = app.stems().collect();
    if let Some(release_version) = release_version
        && let Some(parsed) = crate::platform::parse_asset_name(&asset.name, &stems)
        && version::parse(&parsed.version).as_ref() != Some(release_version)
    {
        return Err(mismatch());
    }

    // The release tag and filename embedded in a release download URL (`.../releases/download/<tag>/<name>`).
    // A `+` in either is `%2B` there, as GitHub and `expected_download_url` write it; no other
    // escape is decoded, so anything else still has to match as written.
    if let Some((_, rest)) = asset.url.split_once("/releases/download/") {
        let mut parts = rest.split('/');
        let tag = parts.next().unwrap_or("").replace("%2B", "+");
        let url_filename = parts.next().unwrap_or("").replace("%2B", "+");
        if tag.is_empty()
            || release_version
                .map_or(tag != release.tag, |release_version| tag != release.tag && version::parse(&tag).as_ref() != Some(release_version))
        {
            return Err(mismatch());
        }
        if !url_filename.is_empty() && url_filename != asset.name {
            return Err(format!("{}'s download URL filename does not match selected asset {}.", app.name, asset.name));
        }
    }
    Ok(())
}

/// The last check before any bytes are fetched (storytold/craft-launcher#764): the link must be
/// exactly the GitHub download for this release's tag and this asset's name in the release's
/// repository ([`Release::repo`], else the app's own), whichever converter (API, feed or
/// `SHA256SUMS.txt`) built the asset, and so must the link of the release's `SHA256SUMS.txt` that
/// the download is checked against. A `+` may be written plainly or as `%2B`
/// ([`release::same_download_url`]).
///
/// This crate's tests accept a loopback link as well, as their downloads come from a server on
/// 127.0.0.1.
pub fn check_download_link(app: &App, release: &Release, asset: &Asset) -> Result<(), String> {
    let repo = if release.repo.is_empty() { app.repo } else { release.repo.as_str() };
    let own = |a: &Asset| {
        (cfg!(test) && crate::http::is_loopback(&a.url))
            || release::expected_download_url(repo, &release.tag, &a.name).is_some_and(|expected| release::same_download_url(&a.url, &expected))
    };
    if own(asset) && release.sums_asset().is_none_or(own) {
        return Ok(());
    }
    Err(format!("{}'s download link isn't the one its release publishes ({}), so it wasn't downloaded.", app.name, asset.name))
}

/// What a new install would write over: something already at its destination that the launcher
/// didn't put there.
pub fn occupied(
    app: &App,
    #[cfg_attr(not(target_os = "linux"), allow(unused_variables))] asset: &Asset,
    kind: PackageKind,
    existing: Option<&Installed>,
    previous: Option<&Record>,
    locations: &InstallLocations,
    paths: &Paths,
) -> Result<Option<PathBuf>, String> {
    if existing.is_some() || previous.is_some() {
        return Ok(None);
    }
    let folder = destination(app, kind, None, None, locations)?;
    let target = match kind {
        PackageKind::Msi => folder,
        PackageKind::PortableZip => Some(folder.unwrap_or_else(|| paths.apps()).join(app.id)),
        PackageKind::Dmg => folder.map(|f| f.join(format!("{}.app", app.name))),
        #[cfg(target_os = "linux")]
        PackageKind::AppImage => folder.or_else(crate::detect::linux::appimage_dir).map(|f| f.join(&asset.name)),
        _ => None,
    };
    Ok(target.filter(|t| std::fs::symlink_metadata(t).is_ok()))
}

/// The folder a new copy goes in: beside the launcher's copy it replaces, else the user's choice,
/// else the platform default (`None`). macOS updates find their bundle's folder themselves.
fn destination(
    app: &App,
    kind: PackageKind,
    existing: Option<&Installed>,
    previous: Option<&Record>,
    locations: &InstallLocations,
) -> Result<Option<PathBuf>, String> {
    if kind == PackageKind::PortableZip && previous.is_some_and(|record| record.kind == PackageKind::PortableZip) {
        return Ok(None);
    }
    if let Some(inst) = existing {
        let exe = Path::new(&inst.location);
        return Ok(match kind {
            PackageKind::AppImage => previous.filter(|r| r.kind == kind).and_then(|r| r.path.parent()).map(Path::to_path_buf),
            PackageKind::Msi => exe.parent().filter(|_| exe.is_absolute()).map(Path::to_path_buf),
            _ => None,
        });
    }
    let Some(folder) = locations.folder(app.id) else { return Ok(None) };
    if !matches!(kind, PackageKind::Dmg | PackageKind::AppImage | PackageKind::Msi | PackageKind::PortableZip) || process::in_sandbox() {
        return Err("custom install folders aren't supported on this system".into());
    }
    let parent = selected_folder(folder, kind != PackageKind::Msi)?;
    Ok(Some(if kind == PackageKind::Msi { parent.join(app.name) } else { parent }))
}

fn selected_folder(folder: &str, writable: bool) -> Result<PathBuf, String> {
    let path = if folder == "~" {
        crate::folders::home_dir().ok_or("no home folder")?
    } else if let Some(relative) = folder.strip_prefix("~/") {
        crate::folders::home_dir().ok_or("no home folder")?.join(relative)
    } else {
        PathBuf::from(folder)
    };
    let invalid = |reason: &str| format!("couldn't use install folder {}: {reason}", path.display());
    if folder.chars().any(|c| c.is_control() || (cfg!(windows) && c == '"')) {
        return Err(invalid("the path contains invalid characters"));
    }
    if !path.is_absolute() {
        return Err(invalid("choose an absolute path"));
    }
    if !path.is_dir() {
        return Err(invalid("the folder is unavailable. Create it or reconnect its drive first"));
    }
    if writable {
        let probe = path.join(format!(
            ".artcraft-launcher-write-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos())
        ));
        let output = std::fs::OpenOptions::new().write(true).create_new(true).open(&probe).map_err(|e| invalid(&e.to_string()))?;
        drop(output);
        std::fs::remove_file(&probe).map_err(|e| invalid(&e.to_string()))?;
    }
    Ok(path)
}

#[cfg(windows)]
fn msi_folder_argument(path: &Path) -> Result<String, String> {
    let value = path.to_str().ok_or("unusual characters in a path")?;
    if value.chars().any(|c| c == '"' || c.is_control()) {
        return Err(format!("couldn't use install folder {value}: the path contains invalid characters"));
    }
    Ok(format!("INSTALLFOLDER=\"{value}\""))
}

/// Where `asset` is downloaded to (and its partial file kept while unfinished).
pub fn download_path(paths: &Paths, asset: &Asset) -> PathBuf {
    paths.downloads().join(&asset.name)
}

#[allow(dead_code)]
fn record(version: &str, kind: PackageKind, path: PathBuf, files: Vec<PathBuf>, needs_root: bool) -> Record {
    Record { version: version.to_string(), kind, path, installed_at: crate::now(), files, needs_root }
}

/// The package format to install `app` with: the format of the copy that's there (an update
/// stays a `.deb` update), else the user's choice, else the best this system supports.
///
/// Returns an empty `Vec` when `app` is already installed outside the launcher (such as an AUR
/// or repository Arch package) and must not be updated or replaced by the launcher.
pub fn choose_kinds(app: &App, existing: Option<&Installed>, record: Option<&Record>, preferred: &[PackageKind]) -> Vec<PackageKind> {
    let available = available_kinds();
    let can_pacman = available.contains(&PackageKind::ArchPkg) || can_pacman();
    choose_kinds_with(app, existing, record, preferred, &available, can_pacman)
}

pub(crate) fn choose_kinds_with(
    app: &App,
    existing: Option<&Installed>,
    record: Option<&Record>,
    preferred: &[PackageKind],
    available: &[PackageKind],
    can_pacman: bool,
) -> Vec<PackageKind> {
    if let Some(inst) = existing {
        let kind = match inst.source {
            InstallSource::Launcher => record.map(|r| r.kind),
            InstallSource::MacApp => Some(PackageKind::Dmg),
            InstallSource::Msi => Some(PackageKind::Msi),
            InstallSource::Deb => Some(PackageKind::Deb),
            InstallSource::Rpm => Some(PackageKind::Rpm),
            // The release's own Arch package is updated like a .deb. AUR builds and repository
            // packages are updated outside the launcher (see `managed_elsewhere`); the launcher
            // opens and removes them but never replaces them.
            InstallSource::Pacman if is_release_arch_package(app, inst) => Some(PackageKind::ArchPkg),
            InstallSource::Pacman => return Vec::new(),
            InstallSource::Flatpak => Some(PackageKind::Flatpak),
            InstallSource::AppImage => Some(PackageKind::AppImage),
            InstallSource::Binary => Some(PackageKind::TarGz),
        };
        if let Some(k) = kind.filter(|k| available.contains(k) || (*k == PackageKind::ArchPkg && can_pacman)) {
            return vec![k];
        }
    }
    let mut kinds: Vec<PackageKind> = preferred.iter().copied().filter(|k| available.contains(k)).collect();
    for k in available {
        if !kinds.contains(k) {
            kinds.push(*k);
        }
    }
    kinds
}

/// Whether this system has a working pacman installation and pacman database.
fn can_pacman() -> bool {
    #[cfg(target_os = "linux")]
    return find_in(&SYSTEM_BIN_DIRS, std::ffi::OsStr::new("pacman")).is_some() && crate::detect::linux::has_pacman_db();
    #[cfg(not(target_os = "linux"))]
    return false;
}

/// Whether a pacman install is the release's own Arch package rather than an AUR build. The
/// release names it by the bundle id (`ai.storyteller.<app>`), which no AUR package uses
/// (they are `<app>`, `<app>-bin` and `<app>-git`), so an AUR helper never mistakes it
/// for one of its own and rebuilds it from source.
pub fn is_release_arch_package(app: &App, installed: &Installed) -> bool {
    installed.source == InstallSource::Pacman && installed.package.as_deref() == Some(&app.bundle_id())
}

/// Package formats this machine can install, best first.
pub fn available_kinds() -> Vec<PackageKind> {
    #[cfg(target_os = "macos")]
    return vec![PackageKind::Dmg];
    #[cfg(windows)]
    return vec![PackageKind::Msi, PackageKind::PortableZip];
    #[cfg(target_os = "linux")]
    return linux::available_kinds();
    #[cfg(target_os = "freebsd")]
    return vec![PackageKind::TarGz];
    #[allow(unreachable_code)]
    Vec::new()
}

/// Fetches `asset` into the download folder, checked against its published SHA-256: the feed's
/// or GitHub's per-file digest, else the release's `SHA256SUMS.txt`. A release with neither is
/// refused before anything is fetched: a file nothing vouches for is never installed (#90).
pub(crate) fn download(release: &Release, asset: &Asset, job: &mut Job<'_>) -> Result<PathBuf, String> {
    if !release::valid_file_name(&asset.name) {
        return Err("the release has an unexpected file name".into());
    }
    let sha = match &asset.sha256 {
        Some(s) => Some(s.clone()),
        None => published_checksum(release, asset, job.http)?,
    };
    let Some(sha) = sha else {
        return Err(format!(
            "the release publishes no checksum for {}, so it can't be verified and wasn't installed. Try again later, or report it on GitHub.",
            asset.name
        ));
    };
    let dest = download_path(job.paths, asset);
    let phase = &mut *job.phase;
    job.http.download(&asset.url, &dest, asset.size, &sha, job.stop, &mut |t| match t {
        Transfer::Bytes { done, total } => phase(JobPhase::Downloading { done, total }),
        Transfer::Reconnecting { done, total } => phase(JobPhase::Reconnecting { done, total }),
    })?;
    (job.phase)(JobPhase::Verifying);
    Ok(dest)
}

/// The file's line in the release's `SHA256SUMS.txt`. `Ok(None)` only when the release really
/// publishes no checksum for it; a list that exists but can't be fetched is an error, so a
/// network hiccup never downgrades the check.
fn published_checksum(release: &Release, asset: &Asset, http: &Http) -> Result<Option<String>, String> {
    let Some(sums) = release.sums_asset() else { return Ok(None) };
    let Some(text) = http.get_text(&sums.url).map_err(|e| format!("couldn't fetch the release's checksums ({e}). Try again"))? else {
        return Ok(None);
    };
    Ok(release::parse_sha256sums(&text).into_iter().find(|(_, n)| *n == asset.name).map(|(h, _)| h))
}

/// When `installed` is owned by something the launcher doesn't drive, what to tell the user
/// instead of installing a second copy. Arch packages other than the release's own are removed
/// through pacman like any system package, but updated by whatever they came from: a system
/// update for a repository package (Omarchy's, Chaotic-AUR, ...), the user's AUR helper (which
/// builds as the user and can't run behind the password prompt) for anything else.
pub fn managed_elsewhere(app: &App, installed: &Installed, uninstall: bool) -> Option<String> {
    managed_elsewhere_with(app, installed, uninstall, pacman_repo)
}

fn managed_elsewhere_with(app: &App, installed: &Installed, uninstall: bool, repo_fn: impl Fn(&str) -> Option<String>) -> Option<String> {
    match installed.source {
        InstallSource::Pacman if !uninstall && !is_release_arch_package(app, installed) => {
            let pkg = installed.package.clone().unwrap_or_else(|| app.id.to_string());
            Some(pacman_update_hint(app, &pkg, repo_fn(&pkg).as_deref()))
        }
        InstallSource::Binary => Some(format!("{} was installed outside the launcher. Update or remove it the way it was installed.", app.name)),
        _ => None,
    }
}

/// How to update an Arch package the launcher doesn't own: `repo` is the sync repository it
/// comes from, or `None` for a foreign (AUR) package.
fn pacman_update_hint(app: &App, pkg: &str, repo: Option<&str>) -> String {
    let helper = ["paru", "yay"].into_iter().find(|h| which(h).is_some());
    let cmd = match helper {
        Some(h) => format!("{h} -S {pkg}"),
        None => format!("your AUR helper (for example: yay -S {pkg})"),
    };
    if let Some(repo) = repo {
        return format!("{} was installed from the {repo} repository or the AUR ({pkg}). Update it with {cmd} or sudo pacman -Syu", app.name);
    }
    format!("{} was installed from the AUR ({pkg}). Update it with {cmd}", app.name)
}

/// The sync repository (`extra`, `omarchy`, ...) that carries the installed package `pkg`, or
/// `None` when no configured repository does (an AUR or locally built package) or pacman can't
/// be asked.
fn pacman_repo(pkg: &str) -> Option<String> {
    #[cfg(target_os = "linux")]
    if !crate::detect::linux::has_pacman_db() {
        return None;
    }
    let pacman = which("pacman")?;
    let t = std::time::Duration::from_secs(5);
    // For packages carried by a configured sync repository, ask pacman -Si which repo carries it.
    // pacman translates its field names; the C locale keeps "Repository" parseable.
    let mut si = process::command(&pacman);
    si.args(["-Si", "--", pkg]).env("LC_ALL", "C");
    let out = process::run_command(si, "pacman", t).ok()?;
    parse_pacman_repo(&out)
}

/// The `Repository` field of `pacman -Si` output (the first one, when several repos carry it:
/// that is the one pacman installs from).
fn parse_pacman_repo(si: &str) -> Option<String> {
    si.lines().find_map(|l| {
        let (key, value) = l.split_once(':')?;
        (key.trim() == "Repository").then(|| value.trim().to_string()).filter(|r| !r.is_empty())
    })
}

/// Removes an installed copy. `phase` hears when the removal has to wait for another install;
/// `stop` cancels it while it waits.
pub fn uninstall(
    app: &App,
    installed: &Installed,
    record: Option<&Record>,
    paths: &Paths,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
) -> Result<(), String> {
    let _ = (record, stop, &phase, paths);
    let path = PathBuf::from(&installed.location);
    if let Some(msg) = managed_elsewhere(app, installed, true) {
        return Err(msg);
    }
    if matches!(installed.source, InstallSource::MacApp | InstallSource::Msi | InstallSource::AppImage | InstallSource::Launcher) {
        not_running(app, process::running(&path))?;
    }
    match installed.source {
        #[cfg(windows)]
        InstallSource::Launcher if record.is_some_and(|r| r.kind == PackageKind::PortableZip) => {
            crate::portable::remove(app, record.ok_or("the launcher has no record of these files")?)
        }
        #[cfg(windows)]
        InstallSource::Msi => windows::uninstall_msi(app, stop, phase),
        #[cfg(target_os = "macos")]
        InstallSource::MacApp | InstallSource::Launcher => macos::move_to_trash(&path),
        #[cfg(target_os = "linux")]
        InstallSource::Deb | InstallSource::Rpm | InstallSource::Pacman | InstallSource::Flatpak => {
            linux::remove_package(app, installed, stop, phase)
        }
        #[cfg(target_os = "freebsd")]
        InstallSource::Launcher => match record {
            Some(r) => freebsd::remove_files(&r.files, r.needs_root),
            None => Err("the launcher has no record of these files".into()),
        },
        #[cfg(target_os = "linux")]
        InstallSource::AppImage => remove_appimage(app, &path, record),
        // A tarball the launcher unpacked into its own folder: take the tree out and leave the
        // app's settings (`home/`) alone, so a reinstall finds its projects where they were.
        #[cfg(target_os = "linux")]
        InstallSource::Launcher if record.is_some_and(|r| r.kind == PackageKind::TarGz) => linux::remove_tree(app, paths),
        #[cfg(target_os = "linux")]
        InstallSource::Launcher => remove_appimage(app, &path, record),
        other => Err(format!("{} can't be removed from here", other.label())),
    }
}

/// Deletes an AppImage, the menu entry and icon the launcher wrote for it, and the menu entry
/// the app may have written for itself on first run.
#[cfg(target_os = "linux")]
fn remove_appimage(app: &App, path: &Path, record: Option<&Record>) -> Result<(), String> {
    if path.extension().is_none_or(|e| !e.eq_ignore_ascii_case("AppImage")) || !path.is_file() {
        return Err(format!("{} isn't an AppImage, so it was left alone", path.display()));
    }
    // AppImages found elsewhere (Gear Lever's ~/AppImages, ~/.local/bin) are the user's own:
    // remove only the launcher's folder or a copy the launcher recorded installing.
    let ours =
        path.parent().is_some_and(|dir| crate::detect::linux::appimage_dir().is_some_and(|own| own == dir)) || record.is_some_and(|r| r.path == path);
    if !ours {
        return Err(format!("{} wasn't installed by the launcher. Remove it the way it was added.", path.display()));
    }
    std::fs::remove_file(path).map_err(|e| format!("couldn't remove {} ({e})", path.display()))?;
    linux::unintegrate(app, path, record);
    Ok(())
}

/// Starts an installed app.
pub fn launch(app: &App, installed: &Installed, paths: &Paths) -> Result<(), String> {
    let _ = paths;
    let path = Path::new(&installed.location);
    match installed.source {
        InstallSource::Flatpak => {
            let mut cmd = process::flatpak_command();
            cmd.args(["run", &app.bundle_id()]);
            process::spawn_detached(cmd)
        }
        InstallSource::MacApp => process::open(path.as_os_str()),
        // A tarball the launcher unpacked into its own folder: run it here, against its own
        // home, rather than on the host through `flatpak-spawn` (which is what `launch_binary`
        // would do inside the sandbox). Its settings then land beside it instead of in the
        // launcher's or the host's home folder.
        #[cfg(target_os = "linux")]
        InstallSource::Launcher if path.starts_with(paths.apps()) => {
            let home = paths.app_home(app.id);
            prepare_home(&home);
            let mut cmd = process::command(path);
            for (name, value) in home_env(&home) {
                cmd.env(name, value);
            }
            if let Some(dir) = path.parent() {
                cmd.current_dir(dir);
            }
            process::spawn_detached(cmd)
        }
        _ if cfg!(target_os = "macos") && path.extension().is_some_and(|e| e == "app") => process::open(path.as_os_str()),
        _ => {
            if !path.exists() {
                return Err(format!("{} isn't where it was installed ({})", app.name, path.display()));
            }
            process::launch_binary(path, &[])
        }
    }
    .map_err(|e| format!("Couldn't open {}: {e}", app.name))
}

/// The environment a Craft App unpacked by the launcher runs with: its own `HOME` and the XDG
/// base directories inside it. Apps read their settings from these, so everything they keep
/// (settings, projects, caches) stays in that one folder.
#[cfg(target_os = "linux")]
fn home_env(home: &Path) -> [(&'static str, PathBuf); 5] {
    [
        ("HOME", home.to_path_buf()),
        ("XDG_CONFIG_HOME", home.join(".config")),
        ("XDG_DATA_HOME", home.join(".local/share")),
        ("XDG_CACHE_HOME", home.join(".cache")),
        ("XDG_STATE_HOME", home.join(".local/state")),
    ]
}

/// Creates the folders an app unpacked by the launcher expects to find, and links the host's
/// `~/.vst3` and `~/.clap` (SoundCraft's plugin folders) in when they exist, so an app running
/// inside the sandbox still sees the plugins installed on the host. Every failure here is
/// ignored: the app is worth starting even if its home is not quite ready.
#[cfg(target_os = "linux")]
fn prepare_home(home: &Path) {
    for (_, dir) in home_env(home).iter().skip(1) {
        let _ = std::fs::create_dir_all(dir);
    }
    let Some(real) = crate::folders::home_dir() else { return };
    if real == home {
        return;
    }
    for name in [".vst3", ".clap"] {
        let link = home.join(name);
        if link.symlink_metadata().is_ok() {
            continue;
        }
        if real.join(name).is_dir() {
            let _ = std::os::unix::fs::symlink(real.join(name), link);
        }
    }
}

/// A new folder of our own inside `dir`, removed when dropped: a predictable name there may be
/// the user's.
#[cfg(target_os = "linux")]
fn staging_dir(dir: &Path) -> Result<tempfile::TempDir, String> {
    tempfile::Builder::new()
        .prefix(".artcraft-launcher-")
        .tempdir_in(dir)
        .map_err(|e| format!("couldn't create a staging folder in {} ({e})", dir.display()))
}

/// Moves `staged` into place at `dest`, keeping the old copy until the new one is there.
#[allow(dead_code)]
fn swap_into_place(staged: &Path, dest: &Path) -> Result<(), String> {
    swap_into_place_via(staged, dest, |from, to| std::fs::rename(from, to))
}

fn swap_into_place_via(staged: &Path, dest: &Path, rename: impl Fn(&Path, &Path) -> std::io::Result<()>) -> Result<(), String> {
    // Reserve a directory of our own beside the destination. A predictable sibling such as
    // `app.old` may be a user's backup, and must never be removed or overwritten.
    let backup = if dest.exists() {
        let parent = dest.parent().ok_or("the installed app has no parent folder")?;
        let backup = tempfile::Builder::new()
            .prefix(".artcraft-launcher-backup-")
            .tempdir_in(parent)
            .map_err(|e| format!("couldn't create a backup folder in {} ({e})", parent.display()))?;
        rename(dest, &backup.path().join("old")).map_err(|e| format!("couldn't move the old copy aside ({e})"))?;
        Some(backup)
    } else {
        None
    };
    if let Err(e) = rename(staged, dest) {
        if let Some(backup) = backup
            && let Err(restore) = rename(&backup.path().join("old"), dest)
        {
            // A failed rollback must not let TempDir's cleanup delete the only old copy.
            let saved = backup.keep().join("old");
            return Err(format!(
                "couldn't put the new copy in place ({e}); couldn't restore the old copy ({restore}); it was kept at {}",
                saved.display()
            ));
        }
        return Err(format!("couldn't put the new copy in place ({e})"));
    }
    Ok(())
}

/// After unpacking an archive into `dir`: its single top-level folder, if it has one.
#[allow(dead_code)]
fn single_top_folder(dir: &Path) -> Result<PathBuf, String> {
    let entries: Vec<_> = std::fs::read_dir(dir).map_err(|e| e.to_string())?.flatten().collect();
    match entries.as_slice() {
        [only] if only.file_type().is_ok_and(|ft| ft.is_dir() && !ft.is_symlink()) => Ok(only.path()),
        _ => Ok(dir.to_path_buf()),
    }
}

/// The app binary inside an unpacked archive: `bin/<id>`, `<id>` or `<id>.exe`.
#[allow(dead_code)]
fn find_binary(root: &Path, app: &App) -> Option<PathBuf> {
    let exe = if cfg!(windows) { format!("{}.exe", app.id) } else { app.id.to_string() };
    [root.join("bin").join(&exe), root.join(&exe)].into_iter().find(|p| p.is_file())
}

/// Finds a program on `PATH`.
#[allow(dead_code)]
fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| p.is_file())
}

/// The root-owned directories that programs run as root come from. `pkexec` looks a bare program
/// name up on the caller's `PATH`, so it is always handed an absolute path from one of these.
#[cfg(target_os = "linux")]
const SYSTEM_BIN_DIRS: [&str; 4] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin"];
/// FreeBSD's packages, polkit's `pkexec` among them, install under `/usr/local`.
#[cfg(target_os = "freebsd")]
const SYSTEM_BIN_DIRS: [&str; 6] = ["/usr/bin", "/bin", "/usr/sbin", "/sbin", "/usr/local/bin", "/usr/local/sbin"];

/// `name` in the first of `dirs` that has it.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn find_in(dirs: &[impl AsRef<Path>], name: &std::ffi::OsStr) -> Option<PathBuf> {
    dirs.iter().map(|d| d.as_ref().join(name)).find(|p| p.is_file())
}

/// `args` with its program as an absolute path: a bare name is looked up in `dirs`.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn with_absolute_program(dirs: &[impl AsRef<Path>], args: &[&std::ffi::OsStr]) -> Result<Vec<std::ffi::OsString>, String> {
    let (program, rest) = args.split_first().ok_or("there is no command to run")?;
    let path = Path::new(program);
    let program = if path.is_absolute() {
        path.to_path_buf()
    } else if path.file_name() == Some(*program) {
        find_in(dirs, program).ok_or_else(|| format!("{} isn't installed", path.display()))?
    } else {
        return Err(format!("{} isn't a program name", path.display()));
    };
    Ok(std::iter::once(program.into_os_string()).chain(rest.iter().map(|a| a.to_os_string())).collect())
}

/// polkit's `pkexec`, from the same directories as the programs it runs.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn pkexec() -> Option<PathBuf> {
    find_in(&SYSTEM_BIN_DIRS, std::ffi::OsStr::new("pkexec"))
}

/// Runs a command as root through polkit (`pkexec`), which shows the desktop's password prompt.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn run_as_root(args: &[&std::ffi::OsStr]) -> Result<(), String> {
    let args = with_absolute_program(&SYSTEM_BIN_DIRS, args)?;
    let pkexec = pkexec().ok_or("installing system-wide needs polkit (pkexec), which isn't installed")?;
    let status = process::command(pkexec).args(&args).status().map_err(|e| format!("couldn't ask for permission ({e})"))?;
    root_outcome(status.code())
}

/// What `pkexec`'s exit code says about the command it ran.
#[cfg(any(target_os = "linux", target_os = "freebsd"))]
fn root_outcome(code: Option<i32>) -> Result<(), String> {
    match code {
        Some(0) => Ok(()),
        // pkexec: 126 = the password prompt was dismissed, 127 = not authorized.
        Some(126 | 127) => Err("cancelled".into()),
        Some(c) => Err(format!("the system installer stopped with error {c}")),
        None => Err("the system installer was interrupted".into()),
    }
}

/// How the error of an installer call shared by several apps ends (see `linux::shared_outcome`).
const SHARED_CALL: &str = "apps together. Try them one at a time.";

/// Whether an install failed in one installer call shared with other apps (Linux packages
/// installed together). Such a call can fail after it has installed some of its apps; any other
/// failure is the app's own.
pub fn failed_in_shared_call(error: &str) -> bool {
    cfg!(target_os = "linux") && error.ends_with(SHARED_CALL)
}

/// The system's installer runs one job at a time; the others wait their turn here.
#[cfg(any(target_os = "linux", windows))]
static INSTALLER: crate::queue::Queue = crate::queue::Queue::new();

/// For a [`crate::queue::Queue`]: reports [`JobPhase::Waiting`] while a job waits, then `working` again.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn turn<'a>(phase: &'a mut dyn FnMut(JobPhase), working: JobPhase) -> impl FnMut(bool) + 'a {
    move |waiting| phase(if waiting { JobPhase::Waiting } else { working })
}

/// For a [`crate::queue::Queue`]: whether the user cancelled the job while it waits.
#[cfg(any(target_os = "linux", target_os = "macos", windows))]
fn cancelled(stop: &Stop) -> impl Fn() -> bool + '_ {
    || stop.check().is_err_and(|e| e == crate::protocol::CANCELLED)
}

#[cfg(target_os = "linux")]
pub(crate) mod linux;

#[cfg(target_os = "macos")]
pub mod macos;

/// FreeBSD: the release tarball holds a `/usr/local`-style tree (`bin/<app>`,
/// `share/applications`, icons). Older releases had the files flat in one folder; those are laid
/// out the same way first.
#[cfg(unix)]
#[allow(dead_code)]
pub(crate) mod tree;

#[cfg(target_os = "freebsd")]
mod freebsd;

/// Builds the Windows Installer command separately so its package argument can be checked
/// without starting an installer.
#[cfg_attr(not(windows), allow(dead_code))]
fn msi_command(program: &Path, msi: &Path) -> Result<std::process::Command, String> {
    let msi = msi_path(msi)?;
    let mut command = process::command(program);
    // A verbose log beside the package: Windows Installer's own errors (1603 above all) say
    // nothing without it.
    command.arg("/i").arg(&msi).args(["/passive", "/norestart", "/l*v"]).arg(msi_log(&msi));
    Ok(command)
}

/// The self-update command: [`msi_command`] plus the properties that keep the installed copy's
/// Windows Installer context (`msi_copy::update_arguments`).
#[cfg_attr(not(windows), allow(dead_code))]
fn reinstall_command(program: &Path, msi: &Path, context: &[&str]) -> Result<std::process::Command, String> {
    let mut command = msi_command(program, msi)?;
    command.args(context);
    Ok(command)
}

/// Where `msi_command` asks Windows Installer to write its log.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn msi_log(msi: &Path) -> PathBuf {
    msi.with_extension("log")
}

/// Windows Installer's storage APIs can fail on paths through junctions or symlinks (#338).
/// Keep ordinary drive/UNC syntax when possible: Windows canonicalization alone introduces
/// a `\\?\` prefix that older APIs also cannot handle.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn msi_path(msi: &Path) -> Result<PathBuf, String> {
    #[cfg(windows)]
    let resolved = dunce::canonicalize(msi);
    #[cfg(not(windows))]
    let resolved = std::fs::canonicalize(msi);
    resolved.map_err(|e| format!("couldn't resolve the Windows Installer package {} ({e})", msi.display()))
}

#[cfg(windows)]
pub mod windows;

#[cfg(test)]
mod tests;
