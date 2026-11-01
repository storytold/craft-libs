//! What a backend hands back once a printer has accepted a document, and how a job's progress is
//! described afterwards. See "Job lifecycle" in the crate README for the reasoning.
//!
//! Submitting and watching are separate calls. [`crate::backends::Backend::submit`] returns a
//! [`Submitted`] as soon as the printer has accepted the document; everything after that (is it
//! printing, did it finish, cancel it) is a [`JobId`] plus one bounded request at a time
//! ([`crate::backends::Backend::job_status`], [`crate::backends::Backend::cancel_job`]), driven by
//! the caller (see [`crate::JobWatch`]).

use std::time::{Duration, SystemTime};

use craft_job::{ExpectedTotals, JobId, JobState, ParseIdError, Tag};

use crate::backend::{ErrorKind, PrintError, Receipt};
use crate::convert::Conversion;
use crate::options::JobOptions;
use crate::{Duplex, Job};

// `JobId`, `ParseIdError`, `JobState` and `ExpectedTotals` are `craft-job`'s (shared with scanning);
// the crate root re-exports them, so `craft_print::JobId` and the others keep their paths.

/// A malformed job id is a bad request for [`PrintError`] consumers.
impl From<ParseIdError> for PrintError {
    fn from(error: ParseIdError) -> Self {
        PrintError::new(ErrorKind::InvalidRequest, error.to_string())
    }
}

/// Printing's additions to [`ExpectedTotals`]: reading the totals off a request and filling them
/// into a status reading.
pub trait ExpectedTotalsExt {
    /// What a request expects ([`ExpectedTotals::for_request`] over the job's duplex and copies and
    /// the option's page count). Zero pages is "unknown".
    fn for_job(job: &Job, options: &JobOptions) -> ExpectedTotals;
    /// Fill the totals a reading lacks; a total the backend reported is kept.
    fn apply(&self, status: &mut JobStatus);
    /// Whether anything is expected.
    fn is_empty(&self) -> bool;
}

impl ExpectedTotalsExt for ExpectedTotals {
    fn for_job(job: &Job, options: &JobOptions) -> ExpectedTotals {
        let two_sided = matches!(job.duplex, Some(Duplex::LongEdge | Duplex::ShortEdge));
        ExpectedTotals::for_request(options.expected_pages, two_sided, job.copies)
    }

    fn apply(&self, status: &mut JobStatus) {
        if status.pages_total.is_none() {
            status.pages_total = self.pages.filter(|p| *p > 0);
        }
        if status.sheets_total.is_none() {
            status.sheets_total = self.sheets.filter(|s| *s > 0);
        }
    }

    fn is_empty(&self) -> bool {
        self.pages.is_none() && self.sheets.is_none()
    }
}

/// One reading of a job's status. Counters are `None` when the backend does not publish them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobStatus {
    pub state: JobState,
    /// The device's reasons, as reported (`"JobPrinting"`, `"media-empty"`); empty when none.
    pub reasons: Vec<String>,
    /// What the printer says went slightly wrong in a job that otherwise completed (text for
    /// people). Empty when nothing. This is how "completed with warnings" is modelled: the state is
    /// [`JobState::Completed`] and this list says more; a [`crate::PrintJob`] then ends as
    /// [`crate::Outcome::CompletedWithWarnings`].
    pub warnings: Vec<String>,
    pub sheets_done: Option<u32>,
    pub sheets_total: Option<u32>,
    pub pages_done: Option<u32>,
    pub pages_total: Option<u32>,
    /// When this reading was taken (the caller's wall clock, not the printer's).
    pub updated_at: SystemTime,
}

impl JobStatus {
    /// A reading with this state and no counters or reasons, taken now.
    pub fn new(state: JobState) -> Self {
        Self {
            state,
            reasons: Vec::new(),
            warnings: Vec::new(),
            sheets_done: None,
            sheets_total: None,
            pages_done: None,
            pages_total: None,
            updated_at: crate::clock::wall_now(),
        }
    }

    /// "Accepted, outcome unknown": what a backend that cannot report status answers.
    pub fn unknown() -> Self {
        Self::new(JobState::Unknown)
    }

    /// Whether the job has ended for good ([`JobState::is_terminal`]).
    pub const fn terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Fraction of the sheets done in `0.0..=1.0`, when both counters are known and the total is
    /// not zero.
    pub fn sheet_fraction(&self) -> Option<f32> {
        match (self.sheets_done, self.sheets_total) {
            (Some(done), Some(total)) if total > 0 => Some((done.min(total) as f32) / (total as f32)),
            _ => None,
        }
    }

    /// Whether two readings say the same thing (ignoring when they were taken); a watcher uses it
    /// to slow down while nothing changes.
    pub fn same_as(&self, other: &Self) -> bool {
        self.state == other.state
            && self.reasons == other.reasons
            && self.warnings == other.warnings
            && self.sheets_done == other.sheets_done
            && self.sheets_total == other.sheets_total
            && self.pages_done == other.pages_done
            && self.pages_total == other.pages_total
    }
}

/// Whether and how a backend can tell what happens to a job after it was accepted.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
#[non_exhaustive]
pub enum WatchSupport {
    /// It cannot: [`crate::backends::Backend::job_status`] answers [`JobState::Unknown`] and a UI
    /// should not poll. "Accepted, outcome unknown" is the final answer.
    #[default]
    None,
    /// Ask again every [`crate::backends::Backend::recommended_poll_interval`].
    Poll,
    /// The device can push changes and the backend currently trusts the channel. A push is only a
    /// wake-up ([`crate::backends::Backend::wake_source`]): status always comes from polling, and
    /// a [`crate::JobWatch`] polls at its (longer) safety interval in between. The WS-Print
    /// backend reports it only when the app opted in (`WsdBackend::with_events`).
    Events,
}

impl WatchSupport {
    /// Whether a caller should ask for the status at all.
    pub const fn is_watchable(self) -> bool {
        !matches!(self, Self::None)
    }
}

/// What [`crate::backends::Backend::submit`] returns once the printer has **accepted** the
/// document. It says nothing about the paper: follow the job with its [`JobId`] if the backend
/// can ([`Submitted::watch`]).
///
/// The call that produced it is bounded by the backend's per-request timeouts only; it never waits
/// for the job to print.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Submitted {
    /// The backend's registry name.
    pub backend: &'static str,
    /// The accepted job, when the backend has a handle for it (the built-in spooler has none).
    pub job: Option<JobId>,
    /// When the printer accepted the document (the caller's wall clock).
    pub accepted_at: SystemTime,
    /// The job's state at that moment, when the reply carried it or a read was cheap.
    pub status: Option<JobStatus>,
    /// Settings that were dropped or changed on the way.
    pub warnings: Vec<String>,
    /// A sentence for people ("job 7 on the printer was accepted").
    pub message: String,
    /// Filled in by the registry from the backend; see [`WatchSupport`].
    pub watch: WatchSupport,
    /// Filled in by the registry from the backend; `None` when there is nothing to poll.
    pub poll_interval: Option<Duration>,
    /// Filled in by the registry from the request's [`JobOptions::expected_pages`]; see
    /// [`ExpectedTotals`]. [`crate::JobWatch::for_submitted`] uses it.
    pub expected: ExpectedTotals,
    /// Filled in by the registry: the idempotency tag the backend carried for this job, `None` when
    /// the request had none or the backend cannot carry one.
    pub tag: Option<Tag>,
    /// How the backend sent the request, in its own words, for diagnostics (WS-Print: the ticket
    /// shape). Not for parsing.
    pub effective: Option<String>,
    /// Filled in by the registry when the app's [`DocumentConverter`](crate::DocumentConverter)
    /// had to make the document the printer accepts; `None` when it was sent as it was.
    pub conversion: Option<Conversion>,
}

impl Submitted {
    /// An accepted job with no handle and nothing known about it.
    pub fn new(backend: &'static str, message: impl Into<String>) -> Self {
        Self {
            backend,
            job: None,
            accepted_at: crate::clock::wall_now(),
            status: None,
            warnings: Vec::new(),
            message: message.into(),
            watch: WatchSupport::None,
            poll_interval: None,
            expected: ExpectedTotals::default(),
            tag: None,
            effective: None,
            conversion: None,
        }
    }

    /// A short, backend-specific description of how the request was sent (diagnostics only).
    #[must_use]
    pub fn with_effective(mut self, effective: impl Into<String>) -> Self {
        self.effective = Some(effective.into());
        self
    }

    #[must_use]
    pub fn with_job(mut self, job: JobId) -> Self {
        self.job = Some(job);
        self
    }

    #[must_use]
    pub fn with_status(mut self, status: JobStatus) -> Self {
        self.status = Some(status);
        self
    }

    #[must_use]
    pub fn with_warnings(mut self, warnings: Vec<String>) -> Self {
        self.warnings = warnings;
        self
    }

    /// The older receipt view: the message and the backend's own job id.
    pub fn receipt(&self) -> Receipt {
        Receipt { backend: self.backend, message: self.message.clone(), job_id: self.job.as_ref().map(|j| j.native().to_owned()) }
    }
}

/// A receipt becomes an accepted job whose handle is the receipt's job id (so it is only
/// watchable if the backend overrides [`crate::backends::Backend::job_status`]).
impl From<Receipt> for Submitted {
    fn from(receipt: Receipt) -> Self {
        let mut submitted = Submitted::new(receipt.backend, receipt.message);
        submitted.job = receipt.job_id.map(|native| JobId::new(receipt.backend, native));
        submitted
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn terminal_states_are_exactly_the_four_endings() {
        use JobState as S;
        let all = [S::Pending, S::Held, S::Processing, S::NeedsAttention, S::Completed, S::CompletedWithErrors, S::Cancelled, S::Aborted, S::Unknown];
        let terminal: Vec<_> = all.iter().filter(|s| s.is_terminal()).map(|s| s.name()).collect();
        assert_eq!(terminal, ["completed", "completed_with_errors", "cancelled", "aborted"]);
        assert!(!S::Unknown.is_terminal(), "unknown is 'accepted, outcome unknown', not an ending");
        assert!(!S::NeedsAttention.is_terminal(), "a stopped printer can be fixed");
        assert!(S::Completed.is_success() && !S::CompletedWithErrors.is_success());
        assert!(JobStatus::new(S::Cancelled).terminal() && !JobStatus::unknown().terminal());
    }

    #[test]
    fn warnings_are_part_of_a_reading_but_not_of_the_state() {
        let mut a = JobStatus::new(JobState::Completed);
        let b = a.clone();
        assert!(a.same_as(&b));
        a.warnings.push("job completed with warnings".into());
        assert!(!a.same_as(&b), "a new warning is a change");
        assert_eq!(a.state, b.state);
    }

    #[test]
    fn job_ids_round_trip_through_their_text_form() {
        let job = JobId::new("wsd", "http://192.0.2.1:5357/p#7");
        assert_eq!((job.backend(), job.native()), ("wsd", "http://192.0.2.1:5357/p#7"));
        assert_eq!(job.to_string(), "wsd:http://192.0.2.1:5357/p#7");
        assert_eq!(job.to_string().parse::<JobId>(), Ok(job.clone()));
        assert_eq!(JobId::try_new("wsd", "http://192.0.2.1:5357/p#7"), Ok(job));
    }

    #[test]
    fn malformed_job_ids_are_typed_errors() {
        use craft_job::ParseIdErrorKind as K;
        let kind = |text: &str| text.parse::<JobId>().map_err(|e| e.kind());
        assert_eq!(kind("no-prefix"), Err(K::MissingSeparator));
        assert_eq!(kind(""), Err(K::MissingSeparator));
        assert_eq!(kind(":7"), Err(K::EmptyBackend));
        assert_eq!(kind("Wsd:7"), Err(K::InvalidBackend));
        assert_eq!(kind("a b:7"), Err(K::InvalidBackend));
        assert_eq!(kind("wsd:"), Err(K::EmptyNative));
        let error: PrintError = "x".parse::<JobId>().map(|_| ()).unwrap_err().into();
        assert_eq!(error.kind(), ErrorKind::InvalidRequest);
    }

    #[test]
    fn expected_totals_fill_only_what_the_backend_did_not_report() {
        let totals = ExpectedTotals { pages: Some(8), sheets: Some(4) };
        let mut status = JobStatus::new(JobState::Processing);
        status.sheets_done = Some(1);
        totals.apply(&mut status);
        assert_eq!((status.sheets_total, status.pages_total), (Some(4), Some(8)));
        assert_eq!(status.sheet_fraction(), Some(0.25));
        let mut reported = JobStatus::new(JobState::Processing);
        reported.sheets_total = Some(10);
        totals.apply(&mut reported);
        assert_eq!(reported.sheets_total, Some(10), "a backend-reported total wins");
        let mut zero = JobStatus::new(JobState::Processing);
        ExpectedTotals { pages: Some(0), sheets: Some(0) }.apply(&mut zero);
        assert_eq!((zero.sheets_total, zero.pages_total), (None, None), "zero is no total");
    }

    #[test]
    fn expected_sheets_follow_duplex_and_copies() {
        use crate::{Duplex, Job};
        let options = JobOptions::new().with_expected_pages(5);
        let simplex = ExpectedTotals::for_job(&Job::default(), &options);
        assert_eq!((simplex.pages, simplex.sheets), (Some(5), Some(5)));
        let duplex = Job { duplex: Some(Duplex::LongEdge), copies: 2, ..Job::default() };
        let both = ExpectedTotals::for_job(&duplex, &options);
        assert_eq!((both.pages, both.sheets), (Some(5), Some(6)));
        assert_eq!(ExpectedTotals::for_job(&Job::default(), &JobOptions::new()), ExpectedTotals::default());
    }

    #[test]
    fn sheet_fraction_needs_both_counters() {
        let mut s = JobStatus::new(JobState::Processing);
        assert_eq!(s.sheet_fraction(), None);
        s.sheets_done = Some(1);
        assert_eq!(s.sheet_fraction(), None);
        s.sheets_total = Some(4);
        assert_eq!(s.sheet_fraction(), Some(0.25));
        s.sheets_total = Some(0);
        assert_eq!(s.sheet_fraction(), None);
        s.sheets_total = Some(1);
        s.sheets_done = Some(5);
        assert_eq!(s.sheet_fraction(), Some(1.0));
    }

    #[test]
    fn a_receipt_converts_and_the_receipt_view_comes_back() {
        let receipt = Receipt { backend: "cups", message: "request id is Office-1".into(), job_id: Some("Office-1".into()) };
        let submitted = Submitted::from(receipt.clone());
        assert_eq!(submitted.job.as_ref().map(|j| j.native()), Some("Office-1"));
        assert_eq!(submitted.watch, WatchSupport::None);
        assert_eq!(submitted.receipt(), receipt);
    }
}
