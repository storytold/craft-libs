//! Hostile inputs made from a valid seed, for tests that a parser fails cleanly on damaged data.
//!
//! [`mutations`] yields a fixed sequence of damaged copies of a seed: each round applies one to
//! four edits (a bit flip, a random byte, a 1, 2, 4 or 8-byte overwrite with an extreme value in
//! either byte order, a deletion of up to 16 bytes, or a truncation) and labels the result with
//! the edits it made. The sequence depends only on the seed's length and the number of rounds, so
//! a failing case can be found again from its label. It builds for every target, `wasm32`
//! included.
//!
//! [`hammer`] runs a parser on the unmodified seed, on every truncation of it and on the
//! mutations, one case after another on a worker thread, with a time limit for each case. It fails
//! the test, naming the case and the input length, when the parser panics or does not finish in
//! time. The number of rounds is a count the caller gives, multiplied by a variable the caller
//! names. [`DIVISOR_VARIABLE`] (`CRAFT_HOSTILE_DIVISOR`) runs a fraction of every sweep for quick
//! runs such as pull requests: every N-th truncation and 1/N of the rounds; [`divisor`] reads it
//! for sweeps written without [`hammer`]. [`hammer`], [`HammerConfig`] and [`divisor`] are not
//! built for `wasm32`.

use std::fmt::Write as _;

#[cfg(not(target_arch = "wasm32"))]
use crate::deadline::{DeadlineLimit, Resolved, resolve};
#[cfg(not(target_arch = "wasm32"))]
use std::any::Any;
#[cfg(not(target_arch = "wasm32"))]
use std::ffi::OsString;
#[cfg(not(target_arch = "wasm32"))]
use std::fmt;
#[cfg(not(target_arch = "wasm32"))]
use std::panic::{AssertUnwindSafe, catch_unwind};
#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Condvar, Mutex, MutexGuard, PoisonError};
#[cfg(not(target_arch = "wasm32"))]
use std::thread;
#[cfg(not(target_arch = "wasm32"))]
use std::time::{Duration, Instant};

/// The name of the thread [`hammer`] runs its cases on.
#[cfg(not(target_arch = "wasm32"))]
const WORKER_NAME: &str = "craft-testkit hostile";

/// The variable that multiplies a [`HammerConfig`]'s rounds unless
/// [`HammerConfig::with_multiplier_variable`] names another.
pub const DEFAULT_MULTIPLIER_VARIABLE: &str = "CRAFT_HOSTILE_MULT";

/// The variable that divides every hostile-input sweep: with `N`, [`hammer`] runs the unmodified
/// seed, every N-th truncation (lengths 0, N, 2N, ...) and 1/N of its rounds, rounded up. Unset, or
/// 1, runs everything. The Workspace gate sets it and the nightly full run leaves it unset; a
/// failing case is found again with the same value.
pub const DIVISOR_VARIABLE: &str = "CRAFT_HOSTILE_DIVISOR";

/// The default limit for one case of [`hammer`]. The limit's variable, `CRAFT_TEST_TIMEOUT_SECS`
/// unless [`HammerConfig::with_timeout`] names another, replaces it.
#[cfg(not(target_arch = "wasm32"))]
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

/// The most cases one [`hammer`] call runs: the unmodified seed, its truncations and the rounds
/// together. A call that would run more fails before it runs any.
pub const MAX_CASES: usize = 10_000_000;

/// The values the overwrite edits write, in 1, 2, 4 or 8 bytes.
const EXTREMES: [u64; 8] = [0, 1, 0x7F, 0x80, 0xFF, 0xFFFF, 0x8000_0000, u64::MAX];

/// The xorshift64* generator behind [`mutations`].
///
/// ```
/// use craft_testkit::hostile::Rng;
///
/// let mut rng = Rng(1);
/// assert_eq!(rng.next_u64(), 0x47E4_CE4B_896C_DD1D);
/// ```
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rng(pub u64);

impl Rng {
    /// The next value.
    pub fn next_u64(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    /// A value below `n`, or 0 when `n` is 0.
    pub fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n.max(1) as u64) as usize
    }
}

/// One damaged copy of a seed, from [`mutations`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mutation {
    /// The damaged bytes.
    pub bytes: Vec<u8>,
    /// The edits made, in order, separated by spaces, such as `flip[3].5 del[7..9]`.
    pub label: String,
}

/// `rounds` damaged copies of `seed`, in a sequence fixed by the seed's length and `rounds`.
///
/// An empty seed yields no copies, since there is no byte to edit.
///
/// ```
/// use craft_testkit::hostile::mutations;
///
/// let all: Vec<_> = mutations(&[0, 1, 2, 3], 2).collect();
/// assert_eq!(all[1].label, "trunc2");
/// assert_eq!(all[1].bytes, [0, 1]);
/// ```
pub fn mutations(seed: &[u8], rounds: usize) -> Mutations<'_> {
    Mutations { seed, rng: Rng((0x5EED_F00D ^ seed.len() as u64) | 1), remaining: if seed.is_empty() { 0 } else { rounds } }
}

/// The iterator [`mutations`] returns.
#[derive(Clone, Debug)]
pub struct Mutations<'a> {
    seed: &'a [u8],
    rng: Rng,
    remaining: usize,
}

impl Iterator for Mutations<'_> {
    type Item = Mutation;

    fn next(&mut self) -> Option<Mutation> {
        if self.remaining == 0 {
            return None;
        }
        self.remaining -= 1;
        let rng = &mut self.rng;
        let mut d = self.seed.to_vec();
        let mut label = String::new();
        for _ in 0..1 + rng.below(4) {
            if d.is_empty() {
                break;
            }
            if !label.is_empty() {
                label.push(' ');
            }
            let pos = rng.below(d.len());
            match rng.below(7) {
                0 => {
                    let b = rng.below(8);
                    if let Some(byte) = d.get_mut(pos) {
                        *byte ^= 1 << b;
                    }
                    let _ = write!(label, "flip[{pos}].{b}");
                }
                1 => {
                    let value = rng.next_u64() as u8;
                    if let Some(byte) = d.get_mut(pos) {
                        *byte = value;
                    }
                    let _ = write!(label, "set[{pos}]={value:#x}");
                }
                2..=4 => {
                    let w = [1usize, 2, 4, 8][rng.below(4)];
                    let v = EXTREMES[rng.below(EXTREMES.len())];
                    let be = rng.below(2) == 0;
                    let all = if be { v.to_be_bytes() } else { v.to_le_bytes() };
                    let bytes = if be { &all[8 - w..] } else { &all[..w] };
                    let end = (pos + w).min(d.len());
                    if let (Some(target), Some(source)) = (d.get_mut(pos..end), bytes.get(..end - pos)) {
                        target.copy_from_slice(source);
                    }
                    let _ = write!(label, "put{w}{}[{pos}]={v:#x}", if be { "be" } else { "le" });
                }
                5 => {
                    let n = 1 + rng.below(16);
                    let end = (pos + n).min(d.len());
                    d.drain(pos..end);
                    let _ = write!(label, "del[{pos}..{end}]");
                }
                _ => {
                    d.truncate(pos);
                    let _ = write!(label, "trunc{pos}");
                }
            }
        }
        Some(Mutation { bytes: d, label })
    }

    fn size_hint(&self) -> (usize, Option<usize>) {
        (self.remaining, Some(self.remaining))
    }
}

impl ExactSizeIterator for Mutations<'_> {}

/// The settings of one [`hammer`] call: a label for its failures, the number of rounds, the
/// variable that multiplies them, and the limit for each case.
///
/// ```
/// use std::time::Duration;
/// use craft_testkit::hostile::HammerConfig;
///
/// // 2,000 rounds, times CRAFT_HOSTILE_MULT when it is set; 5 s per case, or CRAFT_TEST_TIMEOUT_SECS.
/// let config = HammerConfig::new("png seed 3", 2_000);
///
/// // The app's own variables instead.
/// let config = HammerConfig::new("mxf primer", 5_000)
///     .with_multiplier_variable("CRAFT_MEDIA_HOSTILE_MULT")
///     .with_timeout(craft_testkit::DeadlineLimit::new(Duration::from_secs(5)).with_variable("FILMCRAFT_HOSTILE_TIMEOUT_SECS"));
/// assert_eq!(config.multiplier_variable(), "CRAFT_MEDIA_HOSTILE_MULT");
/// ```
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HammerConfig<'a> {
    label: &'a str,
    iterations: usize,
    multiplier_variable: &'a str,
    timeout: DeadlineLimit<'a>,
}

#[cfg(not(target_arch = "wasm32"))]
impl<'a> HammerConfig<'a> {
    /// `iterations` rounds of mutations under `label`, multiplied by [`DEFAULT_MULTIPLIER_VARIABLE`],
    /// with a limit of [`DEFAULT_TIMEOUT`] for each case.
    pub fn new(label: &'a str, iterations: usize) -> Self {
        Self { label, iterations, multiplier_variable: DEFAULT_MULTIPLIER_VARIABLE, timeout: DeadlineLimit::new(DEFAULT_TIMEOUT) }
    }

    /// The same settings, with the rounds multiplied by the variable `name` instead.
    pub fn with_multiplier_variable(self, name: &'a str) -> Self {
        Self { multiplier_variable: name, ..self }
    }

    /// The same settings, with `limit` for each case.
    pub fn with_timeout(self, limit: impl Into<DeadlineLimit<'a>>) -> Self {
        Self { timeout: limit.into(), ..self }
    }

    /// The label that failures start with.
    pub fn label(&self) -> &'a str {
        self.label
    }

    /// The rounds before the multiplier.
    pub fn iterations(&self) -> usize {
        self.iterations
    }

    /// The name of the variable that multiplies the rounds.
    pub fn multiplier_variable(&self) -> &'a str {
        self.multiplier_variable
    }

    /// The limit for each case.
    pub fn timeout(&self) -> DeadlineLimit<'a> {
        self.timeout
    }

    /// The rounds after reading the multiplier variable, or why they cannot be used.
    ///
    /// When the variable is not set, the multiplier is 1. A value must be a whole number above
    /// zero written in decimal digits.
    pub fn rounds(&self) -> Result<usize, HammerError> {
        self.rounds_with(read_variable)
    }

    /// [`Self::rounds`] with the variable lookup passed in.
    fn rounds_with(&self, lookup: impl FnOnce(&str) -> Option<OsString>) -> Result<usize, HammerError> {
        let variable = self.multiplier_variable.to_owned();
        if variable.is_empty() || variable.contains(['=', '\0']) {
            return Err(HammerError::BadName { variable });
        }
        let multiplier = match lookup(&variable) {
            None => 1,
            Some(raw) => {
                let value = raw.into_string().map_err(|_| HammerError::NotUnicode { variable: variable.clone() })?;
                parse_multiplier(&value).map_err(|reason| HammerError::BadValue { variable: variable.clone(), value, reason })?
            }
        };
        self.iterations.checked_mul(multiplier).ok_or(HammerError::Overflow { iterations: self.iterations, variable, multiplier })
    }
}

/// Why a [`hammer`] call cannot run.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HammerError {
    /// The multiplier variable's name is empty or holds `=` or a NUL character.
    BadName {
        /// The name.
        variable: String,
    },
    /// The multiplier variable's value is not Unicode.
    NotUnicode {
        /// The variable's name.
        variable: String,
    },
    /// The multiplier variable's value is not a whole number above zero.
    BadValue {
        /// The variable's name.
        variable: String,
        /// The value.
        value: String,
        /// What is wrong with the value.
        reason: &'static str,
    },
    /// The rounds times the multiplier is more than a `usize` holds.
    Overflow {
        /// The rounds before the multiplier.
        iterations: usize,
        /// The variable's name.
        variable: String,
        /// The multiplier.
        multiplier: usize,
    },
    /// The call would run more than [`MAX_CASES`] cases.
    TooManyCases {
        /// The cases the call would run.
        cases: u128,
    },
}

#[cfg(not(target_arch = "wasm32"))]
impl fmt::Display for HammerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BadName { variable } => write!(f, "{variable:?} cannot name a variable: it is empty or holds `=` or a NUL character"),
            Self::NotUnicode { variable } => write!(f, "{variable} is not valid Unicode; set it to a whole number above zero"),
            Self::BadValue { variable, value, reason } => write!(f, "{variable}={value:?} {reason}; set it to a whole number above zero"),
            Self::Overflow { iterations, variable, multiplier } => {
                write!(f, "{iterations} rounds times {variable}={multiplier} is more than a usize holds")
            }
            Self::TooManyCases { cases } => write!(f, "{cases} cases is more than the limit of {MAX_CASES}"),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl std::error::Error for HammerError {}

/// Parses a whole number above zero, written as decimal digits only.
#[cfg(not(target_arch = "wasm32"))]
fn parse_multiplier(value: &str) -> Result<usize, &'static str> {
    if value.is_empty() {
        return Err("is empty");
    }
    if !value.bytes().all(|b| b.is_ascii_digit()) {
        return Err("is not a whole number written in digits");
    }
    match value.parse::<usize>() {
        Ok(0) => Err("is zero"),
        Ok(n) => Ok(n),
        Err(_) => Err("is more than a usize holds"),
    }
}

/// The value of [`DIVISOR_VARIABLE`], or 1 when it is not set, for a sweep written without
/// [`hammer`]: run the cases whose index plus an offset is a multiple of it, and give each input of
/// the sweep a different offset so that together they still reach every index. Panics, naming the
/// variable, when the value is not a whole number above zero.
///
/// ```
/// let step = craft_testkit::hostile::divisor();
/// let positions: Vec<usize> = (0..64).filter(|i| (i + 3) % step == 0).collect();
/// assert!(!positions.is_empty());
/// ```
// Failing the calling test is what this function is for.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::panic)]
#[track_caller]
pub fn divisor() -> usize {
    match divisor_with(read_variable) {
        Ok(divisor) => divisor,
        Err(error) => panic!("{error}"),
    }
}

/// [`divisor`] with the variable lookup passed in.
#[cfg(not(target_arch = "wasm32"))]
fn divisor_with(lookup: impl FnOnce(&str) -> Option<OsString>) -> Result<usize, HammerError> {
    let Some(raw) = lookup(DIVISOR_VARIABLE) else { return Ok(1) };
    let variable = DIVISOR_VARIABLE.to_owned();
    let value = raw.into_string().map_err(|_| HammerError::NotUnicode { variable: variable.clone() })?;
    parse_multiplier(&value).map_err(|reason| HammerError::BadValue { variable, value, reason })
}

/// Reads the variable `name` from the process environment.
#[cfg(not(target_arch = "wasm32"))]
fn read_variable(name: &str) -> Option<OsString> {
    std::env::var_os(name)
}

/// Runs `parse` on `seed`, on every truncation of it (lengths 0 to one less than its length), and
/// on [`mutations`] of it for the config's rounds, failing the calling test on the first case
/// that panics or does not finish within the config's limit.
///
/// The cases run one after another on one worker thread, while the calling thread fails the test
/// when a case runs past the config's limit. A case that finishes after the limit also fails it. A
/// failure starts with the config's label and names the case (`the unmodified seed`,
/// `truncation to N` or the mutation's edits), the input's length and the panic's message. A case
/// that does not finish fails the test while the worker thread is left running, so `parse` must be
/// owned and able to move to another thread. The multiplier and limit variables are read before
/// any case runs; a value that cannot be used, or a call that would run more than [`MAX_CASES`]
/// cases, fails the test before `parse` is called.
///
/// ```
/// use craft_testkit::hostile::{HammerConfig, hammer};
///
/// hammer(&HammerConfig::new("length prefix", 200), &[3, b'a', b'b', b'c'], |data| {
///     if let Some((&n, rest)) = data.split_first() {
///         let _ = rest.get(..usize::from(n));
///     }
/// });
/// ```
// Failing the calling test is what this function is for.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::panic)]
#[track_caller]
pub fn hammer<F>(config: &HammerConfig<'_>, seed: &[u8], parse: F)
where
    F: Fn(&[u8]) + Send + Sync + 'static,
{
    if let Err(message) = hammer_with(config, seed, parse, read_variable) {
        panic!("{message}");
    }
}

/// [`hammer`] with the variable lookup passed in, returning the failure's message.
#[cfg(not(target_arch = "wasm32"))]
fn hammer_with<F>(config: &HammerConfig<'_>, seed: &[u8], parse: F, lookup: impl Fn(&str) -> Option<OsString>) -> Result<(), String>
where
    F: Fn(&[u8]) + Send + Sync + 'static,
{
    let label = config.label;
    let rounds = config.rounds_with(&lookup).map_err(|error| format!("{label}: {error}"))?;
    let divisor = divisor_with(&lookup).map_err(|error| format!("{label}: {error}"))?;
    let rounds = if seed.is_empty() { 0 } else { rounds.div_ceil(divisor) };
    let cases = 1 + seed.len().div_ceil(divisor) as u128 + rounds as u128;
    if cases > MAX_CASES as u128 {
        return Err(format!("{label}: {}", HammerError::TooManyCases { cases }));
    }
    let start = Instant::now();
    let limit = resolve(&config.timeout, &lookup, start).map(Limit::from).map_err(|error| format!("{label}: {error}"))?;
    let clock: Clock = Arc::new(move || start.elapsed());
    run_cases(label.to_owned(), limit, Arc::from(seed), rounds, divisor, parse, clock)
}

/// A clock that returns the time since a [`hammer`] call started.
#[cfg(not(target_arch = "wasm32"))]
type Clock = Arc<dyn Fn() -> Duration + Send + Sync>;

/// The limit for each case, and how failures describe it.
#[cfg(not(target_arch = "wasm32"))]
#[derive(Clone, Debug)]
struct Limit {
    limit: Duration,
    text: String,
}

#[cfg(not(target_arch = "wasm32"))]
impl From<Resolved<'_>> for Limit {
    fn from(resolved: Resolved<'_>) -> Self {
        Self { limit: resolved.limit, text: resolved.to_string() }
    }
}

/// The case the worker is running, shared with the calling thread that watches it.
#[cfg(not(target_arch = "wasm32"))]
struct Progress {
    /// The case's description: `the unmodified seed`, `truncation to N` or the mutation's edits.
    case: String,
    /// The length of the case's input.
    len: usize,
    /// The clock reading when the case started.
    started: Duration,
    /// Whether the case is still running.
    running: bool,
    /// Set when the worker stops: `Ok` after the last case, or the failure's message.
    outcome: Option<Result<(), String>>,
}

/// [`Progress`] under a lock, with a signal for when the worker stops.
#[cfg(not(target_arch = "wasm32"))]
struct Watch {
    state: Mutex<Progress>,
    stopped: Condvar,
}

#[cfg(not(target_arch = "wasm32"))]
impl Watch {
    fn lock(&self) -> MutexGuard<'_, Progress> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Hands the worker's outcome to the calling thread when the worker stops, including when it
/// unwinds outside a case.
#[cfg(not(target_arch = "wasm32"))]
struct Report {
    watch: Arc<Watch>,
    outcome: Result<(), String>,
}

#[cfg(not(target_arch = "wasm32"))]
impl Drop for Report {
    fn drop(&mut self) {
        let mut state = self.watch.lock();
        state.outcome = Some(std::mem::replace(&mut self.outcome, Ok(())));
        self.watch.stopped.notify_all();
    }
}

/// Starts the worker that runs every case, then waits for it, failing when a case runs past the
/// limit.
#[cfg(not(target_arch = "wasm32"))]
fn run_cases<F>(label: String, limit: Limit, seed: Arc<[u8]>, rounds: usize, divisor: usize, parse: F, clock: Clock) -> Result<(), String>
where
    F: Fn(&[u8]) + Send + Sync + 'static,
{
    // No case counts as running until the worker starts it, so a worker that starts late is not
    // reported as a case past its limit.
    let first = Progress { case: "the unmodified seed".to_owned(), len: seed.len(), started: clock(), running: false, outcome: None };
    let watch = Arc::new(Watch { state: Mutex::new(first), stopped: Condvar::new() });
    let (worker_watch, worker_clock, worker_label, worker_limit) = (Arc::clone(&watch), Arc::clone(&clock), label.clone(), limit.clone());
    thread::Builder::new()
        .name(WORKER_NAME.to_owned())
        .spawn(move || {
            let mut report = Report { watch: worker_watch, outcome: Err(format!("{worker_label}: the worker thread stopped without a result")) };
            report.outcome = run_all(&report.watch, &worker_clock, &worker_label, &worker_limit, &seed, rounds, divisor, &parse);
        })
        .map_err(|error| format!("{label}: could not start the worker thread: {error}"))?;
    let mut state = watch.lock();
    loop {
        if let Some(outcome) = state.outcome.take() {
            return outcome;
        }
        let deadline = state.started.saturating_add(limit.limit);
        let now = clock();
        if state.running && now >= deadline {
            return Err(format!("{label}: {} (len {}) did not finish within {}; its thread is left running", state.case, state.len, limit.text));
        }
        let wait = if state.running { deadline.saturating_sub(now) } else { limit.limit };
        state = watch.stopped.wait_timeout(state, wait).unwrap_or_else(PoisonError::into_inner).0;
    }
}

/// Runs the unmodified seed, every `divisor`-th truncation and its mutations in order, stopping at
/// the first failure.
#[cfg(not(target_arch = "wasm32"))]
#[allow(clippy::too_many_arguments)]
fn run_all<F: Fn(&[u8])>(
    watch: &Watch,
    clock: &Clock,
    label: &str,
    limit: &Limit,
    seed: &[u8],
    rounds: usize,
    divisor: usize,
    parse: &F,
) -> Result<(), String> {
    for n in std::iter::once(seed.len()).chain((0..seed.len()).step_by(divisor)) {
        let case = if n == seed.len() { "the unmodified seed".to_owned() } else { format!("truncation to {n}") };
        run_one(watch, clock, label, limit, case, seed.get(..n).unwrap_or_default(), parse)?;
    }
    for mutation in mutations(seed, rounds) {
        run_one(watch, clock, label, limit, mutation.label, &mutation.bytes, parse)?;
    }
    Ok(())
}

/// Runs one case, recording it in `watch` while it runs, and returns the failure's message when it
/// panics or finishes after the limit.
#[cfg(not(target_arch = "wasm32"))]
fn run_one<F: Fn(&[u8])>(watch: &Watch, clock: &Clock, label: &str, limit: &Limit, case: String, data: &[u8], parse: &F) -> Result<(), String> {
    let started = clock();
    {
        let mut state = watch.lock();
        (state.case, state.len, state.started, state.running) = (case, data.len(), started, true);
    }
    let outcome = catch_unwind(AssertUnwindSafe(|| parse(data)));
    let spent = clock().saturating_sub(started);
    // The payload is dropped before the lock is taken and while the case still counts as running, so
    // the watchdog still reports a payload whose drop does not return.
    let panicked = outcome.err().map(|payload| panic_text(payload.as_ref()));
    let mut state = watch.lock();
    state.running = false;
    let name = || format!("{label}: {} (len {})", state.case, state.len);
    if spent > limit.limit {
        let panicked = panicked.map_or_else(String::new, |text| format!(" (it panicked: {text})"));
        return Err(format!("{} finished only after its limit of {}{panicked}", name(), limit.text));
    }
    panicked.map_or(Ok(()), |text| Err(format!("{}: the parser panicked: {text}", name())))
}

/// The text of a panic payload.
#[cfg(not(target_arch = "wasm32"))]
fn panic_text(payload: &(dyn Any + Send)) -> String {
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|text| (*text).to_owned()))
        .unwrap_or_else(|| "a payload that is not a message".to_owned())
}

#[cfg(test)]
#[cfg(not(target_arch = "wasm32"))]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::sync::Mutex;
    use std::sync::mpsc;

    /// A variable no test sets, so the environment cannot change a test's rounds or limits.
    const UNSET: &str = "CRAFT_TESTKIT_HOSTILE_UNSET_IN_TESTS";

    fn config(label: &'static str, iterations: usize) -> HammerConfig<'static> {
        HammerConfig::new(label, iterations)
            .with_multiplier_variable(UNSET)
            .with_timeout(DeadlineLimit::new(Duration::from_secs(600)).with_variable(UNSET))
    }

    fn unset(_: &str) -> Option<OsString> {
        None
    }

    /// Runs `hammer_with` with no variables set and returns the inputs `parse` saw, in order.
    fn inputs(config: &HammerConfig<'_>, seed: &[u8]) -> Vec<Vec<u8>> {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        hammer_with(config, seed, move |d| record.lock().unwrap().push(d.to_vec()), unset).unwrap();
        seen.lock().unwrap().clone()
    }

    #[test]
    fn reports_truncation_panic_with_case_and_payload() {
        let result = hammer_with(&config("demo seed 2", 0), &[9, 8, 7, 6], |d| assert!(d.len() != 2, "boom at {}", d.len()), unset);
        let message = result.unwrap_err();
        assert!(message.starts_with("demo seed 2: truncation to 2 (len 2): the parser panicked: "), "{message}");
        assert!(message.contains("boom at 2"), "{message}");
    }

    #[test]
    fn a_panic_without_a_message_is_reported() {
        let message = hammer_with(&config("demo", 0), &[1], |_| std::panic::panic_any(7u8), unset).unwrap_err();
        assert_eq!(message, "demo: the unmodified seed (len 1): the parser panicked: a payload that is not a message");
    }

    #[test]
    fn deadline_names_stuck_case() {
        let (release, wait) = mpsc::channel::<()>();
        let wait = Mutex::new(wait);
        let short = config("stuck demo", 0).with_timeout(DeadlineLimit::new(Duration::from_millis(50)).with_variable(UNSET));
        let result = hammer_with(
            &short,
            &[1, 2, 3],
            move |d| {
                if d.len() == 3 {
                    let _ = wait.lock().unwrap().recv();
                }
            },
            unset,
        );
        release.send(()).unwrap();
        let message = result.unwrap_err();
        assert!(message.contains("stuck demo: the unmodified seed (len 3) did not finish within 50ms"), "{message}");
    }

    #[test]
    fn a_case_that_finishes_after_the_limit_fails() {
        let readings = Arc::new(Mutex::new(vec![Duration::from_secs(6), Duration::ZERO]));
        let clock: Clock = Arc::new(move || readings.lock().unwrap().pop().unwrap_or(Duration::MAX));
        let watch = Watch {
            state: Mutex::new(Progress { case: String::new(), len: 0, started: Duration::ZERO, running: true, outcome: None }),
            stopped: Condvar::new(),
        };
        let limit = Limit { limit: Duration::from_secs(5), text: "5s".to_owned() };
        let message = run_one(&watch, &clock, "late", &limit, "the unmodified seed".to_owned(), &[1], &|_: &[u8]| {}).unwrap_err();
        assert_eq!(message, "late: the unmodified seed (len 1) finished only after its limit of 5s");
        assert!(!watch.lock().running);
    }

    /// A panic payload that records, when it is dropped, whether the progress lock was free and
    /// whether the case still counted as running.
    struct DropProbe {
        watch: Arc<Watch>,
        seen: Arc<Mutex<Option<Option<bool>>>>,
    }

    impl Drop for DropProbe {
        fn drop(&mut self) {
            let running = self.watch.state.try_lock().ok().map(|state| state.running);
            *self.seen.lock().unwrap() = Some(running);
        }
    }

    #[test]
    fn a_panic_payload_is_dropped_outside_the_lock_while_the_case_runs() {
        let watch = Arc::new(Watch {
            state: Mutex::new(Progress { case: String::new(), len: 0, started: Duration::ZERO, running: false, outcome: None }),
            stopped: Condvar::new(),
        });
        let seen = Arc::new(Mutex::new(None));
        let clock: Clock = Arc::new(|| Duration::ZERO);
        let limit = Limit { limit: Duration::from_secs(5), text: "5s".to_owned() };
        let probe = (Arc::clone(&watch), Arc::clone(&seen));
        let parse = move |_: &[u8]| std::panic::panic_any(DropProbe { watch: Arc::clone(&probe.0), seen: Arc::clone(&probe.1) });
        let message = run_one(&watch, &clock, "probe", &limit, "the unmodified seed".to_owned(), &[1], &parse).unwrap_err();
        assert_eq!(message, "probe: the unmodified seed (len 1): the parser panicked: a payload that is not a message");
        assert_eq!(*seen.lock().unwrap(), Some(Some(true)), "the payload was dropped with the lock held or the case stopped");
        assert!(!watch.lock().running);
    }

    #[test]
    fn a_worker_that_starts_late_is_not_a_case_past_its_limit() {
        // The worker's first clock reading waits until the calling thread has checked the limit once,
        // and that check reads an hour after the start; every other reading is zero.
        let (checked, worker_waits) = mpsc::channel::<()>();
        let (checked, worker_waits) = (Mutex::new(Some(checked)), Mutex::new(worker_waits));
        let caller_reads = Mutex::new(0usize);
        let clock: Clock = Arc::new(move || {
            if thread::current().name() == Some(WORKER_NAME) {
                let _ = worker_waits.lock().unwrap().recv();
                return Duration::ZERO;
            }
            let mut reads = caller_reads.lock().unwrap();
            *reads += 1;
            if *reads == 2 {
                drop(checked.lock().unwrap().take());
                return Duration::from_secs(3600);
            }
            Duration::ZERO
        });
        let limit = Limit { limit: Duration::from_secs(5), text: "5s".to_owned() };
        assert_eq!(run_cases("late start".to_owned(), limit, Arc::from(&[1u8][..]), 0, 1, |_: &[u8]| {}, clock), Ok(()));
    }

    #[test]
    fn every_case_runs_on_one_worker_thread() {
        let threads = Arc::new(Mutex::new(std::collections::BTreeSet::new()));
        let record = Arc::clone(&threads);
        let parse = move |_: &[u8]| {
            record.lock().unwrap().insert(format!("{:?}", thread::current().id()));
        };
        hammer_with(&config("one thread", 50), &[1, 2, 3, 4], parse, unset).unwrap();
        let threads = threads.lock().unwrap();
        assert_eq!(threads.len(), 1, "{threads:?}");
        assert!(!threads.contains(&format!("{:?}", thread::current().id())));
    }

    #[test]
    fn invalid_limit_variable_fails_before_any_parse() {
        let called = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&called);
        let config = config("bad limit", 3).with_timeout(DeadlineLimit::new(Duration::from_secs(5)).with_variable("MY_LIMIT"));
        let lookup = |name: &str| (name == "MY_LIMIT").then(|| OsString::from("soon"));
        let message = hammer_with(&config, &[1, 2], move |_| *flag.lock().unwrap() = true, lookup).unwrap_err();
        assert_eq!(
            message,
            r#"bad limit: MY_LIMIT="soon" is not a whole number of seconds written in digits; set it to a whole number of seconds above zero"#
        );
        assert!(!*called.lock().unwrap());
    }

    #[test]
    fn mutation_sequence_matches_source() {
        let all: Vec<_> = mutations(&[0, 1, 2, 3], 4).collect();
        let bytes: Vec<&[u8]> = all.iter().map(|m| m.bytes.as_slice()).collect();
        let labels: Vec<&str> = all.iter().map(|m| m.label.as_str()).collect();
        assert_eq!(bytes, [&[0u8, 1, 2, 0][..], &[0, 1][..], &[0, 1, 0, 0][..], &[0, 0, 0, 1][..]]);
        assert_eq!(labels, ["put1be[3]=0x0", "trunc2", "put4le[1]=0x1", "put4be[0]=0x1"]);
        let mut expected: Vec<Vec<u8>> = vec![vec![0, 1, 2, 3], vec![], vec![0], vec![0, 1], vec![0, 1, 2]];
        expected.extend(all.into_iter().map(|m| m.bytes));
        assert_eq!(inputs(&config("sequence", 4), &[0, 1, 2, 3]), expected);
    }

    #[test]
    fn mutation_labels_join_edits_with_spaces() {
        let seed: Vec<u8> = (0..64).collect();
        let joined = mutations(&seed, 200).find(|m| m.label.contains(' ')).unwrap();
        assert!(!joined.label.starts_with(' ') && !joined.label.ends_with(' ') && !joined.label.contains("  "), "{:?}", joined.label);
    }

    #[test]
    fn uses_caller_multiplier_variable() {
        let config = config("mult", 7).with_multiplier_variable("CRAFT_MEDIA_HOSTILE_MULT");
        let lookup = |name: &str| match name {
            "CRAFT_MEDIA_HOSTILE_MULT" => Some(OsString::from("3")),
            DEFAULT_MULTIPLIER_VARIABLE => Some(OsString::from("100")),
            _ => None,
        };
        assert_eq!(config.rounds_with(lookup), Ok(21));
        let calls = Arc::new(Mutex::new(0usize));
        let count = Arc::clone(&calls);
        hammer_with(&config, &[5, 6], move |_| *count.lock().unwrap() += 1, lookup).unwrap();
        // The seed, two truncations and 21 rounds.
        assert_eq!(*calls.lock().unwrap(), 24);
        assert_eq!(HammerConfig::new("default", 7).multiplier_variable(), "CRAFT_HOSTILE_MULT");
        assert_eq!(config.rounds_with(unset), Ok(7));
    }

    #[test]
    fn divisor_runs_every_nth_truncation_and_a_fraction_of_the_rounds() {
        let lookup = |name: &str| (name == DIVISOR_VARIABLE).then(|| OsString::from("4"));
        let seen = Arc::new(Mutex::new(Vec::new()));
        let record = Arc::clone(&seen);
        hammer_with(&config("divided", 10), &[0; 10], move |d| record.lock().unwrap().push(d.len()), lookup).unwrap();
        let lengths = seen.lock().unwrap().clone();
        // The seed, truncations to 0, 4 and 8, then 10 / 4 = 3 rounds rounded up.
        assert_eq!(lengths.len(), 7, "{lengths:?}");
        assert_eq!(lengths[..4], [10, 0, 4, 8]);
        assert_eq!(divisor_with(unset), Ok(1));
    }

    #[test]
    fn invalid_divisor_fails_before_any_parse() {
        let called = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&called);
        let lookup = |name: &str| (name == DIVISOR_VARIABLE).then(|| OsString::from("0"));
        let message = hammer_with(&config("bad divisor", 3), &[1, 2], move |_| *flag.lock().unwrap() = true, lookup).unwrap_err();
        assert_eq!(message, r#"bad divisor: CRAFT_HOSTILE_DIVISOR="0" is zero; set it to a whole number above zero"#);
        assert!(!*called.lock().unwrap());
    }

    #[test]
    fn invalid_multiplier_names_variable() {
        for (value, reason) in [
            ("0", "is zero"),
            ("", "is empty"),
            ("ten", "is not a whole number written in digits"),
            ("-2", "is not a whole number written in digits"),
            ("1.5", "is not a whole number written in digits"),
            ("99999999999999999999999999", "is more than a usize holds"),
        ] {
            let called = Arc::new(Mutex::new(false));
            let flag = Arc::clone(&called);
            let message = hammer_with(
                &config("bad mult", 3).with_multiplier_variable("MY_MULT"),
                &[1, 2],
                move |_| *flag.lock().unwrap() = true,
                |_| Some(OsString::from(value)),
            )
            .unwrap_err();
            assert_eq!(message, format!("bad mult: MY_MULT={value:?} {reason}; set it to a whole number above zero"));
            assert!(!*called.lock().unwrap(), "parse ran before the bad value {value:?} was reported");
        }
        for name in ["", "A=B", "A\0B"] {
            assert_eq!(config("x", 1).with_multiplier_variable(name).rounds_with(unset), Err(HammerError::BadName { variable: name.to_owned() }));
        }
    }

    #[test]
    fn empty_seed_runs_once() {
        assert_eq!(inputs(&config("empty", 1_000), &[]), [Vec::<u8>::new()]);
    }

    #[test]
    fn large_seed_checks_every_prefix() {
        let seed: Vec<u8> = (0..10_000u32).map(|i| i as u8).collect();
        let mut lengths = BTreeMap::new();
        for input in inputs(&config("large", 0), &seed) {
            assert_eq!(input, seed[..input.len()]);
            *lengths.entry(input.len()).or_insert(0usize) += 1;
        }
        assert_eq!(lengths.len(), 10_001);
        assert!(lengths.values().all(|&n| n == 1), "a length ran more than once");
    }

    #[test]
    fn iteration_product_overflow_is_error() {
        let config = config("overflow", usize::MAX / 2 + 1);
        let error = config.rounds_with(|_| Some(OsString::from("2"))).unwrap_err();
        assert_eq!(error, HammerError::Overflow { iterations: usize::MAX / 2 + 1, variable: UNSET.to_owned(), multiplier: 2 });
        let message = hammer_with(&config, &[1], |_| {}, |_| Some(OsString::from("2"))).unwrap_err();
        assert!(message.starts_with("overflow: ") && message.ends_with("is more than a usize holds"), "{message}");
    }

    #[test]
    fn too_many_cases_fail_before_any_parse() {
        let called = Arc::new(Mutex::new(false));
        let flag = Arc::clone(&called);
        // The seed and its two truncations make three cases more.
        let message = hammer_with(&config("budget", MAX_CASES - 2), &[1, 2], move |_| *flag.lock().unwrap() = true, unset).unwrap_err();
        assert_eq!(message, format!("budget: {} cases is more than the limit of {MAX_CASES}", MAX_CASES + 1));
        assert!(!*called.lock().unwrap());
    }

    #[test]
    fn mutations_are_deterministic_and_count_rounds() {
        let seed = b"a seed of some length";
        let first: Vec<_> = mutations(seed, 50).collect();
        assert_eq!(first.len(), 50);
        assert_eq!(first, mutations(seed, 50).collect::<Vec<_>>());
        assert_eq!(mutations(seed, 50).len(), 50);
        assert_eq!(mutations(&[], 50).count(), 0);
    }
}
