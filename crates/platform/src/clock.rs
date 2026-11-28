//! The app's clock: monotonic seconds for animation, autosave, deadlines and background work, and
//! a manual mode tests set by hand.
//!
//! `Context::input(|i| i.time)` is the UI pass's time: it is stale in `App::logic` and doesn't
//! move while the window is hidden. Wall-clock time can jump. [`Clock`] reads
//! `web_time::Instant`, which is monotonic on native targets and in the browser (`performance.now()`
//! there, where `std::time::Instant` panics). A test switches a clock to manual time with
//! [`Clock::set`] or starts one with [`Clock::manual`], and moves it with [`Clock::advance`], so
//! a deadline or an animation is checked without sleeping.
//!
//! ```ignore
//! let mut clock = Clock::manual(0.0);
//! app.autosave_due = clock.now() + AUTOSAVE_SECS;
//! clock.advance(AUTOSAVE_SECS);
//! assert!(app.autosave_due <= clock.now());
//! ```
//!
//! [`monotonic_ms`] is the process-wide clock for code that holds no `Clock` (profilers, log
//! lines); [`unix_ms`] is wall time, only for names that must be unique across runs.

/// Monotonic seconds since the clock was made, or the manual time a test set.
///
/// `Copy`, so a host hands it to the parts that need it; a test sets the copy the app reads.
#[derive(Clone, Copy, Debug)]
pub struct Clock {
    origin: web_time::Instant,
    manual: Option<f64>,
}

impl Default for Clock {
    /// A real clock starting now ([`Clock::new`]).
    fn default() -> Self {
        Self::new()
    }
}

impl Clock {
    /// A real clock starting now: [`Self::now`] reads 0.0 at once and counts up.
    pub fn new() -> Self {
        Self { origin: web_time::Instant::now(), manual: None }
    }

    /// A manual clock reading `at` seconds until a test sets or advances it.
    pub fn manual(at: f64) -> Self {
        Self { origin: web_time::Instant::now(), manual: Some(at) }
    }

    /// Seconds on the clock: the manual time when set, otherwise the time since [`Self::new`].
    pub fn now(&self) -> f64 {
        self.manual.unwrap_or_else(|| self.origin.elapsed().as_secs_f64())
    }

    /// Switch to manual time `at` seconds; `None` returns to the real clock (which kept counting
    /// from [`Self::new`] meanwhile).
    pub fn set(&mut self, at: Option<f64>) {
        self.manual = at;
    }

    /// Move the clock `secs` forward. A real clock becomes manual at `now() + secs`; a negative
    /// or non-finite `secs` leaves the clock where it is (monotonic).
    pub fn advance(&mut self, secs: f64) {
        if secs.is_finite() && secs >= 0.0 {
            self.manual = Some(self.now() + secs);
        }
    }

    /// Whether a test set this clock's time.
    pub fn is_manual(&self) -> bool {
        self.manual.is_some()
    }
}

/// Monotonic milliseconds since the first call in this process, for durations and deadlines.
pub fn monotonic_ms() -> f64 {
    static START: std::sync::OnceLock<web_time::Instant> = std::sync::OnceLock::new();
    START.get_or_init(web_time::Instant::now).elapsed().as_secs_f64() * 1000.0
}

/// Wall-clock milliseconds since the Unix epoch, 0.0 for a clock set before 1970. Only for names
/// that must be unique across runs (`library-restored-<ms>`), never for timing or deadlines.
pub fn unix_ms() -> f64 {
    use web_time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64() * 1000.0).unwrap_or(0.0)
}
