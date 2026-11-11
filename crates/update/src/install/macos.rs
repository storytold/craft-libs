//! macOS installs: the signed `.app` from the release DMG.

use std::time::Duration;

use crate::process::HelperFailure;
use crate::protocol::CANCELLED;
use crate::queue::Queue;
use crate::version::Version;

use super::{App, InstallSource, Installed, JobPhase, Path, PathBuf, Paths, Release, Stop, cancelled, not_running, process, swap_into_place, turn};

/// `/Applications` when we can write there, else `~/Applications`.
fn applications_dir(existing: Option<&Installed>, destination: Option<&Path>) -> Result<PathBuf, String> {
    if let Some(parent) = existing.filter(|i| i.source == InstallSource::MacApp).and_then(|i| Path::new(&i.location).parent().map(Path::to_path_buf))
    {
        return Ok(parent);
    }
    if let Some(dir) = destination {
        return Ok(dir.to_path_buf());
    }
    let system = PathBuf::from("/Applications");
    let probe = system.join(".artcraft-launcher-write-test");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        return Ok(system);
    }
    let user = crate::folders::home_dir().ok_or("no home folder")?.join("Applications");
    std::fs::create_dir_all(&user).map_err(|e| format!("couldn't create {} ({e})", user.display()))?;
    Ok(user)
}

/// The Apple Developer team that signs every Craft App and ArtCraft Launcher (Learning
/// Machines LLC): the macOS side of the Windows publisher check.
pub const TEAM_ID: &str = "DJ6XS33FX8";

/// The start of [`install_dmg`]'s error when hdiutil can't attach the disk image.
pub(crate) const IMAGE_NOT_OPENED: &str = "couldn't open the disk image";

/// What a new copy must prove before it replaces anything. The checksum shows the download is
/// the file the release lists; these show the app inside is the one the release promises.
pub struct Expect<'a> {
    /// The team whose Developer ID signature the bundle must carry. `None` only in tests,
    /// which build unsigned bundles.
    pub team: Option<&'a str>,
    /// The release's version, which the bundle must report. `None` when the tag isn't one.
    pub version: Option<&'a Version>,
    /// Refuse the swap when the app it replaces is running. `false` for an app replacing
    /// itself, which is running on purpose.
    pub quit_first: bool,
}

impl<'a> Expect<'a> {
    pub fn release(release: &'a Release) -> Self {
        Self { team: Some(TEAM_ID), version: release.version.as_ref(), quit_first: true }
    }

    /// [`Self::release`] for an app replacing itself.
    pub fn self_update(release: &'a Release) -> Self {
        Self { quit_first: false, ..Self::release(release) }
    }
}

/// Held while this process asks diskimages-helper for something (an attach here; the tests'
/// `hdiutil create` too), since it turns concurrent requests away.
pub(crate) static DISK_IMAGES: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Deep signature checks wait their turn here. `codesign --deep` reads every file of the
/// bundle, so checks that overlap compete for the disk and all slow down. The time limit
/// starts when a check's turn comes.
static SIGNATURE_CHECKS: Queue = Queue::new();

/// How a job waits for its turn at the signature check: `phase` hears of the wait, and `stop`
/// can cancel it.
pub struct CheckTurn<'a> {
    pub stop: &'a Stop,
    pub phase: &'a mut dyn FnMut(JobPhase),
}

/// Apple's tools, by full path: a launcher started from Finder or at login gets a minimal
/// `PATH`, and nothing earlier on it should stand in for these.
const HDIUTIL: &str = "/usr/bin/hdiutil";
const DITTO: &str = "/usr/bin/ditto";
const CODESIGN: &str = "/usr/bin/codesign";

/// Stops a `codesign` that hangs.
const CODESIGN_TIMEOUT: Duration = Duration::from_secs(600);

#[allow(clippy::too_many_arguments)]
pub fn install_dmg(
    app: &App,
    dmg: &Path,
    existing: Option<&Installed>,
    destination: Option<&Path>,
    replace: bool,
    paths: &Paths,
    expect: &Expect<'_>,
    wait: CheckTurn<'_>,
) -> Result<PathBuf, String> {
    // A new, empty mount point for every install. A fixed one can still hold an image from an
    // attach that was interrupted or timed out, and hdiutil won't mount another over it.
    let attempt = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or_default();
    let mount = paths.cache.join(format!("mount-{}-{}-{attempt}", app.id, std::process::id()));
    let _in_use = InUse::new(&mount);
    std::fs::create_dir_all(&mount).map_err(|e| format!("couldn't create a folder to open the disk image in ({e})"))?;
    let (Some(dmg_s), Some(mount_s)) = (dmg.to_str(), mount.to_str()) else {
        let _ = std::fs::remove_dir(&mount);
        return Err("unusual characters in the cache folder's path".into());
    };
    // diskimages-helper refuses an attach while it is busy with another image ("Resource
    // temporarily unavailable"): attach one image at a time (several apps may be updating at
    // once), and wait a little and try again when something outside the launcher holds it.
    // The download's SHA-256 is already checked, so hdiutil's own checksum pass (-noverify)
    // would only add minutes on a large image.
    let attached = {
        let _one_at_a_time = DISK_IMAGES.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        // An image an earlier install left attached makes hdiutil refuse the same file.
        clear_mounts(app, &paths.cache, process_ended(), detach);
        let attach = || {
            process::run(
                HDIUTIL,
                &["attach", "-nobrowse", "-readonly", "-noautoopen", "-noverify", "-mountpoint", mount_s, dmg_s],
                Duration::from_secs(300),
            )
        };
        // diskimages-helper can stay busy for a while on a shared runner; a minute of retries is still quick.
        let mut attached = attach();
        for wait in [1, 2, 4, 8, 16, 32] {
            if attached.is_ok() {
                break;
            }
            let _ = detach(&mount);
            let _ = detach_unmounted(dmg);
            std::thread::sleep(Duration::from_secs(wait));
            attached = attach();
        }
        attached
    };
    if let Err(reason) = attached {
        let mut reason = format!("{IMAGE_NOT_OPENED}: {reason}");
        if let Err(why) = detach_unmounted(dmg) {
            log::warn!("couldn't detach the disk image {} ({why})", dmg.display());
            reason = format!("{reason}; the disk image may still be attached ({why})");
        }
        // An attach that timed out can still finish mounting the image.
        return close_image(Err(reason), &mount, detach);
    }
    let result = copy_app(app, &mount, existing, destination, replace, expect, wait);
    close_image(result, &mount, detach)
}

/// Detaches the image at `mount` through `detach` and removes the mount point. An image that
/// stays attached is logged, and the error of a failed install says so after its reason. A
/// later install detaches it.
pub(crate) fn close_image(result: Result<PathBuf, String>, mount: &Path, detach: impl Fn(&Path) -> Result<(), String>) -> Result<PathBuf, String> {
    if let Err(why) = detach(mount) {
        log::warn!("couldn't detach the disk image at {} ({why})", mount.display());
        return result.map_err(|reason| format!("{reason}; the disk image is still attached at {} ({why})", mount.display()));
    }
    if let Err(e) = std::fs::remove_dir(mount) {
        log::warn!("couldn't remove the empty folder {} ({e})", mount.display());
    }
    result
}

fn copy_app(
    app: &App,
    mount: &Path,
    existing: Option<&Installed>,
    destination: Option<&Path>,
    replace: bool,
    expect: &Expect<'_>,
    wait: CheckTurn<'_>,
) -> Result<PathBuf, String> {
    let bundle = format!("{}.app", app.name);
    let src = mount.join(&bundle);
    crate::detect::macos::read_bundle(app, &src).ok_or("the disk image doesn't contain the app")?;
    let dir = applications_dir(existing, destination)?;
    let dest = existing.filter(|i| i.source == InstallSource::MacApp).map_or_else(|| dir.join(&bundle), |i| PathBuf::from(&i.location));
    if !replace
        && std::fs::symlink_metadata(&dest).is_ok()
        && (std::fs::symlink_metadata(&dest).is_ok_and(|m| m.file_type().is_symlink()) || crate::detect::macos::read_bundle(app, &dest).is_none())
    {
        return Err(format!("{} already exists and wasn't installed by ArtCraft Launcher", dest.display()));
    }
    clear_staging(app, &dir, process_ended(), std::time::SystemTime::now());
    let staging = staging_dir(&dir)?;
    let result = place(app, &src, staging.path(), &dest, expect, wait);
    remove_staging(result, staging)
}

/// Copies the app at `src` into `staging`, checks the copy and replaces `dest` with it.
fn place(app: &App, src: &Path, staging: &Path, dest: &Path, expect: &Expect<'_>, wait: CheckTurn<'_>) -> Result<PathBuf, String> {
    let staged = staging.join(format!("{}.app", app.name));
    let (Some(src_s), Some(staged_s)) = (src.to_str(), staged.to_str()) else { return Err("unusual characters in a path".into()) };
    // ditto keeps the code signature and symlinks intact. The disk image's volume adds Finder
    // metadata (com.apple.FinderInfo) to the files; leave it and the extended attributes
    // behind, or `codesign --verify --strict` rejects the bundle as carrying "detritus". A
    // bundle's signature lives in its files, not in extended attributes.
    process::run(DITTO, &["--norsrc", "--noextattr", "--noacl", src_s, staged_s], Duration::from_secs(600))
        .map_err(|reason| format!("couldn't copy the app: {reason}"))?;
    place_checked(app, &staged, dest, expect, process::run_detailed, process::running, wait)
}

/// Checks the copy at `staged` and replaces `dest` with it. `codesign` runs through `run`, and
/// `running` says whether `dest` is running. Tests pass stand-ins.
pub(crate) fn place_checked(
    app: &App,
    staged: &Path,
    dest: &Path,
    expect: &Expect<'_>,
    run: impl FnOnce(&str, &[&str], Duration) -> Result<String, HelperFailure>,
    running: impl FnOnce(&Path) -> Result<bool, String>,
    wait: CheckTurn<'_>,
) -> Result<PathBuf, String> {
    let found = crate::detect::macos::read_bundle(app, staged).ok_or("the copied app is incomplete")?;
    check(staged, &found, expect, run, wait)?;
    // The check can wait behind other apps' checks, and the user may have opened the app.
    if expect.quit_first {
        not_running(app, running(dest))?;
    }
    swap_into_place(staged, dest)?;
    Ok(dest.to_path_buf())
}

/// Removes `staging` with whatever is still in it: the new copy when the install failed. A
/// folder that can't be removed is logged, and the error of a failed install says so after
/// its reason.
pub(crate) fn remove_staging(result: Result<PathBuf, String>, staging: tempfile::TempDir) -> Result<PathBuf, String> {
    let path = staging.path().to_path_buf();
    let Err(e) = staging.close() else { return result };
    log::warn!("couldn't remove the staging folder {} ({e})", path.display());
    result.map_err(|reason| format!("{reason}; couldn't remove the partial copy at {} ({e})", path.display()))
}

/// Checks a copied bundle against what its release promises, before it replaces anything.
/// `codesign` runs through `run`.
pub(crate) fn check(
    bundle: &Path,
    found: &Installed,
    expect: &Expect<'_>,
    run: impl FnOnce(&str, &[&str], Duration) -> Result<String, HelperFailure>,
    wait: CheckTurn<'_>,
) -> Result<(), String> {
    if let Some(team) = expect.team {
        match signature(bundle, team, run, wait) {
            Signature::Valid => {}
            Signature::Canceled => return Err(CANCELLED.into()),
            Signature::Wrong => return Err("the app isn't signed by its developer, so it wasn't installed".into()),
            Signature::Failed(reason) => return Err(format!("the app failed its signature check ({reason}), so it wasn't installed")),
            Signature::Unchecked(reason) => return Err(format!("the app's signature couldn't be checked ({reason}), so it wasn't installed")),
        }
    }
    if let Some(want) = expect.version
        && !found.version.as_ref().is_some_and(|v| same_release(v, want))
    {
        return Err(format!("the app inside isn't version {want}, so it wasn't installed"));
    }
    Ok(())
}

/// Whether `bundle` has a valid Developer ID signature from `team`, nested code included. The
/// requirement pins the signer, so a valid ad-hoc or other developer's signature fails it.
/// Unlike a Gatekeeper assessment it needs no network. `false` also means the check couldn't
/// finish, so an app that couldn't be checked never passes.
pub fn signed_by(bundle: &Path, team: &str) -> bool {
    let stop = Stop::default();
    matches!(signature(bundle, team, process::run_detailed, CheckTurn { stop: &stop, phase: &mut |_| {} }), Signature::Valid)
}

/// What `codesign` found out about a bundle.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Signature {
    Valid,
    /// The signature isn't the developer's.
    Wrong,
    /// `codesign` failed the verification, with its reason: no signature at all, a damaged
    /// signature, or a bundle it couldn't read.
    Failed(String),
    /// `codesign` gave no answer, with the reason why.
    Unchecked(String),
    /// The user canceled the job while it waited for its turn. `codesign` did not run.
    Canceled,
}

/// Runs `codesign` on `bundle` through `run`, one bundle at a time.
pub(crate) fn signature(
    bundle: &Path,
    team: &str,
    run: impl FnOnce(&str, &[&str], Duration) -> Result<String, HelperFailure>,
    wait: CheckTurn<'_>,
) -> Signature {
    let Some(path) = bundle.to_str() else { return Signature::Unchecked("unusual characters in the app's path".into()) };
    if team.is_empty() || !team.bytes().all(|b| b.is_ascii_alphanumeric()) {
        return Signature::Wrong;
    }
    let requirement = format!("=anchor apple generic and certificate leaf[subject.OU] = \"{team}\"");
    let verify = || match run(CODESIGN, &["--verify", "--deep", "-R", &requirement, path], CODESIGN_TIMEOUT) {
        Ok(_) => Signature::Valid,
        // codesign(1): 3 is a signature that fails the requirement, and 1 any other failed
        // verification.
        Err(HelperFailure::Exited { code: 3, .. }) => Signature::Wrong,
        Err(HelperFailure::Exited { code: 1, reason }) => Signature::Failed(reason),
        Err(failure) => Signature::Unchecked(failure.into_reason()),
    };
    SIGNATURE_CHECKS.alone(&cancelled(wait.stop), &mut turn(wait.phase, JobPhase::Installing), verify).unwrap_or(Signature::Canceled)
}

/// Whether a bundle's version is the release's. A bundle that reports only the numbers
/// (`CFBundleShortVersionString`) matches a pre-release with the same numbers.
fn same_release(found: &Version, release: &Version) -> bool {
    (found.major, found.minor, found.patch) == (release.major, release.minor, release.patch) && (found.pre.is_empty() || found.pre == release.pre)
}

/// Detaches the image mounted at `mount`, if one is. `Ok` when none is mounted there.
fn detach(mount: &Path) -> Result<(), String> {
    if !is_mounted(mount) {
        return Ok(());
    }
    let m = mount.to_str().ok_or("unusual characters in the mount point's path")?;
    process::run(HDIUTIL, &["detach", m, "-quiet"], Duration::from_secs(60))
        .or_else(|_| process::run(HDIUTIL, &["detach", m, "-force", "-quiet"], Duration::from_secs(60)))
        .map(drop)
}

/// Detaches the image file `dmg` where it is attached with no volume mounted, which a failed
/// attach can leave. An attachment with a mounted volume is left alone: another install may
/// be using it. So is every attachment while a program started with `dmg` in its arguments
/// (another process's `hdiutil attach`) is running, or while the check for one gets no answer;
/// in that case the error gives the check's reason.
fn detach_unmounted(dmg: &Path) -> Result<(), String> {
    detach_unmounted_with(dmg, process::running, process::run)
}

/// [`detach_unmounted`], with `running` checking for a program started with `dmg` in its
/// arguments and `run` running `hdiutil`.
pub(crate) fn detach_unmounted_with(
    dmg: &Path,
    running: impl FnOnce(&Path) -> Result<bool, String>,
    run: impl Fn(&str, &[&str], Duration) -> Result<String, String>,
) -> Result<(), String> {
    match running(dmg) {
        Ok(false) => {}
        Ok(true) => return Ok(()),
        Err(why) => return Err(format!("couldn't check for a running attach of the image ({why})")),
    }
    let info = run(HDIUTIL, &["info", "-plist"], Duration::from_secs(30))?;
    let image = [Some(dmg.to_path_buf()), std::fs::canonicalize(dmg).ok()];
    for device in unmounted_attachments(&info, &image.into_iter().flatten().collect::<Vec<_>>()) {
        run(HDIUTIL, &["detach", &device, "-force", "-quiet"], Duration::from_secs(60))?;
    }
    Ok(())
}

/// The devices of the attachments, in `hdiutil info -plist`'s output `info`, of an image file
/// at one of the paths in `image` that have no volume mounted.
pub(crate) fn unmounted_attachments(info: &str, image: &[PathBuf]) -> Vec<String> {
    let Ok(info) = plist::Value::from_reader_xml(info.as_bytes()) else { return Vec::new() };
    let images = info.as_dictionary().and_then(|d| d.get("images")).and_then(plist::Value::as_array);
    let mut devices = Vec::new();
    for attached in images.into_iter().flatten().filter_map(plist::Value::as_dictionary) {
        let path = attached.get("image-path").and_then(plist::Value::as_string);
        if !path.is_some_and(|p| image.iter().any(|i| i.as_path() == Path::new(p))) {
            continue;
        }
        let entities: Vec<_> =
            attached.get("system-entities").and_then(plist::Value::as_array).into_iter().flatten().filter_map(plist::Value::as_dictionary).collect();
        if entities.iter().any(|e| e.contains_key("mount-point")) {
            continue;
        }
        // The first entity is the whole disk; detaching it detaches the rest.
        if let Some(device) = entities.first().and_then(|e| e.get("dev-entry")).and_then(plist::Value::as_string) {
            devices.push(device.to_string());
        }
    }
    devices
}

/// Whether a volume is mounted at `dir`: it is on a different device from its parent folder.
fn is_mounted(dir: &Path) -> bool {
    use std::os::unix::fs::MetadataExt;
    let (Ok(here), Some(Ok(above))) = (std::fs::symlink_metadata(dir), dir.parent().map(std::fs::metadata)) else { return false };
    here.is_dir() && here.dev() != above.dev()
}

/// The start of the name of the folder an install copies the new app into, beside the
/// installed app. The installing process's id, `-` and six random letters and digits follow it.
const STAGING: &str = ".artcraft-launcher-staging-";

/// The empty file an install writes in its staging folder before copying anything into it.
const STAGING_MARKER: &str = ".artcraft-launcher-staging";

/// The mount points of this process's installs in progress.
static MOUNTS_IN_USE: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// Lists a mount point in [`MOUNTS_IN_USE`] until dropped.
pub(crate) struct InUse(PathBuf);

impl InUse {
    pub(crate) fn new(mount: &Path) -> Self {
        MOUNTS_IN_USE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).push(mount.to_path_buf());
        Self(mount.to_path_buf())
    }
}

impl Drop for InUse {
    fn drop(&mut self) {
        let mut in_use = MOUNTS_IN_USE.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(at) = in_use.iter().position(|m| *m == self.0) {
            in_use.swap_remove(at);
        }
    }
}

/// The process id in `name` when `name` is `prefix`, then digits, then `-` and a rest that
/// `rest` accepts.
fn owner(name: &str, prefix: &str, rest: impl Fn(&str) -> bool) -> Option<u32> {
    let (pid, after) = name.strip_prefix(prefix)?.split_once('-')?;
    if pid.is_empty() || !pid.bytes().all(|b| b.is_ascii_digit()) || after.is_empty() || !rest(after) {
        return None;
    }
    pid.parse().ok()
}

/// The ids of the running processes, from one `ps` run. `None` when `ps` fails.
fn running_processes() -> Option<std::collections::HashSet<u32>> {
    let out = process::run("/bin/ps", &["-A", "-o", "pid="], Duration::from_secs(10)).ok()?;
    out.lines().map(str::trim).filter(|l| !l.is_empty()).map(|l| l.parse().ok()).collect()
}

/// Says whether the process with a given id has ended, from one `ps` run made at the first
/// call. No process counts as ended when `ps` fails.
fn process_ended() -> impl Fn(u32) -> bool {
    let running = std::cell::OnceCell::new();
    move |pid| running.get_or_init(running_processes).as_ref().is_some_and(|r| !r.contains(&pid))
}

/// Detaches the images that installs of `app` left attached in `cache` when they ended
/// partway, and removes their mount points. A mount point belongs to an install that ended
/// when `ended` says its process ended, or when it is this process's and no install here uses
/// it. `detach` detaches an image. Failures are logged and the install goes on.
pub(crate) fn clear_mounts(app: &App, cache: &Path, ended: impl Fn(u32) -> bool, detach: impl Fn(&Path) -> Result<(), String>) {
    let Ok(entries) = std::fs::read_dir(cache) else { return };
    let prefix = format!("mount-{}-", app.id);
    // Read before `ended` runs `ps`, which then lists every process that made one of them.
    let names: Vec<_> = entries.flatten().map(|e| e.file_name()).collect();
    // Listed after the folder is read: an install registers its mount point before creating it.
    let in_use = MOUNTS_IN_USE.lock().unwrap_or_else(std::sync::PoisonError::into_inner).clone();
    for name in names {
        let Some(pid) = name.to_str().and_then(|n| owner(n, &prefix, |rest| rest.bytes().all(|b| b.is_ascii_digit()))) else { continue };
        let mount = cache.join(&name);
        let gone = if pid == std::process::id() { !in_use.contains(&mount) } else { ended(pid) };
        if !gone {
            continue;
        }
        match detach(&mount).and_then(|()| std::fs::remove_dir(&mount).map_err(|e| e.to_string())) {
            Ok(()) => log::info!("removed {}, left by an install that ended partway", mount.display()),
            Err(e) => log::warn!("couldn't clear {}, left by an install that ended partway ({e})", mount.display()),
        }
    }
}

/// The start of the name of the staging folders of launcher versions that didn't record the
/// installing process. Six random letters and digits follow it.
const OLD_STAGING: &str = ".artcraft-launcher-";

/// How long a staging folder of [`OLD_STAGING`]'s kind stays unchanged before it counts as
/// left by an install that ended. An install's copy times out after 10 minutes.
const OLD_STAGING_AGE: Duration = Duration::from_secs(24 * 60 * 60);

/// Removes the staging folders in `dir` of installs that ended partway, with the partial copies
/// in them:
///
/// - a folder named as [`STAGING`] describes, holding [`STAGING_MARKER`], whose process
///   `ended` says has ended, whichever app that install was for. Without the marker, the
///   folder is removed only when it is empty.
/// - a folder named as [`OLD_STAGING`] describes that holds only `<app>.app` and has not
///   changed for [`OLD_STAGING_AGE`] before `now`.
///
/// Folders of this process's installs and of installs still running, [`OLD_STAGING`] folders
/// of other apps and backup folders (one may hold the only copy of an app) are left alone.
/// Failures are logged and the install goes on.
pub(crate) fn clear_staging(app: &App, dir: &Path, ended: impl Fn(u32) -> bool, now: std::time::SystemTime) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    // Read before `ended` runs `ps`, which then lists every process that made one of them.
    let folders: Vec<_> = entries.flatten().filter(|e| e.file_type().is_ok_and(|t| t.is_dir())).collect();
    let random = |rest: &str| rest.len() == 6 && rest.bytes().all(|b| b.is_ascii_alphanumeric());
    for folder in folders {
        let (name, path) = (folder.file_name(), folder.path());
        let Some(name) = name.to_str() else { continue };
        let removed = match owner(name, STAGING, random) {
            Some(pid) if pid == std::process::id() || !ended(pid) => continue,
            Some(_) if std::fs::symlink_metadata(path.join(STAGING_MARKER)).is_ok_and(|m| m.is_file()) => std::fs::remove_dir_all(&path),
            // Stopped before it wrote the marker. A folder with something in it may be the user's.
            Some(_) => match std::fs::remove_dir(&path) {
                Ok(()) => Ok(()),
                Err(_) => continue,
            },
            None if name.strip_prefix(OLD_STAGING).is_some_and(random) && left_by_old_install(app, &path, now) => std::fs::remove_dir_all(&path),
            None => continue,
        };
        match removed {
            Ok(()) => log::info!("removed {}, left by an install that ended partway", path.display()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("couldn't remove {}, left by an install that ended partway ({e})", path.display()),
        }
    }
}

/// Whether `folder` holds only `<app>.app`, a folder, and has not changed for
/// [`OLD_STAGING_AGE`] before `now`.
fn left_by_old_install(app: &App, folder: &Path, now: std::time::SystemTime) -> bool {
    let Ok(entries) = std::fs::read_dir(folder) else { return false };
    let bundle = format!("{}.app", app.name);
    let names: Vec<_> = entries.take(2).map(|e| e.map(|e| e.file_name())).collect();
    let [Ok(only)] = names.as_slice() else { return false };
    only.to_str() == Some(bundle.as_str())
        && std::fs::symlink_metadata(folder.join(&bundle)).is_ok_and(|m| m.is_dir())
        && std::fs::symlink_metadata(folder).and_then(|m| m.modified()).is_ok_and(|at| now.duration_since(at).is_ok_and(|age| age >= OLD_STAGING_AGE))
}

/// A new staging folder for this process in `dir`, holding [`STAGING_MARKER`]. See [`STAGING`].
fn staging_dir(dir: &Path) -> Result<tempfile::TempDir, String> {
    let failed = |e: std::io::Error| format!("couldn't create a staging folder in {} ({e})", dir.display());
    let staging = tempfile::Builder::new().prefix(&format!("{STAGING}{}-", std::process::id())).rand_bytes(6).tempdir_in(dir).map_err(failed)?;
    std::fs::write(staging.path().join(STAGING_MARKER), b"").map_err(failed)?;
    Ok(staging)
}

/// Moves an app to the user's Trash (so it can be put back).
pub fn move_to_trash(path: &Path) -> Result<(), String> {
    let trash = crate::folders::home_dir().ok_or("no home folder")?.join(".Trash");
    let name = path.file_name().ok_or("not an app")?;
    let mut dest = trash.join(name);
    if dest.exists() {
        dest = trash.join(format!("{} {}", name.to_string_lossy(), crate::now()));
    }
    std::fs::rename(path, &dest).map_err(|e| format!("couldn't move {} to Trash ({e}). Remove it in Finder instead.", path.display()))
}
