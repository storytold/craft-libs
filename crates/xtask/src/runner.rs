//! The gate's step runner.
//!
//! An app builds its step list (`fmt`, `clippy`, `test` and its own checks) and passes it to
//! [`run_steps`], which runs the steps in order, stops at the first failure and prints a summary:
//!
//! ```text
//! CI summary:
//!   ok    fmt
//!   FAIL  clippy: cargo clippy --workspace --all-targets -- -D warnings: exited with exit status: 101
//!   skip  test
//! ```
//!
//! The workspace root comes from the caller ([`root`] takes the app's own xtask manifest directory),
//! and the test commands that [`nextest_or_test`] and [`doctest`] build exclude the running xtask
//! package, so that a test build never has to replace the `xtask` executable that is running it.
//! Child processes inherit stdout and stderr; their output is passed through as bytes and never
//! decoded or stored.

use std::ffi::{OsStr, OsString};
use std::fmt;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// How often [`run_cancelable`] checks a running child for exit or cancellation.
const CHILD_POLL: Duration = Duration::from_millis(50);

/// Why a run did not pass.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The caller passed a configuration this crate cannot use (a root without `Cargo.toml`, an
    /// invalid package name, a slot count outside 1 to 64).
    Config(String),
    /// A step returned an error. Later steps did not run.
    Step {
        /// The step's name.
        step: String,
        /// The error the step returned.
        cause: String,
    },
    /// The run was canceled, at the named step or before the first one.
    Canceled {
        /// The step that was canceled, or `None` when the run was canceled while waiting for a lock.
        step: Option<String>,
    },
    /// The lock could not be set up or taken.
    Lock(String),
    /// The gate passed but its report could not be written.
    Report(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::Config(m) => f.write_str(m),
            Error::Step { step, cause } => write!(f, "step `{step}` failed: {cause}"),
            Error::Canceled { step: Some(step) } => write!(f, "canceled at step `{step}`"),
            Error::Canceled { step: None } => f.write_str("canceled before the first step"),
            Error::Lock(m) => write!(f, "lock: {m}"),
            Error::Report(m) => write!(f, "report: {m}"),
        }
    }
}

impl std::error::Error for Error {}

impl From<Error> for String {
    fn from(e: Error) -> Self {
        e.to_string()
    }
}

/// A cancellation flag shared between the run and whoever stops it (a Ctrl-C handler, a test).
#[derive(Debug, Clone, Default)]
pub struct Cancel(Arc<AtomicBool>);

impl Cancel {
    /// A flag that is not set.
    pub fn new() -> Self {
        Self::default()
    }

    /// Sets the flag. Every clone sees it.
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    /// Whether [`Cancel::cancel`] has been called on this flag or a clone of it.
    pub fn is_canceled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// What a running step records about itself.
#[derive(Debug, Default)]
pub struct StepLog {
    command: Option<String>,
}

impl StepLog {
    /// Records the command the step runs, for the report, and returns `label` so that it can be
    /// passed on to [`run`].
    pub fn command<'s>(&mut self, label: &'s str) -> &'s str {
        self.command = Some(label.to_owned());
        label
    }
}

/// A step's body. It returns the error to print in the summary.
pub type StepFn<'a> = Box<dyn FnMut(&mut StepLog) -> Result<(), String> + 'a>;

/// One step of a gate run.
pub struct Step<'a> {
    /// The name shown in the heading, the summary and the report.
    pub name: String,
    /// The command shown in the report when the step does not record one itself.
    pub command: Option<String>,
    /// The step's body.
    pub run: StepFn<'a>,
}

impl<'a> Step<'a> {
    /// A step named `name` that runs `run`.
    pub fn new(name: impl Into<String>, run: impl FnMut(&mut StepLog) -> Result<(), String> + 'a) -> Self {
        Self { name: name.into(), command: None, run: Box::new(run) }
    }

    /// Sets the command shown in the report until the step records another one.
    pub fn with_command(mut self, label: impl Into<String>) -> Self {
        self.command = Some(label.into());
        self
    }
}

impl fmt::Debug for Step<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Step").field("name", &self.name).field("command", &self.command).finish_non_exhaustive()
    }
}

/// How a step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// The step passed.
    Ok,
    /// The step returned an error.
    Fail,
    /// The run was canceled before or while this step ran.
    Canceled,
    /// The step did not run because an earlier step failed or was canceled.
    Skip,
}

impl Status {
    /// The status as the summary and the report write it: `ok`, `fail`, `canceled` or `skip`.
    pub fn as_str(self) -> &'static str {
        match self {
            Status::Ok => "ok",
            Status::Fail => "fail",
            Status::Canceled => "canceled",
            Status::Skip => "skip",
        }
    }
}

/// The result of one step.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StepRecord {
    /// The step's name.
    pub step: String,
    /// The command the step recorded, if any.
    pub command: Option<String>,
    /// How the step ended.
    pub status: Status,
    /// How long the step ran; `None` for a step that did not start.
    pub duration: Option<Duration>,
}

/// Options for [`run_steps`].
#[non_exhaustive]
#[derive(Debug, Clone)]
pub struct RunOptions {
    /// The word in each step heading (`=== ci: fmt ===`) and, in capitals, the summary (`CI summary:`).
    pub title: String,
    /// Stops the run when set: the current child is ended and no later step starts.
    pub cancel: Option<Cancel>,
    /// A slot to hold for the whole run; `None` runs without a lock.
    #[cfg(feature = "lock")]
    pub lock: Option<crate::lock::LockConfig>,
    /// Where to write the per-step JSON report; `None` writes none.
    #[cfg(feature = "report")]
    pub report: Option<PathBuf>,
}

impl Default for RunOptions {
    fn default() -> Self {
        Self {
            title: "ci".to_owned(),
            cancel: None,
            #[cfg(feature = "lock")]
            lock: None,
            #[cfg(feature = "report")]
            report: None,
        }
    }
}

impl RunOptions {
    /// Sets the heading and summary word.
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    /// Sets the cancellation flag.
    pub fn with_cancel(mut self, cancel: Cancel) -> Self {
        self.cancel = Some(cancel);
        self
    }

    /// Holds a slot of `lock` for the whole run.
    #[cfg(feature = "lock")]
    pub fn with_lock(mut self, lock: crate::lock::LockConfig) -> Self {
        self.lock = Some(lock);
        self
    }

    /// Writes the per-step JSON report to `path` after the run, whether it passed or not. A run that
    /// does not take its lock writes no report.
    #[cfg(feature = "report")]
    pub fn with_report(mut self, path: impl Into<PathBuf>) -> Self {
        self.report = Some(path.into());
        self
    }

    fn is_canceled(&self) -> bool {
        self.cancel.as_ref().is_some_and(Cancel::is_canceled)
    }
}

/// Runs `steps` in order and stops at the first failure.
///
/// Each step's heading goes to stderr and the summary to stdout. With a lock configured, a slot is
/// taken before the first step and released when this function returns, whatever the outcome. With a
/// report configured, it is written after the run, passed or not, while the slot is still held. When
/// the lock is not taken (a configuration or lock error, or a cancel while waiting), no step runs and
/// no report is written. A report that cannot be written turns a passed run into [`Error::Report`]; a
/// failed run keeps its own error and the report error is printed. An empty list passes.
pub fn run_steps(steps: &mut [Step<'_>], options: &RunOptions) -> Result<Vec<StepRecord>, Error> {
    run_steps_to(steps, options, &mut io::stdout(), &mut io::stderr())
}

/// [`run_steps`] with the summary written to `out` and the headings to `err`.
pub fn run_steps_to(steps: &mut [Step<'_>], options: &RunOptions, out: &mut dyn Write, err: &mut dyn Write) -> Result<Vec<StepRecord>, Error> {
    // The slot stays held until the report is written, so a later run's report replaces this one.
    // A run that did not take the lock writes no report, so it never replaces the report of a run
    // that holds the slot.
    let held = take_lock(options);
    let (records, result) = match &held {
        Ok(_) => {
            let (records, result) = execute(steps, options, err);
            let result = finish_report(&records, result, options, err);
            (records, result)
        }
        Err(e) => (not_started(steps, Status::Skip), Err(e.clone())),
    };
    print_summary(&records, &result, options, out);
    drop(held);
    result.map(|()| records)
}

#[cfg(feature = "lock")]
fn take_lock(options: &RunOptions) -> Result<Option<crate::lock::Held>, Error> {
    options.lock.as_ref().map(|config| crate::lock::acquire(config, options.cancel.as_ref())).transpose()
}

#[cfg(not(feature = "lock"))]
fn take_lock(_options: &RunOptions) -> Result<(), Error> {
    Ok(())
}

#[cfg(feature = "report")]
fn finish_report(records: &[StepRecord], result: Result<(), Error>, options: &RunOptions, err: &mut dyn Write) -> Result<(), Error> {
    let Some(path) = &options.report else {
        return result;
    };
    match (crate::report::write(path, records), result) {
        (Ok(()), result) => result,
        (Err(e), Ok(())) => Err(Error::Report(format!("{}: {e}", path.display()))),
        (Err(e), Err(gate)) => {
            let _ = writeln!(err, "report: {}: {e}", path.display());
            Err(gate)
        }
    }
}

#[cfg(not(feature = "report"))]
fn finish_report(_records: &[StepRecord], result: Result<(), Error>, _options: &RunOptions, _err: &mut dyn Write) -> Result<(), Error> {
    result
}

fn not_started(steps: &[Step<'_>], status: Status) -> Vec<StepRecord> {
    steps.iter().map(|s| StepRecord { step: s.name.clone(), command: s.command.clone(), status, duration: None }).collect()
}

fn execute(steps: &mut [Step<'_>], options: &RunOptions, err: &mut dyn Write) -> (Vec<StepRecord>, Result<(), Error>) {
    let mut records = Vec::with_capacity(steps.len());
    let mut result = Ok(());
    for step in steps.iter_mut() {
        if result.is_err() {
            records.push(StepRecord { step: step.name.clone(), command: step.command.clone(), status: Status::Skip, duration: None });
            continue;
        }
        if options.is_canceled() {
            records.push(StepRecord { step: step.name.clone(), command: step.command.clone(), status: Status::Canceled, duration: None });
            result = Err(Error::Canceled { step: Some(step.name.clone()) });
            continue;
        }
        let (record, outcome) = run_one(step, options, err);
        records.push(record);
        result = outcome;
    }
    (records, result)
}

fn run_one(step: &mut Step<'_>, options: &RunOptions, err: &mut dyn Write) -> (StepRecord, Result<(), Error>) {
    let _ = writeln!(err, "\n=== {}: {} ===", options.title, step.name);
    let mut log = StepLog { command: step.command.clone() };
    let started = Instant::now();
    let outcome = (step.run)(&mut log);
    let duration = Some(started.elapsed());
    let (status, result) = match outcome {
        _ if options.is_canceled() => (Status::Canceled, Err(Error::Canceled { step: Some(step.name.clone()) })),
        Ok(()) => (Status::Ok, Ok(())),
        Err(cause) => (Status::Fail, Err(Error::Step { step: step.name.clone(), cause })),
    };
    (StepRecord { step: step.name.clone(), command: log.command, status, duration }, result)
}

fn print_summary(records: &[StepRecord], result: &Result<(), Error>, options: &RunOptions, out: &mut dyn Write) {
    let title = options.title.to_uppercase();
    if result.is_ok() {
        let names: Vec<&str> = records.iter().map(|r| r.step.as_str()).collect();
        let _ = if names.is_empty() {
            writeln!(out, "\n{title} summary: all 0 steps passed")
        } else {
            writeln!(out, "\n{title} summary: all {} steps passed ({})", names.len(), names.join(", "))
        };
        return;
    }
    let _ = writeln!(out, "\n{title} summary:");
    if let Err(e @ (Error::Config(_) | Error::Lock(_) | Error::Report(_) | Error::Canceled { step: None })) = result {
        let _ = writeln!(out, "  FAIL  {e}");
    }
    for r in records {
        let _ = match (r.status, result) {
            (Status::Ok, _) => writeln!(out, "  ok    {}", r.step),
            (Status::Fail, Err(Error::Step { cause, .. })) => writeln!(out, "  FAIL  {}: {cause}", r.step),
            (Status::Fail, _) => writeln!(out, "  FAIL  {}", r.step),
            (Status::Canceled, _) => writeln!(out, "  cancel {}", r.step),
            (Status::Skip, _) => writeln!(out, "  skip  {}", r.step),
        };
    }
}

/// The workspace root of an app: the parent of its xtask manifest directory.
///
/// Pass the app's own `Path::new(env!("CARGO_MANIFEST_DIR"))`. The parent must hold a
/// `Cargo.toml`; a layout where it does not passes its root to [`cargo`] directly instead.
pub fn root(xtask_manifest_dir: &Path) -> Result<PathBuf, Error> {
    let parent = xtask_manifest_dir
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or_else(|| Error::Config(format!("{}: the xtask manifest directory has no parent directory", xtask_manifest_dir.display())))?;
    if !parent.join("Cargo.toml").is_file() {
        return Err(Error::Config(format!("{}: no Cargo.toml here, so this is not the workspace root", parent.display())));
    }
    Ok(parent.to_path_buf())
}

/// `cargo` (the `CARGO` the xtask was started with, or `cargo` from `PATH`) with `root` as its
/// working directory.
pub fn cargo(root: &Path) -> Command {
    let program = std::env::var_os("CARGO").filter(|p| !p.is_empty()).unwrap_or_else(|| OsString::from("cargo"));
    let mut c = Command::new(program);
    c.current_dir(root);
    c
}

/// Prints `$ label` to stderr and runs `cmd` with inherited stdin, stdout and stderr.
///
/// Returns an error when the command cannot be started or exits with a failure status.
pub fn run(mut cmd: Command, label: &str) -> Result<(), String> {
    crate::note(format_args!("$ {label}"));
    let status = cmd.status().map_err(|e| spawn_error(label, &e))?;
    exit_result(label, status)
}

/// [`run`] that ends the child, with the processes it started, and waits for it to exit when
/// `cancel` is set while it runs.
///
/// On Unix the child runs in a process group of its own, so a terminal's Ctrl-C reaches the xtask
/// and not the child: the app's Ctrl-C handler sets `cancel`, and this function ends the group with
/// `kill -s KILL`. On Windows it ends the process tree with `taskkill /T /F`. When that tool cannot
/// be run, only the child itself is ended. `taskkill` finds the tree through the running child, so on
/// Windows the processes a child started keep running when the child has already exited.
///
/// On Unix the child's process group is not the terminal's foreground group. A child that reads from
/// the terminal is stopped by `SIGTTIN`, and this function waits until `cancel` is set. Run a command
/// that reads from the terminal with [`run`].
pub fn run_cancelable(mut cmd: Command, label: &str, cancel: &Cancel) -> Result<(), String> {
    crate::note(format_args!("$ {label}"));
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
    let mut child = spawn(&mut cmd, label)?;
    wait_cancelable(&mut child, label, cancel)
}

/// Waits for `child`, ending its group or tree when `cancel` is set. The flag is checked before and
/// after each look at the child's status, so a child that exits before or while `cancel` is set still
/// has its group or tree ended, and the run counts as canceled.
fn wait_cancelable(child: &mut Child, label: &str, cancel: &Cancel) -> Result<(), String> {
    wait_cancelable_with(child, label, cancel, Child::try_wait)
}

/// [`wait_cancelable`] that reads the child's status with `status`.
fn wait_cancelable_with(
    child: &mut Child,
    label: &str,
    cancel: &Cancel,
    mut status: impl FnMut(&mut Child) -> io::Result<Option<ExitStatus>>,
) -> Result<(), String> {
    loop {
        // The status is not read once `cancel` is set, so a child that is still running is not reaped
        // before its tree is ended.
        let exited = if cancel.is_canceled() { Ok(None) } else { status(child) };
        if cancel.is_canceled() {
            let killed = end_tree(child);
            let reaped = child.wait();
            return match (killed, reaped) {
                (_, Ok(_)) => Err(format!("{label}: canceled")),
                (Err(k), Err(w)) => Err(format!("{label}: canceled, but the child could not be ended ({k}; {w})")),
                (Ok(()), Err(w)) => Err(format!("{label}: canceled, but waiting for the child failed ({w})")),
            };
        }
        if let Some(status) = exited.map_err(|e| format!("{label}: {e}"))? {
            return exit_result(label, status);
        }
        std::thread::sleep(CHILD_POLL);
    }
}

/// Ends `child` and every process in its group (Unix) or tree (Windows), falling back to the child
/// alone when the system tool fails.
fn end_tree(child: &mut Child) -> io::Result<()> {
    let id = child.id().to_string();
    #[cfg(unix)]
    let tree = Command::new("kill").args(["-s", "KILL", "--", &format!("-{id}")]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    #[cfg(windows)]
    let tree = Command::new("taskkill").args(["/T", "/F", "/PID", &id]).stdout(Stdio::null()).stderr(Stdio::null()).status();
    #[cfg(not(any(unix, windows)))]
    let tree: io::Result<ExitStatus> = Err(io::Error::new(io::ErrorKind::Unsupported, id));
    match tree {
        Ok(status) if status.success() => Ok(()),
        _ => child.kill(),
    }
}

/// Starts `cmd` and closes its stdin when the caller piped it, as `Command::status` does in [`run`], so
/// a child that reads stdin to the end can exit.
fn spawn(cmd: &mut Command, label: &str) -> Result<Child, String> {
    let mut child = cmd.spawn().map_err(|e| spawn_error(label, &e))?;
    drop(child.stdin.take());
    Ok(child)
}

fn spawn_error(label: &str, e: &io::Error) -> String {
    format!("{label}: failed to spawn: {e}")
}

fn exit_result(label: &str, status: ExitStatus) -> Result<(), String> {
    if status.success() { Ok(()) } else { Err(format!("{label}: exited with {status}")) }
}

/// Which test runner [`nextest_or_test`] uses.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum TestMode {
    /// `cargo test`. Nothing is probed.
    #[default]
    CargoTest,
    /// `cargo nextest run` when `cargo nextest --version` succeeds, otherwise `cargo test` with a note.
    PreferNextest,
}

/// The workspace test command and its label, with the running xtask package excluded.
///
/// The command is `cargo test --workspace --exclude <xtask_package> <args>` or, with
/// [`TestMode::PreferNextest`] and nextest installed, `cargo nextest run --workspace --exclude
/// <xtask_package> <args>`. `args` holds the app's targets, features and filters. The label names
/// the command that will run, so a fallback shows as `cargo test`.
pub fn nextest_or_test(root: &Path, xtask_package: &str, mode: TestMode, args: &[impl AsRef<OsStr>]) -> Result<(Command, String), Error> {
    select_test(cargo(root), xtask_package, mode, args, || nextest_available(cargo(root)))
}

/// [`nextest_or_test`] with the nextest probe passed in.
fn select_test(
    cargo: Command,
    xtask_package: &str,
    mode: TestMode,
    args: &[impl AsRef<OsStr>],
    probe: impl FnOnce() -> bool,
) -> Result<(Command, String), Error> {
    check_package(xtask_package)?;
    let nextest = match mode {
        TestMode::CargoTest => false,
        TestMode::PreferNextest => {
            let found = probe();
            if !found {
                crate::note(format_args!("note: cargo-nextest is not installed, running cargo test (cargo install cargo-nextest --locked)"));
            }
            found
        }
    };
    Ok(test_command(cargo, xtask_package, if nextest { &["nextest", "run"] } else { &["test"] }, args))
}

/// The workspace doctest command and its label: `cargo test --workspace --exclude <xtask_package>
/// --doc <args>`. nextest does not run doctests.
pub fn doctest(root: &Path, xtask_package: &str, args: &[impl AsRef<OsStr>]) -> Result<(Command, String), Error> {
    check_package(xtask_package)?;
    let doc_args: Vec<&OsStr> = std::iter::once(OsStr::new("--doc")).chain(args.iter().map(AsRef::as_ref)).collect();
    Ok(test_command(cargo(root), xtask_package, &["test"], &doc_args))
}

/// Whether `cargo nextest --version` succeeds for this `cargo` command.
pub fn nextest_available(mut cargo: Command) -> bool {
    cargo.args(["nextest", "--version"]).stdout(Stdio::null()).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
}

fn test_command(mut c: Command, xtask_package: &str, tool: &[&str], args: &[impl AsRef<OsStr>]) -> (Command, String) {
    c.args(tool).args(["--workspace", "--exclude", xtask_package]).args(args);
    let mut label = format!("cargo {} --workspace --exclude {xtask_package}", tool.join(" "));
    for a in args {
        label.push(' ');
        label.push_str(&a.as_ref().to_string_lossy());
    }
    (c, label)
}

/// A Cargo package name: ASCII letters, digits, `-` and `_`, not starting with `-`.
fn check_package(name: &str) -> Result<(), Error> {
    let valid = !name.is_empty() && !name.starts_with('-') && name.chars().all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
    if valid { Ok(()) } else { Err(Error::Config(format!("`{name}` is not a package name to exclude from the test build"))) }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;

    fn args(c: &Command) -> Vec<String> {
        c.get_args().map(|a| a.to_string_lossy().into_owned()).collect()
    }

    fn run_capture(steps: &mut [Step<'_>], options: &RunOptions) -> (Result<Vec<StepRecord>, Error>, String, String) {
        let (mut out, mut err) = (Vec::new(), Vec::new());
        let result = run_steps_to(steps, options, &mut out, &mut err);
        (result, String::from_utf8_lossy(&out).into_owned(), String::from_utf8_lossy(&err).into_owned())
    }

    #[test]
    fn run_steps_success_failure_and_skips() {
        let later_ran = Cell::new(false);
        let mut steps = vec![
            Step::new("fmt", |_| Ok(())),
            Step::new("clippy", |log| Err(format!("{}: exited with 101", log.command("cargo clippy")))),
            Step::new("test", |_| {
                later_ran.set(true);
                Ok(())
            }),
        ];
        let (result, out, err) = run_capture(&mut steps, &RunOptions::default());
        assert_eq!(result, Err(Error::Step { step: "clippy".into(), cause: "cargo clippy: exited with 101".into() }));
        assert!(!later_ran.get(), "a step after the failure ran");
        assert_eq!(out, "\nCI summary:\n  ok    fmt\n  FAIL  clippy: cargo clippy: exited with 101\n  skip  test\n");
        assert!(err.contains("=== ci: fmt ===") && err.contains("=== ci: clippy ===") && !err.contains("=== ci: test ==="), "{err}");

        let mut passing = vec![Step::new("fmt", |_| Ok(())), Step::new("test", |_| Ok(()))];
        let (result, out, _) = run_capture(&mut passing, &RunOptions::default().with_title("gate"));
        let records = result.unwrap_or_default();
        assert_eq!(records.iter().map(|r| r.status).collect::<Vec<_>>(), [Status::Ok, Status::Ok]);
        assert_eq!(out, "\nGATE summary: all 2 steps passed (fmt, test)\n");
    }

    #[test]
    fn run_steps_empty() {
        let (result, out, err) = run_capture(&mut [], &RunOptions::default());
        assert_eq!(result, Ok(Vec::new()));
        assert_eq!(out, "\nCI summary: all 0 steps passed\n");
        assert!(err.is_empty());
    }

    #[test]
    fn run_steps_canceled_runs_nothing_more() {
        let cancel = Cancel::new();
        let ran = Cell::new(0);
        let mut steps = vec![
            Step::new("fmt", |_| {
                ran.set(ran.get() + 1);
                cancel.cancel();
                Err("interrupted".to_owned())
            }),
            Step::new("test", |_| {
                ran.set(ran.get() + 1);
                Ok(())
            }),
        ];
        let options = RunOptions::default().with_cancel(cancel.clone());
        let (result, out, _) = run_capture(&mut steps, &options);
        assert_eq!(result, Err(Error::Canceled { step: Some("fmt".into()) }));
        assert_eq!(ran.get(), 1);
        assert_eq!(out, "\nCI summary:\n  cancel fmt\n  skip  test\n");

        // A last step that passes after the flag was set does not make the run pass.
        let late = Cancel::new();
        let mut last = vec![Step::new("last", |_| {
            late.cancel();
            Ok(())
        })];
        let (result, out, _) = run_capture(&mut last, &RunOptions::default().with_cancel(late.clone()));
        assert_eq!(result, Err(Error::Canceled { step: Some("last".into()) }));
        assert_eq!(out, "\nCI summary:\n  cancel last\n");

        let mut again = vec![Step::new("fmt", |_| Ok(()))];
        let (result, out, _) = run_capture(&mut again, &options);
        assert_eq!(result, Err(Error::Canceled { step: Some("fmt".into()) }));
        assert!(!out.contains("passed"), "{out}");
    }

    #[cfg(all(feature = "lock", feature = "report"))]
    #[test]
    fn a_run_without_its_lock_writes_no_report() {
        use crate::lock::{LockConfig, try_acquire};
        let _guard = crate::spawn_guard();
        let Ok(dir) = tempfile::tempdir() else { panic!("no temporary directory") };
        let locks = dir.path().join("locks");
        let lock = LockConfig::new(&locks, 1, "holder").with_poll(Duration::from_millis(1));
        let Ok(Some(_held)) = try_acquire(&lock) else { panic!("free slot") };

        // A run canceled while another run holds the slot leaves that run's report alone.
        let report = dir.path().join("ci.json");
        let Ok(()) = std::fs::write(&report, b"sentinel") else { panic!("cannot write the sentinel") };
        let cancel = Cancel::new();
        cancel.cancel();
        let ran = Cell::new(false);
        let mut steps = vec![Step::new("test", |_| {
            ran.set(true);
            Ok(())
        })];
        let options = RunOptions::default().with_lock(lock.clone()).with_report(&report).with_cancel(cancel);
        let (result, out, _) = run_capture(&mut steps, &options);
        assert_eq!(result, Err(Error::Canceled { step: None }));
        assert!(!ran.get(), "a step ran without the lock");
        assert_eq!(out, "\nCI summary:\n  FAIL  canceled before the first step\n  skip  test\n");
        assert_eq!(std::fs::read(&report).ok(), Some(b"sentinel".to_vec()));

        // A rejected lock configuration creates no report file.
        let fresh = dir.path().join("rejected.json");
        let options = RunOptions::default().with_lock(LockConfig::new(&locks, 0, "zero")).with_report(&fresh);
        let (result, _, _) = run_capture(&mut [Step::new("test", |_| Ok(()))], &options);
        assert!(matches!(result, Err(Error::Config(_))), "{result:?}");
        assert!(!fresh.exists(), "a report was written without the lock");

        // A lock directory that cannot be created leaves the existing report alone.
        let not_a_dir = dir.path().join("file");
        let Ok(()) = std::fs::write(&not_a_dir, b"") else { panic!("cannot write the file") };
        let options = RunOptions::default().with_lock(LockConfig::new(not_a_dir.join("locks"), 1, "run")).with_report(&report);
        let (result, _, _) = run_capture(&mut [Step::new("test", |_| Ok(()))], &options);
        assert!(matches!(result, Err(Error::Lock(_))), "{result:?}");
        assert_eq!(std::fs::read(&report).ok(), Some(b"sentinel".to_vec()));
    }

    #[test]
    fn root_uses_callers_manifest() {
        let Ok(dir) = tempfile::tempdir() else { panic!("no temporary directory") };
        let app = dir.path().join("app");
        let xtask = app.join("xtask");
        let Ok(()) = std::fs::create_dir_all(&xtask) else { panic!("cannot create the fixture") };
        let Ok(()) = std::fs::write(app.join("Cargo.toml"), "[workspace]\n") else { panic!("cannot write the fixture") };
        assert_eq!(root(&xtask), Ok(app.clone()));
        assert_eq!(cargo(&app).get_current_dir(), Some(app.as_path()));

        // A manifest directory whose parent is not a workspace, and one with no parent at all.
        assert!(matches!(root(&dir.path().join("xtask")), Err(Error::Config(_))));
        assert!(matches!(root(Path::new("xtask")), Err(Error::Config(_))));
        assert!(matches!(root(Path::new("")), Err(Error::Config(_))));
    }

    #[test]
    fn workspace_tests_exclude_running_package() {
        let root = Path::new(".");
        let targets = ["--lib", "--bins", "--tests"];
        let (c, label) = test_command(cargo(root), "designcraft-xtask", &["test"], &targets);
        assert_eq!(args(&c), ["test", "--workspace", "--exclude", "designcraft-xtask", "--lib", "--bins", "--tests"]);
        assert_eq!(label, "cargo test --workspace --exclude designcraft-xtask --lib --bins --tests");

        let (c, label) = test_command(cargo(root), "xtask", &["nextest", "run"], &targets);
        assert_eq!(args(&c), ["nextest", "run", "--workspace", "--exclude", "xtask", "--lib", "--bins", "--tests"]);
        assert_eq!(label, "cargo nextest run --workspace --exclude xtask --lib --bins --tests");

        let Ok((c, label)) = nextest_or_test(root, "xtask", TestMode::CargoTest, &targets) else { panic!("valid package") };
        assert_eq!(args(&c), ["test", "--workspace", "--exclude", "xtask", "--lib", "--bins", "--tests"]);
        assert_eq!(label, "cargo test --workspace --exclude xtask --lib --bins --tests");

        let Ok((c, label)) = doctest(root, "xtask", &["--features", "full"]) else { panic!("valid package") };
        assert_eq!(args(&c), ["test", "--workspace", "--exclude", "xtask", "--doc", "--features", "full"]);
        assert_eq!(label, "cargo test --workspace --exclude xtask --doc --features full");

        for bad in ["", "-p", "--workspace", "two words", "x;y"] {
            assert!(matches!(nextest_or_test(root, bad, TestMode::CargoTest, &targets), Err(Error::Config(_))), "{bad:?}");
            assert!(matches!(doctest(root, bad, &[] as &[&str]), Err(Error::Config(_))), "{bad:?}");
        }
    }

    #[test]
    fn missing_tools_and_output_bytes() {
        let _guard = crate::spawn_guard();
        let missing = "craft-xtask-test-no-such-program";
        let Err(e) = run(Command::new(missing), "missing tool") else { panic!("a missing program ran") };
        assert!(e.starts_with("missing tool: failed to spawn: "), "{e}");
        assert!(!nextest_available(Command::new(missing)));

        // PreferNextest without nextest falls back to cargo test; CargoTest never probes.
        let targets = ["--lib"];
        let fallback = select_test(cargo(Path::new(".")), "xtask", TestMode::PreferNextest, &targets, || nextest_available(Command::new(missing)));
        let Ok((c, label)) = fallback else { panic!("valid package") };
        assert_eq!(args(&c), ["test", "--workspace", "--exclude", "xtask", "--lib"]);
        assert_eq!(label, "cargo test --workspace --exclude xtask --lib");
        let Ok((c, label)) = select_test(cargo(Path::new(".")), "xtask", TestMode::PreferNextest, &targets, || true) else { panic!("valid package") };
        assert_eq!(args(&c), ["nextest", "run", "--workspace", "--exclude", "xtask", "--lib"]);
        assert_eq!(label, "cargo nextest run --workspace --exclude xtask --lib");
        let probed = Cell::new(false);
        let Ok((c, _)) = select_test(cargo(Path::new(".")), "xtask", TestMode::CargoTest, &targets, || {
            probed.set(true);
            true
        }) else {
            panic!("valid package")
        };
        assert_eq!(args(&c)[0], "test");
        assert!(!probed.get(), "CargoTest probed for nextest");

        // A child's output that is not UTF-8 reaches the stream it was given unchanged.
        let (mut reader, writer) = io::pipe().expect("pipe");
        let mut child = test_child("runner::tests::bytes_child", BYTES_CHILD);
        child.stdout(writer).stderr(Stdio::null());
        assert_eq!(run(child, "bytes"), Ok(()));
        let mut got = Vec::new();
        io::Read::read_to_end(&mut reader, &mut got).expect("child output");
        assert!(got.windows(RAW_BYTES.len()).any(|w| w == RAW_BYTES), "{got:?}");

        // The test binary itself, given a flag it does not know, exits with a failure status.
        let Ok(exe) = std::env::current_exe() else { panic!("no test executable") };
        let mut child = Command::new(&exe);
        child.arg("--craft-xtask-unknown-flag").stdout(Stdio::null()).stderr(Stdio::null());
        let Err(e) = run(child, "child") else { panic!("an unknown flag passed") };
        assert!(e.starts_with("child: exited with "), "{e}");

        let mut steps = vec![Step::new("cargo", |log| run(Command::new(missing), log.command("cargo"))), Step::new("after", |_| Ok(()))];
        let (result, out, _) = run_capture(&mut steps, &RunOptions::default());
        assert!(matches!(&result, Err(Error::Step { step, cause }) if step == "cargo" && cause.contains("failed to spawn")), "{result:?}");
        assert!(out.contains("  FAIL  cargo: cargo: failed to spawn: ") && out.ends_with("\n  skip  after\n"), "{out}");
    }

    const TREE_CHILD: &str = "CRAFT_XTASK_TREE_CHILD";
    const BLOCKED_CHILD: &str = "CRAFT_XTASK_BLOCKED_CHILD";
    const BYTES_CHILD: &str = "CRAFT_XTASK_BYTES_CHILD";
    const STDIN_CHILD: &str = "CRAFT_XTASK_STDIN_CHILD";
    const CLOSED_STDERR_CHILD: &str = "CRAFT_XTASK_CLOSED_STDERR_CHILD";
    const RAW_BYTES: &[u8] = b"\xff\xfe\x00 not UTF-8 \x80\xc3\n";

    /// This test binary, run as a child that runs only the ignored test `name`.
    fn test_child(name: &str, env: &str) -> Command {
        let exe = std::env::current_exe().expect("test executable");
        let mut c = Command::new(exe);
        c.args(["--ignored", "--exact", name, "--nocapture"]).env(env, "1").stdin(Stdio::null());
        c
    }

    /// Reads a pipe whose writing end this process also holds, so the read never ends.
    fn block_forever() {
        let (mut reader, _writer) = io::pipe().expect("pipe");
        let _ = io::Read::read_to_end(&mut reader, &mut Vec::new());
    }

    #[test]
    fn run_cancelable_ends_the_process_tree() {
        use io::BufRead;
        let _guard = crate::spawn_guard();
        let (reader, writer) = io::pipe().expect("pipe");
        let mut child = test_child("runner::tests::tree_child", TREE_CHILD);
        child.stdout(writer).stderr(Stdio::null());
        let cancel = Cancel::new();
        let runner = {
            let cancel = cancel.clone();
            std::thread::spawn(move || run_cancelable(child, "tree", &cancel))
        };
        // The child prints `ready` once it has started its own child, which shares the pipe.
        let mut lines = io::BufReader::new(reader);
        let mut line = String::new();
        while !line.trim_end().ends_with("ready") {
            line.clear();
            assert_ne!(lines.read_line(&mut line).expect("child output"), 0, "the child ended before it was ready");
        }
        cancel.cancel();
        assert_eq!(runner.join().expect("runner thread"), Err("tree: canceled".to_owned()));
        // The pipe ends only when every process holding it has ended, the child's child included.
        io::Read::read_to_end(&mut lines, &mut Vec::new()).expect("pipe end");
    }

    #[test]
    fn spawn_closes_a_piped_stdin() {
        let _guard = crate::spawn_guard();
        let mut cmd = test_child("runner::tests::stdin_child", STDIN_CHILD);
        cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null());
        // The child reads stdin to the end, so it exits only once the pipe is closed. A failed check
        // drops the child's stdin with it, so the child still exits.
        let mut child = spawn(&mut cmd, "stdin").expect("child");
        assert!(child.stdin.is_none(), "the child's stdin is still open");
        assert!(child.wait().expect("child").success());
    }

    #[test]
    fn a_child_that_exits_after_cancel_counts_as_canceled() {
        let _guard = crate::spawn_guard();
        // The filter matches no test, so the child exits at once with success. Waiting here first
        // stands for a child that exits while `cancel` is already set.
        let mut cmd = test_child("runner::tests::no_such_test", "CRAFT_XTASK_EXITED_CHILD");
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = spawn(&mut cmd, "exited").expect("child");
        assert!(child.wait().expect("child").success());
        let cancel = Cancel::new();
        cancel.cancel();
        assert_eq!(wait_cancelable(&mut child, "exited", &cancel), Err("exited: canceled".to_owned()));
    }

    #[test]
    fn a_child_that_exits_as_cancel_is_set_counts_as_canceled() {
        let _guard = crate::spawn_guard();
        let mut cmd = test_child("runner::tests::no_such_test", "CRAFT_XTASK_EXITED_CHILD");
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = spawn(&mut cmd, "exited").expect("child");
        let cancel = Cancel::new();
        // `cancel` is set while the status is read, after the flag's last check before the read.
        let result = wait_cancelable_with(&mut child, "exited", &cancel, |c| {
            cancel.cancel();
            c.wait().map(Some)
        });
        assert_eq!(result, Err("exited: canceled".to_owned()));
    }

    #[test]
    fn the_status_is_not_read_once_cancel_is_set() {
        let _guard = crate::spawn_guard();
        let mut cmd = test_child("runner::tests::no_such_test", "CRAFT_XTASK_EXITED_CHILD");
        cmd.stdout(Stdio::null()).stderr(Stdio::null());
        let mut child = spawn(&mut cmd, "exited").expect("child");
        let cancel = Cancel::new();
        cancel.cancel();
        let mut reads = 0;
        let result = wait_cancelable_with(&mut child, "exited", &cancel, |c| {
            reads += 1;
            c.try_wait()
        });
        assert_eq!(result, Err("exited: canceled".to_owned()));
        assert_eq!(reads, 0, "the status was read after cancel was set");
    }

    #[cfg(unix)]
    #[test]
    fn a_child_that_exits_as_cancel_is_set_has_its_group_ended() {
        use io::BufRead;
        let _guard = crate::spawn_guard();
        let (reader, writer) = io::pipe().expect("pipe");
        let mut cmd = test_child("runner::tests::tree_child", TREE_CHILD);
        cmd.stdout(writer).stderr(Stdio::null());
        std::os::unix::process::CommandExt::process_group(&mut cmd, 0);
        let mut child = spawn(&mut cmd, "tree").expect("child");
        drop(cmd);
        // The child prints `ready` once it has started its own child, which shares the pipe.
        let mut lines = io::BufReader::new(reader);
        let mut line = String::new();
        while !line.trim_end().ends_with("ready") {
            line.clear();
            assert_ne!(lines.read_line(&mut line).expect("child output"), 0, "the child ended before it was ready");
        }
        let cancel = Cancel::new();
        // The direct child is killed while `cancel` is set, and its own child keeps running.
        let result = wait_cancelable_with(&mut child, "tree", &cancel, |c| {
            cancel.cancel();
            c.kill()?;
            c.wait().map(Some)
        });
        if result != Err("tree: canceled".to_owned()) {
            // A failed check still ends the group, so the test leaves no process behind.
            let _ = end_tree(&mut child);
        }
        assert_eq!(result, Err("tree: canceled".to_owned()));
        // The pipe ends only when every process holding it has ended, the child's child included.
        io::Read::read_to_end(&mut lines, &mut Vec::new()).expect("pipe end");
    }

    #[test]
    fn diagnostics_to_a_closed_stderr_do_not_panic() {
        let _guard = crate::spawn_guard();
        let (reader, writer) = io::pipe().expect("pipe");
        drop(reader);
        let mut child = test_child("runner::tests::closed_stderr_child", CLOSED_STDERR_CHILD);
        child.stdout(Stdio::null()).stderr(writer);
        let status = child.status().expect("child");
        assert!(status.success(), "the child failed with a closed stderr: {status}");
    }

    /// Reads stdin to the end; run by `spawn_closes_a_piped_stdin`.
    #[test]
    #[ignore = "child process of spawn_closes_a_piped_stdin"]
    fn stdin_child() {
        if std::env::var_os(STDIN_CHILD).is_some() {
            io::Read::read_to_end(&mut io::stdin(), &mut Vec::new()).expect("stdin");
        }
    }

    /// Prints every diagnostic to a stderr whose reading end is closed; run by
    /// `diagnostics_to_a_closed_stderr_do_not_panic`.
    #[test]
    #[ignore = "child process of diagnostics_to_a_closed_stderr_do_not_panic"]
    fn closed_stderr_child() {
        if std::env::var_os(CLOSED_STDERR_CHILD).is_none() {
            return;
        }
        let missing = "craft-xtask-test-no-such-program";
        assert!(run(Command::new(missing), "missing").is_err_and(|e| e.contains("failed to spawn")));
        assert!(run_cancelable(Command::new(missing), "missing", &Cancel::new()).is_err_and(|e| e.contains("failed to spawn")));
        assert!(select_test(cargo(Path::new(".")), "xtask", TestMode::PreferNextest, &["--lib"], || false).is_ok());
        #[cfg(feature = "lock")]
        {
            let dir = tempfile::tempdir().expect("temp dir");
            let config = crate::lock::LockConfig::new(dir.path(), 1, "holder");
            let _held = crate::lock::try_acquire(&config).expect("lock").expect("free slot");
            let cancel = Cancel::new();
            cancel.cancel();
            assert_eq!(crate::lock::acquire(&config, Some(&cancel)).err(), Some(Error::Canceled { step: None }));
        }
    }

    /// Starts a blocked child of its own, says `ready`, and blocks; run by `run_cancelable_ends_the_process_tree`.
    #[test]
    #[ignore = "child process of run_cancelable_ends_the_process_tree"]
    fn tree_child() {
        if std::env::var_os(TREE_CHILD).is_some() {
            let mut grandchild = test_child("runner::tests::blocked_child", BLOCKED_CHILD).spawn().expect("grandchild");
            let mut out = io::stdout();
            writeln!(out, "ready").and_then(|()| out.flush()).expect("stdout");
            // The grandchild never exits, so this blocks too.
            let _ = grandchild.wait();
        }
    }

    /// Writes bytes that are not UTF-8 to stdout; run by `missing_tools_and_output_bytes`.
    #[test]
    #[ignore = "child process of missing_tools_and_output_bytes"]
    fn bytes_child() {
        if std::env::var_os(BYTES_CHILD).is_some() {
            let mut out = io::stdout();
            out.write_all(RAW_BYTES).and_then(|()| out.flush()).expect("stdout");
        }
    }

    /// Blocks; run by `tree_child`.
    #[test]
    #[ignore = "child process of tree_child"]
    fn blocked_child() {
        if std::env::var_os(BLOCKED_CHILD).is_some() {
            block_forever();
        }
    }
}
