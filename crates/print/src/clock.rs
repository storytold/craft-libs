//! The time sources the crate's defaults read, safe on every target.
//!
//! `std::time::Instant::now()`, `SystemTime::now()` and `std::thread::sleep` panic on
//! `wasm32-unknown-unknown`. Everything that reads "now" without being handed a clock goes through
//! this module instead. On other targets it is the system's clocks and a real sleep. On `wasm32` it
//! is a logical clock: every read moves it on by one millisecond and a sleep moves it on by the
//! time slept without waiting, so a deadline loop always ends and nothing panics. That time is not
//! the real time, so a `wasm32` host that wants real readings passes its own: a
//! [`crate::Clock`] (`with_clock`) and a wall clock (`with_wall_clock`), and sets the times of a
//! [`crate::JobStatus`] or [`crate::Submitted`] it builds itself.

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

/// A clock that only moves when it is read or slept on. Saturates instead of overflowing.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
pub(crate) struct Logical {
    nanos: AtomicU64,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl Logical {
    /// Each read moves the clock on by this much.
    const TICK: Duration = Duration::from_millis(1);

    pub(crate) const fn new() -> Self {
        Self { nanos: AtomicU64::new(0) }
    }

    /// Move the clock on by `by` (saturating) and return the new time.
    pub(crate) fn advance(&self, by: Duration) -> Duration {
        let by = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        let mut prev = self.nanos.load(Ordering::Relaxed);
        loop {
            let next = prev.saturating_add(by);
            match self.nanos.compare_exchange_weak(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Duration::from_nanos(next),
                Err(actual) => prev = actual,
            }
        }
    }

    /// A reading: one tick later than the last.
    pub(crate) fn read(&self) -> Duration {
        self.advance(Self::TICK)
    }
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
static LOGICAL: Logical = Logical::new();

/// Time since an arbitrary fixed origin; never goes backwards.
pub(crate) fn monotonic() -> Duration {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::sync::OnceLock;
        static START: OnceLock<std::time::Instant> = OnceLock::new();
        START.get_or_init(std::time::Instant::now).elapsed()
    }
    #[cfg(target_arch = "wasm32")]
    {
        LOGICAL.read()
    }
}

/// Wait: block the thread, or on `wasm32` just move the logical clock on.
pub(crate) fn sleep(duration: Duration) {
    #[cfg(not(target_arch = "wasm32"))]
    {
        std::thread::sleep(duration);
    }
    #[cfg(target_arch = "wasm32")]
    {
        LOGICAL.advance(duration);
    }
}

/// The wall-clock time now. On `wasm32` the logical clock after the epoch.
pub(crate) fn wall_now() -> SystemTime {
    #[cfg(not(target_arch = "wasm32"))]
    {
        SystemTime::now()
    }
    #[cfg(target_arch = "wasm32")]
    {
        SystemTime::UNIX_EPOCH.checked_add(LOGICAL.read()).unwrap_or(SystemTime::UNIX_EPOCH)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::UNIX_EPOCH;

    #[test]
    fn a_logical_clock_ticks_on_every_read_and_a_sleep_moves_it_without_waiting() {
        let clock = Logical::new();
        assert_eq!(clock.read(), Duration::from_millis(1));
        assert_eq!(clock.read(), Duration::from_millis(2));
        assert_eq!(clock.advance(Duration::from_secs(5)), Duration::from_millis(5002));
        assert!(clock.read() > Duration::from_secs(5));
    }

    #[test]
    fn a_logical_clock_saturates_instead_of_overflowing() {
        let clock = Logical::new();
        assert_eq!(clock.advance(Duration::MAX), Duration::from_nanos(u64::MAX));
        assert_eq!(clock.read(), Duration::from_nanos(u64::MAX), "a read at the end stays at the end");
        assert_eq!(clock.advance(Duration::from_secs(1)), Duration::from_nanos(u64::MAX));
    }

    #[test]
    fn the_system_defaults_are_real_time_here() {
        let (a, b) = (monotonic(), monotonic());
        assert!(b >= a);
        let before = wall_now();
        sleep(Duration::from_millis(5));
        let after = wall_now();
        assert!(after.duration_since(before).is_ok_and(|d| d >= Duration::from_millis(5)));
        assert!(wall_now().duration_since(UNIX_EPOCH).is_ok_and(|d| d > Duration::from_secs(1_000_000_000)));
    }
}
