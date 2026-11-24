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

use crate::app::App;
use crate::platform::{Platform, pick_asset};
use crate::protocol::JobPhase;
use crate::release::Release;
use crate::version::{self, Version};
#[cfg(target_os = "macos")]
use crate::{InstallSource, Installed};
use crate::{PackageKind, Reason};

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

mod detect;
pub(crate) mod helper;
#[cfg(unix)]
mod relaunch;
mod replace;

#[cfg(target_os = "linux")]
use detect::{FlatpakLevel, flatpak_level};
#[cfg(not(windows))]
use helper::record_done;
#[cfg(windows)]
use helper::{Finish, PORTABLE_STAGING, msi_finish, windows_check_download, windows_helper, windows_precheck};
#[cfg(unix)]
use relaunch::relaunch_after_exit;
#[cfg(target_os = "linux")]
use relaunch::relaunch_flatpak_after_exit;
#[cfg(target_os = "linux")]
use replace::replace_appimage;
#[cfg(unix)]
use replace::replace_standalone;
use replace::unmark_unless_scheduled;
#[cfg(not(windows))]
use replace::{after_install, keep_download, mark_installed};

pub use detect::{DEV_VAR, PRETEND_VAR, current, current_version, is_self_updating, just_updated_to, release_installs};
#[cfg(test)]
pub(crate) use helper::record_failure;
#[cfg(feature = "ui")]
pub(crate) use helper::record_failure_reason;
pub use helper::{FINISH_FLAG, Outcome, ReasonOutcome, finish, finish_reason, take_failure, take_failure_reason, take_outcome, take_outcome_reason};

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
    update_reason(app, install, release, platform, http, paths, stop, phase, last, running_version).map_err(|reason| reason.to_string())
}

/// Structured counterpart retaining authored failure context.
#[allow(clippy::too_many_arguments)]
pub fn update_reason(
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
) -> Result<(), Reason> {
    prepare_update_reason(app, install, release, platform, http, paths, stop, phase, last, running_version)?.start_reason()
}

/// What remains of an update after [`prepare_update`]: starting the helper, which waits for this
/// process to exit. On Windows the helper then installs the update and starts the new version;
/// on macOS and Linux the update is already in place and the helper starts the new version.
#[must_use = "the update only installs or restarts once the restart is started"]
pub struct Restart {
    helper: Option<Box<dyn FnOnce() -> Result<(), Reason> + Send>>,
}

impl Restart {
    /// Nothing is left to start.
    #[cfg_attr(not(all(test, feature = "ui")), allow(dead_code))]
    pub(crate) fn scheduled() -> Self {
        Self { helper: None }
    }

    /// `start` runs when the app closes.
    #[cfg_attr(not(any(windows, unix, test)), allow(dead_code))]
    pub(crate) fn later(start: impl FnOnce() -> Result<(), String> + Send + 'static) -> Self {
        Self::later_reason(move || start().map_err(Reason::data))
    }

    /// Keeps an authored failure structured until the restart is attempted.
    pub(crate) fn later_reason(start: impl FnOnce() -> Result<(), Reason> + Send + 'static) -> Self {
        Self { helper: Some(Box::new(start)) }
    }

    /// Starts the helper. Call it when the app's exit is certain: the helper waits only a minute
    /// for the app to exit. On Windows it installs nothing when the app is still running after
    /// that; on macOS and Linux the Flatpak helper starts nothing then, and the other helpers
    /// start the new version anyway.
    pub fn start(self) -> Result<(), String> {
        self.start_reason().map_err(|reason| reason.to_string())
    }

    /// Starts the helper, retaining a structured failure for translation.
    pub fn start_reason(self) -> Result<(), Reason> {
        self.helper.map_or(Ok(()), |start| start())
    }
}

impl std::fmt::Debug for Restart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Restart").field("pending", &self.helper.is_some()).finish()
    }
}

/// Options for an explicitly opted-in background update. Other platforms keep their existing
/// installation behaviour; permission prompts are never suppressed.
#[derive(Clone, Copy, Debug, Default)]
pub struct UpdateOptions {
    /// Hide progress for a per-user Windows MSI. Machine installs retain their permission UI.
    pub quiet_msi: bool,
}

/// [`update`] without starting the helper: start the returned [`Restart`] when exit is certain.
/// On macOS and Linux the update is already in place when this returns.
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
    prepare_update_reason(app, install, release, platform, http, paths, stop, phase, last, running_version).map_err(|reason| reason.to_string())
}

/// Structured counterpart retaining authored failure context.
#[allow(clippy::too_many_arguments)]
pub fn prepare_update_reason(
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
) -> Result<Restart, Reason> {
    prepare_update_with_options_reason(app, install, release, platform, http, paths, stop, phase, last, running_version, UpdateOptions::default())
}

/// Like [`prepare_update`], with an explicit choice of Windows Installer progress UI.
#[allow(clippy::too_many_arguments)]
pub fn prepare_update_with_options(
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
    options: UpdateOptions,
) -> Result<Restart, String> {
    prepare_update_with_options_reason(app, install, release, platform, http, paths, stop, phase, last, running_version, options)
        .map_err(|reason| reason.to_string())
}

/// Structured counterpart retaining authored failure context.
#[allow(clippy::too_many_arguments)]
pub fn prepare_update_with_options_reason(
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
    options: UpdateOptions,
) -> Result<Restart, Reason> {
    // The option only changes Windows MSI helpers.
    #[cfg(not(windows))]
    let _ = options;
    let cur_v = running_version.and_then(|v| current_version(app, v));
    if let (Some(rel_v), Some(cur_v)) = (release.version.as_ref(), cur_v.as_ref())
        && !version::offers_update(rel_v, cur_v)
    {
        // Compare the effective version: an explicit pretend version also exercises installation.
        // Without that opt-in, a build from source requires a strictly higher release.
        return Err(Reason::new(
            "{arg0} {arg1} is not newer than currently running version {arg2}",
            &[("arg0", &(app.name).to_string()), ("arg1", &(release.version_label()).to_string()), ("arg2", &(cur_v).to_string())],
        ));
    }
    let (asset, _) = pick_asset(release, &[app.id], platform, &[install.kind()]).ok_or_else(|| {
        Reason::new(
            "{arg0} {arg1} has no {arg2} for {arg3}",
            &[
                ("arg0", &(app.name).to_string()),
                ("arg1", &(release.version_label()).to_string()),
                ("arg2", &(install.kind().label()).to_string()),
                ("arg3", &(platform.label()).to_string()),
            ],
        )
    })?;
    let asset = asset.clone();
    crate::install::bind_artifact_version(app, release, &asset)?;
    crate::install::check_download_link_reason(app, release, &asset)?;
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
    last().map_err(Reason::data)?;
    phase(JobPhase::Installing);
    // Written before the swap starts. On Windows the helper does the swap, so the record is
    // written when it starts (see `windows_helper`): it clears the record when it fails, which a
    // write after it started could undo.
    #[cfg(not(windows))]
    mark_installed(paths, release);
    let pid = std::process::id();
    let scheduled = (|| -> Result<Restart, Reason> {
        match install {
            #[cfg(target_os = "macos")]
            Install::MacApp(bundle) => {
                let existing = Installed {
                    version: None,
                    source: InstallSource::MacApp,
                    location: bundle.display().to_string(),
                    package: None,
                    externally_managed: false,
                    origin: None,
                    installation: None,
                };
                let wait = install::macos::CheckTurn { stop, phase: &mut *phase };
                let placed = install::macos::install_dmg_reason(
                    app,
                    &file,
                    Some(&existing),
                    None,
                    false,
                    paths,
                    &install::macos::Expect::self_update(release),
                    wait,
                );
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
                let result = install::linux::install_package_reason(*kind, &file, None, stop, phase);
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
                    None => {
                        return Err(Reason::new(
                            "this copy of {arg0} can't update itself; download the new version instead",
                            &[("arg0", &(app.name).to_string())],
                        ));
                    }
                };
                let result = install::linux::update_flatpak_bundle_reason(&file, &app.bundle_id(), system, stop, phase);
                after_install(paths, &file, recorded, &result);
                result?;
                relaunch_flatpak_after_exit(app)
            }
            #[cfg(windows)]
            Install::Msi(exe) => windows_helper(app, paths, release, pid, exe, &msi_finish(&file, options.quiet_msi)?),
            #[cfg(windows)]
            Install::Portable(exe) => {
                exe.parent().ok_or_else(|| Reason::new("{arg0} isn't in a folder", &[("arg0", &(app.name).to_string())]))?;
                windows_helper(app, paths, release, pid, exe, &Finish::Portable { zip: file, tmp: paths.cache.join(PORTABLE_STAGING) })
            }
            #[cfg(windows)]
            Install::Standalone(exe) => {
                exe.parent().ok_or_else(|| Reason::new("{arg0} isn't in a folder", &[("arg0", &(app.name).to_string())]))?;
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
                Err(Reason::new("this copy of {arg0} can't update itself; download the new version instead", &[("arg0", &(app.name).to_string())]))
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

#[cfg(test)]
pub(crate) mod tests;
