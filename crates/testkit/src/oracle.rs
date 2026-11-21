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
//! A file the OS can start is, on Windows, one whose extension is `.com`, `.exe`, `.bat` or `.cmd`
//! in any case, and on Unix one with an execute bit set. The search skips any other file and goes on.
//!
//! The program found is run once with its version argument. [`Oracle::version`] is the first
//! non-blank line of its standard output, or of its standard error when standard output is blank.
//! The exit status is not checked. The run is bounded by [`ProbeLimits`]: each output stream may
//! hold at most `max_output_bytes`, and the child is killed and reaped when it passes `timeout` or
//! the byte limit. On Unix the child runs in its own process group, and the whole group is killed,
//! so a process it started that holds the output pipes ends too. A successful lookup is cached for
//! the process under every input it read (the program, the version argument, the explicit variable
//! and its value, `PATH`, `PATHEXT`, the current directory, the install folders and the limits). A
//! failed lookup is not cached.
//!
//! No function here panics. [`require_oracle!`](crate::require_oracle), [`require_ffmpeg!`](crate::require_ffmpeg)
//! and [`require_ffprobe!`](crate::require_ffprobe) expand in the calling test: a missing program
//! prints a `SKIPPED (oracle)` line and returns from the test, unless the required variable
//! ([`REQUIRED_VAR`] by default) is exactly `1`, in which case the test fails. Every other error,
//! such as a wrong explicit path or a timeout, fails the test.

use std::collections::HashMap;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

/// The default variable that turns a missing oracle from a skip into a test failure. It counts as
/// set only when its value is exactly `1`.
pub const REQUIRED_VAR: &str = "CRAFT_REQUIRE_ORACLES";

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

/// Whether a required variable's value turns skips into failures: only exactly `1` does.
pub fn required(value: Option<&OsStr>) -> bool {
    value.is_some_and(|v| v == OsStr::new("1"))
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
    let out = run_bounded(cmd, ff.path(), limits, &mut Observed::default())?;
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
    let rest = first.strip_prefix("ffmpeg version")?.trim_start();
    let version: String = rest.chars().take_while(|c| !c.is_whitespace()).collect();
    if version.is_empty() { None } else { Some(version) }
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
    /// Print this line and return from the test.
    Skip(String),
    /// Fail the test with this message.
    Fail(String),
}

/// Decide a lookup result under `required_var` as the process environment sets it. An invalid
/// `required_var` fails.
pub fn outcome(result: Result<Oracle, OracleError>, required_var: &str) -> Result<Oracle, Outcome> {
    if !valid_var(required_var) {
        return Err(Outcome::Fail(format!("oracle setup failed: {}", OracleError::InvalidVariable { variable: required_var.to_owned() })));
    }
    outcome_with(result, required_var, required(std::env::var_os(required_var).as_deref()))
}

/// Decide a lookup result: a missing program skips unless `required`; every other error fails.
pub fn outcome_with(result: Result<Oracle, OracleError>, required_var: &str, required: bool) -> Result<Oracle, Outcome> {
    match result {
        Ok(oracle) => Ok(oracle),
        Err(e) if e.is_missing() && !required => Err(Outcome::Skip(format!("SKIPPED (oracle): {e}; set {required_var}=1 to make this a failure"))),
        Err(e) if e.is_missing() => Err(Outcome::Fail(format!("oracle required: {e}, and {required_var}=1 forbids skipping"))),
        Err(e) => Err(Outcome::Fail(format!("oracle setup failed: {e}"))),
    }
}

/// `let qpdf = require_oracle!(oracle::find("qpdf", "--version"));` binds the oracle, or prints a
/// `SKIPPED (oracle)` line and returns from the calling test when the program is missing.
/// `require_oracle!(probe, "PDFCRAFT_ORACLES_REQUIRED")` reads that required variable instead of
/// [`REQUIRED_VAR`](crate::oracle::REQUIRED_VAR), and a third argument is the value to return.
///
/// The test fails when the required variable is exactly `1` and the program is missing, and on
/// any other error. The macro fails the test with `panic!`, which expands in the calling test.
#[macro_export]
macro_rules! require_oracle {
    ($probe:expr) => {
        $crate::require_oracle!($probe, $crate::oracle::REQUIRED_VAR, ())
    };
    ($probe:expr, $required_var:expr) => {
        $crate::require_oracle!($probe, $required_var, ())
    };
    ($probe:expr, $required_var:expr, $ret:expr) => {
        match $crate::oracle::outcome($probe, $required_var) {
            ::core::result::Result::Ok(oracle) => oracle,
            ::core::result::Result::Err($crate::oracle::Outcome::Skip(line)) => {
                ::std::eprintln!("{line}");
                return $ret;
            }
            ::core::result::Result::Err($crate::oracle::Outcome::Fail(message)) => {
                // The panic is how the calling test fails.
                #[allow(clippy::panic)]
                ::core::panic!("{message}");
            }
        }
    };
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

fn cache() -> &'static Mutex<HashMap<Key, Oracle>> {
    static CACHE: OnceLock<Mutex<HashMap<Key, Oracle>>> = OnceLock::new();
    CACHE.get_or_init(Mutex::default)
}

/// Find, probe and cache one program. The cache lock is not held while the program runs.
fn lookup(s: &Search<'_>, version_arg: &str, limits: ProbeLimits) -> Result<Oracle, OracleError> {
    let key = Key::new(s, version_arg, limits);
    if let Some(hit) = cache().lock().unwrap_or_else(PoisonError::into_inner).get(&key) {
        return Ok(hit.clone());
    }
    let path = locate(s)?;
    let mut cmd = Command::new(&path);
    cmd.arg(version_arg);
    let out = run_bounded(cmd, &path, limits, &mut Observed::default())?;
    let oracle = Oracle { program: s.program.to_owned(), path, version: version_line(&out.stdout, &out.stderr) };
    cache().lock().unwrap_or_else(PoisonError::into_inner).insert(key, oracle.clone());
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

/// `dir` as an absolute path: a relative one is joined to `cwd`, or dropped without a `cwd`.
fn anchored(dir: &Path, cwd: Option<&Path>) -> Option<PathBuf> {
    if dir.is_absolute() { Some(dir.to_path_buf()) } else { cwd.map(|c| c.join(dir)) }
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
/// [`WINDOWS_SUFFIXES`], and on Unix a file with an execute bit set.
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
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(p).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
}

#[cfg(not(unix))]
fn executable(_: &Path) -> bool {
    true
}

/// The absolute path of `s.program`, in the search order the module documentation gives.
fn locate(s: &Search<'_>) -> Result<PathBuf, OracleError> {
    if !valid_name(s.program) {
        return Err(OracleError::InvalidName { program: s.program.to_owned() });
    }
    if let Some(explicit) = s.explicit.filter(|e| !e.is_empty()) {
        let given = PathBuf::from(explicit);
        return match anchored(&given, s.cwd) {
            Some(p) if startable(&p, s.windows) => Ok(p),
            _ => Err(OracleError::InvalidExplicitPath { variable: s.explicit_label.to_owned(), path: given }),
        };
    }
    let names = candidate_names(s.program, s.windows, s.pathext);
    let on_path = s.path_list.map(|list| std::env::split_paths(list).collect::<Vec<_>>()).unwrap_or_default();
    let dirs = on_path.iter().chain(s.known_dirs).filter(|d| !d.as_os_str().is_empty()).filter_map(|d| anchored(d, s.cwd));
    for dir in dirs {
        for name in &names {
            let p = dir.join(name);
            if startable(&p, s.windows) {
                return Ok(p);
            }
        }
    }
    Err(OracleError::Missing { program: s.program.to_owned() })
}

/// The first non-blank line of `stdout`, or of `stderr` when `stdout` is blank, trimmed.
fn version_line(stdout: &[u8], stderr: &[u8]) -> String {
    let text = if stdout.iter().all(u8::is_ascii_whitespace) { stderr } else { stdout };
    String::from_utf8_lossy(text).lines().map(str::trim).find(|l| !l.is_empty()).unwrap_or_default().to_owned()
}

/// A finished child's status and output.
#[derive(Debug)]
struct Captured {
    status: ExitStatus,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

enum ReadFail {
    Limit,
    Io(io::Error),
}

/// Read `r` to its end, or stop with [`ReadFail::Limit`] once it holds more than `cap` bytes.
fn read_capped(mut r: impl Read, cap: usize) -> Result<Vec<u8>, ReadFail> {
    let mut buf = Vec::new();
    let mut chunk = [0u8; 8192];
    loop {
        let n = match r.read(&mut chunk) {
            Ok(0) => return Ok(buf),
            Ok(n) => n,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => return Err(ReadFail::Io(e)),
        };
        if buf.len().saturating_add(n) > cap {
            return Err(ReadFail::Limit);
        }
        buf.extend_from_slice(&chunk[..n]);
    }
}

type StreamResult = (bool, Result<Vec<u8>, ReadFail>);

/// Read one pipe on its own thread and send the result as `(is_stdout, bytes)`.
fn spawn_reader(pipe: Option<impl Read + Send + 'static>, is_stdout: bool, cap: usize, tx: Sender<StreamResult>) -> io::Result<()> {
    let Some(pipe) = pipe else {
        let _ = tx.send((is_stdout, Ok(Vec::new())));
        return Ok(());
    };
    std::thread::Builder::new().name("craft-testkit-oracle-pipe".into()).spawn(move || {
        let _ = tx.send((is_stdout, read_capped(pipe, cap)));
    })?;
    Ok(())
}

/// Kill the child and reap it. On Unix the child leads its own process group, and the whole group
/// is killed first with the `kill` built into `/bin/sh` (the shell runs without `PATH`), so a
/// process the child started that still holds the output pipes ends too. On Windows only the child
/// is killed.
fn kill_and_reap(child: &mut Child) {
    #[cfg(unix)]
    let _ = Command::new("/bin/sh")
        .args(["-c", "kill -s KILL -- \"-$1\"", "sh", &child.id().to_string()])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    let _ = child.kill();
    let _ = child.wait();
}

/// How long a failed run waits for its pipe readers to finish after the child is killed.
const PIPE_GRACE: Duration = Duration::from_secs(2);

/// What a test can observe about one bounded run.
#[derive(Debug, Default)]
struct Observed {
    /// The child's process id.
    pid: Option<u32>,
    /// Whether both pipe readers finished before a failed run returned.
    pipes_closed: bool,
}

/// Kill and reap the child, then wait up to [`PIPE_GRACE`] for the `pending` pipe readers.
fn abort(child: &mut Child, rx: &Receiver<StreamResult>, mut pending: usize, seen: &mut Observed, e: OracleError) -> Result<Captured, OracleError> {
    kill_and_reap(child);
    let until = Instant::now().checked_add(PIPE_GRACE).unwrap_or_else(Instant::now);
    while pending > 0 {
        match rx.recv_timeout(until.saturating_duration_since(Instant::now())) {
            Ok(_) => pending -= 1,
            Err(RecvTimeoutError::Disconnected) => pending = 0,
            Err(RecvTimeoutError::Timeout) => break,
        }
    }
    seen.pipes_closed = pending == 0;
    Err(e)
}

/// Start `cmd`, trying again up to 8 times while the program file is busy (`ETXTBSY`). A file is
/// busy for a moment after it was written while another thread starts a process, which happens
/// when a test writes a stand-in program and runs it.
fn spawn(cmd: &mut Command) -> io::Result<Child> {
    let mut pause = Duration::from_millis(1);
    for _ in 0..8 {
        match cmd.spawn() {
            Err(e) if e.kind() == io::ErrorKind::ExecutableFileBusy => std::thread::sleep(pause),
            started => return started,
        }
        pause *= 2;
    }
    cmd.spawn()
}

/// Run `cmd` with standard input closed and both outputs read under `limits`. On a timeout, an
/// output over the limit or a read error, the child is killed and reaped before the error returns.
/// A timeout too long to set a deadline returns [`OracleError::InvalidTimeout`] without starting
/// the program.
fn run_bounded(mut cmd: Command, path: &Path, limits: ProbeLimits, seen: &mut Observed) -> Result<Captured, OracleError> {
    let invalid_timeout = || OracleError::InvalidTimeout { timeout: limits.timeout };
    if Instant::now().checked_add(limits.timeout).is_none() {
        return Err(invalid_timeout());
    }
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::piped());
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = spawn(&mut cmd).map_err(|source| OracleError::Start { path: path.to_path_buf(), source })?;
    seen.pid = Some(child.id());
    // The timeout counts from the start of the child, not from the start of `spawn`'s retries.
    let Some(deadline) = Instant::now().checked_add(limits.timeout) else {
        kill_and_reap(&mut child);
        return Err(invalid_timeout());
    };
    let (tx, rx) = mpsc::channel();
    let read_error = |source| OracleError::Read { path: path.to_path_buf(), source };
    if let Err(source) = spawn_reader(child.stdout.take(), true, limits.max_output_bytes, tx.clone()) {
        return abort(&mut child, &rx, 0, seen, read_error(source));
    }
    if let Err(source) = spawn_reader(child.stderr.take(), false, limits.max_output_bytes, tx) {
        return abort(&mut child, &rx, 1, seen, read_error(source));
    }
    let timeout = OracleError::Timeout { path: path.to_path_buf(), timeout: limits.timeout };
    let (mut stdout, mut stderr) = (None, None);
    let mut pending = 2;
    while pending > 0 {
        let received = rx.recv_timeout(deadline.saturating_duration_since(Instant::now()));
        if received.is_ok() {
            pending -= 1;
        }
        match received {
            Ok((is_stdout, Ok(bytes))) => *(if is_stdout { &mut stdout } else { &mut stderr }) = Some(bytes),
            Ok((_, Err(ReadFail::Limit))) => {
                let limit = OracleError::OutputLimit { path: path.to_path_buf(), limit: limits.max_output_bytes };
                return abort(&mut child, &rx, pending, seen, limit);
            }
            Ok((_, Err(ReadFail::Io(source)))) => return abort(&mut child, &rx, pending, seen, read_error(source)),
            Err(RecvTimeoutError::Timeout) => return abort(&mut child, &rx, pending, seen, timeout),
            Err(RecvTimeoutError::Disconnected) => {
                let source = io::Error::other("a pipe reader stopped without a result");
                return abort(&mut child, &rx, 0, seen, read_error(source));
            }
        }
    }
    let status = wait_for_exit(&mut child, deadline).map_err(|e| match e {
        Some(source) => read_error(source),
        None => timeout,
    })?;
    Ok(Captured { status, stdout: stdout.unwrap_or_default(), stderr: stderr.unwrap_or_default() })
}

/// Wait for a child whose pipes have closed. Past `deadline` it is killed and reaped, and the
/// error is `None`; a failed wait kills and reaps it too and returns the error.
fn wait_for_exit(child: &mut Child, deadline: Instant) -> Result<ExitStatus, Option<io::Error>> {
    let mut pause = Duration::from_millis(1);
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) => {}
            Err(e) => {
                kill_and_reap(child);
                return Err(Some(e));
            }
        }
        let left = deadline.saturating_duration_since(Instant::now());
        if left == Duration::ZERO {
            kill_and_reap(child);
            return Err(None);
        }
        std::thread::sleep(left.min(pause));
        pause = (pause * 2).min(Duration::from_millis(50));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

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
    fn a_missing_oracle_is_fatal_exactly_when_it_is_required() {
        let missing = || Err(OracleError::Missing { program: "qpdf".into() });
        let Err(Outcome::Skip(opt)) = outcome_with(missing(), "PDFCRAFT_ORACLES_REQUIRED", false) else { panic!("optional absence must skip") };
        let Err(Outcome::Fail(req)) = outcome_with(missing(), "PDFCRAFT_ORACLES_REQUIRED", true) else { panic!("required absence must fail") };
        assert!(opt.contains("SKIPPED"), "{opt}");
        assert!(opt.contains("apt install qpdf"), "{opt}");
        assert!(opt.contains("PDFCRAFT_ORACLES_REQUIRED"), "{opt}");
        assert!(req.contains("required"), "{req}");
        assert!(req.contains("forbids skipping"), "{req}");
        assert!(!req.contains("SKIPPED"), "{req}");
        // An error other than absence fails even when nothing is required.
        let bad = Err(OracleError::InvalidExplicitPath { variable: "CRAFT_QPDF".into(), path: "/nope".into() });
        assert!(matches!(outcome_with(bad, REQUIRED_VAR, false), Err(Outcome::Fail(_))));
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
    fn required_value_is_exactly_one() {
        assert!(required(Some(OsStr::new("1"))));
        for v in ["", "0", " 1", "1 ", "01", "true", "TRUE", "yes", "on"] {
            assert!(!required(Some(OsStr::new(v))), "{v:?}");
        }
        assert!(!required(None));
    }

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
        assert_eq!(version_line(b"", b""), "");
    }

    #[test]
    fn output_is_read_up_to_the_limit() {
        assert_eq!(read_capped(&[7u8; 100][..], 100).ok(), Some(vec![7u8; 100]));
        assert!(matches!(read_capped(&[7u8; 101][..], 100), Err(ReadFail::Limit)));
        assert_eq!(read_capped(&[][..], 0).ok(), Some(Vec::new()));
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

    /// Whether a process with this id still exists, reaped or not. An unreaped child stays visible.
    #[cfg(unix)]
    fn process_exists(pid: u32) -> bool {
        Command::new("sh").arg("-c").arg(format!("kill -0 {pid} 2>/dev/null")).status().unwrap().success()
    }

    #[cfg(unix)]
    #[test]
    fn version_child_is_reaped_after_timeout() {
        let d = Scratch::new("hang");
        let tool = script(d.path(), "tool", "exec sleep 1000");
        let mut seen = Observed::default();
        // The child writes nothing and a zero timeout has passed when the wait starts, so the result
        // does not depend on timing.
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::ZERO };
        let got = run_bounded(Command::new(&tool), &tool, limits, &mut seen);
        assert!(matches!(got, Err(OracleError::Timeout { .. })), "the child must time out");
        let pid = seen.pid.unwrap();
        assert!(!process_exists(pid), "child {pid} was left running or unreaped");
    }

    #[test]
    fn timeout_without_a_deadline_is_an_error() {
        let d = Scratch::new("no-deadline");
        let tool = d.path().join("craft-testkit-no-such-oracle-binary");
        let mut seen = Observed::default();
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::MAX };
        let got = run_bounded(Command::new(&tool), &tool, limits, &mut seen);
        assert!(matches!(got, Err(OracleError::InvalidTimeout { .. })), "{got:?}");
        assert_eq!(seen.pid, None, "the program must not start");
    }

    #[cfg(unix)]
    #[test]
    fn version_child_is_reaped_after_output_limit() {
        let d = Scratch::new("flood-reap");
        let tool = script(d.path(), "tool", "exec yes");
        let mut seen = Observed::default();
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::from_secs(60) };
        let got = run_bounded(Command::new(&tool), &tool, limits, &mut seen);
        assert!(matches!(got, Err(OracleError::OutputLimit { .. })));
        let pid = seen.pid.unwrap();
        assert!(!process_exists(pid), "child {pid} was left running or unreaped");
    }

    /// A process the child started in the background keeps standard error open. Killing the child's
    /// process group ends it, so both pipe readers finish.
    #[cfg(unix)]
    #[test]
    fn descendants_holding_the_pipes_are_killed_with_the_child() {
        let d = Scratch::new("descendant");
        let tool = script(d.path(), "tool", "sleep 1000 &\nexec yes");
        let mut seen = Observed::default();
        let limits = ProbeLimits { max_output_bytes: 1024, timeout: Duration::from_secs(60) };
        let got = run_bounded(Command::new(&tool), &tool, limits, &mut seen);
        assert!(matches!(got, Err(OracleError::OutputLimit { .. })), "{got:?}");
        assert!(seen.pipes_closed, "a pipe reader was left blocked on a pipe a descendant holds");
    }
}
