//! Detecting an installed app on Linux: packages, Flatpak and AppImage.

use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{App, InstallSource, Installed, Snapshot, appimage_version, parse_cli_version, version};
use crate::packages::{self, Package};
use crate::process::capture;

/// Installed Craft App packages from every package manager on this system (dpkg, rpm,
/// pacman, Flatpak). Read once per detection pass.
pub fn packages() -> Vec<Package> {
    let ours = |name: &str| {
        crate::suite::SUITE.iter().any(|a| {
            [InstallSource::Deb, InstallSource::Rpm, InstallSource::Pacman, InstallSource::Flatpak]
                .iter()
                .any(|m| crate::packages::candidate_names(a, *m).iter().any(|n| n == name))
        })
    };
    let t = Duration::from_secs(10);
    let mut out = Vec::new();
    let mut add = |manager, list: Vec<(String, String)>| {
        out.extend(list.into_iter().filter(|(n, _)| ours(n)).map(|(name, version)| Package { manager, name, version }));
    };
    if let Some(list) = capture("dpkg-query", &["-W", "-f", "${Package}|${db:Status-Status}|${Version}\n"], t) {
        add(InstallSource::Deb, packages::parse_dpkg(&list));
    }
    if let Some(list) = capture("rpm", &["-qa", "--qf", "%{NAME}|%{VERSION}\n"], t) {
        add(InstallSource::Rpm, packages::parse_rpm(&list));
    }
    // Only run `pacman` where its database exists: elsewhere `pacman` on PATH may be an unrelated program
    // (the Pac-Man game in /usr/games on Debian-family systems).
    if has_pacman_db()
        && let Some(list) = capture("pacman", &["-Q"], t)
    {
        add(InstallSource::Pacman, packages::parse_pacman(&list));
    }
    let arch = format!("--arch={}", std::env::consts::ARCH);
    if let Some(list) = crate::process::capture_flatpak(&["list", "--app", &arch, "--columns=application,version"], t) {
        let mut flatpaks = packages::parse_flatpak_list(&list);
        // Bundles built from a release tarball often list no version: ask Flatpak directly.
        for (id, version) in flatpaks.iter_mut().filter(|(id, v)| v.is_empty() && id.starts_with("ai.storyteller.")) {
            if let Some(v) = crate::process::capture_flatpak(&["info", &arch, id], t).as_deref().and_then(packages::parse_flatpak_info) {
                *version = v;
            }
        }
        add(InstallSource::Flatpak, flatpaks);
    }
    out
}

/// The local package database of Arch's pacman.
pub(crate) const PACMAN_DB: &str = "/var/lib/pacman/local";

/// True when `db` is a directory, i.e. this system really uses pacman.
pub(crate) fn pacman_db_present(db: &Path) -> bool {
    db.is_dir()
}

/// True when the system's default pacman database directory exists.
pub(crate) fn has_pacman_db() -> bool {
    pacman_db_present(Path::new(PACMAN_DB))
}

/// Where a package put the app's binary: on `PATH`, else `/usr/bin/<id>`.
fn binary_on_path(id: &str) -> String {
    std::env::var_os("PATH")
        .and_then(|path| std::env::split_paths(&path).map(|d| d.join(id)).find(|p| p.is_file()))
        .unwrap_or_else(|| PathBuf::from("/usr/bin").join(id))
        .display()
        .to_string()
}

/// `~/Applications`, where the launcher puts AppImages (the AppImageLauncher convention).
pub fn appimage_dir() -> Option<PathBuf> {
    crate::folders::home_dir().map(|h| h.join("Applications"))
}

/// Every folder searched for AppImages, the launcher's own first: `~/Applications`,
/// `~/AppImages` (Gear Lever's default) and `~/.local/bin`.
pub fn appimage_dirs() -> Vec<PathBuf> {
    let Some(home) = crate::folders::home_dir() else { return Vec::new() };
    vec![home.join("Applications"), home.join("AppImages"), home.join(".local").join("bin")]
}

/// AppImages of `app` in `dirs`, newest first (unknown versions last; earlier folders win
/// ties).
pub fn appimages_in(app: &App, dirs: &[PathBuf]) -> Vec<(PathBuf, Option<version::Version>)> {
    let stems: Vec<&str> = app.stems().collect();
    let mut out: Vec<(PathBuf, Option<version::Version>)> = Vec::new();
    for dir in dirs {
        let Ok(entries) = std::fs::read_dir(dir) else { continue };
        let mut found: Vec<(PathBuf, Option<version::Version>)> = entries
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let version = appimage_version(&name, &stems)?;
                let path = e.path();
                path.is_file().then(|| (path, version.as_deref().and_then(version::parse)))
            })
            .collect();
        found.sort_by(|a, b| a.0.cmp(&b.0));
        out.extend(found);
    }
    // Stable sort: equal versions keep the folder order above.
    out.sort_by(|a, b| match (&b.1, &a.1) {
        (Some(x), Some(y)) => version::compare(x, y),
        (Some(_), None) => std::cmp::Ordering::Greater,
        (None, Some(_)) => std::cmp::Ordering::Less,
        (None, None) => std::cmp::Ordering::Equal,
    });
    out
}

/// AppImages of `app` in any of the usual folders, newest first.
pub fn appimages(app: &App) -> Vec<(PathBuf, Option<version::Version>)> {
    appimages_in(app, &appimage_dirs())
}

/// What is installed of `app` on this system: an AppImage first, then a deb, rpm, Arch or
/// Flatpak package, then a binary on `PATH`.
pub fn detect(app: &App, snapshot: &Snapshot) -> Option<Installed> {
    if let Some((path, version)) = appimages(app).into_iter().next() {
        return Some(Installed { version, source: InstallSource::AppImage, location: path.display().to_string(), package: None });
    }
    let order = [InstallSource::Deb, InstallSource::Rpm, InstallSource::Pacman, InstallSource::Flatpak];
    if let Some(p) = packages::find(app, &snapshot.packages, &order) {
        let location = if p.manager == InstallSource::Flatpak { app.bundle_id() } else { binary_on_path(app.id) };
        return Some(Installed { version: packages::version_of(p), source: p.manager, location, package: Some(p.name.clone()) });
    }
    detect_system_install(app)
}

/// When the package managers don't report the app (a query timed out or was locked, an
/// unlisted package name, or a sandbox that can't see them) but its binary is there. It is
/// reported as a system binary: the launcher can open it but leaves updating and removing it
/// to whatever installed it, since it can't tell which package manager that was.
fn detect_system_install(app: &App) -> Option<Installed> {
    let bin = find_binary(app.id)?;
    Some(Installed { version: binary_version(&bin, app), source: InstallSource::Binary, location: bin.display().to_string(), package: None })
}

fn find_binary(id: &str) -> Option<PathBuf> {
    let on_path = std::env::var_os("PATH").and_then(|path| std::env::split_paths(&path).map(|d| d.join(id)).find(|p| p.is_file()));
    let usual = ["/usr/bin", "/usr/local/bin"].into_iter().map(PathBuf::from).chain(crate::folders::home_dir().map(|h| h.join(".local/bin")));
    on_path.or_else(|| usual.map(|d| d.join(id)).find(|p| p.is_file()))
}

pub(crate) fn binary_version(bin: &Path, app: &App) -> Option<version::Version> {
    let out = crate::process::capture(bin.to_str()?, &["--version"], Duration::from_secs(5))?;
    parse_cli_version(out.trim(), app)
}
