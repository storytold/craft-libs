//! The Windows update helper, its arguments, the pre-update checks, and the records of what an update came to.

use super::detect::marker;
#[cfg(windows)]
use super::detect::{clear_pretend_version, exe_after_msi, msi_step, restarts};
use super::replace::is_plain_file;
#[cfg(windows)]
use super::replace::{mark_installed, unmark_unless_scheduled};
use super::*;

/// The first argument of the Windows update helper: a copy of this program that finishes an
/// update once the launcher has exited (see [`finish`]).
pub const FINISH_FLAG: &str = "--finish-update";

/// What the update helper does once the old launcher has exited.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) enum Finish {
    /// Run the new MSI, which reinstalls into the same folder.
    Msi(PathBuf),
    /// Unpack the new portable zip into `tmp` and copy it over the launcher's folder.
    Portable { zip: PathBuf, tmp: PathBuf },
}

/// The helper's arguments: [`FINISH_FLAG`], the process id to wait for, the exe to start
/// afterwards, then the step.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn helper_args(pid: u32, exe: &Path, finish: &Finish) -> Vec<OsString> {
    let mut args: Vec<OsString> = vec![FINISH_FLAG.into(), pid.to_string().into(), exe.into()];
    match finish {
        Finish::Msi(msi) => args.extend(["msi".into(), msi.into()]),
        Finish::Portable { zip, tmp } => args.extend(["portable".into(), zip.into(), tmp.into()]),
    }
    args
}

/// Reads [`helper_args`] back.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn parse_helper_args(args: &[OsString]) -> Option<(u32, PathBuf, Finish)> {
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
pub(super) fn msi_finish(file: &Path) -> Result<Finish, String> {
    install::msi_path(file).map(Finish::Msi)
}

/// The cache folder the update helper is copied into.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) const HELPER_DIR: &str = "launcher-update-helper";
/// The cache folder a portable update is unpacked into.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) const PORTABLE_STAGING: &str = "launcher-update";

/// Copies this program into the cache as the update helper, which waits for `pid` to exit, does
/// `finish` and starts `exe`. Run from the cache, the helper doesn't hold the installed exe that
/// the step replaces. The returned [`Restart`] records `release` as installed and starts the
/// helper.
#[cfg(windows)]
pub(super) fn windows_helper(app: &App, paths: &Paths, release: &Release, pid: u32, exe: &Path, finish: &Finish) -> Result<Restart, String> {
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
pub(super) fn finish_step(
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
pub(super) fn with_restart(done: Result<(), String>, started: Result<(), String>) -> Result<(), String> {
    match (done, started) {
        (done, Ok(())) => done,
        (Ok(()), Err(restart)) => Err(restart),
        (Err(update), Err(restart)) => Err(format!("{update}; {restart}")),
    }
}

/// Whether `exe` has `portable.txt` beside it, which makes [`Paths::for_exe`] keep its folders
/// there.
#[cfg(windows)]
pub(super) fn has_portable_marker(exe: &Path) -> bool {
    exe.parent().is_some_and(|dir| dir.join("portable.txt").is_file())
}

/// Moves the record of the scheduled update from `from` to `to`, the folders the updated app
/// reads, and removes outcome records left in `to` by an earlier update.
#[cfg(windows)]
pub(super) fn move_marker(from: &Paths, to: &Paths) {
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

/// Where a finished update records the version it installed, for the new version's first start.
pub(super) fn done_file(paths: &Paths) -> PathBuf {
    paths.data.join("self-update-done.txt")
}

/// Records that the update to `version` has been installed.
pub(super) fn record_done(paths: &Paths, version: &str) {
    if let Err(e) = crate::paths::write_atomic(&done_file(paths), version.as_bytes()) {
        log::warn!("couldn't record the finished self-update: {e}");
    }
}

/// The version the scheduled update installs, from the record `mark_installed` wrote: its third
/// line, or the tag on its first line in a record without one.
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn scheduled_version(paths: &Paths) -> Option<String> {
    let text = read_record(&marker(paths))?;
    let mut lines = text.lines();
    let tag = lines.next();
    let version = lines.nth(1).filter(|version| !version.trim().is_empty()).or(tag);
    version.map(str::trim).filter(|version| !version.is_empty()).map(str::to_owned)
}

/// The most of an outcome record that is read: a reason longer than this is cut short.
pub(super) const RECORD_LIMIT: usize = 4096;

/// The text of the record at `path`, at most [`RECORD_LIMIT`] bytes of it. `None` when it is
/// missing, unreadable or not a plain file (a link is not followed).
pub(super) fn read_record(path: &Path) -> Option<String> {
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
pub(super) fn take_outcome_of(paths: &Paths, running: &str, program: Option<&Path>) -> Option<Outcome> {
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
pub(super) fn failure_predates(paths: &Paths, program: &Path) -> bool {
    let (Ok(record), Ok(program)) = (std::fs::metadata(failure_file(paths)), std::fs::metadata(program)) else { return false };
    let Ok(recorded) = record.modified() else { return false };
    [program.modified(), program.created()].into_iter().flatten().any(|at| at > recorded)
}

/// Where a failed update's error waits for the next start.
pub(super) fn failure_file(paths: &Paths) -> PathBuf {
    paths.logs().join("self-update-error.txt")
}

/// Records a failed update: the next check offers the update again, and the next start shows
/// `error`. A record of the update as finished is removed.
#[cfg_attr(not(any(windows, feature = "ui")), allow(dead_code))]
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
pub(super) fn is_own_executable(app: &App, exe: &Path) -> bool {
    exe.file_stem().and_then(|stem| stem.to_str()).is_some_and(|stem| app.stems().any(|id| id.eq_ignore_ascii_case(stem)))
}

/// Refuses an executable that isn't this app's own, as the helper would.
#[cfg(windows)]
pub(super) fn own_executable(app: &App, exe: &Path) -> Result<(), String> {
    if is_own_executable(app, exe) { Ok(()) } else { Err(format!("the update helper only finishes {}'s own update", app.name)) }
}

/// Checks, before anything is downloaded, what the helper checks before it installs and what
/// does not depend on the download: this program and the installed exe are signed by one
/// publisher, and the helper finds the cache in `paths` that holds its copy, the download and the
/// staging folder.
#[cfg(windows)]
pub(super) fn windows_precheck(app: &App, install: &Install, paths: &Paths) -> Result<(), String> {
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
pub(super) fn windows_check_download(app: &App, install: &Install, download: &Path) -> Result<(), String> {
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
pub(super) fn same_cache_as_helper(app: &App, paths: &Paths, exe: &Path) -> Result<(), String> {
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
pub(super) fn signed_like(app: &App, reference: &Path, file: &Path) -> Result<(), String> {
    let publisher = |path: &Path| verifysign::verify_file(path).map(|signature| signature.subject_name());
    let ours = publisher(reference).map_err(|e| format!("this copy of {} isn't signed ({e}), so it doesn't install updates", app.name))?;
    let theirs = publisher(file).map_err(|e| format!("{} isn't signed ({e})", file.display()))?;
    if ours == theirs { Ok(()) } else { Err(format!("{} isn't signed by {}'s publisher", file.display(), app.name)) }
}

/// Whether `file` is a Windows program or library (it starts with the `MZ` header), whatever its
/// name.
#[cfg(windows)]
pub(super) fn is_program(file: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 2];
    std::fs::File::open(file).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && magic == *b"MZ"
}

/// Allows only the app's own update: `exe` is this app's MSI or portable installation,
/// of the kind `step` updates, and the helper (`me`), the download and the staging folder are in
/// that installation's cache. Anything else is refused before the helper runs or starts a file.
#[cfg(windows)]
pub(super) fn check_helper(app: &App, me: &Path, paths: &Paths, exe: &Path, step: &Finish) -> Result<(), String> {
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
pub(super) fn run_step(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: std::time::Duration) -> Result<(), String> {
    step_after_wait(app, reference, pid, exe, step, &|pid| process::wait_for_exit(pid, wait)).result
}

/// What [`step_after_wait`] came to.
#[cfg(windows)]
pub(super) struct Stepped {
    /// The old app had exited when the wait returned.
    pub(super) exited: bool,
    pub(super) result: Result<(), String>,
}

/// Calls `wait`, which says whether `pid` exited in time, does `step` once it has, and removes
/// the download either way. The MSI, and every program in the portable zip, must be signed like
/// `reference`.
#[cfg(windows)]
pub(super) fn step_after_wait(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: &dyn Fn(u32) -> bool) -> Stepped {
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
