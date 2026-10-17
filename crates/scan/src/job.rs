//! [`ScanJob`]: one user-visible acquisition, from the press of Scan to the last page, across the
//! attempts it takes. See `docs/scan-job.md` for the rules and the limits.
//!
//! Scanning is blocking and pull-based: the app runs [`crate::Registry::acquire`] on a worker thread of
//! its own. A `ScanJob` does not run it. It **plans** an attempt ([`ScanJob::begin_attempt`]), **records**
//! what comes back (pages streamed through the [`ProgressToken`], then the result), and says what may
//! happen next ([`ScanJob::retry_advice`]). It holds pages, so a failed attempt does not lose them, and
//! produces a [`craft_job::JobView`] for a UI. It has no thread, no timer, no clock (the caller passes
//! the time) and no async.
//!
//! ```
//! use craft_job::{RetryAdvice, Timestamp};
//! use craft_scan::job::{Continuation, ScanJob, ScanJobConfig};
//! use craft_scan::{Acquired, Page, ScanError, ScanSettings, Source};
//!
//! let settings = ScanSettings { device: "demo:1".into(), source: Source::Feeder, pages: 3, ..ScanSettings::default() };
//! let mut job = ScanJob::new(settings, ScanJobConfig::default().with_title("Contract"));
//! let t = Timestamp::from_secs;
//!
//! let page = || Page { bytes: vec![0; 8], format: "jpeg", width: 4, height: 4, dpi: 200, dpi_y: 200, requested_dpi: 200, thresholded: false, length: Default::default() };
//! let plan = job.begin_attempt(t(1))?;
//! assert_eq!(plan.settings.pages, 3);
//! // ... the app runs `craft_scan::acquire(&plan.settings)` on its worker thread. Say the feeder
//! // ran dry after two sheets:
//! let shortfall = ScanError::Incomplete {
//!     message: craft_scan::feeder_shortfall_message(3, 2),
//!     acquired: Acquired { source: Source::Feeder, pages: vec![page(), page()] },
//!     cause: Some(craft_scan::Cause { kind: craft_scan::InterruptKind::FeederEmpty, native: None }),
//! };
//! job.finish_attempt(Err(shortfall), t(2))?;
//! assert_eq!(job.snapshot().pages.len(), 2); // kept
//! assert_eq!(job.retry_advice(), RetryAdvice::DoNotRetry); // a person reloads the feeder
//!
//! // The person reloaded and chose to continue: scan the one missing page onto the two kept.
//! let plan = job.begin_attempt_with(t(30), Continuation::Append)?;
//! assert_eq!(plan.settings.pages, 1);
//! job.finish_attempt(Ok(Acquired { source: Source::Feeder, pages: vec![page()] }), t(32))?;
//! assert!(job.state().is_terminal());
//! assert_eq!(job.take_pages()?.len(), 3);
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::sync::atomic::{AtomicU64, Ordering};

use craft_job::{
    BackendName, DocumentRef, ErrorSummary, Fingerprint, JobDocuments, JobId, JobKind, JobState, JobTimes, JobView, Progress, RetryAdvice,
    RetryFacts, RetryPolicy, RetrySafety, TagSupport, Timestamp, advise,
};

use crate::{
    Acquired, CancelToken, ErrorContext, HandlingStatus, InterruptKind, LengthRecord, Page, Phase, ProgressToken, ScanError, ScanErrorInfo,
    ScanSettings, Scanner, Source, auto_fallback_to_glass, split_id,
};

mod device;

pub use device::device_view;

static NEXT_LOCAL_ID: AtomicU64 = AtomicU64::new(1);

/// The id of a scan job inside this process. It never changes across attempts, so a UI row can bind to it.
/// (A scan has no device-side job handle to use instead: see the design note.)
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct LocalJobId(u64);

impl LocalJobId {
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for LocalJobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "local-{}", self.0)
    }
}

/// How the next attempt treats the pages already kept. There is deliberately no default: where the
/// paper stopped is not knowable, so the person (or the app on their behalf) says.
///
/// `Resume` (continue after the last good page without feeding again) is not offered: no backend reports
/// that it can.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Continuation {
    /// Keep the pages and scan the missing ones after them. The attempt asks for what is left (when the
    /// total is known); the person has put the remaining sheets in the feeder.
    Append,
    /// Throw the kept pages away and scan the whole job again. With nothing kept it is the same as `Append`.
    Restart,
}

impl Continuation {
    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Append => "append",
            Self::Restart => "restart",
        }
    }
}

/// How a finished scan job ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum Outcome {
    /// Every page that was wanted arrived (or the feeder emptied normally).
    Completed,
    /// The person accepted a partial stack, or the device delivered fewer pages than asked.
    CompletedWithWarnings,
    /// Stopped by a failure that nothing in the job can cure. [`ScanJob::begin_attempt`] may still start over.
    Failed,
    /// This side stopped it ([`ScanJob::cancel`]). Pages received before the stop are kept.
    Cancelled,
    /// The device ended it ([`ScanJob::device_cancelled`]): the Cancel button on the scanner, or the
    /// device dropping the job. Pages received before are kept. Not a failure of this side.
    CancelledByDevice,
}

impl Outcome {
    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::CompletedWithWarnings => "completed_with_warnings",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
            Self::CancelledByDevice => "cancelled_by_device",
        }
    }
}

/// The lifecycle of the job as a whole. It is separate from anything the device reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ScanJobState {
    /// Created; the first attempt has not begun.
    Preparing,
    /// An attempt is running on the app's worker. It ends with [`ScanJob::finish_attempt`].
    Acquiring,
    /// The last attempt stopped before the job was done, and the cause is known: a jam, an empty feeder with
    /// pages still wanted, a cover, a busy scanner. Pages are kept. [`ScanJob::retry_advice`] says whether
    /// the app may go on by itself (a busy scanner with nothing kept) or must ask the person: continue
    /// ([`Continuation::Append`]), start over ([`Continuation::Restart`]) or [`ScanJob::finish`].
    Interrupted,
    /// The last attempt ended and it is not known whether the scanner did more than we were told (the
    /// connection dropped after pages arrived, or part-way through a page). Nothing is resent. The person
    /// looks at the scanner; the exits are those of `Interrupted`.
    NeedsAttention,
    /// Nothing more happens without an explicit new start.
    Terminal(Outcome),
}

impl ScanJobState {
    /// The job ended ([`Outcome::Failed`] can still be started over by hand).
    pub const fn is_terminal(self) -> bool {
        matches!(self, Self::Terminal(_))
    }

    /// A stable lowercase name.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Preparing => "preparing",
            Self::Acquiring => "acquiring",
            Self::Interrupted => "interrupted",
            Self::NeedsAttention => "needs_attention",
            Self::Terminal(outcome) => outcome.name(),
        }
    }
}

/// What a command-line tool maps to an exit code. Stable names; coarser than [`ScanJobState`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum ExitClass {
    /// Every wanted page arrived (or the feeder emptied normally).
    Completed,
    /// Not finished, or finished short: a person must look (interrupted, outcome unknown, a partial stack
    /// accepted or delivered, or still running).
    NeedsAttention,
    /// Stopped by a failure.
    Failed,
    /// Stopped on purpose, by this side or by the scanner.
    Cancelled,
}

impl ExitClass {
    /// `completed`, `needs_attention`, `failed`, `cancelled`.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Completed => "completed",
            Self::NeedsAttention => "needs_attention",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

impl ScanJobState {
    /// The coarse class: `Terminal(Completed)` is `Completed`, `CompletedWithWarnings` is `NeedsAttention` (pages
    /// are missing), `Failed` is `Failed`, both cancels are `Cancelled`, every state that is not an end is
    /// `NeedsAttention`.
    pub const fn exit_class(self) -> ExitClass {
        match self {
            Self::Terminal(Outcome::Completed) => ExitClass::Completed,
            Self::Terminal(Outcome::Failed) => ExitClass::Failed,
            Self::Terminal(Outcome::Cancelled | Outcome::CancelledByDevice) => ExitClass::Cancelled,
            _ => ExitClass::NeedsAttention,
        }
    }
}

/// Why a [`ScanJob`] call was refused. The job is unchanged unless the variant says otherwise.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum ScanJobError {
    /// An attempt is running, or the job ended and cannot be restarted.
    #[error("a scan job that is {} cannot start another attempt", .0.name())]
    NotStartable(ScanJobState),
    /// Pages are kept, so the caller must say what to do with them: [`ScanJob::begin_attempt_with`].
    #[error("{kept} pages are kept: say whether to append to them or restart")]
    ContinuationRequired { kept: u32 },
    /// Every wanted page is already kept; there is nothing to append. Use [`ScanJob::finish`].
    #[error("all {kept} wanted pages are already kept")]
    NothingLeft { kept: u32 },
    /// The call needs a running attempt.
    #[error("no attempt is running")]
    NotAcquiring,
    /// The call is only for a job that is waiting on a person.
    #[error("the scan job is {}, not waiting for a decision", .0.name())]
    NotWaiting(ScanJobState),
    /// [`ScanJob::finish`] with no pages: there is nothing to accept.
    #[error("no pages were received")]
    NothingToFinish,
    /// Pages cannot be handed over while an attempt is running.
    #[error("pages cannot be taken while an attempt is running")]
    AttemptRunning,
    /// The source cannot be decided: the scanner has neither a feeder nor a glass. The job is `Failed`.
    #[error("{0}")]
    NoSource(String),
}

/// What a caller says about the job once, at creation.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ScanJobConfig {
    /// The title a UI shows. Default `"Scan"`.
    pub title: String,
    /// Applied to a failure that is transient and safe to repeat. Default [`RetryPolicy::default`].
    pub retry: RetryPolicy,
    /// The scanner's display name, for the view.
    pub device_name: Option<String>,
    /// The scanner has a feeder (default `true`; [`Self::with_scanner`] reads it).
    pub has_feeder: bool,
    /// The scanner has a glass (default `true`).
    pub has_glass: bool,
    /// Whether paper is loaded, when known; `None` for a driver that cannot say (network drivers).
    pub feeder_ready: Option<bool>,
    /// What the backend can carry as an idempotency tag. Default [`TagSupport::None`]: it is `None` for every
    /// current backend. See `docs/scan-job.md`.
    pub tag_support: TagSupport,
    /// How many attempt records the timeline keeps (the oldest are dropped). Default 32.
    pub timeline_len: usize,
    /// Seeds the retry jitter, mixed with the job id.
    pub jitter_seed: u64,
    /// When the job was created.
    pub created: Timestamp,
}

impl Default for ScanJobConfig {
    fn default() -> Self {
        Self {
            title: "Scan".to_string(),
            retry: RetryPolicy::default(),
            device_name: None,
            has_feeder: true,
            has_glass: true,
            feeder_ready: None,
            tag_support: TagSupport::None,
            timeline_len: 32,
            jitter_seed: 0,
            created: Timestamp::default(),
        }
    }
}

impl ScanJobConfig {
    #[must_use]
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = title.into();
        self
    }

    #[must_use]
    pub fn with_retry(mut self, retry: RetryPolicy) -> Self {
        self.retry = retry;
        self
    }

    /// Take the name, the sources and the feeder reading from a scanner's details.
    #[must_use]
    pub fn with_scanner(mut self, scanner: &Scanner) -> Self {
        self.device_name = Some(scanner.name.clone());
        self.has_feeder = scanner.feeder;
        self.has_glass = scanner.flatbed;
        self.feeder_ready = scanner.feeder_ready;
        self
    }

    #[must_use]
    pub fn with_capabilities(mut self, has_feeder: bool, has_glass: bool, feeder_ready: Option<bool>) -> Self {
        (self.has_feeder, self.has_glass, self.feeder_ready) = (has_feeder, has_glass, feeder_ready);
        self
    }

    #[must_use]
    pub fn with_tag_support(mut self, support: TagSupport) -> Self {
        self.tag_support = support;
        self
    }

    #[must_use]
    pub fn with_jitter_seed(mut self, seed: u64) -> Self {
        self.jitter_seed = seed;
        self
    }

    #[must_use]
    pub fn with_created(mut self, created: Timestamp) -> Self {
        self.created = created;
        self
    }

    #[must_use]
    pub fn with_timeline_len(mut self, len: usize) -> Self {
        self.timeline_len = len.max(1);
        self
    }
}

/// What the app runs for one attempt.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct AttemptPlan {
    /// 1 for the first attempt of the job.
    pub number: u32,
    /// Pass this to `acquire` on the worker thread. `pages` is what is still wanted, `source` is the one
    /// the job decided (see [`Self::source_guess`]), and it carries this attempt's [`ProgressToken`] and
    /// [`CancelToken`].
    pub settings: ScanSettings,
    pub continuation: Continuation,
    /// Pages kept before this attempt.
    pub kept_pages: u32,
    /// The source asked for. `Auto` is left to the backend, which reads a fresh status.
    pub source_requested: Source,
    /// What `Auto` is expected to resolve to, from what the job knows ([`Source::resolve`]); `None` unless
    /// the request was `Auto`. After an `Auto` fallback the request itself is `Glass`.
    pub source_guess: Option<Source>,
    /// This attempt exists because the previous one found the feeder empty under `Auto`.
    pub fell_back_to_glass: bool,
    /// Pages stream through this token: [`ScanJob::record_progress`] it from the thread that owns the job.
    pub progress: ProgressToken,
    /// Request it (from any thread) to stop the transfer. [`ScanJob::cancel`] does so too.
    pub cancel: CancelToken,
}

/// One page, without its bytes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct PageInfo {
    /// 1-based position in the job.
    pub number: u32,
    /// The attempt that delivered it.
    pub attempt: u32,
    /// The source it came from (never `Auto`); `None` when nothing said.
    pub source: Option<Source>,
    pub format: &'static str,
    pub width: u32,
    pub height: u32,
    pub dpi: u32,
    pub bytes: u64,
    /// How the page's length was obtained (see [`crate::PageLength`]).
    pub length: LengthRecord,
}

/// How an attempt ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "snake_case"))]
pub enum AttemptOutcome {
    /// Still running.
    InProgress,
    /// It delivered the job.
    Completed,
    /// It stopped for a known cause; the job waits (or retries).
    Interrupted,
    /// It stopped and what the scanner did is unknown.
    OutcomeUnknown,
    /// Under `Auto`, the feeder was empty and nothing had arrived: the next attempt uses the glass.
    FellBackToGlass,
    Failed,
    Cancelled,
    CancelledByDevice,
}

/// A failure of one attempt, as data.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AttemptFailure {
    /// The kind, phase, native code and sentence of the error (see [`ScanErrorInfo`]).
    pub info: ScanErrorInfo,
    /// What repeating the acquire would do: `OutcomeUnknown` when the transfer may have done more than
    /// we were told, else `Safe`. Never `Idempotent`: a second acquire always moves the carriage.
    pub safety: RetrySafety,
    /// The condition may clear by itself (busy, a dropped link).
    pub transient: bool,
}

/// One entry of the attempt timeline.
#[derive(Clone, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct AttemptRecord {
    pub number: u32,
    pub started: Timestamp,
    #[cfg_attr(feature = "serde", serde(default))]
    pub ended: Option<Timestamp>,
    pub continuation: Continuation,
    pub source_requested: Source,
    /// The source the transfer used, when known.
    #[cfg_attr(feature = "serde", serde(default))]
    pub source_used: Option<Source>,
    /// The backend used the glass although the plan was the feeder (`Feeder`, or `Auto` that guessed the
    /// feeder): an `Auto` that fell back inside the backend because the feeder gave no pages. The job never
    /// sees the fallback happen; it reads it from the source the backend reports in [`Acquired`]. This is not
    /// the job's own retry on the glass after [`AttemptOutcome::FellBackToGlass`] (that attempt asks for the glass).
    #[cfg_attr(feature = "serde", serde(default))]
    pub fell_back: bool,
    pub pages_before: u32,
    /// New pages this attempt kept (also when it failed).
    pub pages_gained: u32,
    /// How far it got when it stopped (`BeforeFirstPage`, `BetweenPages`, `MidPage`).
    pub phase: Option<Phase>,
    pub outcome: AttemptOutcome,
    #[cfg_attr(feature = "serde", serde(default))]
    pub error: Option<AttemptFailure>,
}

/// The page currently crossing the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct InFlight {
    pub percent: Option<u8>,
    pub bytes_received: u64,
    pub bytes_expected: Option<u64>,
}

/// A cheap picture of the job: no page bytes, nothing that blocks. Clone it to a UI every frame.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanJobSnapshot {
    pub id: LocalJobId,
    pub title: String,
    /// `ScanSettings::device` as given.
    pub device: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub device_name: Option<String>,
    pub state: ScanJobState,
    /// Nothing more will happen without a person or a new start.
    pub settled: bool,
    /// The pages kept so far, in order, with their sizes and sources. First-class: they survive a failed attempt.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pages: Vec<PageInfo>,
    /// Bytes of the kept pages.
    pub bytes: u64,
    /// Pages wanted, when known (`ScanSettings::pages`, not for hardware duplex). `None` is "the whole feeder".
    pub expected_pages: Option<u32>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub in_flight: Option<InFlight>,
    pub source_requested: Source,
    /// The source of the latest attempt, when known.
    #[cfg_attr(feature = "serde", serde(default))]
    pub source_in_use: Option<Source>,
    /// Attempts made, including ones dropped from the timeline.
    pub attempts_total: u32,
    /// The latest attempts, oldest first (bounded by `ScanJobConfig::timeline_len`).
    #[cfg_attr(feature = "serde", serde(default))]
    pub attempts: Vec<AttemptRecord>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub last_error: Option<AttemptFailure>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub warnings: Vec<String>,
    pub advice: RetryAdvice,
    /// The person asked to stop; the running attempt has not ended yet.
    pub cancel_requested: bool,
    pub times: JobTimes,
    /// What a reconcile would look for. Nothing can look today: see `docs/scan-job.md`.
    pub fingerprint: Fingerprint,
    pub tag_support: TagSupport,
}

/// `PageInfo` as read: `format` is a `&'static str`, so it is matched against the formats a page can carry
/// (`bmp`, `png`, `jpeg`, `tiff`, `pdf`); anything else reads as `bin`, which [`crate::image_format`] also
/// answers for bytes it does not know.
#[cfg(feature = "serde")]
impl<'de> serde::Deserialize<'de> for PageInfo {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(serde::Deserialize)]
        struct Wire {
            number: u32,
            #[serde(default)]
            attempt: u32,
            #[serde(default)]
            source: Option<Source>,
            format: String,
            #[serde(default)]
            width: u32,
            #[serde(default)]
            height: u32,
            #[serde(default)]
            dpi: u32,
            #[serde(default)]
            bytes: u64,
            #[serde(default)]
            length: LengthRecord,
        }
        let wire = Wire::deserialize(deserializer)?;
        let format = ["bmp", "png", "jpeg", "tiff", "pdf"].into_iter().find(|f| *f == wire.format).unwrap_or("bin");
        Ok(Self {
            number: wire.number,
            attempt: wire.attempt,
            source: wire.source,
            format,
            width: wire.width,
            height: wire.height,
            dpi: wire.dpi,
            bytes: wire.bytes,
            length: wire.length,
        })
    }
}

/// What a command-line run writes down so a later run can continue: [`ScanJobSnapshot::into_journal`] makes
/// it, [`ScanJob::continuing_from`] reads it. Plain data (with the `serde` feature, also JSON); the app owns
/// the file and the page files it describes. Never page bytes, tokens or the request itself: the app keeps
/// its own `ScanSettings` (device, resolution, colour, ...) and passes it back.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct ScanJobJournal {
    /// [`Self::VERSION`] when written. A reader ignores fields it does not know and defaults ones it lacks.
    #[cfg_attr(feature = "serde", serde(default = "ScanJobJournal::current_version"))]
    pub version: u32,
    pub title: String,
    #[cfg_attr(feature = "serde", serde(default))]
    pub device: String,
    pub state: ScanJobState,
    /// The wanted page count, when one was set.
    #[cfg_attr(feature = "serde", serde(default))]
    pub expected_pages: Option<u32>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub attempts_total: u32,
    /// The kept pages, without bytes.
    #[cfg_attr(feature = "serde", serde(default))]
    pub pages: Vec<PageInfo>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub last_error: Option<AttemptFailure>,
    #[cfg_attr(feature = "serde", serde(default))]
    pub warnings: Vec<String>,
}

impl ScanJobJournal {
    /// The journal format version this crate writes.
    pub const VERSION: u32 = 1;

    #[cfg(feature = "serde")]
    fn current_version() -> u32 {
        Self::VERSION
    }
}

impl ScanJobSnapshot {
    /// The part of the snapshot a later run needs; see [`ScanJobJournal`].
    pub fn into_journal(self) -> ScanJobJournal {
        ScanJobJournal {
            version: ScanJobJournal::VERSION,
            title: self.title,
            device: self.device,
            state: self.state,
            expected_pages: self.expected_pages,
            attempts_total: self.attempts_total,
            pages: self.pages,
            last_error: self.last_error,
            warnings: self.warnings,
        }
    }

    /// [`ScanJobState::exit_class`] of the state.
    pub fn exit_class(&self) -> ExitClass {
        self.state.exit_class()
    }

    /// One line for a terminal: `Invoices: interrupted, 12 of 40 pages, 1 attempt, jam`. Plain ASCII, no
    /// localisation; an app that wants its own wording builds it from the fields.
    pub fn summary_line(&self) -> String {
        let pages = match self.expected_pages {
            Some(total) => format!("{} of {} pages", self.pages.len(), total),
            None => format!("{} pages", self.pages.len()),
        };
        let attempts = if self.attempts_total == 1 { "1 attempt".to_string() } else { format!("{} attempts", self.attempts_total) };
        let mut line = format!("{}: {}, {pages}, {attempts}", self.title, self.state.name());
        if let Some(failure) = &self.last_error {
            line.push_str(", ");
            line.push_str(failure.info.kind.as_str());
        }
        line
    }
}

struct Running {
    number: u32,
    requested: Source,
    guess: Option<Source>,
    pages_before: u32,
    /// Pages of this attempt held at the tail of `ScanJob::pages` (and `infos`).
    absorbed: usize,
    progress: ProgressToken,
    cancel: CancelToken,
    mid_page: bool,
}

/// One user-visible acquisition. See the [module documentation](self).
pub struct ScanJob {
    id: LocalJobId,
    config: ScanJobConfig,
    /// The request as the caller made it; the tokens are replaced per attempt.
    settings: ScanSettings,
    state: ScanJobState,
    /// Page bytes held for the app. [`Self::take_pages`] empties this, not `infos`.
    pages: Vec<Page>,
    infos: Vec<PageInfo>,
    expected: Option<u32>,
    timeline: Vec<AttemptRecord>,
    attempts_total: u32,
    running: Option<Running>,
    forced_source: Option<Source>,
    source_in_use: Option<Source>,
    last_error: Option<AttemptFailure>,
    warnings: Vec<String>,
    stalled: u32,
    advice: RetryAdvice,
    cancel_requested: bool,
    in_flight: Option<InFlight>,
    times: JobTimes,
    fingerprint: Fingerprint,
    feeder_ready: Option<bool>,
}

impl fmt::Debug for ScanJob {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ScanJob")
            .field("id", &self.id)
            .field("state", &self.state)
            .field("pages", &self.infos.len())
            .field("attempts", &self.attempts_total)
            .finish()
    }
}

/// How the engine reads a failure. See `docs/scan-job.md`, "Reading a failure".
struct Reading {
    /// May clear by waiting, and a person need not act.
    transient: bool,
    /// A person can cure it (reload, close the cover, clear the jam).
    person: bool,
    /// The transfer may have done more than we were told.
    unknown: bool,
}

fn read_failure(kind: InterruptKind, kept: usize, mid_page: bool) -> Reading {
    use InterruptKind as K;
    let person = matches!(
        kind,
        K::CoverOpen | K::Jam | K::PaperProblem | K::FeederEmpty | K::MultiFeed | K::Attention | K::DeviceError | K::BusyOrFeederEmpty
    );
    // A link that dropped (the request may have reached the scanner, so even with no page in hand the carriage
    // may have moved), or a failure nobody classified after the transfer began delivering.
    let unknown = kind == K::Offline || (kind == K::Other && (kept > 0 || mid_page));
    // Only a scanner that said "busy" is known to have done nothing.
    let transient = kind == K::Busy;
    Reading { transient, person, unknown }
}

fn expected_from(settings: &ScanSettings) -> Option<u32> {
    (settings.pages > 0 && !settings.duplex).then_some(settings.pages)
}

impl ScanJob {
    /// A job for `settings`. Nothing is sent to a scanner. The source, page count, resolution and the rest are
    /// the request; the tokens in it, if any, are replaced for every attempt.
    pub fn new(settings: ScanSettings, config: ScanJobConfig) -> Self {
        let id = LocalJobId(NEXT_LOCAL_ID.fetch_add(1, Ordering::Relaxed));
        let mut settings = settings;
        settings.cancel = None;
        settings.progress = None;
        let fingerprint = Fingerprint::new(config.title.clone(), config.created);
        let expected = expected_from(&settings);
        let feeder_ready = config.feeder_ready;
        let times = JobTimes::new().with_created(config.created);
        Self {
            id,
            config,
            settings,
            state: ScanJobState::Preparing,
            pages: Vec::new(),
            infos: Vec::new(),
            expected,
            timeline: Vec::new(),
            attempts_total: 0,
            running: None,
            forced_source: None,
            source_in_use: None,
            last_error: None,
            warnings: Vec::new(),
            stalled: 0,
            advice: RetryAdvice::RetryNow,
            cancel_requested: false,
            in_flight: None,
            times,
            fingerprint,
            feeder_ready,
        }
    }

    /// A job that starts where an earlier run stopped: `kept` describes the pages an earlier process received
    /// (the app owns the files; the job holds metadata only). It starts `Interrupted` with those pages counted,
    /// so [`Self::begin_attempt_with`]`(`[`Continuation::Append`]`)` asks for `settings.pages - kept` and the
    /// view reads "12 of 40". `settings` is the **original** request (`pages` is the total). Pages are renumbered
    /// 1..n in the order given, and attempt numbering continues after the highest attempt in `kept`. With no kept
    /// pages this is [`Self::new`]. [`Self::take_pages`] returns only pages received from now on.
    pub fn continuing(settings: ScanSettings, config: ScanJobConfig, kept: Vec<PageInfo>) -> Self {
        let mut job = Self::new(settings, config);
        if kept.is_empty() {
            return job;
        }
        job.attempts_total = kept.iter().map(|p| p.attempt).max().unwrap_or(0);
        job.infos = kept;
        for (index, info) in job.infos.iter_mut().enumerate() {
            info.number = count(index + 1);
        }
        job.state = ScanJobState::Interrupted;
        job.advice = RetryAdvice::DoNotRetry;
        job
    }

    /// [`Self::continuing`] from a [`ScanJobJournal`]. The job starts `NeedsAttention` if the journal says the
    /// outcome was unknown (and stays `CheckDeviceFirst`), else `Interrupted`; the last error, the warnings and
    /// the attempt count carry over. `settings` is the original request; when its `pages` is `0` and the journal
    /// knows the wanted count, that count is used, so `--continue DIR` needs no `--pages`.
    pub fn continuing_from(settings: ScanSettings, config: ScanJobConfig, journal: ScanJobJournal) -> Self {
        let mut settings = settings;
        if settings.pages == 0 && !settings.duplex {
            settings.pages = journal.expected_pages.unwrap_or(0);
        }
        let unknown = journal.state == ScanJobState::NeedsAttention;
        let attempts = journal.attempts_total;
        let mut job = Self::continuing(settings, config, journal.pages);
        if job.infos.is_empty() {
            return job;
        }
        job.attempts_total = job.attempts_total.max(attempts);
        job.last_error = journal.last_error;
        job.warnings = journal.warnings;
        if unknown {
            job.state = ScanJobState::NeedsAttention;
            job.advice = RetryAdvice::CheckDeviceFirst;
        }
        job
    }

    pub fn id(&self) -> LocalJobId {
        self.id
    }

    pub fn state(&self) -> ScanJobState {
        self.state
    }

    pub fn settings(&self) -> &ScanSettings {
        &self.settings
    }

    /// The page bytes still held (those not yet taken). Borrowed, so a preview can read them.
    pub fn pages(&self) -> &[Page] {
        &self.pages
    }

    /// Pages kept so far, taken or not.
    pub fn page_count(&self) -> usize {
        self.infos.len()
    }

    /// Feed a fresh [`crate::status`] read in before the next attempt: it updates what `Auto` is expected to
    /// do. Does not block, does not touch the scanner.
    pub fn observe_status(&mut self, status: Option<HandlingStatus>) {
        self.feeder_ready = status.and_then(|s| s.feeder_ready);
    }

    // ---- attempts -------------------------------------------------------------------------------

    /// Plan the first attempt, or a retry when nothing is kept. With pages kept it refuses
    /// ([`ScanJobError::ContinuationRequired`]): use [`Self::begin_attempt_with`].
    pub fn begin_attempt(&mut self, at: Timestamp) -> Result<AttemptPlan, ScanJobError> {
        let kept = self.infos.len();
        if kept > 0 && !matches!(self.state, ScanJobState::Preparing) {
            return Err(ScanJobError::ContinuationRequired { kept: count(kept) });
        }
        self.begin(at, Continuation::Restart)
    }

    /// Plan the next attempt after an interruption, with the person's choice for the kept pages.
    pub fn begin_attempt_with(&mut self, at: Timestamp, how: Continuation) -> Result<AttemptPlan, ScanJobError> {
        self.begin(at, how)
    }

    fn begin(&mut self, at: Timestamp, how: Continuation) -> Result<AttemptPlan, ScanJobError> {
        match self.state {
            ScanJobState::Preparing | ScanJobState::Interrupted | ScanJobState::NeedsAttention | ScanJobState::Terminal(Outcome::Failed) => {}
            state => return Err(ScanJobError::NotStartable(state)),
        }
        let source_requested = self.forced_source.unwrap_or(self.settings.source);
        let guess = match source_requested {
            Source::Auto => match Source::resolve(Source::Auto, self.feeder_ready, self.config.has_feeder, self.config.has_glass) {
                Ok(source) => Some(source),
                Err(error) => {
                    self.fail_without_attempt(at, &error);
                    return Err(ScanJobError::NoSource(error.to_string()));
                }
            },
            _ => None,
        };
        let restart = how == Continuation::Restart;
        if restart && !self.infos.is_empty() {
            self.pages.clear();
            self.infos.clear();
            self.stalled = 0;
            self.warnings.clear();
        }
        let kept = self.infos.len();
        let remaining = match (restart, self.expected) {
            (false, Some(wanted)) => {
                let left = wanted.saturating_sub(count(kept));
                if left == 0 {
                    return Err(ScanJobError::NothingLeft { kept: count(kept) });
                }
                left
            }
            (true, Some(wanted)) => wanted,
            (_, None) => 0,
        };
        let fell_back = self.forced_source == Some(Source::Glass) && self.settings.source == Source::Auto;
        let progress = ProgressToken::new();
        let cancel = CancelToken::new();
        let mut settings = self.settings.clone();
        settings.source = source_requested;
        settings.pages = if source_requested == Source::Glass { 1 } else { remaining };
        settings.progress = Some(progress.clone());
        settings.cancel = Some(cancel.clone());

        self.attempts_total = self.attempts_total.saturating_add(1);
        let number = self.attempts_total;
        let continuation = if kept == 0 { Continuation::Restart } else { how };
        self.running = Some(Running {
            number,
            requested: source_requested,
            guess,
            pages_before: count(kept),
            absorbed: 0,
            progress: progress.clone(),
            cancel: cancel.clone(),
            mid_page: false,
        });
        self.push_record(AttemptRecord {
            number,
            started: at,
            ended: None,
            continuation,
            source_requested,
            source_used: None,
            fell_back: false,
            pages_before: count(kept),
            pages_gained: 0,
            phase: None,
            outcome: AttemptOutcome::InProgress,
            error: None,
        });
        self.state = ScanJobState::Acquiring;
        self.cancel_requested = false;
        self.in_flight = None;
        self.times.started.get_or_insert(at);
        self.times.finished = None;
        self.advice = RetryAdvice::DoNotRetry;
        Ok(AttemptPlan {
            number,
            settings,
            continuation,
            kept_pages: count(kept),
            source_requested,
            source_guess: guess,
            fell_back_to_glass: fell_back,
            progress,
            cancel,
        })
    }

    fn fail_without_attempt(&mut self, at: Timestamp, error: &ScanError) {
        let info = ScanErrorInfo::new(error, ErrorContext::default());
        self.last_error = Some(AttemptFailure { info, safety: RetrySafety::Safe, transient: false });
        self.finish_with(at, Outcome::Failed);
    }

    /// Take the pages that finished since the last call from the attempt's [`ProgressToken`] and keep them.
    /// Returns how many. Also notes the page in flight and the source. Call it from the thread that owns the
    /// job, as often as a UI refreshes. Does nothing (returns 0) unless an attempt is running.
    pub fn record_progress(&mut self, token: &ProgressToken) -> usize {
        let Some(running) = self.running.as_mut() else { return 0 };
        running.mid_page = token.in_page();
        let in_flight = token.in_page().then(|| InFlight {
            percent: token.page_percent(),
            bytes_received: token.bytes_received(),
            bytes_expected: token.bytes_expected(),
        });
        if let Some(source) = token.source() {
            self.source_in_use = Some(source);
        }
        self.in_flight = in_flight;
        let source = token.source();
        self.absorb(token.take_pages(), source)
    }

    /// Keep pages received some other way (the app drained the token itself). Returns how many were kept.
    /// Does nothing unless an attempt is running.
    pub fn absorb_pages(&mut self, pages: Vec<Page>) -> usize {
        let source = self.running.as_ref().and_then(|r| r.progress.source());
        self.absorb(pages, source)
    }

    fn absorb(&mut self, pages: Vec<Page>, source: Option<Source>) -> usize {
        let Some(running) = self.running.as_mut() else { return 0 };
        let n = pages.len();
        for page in pages {
            let number = count(self.infos.len() + 1);
            self.infos.push(info_of(&page, number, running.number, source));
            self.pages.push(page);
        }
        running.absorbed += n;
        n
    }

    /// The attempt ended with `result`, the return of `acquire`. Records the pages it delivered, decides the
    /// state and the advice. Pages already given to [`Self::record_progress`] are not counted twice: the
    /// result's pages replace them when it has at least as many, otherwise the ones in hand are kept.
    pub fn finish_attempt(&mut self, result: Result<Acquired, ScanError>, at: Timestamp) -> Result<(), ScanJobError> {
        let Some(running) = self.running.as_ref() else { return Err(ScanJobError::NotAcquiring) };
        // Anything the backend pushed that was never drained.
        let leftovers = running.progress.take_pages();
        let token_source = running.progress.source();
        self.absorb(leftovers, token_source);
        let Some(mut running) = self.running.take() else { return Err(ScanJobError::NotAcquiring) };
        running.mid_page = running.mid_page || running.progress.in_page();

        let (acquired, error) = match result {
            Ok(acquired) => (Some(acquired), None),
            Err(ScanError::Incomplete { message, acquired, cause }) => {
                let error = ScanError::Incomplete { message, acquired: Acquired { source: acquired.source, pages: Vec::new() }, cause };
                (Some(acquired), Some(error))
            }
            Err(error) => (None, Some(error)),
        };
        let reported = acquired.as_ref().map(|a| a.source).or(token_source).filter(|s| *s != Source::Auto);
        let used = reported.or(running.guess).filter(|s| *s != Source::Auto);
        // What the plan expected to use: the feeder when asked for it or when Auto guessed it.
        let planned_feeder = match running.requested {
            Source::Auto => running.guess == Some(Source::Feeder),
            other => other == Source::Feeder,
        };
        let fell_back = planned_feeder && reported == Some(Source::Glass);
        if let Some(acquired) = acquired {
            self.merge_result(&mut running, acquired.pages, used);
        }
        self.in_flight = None;
        if used.is_some() {
            self.source_in_use = used;
        }
        let gained = count(self.infos.len()) - running.pages_before;
        let kept = self.infos.len();
        let wanted_this_attempt = self.expected.map(|e| e.saturating_sub(running.pages_before));
        let context = ErrorContext {
            pages_received: usize::try_from(gained).unwrap_or(usize::MAX),
            pages_expected: wanted_this_attempt,
            mid_page: running.mid_page,
        };

        let (outcome, state, advice, failure) = match error {
            None => self.settle_ok(kept),
            Some(error) => self.settle_error(&error, context, &running, used, kept, gained),
        };
        let (outcome, state, advice, failure) = if self.cancel_requested {
            (AttemptOutcome::Cancelled, ScanJobState::Terminal(Outcome::Cancelled), RetryAdvice::DoNotRetry, failure)
        } else {
            (outcome, state, advice, failure)
        };

        if gained > 0 {
            self.stalled = 0;
        }
        self.advice = advice;
        self.last_error = if outcome == AttemptOutcome::Completed { None } else { failure.clone() };
        let phase = Some(Phase::classify(usize::try_from(gained).unwrap_or(usize::MAX), running.mid_page));
        self.update_record(running.number, |record| {
            record.ended = Some(at);
            record.source_used = used;
            record.fell_back = fell_back;
            record.pages_gained = gained;
            record.phase = phase;
            record.outcome = outcome;
            record.error = failure;
        });
        match state {
            ScanJobState::Terminal(done) => self.finish_with(at, done),
            other => self.state = other,
        }
        Ok(())
    }

    /// Replace this attempt's absorbed tail with the result's pages when the result has at least as many.
    fn merge_result(&mut self, running: &mut Running, delivered: Vec<Page>, used: Option<Source>) {
        if delivered.len() < running.absorbed {
            return;
        }
        let keep = self.pages.len().saturating_sub(running.absorbed);
        self.pages.truncate(keep);
        self.infos.truncate(self.infos.len().saturating_sub(running.absorbed));
        running.absorbed = delivered.len();
        for page in delivered {
            let number = count(self.infos.len() + 1);
            self.infos.push(info_of(&page, number, running.number, used));
            self.pages.push(page);
        }
    }

    fn settle_ok(&mut self, kept: usize) -> (AttemptOutcome, ScanJobState, RetryAdvice, Option<AttemptFailure>) {
        if kept == 0 {
            let error = ScanError::Failed("the scanner transferred no pages".into());
            let info = ScanErrorInfo::new(&error, ErrorContext::default());
            let failure = AttemptFailure { info, safety: RetrySafety::Safe, transient: false };
            return (AttemptOutcome::Failed, ScanJobState::Terminal(Outcome::Failed), RetryAdvice::DoNotRetry, Some(failure));
        }
        match self.expected {
            Some(wanted) if count(kept) < wanted => {
                self.warnings.push(shortfall(kept, wanted));
                (AttemptOutcome::Completed, ScanJobState::Terminal(Outcome::CompletedWithWarnings), RetryAdvice::DoNotRetry, None)
            }
            _ => (AttemptOutcome::Completed, ScanJobState::Terminal(Outcome::Completed), RetryAdvice::DoNotRetry, None),
        }
    }

    fn settle_error(
        &mut self,
        error: &ScanError,
        context: ErrorContext,
        running: &Running,
        used: Option<Source>,
        kept: usize,
        gained: u32,
    ) -> (AttemptOutcome, ScanJobState, RetryAdvice, Option<AttemptFailure>) {
        let info = ScanErrorInfo::new(error, context);
        let kind = info.kind;
        let reading = read_failure(kind, kept, running.mid_page);
        let safety = if reading.unknown { RetrySafety::OutcomeUnknown } else { RetrySafety::Safe };
        let failure = AttemptFailure { info, safety, transient: reading.transient };
        let failed = |failure| (AttemptOutcome::Failed, ScanJobState::Terminal(Outcome::Failed), RetryAdvice::DoNotRetry, Some(failure));

        if matches!(error, ScanError::Unsupported) {
            return failed(failure);
        }
        // Auto found the feeder empty before any page: the glass is the answer, no person needed.
        // The scanner itself ended the job: its own decision, not ours and not a fault.
        if kind == InterruptKind::Cancelled {
            return (AttemptOutcome::CancelledByDevice, ScanJobState::Terminal(Outcome::CancelledByDevice), RetryAdvice::DoNotRetry, Some(failure));
        }
        let resolved = used.unwrap_or(Source::Feeder);
        if auto_fallback_to_glass(running.requested, resolved, self.config.has_glass, kept, error) {
            self.forced_source = Some(Source::Glass);
            self.expected = Some(1);
            return (AttemptOutcome::FellBackToGlass, ScanJobState::Interrupted, RetryAdvice::RetryNow, Some(failure));
        }
        // The feeder ran out with pages in hand: a normal end when the whole feeder was wanted or the
        // wanted count is reached; otherwise a stack that stopped short, waiting for paper.
        if kept > 0 && matches!(kind, InterruptKind::FeederEmpty | InterruptKind::BusyOrFeederEmpty) && self.expected.is_none_or(|e| count(kept) >= e)
        {
            return (AttemptOutcome::Completed, ScanJobState::Terminal(Outcome::Completed), RetryAdvice::DoNotRetry, None);
        }
        if reading.unknown {
            self.stalled = self.stalled.saturating_add(1);
            return (AttemptOutcome::OutcomeUnknown, ScanJobState::NeedsAttention, RetryAdvice::CheckDeviceFirst, Some(failure));
        }
        self.stalled = if gained > 0 { 1 } else { self.stalled.saturating_add(1) };
        // Only a failure with nothing kept may be repeated by the app on its own: paper position is not
        // knowable once pages are in hand.
        let retryable = reading.transient && kept == 0;
        let facts = RetryFacts::new(retryable, safety).with_attempts(self.stalled).with_jitter_seed(self.config.jitter_seed ^ self.id.0);
        let advice = advise(&facts, &self.config.retry);
        if kept > 0 || reading.person || advice.is_automatic_retry() {
            return (AttemptOutcome::Interrupted, ScanJobState::Interrupted, advice, Some(failure));
        }
        failed(failure)
    }

    // ---- ends -----------------------------------------------------------------------------------

    /// Stop. Before an attempt or while one waits on a person: the job is `Cancelled` at once. While an attempt
    /// runs: the attempt's [`CancelToken`] is requested and the job ends `Cancelled` when
    /// [`Self::finish_attempt`] reports the worker's return, whatever it returned. Pages already received are
    /// kept (`take_pages`). On an ended job: nothing.
    pub fn cancel(&mut self, at: Timestamp) {
        match self.state {
            ScanJobState::Terminal(_) => {}
            ScanJobState::Acquiring => {
                self.cancel_requested = true;
                if let Some(running) = &self.running {
                    running.cancel.request();
                }
            }
            ScanJobState::Preparing | ScanJobState::Interrupted | ScanJobState::NeedsAttention => {
                self.cancel_requested = true;
                self.finish_with(at, Outcome::Cancelled);
            }
        }
    }

    /// The device ended the job itself (Cancel on the scanner's panel, the job dropped on its side): a different
    /// end from [`Self::cancel`]. No backend reports this through [`ScanError`] today, so the app calls it when
    /// it learns so. Pages received are kept. Needs a running attempt.
    pub fn device_cancelled(&mut self, at: Timestamp) -> Result<(), ScanJobError> {
        let Some(running) = self.running.as_ref() else { return Err(ScanJobError::NotAcquiring) };
        let leftovers = running.progress.take_pages();
        let source = running.progress.source();
        let number = running.number;
        self.absorb(leftovers, source);
        let gained = self.running.take().map_or(0, |r| count(self.infos.len()) - r.pages_before);
        self.update_record(number, |record| {
            record.ended = Some(at);
            record.pages_gained = gained;
            record.outcome = AttemptOutcome::CancelledByDevice;
        });
        self.in_flight = None;
        self.finish_with(at, Outcome::CancelledByDevice);
        Ok(())
    }

    /// The person accepts the pages in hand as the result. `Completed` when nothing more was wanted, else
    /// `CompletedWithWarnings`. From `Interrupted` or `NeedsAttention` only.
    pub fn finish(&mut self, at: Timestamp) -> Result<Outcome, ScanJobError> {
        if !matches!(self.state, ScanJobState::Interrupted | ScanJobState::NeedsAttention) {
            return Err(ScanJobError::NotWaiting(self.state));
        }
        let kept = self.infos.len();
        if kept == 0 {
            return Err(ScanJobError::NothingToFinish);
        }
        let outcome = match self.expected {
            Some(wanted) if count(kept) < wanted => {
                self.warnings.push(shortfall(kept, wanted));
                Outcome::CompletedWithWarnings
            }
            _ => Outcome::Completed,
        };
        self.finish_with(at, outcome);
        Ok(outcome)
    }

    fn finish_with(&mut self, at: Timestamp, outcome: Outcome) {
        self.running = None;
        self.state = ScanJobState::Terminal(outcome);
        self.times.finished = Some(at);
        self.advice = RetryAdvice::DoNotRetry;
        if outcome == Outcome::Completed {
            self.last_error = None;
        }
    }

    /// Hand the page bytes to the app, in order, and forget them here (the snapshot still lists them). Allowed
    /// at any time but while an attempt runs. A later [`Continuation::Append`] counts the pages taken as kept.
    pub fn take_pages(&mut self) -> Result<Vec<Page>, ScanJobError> {
        if self.running.is_some() {
            return Err(ScanJobError::AttemptRunning);
        }
        Ok(std::mem::take(&mut self.pages))
    }

    // ---- reading --------------------------------------------------------------------------------

    /// What may happen next, by [`craft_job::advise`] for a failure the job can repeat, and fixed for the rest.
    ///
    /// * `RetryNow` / `RetryAfter`: the app may call [`Self::begin_attempt`] without asking (the first attempt,
    ///   an `Auto` fallback to the glass, a busy scanner with nothing kept, within the retry budget).
    /// * `CheckDeviceFirst`: the outcome is unknown; never retried automatically.
    /// * `DoNotRetry`: an attempt is running, the job ended, or a person must decide.
    pub fn retry_advice(&self) -> RetryAdvice {
        match self.state {
            ScanJobState::Preparing => RetryAdvice::RetryNow,
            _ => self.advice,
        }
    }

    /// A cheap picture for a UI. Clones small fields; never blocks.
    pub fn snapshot(&self) -> ScanJobSnapshot {
        ScanJobSnapshot {
            id: self.id,
            title: self.config.title.clone(),
            device: self.settings.device.clone(),
            device_name: self.config.device_name.clone(),
            state: self.state,
            settled: !matches!(self.state, ScanJobState::Preparing | ScanJobState::Acquiring)
                && (self.state.is_terminal() || !self.advice.is_automatic_retry()),
            pages: self.infos.clone(),
            bytes: self.infos.iter().fold(0u64, |sum, p| sum.saturating_add(p.bytes)),
            expected_pages: self.expected,
            in_flight: self.in_flight,
            source_requested: self.forced_source.unwrap_or(self.settings.source),
            source_in_use: self.source_in_use,
            attempts_total: self.attempts_total,
            attempts: self.timeline.clone(),
            last_error: self.last_error.clone(),
            warnings: self.warnings.clone(),
            advice: self.retry_advice(),
            cancel_requested: self.cancel_requested,
            times: self.times,
            fingerprint: self.fingerprint.clone(),
            tag_support: self.config.tag_support,
        }
    }

    /// The job as the one record a UI renders for print and scan alike. The id is `<backend>:local-<n>`
    /// (the backend of `ScanSettings::device`, else `scan`); there is no device job to name.
    pub fn view(&self) -> JobView {
        let prefix = split_id(&self.settings.device).map(|(p, _)| p).filter(|p| BackendName::new(*p).is_ok()).unwrap_or("scan");
        let id = JobId::new(prefix, self.id.to_string());
        let advice = self.retry_advice();
        let state = match self.state {
            ScanJobState::Preparing => JobState::Pending,
            ScanJobState::Acquiring => JobState::Processing,
            ScanJobState::Interrupted if advice.is_automatic_retry() => JobState::Pending,
            ScanJobState::Interrupted | ScanJobState::NeedsAttention => JobState::NeedsAttention,
            ScanJobState::Terminal(Outcome::Completed) => JobState::Completed,
            ScanJobState::Terminal(Outcome::CompletedWithWarnings) => JobState::CompletedWithErrors,
            ScanJobState::Terminal(Outcome::Failed) => JobState::Aborted,
            ScanJobState::Terminal(Outcome::Cancelled | Outcome::CancelledByDevice) => JobState::Cancelled,
        };
        let mut view = JobView::new(id, JobKind::Scan, self.config.title.clone(), state)
            .with_progress(Progress::new(count(self.infos.len()), self.expected))
            .with_times(self.times)
            .with_attempts(self.attempts_total);
        let mut documents = JobDocuments::new();
        for page in &self.infos {
            documents.push(DocumentRef::new(format!("page-{}", page.number)).with_format(page.format).with_pages(1).with_bytes(page.bytes));
        }
        view = view.with_documents(documents);
        if let Some(failure) = &self.last_error {
            view = view.with_reason(failure.info.kind.as_str());
            let mut summary = ErrorSummary::new(failure.info.message.clone(), failure.info.kind.as_str()).with_retry_advice(advice);
            if let Some(native) = failure.info.native {
                summary = summary.with_native(native);
            }
            view = view.with_error(summary);
        }
        if self.state == ScanJobState::NeedsAttention {
            view = view.with_reason("outcome_unknown");
        }
        if self.state == ScanJobState::Terminal(Outcome::CancelledByDevice) {
            view = view.with_reason("cancelled_by_device");
        }
        if self.timeline.last().is_some_and(|r| r.fell_back) {
            view = view.with_reason(AUTO_FELL_BACK_REASON);
        }
        for warning in &self.warnings {
            view = view.with_reason(warning.clone());
        }
        if let Some(name) = &self.config.device_name {
            view = view.with_device_name(name.clone());
        }
        view
    }

    fn push_record(&mut self, record: AttemptRecord) {
        self.timeline.push(record);
        let over = self.timeline.len().saturating_sub(self.config.timeline_len.max(1));
        if over > 0 {
            self.timeline.drain(..over);
        }
    }

    fn update_record(&mut self, number: u32, change: impl FnOnce(&mut AttemptRecord)) {
        if let Some(record) = self.timeline.iter_mut().rev().find(|r| r.number == number) {
            change(record);
        }
    }
}

/// The [`JobView`] reason while the latest attempt is one where the backend used the glass instead of the feeder.
pub const AUTO_FELL_BACK_REASON: &str = "auto: used the glass because the feeder gave no pages";

fn count(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

fn shortfall(kept: usize, wanted: u32) -> String {
    format!("{kept} of {wanted} pages were scanned")
}

fn info_of(page: &Page, number: u32, attempt: u32, source: Option<Source>) -> PageInfo {
    PageInfo {
        number,
        attempt,
        source: source.filter(|s| *s != Source::Auto),
        format: page.format,
        width: page.width,
        height: page.height,
        dpi: page.dpi,
        bytes: page.bytes.len() as u64,
        length: page.length,
    }
}
