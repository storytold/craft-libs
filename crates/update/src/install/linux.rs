//! Linux installs: distro packages, Flatpak launcher tarball and AppImage.

use std::ffi::{OsStr, OsString};
use std::os::unix::fs::PermissionsExt;
use std::time::Duration;

use crate::distro;
use crate::protocol::CANCELLED;
use crate::version::{self, Version};

use super::{
    App, INSTALLER, InstallSource, Installed, JobPhase, PackageKind, Path, PathBuf, Paths, Record, SHARED_CALL, SYSTEM_BIN_DIRS, Stop, cancelled,
    find_in, pkexec, process, run_as_root, swap_into_place, tree, turn,
};

/// apt waits this long for another package tool (a software centre, automatic updates) to
/// let go of its lock, instead of failing at once.
const APT_WAITS: &str = "DPkg::Lock::Timeout=60";

/// A package tool that root commands can run, from the same directories `run_as_root` uses.
fn system_tool(name: &str) -> Option<PathBuf> {
    find_in(&SYSTEM_BIN_DIRS, OsStr::new(name))
}

/// Where commands that need root go: the installer's queue, then `pkexec`.
pub struct Root<'a> {
    pub queue: &'a crate::queue::Queue,
    pub run: &'a (dyn Fn(&[&OsStr]) -> Result<(), String> + Sync),
    pub flatpak: &'a (dyn Fn() -> std::process::Command + Sync),
}

const SYSTEM: Root<'static> = Root { queue: &INSTALLER, run: &run_as_root, flatpak: &process::flatpak_command_c };

/// Runs `command target` as root. Jobs waiting for the same command go into one call
/// (`apt-get install a.deb b.deb`), so the password is asked for once.
fn as_root(root: &Root<'_>, command: &[&str], target: &OsStr, stop: &Stop, phase: &mut dyn FnMut(JobPhase), working: JobPhase) -> Result<(), String> {
    let command = command.iter().map(OsString::from).collect();
    root.queue.batched(command, target.to_os_string(), &cancelled(stop), &mut turn(phase, working), &|command, targets| {
        let args: Vec<&OsStr> = command.iter().chain(targets).map(OsString::as_os_str).collect();
        shared_outcome((root.run)(&args), targets.len())
    })
}

/// What one call's result means for each of the `count` apps it handled: a failure can't be
/// pinned on one of them, and trying each again by itself would ask for the password each time.
pub fn shared_outcome(result: Result<(), String>, count: usize) -> Result<(), String> {
    match result {
        Err(e) if count > 1 && e != CANCELLED => Err(format!("{e} while handling {count} {SHARED_CALL}")),
        other => other,
    }
}

/// The command that installs (or upgrades) a downloaded system package.
pub fn install_command(kind: PackageKind) -> Option<&'static [&'static str]> {
    match kind {
        // apt resolves dependencies and records the package like any other.
        PackageKind::Deb => Some(&["apt-get", "-o", APT_WAITS, "install", "-y", "--allow-downgrades"]),
        PackageKind::Rpm if system_tool("dnf").is_some() => Some(&["dnf", "install", "-y"]),
        PackageKind::Rpm if system_tool("zypper").is_some() => Some(&["zypper", "--non-interactive", "install", "--allow-unsigned-rpm"]),
        PackageKind::Rpm => Some(&["rpm", "-U", "--replacepkgs"]),
        PackageKind::ArchPkg => Some(&["pacman", "-U", "--noconfirm"]),
        _ => None,
    }
}

/// Fedora Atomic (Silverblue, Kinoite, Bazzite…): rpm-ostree manages the root filesystem and
/// `dnf install` of a local package is refused (Bazzite wraps `/usr/bin/dnf` to say so).
fn immutable_ostree() -> bool {
    Path::new("/run/ostree-booted").exists()
}

/// What this system can install, ordered with the best default first.
///
/// The default package format aligns with how the user installed the launcher itself:
/// - Running as a Flatpak: prefers Flatpak (per-user, desktop integration). When the host
///   has no usable Flatpak (none installed, or `flatpak-spawn` refused) it prefers the
///   release tarball instead, which the launcher unpacks and runs inside the sandbox; an
///   AppImage cannot mount there and a distro package needs root, so both are dead ends.
/// - Running as an AppImage: prefers AppImage.
/// - Otherwise (system install): prefers the distro's own package format (`.deb` on the
///   Debian family, `.rpm` on the Fedora, Red Hat and SUSE families, `.pkg.tar.zst` on Arch,
///   read from `os-release` rather than from whichever tools are on `PATH`), then Flatpak if
///   available, then AppImage.
pub fn available_kinds() -> Vec<PackageKind> {
    let flatpak = process::is_flatpak_available();
    let is_flatpak = process::in_sandbox();
    let is_appimage = std::env::var_os("APPIMAGE").is_some();
    let root = pkexec().is_some();
    let os = ["/etc/os-release", "/usr/lib/os-release"].iter().find_map(|p| std::fs::read_to_string(p).ok()).unwrap_or_default();
    let native = distro::native_package(&distro::parse_os_release(&os));
    // An ostree system can't take .deb or .rpm installs, whatever os-release says.
    let root = root && !immutable_ostree();
    let can_deb = root && native == Some(PackageKind::Deb) && system_tool("apt-get").is_some() && system_tool("dpkg").is_some();
    let can_rpm = root && native == Some(PackageKind::Rpm) && system_tool("rpm").is_some();
    let can_arch = root && native == Some(PackageKind::ArchPkg) && system_tool("pacman").is_some() && crate::detect::linux::has_pacman_db();
    order_available_kinds(is_flatpak, is_appimage, flatpak, can_deb, can_rpm, can_arch)
}

pub(crate) fn order_available_kinds(
    is_flatpak: bool,
    is_appimage: bool,
    flatpak_available: bool,
    can_deb: bool,
    can_rpm: bool,
    can_arch: bool,
) -> Vec<PackageKind> {
    let mut kinds = Vec::new();
    if is_flatpak && flatpak_available {
        kinds.push(PackageKind::Flatpak);
    } else if is_flatpak {
        kinds.push(PackageKind::TarGz);
    } else if is_appimage {
        kinds.push(PackageKind::AppImage);
    }

    if can_deb {
        kinds.push(PackageKind::Deb);
    }
    if can_rpm {
        kinds.push(PackageKind::Rpm);
    }
    if can_arch {
        kinds.push(PackageKind::ArchPkg);
    }

    if flatpak_available && !kinds.contains(&PackageKind::Flatpak) {
        kinds.push(PackageKind::Flatpak);
    }
    // The tarball runs inside the sandbox, so it is only ever offered there.
    if is_flatpak && !kinds.contains(&PackageKind::TarGz) {
        kinds.push(PackageKind::TarGz);
    }
    if !kinds.contains(&PackageKind::AppImage) {
        kinds.push(PackageKind::AppImage);
    }
    kinds
}

/// Flags for `flatpak install` of one bundle. A per-user install stays noninteractive.
/// A system-wide install is also noninteractive: it runs under `pkexec`, which asks for the
/// password. Flatpak's own install-bundle action does not. Its polkit message needs `$(path)`,
/// and current polkit denies the action when that detail is missing, with no dialog.
pub(crate) fn flatpak_bundle_flags(system: bool) -> &'static [&'static str] {
    if system {
        &["install", "--system", "-y", "--noninteractive", "--reinstall", "--bundle"]
    } else {
        &["install", "--user", "-y", "--noninteractive", "--reinstall", "--bundle"]
    }
}

/// Program prefix for a system-wide bundle reinstall. Inside the sandbox the host runs it,
/// because this process cannot see the host's `pkexec` or write `/var/lib/flatpak`.
pub(crate) fn system_bundle_prefix(in_sandbox: bool) -> &'static [&'static str] {
    if in_sandbox { &["flatpak-spawn", "--host", "pkexec", "/usr/bin/flatpak"] } else { &["pkexec", "/usr/bin/flatpak"] }
}

fn system_bundle_command(file: &Path) -> Result<std::process::Command, String> {
    if !process::in_sandbox() && pkexec().is_none() {
        return Err("installing system-wide needs polkit (pkexec), which isn't installed".into());
    }
    let prefix = system_bundle_prefix(process::in_sandbox());
    let Some((program, rest)) = prefix.split_first() else {
        return Err("couldn't start Flatpak".into());
    };
    let mut cmd = process::command(*program);
    cmd.args(rest);
    cmd.args(flatpak_bundle_flags(true));
    cmd.arg(file);
    Ok(cmd)
}

pub(crate) fn flatpak_bundle_error(output: &std::process::Output) -> Result<(), String> {
    if output.status.success() {
        return Ok(());
    }
    // pkexec: 126 = the password prompt was dismissed, 127 = not authorized.
    if matches!(output.status.code(), Some(126 | 127)) {
        let detail = String::from_utf8_lossy(&output.stderr);
        log::warn!("flatpak bundle install ended with {:?}: {}", output.status.code(), detail.trim());
        return Err(CANCELLED.into());
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    let err = stderr.trim();
    let err = if err.is_empty() { String::from_utf8_lossy(&output.stdout).trim().to_string() } else { err.to_string() };
    if err.is_empty() {
        Err("Flatpak couldn't install the bundle (it needs the Flathub remote for its runtime)".into())
    } else {
        let clean_err = err.strip_prefix("error: ").or_else(|| err.strip_prefix("Error: ")).unwrap_or(&err);
        Err(format!("Flatpak couldn't install the bundle: {clean_err}"))
    }
}

/// Puts a self-update's `.flatpak` bundle in place. `system` replaces the system-wide copy
/// through `pkexec`. Per-user stays a normal `flatpak install --user`, which needs no password.
pub(crate) fn update_flatpak_bundle(file: &Path, id: &str, system: bool, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    update_flatpak_bundle_via(&SYSTEM, file, id, system, stop, phase)
}

/// An app never uninstalls itself here: its download goes after this, so a failed install would
/// leave nothing. "Already installed" means an earlier update landed and never restarted.
pub(crate) fn update_flatpak_bundle_via(
    root: &Root<'_>,
    file: &Path,
    id: &str,
    system: bool,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
) -> Result<(), String> {
    match install_flatpak_bundle_via(root, file, None, system, stop, phase) {
        Err(e) if already_installed(&e, id) => {
            log::info!("{id} is already installed: only the restart is left");
            Ok(())
        }
        result => result,
    }
}

/// Installs a downloaded `.flatpak` bundle. `bundle` is the app id it holds, for a per-user install
/// that may uninstall that app first.
fn install_flatpak_bundle_via(
    root: &Root<'_>,
    file: &Path,
    bundle: Option<&str>,
    system: bool,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
) -> Result<(), String> {
    let install_one = || {
        log::info!("executing flatpak install --reinstall --bundle for {}", file.display());
        let mut cmd = if system {
            system_bundle_command(file)?
        } else {
            // The C locale keeps the "already installed" error below readable.
            let mut cmd = (root.flatpak)();
            cmd.args(flatpak_bundle_flags(false));
            cmd.arg(file);
            cmd
        };
        let output = cmd.output().map_err(|e| format!("couldn't start Flatpak ({e})"))?;
        if output.status.success() {
            log::info!("flatpak install succeeded for {}", file.display());
        }
        flatpak_bundle_error(&output)
    };
    // Inside the queue: an uninstall racing another bundle install can drop its remote.
    let install = || {
        let result = install_one();
        match bundle {
            Some(id) if result.as_ref().is_err_and(|e| already_installed(e, id)) && reinstall_by_hand(root) => {
                log::info!("{id} is already installed and this Flatpak can't reinstall a bundle: uninstalling it first");
                // No `--delete-data`: the app's files stay for the next install.
                if !(root.flatpak)().args(["uninstall", "--user", "-y", "--noninteractive", id]).status().is_ok_and(|s| s.success()) {
                    log::warn!("couldn't uninstall {id} to install its bundle again");
                    return result;
                }
                install_one()
                    .map_err(|e| format!("{e} (It was uninstalled first. Its data is kept, and Install tries again with the same download.)"))
            }
            _ => result,
        }
    };
    root.queue.alone(&cancelled(stop), &mut turn(phase, JobPhase::Installing), install).unwrap_or(Err(CANCELLED.into()))
}

/// Whether a bundle install of `id` failed because the installation already has the bundle's
/// commit: Flatpak's "already installed", in the C locale. Any other failure leaves the app alone.
pub(crate) fn already_installed(err: &str, id: &str) -> bool {
    err.contains(&format!("{id} already installed"))
}

/// Whether the user's copy has to be uninstalled before its bundle installs again: Flatpak
/// before 1.16.2 ignores `--reinstall` for a bundle.
fn reinstall_by_hand(root: &Root<'_>) -> bool {
    let mut cmd = (root.flatpak)();
    cmd.arg("--version");
    process::run_command(cmd, "flatpak", Duration::from_secs(10)).is_ok_and(|v| ignores_bundle_reinstall(&v))
}

/// Whether `flatpak --version` names a Flatpak that ignores `--reinstall` for a bundle (flatpak
/// commit 4c5693b8 added it in 1.16.2).
pub(crate) fn ignores_bundle_reinstall(flatpak_version: &str) -> bool {
    flatpak_version.split_whitespace().last().and_then(version::parse).is_some_and(|v| v < Version::new(1, 16, 2))
}

/// Installs (or upgrades) a downloaded package with the system's own tools. `bundle` is the app
/// id a Flatpak bundle holds.
pub fn install_package(kind: PackageKind, file: &Path, bundle: Option<&str>, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    if kind == PackageKind::Deb && system_tool("apt-get").is_none() {
        return Err("this system has no apt-get to install a Debian package with".into());
    }
    if kind == PackageKind::ArchPkg && system_tool("pacman").is_none() {
        return Err("this system has no pacman to install an Arch package with".into());
    }
    install_package_via(&SYSTEM, kind, file, bundle, stop, phase)
}

pub fn install_package_via(
    root: &Root<'_>,
    kind: PackageKind,
    file: &Path,
    bundle: Option<&str>,
    stop: &Stop,
    phase: &mut dyn FnMut(JobPhase),
) -> Result<(), String> {
    let f = file.as_os_str();
    if let Some(command) = install_command(kind) {
        return as_root(root, command, f, stop, phase, JobPhase::Installing);
    }
    match kind {
        PackageKind::Flatpak => install_flatpak_bundle_via(root, file, bundle, false, stop, phase),
        other => Err(format!("{} isn't a system package", other.label())),
    }
}

pub fn remove_package(app: &App, installed: &Installed, stop: &Stop, phase: &mut dyn FnMut(JobPhase)) -> Result<(), String> {
    let mut remove = |command: &[&str], package: &str| as_root(&SYSTEM, command, OsStr::new(package), stop, phase, JobPhase::Removing);
    match installed.source {
        InstallSource::Deb if system_tool("apt-get").is_some() => remove(&["apt-get", "-o", APT_WAITS, "remove", "-y"], app.id),
        InstallSource::Deb => remove(&["dpkg", "-r"], app.id),
        InstallSource::Rpm => {
            if system_tool("dnf").is_some() {
                remove(&["dnf", "remove", "-y"], app.id)
            } else {
                remove(&["rpm", "-e"], app.id)
            }
        }
        // The package detected (photocraft, photocraft-bin or photocraft-git).
        InstallSource::Pacman => remove(&["pacman", "-R", "--noconfirm"], installed.package.as_deref().unwrap_or(app.id)),
        InstallSource::Flatpak => {
            let id = app.bundle_id();
            let user = process::flatpak_command().args(["uninstall", "--user", "-y", "--noninteractive", &id]).status();
            if user.is_ok_and(|s| s.success()) {
                return Ok(());
            }
            if process::in_sandbox() {
                let system = process::flatpak_command().args(["uninstall", "--system", "-y", "--noninteractive", &id]).status();
                if system.is_ok_and(|s| s.success()) {
                    return Ok(());
                }
                return Err(format!("couldn't uninstall {id}"));
            }
            remove(&["flatpak", "uninstall", "--system", "-y", "--noninteractive"], &id)
        }
        other => Err(format!("{} isn't a system package", other.label())),
    }
}

pub fn install_appimage(
    file: &Path,
    name: &str,
    destination: Option<&Path>,
    previous: Option<&Record>,
    replace: bool,
) -> Result<(PathBuf, Vec<PathBuf>), String> {
    let dir = match destination {
        Some(dir) => dir.to_path_buf(),
        None => {
            let dir = crate::detect::linux::appimage_dir().ok_or("no home folder")?;
            std::fs::create_dir_all(&dir).map_err(|e| format!("couldn't create {} ({e})", dir.display()))?;
            dir
        }
    };
    let dest = dir.join(name);
    let ours = previous.is_some_and(|r| r.kind == PackageKind::AppImage && r.path == dest);
    if std::fs::symlink_metadata(&dest).is_ok_and(|m| m.is_dir() || !(replace || ours && m.is_file())) {
        return Err(format!("{} already exists and wasn't installed by ArtCraft Launcher", dest.display()));
    }
    let staging = super::staging_dir(&dir)?;
    let partial = staging.path().join(name);
    std::fs::copy(file, &partial).map_err(|e| format!("couldn't copy the AppImage ({e})"))?;
    // The AppImage is the program the user just installed and must be executable; 0o755 is
    // not group/world-writable.
    std::fs::set_permissions(&partial, std::fs::Permissions::from_mode(0o755)).map_err(|e| e.to_string())?; // NOSONAR
    std::fs::rename(&partial, &dest).map_err(|e| format!("couldn't put the AppImage in place ({e})"))?;
    let mut removed = Vec::new();
    if let Some(old) = previous.filter(|r| r.kind == PackageKind::AppImage && r.path != dest)
        && std::fs::remove_file(&old.path).is_ok()
    {
        removed.push(old.path.clone());
    }
    Ok((dest, removed))
}

/// Unpacks a release tarball (`bin/<app>`, `share/…`) into `<data>/apps/<app>`, a folder
/// only the launcher owns, and returns the app's binary. This is the fallback for the
/// Flatpak launcher when the host has no usable Flatpak: the app then runs inside the
/// sandbox instead of on the host. `bin` and `share` are swapped in one at a time, so an
/// update leaves the app's own `home/` beside them untouched.
pub fn install_tree(app: &App, file: &Path, paths: &Paths) -> Result<PathBuf, String> {
    let staged = paths.cache.join(format!("{}.new", app.id));
    let result = unpack_tree(app, file, &staged, paths);
    let _ = std::fs::remove_dir_all(&staged);
    result
}

fn unpack_tree(app: &App, file: &Path, staged: &Path, paths: &Paths) -> Result<PathBuf, String> {
    let tree = tree::unpack(app, file, staged)?;
    // Verify tree root is strictly within staged and is not a symlink (CWE-59).
    let rel_tree = tree.strip_prefix(staged).map_err(|_| "staging layout escapes staging".to_string())?;
    let mut check_root = staged.to_path_buf();
    for part in rel_tree.components() {
        check_root.push(part);
        let meta = std::fs::symlink_metadata(&check_root).map_err(|e| format!("couldn't check {} ({e})", check_root.display()))?;
        if meta.file_type().is_symlink() {
            return Err("unpacked tree root is a symlink".into());
        }
    }
    // Validate the staged binary and set its mode *before* either swap: a rejected update
    // (e.g. a symlinked app binary) must never disturb the installed tree it replaces.
    let staged_bin = tree.join("bin").join(app.id);
    make_runnable_within(&tree, &staged_bin)?;
    let root = paths.apps().join(app.id);
    std::fs::create_dir_all(&root).map_err(|e| format!("couldn't create {} ({e})", root.display()))?;
    for dir in ["bin", "share"] {
        if tree.join(dir).is_dir() {
            swap_into_place(&tree.join(dir), &root.join(dir))?;
        }
    }
    Ok(root.join("bin").join(app.id))
}

/// Makes `bin` executable (0755) without ever following an archive-controlled symlink out of
/// the installation tree.
///
/// `bin` comes from an untrusted tarball. If it, or one of its ancestors inside `root`, is a
/// symlink, a plain `std::fs::set_permissions(bin, …)` would follow the link and change the
/// mode of an existing file *outside* the tree (CWE-59). We reject any symlinked component
/// before changing permissions, so the chmod can only ever reach a regular file that lives at
/// the verified path inside the launcher's own apps folder.
fn make_runnable_within(root: &Path, bin: &Path) -> Result<(), String> {
    let root_meta = std::fs::symlink_metadata(root).map_err(|e| format!("couldn't check {} ({e})", root.display()))?;
    if root_meta.file_type().is_symlink() {
        return Err("the install tree root is a symlink".into());
    }
    // No component between `root` and `bin` may be a symlink.
    let rel = bin.strip_prefix(root).map_err(|_| "the app binary isn't inside the install tree".to_string())?;
    let mut here = root.to_path_buf();
    for part in rel.components() {
        here.push(part);
        let meta = std::fs::symlink_metadata(&here).map_err(|e| format!("couldn't check {} ({e})", here.display()))?;
        if meta.file_type().is_symlink() {
            return Err("the download's app binary escapes the install tree through a symlink".into());
        }
    }
    // The final component is a verified non-symlink regular file inside the launcher's own
    // apps folder, so changing its mode cannot reach anything outside the tree.
    if !here.metadata().map_err(|e| e.to_string())?.file_type().is_file() {
        return Err("the download's app binary isn't a regular file".into());
    }
    // Verified non-symlink regular file inside the launcher's apps folder; the app binary must
    // be executable and 0o755 is not group/world-writable.
    std::fs::set_permissions(bin, std::fs::Permissions::from_mode(0o755)).map_err(|e| format!("couldn't make {} runnable ({e})", bin.display())) // NOSONAR
}

/// Removes a tree the launcher unpacked in [`install_tree`], keeping the app's `home/`: the
/// settings and projects in it are the user's, not the installer's. Only the two folders the
/// launcher put there are removed, and only inside the launcher's own apps folder.
pub fn remove_tree(app: &App, paths: &Paths) -> Result<(), String> {
    let root = paths.apps().join(app.id);
    let mut removed = false;
    for dir in ["bin", "share"] {
        let path = root.join(dir);
        if path.starts_with(paths.apps()) && path.is_dir() {
            std::fs::remove_dir_all(&path).map_err(|e| format!("couldn't remove {} ({e})", path.display()))?;
            removed = true;
        }
    }
    if removed { Ok(()) } else { Err(format!("{} isn't installed in this folder", app.name)) }
}

/// Where menu entries and icons go: `$XDG_DATA_HOME`, else `~/.local/share`. Inside the
/// launcher's own Flatpak that variable points into the sandbox, so the host's is used.
pub fn host_data_dir() -> Option<PathBuf> {
    crate::folders::host_data_local_dir()
}

/// The app's 128 px icon, as bundled in `assets/app-icons`.
fn icon_png(app: &App) -> Option<&'static [u8]> {
    app.icon_png
}

fn desktop_path(data: &Path, app: &App) -> PathBuf {
    data.join("applications").join(format!("{}.desktop", app.bundle_id()))
}

fn icon_path(data: &Path, app: &App) -> PathBuf {
    data.join("icons/hicolor/128x128/apps").join(format!("{}.png", app.bundle_id()))
}

/// Adds `appimage` to the desktop's menu. Best effort: a failure here never fails the
/// install. Returns the files written, for the record.
pub fn integrate(app: &App, appimage: &Path, replaced: &[PathBuf], previous: Option<&Record>) -> Vec<PathBuf> {
    match host_data_dir() {
        Some(data) => integrate_into(&data, app, appimage, replaced, previous),
        None => Vec::new(),
    }
}

/// [`integrate`] into the data folder `data`. Only writes over files the launcher wrote
/// before: its own marked menu entry, an entry pointing at a copy this install replaced, and
/// an icon listed in `previous`.
pub(crate) fn integrate_into(data: &Path, app: &App, appimage: &Path, replaced: &[PathBuf], previous: Option<&Record>) -> Vec<PathBuf> {
    let ours_before = |p: &Path| previous.is_some_and(|r| r.files.iter().any(|f| f == p));
    let mut written = Vec::new();
    let icon = icon_path(data, app);
    let has_icon = match icon_png(app) {
        Some(png) if !icon.exists() || ours_before(&icon) => match crate::paths::write_atomic(&icon, png) {
            Ok(()) => {
                written.push(icon.clone());
                true
            }
            Err(e) => {
                log::warn!("{}: couldn't install the menu icon: {e}", app.name);
                false
            }
        },
        // Someone else's icon under the app's id (the app's own AppImage puts one there).
        _ => icon.is_file(),
    };
    let Some(text) = crate::desktop_entry::render(app, appimage, has_icon) else {
        log::warn!("{}: {} can't be written into a menu entry; skipping it", app.name, appimage.display());
        return written;
    };
    let desktop = desktop_path(data, app);
    let replace = match std::fs::read_to_string(&desktop) {
        Ok(old) => {
            crate::desktop_entry::is_ours(&old)
                || crate::desktop_entry::exec_program(&old).is_some_and(|p| Path::new(&p) == appimage || replaced.iter().any(|r| r == Path::new(&p)))
        }
        Err(_) => !desktop.exists(),
    };
    if !replace {
        log::info!("{}: leaving the existing menu entry {} alone", app.name, desktop.display());
        return written;
    }
    match crate::paths::write_atomic(&desktop, text.as_bytes()) {
        Ok(()) => written.push(desktop),
        Err(e) => log::warn!("{}: couldn't add the menu entry: {e}", app.name),
    }
    written
}

/// Takes away what [`integrate`] added for the removed AppImage `appimage`.
pub fn unintegrate(app: &App, appimage: &Path, record: Option<&Record>) {
    if let Some(data) = host_data_dir() {
        unintegrate_from(&data, app, appimage, record);
    }
}

/// [`unintegrate`] from the data folder `data`: the menu entry when it runs the removed copy,
/// or when the launcher wrote it and what it runs is gone; the icon only when the
/// launcher's record lists it.
pub(crate) fn unintegrate_from(data: &Path, app: &App, appimage: &Path, record: Option<&Record>) {
    let desktop = desktop_path(data, app);
    if let Ok(text) = std::fs::read_to_string(&desktop) {
        let program = crate::desktop_entry::exec_program(&text).map(PathBuf::from);
        let runs_removed = program.as_deref() == Some(appimage);
        let ours_and_dangling = crate::desktop_entry::is_ours(&text) && program.is_none_or(|p| !p.exists());
        if runs_removed || ours_and_dangling {
            let _ = std::fs::remove_file(&desktop);
        }
    }
    let icon = icon_path(data, app);
    if record.is_some_and(|r| r.files.contains(&icon)) {
        let _ = std::fs::remove_file(&icon);
    }
}
