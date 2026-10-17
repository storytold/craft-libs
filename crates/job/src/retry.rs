//! Retry vocabulary and the decision rule.
//!
//! Two questions, kept apart on purpose:
//!
//! * *Is the condition transient?* A busy device, a dropped connection and a timeout are; a
//!   refused address or a format the device does not take are not. That is `retryable`.
//! * *Is repeating the request safe?* Only if nothing was accepted, or the request is idempotent.
//!   A connection that closed after the request was written may have been delivered: sending it
//!   again could do the work twice. That is [`RetrySafety`].
//!
//! [`advise`] combines the two with a [`RetryPolicy`] and says what to do next. Nothing here
//! retries anything or sleeps; the caller owns the loop and the clock.

use std::time::Duration;

/// What repeating a failed request would do. (craft-wsd keeps its own, identical enum.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RetrySafety {
    /// Nothing was sent or accepted (refused before connect, policy, a connect failure, a device
    /// that said it is busy). Repeating cannot do the work twice.
    Safe,
    /// The request may have been delivered and acted on, and repeating it is not known to be
    /// harmless. Look at the device first, never blindly resend.
    OutcomeUnknown,
    /// The request may have been delivered, but repeating it is harmless (a query or a cancel).
    Idempotent,
}

impl RetrySafety {
    /// Whether a retry may be made without asking a person or checking the device first.
    pub const fn allows_automatic_retry(self) -> bool {
        !matches!(self, Self::OutcomeUnknown)
    }

    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Safe => "safe",
            Self::OutcomeUnknown => "outcome_unknown",
            Self::Idempotent => "idempotent",
        }
    }
}

/// A tiny deterministic random source for jitter (xorshift64*). It spreads retries out and nothing
/// else; the same seed gives the same sequence.
#[derive(Clone, Debug)]
pub struct Jitter(u64);

impl Jitter {
    /// A source with this seed (a zero seed is replaced).
    pub fn new(seed: u64) -> Self {
        Self(if seed == 0 { 0x9E37_79B9_7F4A_7C15 } else { seed })
    }

    /// The next value in `[0, 1)`.
    pub fn unit(&mut self) -> f64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        let out = x.wrapping_mul(0x2545_F491_4F6C_DD1D);
        (out >> 11) as f64 / (1u64 << 53) as f64
    }
}

/// How often and how long to retry. Attempts count tries, the first one included:
/// `max_attempts: 1` never retries.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RetryPolicy {
    pub max_attempts: u32,
    /// The wait after the first failure; it doubles per failure up to `max_delay`.
    pub base: Duration,
    pub max_delay: Duration,
    /// Spread each wait by up to this many percent either way (0 to 100).
    pub jitter_percent: u32,
    /// Longest a device's own "come back in" hint is honoured.
    pub max_retry_after: Duration,
}

impl Default for RetryPolicy {
    fn default() -> Self {
        Self {
            max_attempts: 4,
            base: Duration::from_millis(500),
            max_delay: Duration::from_secs(8),
            jitter_percent: 20,
            max_retry_after: Duration::from_secs(60),
        }
    }
}

impl RetryPolicy {
    /// A policy that never retries.
    pub fn none() -> Self {
        Self { max_attempts: 1, ..Self::default() }
    }

    /// The wait after failure number `attempt` (1 is the first failure; 0 counts as 1):
    /// `base * 2^(attempt-1)` capped at `max_delay`, spread by `jitter_percent` around its value
    /// with `unit` in `[0, 1]` (out of range is clamped, a non-finite `unit` counts as the middle,
    /// no spread). Never longer than `max_delay`.
    pub fn delay(&self, attempt: u32, unit: f64) -> Duration {
        let doublings = attempt.saturating_sub(1).min(30);
        let backoff = self.base.saturating_mul(1u32 << doublings).min(self.max_delay);
        let unit = if unit.is_finite() { unit.clamp(0.0, 1.0) } else { 0.5 };
        let spread = f64::from(self.jitter_percent.min(100)) / 100.0;
        let factor = 1.0 + (unit * 2.0 - 1.0) * spread;
        Duration::try_from_secs_f64(backoff.as_secs_f64() * factor).unwrap_or(backoff).min(self.max_delay)
    }
}

/// What the caller knows about one failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RetryFacts {
    /// Is the condition transient?
    pub retryable: bool,
    /// What repeating the request would do.
    pub safety: RetrySafety,
    /// The device's own "come back in" hint, if it gave one.
    pub retry_after: Option<Duration>,
    /// Attempts made so far, the failed one included (so the first failure is `1`).
    pub attempts_so_far: u32,
    /// Seeds the jitter of this decision; the same facts and seed always give the same advice.
    pub jitter_seed: u64,
}

impl RetryFacts {
    pub const fn new(retryable: bool, safety: RetrySafety) -> Self {
        Self { retryable, safety, retry_after: None, attempts_so_far: 1, jitter_seed: 0 }
    }

    #[must_use]
    pub const fn with_retry_after(mut self, retry_after: Duration) -> Self {
        self.retry_after = Some(retry_after);
        self
    }

    #[must_use]
    pub const fn with_attempts(mut self, attempts_so_far: u32) -> Self {
        self.attempts_so_far = attempts_so_far;
        self
    }

    #[must_use]
    pub const fn with_jitter_seed(mut self, seed: u64) -> Self {
        self.jitter_seed = seed;
        self
    }
}

/// What to do after a failed attempt.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum RetryAdvice {
    /// Try again immediately.
    RetryNow,
    /// Try again after this wait.
    RetryAfter(#[cfg_attr(feature = "serde", serde(with = "crate::wire::duration_ms"))] Duration),
    /// The request may have been delivered: query the device (or ask a person) before any resend.
    CheckDeviceFirst,
    /// Give up: the condition is not transient, or the attempts are used up.
    DoNotRetry,
}

impl RetryAdvice {
    /// Whether the advice is to send the request again without further checks.
    pub const fn is_automatic_retry(self) -> bool {
        matches!(self, Self::RetryNow | Self::RetryAfter(_))
    }
}

/// Decide what follows a failed attempt, in this order:
///
/// 1. not `retryable`: [`RetryAdvice::DoNotRetry`];
/// 2. [`RetrySafety::OutcomeUnknown`]: [`RetryAdvice::CheckDeviceFirst`], whatever the attempts
///    left (the question "did it arrive?" does not go away when the budget does), and never an
///    automatic retry;
/// 3. `attempts_so_far >= max_attempts`: [`RetryAdvice::DoNotRetry`];
/// 4. otherwise the policy's backoff, jittered from `jitter_seed` and raised to the device's
///    `retry_after` hint capped at `max_retry_after`: [`RetryAdvice::RetryNow`] when that wait is
///    zero, else [`RetryAdvice::RetryAfter`]. The wait never exceeds the larger of `max_delay` and
///    the capped hint.
pub fn advise(facts: &RetryFacts, policy: &RetryPolicy) -> RetryAdvice {
    if !facts.retryable {
        return RetryAdvice::DoNotRetry;
    }
    if !facts.safety.allows_automatic_retry() {
        return RetryAdvice::CheckDeviceFirst;
    }
    if facts.attempts_so_far >= policy.max_attempts {
        return RetryAdvice::DoNotRetry;
    }
    let unit = Jitter::new(facts.jitter_seed ^ u64::from(facts.attempts_so_far).wrapping_mul(0x9E37_79B9_7F4A_7C15)).unit();
    let hint = facts.retry_after.map_or(Duration::ZERO, |d| d.min(policy.max_retry_after));
    let wait = policy.delay(facts.attempts_so_far, unit).max(hint);
    if wait.is_zero() { RetryAdvice::RetryNow } else { RetryAdvice::RetryAfter(wait) }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: fn(u64) -> Duration = Duration::from_millis;

    fn policy() -> RetryPolicy {
        RetryPolicy { max_attempts: 5, base: MS(100), max_delay: MS(1000), jitter_percent: 0, max_retry_after: MS(2500) }
    }

    #[test]
    fn safety_names_and_automatic_retry() {
        assert!(RetrySafety::Safe.allows_automatic_retry());
        assert!(RetrySafety::Idempotent.allows_automatic_retry());
        assert!(!RetrySafety::OutcomeUnknown.allows_automatic_retry());
        assert_eq!(RetrySafety::Safe.name(), "safe");
        assert_eq!(RetrySafety::OutcomeUnknown.name(), "outcome_unknown");
        assert_eq!(RetrySafety::Idempotent.name(), "idempotent");
    }

    #[test]
    fn delays_double_up_to_the_cap() {
        let p = policy();
        let ms: Vec<u128> = (1..=6).map(|a| p.delay(a, 0.5).as_millis()).collect();
        assert_eq!(ms, [100, 200, 400, 800, 1000, 1000]);
        assert_eq!(p.delay(0, 0.5), MS(100), "attempt 0 is treated like 1");
        assert_eq!(p.delay(u32::MAX, 0.5), MS(1000), "no overflow");
    }

    #[test]
    fn jitter_bounds() {
        let p = RetryPolicy { jitter_percent: 50, ..policy() };
        assert_eq!(p.delay(1, 0.0), MS(50));
        assert_eq!(p.delay(1, 1.0), MS(150));
        assert_eq!(p.delay(1, 0.5), MS(100));
        assert_eq!(p.delay(5, 1.0), MS(1000), "capped after jitter");
        assert_eq!(p.delay(1, f64::NAN), MS(100));
        assert_eq!(p.delay(1, f64::INFINITY), MS(100));
        assert_eq!(p.delay(1, -3.0), MS(50));
        assert_eq!(p.delay(1, 9.0), MS(150));
        let over = RetryPolicy { jitter_percent: 1000, ..policy() };
        assert_eq!(over.delay(1, 0.0), Duration::ZERO, "percent is clamped to 100");
        assert_eq!(over.delay(1, 1.0), MS(200));
        let huge = RetryPolicy { max_delay: Duration::MAX, base: Duration::MAX, ..p };
        let _ = huge.delay(40, 1.0); // must not panic
    }

    #[test]
    fn jitter_source_is_deterministic_and_in_range() {
        let mut a = Jitter::new(42);
        let mut b = Jitter::new(42);
        let mut seen = std::collections::HashSet::new();
        for _ in 0..1000 {
            let (x, y) = (a.unit(), b.unit());
            assert_eq!(x.to_bits(), y.to_bits());
            assert!((0.0..1.0).contains(&x));
            seen.insert(x.to_bits());
        }
        assert!(seen.len() > 990, "not stuck");
        assert_eq!(Jitter::new(0).unit().to_bits(), Jitter::new(0).unit().to_bits(), "zero seed is usable");
        assert_ne!(Jitter::new(1).unit().to_bits(), Jitter::new(2).unit().to_bits());
    }

    #[test]
    fn decision_table() {
        use RetryAdvice::*;
        use RetrySafety::*;
        let p = policy();
        let f = |retryable, safety, attempts| RetryFacts::new(retryable, safety).with_attempts(attempts);
        // not retryable always stops, whatever the safety
        for s in [Safe, OutcomeUnknown, Idempotent] {
            assert_eq!(advise(&f(false, s, 1), &p), DoNotRetry);
        }
        // safe and idempotent retry within the budget
        assert_eq!(advise(&f(true, Safe, 1), &p), RetryAfter(MS(100)));
        assert_eq!(advise(&f(true, Idempotent, 2), &p), RetryAfter(MS(200)));
        assert_eq!(advise(&f(true, Safe, 4), &p), RetryAfter(MS(800)));
        // the budget is the number of tries, the first included
        assert_eq!(advise(&f(true, Safe, 5), &p), DoNotRetry);
        assert_eq!(advise(&f(true, Safe, 99), &p), DoNotRetry);
        assert_eq!(advise(&f(true, Safe, 1), &RetryPolicy::none()), DoNotRetry);
        // an unknown outcome is checked, never retried, even with budget left or spent
        assert_eq!(advise(&f(true, OutcomeUnknown, 1), &p), CheckDeviceFirst);
        assert_eq!(advise(&f(true, OutcomeUnknown, 99), &p), CheckDeviceFirst);
        assert_eq!(advise(&f(true, OutcomeUnknown, 1), &RetryPolicy::none()), CheckDeviceFirst);
        // a zero wait is RetryNow
        let zero = RetryPolicy { base: Duration::ZERO, ..p };
        assert_eq!(advise(&f(true, Safe, 1), &zero), RetryNow);
        assert!(RetryNow.is_automatic_retry() && RetryAfter(MS(1)).is_automatic_retry());
        assert!(!CheckDeviceFirst.is_automatic_retry() && !DoNotRetry.is_automatic_retry());
    }

    #[test]
    fn retry_after_raises_the_wait_and_is_capped() {
        let p = policy();
        let base = RetryFacts::new(true, RetrySafety::Safe).with_attempts(1);
        assert_eq!(advise(&base.with_retry_after(MS(1500)), &p), RetryAdvice::RetryAfter(MS(1500)), "above max_delay but under the hint cap");
        assert_eq!(advise(&base.with_retry_after(Duration::from_secs(3600)), &p), RetryAdvice::RetryAfter(MS(2500)), "capped");
        assert_eq!(advise(&base.with_retry_after(MS(10)), &p), RetryAdvice::RetryAfter(MS(100)), "a small hint does not shorten the backoff");
        assert_eq!(advise(&base.with_retry_after(Duration::MAX), &p), RetryAdvice::RetryAfter(MS(2500)));
        let zero = RetryPolicy { base: Duration::ZERO, ..p };
        assert_eq!(advise(&base.with_retry_after(Duration::ZERO), &zero), RetryAdvice::RetryNow);
        let no_hint = RetryPolicy { max_retry_after: Duration::ZERO, ..p };
        assert_eq!(
            advise(&base.with_retry_after(Duration::from_secs(9)), &no_hint),
            RetryAdvice::RetryAfter(MS(100)),
            "hint cap of zero ignores hints"
        );
        // the hint never turns a terminal decision into a retry
        let spent = base.with_attempts(5).with_retry_after(MS(5));
        assert_eq!(advise(&spent, &p), RetryAdvice::DoNotRetry);
        let unknown = RetryFacts::new(true, RetrySafety::OutcomeUnknown).with_retry_after(MS(5));
        assert_eq!(advise(&unknown, &p), RetryAdvice::CheckDeviceFirst);
    }

    #[test]
    fn jitter_in_advice_is_seeded_bounded_and_repeatable() {
        let p = RetryPolicy { jitter_percent: 25, ..policy() };
        let mut distinct = std::collections::HashSet::new();
        for seed in 0..200u64 {
            let facts = RetryFacts::new(true, RetrySafety::Safe).with_attempts(2).with_jitter_seed(seed);
            let first = advise(&facts, &p);
            assert_eq!(first, advise(&facts, &p), "same facts and seed, same advice");
            let RetryAdvice::RetryAfter(wait) = first else { panic!("expected a wait, got {first:?}") };
            assert!(wait >= MS(150) && wait <= MS(250), "{wait:?} outside 200ms +/- 25%");
            distinct.insert(wait);
        }
        assert!(distinct.len() > 20, "seeds spread the waits");
    }

    #[test]
    fn facts_builders() {
        let f = RetryFacts::new(true, RetrySafety::Idempotent);
        assert_eq!((f.retryable, f.safety, f.retry_after, f.attempts_so_far, f.jitter_seed), (true, RetrySafety::Idempotent, None, 1, 0));
        let f = f.with_attempts(3).with_jitter_seed(9).with_retry_after(MS(7));
        assert_eq!((f.attempts_so_far, f.jitter_seed, f.retry_after), (3, 9, Some(MS(7))));
    }
}
