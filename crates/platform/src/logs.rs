//! The app's log: `log::` records go to standard error and to a log file a bug report can attach,
//! and [`crash`] writes panics to a crash log.
//!
//! A launch from a desktop menu has no terminal, so without the file every `log::warn!` (a GPU
//! fallback, a lost device, a panicked worker) is gone. The file is `<dir>/<app>.log` in a
//! directory the app chooses (PhotoCraft: `<settings dir>/logs`; FilmCraft: `<data dir>/Logs`,
//! which Help › Reveal Log Files opens). Each start moves the previous log to `<app>.1.log` and
//! that one to `.2`, so the log of a run that crashed survives the next launch. A start while
//! another instance still writes `<app>.log` leaves it alone and writes `<app>-<pid>.log`.
//!
//! Levels: `info` for the app's own crates, `warn` for the rest (wgpu and naga are chatty).
//! `RUST_LOG` replaces that with env_logger-style directives (`debug`, `warn,photocraft_gpu=trace`);
//! a directive ending in `*` matches every target starting with it (`photocraft*=debug`).
//!
//! Records logged before the directory is known are kept (up to [`MAX_PENDING`]) and written
//! once the file is attached. Writing never panics: a file that can't be created or written
//! leaves standard error as the only sink. A record takes the sink lock for one `write_all` of an
//! already formatted line, so a realtime thread that logs never waits on more than one line.
//!
//! ```ignore
//! const NAMES: LogNames = LogNames::new("photocraft");
//! let logger = craft_platform::logs::install(NAMES);
//! logs::crash::install("PhotoCraft", VERSION, None);
//! // after argument parsing, once the settings directory is known:
//! if let (Some(logger), Some(dir)) = (logger, config_dir()) {
//!     match logger.attach_dir(&dir.join("logs")) {
//!         Ok(path) => log::info!("PhotoCraft {version}, log file {}", path.display()),
//!         Err(e) => log::warn!("no log file: {e}"),
//!     }
//!     logs::crash::install("PhotoCraft", VERSION, Some(dir.join("logs")));
//! }
//! ```

pub mod crash;

#[cfg(not(target_arch = "wasm32"))]
use std::fs::{File, OpenOptions, TryLockError};
use std::io::Write;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use web_time::SystemTime;

use log::LevelFilter;

/// How many previous logs are kept (`<app>.1.log` … `<app>.<KEEP>.log`), and how many
/// side-by-side instance logs of exited instances.
pub const KEEP: usize = 2;
/// The log file stops growing past this size, with one line saying so (a runaway warning can't
/// fill the disk).
pub const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;
/// Records kept in memory until the log file is attached.
pub const MAX_PENDING: usize = 512;
/// Held while one start rotates and prunes the logs, so two starts at once can't interleave.
pub const ROTATE_LOCK: &str = "rotate.lock";

/// The app's log file names, from its lower-case name: `photocraft` gives `photocraft.log`,
/// `photocraft.1.log`, `photocraft-<pid>.log`, the owner lock `photocraft.lock`, and the default
/// filter `warn,photocraft*=info`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct LogNames {
    stem: &'static str,
}

#[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "the file side is native only"))]
impl LogNames {
    /// Names for the app `stem`, the lower-case app name its crates start with.
    pub const fn new(stem: &'static str) -> Self {
        Self { stem }
    }

    /// The current log file's name: `<stem>.log`.
    pub fn file(&self) -> String {
        // pc: apps/photocraft/src/logging.rs:29
        format!("{}.log", self.stem)
    }

    /// The filter when `RUST_LOG` is unset or blank: `warn,<stem>*=info`.
    pub fn default_filter(&self) -> String {
        // pc: apps/photocraft/src/logging.rs:37
        format!("warn,{}*=info", self.stem)
    }

    /// `<stem>.<n>.log`, and [`Self::file`] for `n == 0`.
    fn numbered(&self, n: usize) -> String {
        // pc: apps/photocraft/src/logging.rs:126
        if n == 0 { self.file() } else { format!("{}.{n}.log", self.stem) }
    }

    /// The lock file the instance writing [`Self::file`] holds until it exits: `<stem>.lock`. A
    /// separate file, since Windows locks are mandatory and a locked log couldn't be opened for a
    /// bug report while the app runs.
    fn owner_lock(&self) -> String {
        // pc: apps/photocraft/src/logging.rs:133
        format!("{}.lock", self.stem)
    }

    /// The log of an instance started while another one wrote [`Self::file`]: `<stem>-<pid>.log`.
    fn instance_log(&self, pid: u32) -> String {
        // pc: apps/photocraft/src/logging.rs:138
        format!("{}-{pid}.log", self.stem)
    }

    /// The lock marking [`Self::instance_log`] as in use: `<stem>-<pid>.lock`.
    fn instance_lock(&self, pid: u32) -> String {
        // pc: apps/photocraft/src/logging.rs:143
        format!("{}-{pid}.lock", self.stem)
    }

    /// The process id in an instance log's file name; `None` for any other name.
    fn instance_pid(&self, name: &str) -> Option<u32> {
        // pc: apps/photocraft/src/logging.rs:148
        let digits = name.strip_prefix(self.stem)?.strip_prefix('-')?.strip_suffix(".log")?;
        if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
            return None;
        }
        digits.parse().ok()
    }
}

/// Per-target level filter parsed from env_logger-style directives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    default: LevelFilter,
    directives: Vec<(String, LevelFilter)>,
}

impl Filter {
    /// Parse `spec` (`warn,photocraft*=info`). Unknown levels and empty parts are skipped; a bare
    /// target name turns everything from it on (as env_logger does). Without a bare level,
    /// targets no directive names log errors only. Never panics.
    pub fn parse(spec: &str) -> Self {
        // pc: apps/photocraft/src/logging.rs:49
        let mut f = Self { default: LevelFilter::Error, directives: Vec::new() };
        for part in spec.split(',').map(str::trim).filter(|p| !p.is_empty()) {
            match part.split_once('=') {
                Some((name, level)) => {
                    let name = name.trim();
                    if let (false, Some(level)) = (name.is_empty(), parse_level(level)) {
                        f.directives.push((name.to_owned(), level));
                    }
                }
                None => match parse_level(part) {
                    Some(level) => f.default = level,
                    None => f.directives.push((part.to_owned(), LevelFilter::Trace)),
                },
            }
        }
        f
    }

    /// The level for `target`: the most specific matching directive (`a` matches `a` and `a::b`,
    /// not `ab`; `a*` matches every target starting with `a`), else the default.
    pub fn level_for(&self, target: &str) -> LevelFilter {
        // pc: apps/photocraft/src/logging.rs:70
        self.directives.iter().filter(|(name, _)| matches(name, target)).max_by_key(|(name, _)| name.len()).map_or(self.default, |(_, level)| *level)
    }

    /// The most verbose level any target can reach, for `log::set_max_level`.
    pub fn max(&self) -> LevelFilter {
        // pc: apps/photocraft/src/logging.rs:75
        self.directives.iter().map(|(_, level)| *level).fold(self.default, Ord::max)
    }
}

fn parse_level(s: &str) -> Option<LevelFilter> {
    s.trim().parse().ok()
}

/// `name` matches `target` itself and its submodules; `name*` matches every target starting with `name`.
fn matches(name: &str, target: &str) -> bool {
    match name.strip_suffix('*') {
        Some(prefix) => target.starts_with(prefix),
        None => target.strip_prefix(name).is_some_and(|rest| rest.is_empty() || rest.starts_with("::")),
    }
}

/// (year, month, day) of the day count since 1970-01-01 in the proleptic Gregorian calendar.
fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

#[cfg(not(target_arch = "wasm32"))]
fn open_lock(path: &Path) -> std::io::Result<File> {
    OpenOptions::new().read(true).write(true).create(true).truncate(false).open(path)
}

/// Take `file`'s lock without waiting: `Ok(true)` holds it, `Ok(false)` when another holder has it.
#[cfg(not(target_arch = "wasm32"))]
fn try_hold(file: &File) -> std::io::Result<bool> {
    match file.try_lock() {
        Ok(()) => Ok(true),
        Err(TryLockError::WouldBlock) => Ok(false),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn rotate_locked(dir: &Path, names: LogNames) -> std::io::Result<LogFile> {
    prune_instance_logs(dir, names);
    let owner = open_lock(&dir.join(names.owner_lock()))?;
    let state = try_hold(&owner);
    if let Ok(false) = state {
        let pid = std::process::id();
        let lock = open_lock(&dir.join(names.instance_lock(pid)))?;
        let held = try_hold(&lock).unwrap_or(false);
        let path = dir.join(names.instance_log(pid));
        let file = File::create(&path)?;
        return Ok(LogFile { path, file, owner: held.then_some(lock) });
    }
    // This start holds the owner lock (or the file system has no locks).
    let held = state.unwrap_or(false);
    for n in (1..=KEEP).rev() {
        match std::fs::rename(dir.join(names.numbered(n - 1)), dir.join(names.numbered(n))) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e),
            _ => {}
        }
    }
    let path = dir.join(names.numbered(0));
    let file = File::create(&path)?;
    Ok(LogFile { path, file, owner: held.then_some(owner) })
}

/// Remove the logs of exited side-by-side instances, all but the newest [`KEEP`]. A log whose
/// lock is held belongs to a running instance and stays. Failures are ignored: pruning only
/// saves disk space.
#[cfg(not(target_arch = "wasm32"))]
fn prune_instance_logs(dir: &Path, names: LogNames) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    let mut finished: Vec<(SystemTime, u32)> = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry.file_name().to_str().and_then(|n| names.instance_pid(n)) else { continue };
        // A missing lock file means nobody holds it; an unknown lock state counts as running.
        let running = match OpenOptions::new().read(true).write(true).open(dir.join(names.instance_lock(pid))) {
            Ok(lock) => !try_hold(&lock).unwrap_or(false),
            Err(_) => false,
        };
        if !running {
            let modified = entry.metadata().and_then(|m| m.modified()).unwrap_or(web_time::UNIX_EPOCH);
            finished.push((modified, pid));
        }
    }
    finished.sort_unstable_by(|a, b| b.cmp(a));
    for &(_, pid) in finished.iter().skip(KEEP) {
        let _ = std::fs::remove_file(dir.join(names.instance_log(pid)));
        let _ = std::fs::remove_file(dir.join(names.instance_lock(pid)));
    }
}

impl Sink {
    #[cfg(not(target_arch = "wasm32"))]
    fn append(&mut self, line: &str) {
        if self.capped {
            return;
        }
        let Some(file) = self.file.as_mut() else { return };
        let len = u64::try_from(line.len()).unwrap_or(u64::MAX);
        let text = if self.written.saturating_add(len) > self.max_bytes {
            self.capped = true;
            format!("log file reached {} bytes; later records go to standard error only\n", self.max_bytes)
        } else {
            self.written = self.written.saturating_add(len);
            line.to_owned()
        };
        // A full disk or a vanished file must not take the app down: drop the file sink and
        // stop keeping lines, since no file will come.
        if file.write_all(text.as_bytes()).is_err() {
            self.file = None;
            self.capped = true;
        }
    }
}

/// `2026-10-08T07:59:17.728Z`: UTC with milliseconds. A clock before 1970 reads as the epoch; a
/// year past 9999 is written in full, without a `+`. No chrono: the date comes from the day
/// count by the civil-from-days algorithm.
pub fn timestamp(t: SystemTime) -> String {
    // pc: apps/photocraft/src/logging.rs:96
    let since = t.duration_since(web_time::UNIX_EPOCH).unwrap_or_default();
    let secs = i64::try_from(since.as_secs()).unwrap_or(i64::MAX);
    let (days, sod) = (secs.div_euclid(86_400), secs.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}.{:03}Z", sod / 3600, sod % 3600 / 60, sod % 60, since.subsec_millis())
}

/// One formatted line: `<timestamp> <LEVEL> [<thread>] <target>: <message>\n`, the message's own
/// trailing newline dropped.
pub fn format_line(ts: &str, level: log::Level, thread: &str, target: &str, message: &str) -> String {
    // pc: apps/photocraft/src/logging.rs:245
    let message = message.strip_suffix('\n').unwrap_or(message);
    format!("{ts} {level:<5} [{thread}] {target}: {message}\n")
}

/// A log file opened by [`rotate`].
#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub struct LogFile {
    /// The log file's path.
    pub path: PathBuf,
    /// The open file, for writing.
    pub file: File,
    /// The held lock that marks `path` as in use, until the logger is dropped; `None` where the
    /// file system has no locks.
    owner: Option<File>,
}

/// Open a fresh, empty log file in `dir` (created if missing).
///
/// Normally the previous logs shift up one (`.log` → `.1.log` → … → `.<KEEP>.log`, the oldest
/// dropped). While another instance holds the owner lock, the chain is left alone and this start
/// writes `<stem>-<pid>.log`: rotating would move that instance's live log away. Instance logs of
/// exited instances are pruned to the newest [`KEEP`]. On a file system without locks every start
/// rotates. [`ROTATE_LOCK`] serialises two starts at once.
#[cfg(not(target_arch = "wasm32"))]
pub fn rotate(dir: &Path, names: LogNames) -> std::io::Result<LogFile> {
    // pc: apps/photocraft/src/logging.rs:177 (rotate_locked :186, prune_instance_logs :214)
    std::fs::create_dir_all(dir)?;
    // Waits only for another start's few file operations; without locking support, go on.
    let guard = open_lock(&dir.join(ROTATE_LOCK)).ok().filter(|g| g.lock().is_ok());
    let log = rotate_locked(dir, names);
    drop(guard);
    log
}

/// Where formatted lines go besides standard error: kept in memory until a file is attached,
/// then appended to it until it reaches its size cap.
#[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "the file side is native only"))]
#[derive(Debug)]
pub struct Sink {
    #[cfg(not(target_arch = "wasm32"))]
    file: Option<File>,
    pending: Vec<String>,
    dropped: usize,
    written: u64,
    max_bytes: u64,
    capped: bool,
}

impl Sink {
    /// An empty sink whose file stops at `max_bytes`.
    pub fn new(max_bytes: u64) -> Self {
        // pc: apps/photocraft/src/logging.rs:262
        Self {
            #[cfg(not(target_arch = "wasm32"))]
            file: None,
            pending: Vec::new(),
            dropped: 0,
            written: 0,
            max_bytes,
            capped: false,
        }
    }

    /// Append `line` to the file, or keep it (up to [`MAX_PENDING`], counting the rest) until
    /// [`Self::attach`].
    pub fn write(&mut self, line: &str) {
        // pc: apps/photocraft/src/logging.rs:267
        if self.capped {
            return;
        }
        #[cfg(not(target_arch = "wasm32"))]
        if self.file.is_some() {
            self.append(line);
            return;
        }
        if self.pending.len() < MAX_PENDING {
            self.pending.push(line.to_owned());
        } else {
            self.dropped = self.dropped.saturating_add(1);
        }
    }

    /// Start writing to `file`: first the kept lines, then, if any were dropped, the line
    /// `{n} earlier log lines were dropped before the log file was opened\n`. A line that would
    /// take the file past `max_bytes` is replaced by
    /// `log file reached {max_bytes} bytes; later records go to standard error only\n` and nothing
    /// follows it. A write error drops the file and stops keeping lines (no file will come).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn attach(&mut self, file: File) {
        // pc: apps/photocraft/src/logging.rs:280
        self.file = Some(file);
        for line in std::mem::take(&mut self.pending) {
            self.append(&line);
        }
        if self.dropped > 0 {
            let note = format!("{} earlier log lines were dropped before the log file was opened\n", self.dropped);
            self.dropped = 0;
            self.append(&note);
        }
    }

    /// No file will come: drop the kept lines and keep none from now on.
    pub fn close(&mut self) {
        // filmcraft: apps/filmcraft/src/logging.rs:202
        #[cfg(not(target_arch = "wasm32"))]
        {
            self.file = None;
        }
        self.pending = Vec::new();
        self.dropped = 0;
        self.capped = true;
    }
}

/// The installed logger: a [`Filter`], standard error and a [`Sink`].
#[cfg_attr(target_arch = "wasm32", allow(dead_code, reason = "the file side is native only"))]
#[derive(Debug)]
pub struct AppLogger {
    names: LogNames,
    filter: Filter,
    stderr: bool,
    sink: Mutex<Sink>,
    /// The lock marking the attached file as in use (see [`rotate`]); held until exit.
    #[cfg(not(target_arch = "wasm32"))]
    owner: Mutex<Option<File>>,
}

impl AppLogger {
    /// A logger for `names` with `filter`, also writing to standard error when `stderr`.
    pub fn new(names: LogNames, filter: Filter, stderr: bool) -> Self {
        // pc: apps/photocraft/src/logging.rs:323
        Self {
            names,
            filter,
            stderr,
            sink: Mutex::new(Sink::new(MAX_FILE_BYTES)),
            #[cfg(not(target_arch = "wasm32"))]
            owner: Mutex::new(None),
        }
    }

    /// Rotate the logs in `dir` and send records to the fresh file; returns its path. An unusable
    /// directory leaves standard error as the only sink ([`Self::stderr_only`]) and returns
    /// `<dir>: <error>`.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn attach_dir(&self, dir: &Path) -> Result<PathBuf, String> {
        // pc: apps/photocraft/src/logging.rs:328 with filmcraft: apps/filmcraft/src/logging.rs:244 (stderr_only on error)
        match rotate(dir, self.names) {
            Ok(LogFile { path, file, owner }) => {
                self.sink.lock().unwrap_or_else(PoisonError::into_inner).attach(file);
                *self.owner.lock().unwrap_or_else(PoisonError::into_inner) = owner;
                Ok(path)
            }
            Err(e) => {
                self.stderr_only();
                Err(format!("{}: {e}", dir.display()))
            }
        }
    }

    /// No log file this run (no directory, or it can't be used): stop keeping records for one.
    pub fn stderr_only(&self) {
        // filmcraft: apps/filmcraft/src/logging.rs:258
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).close();
    }
}

impl log::Log for AppLogger {
    fn enabled(&self, metadata: &log::Metadata<'_>) -> bool {
        // pc: apps/photocraft/src/logging.rs:337
        metadata.level() <= self.filter.level_for(metadata.target())
    }

    /// Format outside the lock (timestamp, level, thread name or `?`, target), write standard
    /// error outside the lock, then the sink; a poisoned lock is taken over, never a panic.
    fn log(&self, record: &log::Record<'_>) {
        // pc: apps/photocraft/src/logging.rs:341
        if !self.enabled(record.metadata()) {
            return;
        }
        let thread = std::thread::current();
        let line =
            format_line(&timestamp(SystemTime::now()), record.level(), thread.name().unwrap_or("?"), record.target(), &record.args().to_string());
        if self.stderr {
            let _ = std::io::stderr().write_all(line.as_bytes());
        }
        self.sink.lock().unwrap_or_else(PoisonError::into_inner).write(&line);
    }

    fn flush(&self) {}
}

/// Install the logger for `names` (filter from `RUST_LOG`, else [`LogNames::default_filter`])
/// and set `log`'s max level; `None` when another logger was installed first. Call
/// [`AppLogger::attach_dir`] once the directory is known, after argument parsing, so `--version`
/// and usage errors leave no file behind.
pub fn install(names: LogNames) -> Option<&'static AppLogger> {
    // pc: apps/photocraft/src/logging.rs:358
    static LOGGER: OnceLock<AppLogger> = OnceLock::new();
    let spec = std::env::var("RUST_LOG").ok().filter(|s| !s.trim().is_empty());
    let filter = Filter::parse(spec.as_deref().unwrap_or(&names.default_filter()));
    let max = filter.max();
    let logger = LOGGER.get_or_init(|| AppLogger::new(names, filter, true));
    log::set_logger(logger).ok()?;
    log::set_max_level(max);
    Some(logger)
}

#[cfg(all(test, not(target_arch = "wasm32")))]
#[path = "logs_tests.rs"]
mod logs_tests;
