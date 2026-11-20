//! Finding and running a package's binaries from its integration tests.
//!
//! [`bin_path!`](crate::bin_path) expands in the calling test and looks the binary up in this
//! order:
//!
//! 1. `NEXTEST_BIN_EXE_<name>` with each `-` of the name written as `_` (nextest 0.9.113 and later),
//! 2. `NEXTEST_BIN_EXE_<name>` with the name as written,
//! 3. `CARGO_BIN_EXE_<name>` read when the test runs (Cargo 1.94 and later, nextest 0.9.130 and
//!    later),
//! 4. `CARGO_BIN_EXE_<name>` as Cargo set it when it compiled the test.
//!
//! A value counts only when it names an absolute path to an existing regular file. An empty,
//! relative or missing path, or one that names a folder, is passed over, and the next variable is
//! read. When a test runs from an extracted nextest archive, nextest sets the first two variables
//! to the extracted path, so the compile-time path of the build machine is not used. When no
//! variable names a file, the macro fails the test with a message that lists every variable it
//! read and why each value was passed over, before anything is started. The `.exe` suffix is never
//! added or removed: Cargo and nextest set the whole file name.
//!
//! [`resolve_binary`] does the same lookup and returns a [`BinPathError`] instead of failing the
//! test, and [`resolve_binary_in`] reads the variables through a function the caller passes.
//!
//! [`run`] starts a program with stdin connected to nothing and returns its exit status, stdout
//! and stderr. It returns an error when either stream passes `max_output` bytes, after it has
//! stopped the program and waited for it. [`DEFAULT_MAX_OUTPUT`] is 1 MiB for each stream, a value
//! of this crate. `run` has no time limit: a program that never exits and stays under the limit
//! keeps `run` waiting.

use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};
use std::sync::mpsc;
use std::thread;

/// The default for `max_output` in [`run`]: 1 MiB for each of stdout and stderr.
pub const DEFAULT_MAX_OUTPUT: usize = 1 << 20;

/// Returns the path of the package's binary `$name`, or fails the test.
///
/// `$name` is the binary target's name exactly as `Cargo.toml` gives it, hyphens included
/// (`bin_path!("soundcraft-cli")`). The lookup order is described in the [`bin`](crate::bin)
/// module. The compile-time `CARGO_BIN_EXE_<name>` is read with `option_env!`, so a test compiled
/// without it still builds and reports the variable as not set.
///
/// # Panics
///
/// Fails the calling test when no variable names an existing file. The message names the binary,
/// every variable read and why each value was passed over. [`resolve_binary`] returns the same
/// information as an error.
#[macro_export]
macro_rules! bin_path {
    ($name:literal) => {{
        // Failing the test is this macro's purpose; it expands in the calling test.
        #[allow(clippy::panic)]
        let path = match $crate::bin::resolve_binary($name, ::core::option_env!(::core::concat!("CARGO_BIN_EXE_", $name))) {
            ::core::result::Result::Ok(path) => path,
            ::core::result::Result::Err(error) => ::core::panic!("{error}"),
        };
        path
    }};
}

/// Where a candidate path for a binary comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Source {
    /// `NEXTEST_BIN_EXE_<name>` with each `-` written as `_`.
    NextestUnderscore,
    /// `NEXTEST_BIN_EXE_<name>` with the name as written.
    NextestExact,
    /// `CARGO_BIN_EXE_<name>` read when the test runs.
    CargoRuntime,
    /// `CARGO_BIN_EXE_<name>` as Cargo set it when it compiled the test.
    CargoCompileTime,
}

/// Why a candidate was passed over.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Rejection {
    /// The variable is not set.
    NotSet,
    /// The variable is set to an empty value.
    Empty,
    /// The value is a relative path.
    Relative,
    /// Nothing exists at the path.
    Missing,
    /// The path names something other than a regular file, such as a folder.
    NotAFile,
    /// The path could not be inspected.
    Inaccessible(io::ErrorKind),
}

impl fmt::Display for Rejection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotSet => f.write_str("not set"),
            Self::Empty => f.write_str("set to an empty value"),
            Self::Relative => f.write_str("a relative path"),
            Self::Missing => f.write_str("does not exist"),
            Self::NotAFile => f.write_str("not a regular file"),
            Self::Inaccessible(kind) => write!(f, "could not be inspected ({kind})"),
        }
    }
}

/// One variable that was read, its value and why it was passed over.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Which lookup step the variable belongs to.
    pub source: Source,
    /// The variable's name.
    pub variable: String,
    /// The variable's value, when it is set.
    pub value: Option<OsString>,
    /// Why the value was passed over.
    pub rejection: Rejection,
}

impl fmt::Display for Candidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.variable)?;
        if self.source == Source::CargoCompileTime {
            f.write_str(" (compile time)")?;
        }
        match &self.value {
            Some(value) if !value.is_empty() => write!(f, ": {} ({})", Path::new(value).display(), self.rejection),
            _ => write!(f, ": {}", self.rejection),
        }
    }
}

/// The error of [`resolve_binary`] and [`resolve_binary_in`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BinPathError {
    /// The binary name is empty or holds `=` or a NUL character, so no variable can carry it.
    InvalidName(String),
    /// No variable names an existing file. `candidates` lists every variable read, in order.
    NotFound {
        /// The binary name the caller passed.
        binary: String,
        /// Every variable read, with its value and why it was passed over.
        candidates: Vec<Candidate>,
    },
}

impl fmt::Display for BinPathError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidName(name) => write!(f, "{name:?} is not a binary name that an environment variable can carry"),
            Self::NotFound { binary, candidates } => {
                write!(f, "no executable found for the binary `{binary}`; the variables read were:")?;
                for candidate in candidates {
                    write!(f, "\n  {candidate}")?;
                }
                Ok(())
            }
        }
    }
}

impl Error for BinPathError {}

/// Looks up the executable of the package's binary `binary` in the current environment.
///
/// `compile_time` is the value of `CARGO_BIN_EXE_<binary>` when the calling test was compiled,
/// which only the caller can read; [`bin_path!`](crate::bin_path) passes it. The environment is
/// read and never changed.
pub fn resolve_binary(binary: &str, compile_time: Option<&str>) -> Result<PathBuf, BinPathError> {
    resolve_binary_in(binary, compile_time, |name| std::env::var_os(name))
}

/// [`resolve_binary`] with the variables read through `lookup`, which takes a variable name and
/// returns its value.
pub fn resolve_binary_in(
    binary: &str,
    compile_time: Option<&str>,
    mut lookup: impl FnMut(&str) -> Option<OsString>,
) -> Result<PathBuf, BinPathError> {
    if binary.is_empty() || binary.contains(['=', '\0']) {
        return Err(BinPathError::InvalidName(binary.to_owned()));
    }
    let cargo = format!("CARGO_BIN_EXE_{binary}");
    let steps = [
        (Source::NextestUnderscore, format!("NEXTEST_BIN_EXE_{}", binary.replace('-', "_"))),
        (Source::NextestExact, format!("NEXTEST_BIN_EXE_{binary}")),
        (Source::CargoRuntime, cargo.clone()),
        (Source::CargoCompileTime, cargo),
    ];
    let mut candidates = Vec::with_capacity(steps.len());
    for (source, variable) in steps {
        // A name without `-` gives the same nextest variable twice; read it once.
        if source == Source::NextestExact && !binary.contains('-') {
            continue;
        }
        let value = match source {
            Source::CargoCompileTime => compile_time.map(OsString::from),
            _ => lookup(&variable),
        };
        match check(value.as_deref()) {
            Ok(()) => {
                if let Some(value) = value {
                    return Ok(PathBuf::from(value));
                }
            }
            Err(rejection) => candidates.push(Candidate { source, variable, value, rejection }),
        }
    }
    Err(BinPathError::NotFound { binary: binary.to_owned(), candidates })
}

fn check(value: Option<&OsStr>) -> Result<(), Rejection> {
    let value = value.ok_or(Rejection::NotSet)?;
    if value.is_empty() {
        return Err(Rejection::Empty);
    }
    let path = Path::new(value);
    if !path.is_absolute() {
        return Err(Rejection::Relative);
    }
    match std::fs::metadata(path) {
        Ok(metadata) if metadata.is_file() => Ok(()),
        Ok(_) => Err(Rejection::NotAFile),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Err(Rejection::Missing),
        Err(error) => Err(Rejection::Inaccessible(error.kind())),
    }
}

/// One of a child's output streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stream {
    /// The child's standard output.
    Stdout,
    /// The child's standard error.
    Stderr,
}

impl fmt::Display for Stream {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Stdout => "stdout",
            Self::Stderr => "stderr",
        })
    }
}

/// The error inside the [`io::Error`] that [`run`] returns when a stream passes its limit. Read it
/// with [`io::Error::get_ref`] and `downcast_ref`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutputLimitExceeded {
    /// The stream that passed the limit.
    pub stream: Stream,
    /// The limit in bytes.
    pub limit: usize,
}

impl fmt::Display for OutputLimitExceeded {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "the child wrote more than {} bytes to {}", self.limit, self.stream)
    }
}

impl Error for OutputLimitExceeded {}

/// Runs the program at `path` with `args` and returns its exit status, stdout and stderr.
///
/// The child inherits the test's environment with each `(name, value)` pair of `env` set on top;
/// a later pair for the same name replaces an earlier one. The test's own environment is not
/// changed. stdin is connected to nothing, so a child that reads it sees the end at once.
///
/// stdout and stderr are read at the same time, each into at most `max_output` bytes
/// ([`DEFAULT_MAX_OUTPUT`] is the usual value). A non-zero exit status is returned in the
/// [`Output`], not as an error.
///
/// # Errors
///
/// - The program could not be started; the error keeps the kind of the underlying error and names
///   `path`.
/// - stdout or stderr passed `max_output` bytes: the error's kind is [`io::ErrorKind::Other`] and
///   it holds an [`OutputLimitExceeded`]. The child has been stopped and waited for.
/// - Reading a stream or waiting for the child failed. The child has been stopped and waited for.
///
/// `run` has no time limit. A child that never exits, or that leaves a process running with its
/// output streams open, keeps `run` waiting.
pub fn run(path: &Path, args: &[OsString], env: &[(OsString, OsString)], max_output: usize) -> io::Result<Output> {
    let mut child = Command::new(path)
        .args(args)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|error| io::Error::new(error.kind(), format!("could not start {}: {error}", path.display())))?;
    let (Some(stdout), Some(stderr)) = (child.stdout.take(), child.stderr.take()) else {
        stop(&mut child);
        return Err(io::Error::other("the child's output pipes were not opened"));
    };
    let (sender, receiver) = mpsc::channel();
    let readers =
        spawn_reader(Stream::Stdout, stdout, max_output, sender.clone()).and_then(|()| spawn_reader(Stream::Stderr, stderr, max_output, sender));
    if let Err(error) = readers {
        stop(&mut child);
        return Err(error);
    }
    let mut out = Vec::new();
    let mut err = Vec::new();
    for _ in 0..2 {
        let Ok((stream, result)) = receiver.recv() else {
            stop(&mut child);
            return Err(io::Error::other("a reader of the child's output stopped without a result"));
        };
        match result {
            Ok(Some(bytes)) => match stream {
                Stream::Stdout => out = bytes,
                Stream::Stderr => err = bytes,
            },
            Ok(None) => {
                stop(&mut child);
                return Err(io::Error::other(OutputLimitExceeded { stream, limit: max_output }));
            }
            Err(error) => {
                stop(&mut child);
                return Err(error);
            }
        }
    }
    let status = child.wait()?;
    Ok(Output { status, stdout: out, stderr: err })
}

type ReadResult = (Stream, io::Result<Option<Vec<u8>>>);

fn spawn_reader(stream: Stream, pipe: impl Read + Send + 'static, max_output: usize, sender: mpsc::Sender<ReadResult>) -> io::Result<()> {
    thread::Builder::new().name(format!("craft-testkit {stream}")).spawn(move || {
        // The receiver is gone only when `run` has already returned.
        let _ = sender.send((stream, read_capped(pipe, max_output)));
    })?;
    Ok(())
}

/// Reads `pipe` to its end. Returns `None` as soon as it holds more than `max_output` bytes.
fn read_capped(pipe: impl Read, max_output: usize) -> io::Result<Option<Vec<u8>>> {
    let limit = u64::try_from(max_output).unwrap_or(u64::MAX).saturating_add(1);
    let mut bytes = Vec::new();
    pipe.take(limit).read_to_end(&mut bytes)?;
    Ok((bytes.len() <= max_output).then_some(bytes))
}

/// Stops the child and waits for it, so that no process is left behind.
fn stop(child: &mut Child) {
    // An error here means the child has already exited.
    let _ = child.kill();
    let _ = child.wait();
}
