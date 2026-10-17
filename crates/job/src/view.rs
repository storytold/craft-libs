//! [`JobView`]: the one snapshot a UI renders for a print job or a scan job.
//!
//! Each domain crate builds a `JobView` from its own job object; this crate only defines the
//! shape. It is plain data with public fields.

use crate::document::JobDocuments;
use crate::id::JobId;
use crate::job::{JobKind, JobState, Progress};
use crate::native::NativeCode;
use crate::retry::RetryAdvice;
use crate::tag::Tag;
use crate::time::JobTimes;

/// The last failure, in the terms a UI needs.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ErrorSummary {
    /// The sentence for a person.
    pub message: String,
    /// The domain's error kind as a stable label (`"busy"`, `"jam"`); the kind types stay in the domain crate.
    pub kind_label: String,
    /// What to do about it, when the domain crate decided.
    #[cfg_attr(feature = "serde", serde(default))]
    pub retry_advice: Option<RetryAdvice>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub native: Option<NativeCode>,
}

impl ErrorSummary {
    pub fn new(message: impl Into<String>, kind_label: impl Into<String>) -> Self {
        Self { message: message.into(), kind_label: kind_label.into(), retry_advice: None, native: None }
    }

    #[must_use]
    pub fn with_retry_advice(mut self, advice: RetryAdvice) -> Self {
        self.retry_advice = Some(advice);
        self
    }

    #[must_use]
    pub fn with_native(mut self, native: NativeCode) -> Self {
        self.native = Some(native);
        self
    }
}

/// A snapshot of one job for display.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct JobView {
    pub id: JobId,
    pub kind: JobKind,
    pub title: String,
    pub state: JobState,
    /// Device-reported reasons ("media-empty", "cover open"), as text.
    #[cfg_attr(feature = "serde", serde(default))]
    pub reasons: Vec<String>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub progress: Option<Progress>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub times: JobTimes,
    #[cfg_attr(feature = "serde", serde(default))]
    pub documents: JobDocuments,
    /// Attempts made so far (submits for print, acquisitions for scan).
    #[cfg_attr(feature = "serde", serde(default))]
    pub attempts: u32,
    #[cfg_attr(feature = "serde", serde(default))]
    pub last_error: Option<ErrorSummary>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub tag: Option<Tag>,
    /// The printer or scanner, by display name.
    #[cfg_attr(feature = "serde", serde(default))]
    pub device_name: Option<String>,
}

impl JobView {
    pub fn new(id: JobId, kind: JobKind, title: impl Into<String>, state: JobState) -> Self {
        Self {
            id,
            kind,
            title: title.into(),
            state,
            reasons: Vec::new(),
            progress: None,
            times: JobTimes::default(),
            documents: JobDocuments::default(),
            attempts: 0,
            last_error: None,
            tag: None,
            device_name: None,
        }
    }

    #[must_use]
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reasons.push(reason.into());
        self
    }

    #[must_use]
    pub fn with_progress(mut self, progress: Progress) -> Self {
        self.progress = Some(progress);
        self
    }

    #[must_use]
    pub fn with_times(mut self, times: JobTimes) -> Self {
        self.times = times;
        self
    }

    #[must_use]
    pub fn with_documents(mut self, documents: JobDocuments) -> Self {
        self.documents = documents;
        self
    }

    #[must_use]
    pub fn with_attempts(mut self, attempts: u32) -> Self {
        self.attempts = attempts;
        self
    }

    #[must_use]
    pub fn with_error(mut self, error: ErrorSummary) -> Self {
        self.last_error = Some(error);
        self
    }

    #[must_use]
    pub fn with_tag(mut self, tag: Tag) -> Self {
        self.tag = Some(tag);
        self
    }

    #[must_use]
    pub fn with_device_name(mut self, name: impl Into<String>) -> Self {
        self.device_name = Some(name.into());
        self
    }

    /// Nothing more will happen to the job.
    pub fn is_terminal(&self) -> bool {
        self.state.is_terminal()
    }

    /// Whether the UI should draw attention: the job is stuck on a person, or ended badly.
    pub fn needs_attention(&self) -> bool {
        matches!(self.state, JobState::NeedsAttention | JobState::Aborted | JobState::Abandoned | JobState::CompletedWithErrors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::document::DocumentRef;
    use crate::time::Timestamp;

    #[test]
    fn a_print_job_and_a_scan_job_fill_the_same_view() {
        let print = JobView::new(JobId::new("wsd", "urn:1"), JobKind::Print, "Report", JobState::Processing)
            .with_progress(Progress::of(1, 4))
            .with_documents(JobDocuments::new().with(DocumentRef::new("report.pdf").with_format("pdf").with_pages(4)))
            .with_attempts(1)
            .with_tag(Tag::from_seed(1))
            .with_device_name("Office printer");
        let scan = JobView::new(JobId::new("wia", "7"), JobKind::Scan, "Scan", JobState::NeedsAttention)
            .with_reason("feeder empty")
            .with_progress(Progress::done_only(2))
            .with_times(JobTimes::new().with_started(Timestamp::from_secs(5)))
            .with_documents(JobDocuments::new().with(DocumentRef::new("page-1.png").with_format("png")))
            .with_error(
                ErrorSummary::new("The feeder is empty.", "feeder_empty")
                    .with_retry_advice(RetryAdvice::DoNotRetry)
                    .with_native(NativeCode::hresult(0x8021_0003, None)),
            );
        assert!(!print.is_terminal() && !print.needs_attention());
        assert!(!scan.is_terminal() && scan.needs_attention());
        assert_eq!(print.kind, JobKind::Print);
        assert_eq!(scan.reasons, ["feeder empty"]);
        assert_eq!(scan.last_error.as_ref().map(|e| e.kind_label.as_str()), Some("feeder_empty"));
        assert_eq!(print.progress.and_then(|p| p.fraction()), Some(0.25));
        assert_eq!(print.documents.total_pages(), Some(4));
        assert_eq!(print.clone(), print);
    }

    #[test]
    fn defaults_are_empty() {
        let v = JobView::new(JobId::new("a", "b"), JobKind::Scan, "t", JobState::Unknown);
        assert!(v.reasons.is_empty() && v.progress.is_none() && v.documents.is_empty());
        assert!(v.last_error.is_none() && v.tag.is_none() && v.device_name.is_none());
        assert_eq!((v.attempts, v.times), (0, JobTimes::default()));
    }

    #[test]
    fn needs_attention_and_terminal_follow_the_state() {
        let v = |s| JobView::new(JobId::new("a", "b"), JobKind::Print, "t", s);
        assert!(v(JobState::Aborted).needs_attention() && v(JobState::Aborted).is_terminal());
        assert!(v(JobState::CompletedWithErrors).needs_attention());
        assert!(!v(JobState::Completed).needs_attention() && v(JobState::Completed).is_terminal());
        assert!(!v(JobState::Cancelled).needs_attention());
        assert!(!v(JobState::Held).needs_attention() && !v(JobState::Held).is_terminal());
    }

    #[test]
    fn error_summary_builders() {
        let e = ErrorSummary::new("m", "k");
        assert_eq!((e.retry_advice, e.native), (None, None));
        let e = e.with_retry_advice(RetryAdvice::CheckDeviceFirst).with_native(NativeCode::new("http", 503, None));
        assert_eq!(e.retry_advice, Some(RetryAdvice::CheckDeviceFirst));
        assert_eq!(e.native.map(|n| n.code), Some(503));
    }
}
