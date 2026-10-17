//! Retry semantics for printing: where a failure happened ([`Phase`]), the clock that makes waiting
//! testable, and the glue from a [`PrintError`] to [`craft_job`]'s retry decision.
//!
//! [`crate::RetrySafety`], [`crate::RetryPolicy`], [`RetryFacts`], [`RetryAdvice`], [`advise`] and [`Jitter`] are
//! `craft-job`'s (shared with scanning) and re-exported from the crate root; [`retry_facts`] and
//! [`advise_error`] build the facts from the error.
//!
//! Two questions, kept apart on purpose:
//!
//! * *Is the condition transient?* A busy printer, a dropped connection and a timeout are; a
//!   refused address, a format the printer does not take and a redirect are not.
//! * *Is repeating the request safe?* Only if nothing was accepted, or the request is idempotent.
//!   A connection that closed after the request was written, or a timeout while waiting for the
//!   reply to a create/send, may have been delivered: retrying could print twice. Such an error is
//!   transient **and** [`crate::RetrySafety::OutcomeUnknown`]; [`advise`] answers
//!   [`RetryAdvice::CheckDeviceFirst`] for it (also when the attempts are used up) and nothing here
//!   retries it on its own; a backend that can offers a reconcile step (look for the job before
//!   resubmitting).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use craft_job::{Jitter, RetryAdvice, RetryFacts, RetryPolicy, advise};

use crate::backend::{CancelToken, PrintError};

/// Where in an exchange a failure happened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Phase {
    /// Before any traffic: validation, policy, consent, encoding the document.
    Prepare,
    /// Making the connection. Nothing was sent.
    Connect,
    /// Writing the request. It may or may not have arrived.
    Request,
    /// The request was sent; the reply is missing, late or unusable.
    Reply,
    /// After the printer accepted the job, while following it.
    Poll,
}

impl Phase {
    pub const fn name(self) -> &'static str {
        match self {
            Self::Prepare => "prepare",
            Self::Connect => "connect",
            Self::Request => "request",
            Self::Reply => "reply",
            Self::Poll => "poll",
        }
    }
}

/// A monotonic clock that can also wait. [`SystemClock`] is the real one; [`ManualClock`] makes
/// tests instant and exact.
pub trait Clock: Send + Sync {
    /// Time since an arbitrary fixed origin; never goes backwards.
    fn now(&self) -> Duration;
    /// Wait. A manual clock just moves its time forward.
    fn sleep(&self, duration: Duration);
}

/// The real clock (`std::time::Instant`, `std::thread::sleep`). Not available on
/// `wasm32-unknown-unknown`: pass another [`Clock`] there.
#[derive(Clone, Copy, Debug, Default)]
pub struct SystemClock;

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        static START: OnceLock<Instant> = OnceLock::new();
        START.get_or_init(Instant::now).elapsed()
    }

    fn sleep(&self, duration: Duration) {
        std::thread::sleep(duration);
    }
}

/// A clock a test moves by hand. Clones share the time; `sleep` advances it, so code that waits
/// finishes at once and the total waited can be asserted.
#[derive(Clone, Debug, Default)]
pub struct ManualClock {
    now: Arc<Mutex<Duration>>,
    slept: Arc<Mutex<Vec<Duration>>>,
}

impl ManualClock {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn advance(&self, by: Duration) {
        let mut now = self.now.lock().unwrap_or_else(PoisonError::into_inner);
        *now = now.saturating_add(by);
    }

    /// Every duration passed to `sleep`, in order.
    pub fn sleeps(&self) -> Vec<Duration> {
        self.slept.lock().unwrap_or_else(PoisonError::into_inner).clone()
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        *self.now.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn sleep(&self, duration: Duration) {
        self.slept.lock().unwrap_or_else(PoisonError::into_inner).push(duration);
        self.advance(duration);
    }
}

/// Sleep in short slices so a [`CancelToken`] is noticed within [`CANCEL_SLICE`].
pub(crate) fn sleep_cancellable(clock: &dyn Clock, cancel: &CancelToken, mut duration: Duration) {
    while !duration.is_zero() && !cancel.is_cancelled() {
        let step = duration.min(CANCEL_SLICE);
        clock.sleep(step);
        duration -= step;
    }
}

/// The longest a waiting helper sleeps without looking at its cancel token.
pub const CANCEL_SLICE: Duration = Duration::from_millis(100);

/// Jitter seeded from a clock, the one thing [`craft_job::Jitter`] leaves to the domain crate (it
/// has no clock of its own).
pub trait JitterExt: Sized {
    /// A source seeded from a clock reading and a process-wide counter, so two watchers started
    /// together do not wait in step.
    fn from_clock(clock: &dyn Clock) -> Self;
    /// A seed for [`RetryFacts::with_jitter_seed`], drawn from this source.
    fn next_seed(&mut self) -> u64;
}

impl JitterExt for Jitter {
    fn from_clock(clock: &dyn Clock) -> Self {
        static COUNTER: AtomicU64 = AtomicU64::new(1);
        let salt = COUNTER.fetch_add(0x9E37_79B9, Ordering::Relaxed);
        Self::new((clock.now().as_nanos() as u64) ^ salt.rotate_left(17))
    }

    fn next_seed(&mut self) -> u64 {
        (self.unit() * (1u64 << 53) as f64) as u64
    }
}

/// The facts [`craft_job::advise`] needs about a failed try, read from the error: is it transient,
/// is repeating it safe, the device's own wait, and which try this was (1 for the first).
pub fn retry_facts(error: &PrintError, attempts_so_far: u32) -> RetryFacts {
    let facts = RetryFacts::new(error.is_retryable(), error.retry_safety()).with_attempts(attempts_so_far);
    match error.retry_after() {
        Some(wait) => facts.with_retry_after(wait),
        None => facts,
    }
}

/// What to do after `error` on try number `attempts_so_far` ([`craft_job::advise`] over
/// [`retry_facts`]). [`RetryAdvice::RetryNow`] and [`RetryAdvice::RetryAfter`] mean try again;
/// [`RetryAdvice::CheckDeviceFirst`] means the request may have been delivered (never repeated on
/// its own, even when the attempts are used up); [`RetryAdvice::DoNotRetry`] means give up.
pub fn advise_error(error: &PrintError, attempts_so_far: u32, policy: &RetryPolicy, jitter: &mut Jitter) -> RetryAdvice {
    advise(&retry_facts(error, attempts_so_far).with_jitter_seed(jitter.next_seed()), policy)
}

/// Run `op` (given the try number, from 1) until it succeeds or [`advise_error`] says to stop.
/// The waits go through `clock` and a set `cancel` ends the loop with the last error.
///
/// Use it for queries, cancels and anything else whose errors are [`crate::RetrySafety::Safe`] or
/// [`crate::RetrySafety::Idempotent`]. A submit whose error is [`crate::RetrySafety::OutcomeUnknown`] comes back
/// after the first try: reconcile, do not repeat.
pub fn retry<T>(
    policy: &RetryPolicy,
    clock: &dyn Clock,
    cancel: &CancelToken,
    jitter: &mut Jitter,
    mut op: impl FnMut(u32) -> Result<T, PrintError>,
) -> Result<T, PrintError> {
    let mut attempt = 1u32;
    loop {
        let error = match op(attempt) {
            Ok(value) => return Ok(value),
            Err(error) => error,
        };
        match advise_error(&error, attempt, policy, jitter) {
            RetryAdvice::RetryNow if !cancel.is_cancelled() => {}
            RetryAdvice::RetryAfter(wait) if !cancel.is_cancelled() => sleep_cancellable(clock, cancel, wait),
            _ => return Err(error),
        }
        attempt = attempt.saturating_add(1);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::RetrySafety;
    use crate::backend::ErrorKind;

    fn policy() -> RetryPolicy {
        RetryPolicy {
            max_attempts: 5,
            base: Duration::from_millis(100),
            max_delay: Duration::from_millis(1000),
            jitter_percent: 0,
            ..Default::default()
        }
    }

    fn error(kind: ErrorKind, safety: RetrySafety) -> PrintError {
        PrintError::new(kind, "x").with_safety(safety)
    }

    fn advice(p: &RetryPolicy, error: &PrintError, attempt: u32) -> RetryAdvice {
        advise_error(error, attempt, p, &mut Jitter::new(1))
    }

    #[test]
    fn only_transient_and_repeatable_errors_are_retried_and_attempts_are_bounded() {
        let p = policy();
        assert!(advice(&p, &error(ErrorKind::Busy, RetrySafety::Safe), 1).is_automatic_retry());
        assert!(advice(&p, &error(ErrorKind::Timeout, RetrySafety::Idempotent), 1).is_automatic_retry());
        assert_eq!(
            advice(&p, &error(ErrorKind::Timeout, RetrySafety::OutcomeUnknown), 1),
            RetryAdvice::CheckDeviceFirst,
            "may have printed: never automatic, look at the printer first"
        );
        assert_eq!(
            advice(&p, &error(ErrorKind::Timeout, RetrySafety::OutcomeUnknown), 5),
            RetryAdvice::CheckDeviceFirst,
            "an unknown outcome stays one when the attempts are used up (craft-job's rule)"
        );
        assert_eq!(advice(&p, &error(ErrorKind::Rejected, RetrySafety::Safe), 1), RetryAdvice::DoNotRetry, "not transient");
        assert_eq!(advice(&p, &error(ErrorKind::Busy, RetrySafety::Safe), 5), RetryAdvice::DoNotRetry, "attempts used up");
        assert_eq!(advice(&RetryPolicy::none(), &error(ErrorKind::Busy, RetrySafety::Safe), 1), RetryAdvice::DoNotRetry);
    }

    #[test]
    fn a_device_hint_lengthens_the_wait_up_to_a_cap() {
        let p = RetryPolicy { max_retry_after: Duration::from_secs(10), ..policy() };
        let busy = |secs| error(ErrorKind::Busy, RetrySafety::Safe).with_retry_after(Duration::from_secs(secs));
        assert_eq!(advice(&p, &busy(5), 1), RetryAdvice::RetryAfter(Duration::from_secs(5)));
        assert_eq!(advice(&p, &busy(3600), 1), RetryAdvice::RetryAfter(Duration::from_secs(10)));
        assert_eq!(advice(&p, &busy(0), 1), RetryAdvice::RetryAfter(Duration::from_millis(100)), "backoff still applies");
    }

    #[test]
    fn retry_facts_read_the_error() {
        let e = error(ErrorKind::Timeout, RetrySafety::OutcomeUnknown).with_retry_after(Duration::from_secs(2));
        let f = retry_facts(&e, 3);
        assert_eq!((f.retryable, f.safety, f.retry_after, f.attempts_so_far), (true, RetrySafety::OutcomeUnknown, Some(Duration::from_secs(2)), 3));
        assert!(!retry_facts(&error(ErrorKind::Rejected, RetrySafety::Safe), 1).retryable);
    }

    #[test]
    fn retry_waits_on_the_clock_and_stops_at_success_or_an_unsafe_error() {
        let clock = ManualClock::new();
        let cancel = CancelToken::new();
        let mut jitter = Jitter::new(1);
        let mut tries = 0;
        let got = retry(&policy(), &clock, &cancel, &mut jitter, |n| {
            tries = n;
            if n < 3 { Err(error(ErrorKind::Unavailable, RetrySafety::Idempotent)) } else { Ok(n) }
        });
        assert_eq!((got, tries), (Ok(3), 3));
        let total: Duration = clock.sleeps().iter().sum();
        assert!(total >= Duration::from_millis(100 + 200) * 8 / 10, "{total:?}");

        let mut calls = 0;
        let got: Result<(), _> = retry(&policy(), &clock, &cancel, &mut jitter, |_| {
            calls += 1;
            Err(error(ErrorKind::Timeout, RetrySafety::OutcomeUnknown))
        });
        assert_eq!(calls, 1);
        assert_eq!(got.map_err(|e| e.retry_safety()), Err(RetrySafety::OutcomeUnknown));

        let mut calls = 0;
        let _: Result<(), _> = retry(&policy(), &clock, &cancel, &mut jitter, |_| {
            calls += 1;
            Err(error(ErrorKind::Busy, RetrySafety::Safe))
        });
        assert_eq!(calls, 5, "max_attempts bounds the loop");
    }

    #[test]
    fn a_cancelled_token_ends_the_loop_after_the_current_try() {
        let clock = ManualClock::new();
        let cancel = CancelToken::new();
        let mut jitter = Jitter::new(1);
        let mut calls = 0;
        let _: Result<(), _> = retry(&policy(), &clock, &cancel, &mut jitter, |_| {
            calls += 1;
            cancel.cancel();
            Err(error(ErrorKind::Busy, RetrySafety::Safe))
        });
        assert_eq!(calls, 1);
    }

    #[test]
    fn the_manual_clock_records_and_shares_time() {
        let clock = ManualClock::new();
        let other = clock.clone();
        clock.sleep(Duration::from_secs(2));
        other.advance(Duration::from_secs(1));
        assert_eq!(clock.now(), Duration::from_secs(3));
        assert_eq!(other.sleeps(), [Duration::from_secs(2)]);
        let cancel = CancelToken::new();
        sleep_cancellable(&clock, &cancel, Duration::from_millis(250));
        assert_eq!(clock.sleeps().len(), 4, "100 + 100 + 50 ms after the 2 s sleep");
    }

    #[test]
    fn the_system_clock_moves_forward() {
        let a = SystemClock.now();
        SystemClock.sleep(Duration::from_millis(2));
        assert!(SystemClock.now() > a);
    }
}
