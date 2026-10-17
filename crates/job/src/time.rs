//! Caller-supplied timestamps. The crate never reads a clock; the domain crate stamps the job.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// A point in time as milliseconds since the Unix epoch. One type for every timestamp on a job.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Timestamp(u64);

impl Timestamp {
    pub const fn from_millis(ms: u64) -> Self {
        Self(ms)
    }

    /// Seconds since the epoch; saturates instead of overflowing.
    pub const fn from_secs(secs: u64) -> Self {
        Self(secs.saturating_mul(1000))
    }

    /// From a `SystemTime`; `None` before 1970. Beyond `u64` milliseconds saturates.
    pub fn from_system_time(time: SystemTime) -> Option<Self> {
        let d = time.duration_since(UNIX_EPOCH).ok()?;
        Some(Self(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)))
    }

    pub const fn millis(self) -> u64 {
        self.0
    }

    pub const fn secs(self) -> u64 {
        self.0 / 1000
    }

    #[must_use]
    pub fn saturating_add(self, d: Duration) -> Self {
        Self(self.0.saturating_add(u64::try_from(d.as_millis()).unwrap_or(u64::MAX)))
    }

    /// The distance between two timestamps, whichever is later.
    pub const fn abs_diff(self, other: Self) -> Duration {
        Duration::from_millis(self.0.abs_diff(other.0))
    }
}

/// When things happened to a job. Every field is optional: a device may not say.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct JobTimes {
    pub created: Option<Timestamp>,
    pub started: Option<Timestamp>,
    pub finished: Option<Timestamp>,
}

impl JobTimes {
    pub fn new() -> Self {
        Self::default()
    }

    #[must_use]
    pub const fn with_created(mut self, t: Timestamp) -> Self {
        self.created = Some(t);
        self
    }

    #[must_use]
    pub const fn with_started(mut self, t: Timestamp) -> Self {
        self.started = Some(t);
        self
    }

    #[must_use]
    pub const fn with_finished(mut self, t: Timestamp) -> Self {
        self.finished = Some(t);
        self
    }

    /// From start (else creation) to finish, when both are known and in order.
    pub fn elapsed(&self) -> Option<Duration> {
        let (from, to) = (self.started.or(self.created)?, self.finished?);
        (to >= from).then(|| to.abs_diff(from))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conversions() {
        assert_eq!(Timestamp::from_secs(2).millis(), 2000);
        assert_eq!(Timestamp::from_millis(2999).secs(), 2);
        assert_eq!(Timestamp::from_secs(u64::MAX).millis(), u64::MAX);
        assert_eq!(Timestamp::from_system_time(UNIX_EPOCH + Duration::from_millis(1500)), Some(Timestamp::from_millis(1500)));
        assert_eq!(Timestamp::from_system_time(UNIX_EPOCH), Some(Timestamp::default()));
        assert_eq!(Timestamp::from_system_time(UNIX_EPOCH - Duration::from_secs(1)), None);
    }

    #[test]
    fn arithmetic_saturates() {
        let t = Timestamp::from_millis(10);
        assert_eq!(t.saturating_add(Duration::from_millis(5)).millis(), 15);
        assert_eq!(t.saturating_add(Duration::MAX).millis(), u64::MAX);
        assert_eq!(Timestamp::from_millis(u64::MAX).saturating_add(Duration::from_secs(1)).millis(), u64::MAX);
        assert_eq!(t.abs_diff(Timestamp::from_millis(40)), Duration::from_millis(30));
        assert_eq!(Timestamp::from_millis(40).abs_diff(t), Duration::from_millis(30));
    }

    #[test]
    fn elapsed_needs_both_ends_in_order() {
        let t = Timestamp::from_secs;
        assert_eq!(JobTimes::new().elapsed(), None);
        assert_eq!(JobTimes::new().with_started(t(1)).elapsed(), None);
        assert_eq!(JobTimes::new().with_started(t(1)).with_finished(t(4)).elapsed(), Some(Duration::from_secs(3)));
        assert_eq!(JobTimes::new().with_created(t(1)).with_finished(t(2)).elapsed(), Some(Duration::from_secs(1)), "creation stands in for start");
        assert_eq!(JobTimes::new().with_created(t(1)).with_started(t(2)).with_finished(t(5)).elapsed(), Some(Duration::from_secs(3)), "start wins");
        assert_eq!(JobTimes::new().with_started(t(5)).with_finished(t(1)).elapsed(), None, "out of order");
    }
}
