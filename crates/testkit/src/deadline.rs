//! Limits on how long a test waits.
//!
//! [`with_deadline`] runs a test body on its own thread and returns its value. When the body does not
//! finish within the limit, the test fails with the call's label, the limit and the variable that sets
//! it. A panic in the body is raised again in the test with its original payload.
//!
//! [`wait_until`] calls a condition on the test's thread until it returns `true`, pausing between
//! calls, and fails the test with its description when the limit passes first.
//!
//! A limit is a default [`Duration`] and the name of a variable that replaces it. A plain `Duration`
//! reads [`DEFAULT_VARIABLE`]; [`DeadlineLimit::with_variable`] names another one. The variable holds
//! whole seconds as decimal digits and is read on every call. A value that is empty, signed,
//! fractional, not Unicode, zero, or larger than a `u64` fails the test that reads it, as does a zero
//! default and a limit past the latest time the platform's clock can represent.
//!
//! Neither function can stop code that does not return. A body that runs past its limit fails the
//! test, and its thread is left running until the test process exits. A condition that blocks is
//! not interrupted: [`wait_until`] checks its limit only between calls, so a condition must return
//! promptly.

use std::any::Any;
use std::ffi::OsString;
use std::fmt;
use std::io;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

/// The variable a limit reads unless [`DeadlineLimit::with_variable`] names another.
pub const DEFAULT_VARIABLE: &str = "CRAFT_TEST_TIMEOUT_SECS";

/// The first pause of [`wait_until`] between two calls of its condition.
const FIRST_PAUSE: Duration = Duration::from_millis(1);

/// The longest pause of [`wait_until`]: each pause doubles up to this.
const MAX_PAUSE: Duration = Duration::from_millis(50);

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

/// Runs `f` on its own thread and returns its value, failing the calling test when `f` does not
/// finish within `limit`.
///
/// The failure names `label`, the limit and the variable that sets it. A panic in `f` is raised
/// again here with its original payload. The limit is read and checked before `f` starts, and a
/// limit that is not valid fails the test without running `f`. The limit counts from the call, so
/// the time spent starting the thread counts against it. When `f` returns or panics after the limit
/// has passed, the test fails with the label and the limit, and with the panic's message if it has
/// one. When the limit passes, the thread running `f` is left running.
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
    let start = Instant::now();
    match run_with_deadline(limit.into(), label, f, read_variable, spawn_worker, Arc::new(move || start.elapsed())) {
        Ok(value) => value,
        Err(Failure::Panic(payload)) => resume_unwind(payload),
        Err(Failure::Message(message)) => panic!("{message}"),
    }
}

/// Calls `cond` on the calling thread until it returns `true`, failing the calling test when the
/// limit passes first.
///
/// `cond` is called at once, and then after pauses that grow from 1 ms to 50 ms, but not again once
/// the limit has passed. The failure names `what`, the limit and the variable that sets it. A `true`
/// that `cond` returns after the limit has passed also fails the test. A panic in `cond` reaches the
/// caller unchanged. `cond` runs on the calling thread and is not interrupted, so it must return
/// promptly.
///
/// ```
/// use std::sync::atomic::{AtomicBool, Ordering};
/// use std::time::Duration;
/// use craft_testkit::deadline::wait_until;
///
/// let ready = AtomicBool::new(false);
/// std::thread::scope(|s| {
///     s.spawn(|| ready.store(true, Ordering::SeqCst));
///     wait_until(Duration::from_secs(30), "the worker to start", || ready.load(Ordering::SeqCst));
/// });
/// ```
// Failing the calling test is what this function is for.
#[allow(clippy::panic)]
#[track_caller]
pub fn wait_until<'a>(limit: impl Into<DeadlineLimit<'a>>, what: &str, mut cond: impl FnMut() -> bool) {
    let limit = limit.into();
    let start = Instant::now();
    let resolved = match resolve(&limit, read_variable, start) {
        Ok(resolved) => resolved,
        Err(error) => panic!("waiting for {what}: {error}"),
    };
    let mut elapsed = || start.elapsed();
    if let Err(message) = poll_until(&resolved, what, &mut cond, &mut elapsed, &mut thread::sleep) {
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

/// The body's result and the clock reading when it finished, filled in by the worker.
type Slot<T> = Arc<Mutex<Option<(thread::Result<T>, Duration)>>>;

/// How a [`with_deadline`] call failed.
enum Failure {
    /// The body panicked with this payload.
    Panic(Box<dyn Any + Send>),
    /// The call failed for this reason.
    Message(String),
}

/// A clock that returns the time since a [`with_deadline`] call started.
type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The text of a panic payload that holds a message.
fn panic_text(payload: &(dyn Any + Send)) -> Option<&str> {
    payload.downcast_ref::<String>().map(String::as_str).or_else(|| payload.downcast_ref::<&str>().copied())
}

/// [`with_deadline`] with the variable lookup, the thread start and the clock passed in. Starting
/// the worker counts against the limit. The worker stores the body's result with the clock reading
/// when it finished, under a lock, and then sends on the channel `spawn` receives. When the wait
/// times out, the stored result is still used if it is there, so a body that finished in time passes
/// even when its thread sends late.
fn run_with_deadline<T: Send + 'static>(
    limit: DeadlineLimit<'_>,
    label: &str,
    f: impl FnOnce() -> T + Send + 'static,
    lookup: impl FnOnce(&str) -> Option<OsString>,
    spawn: impl FnOnce(Job, Sender<()>) -> io::Result<()>,
    clock: Clock,
) -> Result<T, Failure> {
    let resolved = resolve(&limit, lookup, Instant::now()).map_err(|error| Failure::Message(format!("{label}: {error}")))?;
    let (tx, rx) = mpsc::channel();
    let slot: Slot<T> = Arc::new(Mutex::new(None));
    let worker_slot = Arc::clone(&slot);
    let worker_clock = Arc::clone(&clock);
    let job: Job = Box::new(move || {
        let result = catch_unwind(AssertUnwindSafe(f));
        // The clock is read under the lock: once the caller finds the slot empty after its limit,
        // any later reading is past the limit too.
        let mut stored = worker_slot.lock().unwrap_or_else(PoisonError::into_inner);
        *stored = Some((result, worker_clock()));
    });
    spawn(job, tx).map_err(|error| Failure::Message(format!("{label}: could not start the worker thread: {error}")))?;
    let waited = rx.recv_timeout(resolved.limit.saturating_sub(clock()));
    let stored = slot.lock().unwrap_or_else(PoisonError::into_inner).take();
    match (stored, waited) {
        (Some((result, finished)), _) if finished > resolved.limit => {
            let panicked = match &result {
                Ok(_) => String::new(),
                Err(payload) => format!(" (it panicked: {})", panic_text(payload.as_ref()).unwrap_or("a payload that is not a message")),
            };
            Err(Failure::Message(format!("{label} finished only after its limit of {resolved}{panicked}")))
        }
        (Some((Ok(value), _)), _) => Ok(value),
        (Some((Err(payload), _)), _) => Err(Failure::Panic(payload)),
        (None, Err(RecvTimeoutError::Timeout)) => {
            Err(Failure::Message(format!("{label} did not finish within {resolved}; its thread is left running")))
        }
        (None, _) => Err(Failure::Message(format!("{label}: the worker thread stopped without a result"))),
    }
}

/// The loop of [`wait_until`], with the clock and the pause passed in. `elapsed` returns the time
/// since the wait started.
fn poll_until(
    resolved: &Resolved<'_>,
    what: &str,
    cond: &mut dyn FnMut() -> bool,
    elapsed: &mut dyn FnMut() -> Duration,
    pause: &mut dyn FnMut(Duration),
) -> Result<(), String> {
    let mut next_pause = FIRST_PAUSE;
    let mut first = true;
    loop {
        // After the first call, the condition is called only while the limit has not passed.
        if !first && elapsed() >= resolved.limit {
            return Err(format!("timed out waiting for {what} within {resolved}"));
        }
        first = false;
        let held = cond();
        let spent = elapsed();
        if held && spent <= resolved.limit {
            return Ok(());
        }
        if held {
            return Err(format!("timed out waiting for {what} within {resolved} (it held only after the limit had passed)"));
        }
        if spent < resolved.limit {
            pause(next_pause.min(resolved.limit - spent));
        }
        next_pause = next_pause.saturating_mul(2).min(MAX_PAUSE);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsStr;
    use std::process::Command;
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

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
    #[should_panic(expected = "seed 7 byte 3 did not finish within 50ms")]
    fn a_hang_fails_and_names_the_input() {
        let (gate, gate_rx) = mpsc::channel::<()>();
        let (done_tx, done) = mpsc::channel();
        let result = catch_unwind(AssertUnwindSafe(|| {
            with_deadline(short(), "seed 7 byte 3", move || {
                let _ = gate_rx.recv();
                done_tx.send(()).unwrap();
            })
        }));
        drop(gate);
        done.recv().unwrap();
        if let Err(payload) = result {
            resume_unwind(payload);
        }
    }

    #[test]
    fn with_deadline_completes() {
        with_deadline(Duration::from_secs(5), "fast body", || {
            // fast operation
        });
    }

    #[test]
    #[should_panic(expected = "did not finish within")]
    fn with_deadline_times_out() {
        let (gate, gate_rx) = mpsc::channel::<()>();
        let result = catch_unwind(AssertUnwindSafe(|| with_deadline(short(), "blocked body", move || gate_rx.recv().unwrap_or_default())));
        drop(gate);
        if let Err(payload) = result {
            resume_unwind(payload);
        }
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
        let call = |label: &'static str| {
            thread::spawn(move || {
                let (gate, gate_rx) = mpsc::channel::<()>();
                let result = catch_unwind(AssertUnwindSafe(|| with_deadline(short(), label, move || gate_rx.recv().unwrap_or_default())));
                drop(gate);
                message(result.unwrap_err())
            })
        };
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
        let (gate, gate_rx) = mpsc::channel::<()>();
        let short_call = catch_unwind(AssertUnwindSafe(|| with_deadline(short(), "short call", move || gate_rx.recv().unwrap_or_default())));
        let text = message(short_call.unwrap_err());
        assert!(text.contains("short call") && text.contains("50ms") && !text.contains("long call"), "{text}");
        release.send(()).unwrap();
        assert_eq!(long_call.join().unwrap(), 7);
        drop(gate);
    }

    #[test]
    fn spawn_failure_reports_cause() {
        let ran = AtomicBool::new(false);
        let spawn = |_job: Job, _done: Sender<()>| {
            ran.store(true, Ordering::SeqCst);
            Err(io::Error::other("no threads left"))
        };
        let result = run_with_deadline(long(), "no thread", || (), |_| None, spawn, Arc::new(|| Duration::ZERO));
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
        let text = failure_message(run_with_deadline(huge, "huge", || (), |_| None, spawn, Arc::new(|| Duration::ZERO)));
        assert!(text.contains("ends past the latest time"), "{text}");
        assert!(!ran.load(Ordering::SeqCst), "the body started");
        let text = message(catch_unwind(|| wait_until(huge, "a huge limit", || true)).unwrap_err());
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
        let (asked, asked_rx) = mpsc::channel();
        let (stored, stored_rx) = mpsc::channel();
        thread::scope(|s| {
            s.spawn(move || {
                asked_rx.recv().unwrap();
                ready.store(true, Ordering::SeqCst);
                stored.send(()).unwrap();
            });
            let mut calls = 0;
            wait_until(long(), "the flag", || {
                calls += 1;
                if calls == 1 {
                    // The flag is set before this call returns, and it still answers false, so the
                    // condition is called a second time whatever order the threads run in.
                    asked.send(()).unwrap();
                    stored_rx.recv().unwrap();
                    return false;
                }
                ready.load(Ordering::SeqCst)
            });
            assert_eq!(calls, 2);
        });
    }

    #[test]
    #[should_panic(expected = "timed out waiting for the export to finish within 50ms")]
    fn wait_until_times_out_with_label() {
        wait_until(short(), "the export to finish", || false);
    }

    #[test]
    #[should_panic(expected = "condition boom")]
    fn wait_until_propagates_condition_panic() {
        wait_until(long(), "a panicking condition", || panic!("condition boom"));
    }

    #[test]
    fn wait_until_keeps_a_typed_condition_payload() {
        #[derive(Debug, PartialEq)]
        struct Marker(u32);
        let payload = catch_unwind(|| wait_until(long(), "a typed panic", || std::panic::panic_any(Marker(9)))).unwrap_err();
        assert_eq!(payload.downcast_ref::<Marker>(), Some(&Marker(9)));
    }

    /// A default limit for the loop tests, which pass their own clock.
    fn fake_resolved(limit: Duration) -> Resolved<'static> {
        Resolved { limit, variable: UNSET, from_variable: false }
    }

    #[test]
    fn wait_until_rejects_a_true_after_the_limit() {
        let resolved = fake_resolved(Duration::from_millis(10));
        let mut now = Duration::ZERO;
        let mut cond = || true;
        // The condition took 11 ms to return.
        let mut elapsed = || {
            now += Duration::from_millis(11);
            now
        };
        let result = poll_until(&resolved, "a late flag", &mut cond, &mut elapsed, &mut |_| panic!("paused"));
        let text = result.unwrap_err();
        assert!(text.contains("timed out waiting for a late flag") && text.contains("held only after the limit"), "{text}");
    }

    #[test]
    fn wait_until_pauses_grow_and_stop_at_the_limit() {
        let resolved = fake_resolved(Duration::from_millis(100));
        let now = std::cell::Cell::new(Duration::ZERO);
        let mut pauses = Vec::new();
        let mut calls = 0;
        let mut cond = || {
            calls += 1;
            false
        };
        let mut elapsed = || now.get();
        let mut pause = |d: Duration| {
            pauses.push(d);
            now.set(now.get() + d);
        };
        let result = poll_until(&resolved, "never", &mut cond, &mut elapsed, &mut pause);
        assert!(result.is_err());
        let ms: Vec<u128> = pauses.iter().map(Duration::as_millis).collect();
        // 1 + 2 + 4 + 8 + 16 + 32 = 63, then the 37 ms left, after which the limit is reached.
        assert_eq!(ms, [1, 2, 4, 8, 16, 32, 37]);
        // One call before each pause, and none after the last one.
        assert_eq!(calls, 7);
    }

    #[test]
    fn wait_until_stops_calling_after_an_overshooting_pause() {
        let resolved = fake_resolved(Duration::from_millis(100));
        let now = std::cell::Cell::new(Duration::from_millis(99));
        let mut calls = 0;
        let mut cond = || {
            calls += 1;
            false
        };
        let mut elapsed = || now.get();
        // The thread wakes 50 ms late from the 1 ms pause.
        let mut pause = |d: Duration| now.set(now.get() + d + Duration::from_millis(50));
        let result = poll_until(&resolved, "late wake", &mut cond, &mut elapsed, &mut pause);
        assert!(result.unwrap_err().contains("timed out waiting for late wake"));
        assert_eq!(calls, 1);
    }

    /// Runs `run_with_deadline` with a 5 s limit on a fake clock. The worker runs on the calling
    /// thread: the clock reads `start_secs` while the body runs and `received_secs` when the result
    /// is received. When `sends` is false, the worker stores its result but does not send, as if its
    /// thread stopped running between the two.
    fn on_fake_clock<T: Send + 'static>(
        f: impl FnOnce() -> T + Send + 'static,
        start_secs: u64,
        received_secs: u64,
        sends: bool,
    ) -> Result<T, Failure> {
        let now = Arc::new(AtomicU64::new(0));
        let clock_now = Arc::clone(&now);
        let clock: Clock = Arc::new(move || Duration::from_secs(clock_now.load(Ordering::SeqCst)));
        let unsent = std::cell::RefCell::new(None);
        let spawn = |job: Job, done: Sender<()>| {
            now.store(start_secs, Ordering::SeqCst);
            job();
            now.store(received_secs, Ordering::SeqCst);
            if sends {
                let _ = done.send(());
            } else {
                // Kept alive so that the wait times out instead of seeing a closed channel.
                *unsent.borrow_mut() = Some(done);
            }
            Ok(())
        };
        let limit = DeadlineLimit::new(Duration::from_secs(5)).with_variable(UNSET);
        run_with_deadline(limit, "fake clock", f, |_| None, spawn, clock)
    }

    #[test]
    fn a_value_returned_after_the_limit_fails() {
        let text = failure_message(on_fake_clock(|| 1, 6, 6, true));
        assert!(text.contains("fake clock finished only after its limit of 5s"), "{text}");
    }

    #[test]
    fn a_value_finished_in_time_but_received_late_passes() {
        assert!(matches!(on_fake_clock(|| 1, 1, 6, true), Ok(1)));
    }

    #[test]
    fn a_value_finished_in_time_but_sent_late_passes() {
        assert!(matches!(on_fake_clock(|| 1, 1, 6, false), Ok(1)));
    }

    #[test]
    fn a_panic_finished_in_time_but_sent_late_keeps_its_payload() {
        let payload = match on_fake_clock(|| -> u8 { panic!("early boom") }, 1, 6, false) {
            Err(Failure::Panic(payload)) => payload,
            Ok(_) => panic!("the call did not fail"),
            Err(Failure::Message(text)) => panic!("the payload was replaced: {text}"),
        };
        assert_eq!(message(payload), "early boom");
    }

    #[test]
    fn a_panic_after_the_limit_names_the_limit_and_the_panic() {
        let text = failure_message(on_fake_clock(|| -> u8 { panic!("late boom") }, 6, 6, true));
        assert!(text.contains("fake clock finished only after its limit of 5s") && text.ends_with("(it panicked: late boom)"), "{text}");
    }
}
