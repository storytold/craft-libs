//! What a job is and how far along it is: [`JobKind`], [`JobState`], [`Progress`], [`ExpectedTotals`].
//!
//! These are plain data. The local lifecycle of a print or scan job (what it may do next) lives in
//! the domain crate; [`JobState`] is only the one overall state a UI shows.

/// The domain a job belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum JobKind {
    Print,
    Scan,
}

impl JobKind {
    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Print => "print",
            Self::Scan => "scan",
        }
    }
}

/// The overall state of a job, as one value a UI can show for either kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum JobState {
    /// Accepted or queued; nothing has happened yet.
    Pending,
    /// The device is working on it (printing, acquiring).
    Processing,
    /// Held (queue paused, hold-until).
    Held,
    /// Finished without a problem.
    Completed,
    /// Finished, but with errors (some pages failed or are missing).
    CompletedWithErrors,
    /// Cancelled by the user or the device.
    Cancelled,
    /// Stopped by a failure.
    Aborted,
    /// Given up waiting: the device never reported an outcome. The job may or may not have been
    /// carried out, so this is terminal but neither a success nor a known failure.
    Abandoned,
    /// Not finished and stuck until a person acts (paper jam, cover open, feeder empty).
    NeedsAttention,
    /// The device did not say.
    #[default]
    #[cfg_attr(feature = "serde", serde(other))]
    Unknown,
}

impl JobState {
    /// Nothing more will happen to the job.
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithErrors | Self::Cancelled | Self::Aborted | Self::Abandoned)
    }

    /// The job finished cleanly. [`Self::CompletedWithErrors`] is terminal but not a success.
    pub const fn is_success(self) -> bool {
        matches!(self, Self::Completed)
    }

    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::Processing => "processing",
            Self::Held => "held",
            Self::Completed => "completed",
            Self::CompletedWithErrors => "completed_with_errors",
            Self::Cancelled => "cancelled",
            Self::Aborted => "aborted",
            Self::Abandoned => "abandoned",
            Self::NeedsAttention => "needs_attention",
            Self::Unknown => "unknown",
        }
    }
}

/// One progress event: units done so far (sheets, pages) and, **when anyone knows it**, how many
/// there will be. A UI shows a determinate bar when `total` is `Some` and an indeterminate one
/// otherwise.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct Progress {
    pub done: u32,
    pub total: Option<u32>,
}

impl Progress {
    pub const fn new(done: u32, total: Option<u32>) -> Self {
        Self { done, total }
    }

    /// An event with a known total.
    pub const fn of(done: u32, total: u32) -> Self {
        Self { done, total: Some(total) }
    }

    /// An event whose total is not known.
    pub const fn done_only(done: u32) -> Self {
        Self { done, total: None }
    }

    /// `done / total` in `0.0..=1.0` when the total is known and not zero; `done` beyond `total`
    /// reads as complete.
    pub fn fraction(&self) -> Option<f32> {
        match self.total {
            Some(total) if total > 0 => Some((self.done.min(total) as f32) / (total as f32)),
            _ => None,
        }
    }

    /// Fill a missing (or zero) total from `expected`; a total that is present (a device-reported
    /// one) is kept. A zero `expected` is "unknown".
    #[must_use]
    pub fn or_expected(self, expected: Option<u32>) -> Self {
        match self.total {
            Some(total) if total > 0 => self,
            _ => Self { done: self.done, total: expected.filter(|t| *t > 0) },
        }
    }
}

/// The totals a caller expects for a job, used only where the device reports none.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(default))]
pub struct ExpectedTotals {
    /// Pages of the document.
    pub pages: Option<u32>,
    /// Sheets the job should use: pages halved (rounded up) for two-sided jobs, times the copies.
    /// An estimate; collation, booklet layouts and the device's own counting can differ.
    pub sheets: Option<u32>,
}

impl ExpectedTotals {
    /// What a request expects. Zero pages is "unknown"; zero copies count as one.
    pub fn for_request(pages: Option<u32>, two_sided: bool, copies: u32) -> Self {
        let Some(pages) = pages.filter(|p| *p > 0) else {
            return Self::default();
        };
        let per_sheet = if two_sided { 2 } else { 1 };
        Self { pages: Some(pages), sheets: Some(pages.div_ceil(per_sheet).saturating_mul(copies.max(1))) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn state_classification() {
        use JobState::*;
        for s in [Completed, CompletedWithErrors, Cancelled, Aborted, Abandoned] {
            assert!(s.is_terminal(), "{s:?}");
        }
        for s in [Pending, Processing, Held, NeedsAttention, Unknown] {
            assert!(!s.is_terminal(), "{s:?}");
            assert!(!s.is_success(), "{s:?}");
        }
        assert!(Completed.is_success());
        assert!(!CompletedWithErrors.is_success() && !Cancelled.is_success() && !Aborted.is_success() && !Abandoned.is_success());
        assert_eq!(JobState::default(), Unknown);
        assert_eq!(CompletedWithErrors.name(), "completed_with_errors");
        assert_eq!(NeedsAttention.name(), "needs_attention");
        assert_eq!(JobKind::Print.name(), "print");
        assert_eq!(JobKind::Scan.name(), "scan");
    }

    #[test]
    fn progress_constructors_and_fraction() {
        assert_eq!(Progress::new(2, Some(5)), Progress::of(2, 5));
        assert_eq!(Progress::default(), Progress::done_only(0));
        assert_eq!(Progress::of(1, 4).fraction(), Some(0.25));
        assert_eq!(Progress::of(9, 4).fraction(), Some(1.0));
        assert_eq!(Progress::of(1, 0).fraction(), None);
        assert_eq!(Progress::done_only(1).fraction(), None);
        assert_eq!(Progress::of(u32::MAX, u32::MAX).fraction(), Some(1.0));
    }

    #[test]
    fn or_expected_fills_only_what_is_missing() {
        assert_eq!(Progress::done_only(1).or_expected(Some(6)), Progress::of(1, 6));
        assert_eq!(Progress::of(1, 0).or_expected(Some(6)), Progress::of(1, 6), "a zero total is missing");
        assert_eq!(Progress::of(1, 4).or_expected(Some(6)), Progress::of(1, 4), "a reported total wins");
        assert_eq!(Progress::done_only(1).or_expected(None), Progress::done_only(1));
        assert_eq!(Progress::done_only(1).or_expected(Some(0)), Progress::done_only(1));
        assert_eq!(Progress::of(1, 0).or_expected(Some(0)), Progress::done_only(1));
    }

    #[test]
    fn expected_totals() {
        assert_eq!(ExpectedTotals::for_request(Some(5), false, 1), ExpectedTotals { pages: Some(5), sheets: Some(5) });
        assert_eq!(ExpectedTotals::for_request(Some(5), true, 1).sheets, Some(3));
        assert_eq!(ExpectedTotals::for_request(Some(5), true, 3).sheets, Some(9));
        assert_eq!(ExpectedTotals::for_request(Some(5), false, 0).sheets, Some(5), "zero copies count as one");
        assert_eq!(ExpectedTotals::for_request(Some(0), true, 2), ExpectedTotals::default());
        assert_eq!(ExpectedTotals::for_request(None, false, 1), ExpectedTotals::default());
        assert_eq!(ExpectedTotals::for_request(Some(u32::MAX), false, u32::MAX).sheets, Some(u32::MAX), "saturates");
    }
}
