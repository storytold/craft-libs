//! The [`Backend`] trait and the neutral types a backend speaks.
//!
//! A backend is one way of getting a document onto paper: the system spooler (CUPS, the Windows
//! spooler) or, later, a network protocol (WS-Print, IPP). The [`Registry`](crate::Registry) holds
//! several at once, namespaces their printer ids (`"<backend>:<native id>"`), aggregates their
//! printer lists and routes details and jobs by that prefix.
//!
//! # Writing a backend
//!
//! * **Object safe, blocking, `Send + Sync`.** The registry stores `Arc<dyn Backend>` and calls
//!   from whatever thread the app uses (a worker, by convention). Every call is **bounded**: a
//!   network backend blocks on its own transport for at most its per-request timeouts, never for
//!   the length of a print. There is no async runtime in this crate.
//! * **Submit and watch are separate.** [`Backend::submit`] returns a [`Submitted`] as soon as the
//!   printer has *accepted* the document. What happens next is an optional capability: declare it
//!   with [`Backend::watch_support`] and answer [`Backend::job_status`] (one request, no waiting)
//!   and [`Backend::cancel_job`]. A backend that cannot say keeps the defaults: the status is
//!   [`JobState::Unknown`], "accepted, outcome unknown". Callers follow a job with a
//!   [`crate::JobWatch`]; [`crate::wait_for_job`] is the blocking convenience for a CLI.
//! * **Cancellation and progress travel in the request**: [`SubmitRequest::cancel`] is polled at
//!   the backend's own checkpoints (between sheets, between network round trips); progress is a
//!   callback of [`Progress`] events whose total is optional. Neither is required:
//!   the built-in spooler backends check the token only before they start.
//! * **Per-job options**: [`SubmitRequest::options`] ([`JobOptions`]) carries media, orientation,
//!   quality, resolution and colour as hints, and the page count the app knows. Apply what you can,
//!   say so in [`PrinterDetails::honoured`], and list what you dropped in [`Submitted::warnings`].
//! * **Ids**: a backend names printers with its own native ids and never adds its prefix; the
//!   registry does. A native id may contain colons; only the first colon separates the prefix.
//! * **Capabilities are lists and may be unknown.** [`PrinterDetails`] uses `Option`/empty lists
//!   for "not published". Options such as duplex, colour and scaling are hints a backend may
//!   ignore; [`PrinterDetails::honoured`] tells a UI which ones it applies.
//! * **Status is optional.** `None` is "cannot say", not "all clear". Spoolers report after the
//!   fact; a network protocol can report state up front.
//! * **Documents are bytes plus a declared format.** A backend declares what it accepts with
//!   [`Backend::accepted_formats`] (and, when printers differ, [`PrinterDetails::formats`] and
//!   [`Backend::formats_per_printer`]). When the document's format is not accepted the registry
//!   asks the app's [`DocumentConverter`](crate::DocumentConverter), if one is registered, for a
//!   format the printer takes, and refuses with [`ErrorKind::UnsupportedFormat`] when there is no
//!   way. A backend itself never converts: it sends what the registry hands it.
//! * **Consent.** A backend whose [`Backend::class`] is [`BackendClass::Network`] only receives
//!   requests that carry a [`UserConfirmed`]; the registry enforces it so a backend cannot forget.
//! * **Errors are neutral**: choose an [`ErrorKind`] and attach the native code with
//!   [`PrintError::with_native`]. Say where it failed ([`PrintError::with_phase`]) and whether a
//!   retry could print twice ([`PrintError::with_safety`]; [`RetrySafety::OutcomeUnknown`] after a
//!   request that may have arrived). Offer a way to reconcile if you can.
//! * **Registration**: `registry.register(Box::new(MyBackend::new()))`, once at start-up. The
//!   process-wide registry ([`crate::registry`]) already holds the platform's built-in backend.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use craft_job::{Fingerprint, JobId, JobState, NativeCode, Progress, RetrySafety, Tag, TagSupport};

use crate::job::{JobStatus, Submitted, WatchSupport};
use crate::options::{JobOptions, Resolution};
use crate::remote::{FingerprintExt as _, JobFilter, JobQueryCaps, Reconciled, RemoteJob, reconcile_jobs};
use crate::retry::Phase;
use crate::tag::TagSupportExt as _;
use crate::wake::WakeReceiver;
use crate::{Error, Job, PrinterOption};

/// What went wrong, independent of the backend. The kind and [`ErrorKind::is_retryable`] are the
/// contract callers branch on; the message is for people.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ErrorKind {
    /// No backend is registered, or the platform has no print stack.
    Unsupported,
    /// The printer id names a backend that is not registered (or has no prefix).
    UnknownBackend,
    /// The backend does not know the printer, or there is no default printer.
    NotFound,
    /// The request is malformed or asks for something the printer cannot do.
    InvalidRequest,
    /// The printer or backend does not accept the document's format.
    UnsupportedFormat,
    /// A network-class backend was asked to print without a [`UserConfirmed`].
    ConsentRequired,
    /// The caller cancelled.
    Cancelled,
    /// The spooler, device or network is not reachable right now.
    Unavailable,
    /// The device is busy; try again later.
    Busy,
    /// The device did not answer in time.
    Timeout,
    /// The printer or driver refused the job.
    Rejected,
    /// Reading or writing the document failed.
    Io,
    /// Anything else; the message says more.
    Failed,
}

impl ErrorKind {
    /// A stable lowercase name for logs, tool results and [`craft_job::ErrorSummary::kind_label`].
    pub const fn name(self) -> &'static str {
        match self {
            Self::Unsupported => "unsupported",
            Self::UnknownBackend => "unknown_backend",
            Self::NotFound => "not_found",
            Self::InvalidRequest => "invalid_request",
            Self::UnsupportedFormat => "unsupported_format",
            Self::ConsentRequired => "consent_required",
            Self::Cancelled => "cancelled",
            Self::Unavailable => "unavailable",
            Self::Busy => "busy",
            Self::Timeout => "timeout",
            Self::Rejected => "rejected",
            Self::Io => "io",
            Self::Failed => "failed",
        }
    }

    /// Whether the condition is transient: trying the same request again later can succeed.
    /// This says nothing about whether a retry is *safe*; see [`PrintError::retry_safety`].
    pub const fn is_retryable(self) -> bool {
        matches!(self, Self::Unavailable | Self::Busy | Self::Timeout)
    }

    /// The retry safety an error of this kind has unless its backend says otherwise.
    ///
    /// Refusals and conditions that arise before anything is accepted are [`RetrySafety::Safe`];
    /// a timeout or a failure of unknown cause may have followed a delivered request, so it is
    /// [`RetrySafety::OutcomeUnknown`]. A backend that knows better (a connect failure, a query)
    /// overrides it with [`PrintError::with_safety`].
    pub const fn default_retry_safety(self) -> RetrySafety {
        match self {
            Self::Timeout | Self::Failed => RetrySafety::OutcomeUnknown,
            _ => RetrySafety::Safe,
        }
    }
}

/// What an [`ErrorKind::NotFound`] is about. A wrong printer address or queue path and a job the
/// printer purged from its history look alike on the wire, and a follow-up must treat them
/// differently: only a missing **job** can mean "it most likely finished".
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum NotFoundSubject {
    /// The backend could not tell, or did not say. A [`crate::JobWatch`] keeps its long-standing
    /// reading of this (the job was forgotten); a backend that can tell should say which.
    #[default]
    Unknown,
    /// The printer (its address, queue path or endpoint) does not exist: a configuration error.
    Printer,
    /// The printer is there but no longer knows this job (finished jobs are forgotten).
    Job,
}

/// A print failure: a neutral [`ErrorKind`], a message and optionally the backend's native code.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrintError {
    kind: ErrorKind,
    message: String,
    native: Option<NativeCode>,
    subject: NotFoundSubject,
    phase: Option<Phase>,
    safety: Option<RetrySafety>,
    retry_after: Option<Duration>,
}

impl PrintError {
    pub fn new(kind: ErrorKind, message: impl Into<String>) -> Self {
        Self { kind, message: message.into(), native: None, subject: NotFoundSubject::Unknown, phase: None, safety: None, retry_after: None }
    }

    /// Where the failure happened ([`Phase`]).
    #[must_use]
    pub fn with_phase(mut self, phase: Phase) -> Self {
        self.phase = Some(phase);
        self
    }

    /// Whether repeating the request could duplicate it ([`RetrySafety`]).
    #[must_use]
    pub fn with_safety(mut self, safety: RetrySafety) -> Self {
        self.safety = Some(safety);
        self
    }

    /// How long the device (or the backend's sensible default for a busy device) says to wait.
    #[must_use]
    pub fn with_retry_after(mut self, wait: Duration) -> Self {
        self.retry_after = Some(wait);
        self
    }

    /// Attach the backend's own error code.
    #[must_use]
    pub fn with_native(mut self, native: NativeCode) -> Self {
        self.native = Some(native);
        self
    }

    /// Say what a [`ErrorKind::NotFound`] is about (see [`NotFoundSubject`]). Ignored by every other
    /// kind.
    #[must_use]
    pub fn with_subject(mut self, subject: NotFoundSubject) -> Self {
        self.subject = subject;
        self
    }

    /// What a not-found is about; [`NotFoundSubject::Unknown`] unless the backend said, and always
    /// `Unknown` for other kinds.
    pub fn subject(&self) -> NotFoundSubject {
        if self.kind == ErrorKind::NotFound { self.subject } else { NotFoundSubject::Unknown }
    }

    /// An unknown printer: [`ErrorKind::NotFound`] about the printer, not about a job.
    pub fn printer_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message).with_subject(NotFoundSubject::Printer)
    }

    /// A job the printer no longer knows: [`ErrorKind::NotFound`] about the job.
    pub fn job_not_found(message: impl Into<String>) -> Self {
        Self::new(ErrorKind::NotFound, message).with_subject(NotFoundSubject::Job)
    }

    pub fn kind(&self) -> ErrorKind {
        self.kind
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn native(&self) -> Option<NativeCode> {
        self.native
    }

    /// Whether the condition is transient (see [`ErrorKind::is_retryable`]). Not a licence to
    /// resend: check [`Self::retry_safety`].
    pub fn is_retryable(&self) -> bool {
        self.kind.is_retryable()
    }

    /// Where it failed, when the backend said.
    pub fn phase(&self) -> Option<Phase> {
        self.phase
    }

    /// Whether a retry could print twice. When the backend did not say, the kind's default
    /// ([`ErrorKind::default_retry_safety`]).
    pub fn retry_safety(&self) -> RetrySafety {
        self.safety.unwrap_or_else(|| self.kind.default_retry_safety())
    }

    /// The wait the device or backend suggests before another try, when there is one.
    pub fn retry_after(&self) -> Option<Duration> {
        self.retry_after
    }

    /// The request may have been delivered: reconcile (find out whether the job exists) before
    /// submitting again.
    pub fn outcome_unknown(&self) -> bool {
        self.retry_safety() == RetrySafety::OutcomeUnknown
    }
}

impl std::fmt::Display for PrintError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PrintError {}

/// The legacy spooler error keeps its message; it carries no more detail than that, so the kind
/// is [`ErrorKind::Failed`].
impl From<Error> for PrintError {
    fn from(error: Error) -> Self {
        Self::new(ErrorKind::Failed, error.to_string())
    }
}

/// The format of the bytes in a [`Document`].
///
/// The names are the formats a printer commonly lists; whether one printer takes one is decided
/// against that printer's own list ([`PrinterDetails::formats`], [`Self::negotiate`]). The crate
/// does not convert between them: that is a [`DocumentConverter`](crate::DocumentConverter) the app
/// registers.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum DocumentFormat {
    /// `application/pdf`, what the apps generate.
    Pdf,
    /// `image/pwg-raster`.
    PwgRaster,
    /// `application/postscript`.
    PostScript,
    /// `application/octet-stream`: the printer's own page description language.
    OctetStream,
    /// `image/jpeg`.
    Jpeg,
    /// `image/png`.
    Png,
    /// `image/tiff`.
    Tiff,
    /// `text/plain`.
    PlainText,
    /// `application/vnd.ms-xpsdocument`: XPS, the Windows print format.
    Xps,
    /// `application/oxps`: Open XPS (ECMA-388), the standardised XPS package.
    OpenXps,
}

impl DocumentFormat {
    /// A short lowercase name (`"pdf"`), as a [`craft_job::DocumentRef`] carries it.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::PwgRaster => "pwg-raster",
            Self::PostScript => "postscript",
            Self::OctetStream => "octet-stream",
            Self::Jpeg => "jpeg",
            Self::Png => "png",
            Self::Tiff => "tiff",
            Self::PlainText => "text",
            Self::Xps => "xps",
            Self::OpenXps => "oxps",
        }
    }

    /// Every format this crate names.
    pub const ALL: [DocumentFormat; 10] = [
        Self::Pdf,
        Self::PwgRaster,
        Self::PostScript,
        Self::OctetStream,
        Self::Jpeg,
        Self::Png,
        Self::Tiff,
        Self::PlainText,
        Self::Xps,
        Self::OpenXps,
    ];

    /// The order [`Self::negotiate_default`] and the registry's conversion prefer, best first:
    /// PDF (small, vector, what the apps make), PWG raster (the baseline every driverless printer
    /// takes, and the one the app controls pixel for pixel), then the XPS family, PostScript, the
    /// image formats and plain text. The printer's own language ([`Self::OctetStream`]) is last: it
    /// can be sent but nothing here can produce it.
    pub const DEFAULT_PREFERENCE: [DocumentFormat; 10] = [
        Self::Pdf,
        Self::PwgRaster,
        Self::OpenXps,
        Self::Xps,
        Self::PostScript,
        Self::Tiff,
        Self::Png,
        Self::Jpeg,
        Self::PlainText,
        Self::OctetStream,
    ];

    pub const fn mime(self) -> &'static str {
        match self {
            Self::Pdf => "application/pdf",
            Self::PwgRaster => "image/pwg-raster",
            Self::PostScript => "application/postscript",
            Self::OctetStream => "application/octet-stream",
            Self::Jpeg => "image/jpeg",
            Self::Png => "image/png",
            Self::Tiff => "image/tiff",
            Self::PlainText => "text/plain",
            Self::Xps => "application/vnd.ms-xpsdocument",
            Self::OpenXps => "application/oxps",
        }
    }

    /// The format a MIME type names. Parameters (`; charset=utf-8`), surrounding blanks and ASCII
    /// case are ignored; a type this crate has no name for is `None`. Besides the strings of
    /// [`Self::mime`] two common misspellings are recognised: `image/jpg` and `application/x-pdf`.
    pub fn from_mime(mime: &str) -> Option<Self> {
        let essence = mime.split(';').next().unwrap_or("").trim();
        if essence.eq_ignore_ascii_case("image/jpg") {
            return Some(Self::Jpeg);
        }
        if essence.eq_ignore_ascii_case("application/x-pdf") {
            return Some(Self::Pdf);
        }
        Self::ALL.into_iter().find(|f| f.mime().eq_ignore_ascii_case(essence))
    }

    /// The usual file extension, lowercase, without the dot. [`Self::OctetStream`] has no
    /// meaningful one and answers `bin`.
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Pdf => "pdf",
            Self::PwgRaster => "pwg",
            Self::PostScript => "ps",
            Self::OctetStream => "bin",
            Self::Jpeg => "jpg",
            Self::Png => "png",
            Self::Tiff => "tif",
            Self::PlainText => "txt",
            Self::Xps => "xps",
            Self::OpenXps => "oxps",
        }
    }

    /// The format a file extension (with or without the dot, any case) stands for. `bin` is not
    /// guessed to be anything; `None` for what is not known.
    pub fn from_extension(extension: &str) -> Option<Self> {
        match extension.trim().trim_start_matches('.').to_ascii_lowercase().as_str() {
            "pdf" => Some(Self::Pdf),
            "pwg" | "ras" => Some(Self::PwgRaster),
            "ps" | "eps" => Some(Self::PostScript),
            "jpg" | "jpeg" | "jpe" => Some(Self::Jpeg),
            "png" => Some(Self::Png),
            "tif" | "tiff" => Some(Self::Tiff),
            "txt" | "text" => Some(Self::PlainText),
            "xps" => Some(Self::Xps),
            "oxps" => Some(Self::OpenXps),
            _ => None,
        }
    }

    /// The format the first bytes of a file reveal: the magic numbers of PDF, JPEG, PNG, TIFF
    /// (classic and BigTIFF), PWG raster and PostScript, and plain text when the first 4 KiB are
    /// valid UTF-8 without control characters other than tab, line breaks and form feed. XPS is a
    /// ZIP package that looks like any other, so it is never sniffed: use [`Self::from_extension`].
    /// `None` for anything else (including an empty file).
    pub fn sniff(bytes: &[u8]) -> Option<Self> {
        let starts = |magic: &[u8]| bytes.starts_with(magic);
        if starts(b"%PDF-") {
            Some(Self::Pdf)
        } else if starts(&[0xFF, 0xD8, 0xFF]) {
            Some(Self::Jpeg)
        } else if starts(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A]) {
            Some(Self::Png)
        } else if starts(b"II*\0") || starts(b"MM\0*") || starts(b"II+\0") || starts(b"MM\0+") {
            Some(Self::Tiff)
        } else if starts(b"RaS2") {
            Some(Self::PwgRaster)
        } else if starts(b"%!PS") {
            Some(Self::PostScript)
        } else if is_plain_text(bytes) {
            Some(Self::PlainText)
        } else {
            None
        }
    }

    /// The format of a file: what its bytes reveal ([`Self::sniff`]), else what its name says
    /// ([`Self::from_extension`] of the part after the last dot). The bytes win because a wrongly
    /// named file would otherwise be sent as the wrong thing.
    pub fn detect(file_name: Option<&str>, bytes: &[u8]) -> Option<Self> {
        Self::sniff(bytes).or_else(|| file_name.and_then(|name| name.rsplit_once('.')).and_then(|(_, extension)| Self::from_extension(extension)))
    }

    /// The first format of `preference` that `accepted` lists. `preference` is best first and
    /// decides; `accepted` is usually [`PrinterDetails::formats`] of the printer (an empty list
    /// accepts nothing here: when a backend publishes no list, use its
    /// [`accepted_formats`](Backend::accepted_formats)).
    pub fn negotiate(preference: &[DocumentFormat], accepted: &[DocumentFormat]) -> Option<DocumentFormat> {
        preference.iter().copied().find(|format| accepted.contains(format))
    }

    /// [`Self::negotiate`] with [`Self::DEFAULT_PREFERENCE`].
    pub fn negotiate_default(accepted: &[DocumentFormat]) -> Option<DocumentFormat> {
        Self::negotiate(&Self::DEFAULT_PREFERENCE, accepted)
    }
}

/// Text the printer can take as `text/plain`: UTF-8 (a character cut at the end of the 4 KiB window
/// is fine) and no control characters beyond tab, line feed, carriage return and form feed.
fn is_plain_text(bytes: &[u8]) -> bool {
    let window = bytes.get(..4096).unwrap_or(bytes);
    let text = match std::str::from_utf8(window) {
        Ok(text) => text,
        // Only a character that the window cut in two is acceptable; `valid_up_to` is on a boundary.
        Err(error) if error.error_len().is_none() && window.len() < bytes.len() => {
            std::str::from_utf8(window.get(..error.valid_up_to()).unwrap_or(&[])).unwrap_or("")
        }
        Err(_) => return false,
    };
    !text.is_empty() && text.chars().all(|c| !c.is_control() || matches!(c, '\t' | '\n' | '\r' | '\x0c'))
}

/// Document bytes and their declared format. Constructing one does not look at the bytes (see
/// [`DocumentFormat::sniff`] and [`Document::detect`]) and nothing here converts them (see
/// [`DocumentConverter`](crate::DocumentConverter)).
#[derive(Clone, Copy, Debug)]
pub struct Document<'a> {
    pub format: DocumentFormat,
    pub bytes: &'a [u8],
}

impl<'a> Document<'a> {
    pub fn new(format: DocumentFormat, bytes: &'a [u8]) -> Self {
        Self { format, bytes }
    }

    pub fn pdf(bytes: &'a [u8]) -> Self {
        Self::new(DocumentFormat::Pdf, bytes)
    }

    /// The document with the format its bytes (or, failing that, `file_name`'s extension) reveal;
    /// `None` when neither does. See [`DocumentFormat::detect`].
    pub fn detect(file_name: Option<&str>, bytes: &'a [u8]) -> Option<Self> {
        DocumentFormat::detect(file_name, bytes).map(|format| Self::new(format, bytes))
    }
}

/// Whether a backend stays on this machine or talks to devices over a network.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum BackendClass {
    /// The system spooler: the user already chose the queue.
    #[default]
    Local,
    /// Connects to devices itself; every job needs a [`UserConfirmed`].
    Network,
}

/// Proof that a person agreed to this print. It carries no policy of its own: the registry only
/// requires one for [`BackendClass::Network`] backends, and an app decides how a person gets to
/// say yes. It can be obtained only through [`Self::attest`] or [`Self::interactive`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct UserConfirmed(());

impl UserConfirmed {
    /// The guard for agent and automation callers: pass the `user_confirmed` argument as given.
    /// Absent or false is [`ErrorKind::ConsentRequired`], with a message that says what to do.
    pub fn attest(user_confirmed: Option<bool>) -> Result<Self, PrintError> {
        if user_confirmed == Some(true) {
            Ok(Self(()))
        } else {
            Err(PrintError::new(
                ErrorKind::ConsentRequired,
                "printing to a network printer needs user_confirmed: true, set only after the person has agreed to this print",
            ))
        }
    }

    /// For interactive callers where the person has just pressed Print. Do not use it for anything
    /// an agent or script can reach.
    pub fn interactive() -> Self {
        Self(())
    }
}

/// A cooperative cancellation flag shared between the caller and a running job. Clones share the
/// flag.
#[derive(Clone, Debug, Default)]
pub struct CancelToken(Arc<AtomicBool>);

impl CancelToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }

    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// A printer in a list. In a [`Registry`](crate::Registry) result `id` is namespaced
/// (`"<backend>:<native id>"`); a backend returns its native id.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterRef {
    pub id: String,
    /// What a person sees.
    pub name: String,
    pub default: bool,
    /// Whether the printer takes two-sided jobs; `None` when unknown.
    pub two_sided: Option<bool>,
}

/// The job settings of [`Job`] and [`JobOptions`] a printer may honour. Everything else is a hint
/// it ignores. ([`JobOptions::colour`] is the `Colour` hint, as is `Job::grayscale`.)
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum JobHint {
    Copies,
    Collate,
    Duplex,
    Colour,
    Scaling,
    /// `Job::options`, the driver's own keywords.
    DriverOptions,
    /// [`JobOptions::media`].
    Media,
    /// [`JobOptions::orientation`].
    Orientation,
    /// [`JobOptions::quality`].
    Quality,
    /// [`JobOptions::resolution`].
    Resolution,
}

/// The state a printer reports before a job.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PrinterState {
    Idle,
    Processing,
    Stopped,
    Offline,
}

/// A printer's reported state. Absent from [`PrinterDetails`] when the backend cannot say.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PrinterStatus {
    pub state: PrinterState,
    /// Human-readable reasons the device gives (`"paper empty"`), as reported.
    pub reasons: Vec<String>,
}

/// What a printer can do. Every list may be empty or `None`, which means "not published", not
/// "none supported".
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct PrinterDetails {
    pub id: String,
    pub name: String,
    pub default: bool,
    /// The formats this printer takes; empty when the backend publishes only its own list.
    pub formats: Vec<DocumentFormat>,
    /// The pixel formats the printer lists for PWG raster (colour space and depth), as far as this crate can write
    /// them; empty when it did not publish a list (it is then taken to accept [`crate::pwg::RasterType::BASELINE`]).
    pub raster_types: Vec<crate::pwg::RasterType>,
    /// Driver job options (CUPS: the PPD's). Empty when none or unknown.
    pub options: Vec<PrinterOption>,
    pub two_sided: Option<bool>,
    /// Whether colour is available; `None` when unknown.
    pub colour: Option<bool>,
    /// Paper names the printer offers; empty when not published.
    pub paper: Vec<String>,
    pub max_copies: Option<u32>,
    /// The resolutions the printer lists, as it lists them; empty when not published. A
    /// [`DocumentConverter`](crate::DocumentConverter) renders at one of these (see
    /// [`choose_resolution`](crate::choose_resolution)).
    pub resolutions: Vec<Resolution>,
    /// Which [`Job`] settings this backend applies. `None`: not published, treat as unknown.
    pub honoured: Option<Vec<JobHint>>,
    pub status: Option<PrinterStatus>,
}

/// A message about an accepted job and the backend's own job id, if any. The older view of a
/// [`Submitted`] ([`Submitted::receipt`]); receipts are messages, not portable job ids.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Receipt {
    /// The backend's registry name.
    pub backend: &'static str,
    pub message: String,
    /// The backend's job id, when it has one.
    pub job_id: Option<String>,
}

/// The progress callback of a [`SubmitRequest`].
pub type ProgressCallback<'a> = &'a mut dyn FnMut(Progress);

/// One print request. [`Job::printer`] is the printer: a namespaced id when it goes to a
/// [`Registry`](crate::Registry), the native id when a backend receives it; `None` is the default.
/// [`SubmitRequest::options`] carries the per-job hints beside the spooler's own [`Job`] settings;
/// see [`JobOptions`].
pub struct SubmitRequest<'a> {
    pub document: Document<'a>,
    pub job: Job,
    /// Per-job hints (media, orientation, quality, resolution, colour, expected page count).
    pub options: JobOptions,
    pub consent: Option<UserConfirmed>,
    pub cancel: CancelToken,
    pub progress: Option<ProgressCallback<'a>>,
    /// The idempotency tag of the print this request belongs to ([`Tag`]); the same for every
    /// attempt. A backend carries it where its protocol allows ([`Backend::tag_support`]); the
    /// registry removes it for a backend that cannot.
    pub tag: Option<Tag>,
}

impl<'a> SubmitRequest<'a> {
    pub fn new(document: Document<'a>, job: Job) -> Self {
        Self { document, job, options: JobOptions::default(), consent: None, cancel: CancelToken::new(), progress: None, tag: None }
    }

    /// Put this idempotency tag on the job, where the backend can carry it. See [`Tag`].
    #[must_use]
    pub fn with_tag(mut self, tag: Tag) -> Self {
        self.tag = Some(tag);
        self
    }

    /// Per-job hints; see [`JobOptions`].
    #[must_use]
    pub fn with_options(mut self, options: JobOptions) -> Self {
        self.options = options;
        self
    }

    #[must_use]
    pub fn with_consent(mut self, consent: UserConfirmed) -> Self {
        self.consent = Some(consent);
        self
    }

    #[must_use]
    pub fn with_cancel(mut self, cancel: CancelToken) -> Self {
        self.cancel = cancel;
        self
    }

    #[must_use]
    pub fn with_progress(mut self, progress: ProgressCallback<'a>) -> Self {
        self.progress = Some(progress);
        self
    }
}

/// One way of printing. Ids passed in and returned are the backend's **native** ids; the
/// [`Registry`](crate::Registry) adds and strips the `"<name>:"` prefix. See the module docs.
pub trait Backend: Send + Sync {
    /// The registry name and id prefix: short, lowercase ASCII letters, digits, `-` and `_`
    /// (`"cups"`, `"spooler"`, `"wsd"`, `"ipp"`).
    fn name(&self) -> &'static str;

    /// Whether jobs need a [`UserConfirmed`]. Defaults to [`BackendClass::Local`].
    fn class(&self) -> BackendClass {
        BackendClass::Local
    }

    /// The formats [`Backend::submit`] takes. Defaults to PDF.
    ///
    /// For a backend whose printers differ ([`Self::formats_per_printer`]) this is the superset it
    /// can name and the printer's own list is [`PrinterDetails::formats`].
    fn accepted_formats(&self) -> &[DocumentFormat] {
        &[DocumentFormat::Pdf]
    }

    /// Whether which formats are accepted depends on the printer: when `true`,
    /// [`Self::accepted_formats`] is only what the backend can name and the registry asks
    /// [`Self::printer_details`] (its [`PrinterDetails::formats`]) before it decides whether a
    /// [`DocumentConverter`](crate::DocumentConverter) is needed. Defaults to `false`: the list is
    /// exact and no extra request is made.
    fn formats_per_printer(&self) -> bool {
        false
    }

    /// The printers the backend lists. Opens none of them.
    fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError>;

    /// What one printer (a native id) can do. [`ErrorKind::NotFound`] for an unknown printer.
    fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError>;

    /// Hand the document to the printer and return **as soon as it has accepted it**: the call is
    /// bounded by the backend's own per-request timeouts and never waits for the job to print.
    /// `request.job.printer` is a native id or `None` for the backend's default. The registry has
    /// already checked the format, the consent and that the token is not cancelled.
    ///
    /// Return [`Submitted::job`] when the backend has a handle for the job (`JobId::new(self.name(), native)`;
    /// the registry stamps the backend's name on it either way). An error that follows a request which may have arrived must say
    /// so: [`PrintError::with_safety`]`(`[`RetrySafety::OutcomeUnknown`]`)`.
    fn submit(&self, request: SubmitRequest<'_>) -> Result<Submitted, PrintError>;

    /// Whether and how this backend can report on a job after [`Backend::submit`]. Defaults to
    /// [`WatchSupport::None`]: "accepted, outcome unknown".
    fn watch_support(&self) -> WatchSupport {
        WatchSupport::None
    }

    /// How often a caller should ask [`Backend::job_status`]; `None` when it should not.
    /// Defaults from [`Backend::watch_support`]: 2 s for polling, 15 s as a safety net for events.
    fn recommended_poll_interval(&self) -> Option<Duration> {
        match self.watch_support() {
            WatchSupport::None => None,
            WatchSupport::Poll => Some(Duration::from_secs(2)),
            WatchSupport::Events => Some(Duration::from_secs(15)),
        }
    }

    /// The job's status: **one bounded request**, no waiting. Its errors are idempotent (repeating
    /// is harmless). A job the device no longer knows is [`ErrorKind::NotFound`] (finished jobs are
    /// forgotten; the message says so). The default answers [`JobState::Unknown`]: a backend that
    /// cannot report keeps it, and callers do not poll it ([`WatchSupport::None`]).
    fn job_status(&self, job: &JobId) -> Result<JobStatus, PrintError> {
        let _ = job;
        Ok(JobStatus::new(JobState::Unknown))
    }

    /// A push channel for this job, when the backend has one: a [`WakeReceiver`] that is woken when
    /// the device says the job (or its printer) changed. A wake carries no data and means only "ask
    /// [`Backend::job_status`] now"; the status itself always comes from `job_status`, so a lost,
    /// duplicated or forged wake cannot produce a wrong result. The receiver reports whether the
    /// backend currently trusts the channel ([`WakeReceiver::is_healthy`]); a [`crate::JobWatch`]
    /// only relaxes its polling while it does. Defaults to `None`: poll. Getting the receiver is
    /// cheap and sends nothing; dropping it tells the backend the watch ended.
    fn wake_source(&self, job: &JobId) -> Option<WakeReceiver> {
        let _ = job;
        None
    }

    /// How long the device keeps a job open while it waits for the document of a request whose
    /// reply was lost, plus a small margin: after this the device has given up on the half-made
    /// job on its own (WS-Print aborts it with `DocumentTimeoutError`, IPP printers wait their
    /// `multiple-operation-time-out`, 60 to 240 seconds, and then end it as
    /// `submission-interrupted`). [`crate::PrintJob`] waits this long after a send whose outcome
    /// is unknown before it advises resending. `None` (the default): the job uses
    /// [`crate::Staleness::reconcile_after`].
    fn open_job_timeout(&self) -> Option<Duration> {
        None
    }

    /// Ask the device to cancel an accepted job. Cancelling a job that already ended is not an
    /// error. The default is [`ErrorKind::Unsupported`].
    fn cancel_job(&self, job: &JobId) -> Result<(), PrintError> {
        let _ = job;
        Err(PrintError::new(ErrorKind::Unsupported, format!("the {} backend cannot cancel a job once it is accepted", self.name())))
    }

    /// Whether and where this backend carries a [`SubmitRequest::tag`], and whether
    /// [`Backend::list_jobs`] gives it back. Defaults to [`TagSupport::None`]. A backend that
    /// supports a placement applies the tag itself (a name suffix with [`crate::TagExt::apply_to`], within
    /// its own length limit) and exactly once.
    fn tag_support(&self) -> TagSupport {
        TagSupport::None
    }

    /// What [`Backend::list_jobs`] can fill in. Defaults to [`JobQueryCaps::NONE`].
    fn job_query_caps(&self) -> JobQueryCaps {
        JobQueryCaps::NONE
    }

    /// The jobs of one printer (a native id): its active jobs and, when the source keeps them,
    /// finished ones ([`JobQueryCaps::history`]). **One bounded, idempotent read.** Foreign jobs
    /// are listed too; [`RemoteJob::ours`] is set from [`JobFilter::tag`] (use
    /// [`crate::filter_jobs`]). The default is [`ErrorKind::Unsupported`]: a spooler that cannot
    /// list keeps it, and says so in [`Backend::job_query_caps`].
    fn list_jobs(&self, printer_native: &str, filter: &JobFilter) -> Result<Vec<RemoteJob>, PrintError> {
        let _ = (printer_native, filter);
        Err(PrintError::new(ErrorKind::Unsupported, format!("the {} backend cannot list jobs", self.name())).with_safety(RetrySafety::Safe))
    }

    /// After an attempt whose outcome is unknown: find the job it may have created. The default
    /// lists jobs ([`Backend::list_jobs`]) and decides with [`reconcile_jobs`]: by tag when the
    /// backend can match it, else by title, user and time, which may be
    /// [`Reconciled::Ambiguous`]. A backend overrides it to fill in what only it knows (its own
    /// user name).
    fn find_job(&self, printer_native: &str, fingerprint: &Fingerprint) -> Result<Reconciled, PrintError> {
        let support = self.tag_support();
        let jobs = self.list_jobs(printer_native, &fingerprint.filter(support.matchable()))?;
        Ok(reconcile_jobs(jobs, fingerprint, support))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_format_round_trips_through_its_mime_type_and_extension() {
        let mut mimes = std::collections::HashSet::new();
        for format in DocumentFormat::ALL {
            assert!(mimes.insert(format.mime()), "{} twice", format.mime());
            assert_eq!(DocumentFormat::from_mime(format.mime()), Some(format));
            assert_eq!(DocumentFormat::from_mime(&format!(" {}; q=1", format.mime().to_ascii_uppercase())), Some(format));
            if format != DocumentFormat::OctetStream {
                assert_eq!(DocumentFormat::from_extension(format.extension()), Some(format));
                assert_eq!(DocumentFormat::from_extension(&format!(".{}", format.extension().to_ascii_uppercase())), Some(format));
            }
        }
        assert_eq!(DocumentFormat::DEFAULT_PREFERENCE.len(), DocumentFormat::ALL.len());
        assert!(DocumentFormat::ALL.iter().all(|f| DocumentFormat::DEFAULT_PREFERENCE.contains(f)));
        assert_eq!(DocumentFormat::Jpeg.mime(), "image/jpeg");
        assert_eq!(DocumentFormat::Xps.mime(), "application/vnd.ms-xpsdocument");
        assert_eq!(DocumentFormat::OpenXps.mime(), "application/oxps");
        assert_eq!(DocumentFormat::from_mime("image/jpg"), Some(DocumentFormat::Jpeg));
        assert_eq!(DocumentFormat::from_mime("application/x-pdf"), Some(DocumentFormat::Pdf));
        assert_eq!(DocumentFormat::from_mime("application/vnd.hp-pcl"), None);
        assert_eq!(DocumentFormat::from_extension("jpeg"), Some(DocumentFormat::Jpeg));
        assert_eq!(DocumentFormat::from_extension("bin"), None);
    }

    #[test]
    fn the_first_bytes_reveal_the_format() {
        let sniff = DocumentFormat::sniff;
        assert_eq!(sniff(b"%PDF-1.7\n"), Some(DocumentFormat::Pdf));
        assert_eq!(sniff(&[0xFF, 0xD8, 0xFF, 0xE0, 0, 0x10]), Some(DocumentFormat::Jpeg));
        assert_eq!(sniff(&[0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A, 0]), Some(DocumentFormat::Png));
        assert_eq!(sniff(b"II*\0\x08\0\0\0"), Some(DocumentFormat::Tiff));
        assert_eq!(sniff(b"MM\0*\0\0\0\x08"), Some(DocumentFormat::Tiff));
        assert_eq!(sniff(b"II+\0"), Some(DocumentFormat::Tiff));
        assert_eq!(sniff(b"RaS2\0\0"), Some(DocumentFormat::PwgRaster));
        assert_eq!(sniff(b"%!PS-Adobe-3.0\n"), Some(DocumentFormat::PostScript));
        assert_eq!(sniff("Gr\u{fc}\u{df}e,\n\ttext\r\n".as_bytes()), Some(DocumentFormat::PlainText));
        assert_eq!(sniff(b"PK\x03\x04"), None, "a ZIP package could be XPS or anything else");
        assert_eq!(sniff(b""), None);
        assert_eq!(sniff(b"\0\x01\x02binary"), None);
        assert_eq!(sniff(&[0xC3]), None, "an invalid UTF-8 byte at the very end is not text");
        // A character cut by the 4 KiB window is still text; one that is broken inside is not.
        let mut cut = "a".repeat(4095).into_bytes();
        cut.extend_from_slice("\u{e9}".as_bytes());
        assert_eq!(sniff(&cut), Some(DocumentFormat::PlainText));
        let mut broken = "a".repeat(100).into_bytes();
        broken.push(0xFF);
        broken.extend_from_slice(b"more");
        assert_eq!(sniff(&broken), None);
    }

    #[test]
    fn detection_prefers_the_bytes_and_falls_back_to_the_name() {
        assert_eq!(DocumentFormat::detect(Some("scan.png"), b"%PDF-1.4"), Some(DocumentFormat::Pdf));
        assert_eq!(DocumentFormat::detect(Some("page.XPS"), b"PK\x03\x04"), Some(DocumentFormat::Xps));
        assert_eq!(DocumentFormat::detect(Some("noextension"), b"PK\x03\x04"), None);
        assert_eq!(DocumentFormat::detect(None, b"\0\0"), None);
        let document = Document::detect(Some("a.oxps"), b"PK\x03\x04").unwrap();
        assert_eq!((document.format, document.bytes.len()), (DocumentFormat::OpenXps, 4));
        assert!(Document::detect(None, b"\0").is_none());
    }

    #[test]
    fn negotiation_follows_the_preference_and_the_printers_list() {
        use DocumentFormat::*;
        assert_eq!(DocumentFormat::negotiate(&[Jpeg, Pdf], &[Pdf, Jpeg]), Some(Jpeg));
        assert_eq!(DocumentFormat::negotiate(&[Pdf, Jpeg], &[Jpeg, Png]), Some(Jpeg));
        assert_eq!(DocumentFormat::negotiate(&[Pdf], &[Jpeg]), None);
        assert_eq!(DocumentFormat::negotiate(&[Pdf], &[]), None);
        assert_eq!(DocumentFormat::negotiate_default(&[Png, PwgRaster, Jpeg]), Some(PwgRaster));
        assert_eq!(DocumentFormat::negotiate_default(&[Jpeg, Tiff, Xps]), Some(Xps));
        assert_eq!(DocumentFormat::negotiate_default(&[OctetStream, PlainText]), Some(PlainText));
    }

    #[test]
    fn progress_totals_are_optional_and_a_reported_one_wins() {
        assert_eq!(Progress::done_only(2).fraction(), None);
        assert_eq!(Progress::of(1, 4).fraction(), Some(0.25));
        assert_eq!(Progress::of(9, 4).fraction(), Some(1.0));
        assert_eq!(Progress::of(1, 0).fraction(), None);
        assert_eq!(Progress::done_only(2).or_expected(Some(5)), Progress::of(2, 5));
        assert_eq!(Progress::of(2, 3).or_expected(Some(5)), Progress::of(2, 3));
        assert_eq!(Progress::of(2, 0).or_expected(Some(5)), Progress::of(2, 5), "a zero total is no total");
        assert_eq!(Progress::done_only(2).or_expected(Some(0)), Progress::done_only(2));
        assert_eq!(Progress::new(1, None), Progress::done_only(1));
    }
}
