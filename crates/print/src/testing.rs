//! Test doubles for apps that print through this crate. Cargo feature `testing`; it has no effect
//! on default builds and adds no dependency. Never enable it in a shipped binary.
//!
//! [`FakeBackend`] is a configurable [`Backend`] for a [`Registry`](crate::Registry) of your own:
//! it lists the printers you give it, answers details, takes documents, reports a scripted job
//! status, and records every request so the test can assert on what the app sent (the
//! [`Submission`]s, the cancels, how often the status was asked). Clones share their state, so one
//! clone goes into the registry and the other stays with the test:
//!
//! ```
//! use craft_print::testing::FakeBackend;
//! use craft_print::{Document, Job, JobOptions, JobState, JobStatus, Registry, SubmitRequest, WatchSupport};
//!
//! let fake = FakeBackend::new("fake")
//!     .with_printer("p1", true)
//!     .with_watch(WatchSupport::Poll)
//!     .with_status_script(vec![Ok(JobStatus::new(JobState::Processing)), Ok(JobStatus::new(JobState::Completed))]);
//! let registry = Registry::new();
//! registry.register(Box::new(fake.clone()))?;
//!
//! let request = SubmitRequest::new(Document::pdf(b"%PDF-1.7"), Job::default()).with_options(JobOptions::new().with_expected_pages(2));
//! let submitted = registry.submit_job(request)?;
//! let job = submitted.job.expect("the fake hands out a job id");
//! assert_eq!(registry.job_status(&job)?.state, JobState::Processing);
//! assert_eq!(fake.submissions()[0].options.expected_pages, Some(2));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```
//!
//! For the WS-Print backend see `craft_print::wsd::testing` (an in-memory WS-Print printer; needs
//! the `wsd` feature too). There is no in-memory IPP printer: the fake IPP printer is fakeprint.

use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime};

use craft_job::{JobId, JobState, Progress, Tag, TagPlacement, TagSupport};

use crate::backend::{
    Backend, BackendClass, DocumentFormat, ErrorKind, JobHint, PrintError, PrinterDetails, PrinterRef, PrinterStatus, SubmitRequest,
};
use crate::job::{JobStatus, Submitted, WatchSupport};
use crate::options::JobOptions;
use crate::remote::{JobFilter, JobQueryCaps, RemoteJob, filter_jobs};
use crate::tag::TagExt as _;
use crate::{Job, PrinterOption};

/// What a [`FakeBackend`] was asked to print.
#[derive(Clone, Debug)]
pub struct Submission {
    /// The native printer id the backend received (`None`: the default printer).
    pub printer: Option<String>,
    pub format: DocumentFormat,
    pub bytes: Vec<u8>,
    /// The whole [`Job`] as the backend saw it (the registry has already removed the prefix from
    /// `printer`).
    pub job: Job,
    pub options: JobOptions,
    /// Whether the request carried a [`crate::UserConfirmed`].
    pub consented: bool,
    /// The idempotency tag the request carried (the registry removes it when the fake's
    /// [`TagSupport`] is none).
    pub tag: Option<Tag>,
    /// The document name the tag was put on, with [`TagPlacement::DocumentName`].
    pub document_name: Option<String>,
}

/// What a [`FakeBackend`] does with its next `submit`, scripted with
/// [`FakeBackend::with_submit_script`] (one step per call; when the script is used up, `submit`
/// accepts).
#[derive(Clone, Debug)]
pub enum SubmitStep {
    /// Accept the document.
    Accept,
    /// Fail without creating a job: the printer did not act.
    Fail(PrintError),
    /// The printer **created the job**, but the reply was lost: the call fails with this error
    /// (give it [`crate::RetrySafety::OutcomeUnknown`]) and the job exists in the fake's queue.
    LoseReply(PrintError),
}

type JobHandle = dyn Fn(Option<&str>, u32) -> Option<String> + Send + Sync;

#[derive(Default)]
struct Recorded {
    submissions: Vec<Submission>,
    status_calls: u32,
    detail_calls: u32,
    cancels: Vec<String>,
    script: Vec<Result<JobStatus, PrintError>>,
    submit_script: Vec<SubmitStep>,
    jobs: Vec<RemoteJob>,
    wakers: Vec<crate::WakeSender>,
    push_healthy: bool,
}

#[derive(Clone)]
struct Config {
    name: &'static str,
    class: BackendClass,
    formats: Vec<DocumentFormat>,
    formats_per_printer: bool,
    printers: Vec<PrinterRef>,
    details: PrinterDetails,
    list_error: Option<PrintError>,
    details_error: Option<PrintError>,
    submit_error: Option<PrintError>,
    message: String,
    job_handle: Arc<JobHandle>,
    progress: Vec<Progress>,
    watch: WatchSupport,
    poll_interval: Option<Duration>,
    can_cancel: bool,
    submitted_status: Option<JobStatus>,
    tag_support: TagSupport,
    query_caps: JobQueryCaps,
    user: String,
    created_at: Arc<dyn Fn() -> SystemTime + Send + Sync>,
    title_max: Option<usize>,
    open_job_timeout: Option<Duration>,
    push: bool,
}

/// A configurable [`Backend`] for tests; see the [module documentation](self). Build it with
/// [`Self::new`] and the `with_*` methods, register a clone, keep a clone to inspect.
///
/// Defaults: a local backend taking PDF, no printers, `submit` accepts and hands out the job handle
/// `<printer>#<n>` (n counts from 1), no progress, [`WatchSupport::None`], and a status that is
/// [`JobState::Unknown`] unless a script is set.
#[derive(Clone)]
pub struct FakeBackend {
    config: Arc<Config>,
    recorded: Arc<Mutex<Recorded>>,
}

impl std::fmt::Debug for FakeBackend {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FakeBackend").field("name", &self.config.name).finish_non_exhaustive()
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl FakeBackend {
    /// A fake registered as `name` (the registry's id prefix: lowercase letters, digits, `-`, `_`).
    pub fn new(name: &'static str) -> Self {
        let config = Config {
            name,
            class: BackendClass::Local,
            formats: vec![DocumentFormat::Pdf],
            formats_per_printer: false,
            printers: Vec::new(),
            details: PrinterDetails::default(),
            list_error: None,
            details_error: None,
            submit_error: None,
            message: "accepted".into(),
            job_handle: Arc::new(|printer, n| Some(format!("{}#{n}", printer.unwrap_or_default()))),
            progress: Vec::new(),
            watch: WatchSupport::None,
            poll_interval: None,
            can_cancel: true,
            submitted_status: None,
            tag_support: TagSupport::None,
            query_caps: JobQueryCaps::NONE,
            user: "fake-user".into(),
            created_at: Arc::new(SystemTime::now),
            title_max: None,
            open_job_timeout: None,
            push: false,
        };
        Self { config: Arc::new(config), recorded: Arc::default() }
    }

    /// A fake that behaves like a **CUPS-style spooler**: local class, a job handle at once, no
    /// status after acceptance, **no tag support and no job listing**. Reconciling against it is
    /// impossible, which is the point of using it.
    pub fn cups_like(name: &'static str) -> Self {
        Self::new(name)
    }

    /// A fake that behaves like a **network IPP printer**: network class (every request needs a
    /// [`crate::UserConfirmed`]), pollable status, a job listing that carries the creation time,
    /// the user and finished jobs, and an idempotency tag stored in a **native job attribute**
    /// (the title is left alone and the listing returns the tag).
    pub fn ipp_like(name: &'static str) -> Self {
        Self::new(name)
            .with_class(BackendClass::Network)
            .with_watch(WatchSupport::Poll)
            .with_tag_support(TagSupport::Carried(TagPlacement::NativeAttribute))
            .with_query_caps(JobQueryCaps::ALL)
            .with_open_job_timeout(Duration::from_secs(240))
    }

    fn configured(mut self, change: impl FnOnce(&mut Config)) -> Self {
        // Configuration is set before the fake is cloned; if it was cloned, the clones keep the old one.
        change(Arc::make_mut(&mut self.config));
        self
    }

    /// What [`Backend::open_job_timeout`] answers (default `None`; [`Self::ipp_like`] uses 240
    /// seconds, the top of the range an IPP printer waits for the rest of a request).
    #[must_use]
    pub fn with_open_job_timeout(self, timeout: Duration) -> Self {
        self.configured(|c| c.open_job_timeout = Some(timeout))
    }

    /// [`BackendClass::Network`] fakes need a [`crate::UserConfirmed`] on every request.
    #[must_use]
    pub fn with_class(self, class: BackendClass) -> Self {
        self.configured(|c| c.class = class)
    }

    /// The formats [`Backend::accepted_formats`] answers (default PDF).
    #[must_use]
    pub fn with_formats(self, formats: Vec<DocumentFormat>) -> Self {
        self.configured(|c| c.formats = formats)
    }

    /// Whether [`Backend::formats_per_printer`] is `true` (default `false`): the registry then reads
    /// the details' `formats` before it decides on a conversion, like it does for a network backend.
    #[must_use]
    pub fn with_formats_per_printer(self, per_printer: bool) -> Self {
        self.configured(|c| c.formats_per_printer = per_printer)
    }

    /// Shorthand for the `formats` (the printer's own list) of the details.
    #[must_use]
    pub fn with_printer_formats(self, formats: Vec<DocumentFormat>) -> Self {
        self.configured(|c| c.details.formats = formats)
    }

    /// Shorthand for the `resolutions` of the details.
    #[must_use]
    pub fn with_resolutions(self, resolutions: Vec<crate::Resolution>) -> Self {
        self.configured(|c| c.details.resolutions = resolutions)
    }

    /// Add a printer to the listing (a native id; the name is `Printer <id>`).
    #[must_use]
    pub fn with_printer(self, id: &str, default: bool) -> Self {
        self.with_printer_ref(PrinterRef { id: id.into(), name: format!("Printer {id}"), default, two_sided: None })
    }

    /// Add a fully specified printer to the listing.
    #[must_use]
    pub fn with_printer_ref(self, printer: PrinterRef) -> Self {
        self.configured(|c| c.printers.push(printer))
    }

    /// The details every listed printer answers (its `id` and `name` are filled in per printer).
    /// Without this the details are empty ("not published") apart from the id and name.
    #[must_use]
    pub fn with_details(self, details: PrinterDetails) -> Self {
        self.configured(|c| c.details = details)
    }

    /// Shorthand for the `honoured` list of the details.
    #[must_use]
    pub fn with_honoured(self, honoured: Vec<JobHint>) -> Self {
        self.configured(|c| c.details.honoured = Some(honoured))
    }

    /// Shorthand for the driver options of the details.
    #[must_use]
    pub fn with_printer_options(self, options: Vec<PrinterOption>) -> Self {
        self.configured(|c| c.details.options = options)
    }

    /// Shorthand for the printer state of the details.
    #[must_use]
    pub fn with_printer_status(self, status: PrinterStatus) -> Self {
        self.configured(|c| c.details.status = Some(status))
    }

    /// `list_printers` fails with this error.
    #[must_use]
    pub fn failing_list(self, error: PrintError) -> Self {
        self.configured(|c| c.list_error = Some(error))
    }

    /// `printer_details` fails with this error (for every printer).
    #[must_use]
    pub fn failing_details(self, error: PrintError) -> Self {
        self.configured(|c| c.details_error = Some(error))
    }

    /// `submit` fails with this error (after recording the request).
    #[must_use]
    pub fn failing_submit(self, error: PrintError) -> Self {
        self.configured(|c| c.submit_error = Some(error))
    }

    /// The sentence `submit` answers with (default `accepted`).
    #[must_use]
    pub fn with_message(self, message: &str) -> Self {
        self.configured(|c| c.message = message.to_owned())
    }

    /// How `submit` names the job it accepted, from the native printer id and a counter that starts
    /// at 1. Returning `None` hands out no job handle (a spooler that cannot say). The default is
    /// `<printer>#<n>`.
    #[must_use]
    pub fn with_job_handle(self, handle: impl Fn(Option<&str>, u32) -> Option<String> + Send + Sync + 'static) -> Self {
        self.configured(|c| c.job_handle = Arc::new(handle))
    }

    /// Progress events `submit` reports to the request's callback, in order.
    #[must_use]
    pub fn with_progress_events(self, events: Vec<Progress>) -> Self {
        self.configured(|c| c.progress = events)
    }

    /// What the backend says it can do after acceptance. [`WatchSupport::Poll`] and `Events`
    /// default the recommended interval to two and fifteen seconds unless
    /// [`Self::with_poll_interval`] sets one.
    #[must_use]
    pub fn with_watch(self, watch: WatchSupport) -> Self {
        self.configured(|c| c.watch = watch)
    }

    /// Give the fake a push channel ([`Backend::wake_source`]): every watch gets a receiver, woken by
    /// [`Self::wake_watches`] and trusted according to [`Self::set_push_healthy`] (initially
    /// healthy). Models a backend whose device can say "something changed".
    #[must_use]
    pub fn with_push(self) -> Self {
        lock(&self.recorded).push_healthy = true;
        self.configured(|c| c.push = true)
    }

    /// Wake every receiver handed out by [`Backend::wake_source`]; returns how many still exist.
    pub fn wake_watches(&self) -> usize {
        lock(&self.recorded).wakers.iter().filter(|w| w.wake()).count()
    }

    /// Say whether the push channel is currently trusted, for the receivers handed out so far and
    /// the ones handed out later.
    pub fn set_push_healthy(&self, healthy: bool) {
        let mut recorded = lock(&self.recorded);
        recorded.push_healthy = healthy;
        recorded.wakers.iter().for_each(|w| w.set_healthy(healthy));
    }

    /// How many push receivers are still held by a watch.
    pub fn push_watchers(&self) -> usize {
        lock(&self.recorded).wakers.iter().filter(|w| w.is_connected()).count()
    }

    #[must_use]
    pub fn with_poll_interval(self, interval: Duration) -> Self {
        self.configured(|c| c.poll_interval = Some(interval))
    }

    /// Whether `cancel_job` works (default) or answers [`ErrorKind::Unsupported`].
    #[must_use]
    pub fn with_cancel_support(self, can_cancel: bool) -> Self {
        self.configured(|c| c.can_cancel = can_cancel)
    }

    /// The status `submit` returns with the accepted job (a printer that reports up front).
    #[must_use]
    pub fn with_submitted_status(self, status: JobStatus) -> Self {
        self.configured(|c| c.submitted_status = Some(status))
    }

    /// The answers of successive `job_status` calls; the last one repeats. Empty (the default)
    /// answers [`JobState::Unknown`] like a backend that cannot report.
    #[must_use]
    pub fn with_status_script(self, script: Vec<Result<JobStatus, PrintError>>) -> Self {
        lock(&self.recorded).script = script;
        self
    }

    /// Where the fake carries a tag and whether its listing returns it (default: none).
    #[must_use]
    pub fn with_tag_support(self, support: TagSupport) -> Self {
        self.configured(|c| c.tag_support = support)
    }

    /// What the fake's `list_jobs` fills in (default: nothing, and `list_jobs` is unsupported).
    #[must_use]
    pub fn with_query_caps(self, caps: JobQueryCaps) -> Self {
        self.configured(|c| c.query_caps = caps)
    }

    /// The user the fake lists its jobs under (default `fake-user`).
    #[must_use]
    pub fn with_user(self, user: &str) -> Self {
        self.configured(|c| c.user = user.to_owned())
    }

    /// The creation time the fake gives a job it accepts (default: now). Only listed when the caps
    /// say `created_time`.
    #[must_use]
    pub fn with_created_at(self, created_at: impl Fn() -> SystemTime + Send + Sync + 'static) -> Self {
        self.configured(|c| c.created_at = Arc::new(created_at))
    }

    /// The longest job name the fake keeps, in characters (default: unlimited).
    #[must_use]
    pub fn with_title_limit(self, max_chars: usize) -> Self {
        self.configured(|c| c.title_max = Some(max_chars))
    }

    /// What successive `submit` calls do; see [`SubmitStep`]. Used up, `submit` accepts.
    #[must_use]
    pub fn with_submit_script(self, script: Vec<SubmitStep>) -> Self {
        lock(&self.recorded).submit_script = script;
        self
    }

    /// Put a job in the fake's queue that this app did not send (a foreign job).
    pub fn seed_job(&self, job: RemoteJob) {
        lock(&self.recorded).jobs.push(job);
    }

    /// Replace the answers of `job_status` while the fake is in use (see
    /// [`Self::with_status_script`]); every clone sees the change.
    pub fn set_status_script(&self, script: Vec<Result<JobStatus, PrintError>>) {
        lock(&self.recorded).script = script;
    }

    /// Set the reasons of a job in the fake's queue (by native handle); `false` if there is none.
    pub fn set_remote_reasons(&self, native: &str, reasons: Vec<String>) -> bool {
        let mut recorded = lock(&self.recorded);
        match recorded.jobs.iter_mut().find(|job| job.id.native() == native) {
            Some(job) => {
                job.reasons = reasons;
                true
            }
            None => false,
        }
    }

    /// Change the state of a job in the fake's queue (by native handle); `false` if there is none.
    pub fn set_remote_state(&self, native: &str, state: JobState) -> bool {
        let mut recorded = lock(&self.recorded);
        match recorded.jobs.iter_mut().find(|job| job.id.native() == native) {
            Some(job) => {
                job.state = state;
                true
            }
            None => false,
        }
    }

    /// The fake's queue: every job it accepted (and every seeded one), oldest first.
    pub fn remote_jobs(&self) -> Vec<RemoteJob> {
        lock(&self.recorded).jobs.clone()
    }

    /// Everything `submit` received, oldest first.
    pub fn submissions(&self) -> Vec<Submission> {
        lock(&self.recorded).submissions.clone()
    }

    /// How many times `printer_details` was asked (the registry asks to choose a conversion).
    pub fn detail_calls(&self) -> u32 {
        lock(&self.recorded).detail_calls
    }

    /// How many times `job_status` was asked.
    pub fn status_calls(&self) -> u32 {
        lock(&self.recorded).status_calls
    }

    /// The native handles of the jobs `cancel_job` was asked to cancel.
    pub fn cancels(&self) -> Vec<String> {
        lock(&self.recorded).cancels.clone()
    }
}

impl Backend for FakeBackend {
    fn name(&self) -> &'static str {
        self.config.name
    }

    fn class(&self) -> BackendClass {
        self.config.class
    }

    fn accepted_formats(&self) -> &[DocumentFormat] {
        &self.config.formats
    }

    fn formats_per_printer(&self) -> bool {
        self.config.formats_per_printer
    }

    fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError> {
        match &self.config.list_error {
            Some(error) => Err(error.clone()),
            None => Ok(self.config.printers.clone()),
        }
    }

    fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError> {
        {
            let mut recorded = lock(&self.recorded);
            recorded.detail_calls = recorded.detail_calls.saturating_add(1);
        }
        if let Some(error) = &self.config.details_error {
            return Err(error.clone());
        }
        let printer = self
            .config
            .printers
            .iter()
            .find(|p| p.id == native_id)
            .ok_or_else(|| PrintError::printer_not_found(format!("the fake backend has no printer {native_id}")))?;
        Ok(PrinterDetails { id: printer.id.clone(), name: printer.name.clone(), default: printer.default, ..self.config.details.clone() })
    }

    fn submit(&self, mut request: SubmitRequest<'_>) -> Result<Submitted, PrintError> {
        // The backend carries the tag where its support says, exactly once.
        let placement = self.config.tag_support.placement();
        let tag = request.tag;
        let mut document_name = None;
        if let Some(tag) = &tag {
            match placement {
                TagPlacement::JobNameSuffix => request.job.title = tag.apply_to(&request.job.title, self.config.title_max),
                TagPlacement::DocumentName => document_name = Some(tag.apply_to(&request.job.title, self.config.title_max)),
                _ => {}
            }
        } else if let Some(max) = self.config.title_max {
            request.job.title = request.job.title.chars().take(max).collect();
        }
        let (number, step) = {
            let mut recorded = lock(&self.recorded);
            recorded.submissions.push(Submission {
                printer: request.job.printer.clone(),
                format: request.document.format,
                bytes: request.document.bytes.to_vec(),
                job: request.job.clone(),
                options: request.options.clone(),
                consented: request.consent.is_some(),
                tag,
                document_name,
            });
            let step = if recorded.submit_script.is_empty() { None } else { Some(recorded.submit_script.remove(0)) };
            (u32::try_from(recorded.submissions.len()).unwrap_or(u32::MAX), step)
        };
        if let Some(report) = request.progress.as_deref_mut() {
            for event in &self.config.progress {
                report(*event);
            }
        }
        let step = match (step, &self.config.submit_error) {
            (Some(step), _) => step,
            (None, Some(error)) => SubmitStep::Fail(error.clone()),
            (None, None) => SubmitStep::Accept,
        };
        let lost_reply = match step {
            SubmitStep::Fail(error) => return Err(error),
            SubmitStep::LoseReply(error) => Some(error),
            SubmitStep::Accept => None,
        };
        let handle = (self.config.job_handle)(request.job.printer.as_deref(), number);
        if let Some(native) = &handle {
            let id = JobId::new(self.config.name, native.clone());
            let mut remote = RemoteJob::new(id, request.job.title.clone(), JobState::Pending).with_user(self.config.user.clone());
            if self.config.query_caps.created_time {
                remote = remote.with_created((self.config.created_at)());
            }
            if placement == TagPlacement::NativeAttribute
                && let Some(tag) = &tag
            {
                remote = remote.with_tag(*tag);
            }
            lock(&self.recorded).jobs.push(remote);
        }
        if let Some(error) = lost_reply {
            return Err(error);
        }
        let mut submitted = Submitted::new(self.config.name, self.config.message.clone());
        if let Some(native) = handle {
            submitted = submitted.with_job(JobId::new(self.config.name, native));
        }
        if let Some(status) = &self.config.submitted_status {
            submitted = submitted.with_status(status.clone());
        }
        Ok(submitted)
    }

    fn watch_support(&self) -> WatchSupport {
        self.config.watch
    }

    fn wake_source(&self, _job: &JobId) -> Option<crate::WakeReceiver> {
        if !self.config.push {
            return None;
        }
        let (tx, rx) = crate::wake_channel();
        let mut recorded = lock(&self.recorded);
        tx.set_healthy(recorded.push_healthy);
        recorded.wakers.retain(crate::WakeSender::is_connected);
        recorded.wakers.push(tx);
        Some(rx)
    }

    fn recommended_poll_interval(&self) -> Option<Duration> {
        self.config.poll_interval.or(match self.config.watch {
            WatchSupport::None => None,
            WatchSupport::Poll => Some(Duration::from_secs(2)),
            WatchSupport::Events => Some(Duration::from_secs(15)),
        })
    }

    fn open_job_timeout(&self) -> Option<Duration> {
        self.config.open_job_timeout
    }

    fn job_status(&self, job: &JobId) -> Result<JobStatus, PrintError> {
        let _ = job;
        let mut recorded = lock(&self.recorded);
        recorded.status_calls = recorded.status_calls.saturating_add(1);
        match recorded.script.len() {
            0 => Ok(JobStatus::new(JobState::Unknown)),
            1 => recorded.script.first().cloned().unwrap_or_else(|| Ok(JobStatus::new(JobState::Unknown))),
            _ => recorded.script.remove(0),
        }
    }

    fn cancel_job(&self, job: &JobId) -> Result<(), PrintError> {
        if !self.config.can_cancel {
            return Err(PrintError::new(ErrorKind::Unsupported, format!("the {} backend cannot cancel a job once it is accepted", self.config.name)));
        }
        lock(&self.recorded).cancels.push(job.native().to_owned());
        Ok(())
    }

    fn tag_support(&self) -> TagSupport {
        self.config.tag_support
    }

    fn job_query_caps(&self) -> JobQueryCaps {
        self.config.query_caps
    }

    fn list_jobs(&self, printer_native: &str, filter: &JobFilter) -> Result<Vec<RemoteJob>, PrintError> {
        let _ = printer_native;
        if !self.config.query_caps.list {
            return Err(PrintError::new(ErrorKind::Unsupported, format!("the {} backend cannot list jobs", self.config.name)));
        }
        let mut jobs = lock(&self.recorded).jobs.clone();
        if !self.config.query_caps.history {
            jobs.retain(|job| !job.state.is_terminal());
        }
        // Newest first, as a listing usually is.
        jobs.reverse();
        Ok(filter_jobs(jobs, filter))
    }
}
