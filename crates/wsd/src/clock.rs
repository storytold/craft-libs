//! A monotonic clock that exists on every target.
//!
//! `std::time::Instant::now()` and `SystemTime::now()` panic on `wasm32-unknown-unknown`, so
//! nothing in this crate calls them directly: deadlines use [`Instant`] and message ids draw on
//! `entropy_nanos`. On other targets [`Instant`] is the system's monotonic clock. On `wasm32` it
//! reads the source the host installed with [`set_time_source`]; with none installed it is a
//! logical clock that advances one millisecond per read, so a deadline loop always ends.

use std::ops::Add;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

/// A host's clock: the time elapsed since any fixed point of the host's choosing.
pub type TimeSource = fn() -> Duration;

/// A point on a monotonic clock. Only differences between two readings mean anything.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant(Duration);

impl Instant {
    /// The current time. Never panics, on any target.
    pub fn now() -> Self {
        Self(now_since_origin())
    }

    /// This point plus `d`, or `None` if that cannot be represented.
    pub fn checked_add(self, d: Duration) -> Option<Self> {
        self.0.checked_add(d).map(Self)
    }

    /// Time from `earlier` to this point, or `None` if `earlier` is later.
    pub fn checked_duration_since(self, earlier: Self) -> Option<Duration> {
        self.0.checked_sub(earlier.0)
    }

    /// Time from `earlier` to this point, zero if `earlier` is later.
    pub fn saturating_duration_since(self, earlier: Self) -> Duration {
        self.0.saturating_sub(earlier.0)
    }
}

impl Add<Duration> for Instant {
    type Output = Self;

    /// Saturates instead of overflowing.
    fn add(self, d: Duration) -> Self {
        Self(self.0.saturating_add(d))
    }
}

/// Install the clock `wasm32` builds read ([`Instant::now`]); the first call wins and returns
/// `true`. Other targets always use the system clock, so this has no effect there.
pub fn set_time_source(source: TimeSource) -> bool {
    HOST.set(source)
}

/// A varying number for message ids: wall-clock nanoseconds on most targets, the host's clock
/// (or a logical tick) on `wasm32`. Not a time to compare or display.
pub(crate) fn entropy_nanos() -> u64 {
    #[cfg(not(target_arch = "wasm32"))]
    {
        use std::time::{SystemTime, UNIX_EPOCH};
        SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
    }
    #[cfg(target_arch = "wasm32")]
    {
        HOST.now().as_nanos() as u64
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn now_since_origin() -> Duration {
    static ORIGIN: OnceLock<std::time::Instant> = OnceLock::new();
    ORIGIN.get_or_init(std::time::Instant::now).elapsed()
}

#[cfg(target_arch = "wasm32")]
fn now_since_origin() -> Duration {
    HOST.now()
}

static HOST: HostClock = HostClock::new();

/// The clock a target without `std` time reads: the host's source if one was installed (never
/// allowed to go backwards), else a logical clock.
#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
struct HostClock {
    source: OnceLock<TimeSource>,
    last: AtomicU64,
}

#[cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]
impl HostClock {
    /// Each read without a source moves the logical clock on by this much.
    const TICK_NANOS: u64 = 1_000_000;

    const fn new() -> Self {
        Self { source: OnceLock::new(), last: AtomicU64::new(0) }
    }

    fn set(&self, source: TimeSource) -> bool {
        self.source.set(source).is_ok()
    }

    fn now(&self) -> Duration {
        // The host's reading, or none: then the clock is one tick past its last answer.
        let (reading, step) = match self.source.get() {
            Some(source) => (u64::try_from(source().as_nanos()).unwrap_or(u64::MAX), 0),
            None => (0, Self::TICK_NANOS),
        };
        let mut prev = self.last.load(Ordering::Relaxed);
        loop {
            let next = reading.max(prev.saturating_add(step));
            match self.last.compare_exchange_weak(prev, next, Ordering::Relaxed, Ordering::Relaxed) {
                Ok(_) => return Duration::from_nanos(next),
                Err(actual) => prev = actual,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instants_never_go_backwards_and_compare_by_time() {
        let a = Instant::now();
        let b = Instant::now();
        assert!(b >= a);
        let later = a + Duration::from_secs(2);
        assert_eq!(later.checked_duration_since(a), Some(Duration::from_secs(2)));
        assert_eq!(a.checked_duration_since(later), None);
        assert_eq!(a.saturating_duration_since(later), Duration::ZERO);
        assert_eq!(a.checked_add(Duration::MAX), None);
        assert_eq!((a + Duration::MAX).saturating_duration_since(a), Duration::MAX.saturating_sub(a.0));
    }

    #[test]
    fn a_host_without_a_source_gets_a_logical_clock_that_keeps_moving() {
        // What wasm32 reads when the host installed nothing: every read is later than the last,
        // so a loop waiting for a deadline always ends.
        let clock = HostClock::new();
        let first = clock.now();
        let second = clock.now();
        assert!(second > first);
        let deadline = clock.now() + Duration::from_secs(5);
        let mut reads = 0u32;
        while clock.now() < deadline {
            reads += 1;
            assert!(reads < 10_000, "the logical clock never reached a 5 second deadline");
        }
    }

    #[test]
    fn a_host_source_is_used_and_cannot_move_the_clock_backwards() {
        static NOW_MS: AtomicU64 = AtomicU64::new(0);
        fn source() -> Duration {
            Duration::from_millis(NOW_MS.load(Ordering::Relaxed))
        }
        let clock = HostClock::new();
        assert!(clock.set(source));
        assert!(!clock.set(source), "the first source wins");
        NOW_MS.store(500, Ordering::Relaxed);
        assert_eq!(clock.now(), Duration::from_millis(500));
        NOW_MS.store(200, Ordering::Relaxed);
        assert_eq!(clock.now(), Duration::from_millis(500), "a host clock that steps back is held");
        NOW_MS.store(900, Ordering::Relaxed);
        assert_eq!(clock.now(), Duration::from_millis(900));
    }

    /// `std::time::Instant::now()` and `SystemTime::now()` panic on wasm32-unknown-unknown, where
    /// CI can only build, not run. So the rule is checked in the source: the crate's own code
    /// reads the clock only through this module. Every source file is scanned, so a new module is
    /// checked without being listed; `http` (not built for wasm32), `testing` and this module are
    /// the exceptions.
    #[test]
    fn nothing_but_this_module_reads_the_std_clock() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut scanned = 0;
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            if path.extension().is_none_or(|e| e != "rs") || ["clock.rs", "http.rs", "testing.rs"].contains(&name.as_str()) {
                continue;
            }
            scanned += 1;
            let text = std::fs::read_to_string(&path).unwrap();
            // Test modules run on the host, where the clock exists; only production code counts.
            let production = text.split("#[cfg(test)]").next().unwrap_or(&text);
            for line in production.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let imports_instant = line.contains("std::time::{") && line.contains("Instant");
                for banned in ["std::time::Instant", "SystemTime::now"] {
                    assert!(!line.contains(banned), "{name} reads the std clock ({banned}); use crate::clock");
                }
                assert!(!imports_instant, "{name} imports the std Instant; use crate::clock");
            }
        }
        assert!(scanned >= 10, "the scan found only {scanned} source files");
    }
}
