//! The Windows update helper, its arguments, the pre-update checks, and the records of what an update came to.

use super::detect::marker;
#[cfg(windows)]
use super::detect::{clear_pretend_version, detect, exe_after_msi, msi_step_reason as msi_step, restarts};
use super::replace::is_plain_file;
#[cfg(windows)]
use super::replace::{mark_installed, unmark_unless_scheduled};
use super::{App, OsString, Path, PathBuf, Paths, Version, install, version};
#[cfg(windows)]
use super::{Install, Release, Restart, process};
use crate::translate::Reason;

/// The first argument of the Windows update helper: a copy of this program that finishes an
/// update once the launcher has exited (see [`finish`]).
pub const FINISH_FLAG: &str = "--finish-update";

/// What the update helper does once the old launcher has exited.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(not(windows), allow(dead_code))]
pub(super) enum Finish {
    /// Run the new MSI, which reinstalls into the same folder.
    Msi(PathBuf),
    /// Per-user MSI with no installer progress window.
    QuietMsi(PathBuf),
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
        Finish::QuietMsi(msi) => args.extend(["msi-quiet".into(), msi.into()]),
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
        [kind, msi] if kind == "msi-quiet" => Finish::QuietMsi(msi.into()),
        [kind, zip, tmp] if kind == "portable" => Finish::Portable { zip: zip.into(), tmp: tmp.into() },
        _ => return None,
    };
    Some((pid, exe.into(), finish))
}

#[cfg_attr(not(windows), allow(dead_code))]
pub(super) fn msi_finish(file: &Path, quiet: bool) -> Result<Finish, Reason> {
    install::msi_path_reason(file).map(|file| if quiet { Finish::QuietMsi(file) } else { Finish::Msi(file) })
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
pub(super) fn windows_helper(app: &App, paths: &Paths, release: &Release, pid: u32, exe: &Path, finish: &Finish) -> Result<Restart, Reason> {
    let program = std::env::current_exe().map_err(|e| Reason::new("couldn't find this program ({error})", &[("error", &e.to_string())]))?;
    let dir = paths.cache.join(HELPER_DIR);
    // The previous update's copy, when it's still there.
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).map_err(|e| Reason::new("couldn't schedule the update ({error})", &[("error", &e.to_string())]))?;
    let helper = dir.join(program.file_name().ok_or_else(|| Reason::new("this program has no file name", &[]))?);
    std::fs::copy(&program, &helper).map_err(|e| Reason::new("couldn't schedule the update ({error})", &[("error", &e.to_string())]))?;
    let mut cmd = process::command(&helper);
    cmd.args(helper_args(pid, exe, finish));
    clear_pretend_version(&mut cmd, app);
    let (paths, release) = (paths.clone(), release.clone());
    Ok(Restart::later_reason(move || {
        mark_installed(&paths, &release);
        let started = process::spawn_detached_reason(cmd).map_err(|e| e.context("couldn't schedule the update ({reason})", &[]));
        unmark_unless_scheduled(&paths, started)
    }))
}

/// Finishes an update when this program was started as the update helper; `args` are its
/// arguments without the program name. `None` when they don't start with [`FINISH_FLAG`]. The
/// app is started again even when the step fails: the old copy is then still in place.
#[cfg(windows)]
pub fn finish(app: &App, args: &[OsString]) -> Option<Result<(), String>> {
    finish_reason(app, args).map(|result| result.map_err(|reason| reason.to_string()))
}

/// Finishes the helper with crate-authored errors retained for translation.
#[cfg(windows)]
pub fn finish_reason(app: &App, args: &[OsString]) -> Option<Result<(), Reason>> {
    if args.first().is_none_or(|a| a != FINISH_FLAG) {
        return None;
    }
    let Some((pid, exe, step)) = parse_helper_args(args) else { return Some(Err(Reason::new("the update helper's arguments are incomplete", &[]))) };
    let Ok(me) = std::env::current_exe() else { return Some(Err(Reason::new("the update helper couldn't find itself", &[]))) };
    let paths = match Paths::for_exe_reason(app, &exe) {
        Ok(paths) => paths,
        Err(e) => return Some(Err(e)),
    };
    // Nothing runs unless this is the app's own update, signed like this helper.
    if let Err(e) = check_helper(app, &me, &paths, &exe, &step).and_then(|()| signed_like(app, &me, &exe)) {
        record_failure_reason(&paths, &e);
        return Some(Err(e));
    }
    // Read before the step: the update may move or replace the old exe.
    let registered = matches!(step, Finish::Msi(_) | Finish::QuietMsi(_))
        && matches!(crate::msi_copy::windows::find(app, &exe), Some(crate::msi_copy::MsiCopy::Registered { .. }));
    let wait = |pid| process::wait_for_exit(pid, std::time::Duration::from_secs(60));
    let start = |exe: &Path| {
        signed_like(app, &me, exe).and_then(|()| {
            process::spawn_detached_reason(std::process::Command::new(exe))
                .map_err(|e| e.context("couldn't start {path} ({reason})", &[("path", &exe.display().to_string())]))
        })
    };
    Some(finish_step_reason(app, &me, &paths, pid, exe, &step, registered, &wait, &start))
}

/// The helper's work once its checks have passed: waits for `pid` with `wait`, does `step`, and
/// starts the app with `start` when [`restarts`] allows it. Every failure is recorded for the
/// app's next start.
#[cfg(all(windows, test))]
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
    finish_step_reason(app, reference, paths, pid, exe, step, registered, wait, &|exe| start(exe).map_err(Reason::from))
        .map_err(|reason| reason.to_string())
}

#[cfg(windows)]
#[allow(clippy::too_many_arguments)]
pub(super) fn finish_step_reason(
    app: &App,
    reference: &Path,
    paths: &Paths,
    pid: u32,
    exe: PathBuf,
    step: &Finish,
    registered: bool,
    wait: &dyn Fn(u32) -> bool,
    start: &dyn Fn(&Path) -> Result<(), Reason>,
) -> Result<(), Reason> {
    let portable_before = has_portable_marker(&exe);
    let stepped = step_after_wait(app, reference, pid, &exe, step, wait);
    let done = stepped.result;
    if let Err(e) = &done {
        record_failure_reason(paths, e);
    }
    // The new app, or the old one when the step failed.
    let exe = if done.is_ok() { exe_after_msi(app, registered, &exe) } else { exe };
    // A standalone copy updated from the portable zip gains `portable.txt` and keeps its folders
    // beside it from now on: the records the new app reads go there.
    let version = scheduled_version(paths);
    let moved = (done.is_ok() && has_portable_marker(&exe) != portable_before).then(|| Paths::for_exe_reason(app, &exe).ok()).flatten();
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
    let result = with_restart_reason(done, started);
    if !restarted && let Err(e) = &result {
        record_failure_reason(paths, e);
    }
    result
}

/// The helper's result once it has tried to start the app: when both the update step and the
/// start fail, the error names both, the step's first.
#[cfg(any(windows, test))]
fn with_restart_reason(done: Result<(), Reason>, started: Result<(), Reason>) -> Result<(), Reason> {
    match (done, started) {
        (done, Ok(())) => done,
        (Ok(()), Err(restart)) => Err(restart),
        (Err(update), Err(restart)) => Err(update.joined(restart)),
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
    clear_failure(to);
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

/// Finishes the helper with structured reasons (a no-op outside Windows).
#[cfg(not(windows))]
pub fn finish_reason(_app: &App, _args: &[OsString]) -> Option<Result<(), Reason>> {
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
    take_outcome_reason_of(paths, running, program).map(|outcome| match outcome {
        ReasonOutcome::Installed(version) => Outcome::Installed(version),
        ReasonOutcome::Failed(reason) => Outcome::Failed(reason.to_string()),
    })
}

/// A structured view of the outcome, consumed by the same read-once path as [`take_outcome`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReasonOutcome {
    /// The installed version is the version now running.
    Installed(Version),
    /// Why the update failed, retaining authored messages for later translation.
    Failed(Reason),
}

/// Reads the last outcome once, retaining an authored failure's translator inputs.
pub fn take_outcome_reason(paths: &Paths, running: &str) -> Option<ReasonOutcome> {
    let program = std::env::current_exe().ok();
    take_outcome_reason_of(paths, running, program.as_deref())
}

fn take_outcome_reason_of(paths: &Paths, running: &str, program: Option<&Path>) -> Option<ReasonOutcome> {
    let stale = program.is_some_and(|program| failure_predates(paths, program));
    let failure = take_failure_reason(paths).filter(|_| !stale);
    let done = read_record(&done_file(paths));
    let _ = std::fs::remove_file(done_file(paths));
    if let Some(reason) = failure {
        let reason = if reason.to_string().trim().is_empty() { Reason::new("no reason was recorded", &[]) } else { reason.trimmed() };
        return Some(ReasonOutcome::Failed(reason));
    }
    let installed = version::parse(done?.trim())?;
    (Some(&installed) == version::parse(running).as_ref()).then_some(ReasonOutcome::Installed(installed))
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

/// Keeps the existing old-failure cleanup and also removes a regular recognized companion.
pub(super) fn clear_failure(paths: &Paths) {
    let _ = std::fs::remove_file(failure_file(paths));
    remove_plain(&reason_file(paths));
}

fn remove_plain(file: &Path) {
    if is_plain_file(file) {
        let _ = std::fs::remove_file(file);
    }
}

fn reason_file(paths: &Paths) -> PathBuf {
    paths.logs().join("self-update-error.reason-v1.json")
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct Fingerprint {
    len: u64,
    modified: std::time::SystemTime,
    created: Option<std::time::SystemTime>,
}

impl Fingerprint {
    fn of(metadata: &std::fs::Metadata) -> Option<Self> {
        metadata.is_file().then_some(())?;
        Some(Self { len: metadata.len(), modified: metadata.modified().ok()?, created: metadata.created().ok() })
    }
}

#[derive(serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredReason {
    version: u8,
    english: String,
    reason: Reason,
    primary: Fingerprint,
}

struct FailureSnapshot {
    text: String,
    complete_utf8: bool,
    primary: Option<Fingerprint>,
}

/// The same bounded legacy view, plus a stable complete primary for optional typed pairing.
fn failure_snapshot(file: &Path) -> Option<FailureSnapshot> {
    use std::io::Read;
    if !is_plain_file(file) {
        return None;
    }
    let mut opened = std::fs::File::open(file).ok()?;
    let before = opened.metadata().ok().and_then(|metadata| Fingerprint::of(&metadata));
    let mut bytes = Vec::new();
    opened.by_ref().take(RECORD_LIMIT as u64 + 1).read_to_end(&mut bytes).ok()?;
    let after = opened.metadata().ok().and_then(|metadata| Fingerprint::of(&metadata));
    let named =
        std::fs::symlink_metadata(file).ok().filter(|metadata| metadata.file_type().is_file()).and_then(|metadata| Fingerprint::of(&metadata));
    let primary = if before == after && after == named { before } else { None };
    let cut = bytes.len() > RECORD_LIMIT;
    bytes.truncate(RECORD_LIMIT);
    let complete_utf8 = !cut && std::str::from_utf8(&bytes).is_ok();
    let mut text = String::from_utf8_lossy(&bytes).into_owned();
    if cut {
        text = text.trim_end_matches('\u{FFFD}').to_owned();
        text.push('…');
    }
    Some(FailureSnapshot { text, complete_utf8, primary })
}

/// Records a legacy opaque reason. The primary remains the backward-readable source of truth.
#[cfg(test)]
pub(crate) fn record_failure(paths: &Paths, error: &str) {
    let _ = write_failure(paths, error);
}

fn write_failure(paths: &Paths, english: &str) -> Option<Fingerprint> {
    remove_plain(&reason_file(paths));
    let _ = std::fs::remove_file(marker(paths));
    let _ = std::fs::remove_file(done_file(paths));
    let _ = std::fs::create_dir_all(paths.logs());
    crate::paths::write_atomic(&failure_file(paths), english.as_bytes()).ok()?;
    let metadata = std::fs::symlink_metadata(failure_file(paths)).ok()?;
    metadata.file_type().is_file().then_some(())?;
    Fingerprint::of(&metadata)
}

/// Records an authored reason without changing the legacy primary or installation result.
#[cfg_attr(not(any(windows, feature = "ui")), allow(dead_code))]
pub(crate) fn record_failure_reason(paths: &Paths, reason: &Reason) {
    let english = reason.to_string();
    let Some(primary) = write_failure(paths, &english) else { return };
    if english.len() > RECORD_LIMIT {
        return;
    }
    let file = reason_file(paths);
    match std::fs::symlink_metadata(&file) {
        Ok(metadata) if !metadata.file_type().is_file() => return,
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => return,
        _ => {}
    }
    let stored = StoredReason { version: 1, english, reason: reason.clone(), primary };
    // A fixed writer bounds serialization too; an oversized reason never makes truncated JSON.
    let mut bytes = [0u8; RECORD_LIMIT];
    let mut output = std::io::Cursor::new(bytes.as_mut_slice());
    if serde_json::to_writer(&mut output, &stored).is_err() {
        return;
    }
    let Ok(used) = usize::try_from(output.position()) else { return };
    let _ = crate::paths::write_atomic(&file, &bytes[..used]);
}

fn paired_reason(paths: &Paths, snapshot: &FailureSnapshot) -> Option<Reason> {
    use std::io::Read;
    if !snapshot.complete_utf8 {
        return None;
    }
    let primary = snapshot.primary.as_ref()?;
    let file = reason_file(paths);
    if !is_plain_file(&file) {
        return None;
    }
    let mut bytes = Vec::new();
    std::fs::File::open(file).ok()?.take(RECORD_LIMIT as u64 + 1).read_to_end(&mut bytes).ok()?;
    if bytes.len() > RECORD_LIMIT {
        return None;
    }
    let stored: StoredReason = serde_json::from_slice(&bytes).ok()?;
    // The named primary may have been replaced while the bounded sidecar was read.
    let current = std::fs::symlink_metadata(failure_file(paths)).ok()?;
    if !current.file_type().is_file() || Fingerprint::of(&current).as_ref() != Some(primary) {
        return None;
    }
    (stored.version == 1 && stored.english == snapshot.text && stored.reason.to_string() == snapshot.text && stored.primary == *primary)
        .then_some(stored.reason)
}

/// The last failure once, retaining authored provenance when a recognized companion matches.
/// Content and metadata matching reject ordinary rewrites, not every same-byte legacy rewrite.
pub fn take_failure_reason(paths: &Paths) -> Option<Reason> {
    let file = failure_file(paths);
    let snapshot = failure_snapshot(&file);
    let reason = snapshot.map(|snapshot| paired_reason(paths, &snapshot).unwrap_or_else(|| Reason::data(snapshot.text)));
    remove_plain(&file);
    remove_plain(&reason_file(paths));
    reason
}

/// The last failure once as an English compatibility view; long text retains the old cut-off.
pub fn take_failure(paths: &Paths) -> Option<String> {
    take_failure_reason(paths).map(|reason| reason.to_string())
}

/// Whether `exe` is named like this app's own executable (case-insensitive, any of its ids).
#[cfg(windows)]
pub(super) fn is_own_executable(app: &App, exe: &Path) -> bool {
    exe.file_stem().and_then(|stem| stem.to_str()).is_some_and(|stem| app.stems().any(|id| id.eq_ignore_ascii_case(stem)))
}

/// Refuses an executable that isn't this app's own, as the helper would.
#[cfg(windows)]
pub(super) fn own_executable(app: &App, exe: &Path) -> Result<(), Reason> {
    if is_own_executable(app, exe) { Ok(()) } else { Err(Reason::new("the update helper only finishes {app}'s own update", &[("app", app.name)])) }
}

/// Checks, before anything is downloaded, what the helper checks before it installs and what
/// does not depend on the download: this program and the installed exe are signed by one
/// publisher, and the helper finds the cache in `paths` that holds its copy, the download and the
/// staging folder.
#[cfg(windows)]
pub(super) fn windows_precheck(app: &App, install: &Install, paths: &Paths) -> Result<(), Reason> {
    let me = std::env::current_exe().map_err(|e| Reason::new("couldn't find this program ({error})", &[("error", &e.to_string())]))?;
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
pub(super) fn windows_check_download(app: &App, install: &Install, download: &Path) -> Result<(), Reason> {
    match install {
        Install::Msi(_) => {
            let me = std::env::current_exe().map_err(|e| Reason::new("couldn't find this program ({error})", &[("error", &e.to_string())]))?;
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
pub(super) fn same_cache_as_helper(app: &App, paths: &Paths, exe: &Path) -> Result<(), Reason> {
    std::fs::create_dir_all(&paths.cache)
        .map_err(|e| Reason::new("couldn't create {path} ({error})", &[("path", &paths.cache.display().to_string()), ("error", &e.to_string())]))?;
    let canonical = |p: &Path| p.canonicalize().ok();
    let theirs = Paths::for_exe_reason(app, exe).ok().and_then(|helper| canonical(&helper.cache));
    if theirs.is_some() && theirs == canonical(&paths.cache) {
        Ok(())
    } else {
        Err(Reason::new(
            "{app} can't update itself while it keeps its files in {path}; download the new version instead",
            &[("app", app.name), ("path", &paths.cache.display().to_string())],
        ))
    }
}

/// Accepts `file` when it carries a valid Authenticode signature from the same publisher as
/// `reference`, this program. Refuses an unsigned file, another publisher's, and every file when
/// `reference` itself isn't signed.
#[cfg(windows)]
pub(super) fn signed_like(app: &App, reference: &Path, file: &Path) -> Result<(), Reason> {
    let publisher = |path: &Path| verifysign::verify_file(path).map(|signature| signature.subject_name());
    let ours = publisher(reference).map_err(|e| {
        Reason::new("this copy of {app} isn't signed ({error}), so it doesn't install updates", &[("app", app.name), ("error", &e.to_string())])
    })?;
    let theirs = publisher(file)
        .map_err(|e| Reason::new("{path} isn't signed ({error})", &[("path", &file.display().to_string()), ("error", &e.to_string())]))?;
    if ours == theirs {
        Ok(())
    } else {
        Err(Reason::new("{path} isn't signed by {app}'s publisher", &[("path", &file.display().to_string()), ("app", app.name)]))
    }
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
pub(super) fn check_helper(app: &App, me: &Path, paths: &Paths, exe: &Path, step: &Finish) -> Result<(), Reason> {
    let refused = || Reason::new("the update helper only finishes {app}'s own update", &[("app", app.name)]);
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
        (Some(Install::Msi(installed)), Finish::Msi(msi) | Finish::QuietMsi(msi)) => installed == exe && in_downloads(msi)?,
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
    step_after_wait(app, reference, pid, exe, step, &|pid| process::wait_for_exit(pid, wait)).result.map_err(|reason| reason.to_string())
}

/// What [`step_after_wait`] came to.
#[cfg(windows)]
pub(super) struct Stepped {
    /// The old app had exited when the wait returned.
    pub(super) exited: bool,
    pub(super) result: Result<(), Reason>,
}

/// Calls `wait`, which says whether `pid` exited in time, does `step` once it has, and removes
/// the download either way. The MSI, and every program in the portable zip, must be signed like
/// `reference`.
#[cfg(windows)]
pub(super) fn step_after_wait(app: &App, reference: &Path, pid: u32, exe: &Path, step: &Finish, wait: &dyn Fn(u32) -> bool) -> Stepped {
    let waited = wait(pid);
    let result = if waited {
        match step {
            Finish::Msi(msi) | Finish::QuietMsi(msi) => {
                let quiet = matches!(step, Finish::QuietMsi(_));
                signed_like(app, reference, msi)
                    .and_then(|()| msi_step(app, exe))
                    .and_then(|step| crate::msi_migrate::windows::run_with_ui_reason(msi, &step, quiet))
            }
            Finish::Portable { zip, tmp } => match (exe.parent(), exe.file_name()) {
                (Some(dir), Some(executable)) => crate::portable::unpack_over_reason(zip, tmp, dir, executable, &|file| {
                    // The expected executable must be signed even when its contents aren't a PE file.
                    if file.file_name() == Some(executable) || is_program(file) { signed_like(app, reference, file) } else { Ok(()) }
                }),
                _ => Err(Reason::new("{app} isn't in a folder", &[("app", app.name)])),
            },
        }
    } else {
        Err(Reason::new("{app} was still running after a minute, so the update wasn't installed", &[("app", app.name)]))
    };
    match step {
        Finish::Msi(msi) | Finish::QuietMsi(msi) => {
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
pub(crate) mod reason_tests {
    use super::*;
    use craft_ui::testing::trial;

    fn translated(text: &str, args: &[(&str, &str)]) -> String {
        let key = match text {
            "copy {path}" => "copie {path}",
            "update: {reason}" => "mise: {reason}",
            "restart: {detail}" => "redémarre: {detail}",
            "no reason was recorded" => "aucune raison",
            _ => text,
        };
        crate::translate::english(key, args)
    }

    fn authored() -> Reason {
        Reason::new("copy {path}", &[("path", "{reason} Ω")]).context("update: {reason}", &[])
    }

    #[trial]
    fn translatable_update_failure_pair_survives_reload_and_done() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let first = authored();
        let second = Reason::new("restart: {detail}", &[("detail", "denied {path}")]);
        let joined = with_restart_reason(Err(first.clone()), Err(second.clone())).unwrap_err();
        assert_eq!(joined.to_string(), "update: copy {reason} Ω; restart: denied {path}");
        assert_eq!(with_restart_reason(Err(first.clone()), Ok(())).unwrap_err(), first);
        assert_eq!(with_restart_reason(Ok(()), Err(second.clone())).unwrap_err(), second);
        assert!(with_restart_reason(Ok(()), Ok(())).is_ok());
        record_failure_reason(&paths, &joined);
        assert!(reason_file(&paths).is_file());
        record_done(&paths, "v0.2.0");
        assert!(reason_file(&paths).is_file(), "done must not erase a failure's provenance");
        let Some(ReasonOutcome::Failed(read)) = take_outcome_reason_of(&paths, "0.2.0", None) else { panic!("failure wins") };
        assert_eq!(read.translated(translated), "mise: copie {reason} Ω; redémarre: denied {path}");
        assert_eq!(read.to_string(), joined.to_string());
        assert!(!failure_file(&paths).exists() && !reason_file(&paths).exists() && !done_file(&paths).exists());
        assert_eq!(take_outcome_reason_of(&paths, "0.2.0", None), None);
    }

    #[trial]
    fn translatable_update_failure_rejects_invalid_companions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let original = authored();
        for bad in 0..9 {
            record_failure_reason(&paths, &original);
            let file = reason_file(&paths);
            let mut value: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            match bad {
                0 => value["version"] = 2.into(),
                1 => value["english"] = "different".into(),
                2 => value["reason"] = serde_json::to_value(Reason::data("different")).unwrap(),
                3 => value["primary"]["len"] = 0.into(),
                4 => {
                    std::fs::write(&file, b"{").unwrap();
                }
                6 => value["reason"]["unknown"] = true.into(),
                7 => value["reason"]["contexts"][0]["unknown"] = true.into(),
                8 => value["reason"]["body"]["Message"]["unknown"] = true.into(),
                _ => {
                    std::fs::write(&file, vec![b' '; RECORD_LIMIT + 1]).unwrap();
                }
            }
            if !(4..6).contains(&bad) {
                std::fs::write(&file, serde_json::to_vec(&value).unwrap()).unwrap();
            }
            assert_eq!(take_failure_reason(&paths), Some(Reason::data(original.to_string())), "case {bad}");
            assert!(!file.exists(), "a rejected regular companion is consumed too");
        }
    }

    #[trial]
    fn translatable_update_failure_keeps_legacy_bounds_and_opaque_text() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        let raw = "  GitHub {reason} Ω  ";
        record_failure(&paths, raw);
        assert_eq!(take_failure_reason(&paths).unwrap().translated(translated), raw);
        record_failure(&paths, raw);
        let Some(ReasonOutcome::Failed(read)) = take_outcome_reason_of(&paths, "0.2.0", None) else { panic!("legacy failure") };
        assert_eq!(read.translated(translated), raw.trim());
        record_failure(&paths, "  \n");
        let Some(ReasonOutcome::Failed(read)) = take_outcome_reason_of(&paths, "0.2.0", None) else { panic!("empty failure") };
        assert_eq!(read.translated(translated), "aucune raison");
        let wide = "é".repeat(RECORD_LIMIT);
        record_failure_reason(&paths, &Reason::data(wide));
        assert!(!reason_file(&paths).exists());
        let read = take_failure(&paths).unwrap();
        assert!(read.len() <= RECORD_LIMIT + '…'.len_utf8() && read.ends_with("é…"));
        record_failure(&paths, "seed");
        std::fs::write(failure_file(&paths), b"bad\xffdata").unwrap();
        assert_eq!(take_failure(&paths).as_deref(), Some("bad�data"));
        let fits_primary = "x".repeat(2200);
        record_failure_reason(&paths, &Reason::data(fits_primary.clone()));
        assert!(!reason_file(&paths).exists(), "complete JSON must fit, not only its primary");
        assert_eq!(take_failure(&paths), Some(fits_primary));
    }

    #[trial]
    fn translatable_update_failure_rewrite_stale_and_orphan_cleanup() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        record_failure_reason(&paths, &authored());
        let file = failure_file(&paths);
        let recorded = std::fs::metadata(&file).unwrap().modified().unwrap();
        std::fs::OpenOptions::new().write(true).open(&file).unwrap().set_modified(recorded + std::time::Duration::from_secs(2)).unwrap();
        assert_eq!(take_failure_reason(&paths), Some(Reason::data(authored().to_string())), "ordinary metadata rewrite rejects pairing");
        record_failure_reason(&paths, &authored());
        record_failure(&paths, "legacy replacement");
        assert!(!reason_file(&paths).exists(), "legacy replacement invalidates the old sidecar");
        assert_eq!(take_failure(&paths).as_deref(), Some("legacy replacement"));
        record_failure_reason(&paths, &authored());
        let program = dir.path().join("program");
        std::fs::write(&program, b"program").unwrap();
        let recorded = std::fs::metadata(failure_file(&paths)).unwrap().modified().unwrap();
        std::fs::OpenOptions::new().write(true).open(&program).unwrap().set_modified(recorded + std::time::Duration::from_secs(3600)).unwrap();
        assert_eq!(take_outcome_reason_of(&paths, "0.2.0", Some(&program)), None);
        assert!(!reason_file(&paths).exists() && !failure_file(&paths).exists());
        std::fs::write(reason_file(&paths), b"orphan").unwrap();
        assert_eq!(take_failure_reason(&paths), None);
        assert!(!reason_file(&paths).exists());
    }

    #[trial]
    fn translatable_update_failure_never_replaces_nonplain_companions() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::under(dir.path());
        std::fs::create_dir_all(reason_file(&paths)).unwrap();
        record_failure_reason(&paths, &authored());
        assert!(reason_file(&paths).is_dir());
        assert_eq!(take_failure_reason(&paths), Some(Reason::data(authored().to_string())));
        assert!(reason_file(&paths).is_dir());
        std::fs::remove_dir(reason_file(&paths)).unwrap();
        record_failure_reason(&paths, &authored());
        let unrelated = paths.logs().join("keep.txt");
        std::fs::write(&unrelated, b"keep").unwrap();
        clear_failure(&paths);
        assert!(!failure_file(&paths).exists() && !reason_file(&paths).exists());
        assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&unrelated, reason_file(&paths)).unwrap();
            record_failure_reason(&paths, &authored());
            assert_eq!(take_failure_reason(&paths), Some(Reason::data(authored().to_string())));
            assert!(std::fs::symlink_metadata(reason_file(&paths)).unwrap().file_type().is_symlink());
            assert_eq!(std::fs::read(&unrelated).unwrap(), b"keep");
        }
    }
}
