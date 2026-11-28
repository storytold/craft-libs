//! Finding external oracle programs (ffmpeg, ffprobe, qpdf, Poppler's tools) for tests.
//!
//! [`find`] looks for a program in this order:
//!
//! 1. The file that an explicit-path variable names: `CRAFT_<PROGRAM>` for [`find`] (for example
//!    `CRAFT_FFMPEG`), or the variable the caller passes to [`find_with`]. An empty value counts as
//!    unset. When the value is not a file the OS can start, the lookup returns
//!    [`OracleError::InvalidExplicitPath`] and does not search further.
//! 2. Each directory on `PATH`, in order. Empty entries are skipped, and a relative entry is
//!    resolved against the current directory at the time of the lookup. On Windows the search tries
//!    `<name>.exe`, then each `.com`, `.bat` or `.cmd` suffix that `PATHEXT` lists, in its order.
//!    Elsewhere it tries the bare name, then `<name>.exe`. A name that already has an extension is
//!    tried as it is.
//! 3. The install folders [`known_dirs`] lists.
//!
//! Windows drive-relative paths use the captured current directory on the same drive, or that
//! drive's current directory otherwise. Returned executable paths are absolute.
//!
//! A file the OS can start is, on Windows, one whose extension is `.com`, `.exe`, `.bat` or `.cmd`
//! in any case, and on Unix one the current process has permission to execute. The search skips
//! any other file and goes on.
//! If a Unix permission query is unavailable, the lookup attempts to start the candidate. An
//! execution permission denial continues the search, or rejects an explicit path. Other failures
//! to start return [`OracleError::Start`].
//!
//! The program found is run once with its version argument. [`Oracle::version`] is the first
//! non-blank line of its standard output, or of its standard error when standard output is blank.
//! The exit status is not checked. The run is bounded by [`ProbeLimits`]: each output stream may
//! hold at most `max_output_bytes`, and the child is killed and reaped when it passes `timeout` or
//! the byte limit. On Unix the child runs in its own process group, and the whole group is killed,
//! so a process it started that holds the output pipes ends too. On Windows the child and its
//! descendants run in a Job Object and are stopped together. Each run goes through the crate's
//! process runner. A successful lookup is cached for the process under every input it read (the
//! program, the version argument, the explicit variable and its value, `PATH`, `PATHEXT`, the
//! current directory, the install folders and the limits). A failed lookup is not cached.
//! After one probe succeeds, concurrent and later lookups with the same cache key reuse its
//! result. A failed probe can be retried by a waiting or later lookup.
//!
//! No function here panics. [`require_oracle!`](crate::require_oracle), [`require_ffmpeg!`](crate::require_ffmpeg)
//! and [`require_ffprobe!`](crate::require_ffprobe) expand in the calling test. A missing program
//! goes to [`skip::skip`](crate::skip::skip) with the required variable the caller names
//! ([`skip::REQUIRED_VAR`](crate::skip::REQUIRED_VAR) by default): the test prints a `SKIPPED:` line
//! and returns, or fails under that module's rule for the variable's value. Every other error, such
//! as a wrong explicit path or a timeout, fails the test.

use std::collections::{HashMap, HashSet};
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Output};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::Duration;

use crate::process::{self, Observed, RunError, RunLimits};

const UNIX_DIRS: &[&str] = &["/opt/homebrew/bin", "/usr/local/bin", "/usr/bin", "/opt/local/bin", "/snap/bin"];
const WINDOWS_DIRS: &[&str] = &[r"C:\ffmpeg\bin", r"C:\Program Files\ffmpeg\bin", r"C:\ProgramData\chocolatey\bin"];

/// The `PATHEXT` suffixes that `std::process::Command` can start on Windows.
const WINDOWS_SUFFIXES: &[&str] = &[".com", ".exe", ".bat", ".cmd"];

/// Limits for one probe of an oracle program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ProbeLimits {
    /// The most bytes the child may write to standard output, and separately to standard error.
    pub max_output_bytes: usize,
    /// How long the child may run before it is killed. A timeout too long to add to the current
    /// time, such as `Duration::MAX`, returns [`OracleError::InvalidTimeout`].
    pub timeout: Duration,
}

impl ProbeLimits {
    /// The default for a version probe: 64 KiB per stream and 5 seconds.
    pub const VERSION: Self = Self { max_output_bytes: 64 * 1024, timeout: Duration::from_secs(5) };
    /// The default for a listing such as `ffmpeg -formats`: 1 MiB per stream and 5 seconds.
    pub const LISTING: Self = Self { max_output_bytes: 1024 * 1024, timeout: Duration::from_secs(5) };
}

impl Default for ProbeLimits {
    fn default() -> Self {
        Self::VERSION
    }
}

/// A program that was found and answered its version probe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Oracle {
    program: String,
    path: PathBuf,
    version: String,
}

impl Oracle {
    /// The program name that was looked for.
    pub fn program(&self) -> &str {
        &self.program
    }

    /// The absolute path of the program that was found.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The first non-blank line the program printed for its version argument, trimmed. It is empty
    /// when the program printed nothing.
    pub fn version(&self) -> &str {
        &self.version
    }

    /// A new [`Command`] for the program's path. The caller adds the arguments.
    pub fn command(&self) -> Command {
        Command::new(&self.path)
    }
}

/// Why an oracle could not be used.
#[derive(Debug)]
#[non_exhaustive]
pub enum OracleError {
    /// The program name is empty, contains a path separator, `:` or NUL, or is `.` or `..`.
    InvalidName { program: String },
    /// The probe's timeout is too long to set a deadline from the current time.
    InvalidTimeout { timeout: Duration },
    /// The variable name is empty or contains `=` or NUL, so the environment cannot be read for it.
    InvalidVariable { variable: String },
    /// No file with the program's name was found. The `require_*` macros skip on this error.
    Missing { program: String },
    /// The explicit-path variable is set, but its value is not a file the OS can start.
    InvalidExplicitPath { variable: String, path: PathBuf },
    /// The program was found but could not be started.
    Start { path: PathBuf, source: io::Error },
    /// The program ran past the probe's timeout and was killed.
    Timeout { path: PathBuf, timeout: Duration },
    /// The program wrote more than the probe's byte limit to one stream and was killed.
    OutputLimit { path: PathBuf, limit: usize },
    /// The program's output or exit status could not be read.
    Read { path: PathBuf, source: io::Error },
    /// The program exited with a failure status where success was required.
    Failed { path: PathBuf, status: ExitStatus },
}

impl OracleError {
    /// Whether the program is simply not installed, the one error a test may skip on.
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing { .. })
    }
}

impl fmt::Display for OracleError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName { program } => write!(f, "{program:?} is not a program name"),
            Self::InvalidTimeout { timeout } => write!(f, "a probe timeout of {timeout:?} is too long to set a deadline"),
            Self::InvalidVariable { variable } => write!(f, "{variable:?} is not an environment variable name"),
            Self::Missing { program } => {
                write!(f, "`{program}` was not found on PATH or in the known install folders ({})", install_hint(program))
            }
            Self::InvalidExplicitPath { variable, path } => write!(f, "{variable} is set but `{}` is not a file the OS can start", path.display()),
            Self::Start { path, source } => write!(f, "could not start `{}`: {source}", path.display()),
            Self::Timeout { path, timeout } => write!(f, "`{}` did not finish within {timeout:?} and was killed", path.display()),
            Self::OutputLimit { path, limit } => write!(f, "`{}` wrote more than {limit} bytes to one stream and was killed", path.display()),
            Self::Read { path, source } => write!(f, "could not read the output of `{}`: {source}", path.display()),
            Self::Failed { path, status } => write!(f, "`{}` exited with {status}", path.display()),
        }
    }
}

impl std::error::Error for OracleError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Start { source, .. } | Self::Read { source, .. } => Some(source),
            _ => None,
        }
    }
}

/// How to install a program, for the message of a missing one.
fn install_hint(program: &str) -> &'static str {
    match program {
        "ffmpeg" | "ffprobe" => "apt install ffmpeg / brew install ffmpeg",
        "qpdf" => "apt install qpdf / brew install qpdf",
        "pdftoppm" | "pdftotext" => "apt install poppler-utils / brew install poppler",
        _ => "install it and put it on PATH",
    }
}

/// The default explicit-path variable for `program`: `CRAFT_` and the program name in upper case,
/// with each character other than a letter or digit written as `_` (`CRAFT_FFMPEG`).
pub fn explicit_var(program: &str) -> String {
    let name: String = program.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_uppercase() } else { '_' }).collect();
    format!("CRAFT_{name}")
}

/// The install folders searched after `PATH` on this platform.
pub fn known_dirs() -> Vec<PathBuf> {
    let dirs = if cfg!(windows) { WINDOWS_DIRS } else { UNIX_DIRS };
    dirs.iter().map(PathBuf::from).collect()
}

/// Find `program`, reading its explicit path from `CRAFT_<PROGRAM>`, with [`ProbeLimits::VERSION`].
pub fn find(program: &str, version_arg: &str) -> Result<Oracle, OracleError> {
    find_with(program, version_arg, &explicit_var(program), ProbeLimits::default())
}

/// Find `program`, reading its explicit path from `explicit_var`.
pub fn find_with(program: &str, version_arg: &str, explicit_var: &str, limits: ProbeLimits) -> Result<Oracle, OracleError> {
    if !valid_var(explicit_var) {
        return Err(OracleError::InvalidVariable { variable: explicit_var.to_owned() });
    }
    let explicit = std::env::var_os(explicit_var);
    let path_list = std::env::var_os("PATH");
    let pathext = std::env::var_os("PATHEXT");
    let cwd = std::env::current_dir().ok();
    let dirs = known_dirs();
    let search = Search {
        program,
        explicit_label: explicit_var,
        explicit: explicit.as_deref(),
        path_list: path_list.as_deref(),
        pathext: pathext.as_deref(),
        windows: cfg!(windows),
        cwd: cwd.as_deref(),
        known_dirs: &dirs,
    };
    lookup(&search, version_arg, limits)
}

/// Find `program` from the explicit path, `PATH`-style list and folders the caller passes, instead
/// of the process environment. The current directory and, on Windows, `PATHEXT` still come from
/// the process.
pub fn find_in(
    program: &str,
    version_arg: &str,
    explicit: Option<&OsStr>,
    path_list: Option<&OsStr>,
    known_dirs: &[PathBuf],
    limits: ProbeLimits,
) -> Result<Oracle, OracleError> {
    let pathext = std::env::var_os("PATHEXT");
    let cwd = std::env::current_dir().ok();
    let search = Search {
        program,
        explicit_label: "the explicit path",
        explicit,
        path_list,
        pathext: pathext.as_deref(),
        windows: cfg!(windows),
        cwd: cwd.as_deref(),
        known_dirs,
    };
    lookup(&search, version_arg, limits)
}

/// ffmpeg, found with [`find`] and `-version`.
pub fn ffmpeg() -> Result<Oracle, OracleError> {
    find("ffmpeg", "-version")
}

/// ffprobe, found with [`find`] and `-version`.
pub fn ffprobe() -> Result<Oracle, OracleError> {
    find("ffprobe", "-version")
}

/// Whether `ff` lists `format_name` in the name column of `ffmpeg -hide_banner -formats`. A
/// comma-separated name such as `mov,mp4,m4a` matches each of its parts. A run that cannot start,
/// passes `limits` or exits with a failure status is an error, not an unsupported format.
pub fn ffmpeg_supports_format(ff: &Oracle, format_name: &str, limits: ProbeLimits) -> Result<bool, OracleError> {
    let mut cmd = ff.command();
    cmd.args(["-hide_banner", "-formats"]);
    let out = probe(cmd, ff.path(), limits)?;
    if !out.status.success() {
        return Err(OracleError::Failed { path: ff.path().to_path_buf(), status: out.status });
    }
    Ok(formats_output_lists(&String::from_utf8_lossy(&out.stdout), format_name))
}

/// The version in an ffmpeg banner (`8.0.1`, `7:6.1.1-3ubuntu5`), when `ff`'s version line is one.
pub fn ffmpeg_version(ff: &Oracle) -> Option<String> {
    ffmpeg_version_in(ff.version())
}

fn ffmpeg_version_in(output: &str) -> Option<String> {
    let first = output.lines().next()?;
    let mut words = first.split_whitespace();
    if words.next()? != "ffmpeg" || words.next()? != "version" {
        return None;
    }
    words.next().map(str::to_owned)
}

/// Whether `ffmpeg -formats` output lists `format_name` in its name column.
fn formats_output_lists(output: &str, format_name: &str) -> bool {
    output.lines().any(|line| format_names(line).contains(&format_name))
}

/// The names in one row of `ffmpeg -formats`: the flag column (`D`, `E`, `DE`, and a separate `d`
/// for devices), then the name column. Header and legend rows yield no names.
fn format_names(line: &str) -> Vec<&str> {
    let mut tokens = line.split_whitespace();
    let Some(flags) = tokens.next() else { return Vec::new() };
    if !flags.chars().all(|c| matches!(c, 'D' | 'E' | 'd')) {
        return Vec::new();
    }
    let rest: Vec<&str> = tokens.collect();
    let name = match rest.as_slice() {
        ["d", name, ..] => *name,
        [name, ..] => *name,
        [] => return Vec::new(),
    };
    name.split(',').collect()
}

/// What a `require_*` macro does with a lookup result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    /// The program is missing. [`skip::skip`](crate::skip::skip) takes this reason and decides
    /// whether the test skips or fails.
    Missing(String),
    /// Fail the test with this message.
    Fail(String),
}

/// Sort a lookup result: a missing program becomes [`Outcome::Missing`] with the error as its
/// reason, and every other error becomes [`Outcome::Fail`]. A `required_var` that cannot name a
/// variable (empty, or holding `=` or a NUL character) becomes [`Outcome::Fail`] whatever the
/// result, as [`skip::required`](crate::skip::required) rejects it. The variable's value is not
/// read here.
pub fn outcome(result: Result<Oracle, OracleError>, required_var: &str) -> Result<Oracle, Outcome> {
    if !valid_var(required_var) {
        return Err(Outcome::Fail(format!("oracle setup failed: {}", crate::skip::RequiredError::BadName { variable: required_var.to_owned() })));
    }
    match result {
        Ok(oracle) => Ok(oracle),
        Err(e) if e.is_missing() => Err(Outcome::Missing(e.to_string())),
        Err(e) => Err(Outcome::Fail(format!("oracle setup failed: {e}"))),
    }
}

/// `let qpdf = require_oracle!(oracle::find("qpdf", "--version"));` binds the oracle, or skips the
/// calling test through [`skip::skip`](crate::skip::skip) and returns from it when the program is
/// missing. `require_oracle!(probe, "PDFCRAFT_ORACLES_REQUIRED")` passes that required variable
/// instead of [`skip::REQUIRED_VAR`](crate::skip::REQUIRED_VAR), and a third argument is the value
/// to return.
///
/// A missing program fails the test when [`skip::required`](crate::skip::required) returns `true`
/// or an error for the required variable. Any other error fails the test as well. The macro fails
/// the test with `panic!`, which expands in the calling test.
#[macro_export]
macro_rules! require_oracle {
    ($probe:expr) => {
        $crate::require_oracle!($probe, $crate::skip::REQUIRED_VAR, ())
    };
    ($probe:expr, $required_var:expr) => {
        $crate::require_oracle!($probe, $required_var, ())
    };
    ($probe:expr, $required_var:expr, $ret:expr) => {{
        // The variable expression is evaluated once, so the name checked is the name read.
        let required_var: &str = &$required_var;
        match $crate::oracle::outcome($probe, required_var) {
            ::core::result::Result::Ok(oracle) => oracle,
            ::core::result::Result::Err($crate::oracle::Outcome::Missing(reason)) => {
                let _ = $crate::skip::skip(&reason, required_var);
                return $ret;
            }
            ::core::result::Result::Err($crate::oracle::Outcome::Fail(message)) => {
                // The panic is how the calling test fails.
                #[allow(clippy::panic)]
                ::core::panic!("{message}");
            }
        }
    }};
}

/// `let ff = require_ffmpeg!();` is [`require_oracle!`](crate::require_oracle) for ffmpeg.
/// `require_ffmpeg!("FILMCRAFT_FFMPEG", "FILMCRAFT_REQUIRE_ORACLES")` names the explicit-path and
/// required variables.
#[macro_export]
macro_rules! require_ffmpeg {
    () => {
        $crate::require_oracle!($crate::oracle::ffmpeg())
    };
    ($explicit_var:expr, $required_var:expr) => {
        $crate::require_oracle!($crate::oracle::find_with("ffmpeg", "-version", $explicit_var, $crate::oracle::ProbeLimits::VERSION), $required_var)
    };
}

/// `let fp = require_ffprobe!();` is [`require_oracle!`](crate::require_oracle) for ffprobe.
/// `require_ffprobe!("FILMCRAFT_FFPROBE", "FILMCRAFT_REQUIRE_ORACLES")` names the explicit-path and
/// required variables.
#[macro_export]
macro_rules! require_ffprobe {
    () => {
        $crate::require_oracle!($crate::oracle::ffprobe())
    };
    ($explicit_var:expr, $required_var:expr) => {
        $crate::require_oracle!($crate::oracle::find_with("ffprobe", "-version", $explicit_var, $crate::oracle::ProbeLimits::VERSION), $required_var)
    };
}

/// The inputs of one lookup.
struct Search<'a> {
    program: &'a str,
    explicit_label: &'a str,
    explicit: Option<&'a OsStr>,
    path_list: Option<&'a OsStr>,
    pathext: Option<&'a OsStr>,
    windows: bool,
    cwd: Option<&'a Path>,
    known_dirs: &'a [PathBuf],
}

/// The cache key: every input that can change a lookup's result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
struct Key {
    program: String,
    version_arg: String,
    explicit_label: String,
    explicit: Option<OsString>,
    path_list: Option<OsString>,
    pathext: Option<OsString>,
    windows: bool,
    cwd: Option<PathBuf>,
    known_dirs: Vec<PathBuf>,
    limits: ProbeLimits,
}

impl Key {
    fn new(s: &Search<'_>, version_arg: &str, limits: ProbeLimits) -> Self {
        Self {
            program: s.program.to_owned(),
            version_arg: version_arg.to_owned(),
            explicit_label: s.explicit_label.to_owned(),
            explicit: s.explicit.map(OsStr::to_os_string),
            path_list: s.path_list.map(OsStr::to_os_string),
            pathext: s.pathext.map(OsStr::to_os_string),
            windows: s.windows,
            cwd: s.cwd.map(Path::to_path_buf),
            known_dirs: s.known_dirs.to_vec(),
            limits,
        }
    }
}

type CacheKey = (Key, PathBuf);
type ProbeSlot = Arc<Mutex<Option<Oracle>>>;
type OracleCache = Mutex<HashMap<CacheKey, CacheEntry>>;

#[derive(Default)]
struct CacheEntry {
    value: ProbeSlot,
    users: usize,
}

struct CacheLease {
    key: CacheKey,
    value: ProbeSlot,
}

impl Drop for CacheLease {
    fn drop(&mut self) {
        let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
        let Some(entry) = cache.get_mut(&self.key).filter(|entry| Arc::ptr_eq(&entry.value, &self.value)) else { return };
        entry.users = entry.users.saturating_sub(1);
        if entry.users == 0 && entry.value.lock().unwrap_or_else(PoisonError::into_inner).is_none() {
            cache.remove(&self.key);
        }
    }
}

fn cache() -> &'static OracleCache {
    static CACHE: OnceLock<OracleCache> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Find, probe and cache one program. The cache lock is not held while the program runs.
fn lookup(s: &Search<'_>, version_arg: &str, limits: ProbeLimits) -> Result<Oracle, OracleError> {
    lookup_with_probe(s, version_arg, limits, |path| {
        let mut cmd = Command::new(path);
        cmd.arg(version_arg);
        let out = probe(cmd, path, limits)?;
        Ok(Oracle { program: s.program.to_owned(), path: path.to_owned(), version: version_line(&out.stdout, &out.stderr) })
    })
}

fn lookup_with_probe(
    s: &Search<'_>,
    version_arg: &str,
    limits: ProbeLimits,
    mut probe: impl FnMut(&Path) -> Result<Oracle, OracleError>,
) -> Result<Oracle, OracleError> {
    let mut denied = HashSet::new();
    loop {
        let path = if denied.is_empty() { locate(s)? } else { locate_if(s, |path| !denied.contains(path))? };
        let key = (Key::new(s, version_arg, limits), path.clone());
        match cached_probe(key, || {}, || probe(&path)) {
            Err(OracleError::Start { source, .. }) if retry_denied_probe(&source) => {
                if let Some(explicit) = s.explicit.filter(|value| !value.is_empty()) {
                    return Err(OracleError::InvalidExplicitPath { variable: s.explicit_label.to_owned(), path: PathBuf::from(explicit) });
                }
                denied.insert(path);
            }
            result => return result,
        }
    }
}

fn cached_probe(key: CacheKey, ready: impl FnOnce(), probe: impl FnOnce() -> Result<Oracle, OracleError>) -> Result<Oracle, OracleError> {
    let value = {
        let mut cache = cache().lock().unwrap_or_else(PoisonError::into_inner);
        let entry = cache.entry(key.clone()).or_default();
        entry.users = entry.users.saturating_add(1);
        Arc::clone(&entry.value)
    };
    let lease = CacheLease { key, value };
    ready();
    let mut slot = lease.value.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some(hit) = slot.as_ref() {
        return Ok(hit.clone());
    }
    let oracle = probe()?;
    *slot = Some(oracle.clone());
    Ok(oracle)
}

/// Whether `name` can name an environment variable. `std::env::var_os` reads an empty name, or one
/// with `=` or NUL, as unset (its documentation allows a panic), so such a name is an error here.
fn valid_var(name: &str) -> bool {
    !name.is_empty() && !name.contains(['=', '\0'])
}

/// Whether `program` is a bare file name. `:` is refused because on Windows `C:name` joined to a
/// folder is a path on drive `C:` instead of a file in that folder.
fn valid_name(program: &str) -> bool {
    !program.is_empty() && program != "." && program != ".." && !program.contains(['/', '\\', ':', '\0'])
}

/// Returns an absolute path for `dir`. A relative path requires `cwd`.
/// Windows drive-relative paths use `cwd` on the same drive, or that drive's current directory.
fn anchored(dir: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    if dir.is_absolute() {
        return Some(dir.to_path_buf());
    }
    let cwd = cwd?;
    #[cfg(windows)]
    {
        use std::path::{Component, Prefix};
        let mut parts = dir.components();
        if let Some(Component::Prefix(prefix)) = parts.next() {
            let same_drive = cwd.components().next().is_some_and(|part| match part {
                Component::Prefix(base) => match (prefix.kind(), base.kind()) {
                    (Prefix::Disk(a), Prefix::Disk(b)) => a.eq_ignore_ascii_case(&b),
                    (a, b) => a == b,
                },
                _ => false,
            });
            let base = if same_drive { cwd.to_path_buf() } else { std::path::absolute(prefix.as_os_str()).ok()? };
            return Some(base.join(parts.as_path()));
        }
    }
    Some(cwd.join(dir))
}

/// The file names tried in each folder, in order.
fn candidate_names(program: &str, windows: bool, pathext: Option<&OsStr>) -> Vec<String> {
    if Path::new(program).extension().is_some() {
        return vec![program.to_owned()];
    }
    if !windows {
        return vec![program.to_owned(), format!("{program}.exe")];
    }
    let mut suffixes = vec![".exe".to_owned()];
    for ext in pathext.map(OsStr::to_string_lossy).unwrap_or_default().split(';') {
        let ext = ext.trim().to_ascii_lowercase();
        if WINDOWS_SUFFIXES.contains(&ext.as_str()) && !suffixes.contains(&ext) {
            suffixes.push(ext);
        }
    }
    suffixes.iter().map(|ext| format!("{program}{ext}")).collect()
}

/// Whether the OS can start the file at `p`: on Windows a file whose extension, in any case, is one of
/// [`WINDOWS_SUFFIXES`], and on Unix a file the current process has permission to execute.
fn startable(p: &Path, windows: bool) -> bool {
    if !p.is_file() {
        return false;
    }
    if windows {
        let ext = p.extension().map(|e| format!(".{}", e.to_string_lossy().to_ascii_lowercase()));
        return ext.is_some_and(|e| WINDOWS_SUFFIXES.contains(&e.as_str()));
    }
    executable(p)
}

#[cfg(unix)]
fn executable(p: &Path) -> bool {
    execute_permission(rustix::fs::accessat(rustix::fs::CWD, p, rustix::fs::Access::EXEC_OK, rustix::fs::AtFlags::EACCESS))
}

#[cfg(unix)]
fn execute_permission(result: rustix::io::Result<()>) -> bool {
    !matches!(result, Err(rustix::io::Errno::ACCESS | rustix::io::Errno::NOENT | rustix::io::Errno::NOTDIR))
}

#[cfg(unix)]
fn retry_denied_probe(error: &io::Error) -> bool {
    error.kind() == io::ErrorKind::PermissionDenied
}

#[cfg(not(unix))]
fn retry_denied_probe(_: &io::Error) -> bool {
    false
}

#[cfg(not(unix))]
fn executable(_: &Path) -> bool {
    true
}

/// The absolute path of `s.program`, in the search order the module documentation gives.
fn locate(s: &Search<'_>) -> Result<PathBuf, OracleError> {
    locate_if(s, |_| true)
}

fn locate_if(s: &Search<'_>, mut accept: impl FnMut(&Path) -> bool) -> Result<PathBuf, OracleError> {
    if !valid_name(s.program) {
        return Err(OracleError::InvalidName { program: s.program.to_owned() });
    }
    if let Some(explicit) = s.explicit.filter(|e| !e.is_empty()) {
        let given = PathBuf::from(explicit);
        return match anchored(&given, s.cwd) {
            Some(p) if startable(&p, s.windows) && accept(&p) => Ok(p),
            _ => Err(OracleError::InvalidExplicitPath { variable: s.explicit_label.to_owned(), path: given }),
        };
    }
    let names = candidate_names(s.program, s.windows, s.pathext);
    let on_path = s.path_list.map(|list| std::env::split_paths(list).collect::<Vec<_>>()).unwrap_or_default();
    let dirs = on_path.iter().chain(s.known_dirs).filter(|d| !d.as_os_str().is_empty()).filter_map(|d| anchored(d, s.cwd));
    for dir in dirs {
        for name in &names {
            let p = dir.join(name);
            if startable(&p, s.windows) && accept(&p) {
                return Ok(p);
            }
        }
    }
    Err(OracleError::Missing { program: s.program.to_owned() })
}

/// The first non-blank line of `stdout`, or of `stderr` when `stdout` is blank, trimmed.
fn version_line(stdout: &[u8], stderr: &[u8]) -> String {
    let stdout = String::from_utf8_lossy(stdout);
    let stderr = String::from_utf8_lossy(stderr);
    stdout.lines().chain(stderr.lines()).map(str::trim).find(|line| !line.is_empty()).unwrap_or_default().to_owned()
}

/// Run `cmd` through the crate's process runner under `limits`, with errors that name `path`.
fn probe(cmd: Command, path: &Path, limits: ProbeLimits) -> Result<Output, OracleError> {
    let run_limits = RunLimits { max_output_bytes: limits.max_output_bytes, timeout: Some(limits.timeout) };
    process::run(cmd, run_limits, &mut Observed::default()).map_err(|e| probe_error(path, e))
}

fn probe_error(path: &Path, e: RunError) -> OracleError {
    let path = path.to_path_buf();
    match e {
        RunError::InvalidTimeout(timeout) => OracleError::InvalidTimeout { timeout },
        RunError::Start(source) => OracleError::Start { path, source },
        RunError::Timeout(timeout) => OracleError::Timeout { path, timeout },
        RunError::OutputLimit { limit, .. } => OracleError::OutputLimit { path, limit },
        RunError::Read(source) => OracleError::Read { path, source },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn concurrent_cache_misses_share_one_successful_probe() {
        let folder = Scratch::new("concurrent-cache");
        let input = search("tool", None, None, &[]);
        let key = (Key::new(&input, "--version", ProbeLimits::VERSION), folder.path().join("tool"));
        let calls = AtomicUsize::new(0);
        let ready = std::sync::Barrier::new(3);
        let run = || {
            cached_probe(
                key.clone(),
                || {
                    ready.wait();
                },
                || {
                    let call = calls.fetch_add(1, Ordering::SeqCst);
                    Ok(Oracle { program: "tool".into(), path: key.1.clone(), version: format!("banner {call}") })
                },
            )
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(run);
            let second = scope.spawn(run);
            ready.wait();
            let first = first.join().unwrap().unwrap();
            let second = second.join().unwrap().unwrap();
            assert_eq!(calls.load(Ordering::SeqCst), 1);
            assert_eq!(first, second);
        });
    }

    #[test]
    fn failed_concurrent_probes_leave_no_idle_cache_entry() {
        let folder = Scratch::new("failed-concurrent-cache");
        let input = search("tool", None, None, &[]);
        let key = (Key::new(&input, "--version", ProbeLimits::VERSION), folder.path().join("tool"));
        let ready = std::sync::Barrier::new(3);
        let run = || {
            cached_probe(
                key.clone(),
                || {
                    ready.wait();
                },
                || Err(OracleError::Missing { program: "tool".into() }),
            )
        };
        std::thread::scope(|scope| {
            let first = scope.spawn(run);
            let second = scope.spawn(run);
            ready.wait();
            assert!(first.join().unwrap().is_err());
            assert!(second.join().unwrap().is_err());
        });
        assert!(!cache().lock().unwrap_or_else(PoisonError::into_inner).contains_key(&key));
        let expected = Oracle { program: "tool".into(), path: key.1.clone(), version: "retry succeeded".into() };
        assert_eq!(cached_probe(key.clone(), || {}, || Ok(expected.clone())).unwrap(), expected);
        assert_eq!(cached_probe(key, || {}, || Err(OracleError::Missing { program: "tool".into() })).unwrap(), expected);
    }

    #[cfg(unix)]
    #[test]
    fn an_unavailable_permission_query_does_not_report_a_missing_program() {
        use rustix::io::Errno;
        assert!(execute_permission(Err(Errno::PERM)));
        assert!(execute_permission(Err(Errno::NOSYS)));
        assert!(execute_permission(Ok(())));
        assert!(!execute_permission(Err(Errno::ACCESS)));
        assert!(!execute_permission(Err(Errno::NOENT)));
        assert!(!execute_permission(Err(Errno::NOTDIR)));
    }

    #[cfg(unix)]
    #[test]
    fn a_probe_permission_denial_continues_the_path_search() {
        let first = Scratch::new("denied-probe");
        let second = Scratch::new("usable-probe");
        let denied = first.file("tool");
        let usable = second.file("tool");
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        let input = search("tool", None, Some(&path), &[]);
        let probe = |path: &Path| {
            if path == denied {
                Err(OracleError::Start { path: path.to_owned(), source: io::Error::from(io::ErrorKind::PermissionDenied) })
            } else {
                Ok(Oracle { program: "tool".into(), path: path.to_owned(), version: "second".into() })
            }
        };
        assert_eq!(lookup_with_probe(&input, "--version", ProbeLimits::VERSION, probe).unwrap().path(), usable);
        let explicit = search("tool", Some(denied.as_os_str()), None, &[]);
        assert!(matches!(lookup_with_probe(&explicit, "--version", ProbeLimits::VERSION, probe), Err(OracleError::InvalidExplicitPath { .. })));
        let other = lookup_with_probe(&input, "--other", ProbeLimits::VERSION, |path| {
            Err(OracleError::Start { path: path.to_owned(), source: io::Error::other("not a permission denial") })
        });
        assert!(matches!(other, Err(OracleError::Start { path, .. }) if path == denied));
    }

    /// A new folder under the system temporary folder, removed with its contents on drop.
    struct Scratch(PathBuf);

    impl Scratch {
        fn new(tag: &str) -> Self {
            static N: AtomicUsize = AtomicUsize::new(0);
            let n = N.fetch_add(1, Ordering::Relaxed);
            let nanos = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let dir = std::env::temp_dir().join(format!("craft-testkit-oracle-{tag}-{}-{nanos:x}-{n}", std::process::id()));
            std::fs::create_dir(&dir).unwrap();
            Self(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }

        /// An empty file named `name`, with its execute bits set on Unix.
        fn file(&self, name: &str) -> PathBuf {
            let p = self.0.join(name);
            std::fs::write(&p, b"").unwrap();
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
            }
            p
        }
    }

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn search<'a>(program: &'a str, explicit: Option<&'a OsStr>, path_list: Option<&'a OsStr>, known_dirs: &'a [PathBuf]) -> Search<'a> {
        Search { program, explicit_label: "CRAFT_TEST_EXPLICIT", explicit, path_list, pathext: None, windows: false, cwd: None, known_dirs }
    }

    /// A shell script named `name` in `dir` that runs `body`.
    #[cfg(unix)]
    fn script(dir: &Path, name: &str, body: &str) -> PathBuf {
        use std::os::unix::fs::PermissionsExt;
        let p = dir.join(name);
        std::fs::write(&p, format!("#!/bin/sh\n{body}\n")).unwrap();
        std::fs::set_permissions(&p, std::fs::Permissions::from_mode(0o755)).unwrap();
        p
    }

    #[cfg(unix)]
    fn find_on(dir: &Path, program: &str, version_arg: &str, limits: ProbeLimits) -> Result<Oracle, OracleError> {
        find_in(program, version_arg, None, Some(dir.as_os_str()), &[], limits)
    }

    // Moved from FilmCraft `crates/testkit/src/oracle.rs`.

    #[test]
    fn explicit_path_wins_and_bad_path_errors() {
        let d = Scratch::new("explicit");
        let f = d.file("my-ffmpeg");
        assert_eq!(locate(&search("ffmpeg", Some(f.as_os_str()), None, &[])).unwrap(), f);
        let missing = d.path().join("nope");
        assert!(matches!(locate(&search("ffmpeg", Some(missing.as_os_str()), None, &[])), Err(OracleError::InvalidExplicitPath { .. })));
        // An empty value counts as unset.
        assert!(locate(&search("ffmpeg", Some(OsStr::new("")), None, &[])).unwrap_err().is_missing());
    }

    #[test]
    fn path_search_finds_plain_and_exe_names() {
        let a = Scratch::new("path-a");
        let b = Scratch::new("path-b");
        let exe = b.file("ffprobe.exe");
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(locate(&search("ffprobe", None, Some(&path), &[])).unwrap(), exe);
        let plain = a.file("ffprobe");
        assert_eq!(locate(&search("ffprobe", None, Some(&path), &[])).unwrap(), plain, "earlier PATH entry wins");
    }

    #[test]
    fn known_dirs_are_the_fallback() {
        let d = Scratch::new("known");
        let f = d.file("ffmpeg");
        let dirs = [d.path().to_path_buf()];
        assert_eq!(locate(&search("ffmpeg", None, Some(OsStr::new("")), &dirs)).unwrap(), f);
        assert!(locate(&search("ffmpeg-does-not-exist", None, None, &dirs)).unwrap_err().is_missing());
    }

    const FORMATS_WITH_APV: &str =
        "File formats:\n D  mov             QuickTime / MOV\n DE apv             APV raw bitstream\n D  mp4             MP4 (MPEG-4 Part 14)\n";
    const FORMATS_WITHOUT_APV: &str =
        "File formats:\n D  mov             QuickTime / MOV\n D  mp4             MP4 (MPEG-4 Part 14)\n D  oapv            Obsolete APV alias\n";

    #[test]
    fn formats_listing_detects_a_supported_format() {
        assert!(formats_output_lists(FORMATS_WITH_APV, "apv"));
        // Neither a longer name containing it (oapv) nor an uppercase mention (APV) counts.
        assert!(!formats_output_lists(FORMATS_WITHOUT_APV, "apv"));
        assert!(!formats_output_lists("", "apv"));
    }

    #[test]
    fn version_string_parses_ffmpeg_banner() {
        assert_eq!(ffmpeg_version_in("ffmpeg version 7:6.1.1-3ubuntu5 Copyright (c) 2000-2023"), Some("7:6.1.1-3ubuntu5".to_string()));
        assert_eq!(ffmpeg_version_in("ffmpeg version 8.0.1-full_build-www.gyan.dev xxx"), Some("8.0.1-full_build-www.gyan.dev".to_string()));
        assert_eq!(ffmpeg_version_in(""), None);
        assert_eq!(ffmpeg_version_in("ffprobe version 6.1"), None);
        assert_eq!(ffmpeg_version_in("ffmpeg version "), None);
        assert_eq!(ffmpeg_version_in("ffmpeg versioning 1.0"), None);
        assert_eq!(ffmpeg_version_in("ffmpeg version\t6.1"), Some("6.1".to_string()));
    }

    /// A stand-in ffmpeg answers the format query the way its listing says.
    #[cfg(unix)]
    #[test]
    fn live_ffmpeg_reports_its_own_apv_support() {
        let d = Scratch::new("live");
        let listing = FORMATS_WITH_APV.replace('\n', "\\n");
        script(d.path(), "ffmpeg", &format!("case \"$*\" in *-formats*) printf '{listing}' ;; *) echo 'ffmpeg version 8.0.1 Copyright' ;; esac"));
        let ff = find_on(d.path(), "ffmpeg", "-version", ProbeLimits::VERSION).unwrap();
        assert!(ffmpeg_supports_format(&ff, "apv", ProbeLimits::LISTING).unwrap());
        assert!(!ffmpeg_supports_format(&ff, "oapv", ProbeLimits::LISTING).unwrap());
        assert_eq!(ffmpeg_version(&ff).as_deref(), Some("8.0.1"));
    }

    // Moved from PdfCraft `crates/oracle/src/lib.rs` and craft-libs `crates/pdf/tests/cos/oracle.rs`.

    #[test]
    fn only_a_missing_oracle_goes_to_the_skip_rule() {
        let Err(Outcome::Missing(reason)) = outcome(Err(OracleError::Missing { program: "qpdf".into() }), "PDFCRAFT_ORACLES_REQUIRED") else {
            panic!("an absent program must go to the skip rule")
        };
        assert!(reason.contains("qpdf"), "{reason}");
        assert!(reason.contains("apt install qpdf"), "{reason}");
        // An error other than absence fails without reading any required variable.
        let bad = Err(OracleError::InvalidExplicitPath { variable: "CRAFT_QPDF".into(), path: "/nope".into() });
        let Err(Outcome::Fail(message)) = outcome(bad, "PDFCRAFT_ORACLES_REQUIRED") else { panic!("a wrong explicit path must fail") };
        assert!(message.starts_with("oracle setup failed"), "{message}");
    }

    /// A found program does not hide a required variable name that `skip::required` rejects.
    #[cfg(unix)]
    #[test]
    fn a_found_oracle_still_fails_on_an_invalid_required_variable_name() {
        let d = Scratch::new("found-name");
        script(d.path(), "qpdf", "echo 'qpdf version 11.9.0'");
        let found = || find_on(d.path(), "qpdf", "--version", ProbeLimits::VERSION);
        assert!(outcome(found(), "PDFCRAFT_ORACLES_REQUIRED").is_ok());
        let decided = outcome(found(), "CRAFT=REQUIRED");
        assert!(matches!(decided, Err(Outcome::Fail(ref m)) if m.contains("cannot name a variable")), "{decided:?}");
    }

    #[test]
    fn a_program_that_does_not_exist_is_absent_with_a_reason() {
        let Err(absent) = find("craft-testkit-no-such-oracle-binary", "--version") else {
            panic!("a nonexistent program must not be reported as present");
        };
        assert!(absent.is_missing());
        let text = absent.to_string();
        assert!(text.contains("craft-testkit-no-such-oracle-binary"), "{text}");
        assert!(text.contains("not found"), "{text}");
    }

    #[test]
    fn hostile_program_names_are_absent_not_a_panic() {
        for name in ["", ".", "..", "\0bad", "../../etc/passwd", r"..\..\windows\system32\cmd.exe", "C:ffmpeg", "ffmpeg:stream"] {
            assert!(matches!(find(name, "--version"), Err(OracleError::InvalidName { .. })), "{name:?}");
        }
        for name in [" ", &"x".repeat(4096)] {
            assert!(find(name, "--version").is_err(), "{name:?}");
        }
    }

    #[test]
    fn every_known_oracle_has_an_install_hint() {
        for p in ["qpdf", "pdftoppm", "pdftotext", "ffmpeg", "ffprobe"] {
            let hint = install_hint(p);
            assert!(hint.contains("install"), "{p}: {hint}");
            assert_ne!(hint, install_hint("something-else"), "{p} needs its own hint");
        }
    }

    #[test]
    fn probing_is_stable() {
        let a = find("qpdf", "--version").map(|o| o.path().to_path_buf());
        let b = find("qpdf", "--version").map(|o| o.path().to_path_buf());
        assert_eq!(a.is_ok(), b.is_ok());
        if let (Ok(a), Ok(b)) = (a, b) {
            assert_eq!(a, b);
        }
    }

    // Behavior of the shared module.

    #[test]
    fn invalid_variable_names_are_errors() {
        for name in ["", "CRAFT=FFMPEG", "CRAFT_\0FFMPEG"] {
            let found = find_with("craft-testkit-no-such-oracle-binary", "--version", name, ProbeLimits::VERSION);
            assert!(matches!(found, Err(OracleError::InvalidVariable { .. })), "{name:?}: {found:?}");
            let decided = outcome(Err(OracleError::Missing { program: "qpdf".into() }), name);
            assert!(matches!(decided, Err(Outcome::Fail(_))), "{name:?}: {decided:?}");
        }
    }

    #[test]
    fn wrong_explicit_path_never_falls_back() {
        let d = Scratch::new("no-fallback");
        d.file("ffmpeg");
        let dirs = [d.path().to_path_buf()];
        let missing = d.path().join("not-here");
        let found = locate(&search("ffmpeg", Some(missing.as_os_str()), Some(d.path().as_os_str()), &dirs));
        let Err(OracleError::InvalidExplicitPath { variable, path }) = found else { panic!("expected an explicit-path error, got {found:?}") };
        assert_eq!(variable, "CRAFT_TEST_EXPLICIT");
        assert_eq!(path, missing);
    }

    #[test]
    fn relative_path_entry_is_anchored_before_cache() {
        let a = Scratch::new("rel-a");
        let b = Scratch::new("rel-b");
        std::fs::create_dir(a.path().join("bin")).unwrap();
        a.file("bin/ffmpeg");
        let mut s = search("ffmpeg", None, Some(OsStr::new("bin")), &[]);
        s.cwd = Some(a.path());
        let found = locate(&s).unwrap();
        assert!(found.is_absolute(), "{}", found.display());
        assert_eq!(found, a.path().join("bin").join("ffmpeg"));
        s.cwd = Some(b.path());
        assert!(locate(&s).unwrap_err().is_missing());
        // The current directory is part of the cache key.
        let key_a =
            Key::new(&Search { cwd: Some(a.path()), ..search("ffmpeg", None, Some(OsStr::new("bin")), &[]) }, "-version", ProbeLimits::VERSION);
        let key_b =
            Key::new(&Search { cwd: Some(b.path()), ..search("ffmpeg", None, Some(OsStr::new("bin")), &[]) }, "-version", ProbeLimits::VERSION);
        assert_ne!(key_a, key_b);
    }

    #[cfg(windows)]
    #[test]
    fn drive_relative_paths_use_the_captured_directory() {
        let directory = Scratch::new("drive-relative");
        let expected = directory.file("qpdf.exe");
        let drive = directory.path().components().next().unwrap();
        let relative = PathBuf::from(drive.as_os_str()).join("qpdf.exe");
        let mut inputs = search("qpdf", Some(relative.as_os_str()), None, &[]);
        inputs.windows = true;
        inputs.cwd = Some(directory.path());
        let found = locate(&inputs).unwrap();
        assert!(found.is_absolute(), "{}", found.display());
        assert_eq!(found, expected);
    }

    #[test]
    fn windows_exe_and_pathext_order_is_honored() {
        let pathext = OsStr::new(".COM;.EXE;.BAT;.CMD;.VBS;.cmd;;");
        assert_eq!(candidate_names("ffmpeg", true, Some(pathext)), ["ffmpeg.exe", "ffmpeg.com", "ffmpeg.bat", "ffmpeg.cmd"]);
        assert_eq!(candidate_names("ffmpeg", true, None), ["ffmpeg.exe"]);
        assert_eq!(candidate_names("ffmpeg.exe", true, Some(pathext)), ["ffmpeg.exe"]);
        assert_eq!(candidate_names("ffmpeg", false, Some(pathext)), ["ffmpeg", "ffmpeg.exe"]);
        let d = Scratch::new("pathext");
        let cmd = d.file("tool.cmd");
        let mut s = search("tool", None, Some(d.path().as_os_str()), &[]);
        s.windows = true;
        s.pathext = Some(pathext);
        assert_eq!(locate(&s).unwrap(), cmd);
        let exe = d.file("tool.exe");
        assert_eq!(locate(&s).unwrap(), exe);
    }

    /// A file without an execute bit is skipped on the search path and refused as an explicit path.
    #[cfg(unix)]
    #[test]
    fn a_file_without_an_execute_bit_is_skipped() {
        use std::os::unix::fs::PermissionsExt;
        let a = Scratch::new("noexec-a");
        let b = Scratch::new("noexec-b");
        let data = a.file("ffmpeg");
        std::fs::set_permissions(&data, std::fs::Permissions::from_mode(0o644)).unwrap();
        let exe = b.file("ffmpeg");
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        assert_eq!(locate(&search("ffmpeg", None, Some(&path), &[])).unwrap(), exe);
        let found = locate(&search("ffmpeg", Some(data.as_os_str()), None, &[]));
        assert!(matches!(found, Err(OracleError::InvalidExplicitPath { .. })), "{found:?}");
    }

    #[cfg(unix)]
    #[test]
    fn path_lookup_uses_the_current_process_execute_permission() {
        use std::os::unix::fs::PermissionsExt;
        let first = Scratch::new("owner-noexec");
        let second = Scratch::new("owner-exec");
        let candidate = script(first.path(), "qpdf", "printf 'first\\n'");
        let usable = script(second.path(), "qpdf", "printf 'second\\n'");
        let path = std::env::join_paths([first.path(), second.path()]).unwrap();
        for mode in [0o601, 0o610] {
            std::fs::set_permissions(&candidate, std::fs::Permissions::from_mode(mode)).unwrap();
            let can_start = Command::new(&candidate).output().is_ok();
            let found = find_in("qpdf", "--version", None, Some(&path), &[], ProbeLimits::default()).unwrap();
            assert_eq!(found.path(), if can_start { candidate.as_path() } else { usable.as_path() });
            let explicit = locate(&search("qpdf", Some(candidate.as_os_str()), None, &[]));
            if can_start {
                assert_eq!(explicit.unwrap(), candidate);
            } else {
                assert!(matches!(explicit, Err(OracleError::InvalidExplicitPath { .. })), "{explicit:?}");
            }
        }
    }

    /// On Windows only a `.com`, `.exe`, `.bat` or `.cmd` file, in any case, is found or accepted.
    #[test]
    fn windows_skips_files_it_cannot_start() {
        let a = Scratch::new("winstart-a");
        let b = Scratch::new("winstart-b");
        a.file("tool");
        let script = a.file("tool.ps1");
        let exe = b.file("tool.exe");
        let upper = b.file("TOOL.CMD");
        let path = std::env::join_paths([a.path(), b.path()]).unwrap();
        let mut s = search("tool", None, Some(&path), &[]);
        s.windows = true;
        assert_eq!(locate(&s).unwrap(), exe, "a bare name in an earlier folder does not shadow tool.exe");
        s.program = "tool.ps1";
        assert!(locate(&s).unwrap_err().is_missing());
        s.program = "tool";
        s.path_list = None;
        s.explicit = Some(script.as_os_str());
        assert!(matches!(locate(&s), Err(OracleError::InvalidExplicitPath { .. })));
        s.explicit = Some(upper.as_os_str());
        assert_eq!(locate(&s).unwrap(), upper);
    }

    #[test]
    fn format_matches_name_column_only() {
        let listing = "File formats:\n D. = Demuxing supported\n .E = Muxing supported\n --\n D  mov,mp4,m4a,3gp QuickTime / MOV\n D  wrap            holds apv inside\n D d lavfi           Libavfilter virtual input device\n";
        assert!(formats_output_lists(listing, "mp4"));
        assert!(formats_output_lists(listing, "mov"));
        assert!(formats_output_lists(listing, "lavfi"));
        assert!(!formats_output_lists(listing, "apv"), "a description word is not a format name");
        assert!(!formats_output_lists(listing, "="));
        assert!(!formats_output_lists(listing, "formats:"));
        assert!(!formats_output_lists(listing, "d"));
    }

    #[test]
    fn version_line_prefers_stdout_then_stderr() {
        assert_eq!(version_line(b"qpdf version 11.9.0\nRun qpdf --copyright\n", b"ignored"), "qpdf version 11.9.0");
        assert_eq!(version_line(b"\n", b"pdftoppm version 24.02.0\n"), "pdftoppm version 24.02.0");
        assert_eq!(version_line("\u{a0}\n\u{2003}\n".as_bytes(), b"qpdf version 11.9.0\n"), "qpdf version 11.9.0");
        assert_eq!(version_line(b"", b""), "");
    }

    #[cfg(unix)]
    #[test]
    fn different_version_arguments_do_not_share_cache() {
        let d = Scratch::new("version-args");
        script(d.path(), "tool", "echo \"tool $1\"");
        assert_eq!(find_on(d.path(), "tool", "--one", ProbeLimits::VERSION).unwrap().version(), "tool --one");
        assert_eq!(find_on(d.path(), "tool", "--two", ProbeLimits::VERSION).unwrap().version(), "tool --two");
    }

    #[cfg(unix)]
    #[test]
    fn different_explicit_variables_do_not_share_cache() {
        let d = Scratch::new("explicit-vars");
        let first = script(d.path(), "first", "echo first");
        let second = script(d.path(), "second", "echo second");
        let a = find_in("tool", "-v", Some(first.as_os_str()), None, &[], ProbeLimits::VERSION).unwrap();
        let b = find_in("tool", "-v", Some(second.as_os_str()), None, &[], ProbeLimits::VERSION).unwrap();
        assert_eq!((a.path(), a.version()), (first.as_path(), "first"));
        assert_eq!((b.path(), b.version()), (second.as_path(), "second"));
    }

    #[cfg(unix)]
    #[test]
    fn failed_probe_is_retryable() {
        let d = Scratch::new("retry");
        assert!(find_on(d.path(), "tool", "-v", ProbeLimits::VERSION).unwrap_err().is_missing());
        script(d.path(), "tool", "echo tool 1.0");
        assert_eq!(find_on(d.path(), "tool", "-v", ProbeLimits::VERSION).unwrap().version(), "tool 1.0");
    }

    #[cfg(unix)]
    #[test]
    fn version_output_over_limit_returns_error() {
        let d = Scratch::new("flood");
        script(d.path(), "tool", "exec yes");
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::from_secs(60) };
        let got = find_on(d.path(), "tool", "-v", limits);
        assert!(matches!(got, Err(OracleError::OutputLimit { limit: 1024, .. })), "{got:?}");
    }

    #[cfg(unix)]
    #[test]
    fn format_output_over_limit_returns_error() {
        let d = Scratch::new("format-flood");
        script(d.path(), "ffmpeg", "case \"$*\" in *-formats*) exec yes ' D  mov QuickTime' ;; *) echo 'ffmpeg version 8.0' ;; esac");
        let ff = find_on(d.path(), "ffmpeg", "-version", ProbeLimits::VERSION).unwrap();
        let limits = ProbeLimits { max_output_bytes: 4096, timeout: Duration::from_secs(60) };
        let got = ffmpeg_supports_format(&ff, "mov", limits);
        assert!(matches!(got, Err(OracleError::OutputLimit { limit: 4096, .. })), "{got:?}");
    }

    #[cfg(unix)]
    #[test]
    fn format_probe_that_fails_is_an_error() {
        let d = Scratch::new("format-fail");
        script(d.path(), "ffmpeg", "case \"$*\" in *-formats*) exit 3 ;; *) echo 'ffmpeg version 8.0' ;; esac");
        let ff = find_on(d.path(), "ffmpeg", "-version", ProbeLimits::VERSION).unwrap();
        assert!(matches!(ffmpeg_supports_format(&ff, "mov", ProbeLimits::LISTING), Err(OracleError::Failed { .. })));
    }

    /// A probe that runs past its timeout maps to `OracleError::Timeout` with the program's path.
    #[cfg(unix)]
    #[test]
    fn version_probe_past_its_timeout_is_a_timeout() {
        let d = Scratch::new("hang");
        let tool = script(d.path(), "tool", "exec sleep 1000");
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::ZERO };
        let got = find_on(d.path(), "tool", "-v", limits);
        assert!(matches!(got, Err(OracleError::Timeout { ref path, timeout: Duration::ZERO }) if *path == tool), "{got:?}");
    }

    /// A timeout too long to set a deadline maps to `OracleError::InvalidTimeout`.
    #[cfg(unix)]
    #[test]
    fn version_probe_with_an_unbounded_timeout_is_an_error() {
        let d = Scratch::new("no-deadline");
        script(d.path(), "tool", "echo tool 1.0");
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::MAX };
        let got = find_on(d.path(), "tool", "-v", limits);
        assert!(matches!(got, Err(OracleError::InvalidTimeout { timeout: Duration::MAX })), "{got:?}");
    }
}
