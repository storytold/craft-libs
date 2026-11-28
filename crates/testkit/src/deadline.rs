//! Limits on how long a test waits.
//!
//! A limit here guards against a hang and does not decide a result. A value or panic that a body
//! produces is the call's result whenever it arrives, and a test fails on the limit only when nothing
//! has arrived by then.
//!
//! [`with_deadline`] runs a test body on its own thread and returns its value. A panic in the body is
//! raised again in the test with its original payload. When the body has neither returned nor
//! panicked within the limit, the test fails with the call's label, the limit and the variable that
//! sets it.
//!
//! [`wait_until`] calls a condition on the test's thread, and calls it again each time another
//! thread raises the [`Signal`] the call waits on. It returns when the condition returns `true`, and
//! fails the test with its description when the limit has passed since the call began and the latest
//! call of the condition returned `false`.
//!
//! A limit is a default [`Duration`] and the name of a variable that replaces it. A plain `Duration`
//! reads [`DEFAULT_VARIABLE`]; [`DeadlineLimit::with_variable`] names another one. The variable holds
//! whole seconds as decimal digits and is read on every call. A value that is empty, signed,
//! fractional, not Unicode, zero, or larger than a `u64` fails the test that reads it, as does a zero
//! default and a limit past the latest time the platform's clock can represent.
//!
//! Neither function can stop code that does not return. When a body does not finish within its
//! limit, the test fails and the body's thread is left running until the test process exits. A
//! condition that blocks is not interrupted, so a condition must return promptly.

use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The variable a limit reads unless [`DeadlineLimit::with_variable`] names another.
pub const DEFAULT_VARIABLE: &str = "CRAFT_TEST_TIMEOUT_SECS";

/// The name of the thread [`with_deadline`] runs a body on.
const WORKER_NAME: &str = "craft-testkit deadline";

/// A limit for [`with_deadline`] and [`wait_until`]: a default duration and the name of the
/// variable that replaces it.
///
/// ```
/// use std::time::Duration;
/// use craft_testkit::deadline::DeadlineLimit;
///
/// // Reads CRAFT_TEST_TIMEOUT_SECS.
/// let limit: DeadlineLimit = Duration::from_secs(30).into();
/// assert_eq!(limit.variable(), "CRAFT_TEST_TIMEOUT_SECS");
///
/// // Reads the app's own variable instead.
/// let limit = DeadlineLimit::new(Duration::from_secs(30)).with_variable("FILMCRAFT_HOSTILE_TIMEOUT_SECS");
/// assert_eq!(limit.default_limit(), Duration::from_secs(30));
/// ```
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DeadlineLimit<'a> {
    default: Duration,
    variable: &'a str,
}

impl DeadlineLimit<'static> {
    /// A limit of `default` that [`DEFAULT_VARIABLE`] replaces.
    pub fn new(default: Duration) -> Self {
        Self { default, variable: DEFAULT_VARIABLE }
    }
}

impl<'a> DeadlineLimit<'a> {
    /// The same default, replaced by the variable `name` instead.
    pub fn with_variable<'b>(self, name: &'b str) -> DeadlineLimit<'b> {
        DeadlineLimit { default: self.default, variable: name }
    }

    /// The limit that applies when the variable is not set.
    pub fn default_limit(&self) -> Duration {
        self.default
    }

    /// The name of the variable that replaces the default.
    pub fn variable(&self) -> &'a str {
        self.variable
    }
}

impl From<Duration> for DeadlineLimit<'static> {
    fn from(default: Duration) -> Self {
        Self::new(default)
    }
}

/// A signal that [`wait_until`] waits on: the thread that changes the state a condition reads raises
/// it after each change, and every call waiting on it then calls its condition again.
///
/// ```
/// use craft_testkit::deadline::Signal;
///
/// let signal = Signal::new();
/// // A raise with no call waiting is kept: the next wait sees it and calls its condition again.
/// signal.raise();
/// ```
#[derive(Debug, Default)]
pub struct Signal {
    /// How many times the signal has been raised, wrapping.
    raised: Mutex<u64>,
    changed: Condvar,
}

impl Signal {
    /// A signal that has not been raised.
    pub fn new() -> Self {
        Self::default()
    }

    /// Wakes every [`wait_until`] call waiting on this signal, so that each calls its condition again.
    pub fn raise(&self) {
        let mut raised = self.lock();
        *raised = raised.wrapping_add(1);
        self.changed.notify_all();
    }

    fn lock(&self) -> MutexGuard<'_, u64> {
        self.raised.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Runs `f` on its own thread and returns its value, failing the calling test when `f` has neither
/// returned nor panicked within `limit`.
///
/// A panic in `f` is raised again here with its original payload. The limit guards against a hang:
/// a value or panic that `f` produces is the result whenever it arrives, and the test fails on the
/// limit only when `f` has produced neither by then. That failure names `label`, the limit and the
/// variable that sets it, and the thread running `f` is left running. The limit is read and checked
/// before `f` starts, and a limit that is not valid fails the test without running `f`. The limit
/// counts from the start of the thread running `f`.
///
/// ```
/// use std::time::Duration;
/// use craft_testkit::deadline::with_deadline;
///
/// let sum = with_deadline(Duration::from_secs(30), "sum of 1 to 10", || (1..=10).sum::<u32>());
/// assert_eq!(sum, 55);
/// ```
// Failing the calling test is what this function is for.
#[allow(clippy::panic)]
#[track_caller]
pub fn with_deadline<'a, T: Send + 'static>(limit: impl Into<DeadlineLimit<'a>>, label: &str, f: impl FnOnce() -> T + Send + 'static) -> T {
    match run_with_deadline(limit.into(), label, f, read_variable, spawn_worker, |rx, limit| rx.recv_timeout(limit)) {
        Ok(value) => value,
        Err(Failure::Panic(payload)) => resume_unwind(payload),
        Err(Failure::Message(message)) => panic!("{message}"),
    }
}

/// Calls `cond` on the calling thread, and again each time `signal` is raised, until it returns
/// `true`, failing the calling test when the limit has passed since the call began and the latest
/// call of `cond` returned `false`.
///
/// `cond` is called at once. A raise that comes while `cond` runs is not lost: `cond` is called
/// again straight after. The limit guards against a hang and counts from the start of the call, so
/// a signal that keeps being raised while `cond` stays `false` still fails the test. `cond` is
/// called before the limit is checked, so a condition that holds is never failed by it. The failure
/// names `what`, the limit and the
/// variable that sets it. A panic in `cond` reaches the caller unchanged. `cond` runs on the calling
/// thread and is not interrupted, so it must return promptly. The threads that change what `cond`
/// reads raise `signal` after each change.
///
/// ```
/// use std::sync::atomic::{AtomicBool, Ordering};
/// use std::time::Duration;
/// use craft_testkit::deadline::{Signal, wait_until};
///
/// let ready = AtomicBool::new(false);
/// let signal = Signal::new();
/// std::thread::scope(|s| {
///     s.spawn(|| {
///         ready.store(true, Ordering::SeqCst);
///         signal.raise();
///     });
///     wait_until(Duration::from_secs(30), "the worker to start", &signal, || ready.load(Ordering::SeqCst));
/// });
/// ```
// Failing the calling test is what this function is for.
#[allow(clippy::panic)]
#[track_caller]
pub fn wait_until<'a>(limit: impl Into<DeadlineLimit<'a>>, what: &str, signal: &Signal, mut cond: impl FnMut() -> bool) {
    let limit = limit.into();
    let resolved = match resolve(&limit, read_variable, Instant::now()) {
        Ok(resolved) => resolved,
        Err(error) => panic!("waiting for {what}: {error}"),
    };
    if let Err(message) = wait_for_signal(&resolved, what, signal, &mut cond, Instant::now, wait_for_raise) {
        panic!("{message}");
    }
}

/// A limit after reading its variable.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Resolved<'a> {
    pub(crate) limit: Duration,
    variable: &'a str,
    from_variable: bool,
}

impl fmt::Display for Resolved<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.from_variable {
            write!(f, "{:?}, set by {}", self.limit, self.variable)
        } else {
            write!(f, "{:?}, the default; {} replaces it in whole seconds", self.limit, self.variable)
        }
    }
}

/// Why a limit cannot be used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum LimitError<'a> {
    /// The variable name is empty or holds `=` or a NUL character.
    BadName { variable: &'a str },
    /// The variable's value is not Unicode.
    NotUnicode { variable: &'a str },
    /// The variable's value is not a whole number of seconds above zero.
    BadValue { variable: &'a str, value: String, reason: &'static str },
    /// The default limit is zero.
    ZeroDefault { variable: &'a str },
    /// The limit ends past the latest time the clock can represent.
    TooLarge { resolved: Resolved<'a> },
}

impl fmt::Display for LimitError<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName { variable } => write!(f, "{variable:?} cannot name a variable: it is empty or holds `=` or a NUL character"),
            Self::NotUnicode { variable } => write!(f, "{variable} is not valid Unicode; set it to a whole number of seconds above zero"),
            Self::BadValue { variable, value, reason } => {
                write!(f, "{variable}={value:?} {reason}; set it to a whole number of seconds above zero")
            }
            Self::ZeroDefault { variable } => write!(f, "the default limit is zero; pass a default above zero ({variable} replaces it)"),
            Self::TooLarge { resolved } => write!(f, "the limit of {resolved} ends past the latest time this platform's clock can represent"),
        }
    }
}

/// Reads and checks `limit`: the variable's value when `lookup` finds one, otherwise the default.
/// `now` is the time the limit starts from.
pub(crate) fn resolve<'a>(
    limit: &DeadlineLimit<'a>,
    lookup: impl FnOnce(&str) -> Option<OsString>,
    now: Instant,
) -> Result<Resolved<'a>, LimitError<'a>> {
    let variable = limit.variable;
    if variable.is_empty() || variable.contains(['=', '\0']) {
        return Err(LimitError::BadName { variable });
    }
    if limit.default.is_zero() {
        return Err(LimitError::ZeroDefault { variable });
    }
    let resolved = match lookup(variable) {
        None => Resolved { limit: limit.default, variable, from_variable: false },
        Some(raw) => {
            let value = raw.into_string().map_err(|_| LimitError::NotUnicode { variable })?;
            let secs = parse_secs(&value).map_err(|reason| LimitError::BadValue { variable, value: value.clone(), reason })?;
            Resolved { limit: Duration::from_secs(secs), variable, from_variable: true }
        }
    };
    if now.checked_add(resolved.limit).is_none() {
        return Err(LimitError::TooLarge { resolved });
    }
    Ok(resolved)
}

/// Parses a whole number of seconds above zero, written as decimal digits only.
fn parse_secs(value: &str) -> Result<u64, &'static str> {
    if value.is_empty() {
        return Err("is empty");
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("is not a whole number of seconds written in digits");
    }
    match value.parse::<u64>() {
        Ok(0) => Err("is zero"),
        Ok(secs) => Ok(secs),
        Err(_) => Err("is more seconds than a u64 holds"),
    }
}

/// Reads the variable `name` from the process environment.
fn read_variable(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// A body boxed for the worker thread.
type Job = Box<dyn FnOnce() + Send + 'static>;

/// Starts `job` on a named thread and detaches it. The thread sends on `done` after `job` returns.
fn spawn_worker(job: Job, done: Sender<()>) -> io::Result<()> {
    thread::Builder::new()
        .name(WORKER_NAME.to_owned())
        .spawn(move || {
            job();
            let _ = done.send(());
        })
        .map(drop)
}

/// The body's result, filled in by the worker.
type Slot<T> = Arc<Mutex<Option<thread::Result<T>>>>;

/// How a [`with_deadline`] call failed.
enum Failure {
    /// The body panicked with this payload.
    Panic(Box<dyn Any + Send>),
    /// The call failed for this reason.
    Message(String),
}

/// [`with_deadline`] with the variable lookup, the thread start and the wait for the worker passed
/// in. The worker stores the body's result under a lock and then sends on the channel `spawn`
/// receives. `wait` waits on that channel for at most the limit. Whatever `wait` returns, a stored
/// result is the call's result, so the limit fails the call only when the body has produced nothing.
fn run_with_deadline<T: Send + 'static>(
    limit: DeadlineLimit<'_>,
    label: &str,
    f: impl FnOnce() -> T + Send + 'static,
    lookup: impl FnOnce(&str) -> Option<OsString>,
    spawn: impl FnOnce(Job, Sender<()>) -> io::Result<()>,
    wait: impl FnOnce(&Receiver<()>, Duration) -> Result<(), RecvTimeoutError>,
) -> Result<T, Failure> {
    let resolved = resolve(&limit, lookup, Instant::now()).map_err(|error| Failure::Message(format!("{label}: {error}")))?;
    let (tx, rx) = mpsc::channel();
    let slot: Slot<T> = Arc::new(Mutex::new(None));
    let worker_slot = Arc::clone(&slot);
    let job: Job = Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(f));
        *worker_slot.lock().unwrap_or_else(PoisonError::into_inner) = Some(result);
    });
    spawn(job, tx).map_err(|error| Failure::Message(format!("{label}: could not start the worker thread: {error}")))?;
    let waited = wait(&rx, resolved.limit);
    let stored = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    match (stored, waited) {
        (Some(Ok(value)), _) => Ok(value),
        (Some(Err(payload)), _) => Err(Failure::Panic(payload)),
        (None, Err(RecvTimeoutError::Timeout)) => {
            Err(Failure::Message(format!("{label} did not finish within {resolved}; its thread is left running")))
        }
        (None, _) => Err(Failure::Message(format!("{label}: the worker thread stopped without a result"))),
    }
}

/// Waits on `changed` while `raised` still reads `seen`, for at most `limit`. Returns the guard and
/// whether the limit passed with no raise.
fn wait_for_raise<'g>(changed: &Condvar, raised: MutexGuard<'g, u64>, seen: u64, limit: Duration) -> (MutexGuard<'g, u64>, bool) {
    let (raised, result) = changed.wait_timeout_while(raised, limit, |count| *count == seen).unwrap_or_else(PoisonError::into_inner);
    (raised, result.timed_out())
}

/// The loop of [`wait_until`], with the clock and the wait for a raise passed in. The raise count is
/// read before each call of `cond`, so a raise during the call ends the next wait at once. The limit
/// counts from the start of the loop, and each wait lasts at most the time left.
fn wait_for_signal(
    resolved: &Resolved<'_>,
    what: &str,
    signal: &Signal,
    cond: &mut dyn FnMut() -> bool,
    now: impl Fn() -> Instant,
    wait: impl for<'g> Fn(&Condvar, MutexGuard<'g, u64>, u64, Duration) -> (MutexGuard<'g, u64>, bool),
) -> Result<(), String> {
    let start = now();
    let mut seen = *signal.lock();
    loop {
        if cond() {
            return Ok(());
        }
        let remaining = resolved.limit.saturating_sub(now().saturating_duration_since(start));
        if remaining.is_zero() {
            return Err(format!("timed out waiting for {what} within {resolved}: the condition still returned false at the limit"));
        }
        let (raised, timed_out) = wait(&signal.changed, signal.lock(), seen, remaining);
        if timed_out {
            return Err(format!("timed out waiting for {what} within {resolved}: the signal was not raised after the last check"));
        }
        seen = *raised;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::ffi::OsStr;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, Ordering};

    /// A variable no test sets, so the environment cannot lengthen a test's short limit.
    const UNSET: &str = "CRAFT_TESTKIT_DEADLINE_UNSET_IN_TESTS";

    /// The variable `run_child` sets for the child process.
    const CHILD_VARIABLE: &str = "CRAFT_TESTKIT_DEADLINE_CHILD_SECS";

    fn short() -> DeadlineLimit<'static> {
        DeadlineLimit::new(Duration::from_millis(50)).with_variable(UNSET)
    }

    fn long() -> DeadlineLimit<'static> {
        DeadlineLimit::new(Duration::from_secs(600)).with_variable(UNSET)
    }

    fn message(payload: Box<dyn Any + Send>) -> String {
        match payload.downcast::<String>() {
            Ok(text) => *text,
            Err(payload) => match payload.downcast::<&str>() {
                Ok(text) => (*text).to_owned(),
                Err(_) => panic!("the panic payload is not a message"),
            },
        }
    }

    fn failure_message<T>(result: Result<T, Failure>) -> String {
        match result {
            Ok(_) => panic!("the call did not fail"),
            Err(Failure::Panic(payload)) => message(payload),
            Err(Failure::Message(text)) => text,
        }
    }

    /// A wait for the worker that returns at once, as when the limit passes with nothing received.
    fn limit_passes(_: &Receiver<()>, _: Duration) -> Result<(), RecvTimeoutError> {
        Err(RecvTimeoutError::Timeout)
    }

    /// Runs `run_with_deadline` with its limit passing before the body has a result, and returns the
    /// failure's message. The body runs on its own thread and blocks until the call has returned.
    fn hung<T: Send + 'static>(limit: DeadlineLimit<'_>, label: &str, value: T) -> String {
        let (gate, gate_rx) = mpsc::channel::<()>();
        let (done_tx, done) = mpsc::channel();
        let body = move || {
            let _ = gate_rx.recv();
            done_tx.send(()).unwrap();
            value
        };
        let result = run_with_deadline(limit, label, body, |_| None, spawn_worker, limit_passes);
        drop(gate);
        done.recv().unwrap();
        failure_message(result)
    }

    /// Runs `child_reads_its_variable` in a child process with `CHILD_VARIABLE` set to `value`, and
    /// returns what the child printed. The child must fail.
    fn run_child(value: &OsStr) -> String {
        let out = Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "deadline::tests::child_reads_its_variable", "--nocapture", "--test-threads=1"])
            .env(CHILD_VARIABLE, value)
            .output()
            .unwrap();
        let printed = format!("{}{}", String::from_utf8_lossy(&out.stdout), String::from_utf8_lossy(&out.stderr));
        assert!(!out.status.success(), "the child passed:\n{printed}");
        assert!(printed.contains("running 1 test"), "the child did not run the test:\n{printed}");
        printed
    }

    /// Runs in the child processes of `run_child`. Without the variable it takes its default and
    /// passes; with it, the body blocks until the limit passes.
    #[test]
    fn child_reads_its_variable() {
        let blocks = std::env::var_os(CHILD_VARIABLE).is_some();
        let (_keep, rx) = mpsc::channel::<()>();
        with_deadline(DeadlineLimit::new(Duration::from_secs(2)).with_variable(CHILD_VARIABLE), "child body", move || {
            if blocks {
                let _ = rx.recv();
            }
        });
    }

    #[test]
    fn a_slow_body_passes() {
        let (tx, rx) = mpsc::channel();
        let sender = thread::spawn(move || tx.send(()).unwrap());
        with_deadline(long(), "slow body", move || rx.recv().unwrap());
        sender.join().unwrap();
    }

    #[test]
    #[should_panic(expected = "boom")]
    fn a_panic_in_the_body_is_re_raised() {
        with_deadline(long(), "panicking body", || panic!("boom"));
    }

    #[test]
    fn a_hang_fails_and_names_the_input() {
        let text = hung(short(), "seed 7 byte 3", ());
        assert!(text.contains("seed 7 byte 3 did not finish within 50ms"), "{text}");
    }

    #[test]
    fn with_deadline_completes() {
        with_deadline(Duration::from_secs(5), "fast body", || {
            // fast operation
        });
    }

    #[test]
    fn with_deadline_times_out() {
        let text = hung(short(), "blocked body", 7u8);
        assert!(text.contains("did not finish within") && text.contains("its thread is left running"), "{text}");
    }

    #[test]
    fn returns_worker_value() {
        assert_eq!(with_deadline(long(), "value", || vec![1u8, 2, 3]), vec![1, 2, 3]);
    }

    #[test]
    fn worker_panic_keeps_original_payload() {
        #[derive(Debug, PartialEq)]
        struct Marker(u32);
        let payload = catch_unwind(|| with_deadline(long(), "typed panic", || std::panic::panic_any(Marker(7)))).unwrap_err();
        assert_eq!(payload.downcast_ref::<Marker>(), Some(&Marker(7)));
    }

    #[test]
    fn timeout_names_this_call_only() {
        let call = |label: &'static str| thread::spawn(move || hung(short(), label, ()));
        let first = call("first call");
        let second = call("second call");
        let first = first.join().unwrap();
        let second = second.join().unwrap();
        assert!(first.contains("first call") && !first.contains("second call"), "{first}");
        assert!(second.contains("second call") && !second.contains("first call"), "{second}");
    }

    #[test]
    fn concurrent_calls_keep_limits_and_labels_separate() {
        let (release, release_rx) = mpsc::channel();
        let long_call = thread::spawn(move || {
            with_deadline(long(), "long call", move || {
                release_rx.recv().unwrap();
                7
            })
        });
        let text = hung(short(), "short call", ());
        assert!(text.contains("short call") && text.contains("50ms") && !text.contains("long call"), "{text}");
        release.send(()).unwrap();
        assert_eq!(long_call.join().unwrap(), 7);
    }

    #[test]
    fn spawn_failure_reports_cause() {
        let ran = AtomicBool::new(false);
        let spawn = |_job: Job, _done: Sender<()>| {
            ran.store(true, Ordering::SeqCst);
            Err(io::Error::other("no threads left"))
        };
        let result = run_with_deadline(long(), "no thread", || (), |_| None, spawn, limit_passes);
        let text = failure_message(result);
        assert!(ran.load(Ordering::SeqCst));
        assert!(text.contains("no thread: could not start the worker thread: no threads left"), "{text}");
    }

    #[test]
    fn invalid_override_fails() {
        let printed = run_child(OsStr::new("5s"));
        assert!(printed.contains("CRAFT_TESTKIT_DEADLINE_CHILD_SECS=\"5s\" is not a whole number of seconds"), "{printed}");
    }

    #[test]
    fn zero_override_fails() {
        let printed = run_child(OsStr::new("0"));
        assert!(printed.contains("CRAFT_TESTKIT_DEADLINE_CHILD_SECS=\"0\" is zero"), "{printed}");
    }

    #[cfg(unix)]
    #[test]
    fn non_unicode_override_fails() {
        use std::os::unix::ffi::OsStrExt;
        let printed = run_child(OsStr::from_bytes(b"3\xff"));
        assert!(printed.contains("CRAFT_TESTKIT_DEADLINE_CHILD_SECS is not valid Unicode"), "{printed}");
    }

    #[test]
    fn valid_override_replaces_the_default() {
        let printed = run_child(OsStr::new("1"));
        assert!(printed.contains("child body did not finish within 1s, set by CRAFT_TESTKIT_DEADLINE_CHILD_SECS"), "{printed}");
    }

    #[test]
    #[should_panic(expected = "the default limit is zero")]
    fn zero_default_fails() {
        with_deadline(DeadlineLimit::new(Duration::ZERO).with_variable(UNSET), "zero", || ());
    }

    #[test]
    fn huge_limit_fails_cleanly() {
        let huge = DeadlineLimit::new(Duration::MAX).with_variable(UNSET);
        let ran = AtomicBool::new(false);
        let spawn = |_job: Job, _done: Sender<()>| {
            ran.store(true, Ordering::SeqCst);
            Ok(())
        };
        let text = failure_message(run_with_deadline(huge, "huge", || (), |_| None, spawn, limit_passes));
        assert!(text.contains("ends past the latest time"), "{text}");
        assert!(!ran.load(Ordering::SeqCst), "the body started");
        let text = message(catch_unwind(|| wait_until(huge, "a huge limit", &Signal::new(), || true)).unwrap_err());
        assert!(text.contains("waiting for a huge limit: the limit of") && text.contains("ends past the latest time"), "{text}");
    }

    #[test]
    fn malformed_values_are_rejected() {
        let limit = DeadlineLimit::new(Duration::from_secs(5)).with_variable("X");
        let now = Instant::now();
        for value in ["", "+5", "-5", "1.5", " 5", "5 ", "abc", "5s", "0", "000", "18446744073709551616"] {
            let result = resolve(&limit, |_| Some(OsString::from(value)), now);
            assert!(matches!(result, Err(LimitError::BadValue { .. })), "{value:?} gave {result:?}");
        }
        let resolved = resolve(&limit, |_| Some(OsString::from("007")), now).unwrap();
        assert_eq!(resolved, Resolved { limit: Duration::from_secs(7), variable: "X", from_variable: true });
        let resolved = resolve(&limit, |_| None, now).unwrap();
        assert_eq!(resolved, Resolved { limit: Duration::from_secs(5), variable: "X", from_variable: false });
    }

    #[test]
    fn bad_variable_names_are_rejected_before_reading() {
        for name in ["", "A=B", "A\0B"] {
            let limit = DeadlineLimit::new(Duration::from_secs(5)).with_variable(name);
            let result = resolve(&limit, |_| panic!("the variable was read"), Instant::now());
            assert_eq!(result, Err(LimitError::BadName { variable: name }));
        }
    }

    #[test]
    fn a_duration_reads_the_default_variable() {
        let limit: DeadlineLimit = Duration::from_secs(3).into();
        assert_eq!(limit.variable(), DEFAULT_VARIABLE);
        assert_eq!(limit.default_limit(), Duration::from_secs(3));
    }

    #[test]
    fn wait_until_returns_on_condition() {
        let ready = &AtomicBool::new(false);
        let signal = &Signal::new();
        let (asked, asked_rx) = mpsc::channel();
        thread::scope(|s| {
            s.spawn(move || {
                asked_rx.recv().unwrap();
                ready.store(true, Ordering::SeqCst);
                signal.raise();
            });
            let mut calls = 0;
            wait_until(long(), "the flag", signal, || {
                calls += 1;
                if calls == 1 {
                    // The other thread sets the flag only after this call has started.
                    asked.send(()).unwrap();
                    return false;
                }
                ready.load(Ordering::SeqCst)
            });
            assert_eq!(calls, 2);
        });
    }

    #[test]
    fn a_raise_during_the_condition_ends_the_next_wait_at_once() {
        let signal = Signal::new();
        let mut calls = 0;
        let mut cond = || {
            calls += 1;
            if calls == 1 {
                signal.raise();
            }
            calls == 2
        };
        let result = wait_for_signal(&fake_resolved(Duration::from_secs(600)), "a raise", &signal, &mut cond, Instant::now, |_, raised, seen, _| {
            assert_ne!(*raised, seen, "the raise made during the condition was lost");
            (raised, false)
        });
        assert_eq!(result, Ok(()));
        assert_eq!(calls, 2);
    }

    #[test]
    fn wait_until_times_out_with_label() {
        let mut calls = 0;
        let mut cond = || {
            calls += 1;
            false
        };
        let resolved = fake_resolved(Duration::from_millis(50));
        let text = wait_for_signal(&resolved, "the export to finish", &Signal::new(), &mut cond, Instant::now, |_, raised, _, _| (raised, true))
            .unwrap_err();
        assert!(text.starts_with("timed out waiting for the export to finish within 50ms"), "{text}");
        assert!(text.ends_with("the signal was not raised after the last check"), "{text}");
        assert_eq!(calls, 1);
    }

    #[test]
    fn raises_that_never_satisfy_the_condition_fail_at_the_limit() {
        // A clock that moves 20 s on each reading, and a wait that always reports a raise.
        let start = Instant::now();
        let readings = std::cell::Cell::new(0u32);
        let now = || {
            let n = readings.get();
            readings.set(n + 1);
            start + Duration::from_secs(20) * n
        };
        let mut calls = 0;
        let mut cond = || {
            calls += 1;
            false
        };
        let resolved = fake_resolved(Duration::from_secs(60));
        let text = wait_for_signal(&resolved, "a progress tick", &Signal::new(), &mut cond, now, |_, mut raised, _, _| {
            *raised = raised.wrapping_add(1);
            (raised, false)
        })
        .unwrap_err();
        assert!(text.starts_with("timed out waiting for a progress tick within 60s"), "{text}");
        assert!(text.ends_with("the condition still returned false at the limit"), "{text}");
        assert_eq!(calls, 3);
    }

    #[test]
    fn a_thread_raising_in_a_loop_cannot_hold_wait_until_past_the_limit() {
        let stop = Arc::new(AtomicBool::new(false));
        let signal = Arc::new(Signal::new());
        let raiser = {
            let (stop, signal) = (Arc::clone(&stop), Arc::clone(&signal));
            thread::spawn(move || {
                while !stop.load(Ordering::SeqCst) {
                    signal.raise();
                    thread::yield_now();
                }
            })
        };
        let waiter_signal = Arc::clone(&signal);
        let guard = DeadlineLimit::new(Duration::from_secs(30)).with_variable(UNSET);
        let outcome = with_deadline(guard, "wait_until under constant raises", move || {
            catch_unwind(AssertUnwindSafe(|| wait_until(short(), "a condition that never holds", &waiter_signal, || false))).map_err(message)
        });
        stop.store(true, Ordering::SeqCst);
        raiser.join().unwrap();
        let text = outcome.unwrap_err();
        assert!(text.contains("timed out waiting for a condition that never holds within 50ms"), "{text}");
    }
    #[test]
    #[should_panic(expected = "condition boom")]
    fn wait_until_propagates_condition_panic() {
        wait_until(long(), "a panicking condition", &Signal::new(), || panic!("condition boom"));
    }

    #[test]
    fn wait_until_keeps_a_typed_condition_payload() {
        #[derive(Debug, PartialEq)]
        struct Marker(u32);
        let payload = catch_unwind(|| wait_until(long(), "a typed panic", &Signal::new(), || std::panic::panic_any(Marker(9)))).unwrap_err();
        assert_eq!(payload.downcast_ref::<Marker>(), Some(&Marker(9)));
    }

    /// A limit for the loop tests, which pass their own wait.
    fn fake_resolved(limit: Duration) -> Resolved<'static> {
        Resolved { limit, variable: UNSET, from_variable: false }
    }

    /// Runs `run_with_deadline` on `f` with the body's result arriving only after the limit has
    /// passed: the body does not start until the wait for it, which runs it on the calling thread
    /// and then reports that the limit passed. The worker never sends.
    fn finishing_after_the_limit<T: Send + 'static>(f: impl FnOnce() -> T + Send + 'static) -> Result<T, Failure> {
        let held = RefCell::new(None);
        let spawn = |job: Job, done: Sender<()>| {
            *held.borrow_mut() = Some((job, done));
            Ok(())
        };
        let wait = |_: &Receiver<()>, _: Duration| {
            if let Some((job, _done)) = held.borrow_mut().take() {
                job();
            }
            Err(RecvTimeoutError::Timeout)
        };
        run_with_deadline(short(), "late body", f, |_| None, spawn, wait)
    }

    #[test]
    fn a_value_that_arrives_after_the_limit_is_returned() {
        assert!(matches!(finishing_after_the_limit(|| 1), Ok(1)));
    }

    #[test]
    fn a_panic_that_arrives_after_the_limit_keeps_its_payload() {
        let payload = match finishing_after_the_limit(|| -> u8 { panic!("late boom") }) {
            Err(Failure::Panic(payload)) => payload,
            Ok(_) => panic!("the call did not fail"),
            Err(Failure::Message(text)) => panic!("the payload was replaced: {text}"),
        };
        assert_eq!(message(payload), "late boom");
    }
}
