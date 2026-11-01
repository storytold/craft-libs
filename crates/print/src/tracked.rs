//! [`PrintJob`]: the lifecycle record of one user-visible print, stable across retries.
//!
//! A backend follows one remote job ([`crate::JobWatch`]). A person pressed Print once and expects
//! one sheet of paper, however many connections, retries and printer-side restarts it takes. A
//! `PrintJob` holds that print: it sends the document, follows the job, retries what is safe to
//! retry, refuses to retry what could print twice, reconciles when the outcome of a send is unknown,
//! and keeps the history. It is driven by the caller with [`PrintJob::advance`]: no thread, no
//! timer, no async runtime, all waiting on an injectable [`Clock`]. Everything is in memory.
//!
//! ```
//! use std::sync::Arc;
//! use craft_print::testing::FakeBackend;
//! use craft_print::{BytesSource, DocumentFormat, Job, JobState, JobStatus, LifecycleState, ManualClock, Outcome, PrintJob, Registry, WatchSupport};
//!
//! let fake = FakeBackend::new("fake")
//!     .with_printer("p1", true)
//!     .with_watch(WatchSupport::Poll)
//!     .with_status_script(vec![Ok(JobStatus::new(JobState::Processing)), Ok(JobStatus::new(JobState::Completed))]);
//! let registry = Registry::new();
//! registry.register(Box::new(fake))?;
//!
//! let clock = Arc::new(ManualClock::new());
//! let mut job = PrintJob::new(Job::default(), BytesSource::new(DocumentFormat::Pdf, b"%PDF-1.7".to_vec())).with_clock(clock.clone());
//! while let Some(wait) = job.advance(&registry).next_in {
//!     clock.advance(wait); // an app returns to its frame loop instead
//! }
//! assert_eq!(job.snapshot().state, LifecycleState::Terminal(Outcome::Completed));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::borrow::Cow;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime};

use craft_job::{
    DocumentRef, ErrorSummary, ExpectedTotals, Fingerprint, Jitter, JobDocuments, JobId, JobKind, JobState, JobTimes, JobView, NativeCode, Progress,
    RetryAdvice, RetryPolicy, RetrySafety, Tag, Timestamp,
};

use crate::Job;
use crate::backend::{CancelToken, Document, DocumentFormat, ErrorKind, NotFoundSubject, PrintError, SubmitRequest, UserConfirmed};
use crate::convert::Conversion;
use crate::job::{ExpectedTotalsExt as _, JobStatus, Submitted};
use crate::options::JobOptions;
use crate::registry::{BackendInfo, Registry};
use crate::remote::{Reconciled, RemoteJob};
use crate::retry::{Clock, JitterExt as _, Phase, SystemClock, advise_error};
use crate::tag::{TagExt as _, TagPolicy, TagSupportExt as _};
use crate::watch::{JobWatch, WaitPolicy, WatchOutcome, WatchStep};

/// The id of a [`PrintJob`] inside this process. It never changes across retries; the remote
/// [`JobId`]s of the attempts come and go.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocalJobId(u64);

impl LocalJobId {
    /// The number behind the id; ids are handed out from 1 in creation order within the process.
    pub fn get(self) -> u64 {
        self.0
    }
}

impl std::fmt::Display for LocalJobId {
    /// `print-{n}`.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "print-{}", self.0)
    }
}

/// Where a [`PrintJob`] gets its document. The job does not keep the bytes: it asks the source for
/// them again for every attempt, so an app can re-read a file (or regenerate the document) instead of
/// holding a large buffer for the life of the print.
pub trait DocumentSource: Send {
    /// The format of the bytes [`Self::read`] returns.
    fn format(&self) -> DocumentFormat;
    /// The document. An error is an attempt that failed before anything was sent.
    fn read(&self) -> Result<Cow<'_, [u8]>, PrintError>;
}

/// A [`DocumentSource`] over bytes the app already holds in memory.
#[derive(Clone, Debug)]
pub struct BytesSource {
    format: DocumentFormat,
    bytes: Vec<u8>,
}

impl BytesSource {
    /// A source that hands out `bytes` as a document of `format`; nothing is checked until submit.
    pub fn new(format: DocumentFormat, bytes: Vec<u8>) -> Self {
        Self { format, bytes }
    }
}

impl DocumentSource for BytesSource {
    fn format(&self) -> DocumentFormat {
        self.format
    }

    /// The bytes, borrowed; never fails.
    fn read(&self) -> Result<Cow<'_, [u8]>, PrintError> {
        Ok(Cow::Borrowed(&self.bytes))
    }
}

/// A [`DocumentSource`] over a closure, called for every attempt (read a file, render a page range).
pub struct FnSource<F> {
    format: DocumentFormat,
    read: F,
}

impl<F: Fn() -> Result<Vec<u8>, PrintError> + Send> FnSource<F> {
    /// A source of `format` whose bytes come from calling `read` once per attempt.
    pub fn new(format: DocumentFormat, read: F) -> Self {
        Self { format, read }
    }
}

impl<F: Fn() -> Result<Vec<u8>, PrintError> + Send> DocumentSource for FnSource<F> {
    fn format(&self) -> DocumentFormat {
        self.format
    }

    /// Call the closure; its error fails the attempt in the prepare phase.
    fn read(&self) -> Result<Cow<'_, [u8]>, PrintError> {
        (self.read)().map(Cow::Owned)
    }
}

/// How a print ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Outcome {
    Completed,
    /// Printed, with a printer warning (the job completed and its [`JobStatus::warnings`] say what);
    /// or the printer forgot the job, so it most likely printed (the job's warnings say which).
    CompletedWithWarnings,
    /// Refused, retries used up, a non-transient error, or the printer completed it with errors.
    Failed,
    /// Cancelled by us, or at the printer.
    Cancelled,
    /// The printer gave up on it.
    Aborted,
    /// Nothing conclusive was heard for too long ([`Staleness::ceiling`]) or a person dismissed
    /// the row ([`PrintJob::dismiss`]). **Not a failure**: the device may still print it. Shown
    /// as [`JobState::Abandoned`] with the reason `abandoned-no-outcome`.
    Abandoned,
}

impl Outcome {
    /// A stable snake_case name for logs and serialised snapshots.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithWarnings => "completed_with_warnings",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Aborted => "aborted",
            Self::Abandoned => "abandoned",
        }
    }

    /// The print happened (or most likely did).
    pub const fn printed(self) -> bool {
        matches!(self, Self::Completed | Self::CompletedWithWarnings)
    }
}

/// Where the print is, as a whole. Separate from the printer's [`JobState`] of one remote job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LifecycleState {
    /// Created; the printer, the tag and the backend are not resolved yet.
    Preparing,
    /// The next [`PrintJob::advance`] sends the document.
    Submitting,
    /// The printer accepted the document and no status is being followed: the backend cannot
    /// report, the job has no handle, following stopped, or it has not been read yet.
    Accepted,
    /// The remote job is being followed; carries the printer's last reported state.
    Watching(JobState),
    /// A safe, transient failure; attempt number `attempt` is due at `next_at` (on the job's clock).
    RetryScheduled {
        attempt: u32,
        next_at: Duration,
    },
    /// The outcome of a send is unknown (the request may have arrived). Nothing is resent until a
    /// person or the app has reconciled and chosen.
    NeedsAttention,
    Terminal(Outcome),
}

impl LifecycleState {
    /// Ended (until a manual [`PrintJob::retry`]).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal(_))
    }
}

/// [`LifecycleState`] reduced to what a UI shows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SimpleState {
    Preparing,
    Sending,
    /// Accepted; waiting in the printer's queue.
    Queued,
    /// Held at the printer (a hold, a PIN, a schedule).
    Held,
    Printing,
    /// The printer stopped the job; the reasons say why (paper, jam, cover).
    PrinterNeedsAttention,
    Retrying,
    /// The outcome of a send is unknown; a person has to look.
    NeedsAttention,
    /// Sent; the backend cannot say more.
    Sent,
    Done,
    DoneWithWarnings,
    Failed,
    Cancelled,
    /// Ended without an outcome: nothing was heard for too long, or a person dismissed it.
    Abandoned,
}

impl SimpleState {
    /// A stable snake_case name for logs and serialised snapshots.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Sending => "sending",
            Self::Queued => "queued",
            Self::Held => "held",
            Self::Printing => "printing",
            Self::PrinterNeedsAttention => "printer_needs_attention",
            Self::Retrying => "retrying",
            Self::NeedsAttention => "needs_attention",
            Self::Sent => "sent",
            Self::Done => "done",
            Self::DoneWithWarnings => "done_with_warnings",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::Abandoned => "abandoned",
        }
    }
}

/// The coarse state a person is shown for a lifecycle state (`Submitting` reads as "sending",
/// a printer-side `Processing` as "printing", and so on).
fn simplify(state: LifecycleState) -> SimpleState {
    match state {
        LifecycleState::Preparing => SimpleState::Preparing,
        LifecycleState::Submitting => SimpleState::Sending,
        LifecycleState::Accepted => SimpleState::Sent,
        LifecycleState::Watching(printer) => match printer {
            JobState::Pending => SimpleState::Queued,
            JobState::Held => SimpleState::Held,
            JobState::Processing => SimpleState::Printing,
            JobState::NeedsAttention => SimpleState::PrinterNeedsAttention,
            _ => SimpleState::Sent,
        },
        LifecycleState::RetryScheduled { .. } => SimpleState::Retrying,
        LifecycleState::NeedsAttention => SimpleState::NeedsAttention,
        LifecycleState::Terminal(Outcome::Completed) => SimpleState::Done,
        LifecycleState::Terminal(Outcome::CompletedWithWarnings) => SimpleState::DoneWithWarnings,
        LifecycleState::Terminal(Outcome::Failed | Outcome::Aborted) => SimpleState::Failed,
        LifecycleState::Terminal(Outcome::Cancelled) => SimpleState::Cancelled,
        LifecycleState::Terminal(Outcome::Abandoned) => SimpleState::Abandoned,
    }
}

/// An attempt's failure, typed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct AttemptError {
    pub kind: ErrorKind,
    pub safety: RetrySafety,
    pub retry_after: Option<Duration>,
    pub native: Option<NativeCode>,
    pub phase: Option<Phase>,
    pub message: String,
}

impl AttemptError {
    /// The condition is transient ([`ErrorKind::is_retryable`]).
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }
}

impl From<&PrintError> for AttemptError {
    /// Copy the error's kind, retry safety, `Retry-After`, native code, phase and message.
    fn from(error: &PrintError) -> Self {
        Self {
            kind: error.kind(),
            safety: error.retry_safety(),
            retry_after: error.retry_after(),
            native: error.native(),
            phase: error.phase(),
            message: error.message().to_owned(),
        }
    }
}

/// What an attempt came to.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum AttemptOutcome {
    /// The send is in progress.
    InFlight,
    /// The printer accepted the document; the remote job, when the backend has a handle for it.
    Accepted(Option<JobId>),
    Error(AttemptError),
}

/// One try at getting the document to the printer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Attempt {
    /// 1 for the first; counts across retries, automatic and manual.
    pub number: u32,
    /// On the job's clock.
    pub started: Duration,
    pub ended: Option<Duration>,
    /// How far the attempt got ([`Phase`]).
    pub phase: Option<Phase>,
    pub outcome: AttemptOutcome,
    /// The remote job's last state when it ended (`Aborted`, `Cancelled`, `Completed`, ...).
    pub remote_end: Option<JobState>,
}

/// The document as it was sent: never the bytes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DocumentInfo {
    pub format: DocumentFormat,
    pub len: usize,
    /// A 64-bit FNV-1a hash of the bytes: it detects that the file changed between attempts, it is
    /// not a cryptographic digest.
    pub fingerprint: u64,
}

/// The request as it was sent.
#[derive(Clone, Debug, PartialEq)]
pub struct RequestRecord {
    /// The job settings; `printer` is the resolved namespaced id once known. The title is the one
    /// the app gave, without the tag suffix.
    pub job: Job,
    pub options: JobOptions,
    pub document: Option<DocumentInfo>,
    /// The tag the requests carried (`None` when the policy or the backend left it out).
    pub tag: Option<Tag>,
    /// How the backend sent it, in its own words ([`Submitted::effective`]).
    pub effective: Option<String>,
    /// What the registry's converter did to the document for the accepted attempt
    /// ([`Submitted::conversion`]); `None` when it was sent as it was (or nothing was accepted yet).
    pub conversion: Option<Conversion>,
}

/// What happened, when.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum TrailEvent {
    State(LifecycleState),
    /// A printer reading that differs from the previous one.
    Printer {
        state: JobState,
        reasons: Vec<String>,
        sheets_done: Option<u32>,
    },
    Note(String),
}

/// One line of the status trail.
#[derive(Clone, Debug, PartialEq)]
pub struct TrailEntry {
    /// On the job's clock.
    pub at: Duration,
    pub event: TrailEvent,
}

/// Progress of the current remote job. Totals are optional.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub struct JobProgress {
    /// The submission's own phases (a network backend's protocol steps), not sheets.
    pub submit: Option<Progress>,
    pub sheets_done: Option<u32>,
    pub sheets_total: Option<u32>,
    pub pages_done: Option<u32>,
    pub pages_total: Option<u32>,
}

impl JobProgress {
    /// Sheets done over sheets total, when both are known and the total is not zero.
    pub fn fraction(&self) -> Option<f32> {
        match (self.sheets_done, self.sheets_total) {
            (Some(done), Some(total)) if total > 0 => Some((done.min(total) as f32) / (total as f32)),
            _ => None,
        }
    }
}

/// What the backend adds for people debugging: never the document, never raw protocol.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub struct NativeInfo {
    /// How the backend sent the request ([`Submitted::effective`]).
    pub effective: Option<String>,
    /// The backend's own handle for the current remote job.
    pub remote_native: Option<String>,
    /// The native error code of the last failure.
    pub error_native: Option<NativeCode>,
}

/// Why a print that was accepted needs a look although the device never said anything bad.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum StaleReason {
    /// No status read succeeded for [`Staleness::silence_after`]. Probing goes on; a successful
    /// read ends this on its own.
    NoNews,
    /// The device keeps reporting the same thing ([`Staleness::stall_after_processing`] and
    /// [`Staleness::stall_after_pending`]). Probing goes on; any change ends this.
    Stalled,
    /// The print ended as [`Outcome::Abandoned`].
    Abandoned,
}

impl StaleReason {
    /// The reason token in a [`JobView`]: `no-news-since:<unix seconds of the last good read>`
    /// (`no-news-since:unknown` when none succeeded), `stalled` and `abandoned-no-outcome`.
    pub const fn token(self) -> &'static str {
        match self {
            Self::NoNews => "no-news-since",
            Self::Stalled => "stalled",
            Self::Abandoned => "abandoned-no-outcome",
        }
    }
}

/// When a print that was accepted but is not heard from needs a person, and when a row gives up.
/// Time is the job's [`Clock`]. A timeout is **never** a failure and never causes a resend: the
/// device may still print. See `docs/job-tracking.md`, section 9, for the sources of the numbers;
/// those beyond the protocol figures are judgement. [`Self::disabled`] gives the behaviour
/// before this existed: following stops at [`WaitPolicy::max_wait`] and the print stays
/// `Accepted`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Staleness {
    /// `false`: nothing here applies (see [`Self::disabled`]).
    pub enabled: bool,
    /// The first gap between status reads when neither the config's watch nor the backend names
    /// one. Default 2 s.
    pub poll_initial: Duration,
    /// The longest gap between reads while the status does not change. Default 15 s (well under
    /// the 60 s a printer must remember finished jobs, so a short job is not missed).
    pub poll_cap: Duration,
    /// This long without one successful read: `NeedsAttention` ([`StaleReason::NoNews`]).
    /// Default 3 min.
    pub silence_after: Option<Duration>,
    /// The gap between probes once the print needs attention for lack of news or a stall, so a
    /// printer that comes back is noticed. Default 30 s.
    pub probe_interval: Duration,
    /// This long with a completely unchanged reading while the device says `Processing`:
    /// `NeedsAttention` ([`StaleReason::Stalled`]). Default 10 min.
    pub stall_after_processing: Option<Duration>,
    /// The same for `Pending` (queued behind other jobs). Default 30 min. A held job and a stopped
    /// one never time out: a person or the printer's own reasons speak for them.
    pub stall_after_pending: Option<Duration>,
    /// From acceptance, after this long without a terminal state the print ends as
    /// [`Outcome::Abandoned`]. Default 3 h (the CUPS `MaxJobTime` default).
    pub ceiling: Option<Duration>,
    /// After a send whose outcome is unknown, wait this long before advising a resend, unless the
    /// backend names its own ([`crate::backends::Backend::open_job_timeout`]). Default 65 s.
    pub reconcile_after: Duration,
}

impl Default for Staleness {
    /// On, polling from 2 s up to 15 s; silent after 3 minutes, probing every 30 s; stalled after 10
    /// minutes processing or 30 pending; a 3-hour ceiling; reconcile after 65 s.
    fn default() -> Self {
        Self {
            enabled: true,
            poll_initial: Duration::from_secs(2),
            poll_cap: Duration::from_secs(15),
            silence_after: Some(Duration::from_secs(3 * 60)),
            probe_interval: Duration::from_secs(30),
            stall_after_processing: Some(Duration::from_secs(10 * 60)),
            stall_after_pending: Some(Duration::from_secs(30 * 60)),
            ceiling: Some(Duration::from_secs(3 * 60 * 60)),
            reconcile_after: Duration::from_secs(65),
        }
    }
}

impl Staleness {
    /// No timeouts and no waiting before a resend is advised: following stops at the watch's
    /// `max_wait` and the print stays `Accepted` (refresh resumes it).
    pub fn disabled() -> Self {
        Self {
            enabled: false,
            silence_after: None,
            stall_after_processing: None,
            stall_after_pending: None,
            ceiling: None,
            reconcile_after: Duration::ZERO,
            ..Self::default()
        }
    }
}

/// How a [`PrintJob`] behaves; the default is what an app wants unless it has a reason.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PrintJobConfig {
    /// Automatic retry of safe, transient failures. `None` disables it. The default allows 3
    /// attempts in a row.
    pub retry: Option<RetryPolicy>,
    pub tag: TagPolicy,
    /// How the remote job is followed.
    pub watch: WaitPolicy,
    /// The status trail keeps this many lines (the oldest are dropped).
    pub trail_cap: usize,
    /// The attempt history keeps this many attempts (the oldest are dropped).
    pub attempt_cap: usize,
    /// Allowance for the printer's clock differing from ours when a reconcile narrows by time.
    pub reconcile_slack: Duration,
    /// When an accepted print that is not heard from needs attention or is abandoned.
    pub staleness: Staleness,
}

impl Default for PrintJobConfig {
    /// Three automatic attempts, tags on network backends only, a 10-minute watch, 64 trail entries,
    /// 16 attempts kept, 2 minutes of reconcile slack and the default staleness rules.
    fn default() -> Self {
        Self {
            retry: Some(RetryPolicy { max_attempts: 3, ..RetryPolicy::default() }),
            tag: TagPolicy::Auto,
            watch: WaitPolicy { max_wait: Duration::from_secs(600), ..WaitPolicy::default() },
            trail_cap: 64,
            attempt_cap: 16,
            reconcile_slack: Duration::from_secs(120),
            staleness: Staleness::default(),
        }
    }
}

/// What one [`PrintJob::advance`] did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Advance {
    /// The lifecycle state or the printer's reading changed.
    pub changed: bool,
    /// Call again after this long (zero: now). `None`: nothing happens on its own: the print ended, a
    /// person has to decide, or the backend cannot report.
    pub next_in: Option<Duration>,
}

/// An immutable view of a [`PrintJob`] for a UI. Cheap to make (no I/O, no blocking): the attempt
/// timeline and the trail are shared with the job, not copied.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Snapshot {
    pub id: LocalJobId,
    pub title: String,
    /// The namespaced printer id, once resolved.
    pub printer: Option<String>,
    pub state: LifecycleState,
    pub simple: SimpleState,
    /// Nothing more happens without a person: ended, `NeedsAttention`, or accepted with nothing to
    /// follow.
    pub settled: bool,
    pub progress: JobProgress,
    /// The printer's reasons for the current job, as reported.
    pub reasons: Vec<String>,
    pub warnings: Vec<String>,
    pub last_error: Option<AttemptError>,
    /// The number of the latest attempt (0 before the first).
    pub attempt: u32,
    /// The current remote job, when there is one.
    pub remote: Option<JobId>,
    pub tag: Tag,
    /// Whether the requests carried the tag.
    pub tag_applied: bool,
    /// The document as the source gave it (before any conversion).
    pub document: Option<DocumentInfo>,
    /// What the registry's converter made of it for the accepted attempt, when it ran.
    pub conversion: Option<Conversion>,
    pub advice: RetryAdvice,
    /// Why the print needs a look or was abandoned, when the cause is silence, a stall or a
    /// dismissal ([`Staleness`]).
    pub stale: Option<StaleReason>,
    /// The wall-clock time of the last successful status read.
    pub last_news: Option<Timestamp>,
    /// What the last reconcile found, until the next attempt.
    pub reconciled: Option<Reconciled>,
    pub native: Option<NativeInfo>,
    /// Time since the job was created, on its clock.
    pub elapsed: Duration,
    /// When the job was created, first sent and ended, as wall-clock times (see
    /// [`PrintJob::with_wall_clock`]). `started` is the first attempt's send; `finished` is set
    /// once the print is terminal (and cleared again by a manual [`PrintJob::retry`]).
    pub times: JobTimes,
    expected_pages: Option<u32>,
    attempts: Arc<Vec<Attempt>>,
    trail: Arc<Vec<TrailEntry>>,
    /// Lines dropped from the front of the trail.
    pub trail_dropped: u32,
}

impl Snapshot {
    /// Every attempt, oldest first (bounded by [`PrintJobConfig::attempt_cap`]).
    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    /// The status trail, oldest first (bounded by [`PrintJobConfig::trail_cap`]).
    pub fn trail(&self) -> &[TrailEntry] {
        &self.trail
    }

    /// The print as a [`craft_job::JobView`]. No I/O; the snapshot already holds everything.
    ///
    /// | `JobView` | Filled from |
    /// |---|---|
    /// | `id` | the current remote job's [`JobId`] once the printer accepted a document; before that (and after a manual retry cleared it) a local id, `local:print-<n>` ([`LocalJobId`]) |
    /// | `kind` | [`JobKind::Print`] |
    /// | `title` | the title the app gave, without the tag suffix |
    /// | `state` | see below |
    /// | `reasons` | the printer's reasons, then the print's warnings (a print that completed with warnings has them here) |
    /// | `progress` | sheets done and the sheet total; pages when no sheets are reported; `None` before anything is reported |
    /// | `times` | [`Snapshot::times`] |
    /// | `documents` | the document that was sent: the title as its name, its format, its size, and the expected page count when the options gave one |
    /// | `attempts` | the number of the latest attempt |
    /// | `last_error` | the last failure: message, [`ErrorKind::name`], this snapshot's [`Snapshot::advice`] and the native code |
    /// | `tag` | the tag, when the requests carried it |
    /// | `device_name` | the namespaced printer id once resolved |
    ///
    /// State: `Preparing`, `Submitting` and `RetryScheduled` are [`JobState::Pending`];
    /// `Accepted` (nothing followed) is [`JobState::Unknown`]; `Watching` is the printer's own state
    /// (a stopped printer is [`JobState::NeedsAttention`]); [`LifecycleState::NeedsAttention`] (the
    /// outcome of a send is unknown) is [`JobState::NeedsAttention`]; the endings are `Completed`
    /// (also [`Outcome::CompletedWithWarnings`]), `Cancelled`, `Abandoned` ([`Outcome::Abandoned`],
    /// not a failure, with the reason `abandoned-no-outcome`), `Aborted` ([`Outcome::Aborted`] and
    /// a [`Outcome::Failed`] that is not the next case) and `CompletedWithErrors` (a `Failed`
    /// whose job the printer completed with errors).
    pub fn view(&self) -> JobView {
        let id = self.remote.clone().unwrap_or_else(|| JobId::new("local", self.id.to_string()));
        let mut view = JobView::new(id, JobKind::Print, self.title.clone(), self.job_state())
            .with_times(self.times)
            .with_attempts(self.attempt)
            .with_documents(self.documents());
        for text in self.reasons.iter().chain(&self.warnings) {
            view = view.with_reason(text.clone());
        }
        if let Some(reason) = self.stale {
            let token = match (reason, self.last_news) {
                (StaleReason::NoNews, Some(at)) => format!("{}:{}", reason.token(), at.secs()),
                (StaleReason::NoNews, None) => format!("{}:unknown", reason.token()),
                (other, _) => other.token().to_owned(),
            };
            view = view.with_reason(token);
        }
        let p = &self.progress;
        let progress = match (p.sheets_done, p.pages_done) {
            (Some(done), _) => Some(Progress::new(done, p.sheets_total)),
            (None, Some(done)) => Some(Progress::new(done, p.pages_total)),
            (None, None) => None,
        };
        if let Some(progress) = progress {
            view = view.with_progress(progress);
        }
        if let Some(error) = &self.last_error {
            let mut summary = ErrorSummary::new(error.message.clone(), error.kind.name()).with_retry_advice(self.advice);
            if let Some(native) = error.native {
                summary = summary.with_native(native);
            }
            view = view.with_error(summary);
        }
        if self.tag_applied {
            view = view.with_tag(self.tag);
        }
        if let Some(printer) = &self.printer {
            view = view.with_device_name(printer.clone());
        }
        view
    }

    /// This print as a [`JobState`] for the shared job view; a failure whose last remote job ended
    /// `CompletedWithErrors` keeps that state, other failures read as aborted.
    fn job_state(&self) -> JobState {
        match self.state {
            LifecycleState::Preparing | LifecycleState::Submitting | LifecycleState::RetryScheduled { .. } => JobState::Pending,
            LifecycleState::Accepted => JobState::Unknown,
            LifecycleState::Watching(printer) => printer,
            LifecycleState::NeedsAttention => JobState::NeedsAttention,
            LifecycleState::Terminal(Outcome::Completed | Outcome::CompletedWithWarnings) => JobState::Completed,
            LifecycleState::Terminal(Outcome::Cancelled) => JobState::Cancelled,
            LifecycleState::Terminal(Outcome::Abandoned) => JobState::Abandoned,
            LifecycleState::Terminal(Outcome::Aborted) => JobState::Aborted,
            LifecycleState::Terminal(Outcome::Failed) => {
                if self.attempts.last().and_then(|attempt| attempt.remote_end) == Some(JobState::CompletedWithErrors) {
                    JobState::CompletedWithErrors
                } else {
                    JobState::Aborted
                }
            }
        }
    }

    /// The document as it was sent (the converted one when a converter ran), with its size and the
    /// expected page count when known; empty before the source was first read.
    fn documents(&self) -> JobDocuments {
        let mut documents = JobDocuments::new();
        if let Some(info) = self.document {
            // The document is shown as it was sent: the converted one when the converter ran.
            let (format, len) = match &self.conversion {
                Some(conversion) => (conversion.to, conversion.bytes),
                None => (info.format, info.len),
            };
            let mut document = DocumentRef::new(self.title.clone()).with_format(format.name()).with_bytes(len as u64);
            if let Some(pages) = self.expected_pages.filter(|pages| *pages > 0) {
                document = document.with_pages(pages);
            }
            documents.push(document);
        }
        documents
    }
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

/// Hash of the document for "did it change between attempts" (FNV-1a, 64 bit).
fn fnv1a(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3))
}

/// What one pass over the document source and the backend produced.
struct Sent {
    result: Result<Submitted, PrintError>,
    progress: Option<Progress>,
    document: Option<DocumentInfo>,
}

type WallClock = Arc<dyn Fn() -> SystemTime + Send + Sync>;

/// The lifecycle record of one user-visible print. See the module documentation and
/// `docs/job-tracking.md`.
pub struct PrintJob {
    id: LocalJobId,
    config: PrintJobConfig,
    clock: Arc<dyn Clock>,
    wall: WallClock,
    jitter: Jitter,
    source: Box<dyn DocumentSource>,
    consent: Option<UserConfirmed>,
    cancel: CancelToken,
    tag: Tag,
    tag_applied: bool,
    request: RequestRecord,
    created: Duration,
    created_wall: SystemTime,
    finished_at: Option<Duration>,
    first_wall: Option<SystemTime>,
    state: LifecycleState,
    attempts: Arc<Vec<Attempt>>,
    attempt_counter: u32,
    trail: Arc<Vec<TrailEntry>>,
    trail_dropped: u32,
    last_printer: Option<JobStatus>,
    progress: JobProgress,
    reasons: Vec<String>,
    warnings: Vec<String>,
    last_error: Option<AttemptError>,
    remote: Option<JobId>,
    watch: Option<JobWatch>,
    expected: ExpectedTotals,
    cancel_requested: bool,
    ended_with_errors: bool,
    auto_failures: u32,
    reconciled: Option<Reconciled>,
    info: Option<BackendInfo>,
    watchable: bool,
    accepted_at: Option<Duration>,
    last_news: Option<Duration>,
    last_news_wall: Option<SystemTime>,
    /// The latest moment a push event woke the watch (job clock). News from the printer, but not a
    /// good read: see `news_anchor`.
    last_event: Option<Duration>,
    changed_at: Option<Duration>,
    stale: Option<StaleReason>,
    probe_at: Option<Duration>,
    unknown_since: Option<Duration>,
}

impl std::fmt::Debug for PrintJob {
    /// The id, state, attempt count and remote job id; the document and history are left out.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PrintJob")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("attempts", &self.attempt_counter)
            .field("remote", &self.remote.as_ref().map(ToString::to_string))
            .finish_non_exhaustive()
    }
}

impl PrintJob {
    /// A print of the document `source` provides, with the settings of `job`. `job.printer` is a
    /// namespaced printer id, or `None` for the default printer (resolved at the first
    /// [`Self::advance`] and then fixed, so every retry goes to the same printer). Nothing is sent
    /// until the first advance. A job for a [`BackendClass::Network`](crate::BackendClass::Network)
    /// backend needs [`Self::with_consent`].
    pub fn new(job: Job, source: impl DocumentSource + 'static) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock);
        let options = JobOptions::default();
        let expected = ExpectedTotals::for_job(&job, &options);
        Self {
            id: LocalJobId(NEXT_ID.fetch_add(1, Ordering::Relaxed)),
            config: PrintJobConfig::default(),
            jitter: Jitter::from_clock(clock.as_ref()),
            created: clock.now(),
            created_wall: crate::clock::wall_now(),
            finished_at: None,
            clock,
            wall: Arc::new(crate::clock::wall_now),
            source: Box::new(source),
            consent: None,
            cancel: CancelToken::new(),
            tag: Tag::random(),
            tag_applied: false,
            request: RequestRecord { job, options, document: None, tag: None, effective: None, conversion: None },
            first_wall: None,
            state: LifecycleState::Preparing,
            attempts: Arc::default(),
            attempt_counter: 0,
            trail: Arc::default(),
            trail_dropped: 0,
            last_printer: None,
            progress: JobProgress::default(),
            reasons: Vec::new(),
            warnings: Vec::new(),
            last_error: None,
            remote: None,
            watch: None,
            expected,
            cancel_requested: false,
            ended_with_errors: false,
            auto_failures: 0,
            reconciled: None,
            info: None,
            watchable: false,
            accepted_at: None,
            last_news: None,
            last_news_wall: None,
            last_event: None,
            changed_at: None,
            stale: None,
            probe_at: None,
            unknown_since: None,
        }
    }

    /// Per-job hints (media, orientation, expected page count); see [`JobOptions`].
    #[must_use]
    pub fn with_options(mut self, options: JobOptions) -> Self {
        self.expected = ExpectedTotals::for_job(&self.request.job, &options);
        self.request.options = options;
        self
    }

    /// The person's consent, required by network backends.
    #[must_use]
    pub fn with_consent(mut self, consent: UserConfirmed) -> Self {
        self.consent = Some(consent);
        self
    }

    /// Measure all waiting on this clock (a [`crate::ManualClock`] in tests). Set it before the first
    /// advance.
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.created = clock.now();
        self.jitter = Jitter::from_clock(clock.as_ref());
        self.clock = clock;
        self
    }

    /// The wall clock used to stamp the time window of a reconcile (default [`SystemTime::now`]).
    #[must_use]
    pub fn with_wall_clock(mut self, wall: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        self.wall = Arc::new(wall);
        self.created_wall = (self.wall)();
        self
    }

    /// Behaviour: retry, tag policy, how the job is followed, bounds.
    #[must_use]
    pub fn with_config(mut self, config: PrintJobConfig) -> Self {
        self.config = config;
        self
    }

    /// Automatic retry of safe, transient failures under this policy; `None` turns it off.
    #[must_use]
    pub fn with_retry(mut self, retry: Option<RetryPolicy>) -> Self {
        self.config.retry = retry;
        self
    }

    /// Choose when the idempotency tag is sent; the default sends it to network backends only.
    #[must_use]
    pub fn with_tag_policy(mut self, tag: TagPolicy) -> Self {
        self.config.tag = tag;
        self
    }

    /// Use this tag instead of a random one (tests, or an app that derives it from its own id).
    #[must_use]
    pub fn with_tag(mut self, tag: Tag) -> Self {
        self.tag = tag;
        self
    }

    /// Use another jitter source for the retry delays (a fixed seed in tests).
    #[must_use]
    pub fn with_jitter(mut self, jitter: Jitter) -> Self {
        self.jitter = jitter;
        self
    }

    /// This print's process-local id, the same across every attempt.
    pub fn id(&self) -> LocalJobId {
        self.id
    }

    /// The idempotency tag of this print: generated at creation, the same for every attempt.
    pub fn tag(&self) -> &Tag {
        &self.tag
    }

    /// The current lifecycle state.
    pub fn state(&self) -> LifecycleState {
        self.state
    }

    /// The request as it was (or will be) sent.
    pub fn request(&self) -> &RequestRecord {
        &self.request
    }

    /// The attempts so far, oldest first; only the most recent `attempt_cap` are kept.
    pub fn attempts(&self) -> &[Attempt] {
        &self.attempts
    }

    /// A token an app thread can set while [`Self::advance`] is blocked in a submit: the backend
    /// stops at its next checkpoint, and a pending retry is cancelled. After the document was
    /// accepted use [`Self::cancel`]. A manual [`Self::retry`] replaces the token.
    pub fn cancel_token(&self) -> CancelToken {
        self.cancel.clone()
    }

    /// Nothing more happens without a person (see [`Snapshot::settled`]).
    pub fn is_settled(&self) -> bool {
        match self.state {
            LifecycleState::Terminal(_) | LifecycleState::NeedsAttention => true,
            LifecycleState::Accepted => self.watch.is_none(),
            _ => false,
        }
    }

    /// The current time on the job's clock.
    fn now(&self) -> Duration {
        self.clock.now()
    }

    /// Append a trail entry at the current time, dropping (and counting) the oldest entries past `trail_cap`.
    fn record(&mut self, event: TrailEvent) {
        let at = self.now();
        let cap = self.config.trail_cap.max(1);
        let trail = Arc::make_mut(&mut self.trail);
        trail.push(TrailEntry { at, event });
        if trail.len() > cap {
            let extra = trail.len() - cap;
            trail.drain(..extra);
            self.trail_dropped = self.trail_dropped.saturating_add(u32::try_from(extra).unwrap_or(u32::MAX));
        }
    }

    /// Record a free-text note in the trail.
    fn note(&mut self, text: impl Into<String>) {
        self.record(TrailEvent::Note(text.into()));
    }

    /// Record a warning once: a repeat of the same text is neither noted nor listed again.
    fn warn(&mut self, text: impl Into<String>) {
        let text = text.into();
        if !self.warnings.contains(&text) {
            self.record(TrailEvent::Note(format!("warning: {text}")));
            self.warnings.push(text);
        }
    }

    /// Move to `state`, record it, and stamp or clear the finish time. Returns whether the state changed.
    fn set_state(&mut self, state: LifecycleState) -> bool {
        if self.state == state {
            return false;
        }
        self.state = state;
        self.finished_at = if state.is_terminal() { Some(self.finished_at.unwrap_or_else(|| self.now())) } else { None };
        self.record(TrailEvent::State(state));
        true
    }

    /// When [`PrintJob::advance`] should next be called: now while preparing or sending, at the retry
    /// time, at the next poll or staleness deadline while following, and never once settled.
    fn next_in(&self) -> Option<Duration> {
        match self.state {
            LifecycleState::Terminal(_) => None,
            LifecycleState::NeedsAttention if self.stale.is_none() => None,
            LifecycleState::Preparing | LifecycleState::Submitting => Some(Duration::ZERO),
            LifecycleState::RetryScheduled { next_at, .. } => Some(next_at.saturating_sub(self.now())),
            LifecycleState::Accepted | LifecycleState::Watching(_) | LifecycleState::NeedsAttention => {
                let poll = if self.probing() {
                    Some(self.probe_at.map_or(Duration::ZERO, |at| at.saturating_sub(self.now())))
                } else {
                    self.watch.as_ref().and_then(JobWatch::next_poll_in)
                };
                match (poll, self.stale_deadline()) {
                    (Some(poll), Some(deadline)) => Some(poll.min(deadline)),
                    (poll, deadline) => poll.or(deadline.filter(|_| self.watchable)),
                }
            }
        }
    }

    /// Whether this print is followed by single probes the job schedules itself (the watch ended
    /// or the print needs attention for lack of news) instead of by a [`JobWatch`].
    fn probing(&self) -> bool {
        self.stale.is_some() || (self.config.staleness.enabled && self.watchable && self.remote.is_some() && self.watch.is_none())
    }

    /// The time until the next staleness rule would fire, for [`Advance::next_in`].
    fn stale_deadline(&self) -> Option<Duration> {
        let st = self.config.staleness;
        if !st.enabled || !self.watchable || self.remote.is_none() {
            return None;
        }
        let accepted = self.accepted_at?;
        let now = self.now();
        let mut due: Vec<Duration> = Vec::new();
        if let Some(ceiling) = st.ceiling {
            due.push(accepted.saturating_add(ceiling));
        }
        if self.stale.is_none() {
            if let Some(silence) = st.silence_after {
                due.push(self.news_anchor(accepted, silence).saturating_add(silence));
            }
            if let LifecycleState::Watching(printer) = self.state
                && let Some(stall) = self.stall_threshold(printer)
            {
                due.push(self.changed_at.unwrap_or(accepted).saturating_add(stall));
            }
        }
        due.into_iter().min().map(|at| at.saturating_sub(now))
    }

    /// Remember when a push event last woke the watch.
    fn sync_wake(&mut self, watch: &JobWatch) {
        if let Some(at) = watch.last_wake() {
            self.last_event = Some(self.last_event.map_or(at, |old| old.max(at)));
        }
    }

    /// Where the silence clock ([`Staleness::silence_after`]) starts: the last good read, or the
    /// last push event if later, but an event postpones the silence by at most twice `silence`
    /// after the last good read, so a printer that sends events while every status read fails is
    /// still flagged. Only a good read resets the stall clock; this is the silence rule alone.
    fn news_anchor(&self, accepted: Duration, silence: Duration) -> Duration {
        let read = self.last_news.unwrap_or(accepted).max(accepted);
        match self.last_event {
            Some(event) => read.max(event.min(read.saturating_add(silence.saturating_mul(2)))),
            None => read,
        }
    }

    /// How long the printer may report `state` without change before the print counts as stalled;
    /// only `Processing` and `Pending` have a threshold.
    fn stall_threshold(&self, printer: JobState) -> Option<Duration> {
        match printer {
            JobState::Processing => self.config.staleness.stall_after_processing,
            JobState::Pending => self.config.staleness.stall_after_pending,
            _ => None,
        }
    }

    /// The watch policy: the config's, with the staleness polling cadence and ceiling when enabled.
    fn watch_policy(&self, recommended: Option<Duration>) -> WaitPolicy {
        let st = self.config.staleness;
        if !st.enabled {
            return self.config.watch;
        }
        let mut policy = self.config.watch;
        policy.interval = policy.interval.or(recommended).or(Some(st.poll_initial));
        policy.max_interval = st.poll_cap;
        policy.max_wait = st.ceiling.unwrap_or(policy.max_wait).max(policy.max_wait);
        policy
    }

    /// Apply the staleness rules at `now`. Returns whether the lifecycle state changed.
    fn check_staleness(&mut self) -> bool {
        let st = self.config.staleness;
        if !st.enabled || !self.watchable || self.remote.is_none() {
            return false;
        }
        if !matches!(self.state, LifecycleState::Accepted | LifecycleState::Watching(_))
            && !(self.state == LifecycleState::NeedsAttention && self.stale.is_some())
        {
            return false;
        }
        let Some(accepted) = self.accepted_at else {
            return false;
        };
        let now = self.now();
        if st.ceiling.is_some_and(|ceiling| now.saturating_sub(accepted) >= ceiling) {
            self.note("no outcome was heard before the ceiling; the row is abandoned (the printer may still print it)");
            return self.abandon();
        }
        if self.stale.is_some() {
            return false;
        }
        let quiet = now.saturating_sub(self.news_anchor(accepted, st.silence_after.unwrap_or_default()));
        if st.silence_after.is_some_and(|silence| quiet >= silence) {
            self.note("no status read succeeded for a while; probing on");
            return self.go_stale(StaleReason::NoNews, now);
        }
        if let LifecycleState::Watching(printer) = self.state
            && let Some(stall) = self.stall_threshold(printer)
            && now.saturating_sub(self.changed_at.unwrap_or(accepted)) >= stall
        {
            self.note("the printer keeps reporting the same thing; probing on");
            return self.go_stale(StaleReason::Stalled, now);
        }
        false
    }

    /// Stop the watch and put the print in `NeedsAttention` for `reason`, with probes every
    /// `probe_interval` from `now`. Always reports a change.
    fn go_stale(&mut self, reason: StaleReason, now: Duration) -> bool {
        self.stale = Some(reason);
        self.watch = None;
        self.probe_at = Some(now.saturating_add(self.config.staleness.probe_interval));
        self.set_state(LifecycleState::NeedsAttention);
        true
    }

    /// End the row without an outcome: the device may still print it.
    fn abandon(&mut self) -> bool {
        self.watch = None;
        self.stale = Some(StaleReason::Abandoned);
        self.set_state(LifecycleState::Terminal(Outcome::Abandoned))
    }

    /// End the row now without an outcome (a person dismissed it, or lost interest). Nothing is
    /// sent to the device and nothing is cancelled: it may still print. The state becomes
    /// `Terminal(Outcome::Abandoned)`; it can still be [`Self::retry`]ed on purpose. Does nothing
    /// to a print that already ended.
    pub fn dismiss(&mut self) {
        if self.state.is_terminal() {
            return;
        }
        self.cancel.cancel();
        self.note("dismissed by the user");
        self.abandon();
    }

    /// [`Self::dismiss`], and first ask the device to cancel the job when it has one that is ours
    /// (the backend must support cancel). `Err` is the cancel that did not work; the row is
    /// dismissed either way, because the person asked to be rid of it.
    pub fn dismiss_and_cancel(&mut self, registry: &Registry) -> Result<(), PrintError> {
        let result = match (self.state, self.remote.clone()) {
            (LifecycleState::Terminal(_), _) => return Ok(()),
            (LifecycleState::Accepted | LifecycleState::Watching(_) | LifecycleState::NeedsAttention, Some(remote)) => {
                let result = registry.cancel_job(&remote);
                match &result {
                    Ok(()) => self.note("cancel requested before dismissing"),
                    Err(error) => self.note(format!("cancel failed: {error}; dismissing anyway")),
                }
                result
            }
            _ => Ok(()),
        };
        self.dismiss();
        result
    }

    /// The wait after a send whose outcome is unknown before a resend is advised.
    fn reconcile_window(&self) -> Duration {
        let st = self.config.staleness;
        if !st.enabled {
            return Duration::ZERO;
        }
        self.info.and_then(|info| info.open_job_timeout).unwrap_or(st.reconcile_after)
    }

    /// How much longer to wait before a resend is advised after a send whose outcome is unknown
    /// (the device's own window to give up on the half-made job, see
    /// [`crate::backends::Backend::open_job_timeout`]). `None` when not waiting.
    pub fn reconcile_wait(&self) -> Option<Duration> {
        if self.state != LifecycleState::NeedsAttention || self.stale.is_some() {
            return None;
        }
        let since = self.unknown_since?;
        let left = since.saturating_add(self.reconcile_window()).saturating_sub(self.now());
        (!left.is_zero()).then_some(left)
    }

    /// An [`Advance`] reporting `changed` and the next wake-up.
    fn step(&self, changed: bool) -> Advance {
        Advance { changed, next_in: self.next_in() }
    }

    /// Open a new in-flight attempt and return its number; the oldest attempts past `attempt_cap` are dropped.
    fn begin_attempt(&mut self) -> u32 {
        let started = self.now();
        self.attempt_counter = self.attempt_counter.saturating_add(1);
        let number = self.attempt_counter;
        let cap = self.config.attempt_cap.max(1);
        let attempts = Arc::make_mut(&mut self.attempts);
        attempts.push(Attempt { number, started, ended: None, phase: None, outcome: AttemptOutcome::InFlight, remote_end: None });
        if attempts.len() > cap {
            let extra = attempts.len() - cap;
            attempts.drain(..extra);
        }
        number
    }

    /// The most recent attempt, for updating in place.
    fn latest_attempt(&mut self) -> Option<&mut Attempt> {
        Arc::make_mut(&mut self.attempts).last_mut()
    }

    /// Advance by at most **one** bounded request: send the document, read the job's status, or
    /// wait. Returns what changed and when to come back; see [`Advance`]. Call it from a worker or a
    /// frame loop; it never sleeps. A `Preparing` job takes one cheap step (no I/O) to become
    /// `Submitting`, so a UI can show "sending" before the blocking send.
    pub fn advance(&mut self, registry: &Registry) -> Advance {
        match self.state {
            LifecycleState::Preparing => self.prepare(registry),
            LifecycleState::Submitting => self.submit(registry),
            LifecycleState::RetryScheduled { next_at, .. } => {
                if self.cancel.is_cancelled() {
                    self.cancel_requested = true;
                    let changed = self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
                    return self.step(changed);
                }
                let now = self.now();
                if now >= next_at {
                    let changed = self.set_state(LifecycleState::Submitting);
                    self.step(changed)
                } else {
                    self.step(false)
                }
            }
            LifecycleState::Accepted | LifecycleState::Watching(_) => self.follow(registry),
            LifecycleState::NeedsAttention if self.stale.is_some() => self.follow(registry),
            LifecycleState::NeedsAttention | LifecycleState::Terminal(_) => self.step(false),
        }
    }

    /// Before the first send: honour an early cancel, pin the registry's default printer when none was
    /// given, and decide whether the tag is sent. A lookup failure fails a first, retry-safe attempt.
    fn prepare(&mut self, registry: &Registry) -> Advance {
        if self.cancel.is_cancelled() {
            self.cancel_requested = true;
            let changed = self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
            return self.step(changed);
        }
        if self.request.job.printer.is_none()
            && let Ok(printer) = registry.default_printer()
        {
            // Pinned, so that every retry goes to the same printer.
            self.request.job.printer = Some(printer);
        }
        match registry.backend_info(self.request.job.printer.as_deref()) {
            Ok(info) => {
                let wants = match self.config.tag {
                    TagPolicy::Auto => info.class == crate::BackendClass::Network,
                    TagPolicy::Always => true,
                    TagPolicy::Never => false,
                };
                self.tag_applied = wants && info.tag_support.carries();
                if wants && !self.tag_applied {
                    self.note(format!("the {} backend cannot carry a tag; matching will use the title", info.name));
                }
                self.request.tag = self.tag_applied.then_some(self.tag);
                self.info = Some(info);
                let changed = self.set_state(LifecycleState::Submitting);
                self.step(changed)
            }
            Err(error) => {
                self.begin_attempt();
                self.fail_attempt(error.with_phase(Phase::Prepare).with_safety(RetrySafety::Safe))
            }
        }
    }

    /// Ask the document source, build the request and hand it to the registry. Takes `&self`: the
    /// bytes borrow the source, so nothing may change while they are alive.
    fn send(&self, registry: &Registry) -> Sent {
        let bytes = match self.source.read() {
            Ok(bytes) => bytes,
            Err(error) => {
                return Sent { result: Err(error.with_phase(Phase::Prepare).with_safety(RetrySafety::Safe)), progress: None, document: None };
            }
        };
        let document = DocumentInfo { format: self.source.format(), len: bytes.len(), fingerprint: fnv1a(&bytes) };
        let mut latest = None;
        let mut on_progress = |event: Progress| latest = Some(event);
        let mut request = SubmitRequest::new(Document::new(document.format, &bytes), self.request.job.clone())
            .with_options(self.request.options.clone())
            .with_cancel(self.cancel.clone())
            .with_progress(&mut on_progress);
        if let Some(consent) = self.consent {
            request = request.with_consent(consent);
        }
        if self.tag_applied {
            request = request.with_tag(self.tag);
        }
        let result = registry.submit_job(request);
        Sent { result, progress: latest, document: Some(document) }
    }

    /// Run one attempt: read the document (warning if it changed since the last attempt), submit it,
    /// and continue with [`Self::accepted`] or [`Self::fail_attempt`]. A cancel ends the print first.
    fn submit(&mut self, registry: &Registry) -> Advance {
        if self.cancel.is_cancelled() || self.cancel_requested {
            self.cancel_requested = true;
            let changed = self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
            return self.step(changed);
        }
        self.begin_attempt();
        self.request.conversion = None;
        if self.first_wall.is_none() {
            self.first_wall = Some((self.wall)());
        }
        let sent = self.send(registry);
        if let Some(document) = sent.document {
            if let Some(previous) = self.request.document
                && (previous.fingerprint != document.fingerprint || previous.len != document.len)
            {
                self.warn("the document changed between attempts");
            }
            self.request.document = Some(document);
        }
        if let Some(progress) = sent.progress {
            self.progress.submit = Some(progress);
        }
        match sent.result {
            Ok(submitted) => self.accepted(registry, submitted),
            Err(error) => self.fail_attempt(error),
        }
    }

    /// The backend accepted the document: close the attempt, keep the remote job and what was sent,
    /// start a watch when the job can be followed, and take any status that came with the reply.
    fn accepted(&mut self, registry: &Registry, submitted: Submitted) -> Advance {
        let now = self.now();
        let job = submitted.job.clone();
        if let Some(attempt) = self.latest_attempt() {
            attempt.ended = Some(now);
            attempt.phase = Some(Phase::Reply);
            attempt.outcome = AttemptOutcome::Accepted(job.clone());
        }
        self.auto_failures = 0;
        self.last_error = None;
        self.reconciled = None;
        self.remote = job;
        self.expected = submitted.expected;
        self.request.effective = submitted.effective.clone();
        self.request.conversion = submitted.conversion.clone();
        for warning in &submitted.warnings {
            self.warn(warning.clone());
        }
        let policy = self.watch_policy(submitted.poll_interval);
        self.watch = match (&submitted.job, submitted.watch.is_watchable()) {
            (Some(job), true) => JobWatch::for_submitted(&submitted, policy).map(|watch| {
                let watch = watch.with_clock(Arc::clone(&self.clock));
                match registry.wake_source(job) {
                    Some(wake) => watch.with_wake(wake),
                    None => watch,
                }
            }),
            _ => None,
        };
        self.start_following(submitted.job.is_some() && submitted.watch.is_watchable(), now);
        let changed = self.set_state(LifecycleState::Accepted);
        match submitted.status {
            Some(status) => {
                let reading = self.absorb_status(status);
                self.step(changed || reading)
            }
            None => self.step(changed),
        }
    }

    /// A remote job was accepted (or adopted): the staleness clocks start now.
    fn start_following(&mut self, watchable: bool, now: Duration) {
        self.watchable = watchable;
        self.accepted_at = Some(now);
        self.last_news = None;
        self.last_news_wall = None;
        self.last_event = None;
        self.changed_at = Some(now);
        self.stale = None;
        self.probe_at = None;
        self.unknown_since = None;
    }

    /// Close the attempt with `error` and decide what follows: a cancel ends the print, an unknown
    /// outcome needs attention (never resent automatically), and anything else is retried under the
    /// retry policy or fails the print.
    fn fail_attempt(&mut self, error: PrintError) -> Advance {
        let now = self.now();
        let failure = AttemptError::from(&error);
        let number = self.attempt_counter;
        if let Some(attempt) = self.latest_attempt() {
            attempt.ended = Some(now);
            attempt.phase = failure.phase;
            attempt.outcome = AttemptOutcome::Error(failure.clone());
        }
        self.note(format!("attempt {number} failed: {} ({}, {})", failure.message, kind_name(failure.kind), failure.safety.name()));
        self.last_error = Some(failure);
        let state = if error.kind() == ErrorKind::Cancelled {
            self.cancel_requested = true;
            LifecycleState::Terminal(Outcome::Cancelled)
        } else if error.outcome_unknown() {
            // The request may have arrived: never resent on its own.
            self.unknown_since = Some(now);
            LifecycleState::NeedsAttention
        } else {
            self.auto_failures = self.auto_failures.saturating_add(1);
            let delay = self.config.retry.and_then(|policy| match advise_error(&error, self.auto_failures, &policy, &mut self.jitter) {
                RetryAdvice::RetryNow => Some(Duration::ZERO),
                RetryAdvice::RetryAfter(wait) => Some(wait),
                _ => None,
            });
            match delay {
                Some(wait) => LifecycleState::RetryScheduled { attempt: number.saturating_add(1), next_at: now.saturating_add(wait) },
                None => LifecycleState::Terminal(Outcome::Failed),
            }
        };
        let changed = self.set_state(state);
        self.step(changed)
    }

    /// One step of following an accepted job: check the staleness rules, then either probe or tick the
    /// watch and take what it reports.
    fn follow(&mut self, registry: &Registry) -> Advance {
        let mut changed = self.check_staleness();
        if self.state.is_terminal() {
            return self.step(changed);
        }
        if self.probing() {
            changed |= self.probe(registry).changed;
            changed |= self.check_staleness();
            return self.step(changed);
        }
        let Some(mut watch) = self.watch.take() else {
            return self.step(changed);
        };
        let step = watch.tick(registry);
        self.sync_wake(&watch);
        if !matches!(step, WatchStep::Done(_)) {
            self.watch = Some(watch);
        }
        changed |= self.handle_step(step).changed;
        changed |= self.check_staleness();
        self.step(changed)
    }

    /// One status read on the job's own schedule: used while the print needs attention for lack of
    /// news, and after a watch gave up. A good read ends the silence; a bad one is only noted.
    fn probe(&mut self, registry: &Registry) -> Advance {
        let now = self.now();
        if self.probe_at.is_some_and(|at| now < at) {
            return self.step(false);
        }
        self.probe_at = Some(now.saturating_add(self.config.staleness.probe_interval));
        let Some(remote) = self.remote.clone() else {
            return self.step(false);
        };
        let policy = self.watch_policy(None);
        let mut watch = match registry.job_watch(&remote, policy) {
            Ok(watch) => watch.with_clock(Arc::clone(&self.clock)).with_expected(self.expected),
            Err(error) => {
                self.note(format!("cannot follow the job: {error}"));
                return self.step(false);
            }
        };
        let step = watch.poll_status(registry);
        self.sync_wake(&watch);
        let keep = matches!(step, WatchStep::Update(_));
        let advance = self.handle_step(step);
        if keep && self.stale.is_none() && !self.state.is_terminal() {
            self.watch = Some(watch);
            self.probe_at = None;
        }
        advance
    }

    /// Apply one watch step: a status update is absorbed, a failed read is noted, and a finished watch
    /// goes to [`Self::watch_done`].
    fn handle_step(&mut self, step: WatchStep) -> Advance {
        match step {
            WatchStep::Pending { next_in } => Advance { changed: false, next_in: Some(next_in) },
            WatchStep::Update(status) => {
                let changed = self.absorb_status(status);
                self.step(changed)
            }
            WatchStep::Retrying { error, attempt, next_in } => {
                self.note(format!("status read {attempt} failed ({error}); trying again"));
                Advance { changed: false, next_in: Some(next_in) }
            }
            WatchStep::Done(outcome) => self.watch_done(outcome),
        }
    }

    /// The watch ended: settle the print from its outcome. A forgotten job most likely finished, a
    /// missing printer fails the print, and a timeout or failing reads either hand over to probing or
    /// leave the print accepted with a warning.
    fn watch_done(&mut self, outcome: WatchOutcome) -> Advance {
        self.watch = None;
        match outcome {
            WatchOutcome::Terminal(status) => {
                self.absorb_status(status);
            }
            WatchOutcome::Forgotten { .. } => {
                if self.cancel_requested {
                    self.end_remote(JobState::Cancelled, Outcome::Cancelled);
                } else {
                    self.warn("the printer no longer lists the job; printers forget finished jobs, so it most likely finished");
                    self.set_state(LifecycleState::Terminal(Outcome::CompletedWithWarnings));
                }
            }
            WatchOutcome::NoStatus => {
                self.set_state(LifecycleState::Accepted);
            }
            WatchOutcome::TimedOut { .. } if self.config.staleness.enabled && self.watchable => {
                self.note("the watch ran out; probing on");
                self.probe_at = Some(self.now().saturating_add(self.config.staleness.probe_interval));
            }
            // The printer itself is not found (wrong address or queue path): a configuration error.
            // Probing on, reconciling or waiting for a ceiling cannot help; say so and stop.
            WatchOutcome::Failed(error) if error.kind() == ErrorKind::NotFound && error.subject() == NotFoundSubject::Printer => {
                self.warn(format!("the printer was not found, so the job cannot be followed: check the printer address or queue path ({error})"));
                self.last_error = Some(AttemptError::from(&error));
                self.probe_at = None;
                self.stale = None;
                self.set_state(LifecycleState::Terminal(Outcome::Failed));
            }
            WatchOutcome::Failed(error) if self.config.staleness.enabled && self.watchable => {
                self.note(format!("status reads keep failing ({error}); probing on"));
                self.probe_at = Some(self.now().saturating_add(self.config.staleness.probe_interval));
            }
            WatchOutcome::TimedOut { .. } => {
                self.warn("stopped following the job; it may still be running at the printer");
                self.set_state(LifecycleState::Accepted);
            }
            WatchOutcome::Failed(error) => {
                self.warn(format!("stopped following the job: {error}"));
                self.last_error = Some(AttemptError::from(&error));
                self.set_state(LifecycleState::Accepted);
            }
        }
        self.step(true)
    }

    /// Take a printer reading: progress, reasons, the trail (a reading identical to the previous one
    /// is not recorded again) and the lifecycle state. Returns whether anything changed.
    fn absorb_status(&mut self, mut status: JobStatus) -> bool {
        self.expected.apply(&mut status);
        let new_reading = !self.last_printer.as_ref().is_some_and(|last| last.same_as(&status));
        let now = self.now();
        self.last_news = Some(now);
        self.last_news_wall = Some((self.wall)());
        if new_reading {
            self.changed_at = Some(now);
        }
        if new_reading {
            self.record(TrailEvent::Printer { state: status.state, reasons: status.reasons.clone(), sheets_done: status.sheets_done });
        }
        self.progress.sheets_done = status.sheets_done;
        self.progress.sheets_total = status.sheets_total;
        self.progress.pages_done = status.pages_done;
        self.progress.pages_total = status.pages_total;
        self.reasons = status.reasons.clone();
        let state = status.state;
        let printer_warnings = status.warnings.clone();
        self.last_printer = Some(status);
        let outcome = match state {
            JobState::Completed if printer_warnings.is_empty() => Some(Outcome::Completed),
            JobState::Completed => {
                for warning in printer_warnings {
                    self.warn(warning);
                }
                Some(Outcome::CompletedWithWarnings)
            }
            JobState::CompletedWithErrors => {
                self.ended_with_errors = true;
                Some(Outcome::Failed)
            }
            JobState::Cancelled => Some(Outcome::Cancelled),
            JobState::Aborted => Some(Outcome::Aborted),
            _ => None,
        };
        if let Some(outcome) = outcome {
            self.end_remote(state, outcome);
            return true;
        }
        if self.stale == Some(StaleReason::Stalled) && !new_reading {
            // Still the same thing: the print keeps needing a look.
            return false;
        }
        if self.stale.take().is_some() {
            self.note("news again; following the job");
        }
        let target = match state {
            JobState::Unknown if !matches!(self.state, LifecycleState::Watching(_)) => LifecycleState::Accepted,
            JobState::Unknown => self.state,
            printer => LifecycleState::Watching(printer),
        };
        self.set_state(target) || new_reading
    }

    /// The remote job ended: remember how on its attempt and end the print.
    fn end_remote(&mut self, remote_end: JobState, outcome: Outcome) {
        if let Some(attempt) = Arc::make_mut(&mut self.attempts).iter_mut().rev().find(|a| matches!(a.outcome, AttemptOutcome::Accepted(_))) {
            attempt.remote_end = Some(remote_end);
        }
        if outcome == Outcome::Cancelled && !self.cancel_requested {
            self.note("the job was cancelled at the printer");
        }
        self.watch = None;
        self.stale = None;
        self.set_state(LifecycleState::Terminal(outcome));
    }

    /// Read the job's status now, ignoring the schedule (a person pressed Refresh). Resumes a watch
    /// that stopped (timed out or failed). Does nothing in other states.
    pub fn refresh(&mut self, registry: &Registry) -> Advance {
        let stale_attention = self.state == LifecycleState::NeedsAttention && self.stale.is_some();
        if !matches!(self.state, LifecycleState::Accepted | LifecycleState::Watching(_)) && !stale_attention {
            return self.step(false);
        }
        let Some(remote) = self.remote.clone() else {
            return self.step(false);
        };
        let mut watch = match self.watch.take() {
            Some(watch) => watch,
            None => match registry.job_watch(&remote, self.watch_policy(None)) {
                Ok(watch) => watch.with_clock(Arc::clone(&self.clock)).with_expected(self.expected),
                Err(error) => {
                    self.note(format!("cannot follow the job: {error}"));
                    return self.step(false);
                }
            },
        };
        let step = watch.poll_status(registry);
        self.sync_wake(&watch);
        if !matches!(step, WatchStep::Done(_)) {
            self.watch = Some(watch);
        }
        let advance = self.handle_step(step);
        if self.stale.is_some() {
            // Still no better: keep probing on the slow schedule.
            self.watch = None;
        }
        advance
    }

    /// Cancel the print, in whatever state it is (see `docs/job-tracking.md`, section 6). Before the
    /// document was sent nothing is requested of a backend. After it was accepted this asks the
    /// backend to cancel the remote job (`Err` when it cannot; the state is then unchanged) and the
    /// job is followed until the printer reports how it ended. Cancelling an ended print is not an
    /// error.
    pub fn cancel(&mut self, registry: &Registry) -> Result<(), PrintError> {
        match self.state {
            LifecycleState::Terminal(_) => Ok(()),
            LifecycleState::Preparing | LifecycleState::Submitting | LifecycleState::RetryScheduled { .. } => {
                self.cancel.cancel();
                self.cancel_requested = true;
                self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
                Ok(())
            }
            LifecycleState::Accepted | LifecycleState::Watching(_) => self.cancel_remote(registry),
            LifecycleState::NeedsAttention if self.stale.is_some() && self.remote.is_some() => self.cancel_remote(registry),
            LifecycleState::NeedsAttention => self.cancel_unknown(registry),
        }
    }

    /// Ask the backend to cancel the accepted remote job. With a watch, the status is read once so the
    /// result shows; without one, the print ends cancelled with a warning. Fails if there is no remote
    /// handle or the backend refuses.
    fn cancel_remote(&mut self, registry: &Registry) -> Result<(), PrintError> {
        let Some(remote) = self.remote.clone() else {
            return Err(PrintError::new(ErrorKind::Unsupported, "the backend gave no handle for the job, so it cannot be cancelled from here"));
        };
        if let Err(error) = registry.cancel_job(&remote) {
            self.note(format!("cancel failed: {error}"));
            return Err(error);
        }
        self.cancel_requested = true;
        self.note("cancel requested");
        match self.watch.take() {
            Some(mut watch) => {
                let step = watch.poll_status(registry);
                if !matches!(step, WatchStep::Done(_)) {
                    self.watch = Some(watch);
                }
                self.handle_step(step);
            }
            None => {
                self.warn("cancel requested; the backend cannot report whether it took effect");
                self.end_remote(JobState::Cancelled, Outcome::Cancelled);
            }
        }
        Ok(())
    }

    /// Cancelling while the outcome of a send is unknown: only a job that is provably ours (its tag
    /// matched) is cancelled; a candidate found by title alone could be someone else's.
    fn cancel_unknown(&mut self, registry: &Registry) -> Result<(), PrintError> {
        match self.reconcile(registry) {
            Err(error) => {
                self.warn(format!("cancelled without finding out whether the printer holds a job ({error}); one may still be printing"));
                self.cancel_requested = true;
                self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
                Ok(())
            }
            Ok(found) => {
                let candidates = found.jobs().to_vec();
                let ours: Vec<&RemoteJob> = candidates.iter().filter(|job| job.ours).collect();
                if ours.is_empty() && !candidates.is_empty() {
                    return Err(PrintError::new(
                        ErrorKind::Failed,
                        "the printer lists jobs with this title, but none carries this print's tag; they are not cancelled because they may not be ours",
                    )
                    .with_safety(RetrySafety::Safe));
                }
                for job in ours {
                    registry.cancel_job(&job.id)?;
                    self.note(format!("cancelled {}", job.id));
                }
                self.cancel_requested = true;
                self.set_state(LifecycleState::Terminal(Outcome::Cancelled));
                Ok(())
            }
        }
    }

    /// The fingerprint a reconcile looks for.
    pub fn fingerprint(&self) -> Fingerprint {
        let submitted = self.first_wall.or_else(|| Some((self.wall)())).and_then(Timestamp::from_system_time).unwrap_or_default();
        let fingerprint = Fingerprint::new(self.request.job.title.clone(), submitted).with_window(self.config.reconcile_slack);
        match self.request.tag {
            Some(tag) => fingerprint.with_tag(tag),
            None => fingerprint,
        }
    }

    /// Find out whether an earlier attempt already created a job on the printer: one bounded read
    /// ([`Registry::find_job`]). Nothing is sent that changes a job. The answer is kept (see
    /// [`Snapshot::reconciled`]); `Found` and `Ambiguous` are then adopted with [`Self::accept`], or
    /// the print is sent again with [`Self::retry`]. `Err(Unsupported)` when the backend cannot be
    /// asked: [`RetryAdvice::CheckDeviceFirst`] stays.
    pub fn reconcile(&mut self, registry: &Registry) -> Result<Reconciled, PrintError> {
        if matches!(self.state, LifecycleState::Preparing | LifecycleState::Submitting) {
            return Err(PrintError::new(ErrorKind::InvalidRequest, "nothing has been sent yet, so there is nothing to reconcile"));
        }
        let Some(printer) = self.request.job.printer.clone() else {
            return Err(PrintError::new(ErrorKind::NotFound, "the print has no printer id to ask (it went to a backend's default printer)"));
        };
        match registry.find_job(&printer, &self.fingerprint()) {
            Ok(found) => {
                self.note(match &found {
                    Reconciled::NotFound => "reconcile: the printer lists no matching job".to_owned(),
                    Reconciled::Found(job) => format!("reconcile: found {}", job.id),
                    Reconciled::Ambiguous(jobs) => format!("reconcile: {} jobs match", jobs.len()),
                });
                self.reconciled = Some(found.clone());
                Ok(found)
            }
            Err(error) => {
                self.note(format!("reconcile failed: {error}"));
                Err(error)
            }
        }
    }

    /// Adopt a job that [`Self::reconcile`] found as the outcome of the attempt that was unknown:
    /// the print carries on as `Watching` that job, with no second copy sent.
    pub fn accept(&mut self, registry: &Registry, job: &JobId) -> Result<(), PrintError> {
        let found = (self.state == LifecycleState::NeedsAttention)
            .then(|| self.reconciled.as_ref().and_then(|r| r.jobs().iter().find(|candidate| &candidate.id == job).cloned()))
            .flatten();
        let Some(found) = found else {
            return Err(PrintError::new(
                ErrorKind::InvalidRequest,
                format!(
                    "{job} is not a job the last reconcile found for this print (reconcile first; only a print that needs attention can adopt one)"
                ),
            ));
        };
        let now = self.now();
        if let Some(attempt) = self.latest_attempt() {
            attempt.ended.get_or_insert(now);
            attempt.outcome = AttemptOutcome::Accepted(Some(job.clone()));
        }
        self.note(format!("adopted {job} after reconcile"));
        self.auto_failures = 0;
        self.last_error = None;
        self.reconciled = None;
        self.remote = Some(job.clone());
        self.watch =
            registry.job_watch(job, self.watch_policy(None)).ok().map(|watch| watch.with_clock(Arc::clone(&self.clock)).with_expected(self.expected));
        self.start_following(self.watch.is_some(), now);
        self.set_state(LifecycleState::Accepted);
        let mut status = JobStatus::new(found.state);
        status.reasons = found.reasons;
        status.sheets_done = found.sheets_done;
        status.pages_done = found.pages_done;
        self.absorb_status(status);
        Ok(())
    }

    /// Start a new attempt on purpose. Allowed after `Failed`, `Aborted` or `Cancelled`, from
    /// `NeedsAttention` (the person decided to send again; see [`RetryAdvice`]), and from
    /// `RetryScheduled` (retry now). The attempt history is kept; the new attempt gets a new remote
    /// job id. A print that already printed is [`ErrorKind::InvalidRequest`].
    pub fn retry(&mut self) -> Result<(), PrintError> {
        match self.state {
            LifecycleState::RetryScheduled { attempt, .. } => {
                let now = self.now();
                self.set_state(LifecycleState::RetryScheduled { attempt, next_at: now });
                Ok(())
            }
            LifecycleState::Terminal(Outcome::Failed | Outcome::Aborted | Outcome::Cancelled | Outcome::Abandoned)
            | LifecycleState::NeedsAttention => {
                self.note("retry requested");
                self.stale = None;
                self.accepted_at = None;
                self.probe_at = None;
                self.unknown_since = None;
                self.watchable = false;
                self.cancel = CancelToken::new();
                self.cancel_requested = false;
                self.ended_with_errors = false;
                self.auto_failures = 0;
                self.reconciled = None;
                self.remote = None;
                self.watch = None;
                self.last_printer = None;
                self.reasons.clear();
                self.progress = JobProgress::default();
                self.set_state(LifecycleState::Submitting);
                Ok(())
            }
            LifecycleState::Terminal(_) => {
                Err(PrintError::new(ErrorKind::InvalidRequest, "the print already completed; sending it again would print it twice"))
            }
            _ => Err(PrintError::new(ErrorKind::InvalidRequest, "the print is still in progress; there is nothing to retry yet")),
        }
    }

    /// What a UI should offer about trying again right now; see [`RetryAdvice`].
    pub fn retry_advice(&self) -> RetryAdvice {
        match self.state {
            LifecycleState::Preparing | LifecycleState::Submitting | LifecycleState::Watching(_) => RetryAdvice::DoNotRetry,
            LifecycleState::Accepted => {
                if self.last_error.is_some() {
                    RetryAdvice::CheckDeviceFirst
                } else {
                    RetryAdvice::DoNotRetry
                }
            }
            LifecycleState::RetryScheduled { next_at, .. } => RetryAdvice::RetryAfter(next_at.saturating_sub(self.now())),
            LifecycleState::NeedsAttention if self.stale.is_some() => RetryAdvice::CheckDeviceFirst,
            LifecycleState::NeedsAttention => match &self.reconciled {
                // The device gave up on a half-made job on its own: nothing will print.
                Some(Reconciled::Found(job)) if job.state == JobState::Aborted && gave_up_waiting(&job.reasons) => RetryAdvice::RetryNow,
                Some(Reconciled::Found(_)) => RetryAdvice::DoNotRetry,
                Some(Reconciled::NotFound) if self.info.is_some_and(|info| info.query_caps.history) && self.reconcile_wait().is_none() => {
                    RetryAdvice::RetryNow
                }
                _ => RetryAdvice::CheckDeviceFirst,
            },
            LifecycleState::Terminal(Outcome::Completed | Outcome::CompletedWithWarnings | Outcome::Cancelled) => RetryAdvice::DoNotRetry,
            LifecycleState::Terminal(Outcome::Aborted | Outcome::Abandoned) => RetryAdvice::CheckDeviceFirst,
            LifecycleState::Terminal(Outcome::Failed) => match &self.last_error {
                _ if self.ended_with_errors => RetryAdvice::CheckDeviceFirst,
                Some(error) if error.is_retryable() && error.safety.allows_automatic_retry() => match error.retry_after {
                    Some(wait) => RetryAdvice::RetryAfter(wait),
                    None => RetryAdvice::RetryNow,
                },
                Some(_) => RetryAdvice::DoNotRetry,
                None => RetryAdvice::CheckDeviceFirst,
            },
        }
    }

    /// The wall-clock times of the print: created, first sent, ended. The job's clock is monotonic,
    /// so the end is the creation time plus the clock's offset; the start is the wall time of the
    /// first send.
    fn times(&self) -> JobTimes {
        let mut times = JobTimes::new();
        if let Some(created) = Timestamp::from_system_time(self.created_wall) {
            times = times.with_created(created);
        }
        if let Some(started) = self.first_wall.and_then(Timestamp::from_system_time) {
            times = times.with_started(started);
        }
        // A creation time at the far end of what the platform's clock holds (an injected wall clock)
        // cannot take the offset: no end time then, never a panic.
        let finished =
            self.finished_at.and_then(|end| self.created_wall.checked_add(end.saturating_sub(self.created))).and_then(Timestamp::from_system_time);
        if let Some(finished) = finished {
            times = times.with_finished(finished);
        }
        times
    }

    /// The print as a [`craft_job::JobView`], the record a UI shows the same way for a print and a
    /// scan: no I/O, nothing blocks. Shorthand for `self.snapshot().view()`; see [`Snapshot::view`]
    /// for how each field is filled.
    pub fn view(&self) -> JobView {
        self.snapshot().view()
    }

    /// An immutable view for a UI: no I/O, nothing blocks, the attempt timeline and the trail are
    /// shared rather than copied.
    pub fn snapshot(&self) -> Snapshot {
        let native = NativeInfo {
            effective: self.request.effective.clone(),
            remote_native: self.remote.as_ref().map(|job| job.native().to_owned()),
            error_native: self.last_error.as_ref().and_then(|error| error.native),
        };
        let has_native = native.effective.is_some() || native.remote_native.is_some() || native.error_native.is_some();
        Snapshot {
            id: self.id,
            title: self.request.job.title.clone(),
            printer: self.request.job.printer.clone(),
            state: self.state,
            simple: simplify(self.state),
            settled: self.is_settled(),
            progress: self.progress,
            reasons: self.reasons.clone(),
            warnings: self.warnings.clone(),
            last_error: self.last_error.clone(),
            attempt: self.attempt_counter,
            remote: self.remote.clone(),
            tag: self.tag,
            tag_applied: self.tag_applied,
            document: self.request.document,
            conversion: self.request.conversion.clone(),
            advice: self.retry_advice(),
            stale: self.stale,
            last_news: self.last_news_wall.and_then(Timestamp::from_system_time),
            reconciled: self.reconciled.clone(),
            native: has_native.then_some(native),
            elapsed: self.now().saturating_sub(self.created),
            times: self.times(),
            expected_pages: self.request.options.expected_pages,
            attempts: Arc::clone(&self.attempts),
            trail: Arc::clone(&self.trail),
            trail_dropped: self.trail_dropped,
        }
    }
}

/// Whether a job's reasons say the device ended a half-made job on its own (WS-Print
/// `DocumentTimeoutError`; IPP `submission-interrupted`, `aborted-by-system`, `document-timeout`).
fn gave_up_waiting(reasons: &[String]) -> bool {
    reasons.iter().any(|reason| {
        let reason = reason.to_ascii_lowercase();
        reason.contains("timeout") || reason.contains("time-out") || reason.contains("submission-interrupted") || reason.contains("aborted-by-system")
    })
}

/// A short human name of an error kind, for trail notes.
fn kind_name(kind: ErrorKind) -> &'static str {
    match kind {
        ErrorKind::Unsupported => "unsupported",
        ErrorKind::UnknownBackend => "unknown backend",
        ErrorKind::NotFound => "not found",
        ErrorKind::InvalidRequest => "invalid request",
        ErrorKind::UnsupportedFormat => "unsupported format",
        ErrorKind::ConsentRequired => "consent required",
        ErrorKind::Cancelled => "cancelled",
        ErrorKind::Unavailable => "unavailable",
        ErrorKind::Busy => "busy",
        ErrorKind::Timeout => "timeout",
        ErrorKind::Rejected => "rejected",
        ErrorKind::Io => "i/o",
        ErrorKind::Failed => "failed",
    }
}

#[cfg(test)]
mod news_tests {
    use super::*;
    use crate::{BytesSource, DocumentFormat};

    /// A print of a tiny PDF with default options and no printer chosen.
    fn job() -> PrintJob {
        PrintJob::new(Job::default(), BytesSource::new(DocumentFormat::Pdf, b"%PDF-1.7".to_vec()))
    }

    /// `n` seconds.
    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    #[test]
    fn the_silence_clock_starts_at_the_last_good_read_without_events() {
        let mut job = job();
        job.last_news = Some(secs(100));
        assert_eq!(job.news_anchor(secs(10), secs(60)), secs(100));
        job.last_news = None;
        assert_eq!(job.news_anchor(secs(10), secs(60)), secs(10));
    }

    #[test]
    fn an_event_is_news_but_postpones_the_silence_by_at_most_twice_the_limit() {
        let mut job = job();
        job.last_news = Some(secs(100));
        // A recent event moves the anchor forward ...
        job.last_event = Some(secs(130));
        assert_eq!(job.news_anchor(secs(10), secs(60)), secs(130));
        // ... but never beyond the last good read plus twice the silence.
        job.last_event = Some(secs(100 + 500));
        assert_eq!(job.news_anchor(secs(10), secs(60)), secs(220));
        // An event older than the read changes nothing.
        job.last_event = Some(secs(50));
        assert_eq!(job.news_anchor(secs(10), secs(60)), secs(100));
    }
}
