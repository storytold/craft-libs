//! Finding Craft Apps that are already installed, however they got there.
//!
//! - macOS: `<App>.app` in `/Applications` or `~/Applications`, read from its `Info.plist`.
//! - Windows: the Windows Installer's uninstall entries (`Learning Machines LLC`), and portable
//!   copies the launcher unpacked.
//! - Linux: AppImages in `~/Applications` (where the launcher and AppImageLauncher put them),
//!   `~/AppImages` (Gear Lever) and `~/.local/bin`, Debian/RPM packages, Flatpaks, and tarball
//!   installs.
//! - FreeBSD: tarball installs and `/usr/local/bin/<app>`.

use std::path::{Path, PathBuf};

use crate::{App, InstallSource, Installed, version};

use crate::manifest::Manifest;

/// One look at the system, shared by every app's detection (package lists are read once).
#[derive(Default)]
pub struct Snapshot {
    #[allow(dead_code)]
    packages: Vec<crate::packages::Package>,
}

impl Snapshot {
    /// Reads the system's package lists once (on Linux; elsewhere the snapshot is empty).
    pub fn take() -> Self {
        #[cfg(target_os = "linux")]
        {
            Self { packages: linux::packages() }
        }
        #[cfg(not(target_os = "linux"))]
        {
            Self::default()
        }
    }
}

/// What's installed of `app`, best source first.
pub fn detect(app: &App, manifest: &Manifest, snapshot: &Snapshot) -> Option<Installed> {
    let _ = snapshot;
    if let Some(found) = from_manifest(app, manifest) {
        return Some(found);
    }
    #[cfg(target_os = "macos")]
    return macos::detect(app);
    #[cfg(windows)]
    return windows::detect(app);
    #[cfg(target_os = "linux")]
    return linux::detect(app, snapshot);
    #[cfg(target_os = "freebsd")]
    return freebsd::detect(app);
    #[allow(unreachable_code)]
    None
}

fn from_manifest(app: &App, manifest: &Manifest) -> Option<Installed> {
    let rec = manifest.apps.get(app.id)?;
    if !rec.path.exists() {
        return None;
    }
    // On macOS the bundle itself is the truth (the app may have updated itself).
    #[cfg(target_os = "macos")]
    if rec.kind == crate::PackageKind::Dmg {
        return macos::read_bundle(app, &rec.path);
    }
    Some(Installed {
        version: version::parse(&rec.version),
        source: InstallSource::Launcher,
        location: rec.path.display().to_string(),
        package: None,
    })
}

/// Whether a file name is an AppImage of the app with asset stems `stems`, and the version its
/// name gives. Matches only the names a release AppImage ends up with: the release's own
/// (`photocraft-0.6.0-linux-x86_64.AppImage`) and the bare app name Gear Lever renames it to
/// (`photocraft.appimage`, version unknown). The extension's case doesn't matter.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub(crate) fn appimage_version(name: &str, stems: &[&str]) -> Option<Option<String>> {
    let split = name.len().checked_sub(".appimage".len())?;
    let (base, ext) = (name.get(..split)?, name.get(split..)?);
    if !ext.eq_ignore_ascii_case(".appimage") {
        return None;
    }
    if stems.iter().any(|s| base.eq_ignore_ascii_case(s)) {
        return Some(None);
    }
    let parsed = crate::platform::parse_asset_name(&format!("{base}.AppImage"), stems)?;
    (parsed.kind == crate::PackageKind::AppImage && parsed.token.starts_with("linux-")).then_some(Some(parsed.version))
}

/// Who our Windows Installer packages name as their publisher.
#[cfg_attr(not(windows), allow(dead_code))]
const PUBLISHER: &str = "Learning Machines LLC";

/// Whether an uninstall entry's `DisplayName` and `Publisher` are `app`'s, under any name its
/// installers have registered.
#[cfg_attr(not(windows), allow(dead_code))]
fn is_installer_entry(app: &App, display_name: &str, publisher: &str) -> bool {
    publisher == PUBLISHER && app.names().any(|n| n == display_name)
}

/// Where a Windows install of `app` may keep its exe, in the order to try: the entry's
/// `InstallLocation`, then `<App>` under each Program Files root for each name the app has had,
/// and in each folder `<stem>.exe` for each stem, current first. Doesn't touch the disk.
#[cfg_attr(not(windows), allow(dead_code))]
fn exe_candidates(app: &App, location: Option<&Path>, program_files: &[PathBuf]) -> Vec<PathBuf> {
    let in_program_files = program_files.iter().flat_map(|root| app.names().map(move |n| root.join(n)));
    let dirs = location.map(Path::to_path_buf).into_iter().chain(in_program_files);
    dirs.flat_map(|d| app.stems().map(move |s| d.join(format!("{s}.exe")))).collect()
}

/// Version from `<app> --version` output. Binaries print the machine id (`photocraft 0.7.0`),
/// the display name (`SoundCraft 0.5.0`), or a former id (`printcraft 0.2.1` for PdfCraft).
#[cfg_attr(not(any(target_os = "linux", target_os = "freebsd")), allow(dead_code))]
fn parse_cli_version(line: &str, app: &App) -> Option<version::Version> {
    let line = line.trim();
    for prefix in app.stems().chain(std::iter::once(app.name)) {
        if let Some(rest) = strip_cli_prefix(line, prefix)
            && let Some(tok) = rest.split_whitespace().next()
            && let Some(v) = version::parse(tok)
        {
            return Some(v);
        }
    }
    version::parse_package(line)
}

fn strip_cli_prefix<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    let (head, rest) = line.split_at_checked(prefix.len())?;
    if !head.eq_ignore_ascii_case(prefix) {
        return None;
    }
    match rest.chars().next() {
        None => Some(rest),
        Some(c) if c.is_ascii_whitespace() => Some(rest),
        _ => None,
    }
}

#[cfg(target_os = "macos")]
pub mod macos;

#[cfg(windows)]
pub mod windows;

/// Detection of Linux installs: AppImages in the usual folders and system packages.
#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "freebsd")]
pub mod freebsd;

#[cfg(test)]
mod tests;
