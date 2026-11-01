//! [`WsdBackend`]: the WS-Print client as a registry backend.

use std::fmt;
use std::sync::{Arc, PoisonError, RwLock};
use std::time::Duration;

use super::adapter::{SharedTransport, Transport, default_transport};
use super::client::Client;
use super::encoder::{ColorMode, PassThrough};
use super::error::{Op, WsdError};
use super::events::{Events, NotifyConfig};
use super::fault::FaultCode;
use super::job::{self, PrintOptions};
use super::model::{self, JobReason, PrinterElements};
use super::mtom::Transfer;
use super::ops::JobSection;
use super::policy::{NetworkLimits, PrinterUrl};
use super::ticket::{self, Sides, TicketShape};
use crate::backends::{Backend, BackendClass, DocumentFormat, JobHint, PrinterDetails, PrinterRef, PrinterState, PrinterStatus, SubmitRequest};
use crate::{
    Clock, ColourMode, Duplex, ErrorKind, Fingerprint, FingerprintExt as _, Job, JobFilter, JobId, JobOptions, JobQueryCaps, JobStatus, PrintError,
    Progress, Reconciled as FoundJobs, RemoteJob, Resolution, Scaling, Submitted, SystemClock, Tag, TagExt as _, TagPlacement, TagSupport,
    TagSupportExt as _, WakeReceiver, WatchSupport, filter_jobs, reconcile_jobs,
};

/// The registry name and id prefix.
const NAME: &str = "wsd";

/// The longest job name the ticket carries, in characters.
const MAX_NAME: usize = 255;

/// How many job summaries without a name `list_jobs` completes with a `GetJobElements` each.
const MAX_NAME_LOOKUPS: usize = 8;

/// What the registry may be handed: every format this crate names. A WS-Print printer lists its
/// formats as MIME types and any of them that has a [`DocumentFormat`] is shown in
/// [`Backend::printer_details`]; a document in such a format goes to the printer unchanged. Whether
/// one printer takes one of them is decided against that printer's own list, by the registry when
/// it has a converter ([`Backend::formats_per_printer`]) and in [`Backend::submit`]
/// ([`ErrorKind::UnsupportedFormat`]) in any case.
const FORMATS: &[DocumentFormat] = &DocumentFormat::ALL;

/// The `Job` and `JobOptions` settings this backend applies: copies, duplex, colour (chosen when the
/// picture is made), and the print ticket's media size, orientation, quality and resolution (each
/// still subject to the printer's own capability list and the ticket shape). Collation and scaling
/// have no WS-Print setting (the PrintTicket schema has no scaling element; `JobHint::Scaling` is
/// therefore never honoured here), and driver keywords are a CUPS notion.
const HONOURED: &[JobHint] =
    &[JobHint::Copies, JobHint::Duplex, JobHint::Colour, JobHint::Media, JobHint::Orientation, JobHint::Quality, JobHint::Resolution];

#[derive(Clone, Debug)]
struct Entry {
    url: PrinterUrl,
    name: Option<String>,
}

#[derive(Clone, Debug)]
struct Config {
    ticket_shape: TicketShape,
    drop_ticket_settings: bool,
    strict: bool,
    transfer: Transfer,
    poll_interval: Duration,
    user_name: String,
    limits: NetworkLimits,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            ticket_shape: TicketShape::Full,
            drop_ticket_settings: false,
            strict: false,
            transfer: Transfer::Mtom,
            poll_interval: Duration::from_secs(2),
            user_name: "craft".into(),
            limits: NetworkLimits::default(),
        }
    }
}

/// A WS-Print backend. Registers as `wsd`; its printers are the ones added to it and are named
/// `wsd:<url>`. See the [module documentation](super) for what it does and, above all, does not do
/// on its own.
///
/// The backend is cheap to clone and clones share the printer list: register one clone
/// (`registry.register(Box::new(backend.clone()))`) and keep the other to [`add`](Self::add) or
/// [`remove`](Self::remove) printers later. Configuration is set on a backend before it is cloned.
///
/// # Per-job and per-invocation settings
///
/// What varies per job travels in the request: copies, duplex and grey in the [`Job`], and media
/// size, orientation, quality, resolution and an explicit colour choice in the request's
/// [`JobOptions`]. The backend maps them into that job's ticket (settings the printer's capability
/// list rejects are dropped with a warning, or refused in strict mode). The **ticket shape**, strict
/// mode, the transfer encoding, the user name and the limits are backend-wide configuration. An
/// app that wants different ones for one invocation builds a variant of the backend: the `with_*`
/// methods consume a backend and a clone shares the printer list, so
/// `backend.clone().with_ticket_shape(TicketShape::Description).with_drop_ticket_settings(true)` is
/// a cheap, independent variant that still prints to the same printers. Register it in a
/// [`crate::Registry`] of its own, or call [`Backend::submit`] on it directly (it checks the
/// consent itself).
#[derive(Clone)]
pub struct WsdBackend {
    printers: Arc<RwLock<Vec<Entry>>>,
    transport: SharedTransport,
    config: Arc<Config>,
    clock: Arc<dyn Clock>,
    events: Option<Arc<Events>>,
}

impl fmt::Debug for WsdBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("WsdBackend")
            .field("printers", &self.urls())
            .field("config", &self.config)
            .field("events", &self.events)
            .finish_non_exhaustive()
    }
}

impl Default for WsdBackend {
    fn default() -> Self {
        Self::new()
    }
}

impl WsdBackend {
    /// A backend with no printers and the plain-HTTP transport. Opens no connection.
    pub fn new() -> Self {
        Self { printers: Arc::default(), transport: default_transport(), config: Arc::default(), clock: Arc::new(SystemClock), events: None }
    }

    /// A backend with these printer URLs added. A URL the policy refuses is an
    /// [`ErrorKind::InvalidRequest`] naming it, and no backend is made.
    pub fn with_printers<I>(printers: I) -> Result<Self, PrintError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let backend = Self::new();
        for url in printers {
            backend.add(url.as_ref())?;
        }
        Ok(backend)
    }

    /// Add a printer by URL (`http://<numeric local address>[:port]/path`) and return its native
    /// id, the canonical URL (the registry id is `wsd:<that>`). Adding a printer twice keeps one
    /// entry. Opens no connection.
    pub fn add(&self, url: &str) -> Result<String, PrintError> {
        self.add_entry(url, None)
    }

    /// [`Self::add`] with the name a person sees in the printer list (until the printer reports
    /// its own in [`Backend::printer_details`]).
    pub fn add_named(&self, url: &str, name: &str) -> Result<String, PrintError> {
        self.add_entry(url, Some(name.to_owned()))
    }

    /// Add the printer at `url` (or, if it is already added, update its name when one is given) and
    /// return its native id, the canonical URL. No network.
    fn add_entry(&self, url: &str, name: Option<String>) -> Result<String, PrintError> {
        let url = PrinterUrl::parse(url).map_err(PrintError::from)?;
        let id = url.as_url();
        let mut printers = self.printers.write().unwrap_or_else(PoisonError::into_inner);
        match printers.iter_mut().find(|e| e.url == url) {
            Some(existing) => {
                if name.is_some() {
                    existing.name = name;
                }
            }
            None => printers.push(Entry { url, name }),
        }
        Ok(id)
    }

    /// Remove a printer; `false` when it was not added (or the URL is not one).
    pub fn remove(&self, url: &str) -> bool {
        let Ok(url) = PrinterUrl::parse(url) else { return false };
        let mut printers = self.printers.write().unwrap_or_else(PoisonError::into_inner);
        let before = printers.len();
        printers.retain(|e| e.url != url);
        printers.len() != before
    }

    /// The added printers' native ids (canonical URLs).
    pub fn urls(&self) -> Vec<String> {
        self.snapshot().iter().map(|e| e.url.as_url()).collect()
    }

    /// A copy of the added printers.
    fn snapshot(&self) -> Vec<Entry> {
        self.printers.read().unwrap_or_else(PoisonError::into_inner).clone()
    }

    /// Apply `change` to a copy of the configuration; clones made earlier keep the old one.
    fn configured(mut self, change: impl FnOnce(&mut Config)) -> Self {
        let mut config = (*self.config).clone();
        change(&mut config);
        self.config = Arc::new(config);
        self
    }

    /// Use another clock for the push-status housekeeping (lease, renewal and back-off times). The
    /// default is the system's. Set it before [`Self::with_events`].
    #[must_use]
    pub fn with_clock(mut self, clock: Arc<dyn Clock>) -> Self {
        self.clock = clock;
        self
    }

    /// Turn on push status over WS-Eventing: bind the listener of `config` and start its thread.
    /// **Opt-in**: a backend that never calls this listens to nothing and polls, as before. Events
    /// are only a wake-up for the polling watch (see [`events`](super::events) and
    /// `docs/events.md`); clones of the backend share the listener, and it is stopped (with
    /// `Unsubscribe` for each subscription) by [`Self::shutdown_events`] or when the last clone is
    /// dropped. Set the transport and the clock first: the listener uses the ones the backend has now.
    pub fn with_events(mut self, config: NotifyConfig) -> Result<Self, PrintError> {
        let events = Events::start(config, Arc::clone(&self.clock), Arc::clone(&self.transport), self.config.limits)?;
        self.events = Some(Arc::new(events));
        Ok(self)
    }

    /// The listener and subscription manager, when [`Self::with_events`] was called.
    pub fn events(&self) -> Option<&Events> {
        self.events.as_deref()
    }

    /// The listener and subscriptions as plain data; `None` without [`Self::with_events`].
    pub fn events_status(&self) -> Option<super::events::EventsStatus> {
        self.events.as_deref().map(Events::status)
    }

    /// Do the push housekeeping that is due (subscribe, renew, release); see [`Events::maintain`].
    /// [`Backend::job_status`] calls it, so a watched job needs no extra call. Nothing without events.
    pub fn maintain_events(&self) {
        if let Some(events) = &self.events {
            events.maintain();
        }
    }

    /// Unsubscribe everything and stop the listener now (also done when the last clone is dropped).
    pub fn shutdown_events(&self) {
        if let Some(events) = &self.events {
            events.shutdown();
        }
    }

    /// Use another transport (an in-memory fake, a wrapper that adds logging). The default is the
    /// plain-HTTP transport of `craft-wsd`. The transport is only ever given validated URLs.
    #[must_use]
    pub fn with_transport(mut self, transport: impl Transport + Send + Sync + 'static) -> Self {
        self.transport = Arc::new(transport);
        self
    }

    /// How much of the ticket to send. Anything but [`TicketShape::Full`] is for printers whose
    /// firmware does not answer tickets with `JobProcessing` or `DocumentProcessing`; the settings
    /// such a ticket cannot carry are refused ([`ErrorKind::InvalidRequest`], before anything is sent)
    /// unless [`Self::with_drop_ticket_settings`] is set. The default is `Full`.
    #[must_use]
    pub fn with_ticket_shape(self, shape: TicketShape) -> Self {
        self.configured(|c| c.ticket_shape = shape)
    }

    /// With a reduced ticket, drop the settings it cannot carry (each one is a warning in the
    /// receipt) instead of refusing the job. Off by default.
    #[must_use]
    pub fn with_drop_ticket_settings(self, drop: bool) -> Self {
        self.configured(|c| c.drop_ticket_settings = drop)
    }

    /// Strict mode: refuse a setting the printer lists as unsupported, and send every setting as
    /// `MustHonor`. Off by default (such settings are left out or lowered, with a warning).
    #[must_use]
    pub fn with_strict(self, strict: bool) -> Self {
        self.configured(|c| c.strict = strict)
    }

    /// How the document travels in `SendDocument`: an MTOM part (default) or inline base64.
    #[must_use]
    pub fn with_transfer(self, transfer: Transfer) -> Self {
        self.configured(|c| c.transfer = transfer)
    }

    /// The poll interval this backend recommends for [`Backend::job_status`] (default two seconds).
    /// It is advice for [`crate::JobWatch`] and UIs: the backend itself never polls, and `submit`
    /// does not wait for the job.
    #[must_use]
    pub fn with_poll_interval(self, interval: Duration) -> Self {
        self.configured(|c| c.poll_interval = interval)
    }

    /// The `JobOriginatingUserName` sent with jobs. Defaults to a fixed `craft`, never the account name.
    #[must_use]
    pub fn with_user_name(self, name: &str) -> Self {
        self.configured(|c| c.user_name = name.to_owned())
    }

    /// Timeout and size caps per request.
    #[must_use]
    pub fn with_limits(self, limits: NetworkLimits) -> Self {
        self.configured(|c| c.limits = limits)
    }

    /// The listed printer for a native id: it must be an added printer, not just a valid URL, so a
    /// request can only ever reach what the app chose.
    fn entry(&self, native_id: &str) -> Result<Entry, PrintError> {
        let url = PrinterUrl::parse(native_id).map_err(PrintError::from)?;
        self.snapshot()
            .into_iter()
            .find(|e| e.url == url)
            .ok_or_else(|| PrintError::printer_not_found(format!("{native_id} is not a WS-Print printer that was added to this backend")))
    }

    /// A WSD print client for `entry` with this backend's transport and limits.
    fn client(&self, entry: &Entry) -> Client<SharedTransport> {
        Client::new(Arc::clone(&self.transport), entry.url.clone()).with_limits(self.config.limits)
    }

    /// The options of one job. The returned strings are what could not be applied.
    fn options(&self, job: &Job, per_job: &JobOptions, tag: Option<&Tag>) -> (PrintOptions, Vec<String>) {
        let mut notes = Vec::new();
        let config = &*self.config;
        let sides = job.duplex.map(|d| match d {
            Duplex::Off => Sides::OneSided,
            Duplex::LongEdge => Sides::TwoSidedLongEdge,
            Duplex::ShortEdge => Sides::TwoSidedShortEdge,
        });
        if job.scaling == Scaling::ShrinkToFit {
            notes.push("scaling is not applied: the document is sent as it is".to_owned());
        }
        if !job.options.is_empty() {
            notes.push("driver options are not applied by WS-Print".to_owned());
        }
        let color = match per_job.colour {
            ColourMode::Auto if job.grayscale => ColorMode::Mono,
            ColourMode::Auto => ColorMode::Auto,
            ColourMode::Colour => ColorMode::Color,
            ColourMode::Mono => ColorMode::Mono,
        };
        let options = PrintOptions {
            // The tag, when there is one, is the suffix of the name and survives the length cap.
            job_name: ticket_name(&job.title, tag),
            user_name: config.user_name.clone(),
            document_name: None,
            // One copy is the printer's default and needs no setting (a reduced ticket has no room for it).
            copies: (job.copies != 1).then_some(job.copies),
            sides,
            orientation: per_job.orientation.map(ticket_orientation),
            media_size: per_job.media.clone().filter(|m| !m.is_empty()),
            quality: per_job.quality.map(ticket_quality),
            resolution: per_job.resolution.map(|r| (r.x_dpi, r.y_dpi)),
            color,
            strict: config.strict,
            ticket_shape: config.ticket_shape,
            drop_ticket_settings: config.drop_ticket_settings,
            transfer: config.transfer,
            ..PrintOptions::default()
        };
        (options, notes)
    }

    /// The job id of job number `job_number` on the printer at `printer_url`: the typed way to name
    /// a job this backend made (or one a person typed in), so nothing concatenates strings. The URL
    /// is canonicalised and must pass the printer URL policy ([`ErrorKind::InvalidRequest`]
    /// otherwise); the printer need not be added to any backend. The inverse is
    /// [`Self::parse_native`] (or [`Self::parse_job_id`]).
    pub fn job_id(printer_url: &str, job_number: u32) -> Result<JobId, PrintError> {
        let url = PrinterUrl::parse(printer_url).map_err(PrintError::from)?;
        Ok(JobId::new(NAME, job_native(&url.as_url(), job_number)))
    }

    /// Split the native part of a `wsd` [`JobId`] (`<printer url>#<job number>`) into its printer
    /// URL (canonical) and job number. A malformed handle is [`ErrorKind::InvalidRequest`], never
    /// a panic.
    pub fn parse_native(native: &str) -> Result<WsdJob, PrintError> {
        let bad = |why: &str| {
            PrintError::new(ErrorKind::InvalidRequest, format!("{native:?} is not a wsd job handle: {why}")).with_phase(crate::Phase::Prepare)
        };
        let (url, number) = native.rsplit_once('#').ok_or_else(|| bad("no job number"))?;
        let number: u32 = number.parse().map_err(|_| bad("the job number is not a number"))?;
        let url = PrinterUrl::parse(url).map_err(|e| bad(&PrintError::from(e).to_string()))?;
        Ok(WsdJob { printer_url: url.as_url(), job_number: number })
    }

    /// [`Self::parse_native`] for a whole [`JobId`]; one of another backend is
    /// [`ErrorKind::InvalidRequest`].
    pub fn parse_job_id(job: &JobId) -> Result<WsdJob, PrintError> {
        if job.backend() != NAME {
            return Err(
                PrintError::new(ErrorKind::InvalidRequest, format!("{job} is not a job of the wsd backend")).with_phase(crate::Phase::Prepare)
            );
        }
        Self::parse_native(job.native())
    }
}

/// The printer and job number a `wsd` job id names; see [`WsdBackend::parse_native`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct WsdJob {
    /// The canonical printer URL, the native id of the printer.
    pub printer_url: String,
    pub job_number: u32,
}

/// The ticket orientation for a neutral one; anything unknown is portrait.
fn ticket_orientation(o: crate::Orientation) -> ticket::Orientation {
    match o {
        crate::Orientation::Landscape => ticket::Orientation::Landscape,
        crate::Orientation::ReversePortrait => ticket::Orientation::ReversePortrait,
        crate::Orientation::ReverseLandscape => ticket::Orientation::ReverseLandscape,
        _ => ticket::Orientation::Portrait,
    }
}

/// The ticket's `PrintQuality` token for the neutral hint. Whether the printer takes it is the
/// plan's decision against its capability list: a value it does not list is left out with a warning,
/// or refused in strict mode, like any other setting.
fn ticket_quality(q: crate::Quality) -> ticket::Quality {
    match q {
        crate::Quality::Draft => ticket::Quality::Draft,
        crate::Quality::High => ticket::Quality::High,
        crate::Quality::Photo => ticket::Quality::Photo,
        _ => ticket::Quality::Normal,
    }
}

/// A listed format as a [`DocumentFormat`], when this crate has a name for it.
fn document_format(listed: &str) -> Option<DocumentFormat> {
    FORMATS.iter().copied().find(|f| f.mime().eq_ignore_ascii_case(listed.split(';').next().unwrap_or("").trim()))
}

/// The neutral state of a WSD `PrinterState` (`Idle`, `Processing`, `Stopped`); `None` otherwise.
fn printer_state(state: &str) -> Option<PrinterState> {
    match state {
        "Idle" => Some(PrinterState::Idle),
        "Processing" => Some(PrinterState::Processing),
        "Stopped" => Some(PrinterState::Stopped),
        _ => None,
    }
}

/// Neutral printer details from GetPrinterElements: the name (printer's, given, or the URL),
/// colour, formats, duplex, media sizes, copies, resolutions and state with reasons.
fn details_of(entry: &Entry, elements: PrinterElements) -> PrinterDetails {
    let id = entry.url.as_url();
    let description = elements.description.unwrap_or_default();
    let name = [Some(description.name.as_str()), entry.name.as_deref()].into_iter().flatten().find(|n| !n.is_empty()).unwrap_or(&id).to_owned();
    let mut details = PrinterDetails {
        id: id.clone(),
        name,
        default: false,
        colour: description.color_supported,
        honoured: Some(HONOURED.to_vec()),
        ..PrinterDetails::default()
    };
    if let Some(caps) = elements.capabilities {
        for format in caps.formats.iter().filter_map(|f| document_format(f)) {
            if !details.formats.contains(&format) {
                details.formats.push(format);
            }
        }
        details.two_sided = if caps.sides.is_empty() { None } else { Some(caps.sides.iter().any(|s| s.starts_with("TwoSided"))) };
        details.paper = caps.media_sizes;
        details.max_copies = caps.copies.map(|(_, max)| max);
        details.resolutions = caps.resolutions.iter().map(|(x, y)| Resolution::new(*x, *y)).collect();
    }
    details.status = elements.status.and_then(|s| printer_state(&s.state).map(|state| PrinterStatus { state, reasons: s.reasons }));
    details
}

impl Backend for WsdBackend {
    fn name(&self) -> &'static str {
        NAME
    }

    /// Every job goes over the network, so the registry asks for a [`crate::UserConfirmed`].
    fn class(&self) -> BackendClass {
        BackendClass::Network
    }

    fn accepted_formats(&self) -> &[DocumentFormat] {
        FORMATS
    }

    /// Each printer lists its own formats (and the registry reads them with one `GetPrinterElements`
    /// when it has to decide on a conversion).
    fn formats_per_printer(&self) -> bool {
        true
    }

    /// The printers the app added; no discovery, no connection.
    fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError> {
        Ok(self
            .snapshot()
            .into_iter()
            .map(|e| {
                let id = e.url.as_url();
                PrinterRef { name: e.name.unwrap_or_else(|| id.clone()), id, default: false, two_sided: None }
            })
            .collect())
    }

    /// One `GetPrinterElements` (description, status, capabilities) to the printer.
    fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError> {
        let entry = self.entry(native_id)?;
        let elements = job::printer_info(&self.client(&entry)).map_err(|e| e.print_error(Op::Query))?;
        Ok(details_of(&entry, elements))
    }

    /// Create the job and send the document; returns as soon as the printer has **accepted** it
    /// (the reply to `SendDocument`), never waiting for the job to print. The call makes four bounded
    /// requests at most (capabilities, create, send, and a cleanup cancel when the send is refused),
    /// each limited by the configured request timeout. `request.job.printer` must name an added
    /// printer; there is no default. Progress events are the three protocol phases of the submission
    /// (capabilities read, job created, document accepted: `Progress::of(done, 3)`), not sheets;
    /// the sheets the printer has finished come from [`Backend::job_status`].
    ///
    /// Follow the job with [`Backend::job_status`] (through the registry, or a
    /// [`crate::JobWatch`]). Failures say whether sending again is safe: a refusal is
    /// [`crate::RetrySafety::Safe`]; a connection that closed or timed out **after** the request was
    /// written is [`crate::RetrySafety::OutcomeUnknown`] (the job may exist) and the backend then
    /// leaves any created job alone; call [`WsdBackend::reconcile`] before submitting again.
    fn submit(&self, request: SubmitRequest<'_>) -> Result<Submitted, PrintError> {
        let SubmitRequest { document, job, options: per_job, consent, cancel, progress, tag } = request;
        // The registry checks this too; a caller that holds the backend directly gets the same answer.
        let Some(consent) = consent else {
            return Err(PrintError::new(
                ErrorKind::ConsentRequired,
                "the wsd backend prints over a network and needs the person's confirmation (UserConfirmed)",
            ));
        };
        let Some(id) = job.printer.as_deref() else {
            return Err(PrintError::printer_not_found("the wsd backend has no default printer: name one (wsd:<url>)"));
        };
        let entry = self.entry(id)?;
        let (options, notes) = self.options(&job, &per_job, tag.as_ref());
        let encoder = PassThrough::new(document.format.mime(), document.bytes);
        let mut progress = progress;
        let mut report = |done: usize, total: usize| {
            if let Some(callback) = progress.as_deref_mut() {
                let clamp = |n: usize| u32::try_from(n).unwrap_or(u32::MAX);
                callback(Progress::of(clamp(done), clamp(total)));
            }
        };
        let mut op = Op::Query;
        let accepted =
            job::submit_document(&self.client(&entry), &encoder, &options, consent, &cancel, &mut report, &mut op).map_err(|e| e.print_error(op))?;
        let mut warnings = accepted.warnings;
        warnings.extend(notes);
        let url = entry.url.as_url();
        let mut message = format!("job {} on {url} ({}) was accepted", accepted.job_id, accepted.printer_name);
        if !warnings.is_empty() {
            message.push_str(&format!(" (warnings: {})", warnings.join("; ")));
        }
        let effective = format!("ticket shape {}; job name {:?}", self.config.ticket_shape.token(), options.job_name);
        Ok(Submitted::new(NAME, message)
            .with_job(JobId::new(NAME, job_native(&url, accepted.job_id)))
            .with_warnings(warnings)
            .with_effective(effective))
    }

    /// `Events` while at least one printer's subscription is established and trusted (a verified
    /// event arrived recently; see [`events`](super::events)), otherwise `Poll`. Without
    /// [`WsdBackend::with_events`] it is always `Poll`. Even with `Events`, status comes from polling.
    fn watch_support(&self) -> WatchSupport {
        if self.events.as_deref().is_some_and(Events::any_healthy) { WatchSupport::Events } else { WatchSupport::Poll }
    }

    /// A wake-up channel for the job's printer, when events are on. Sends nothing; the subscription
    /// is made by the next housekeeping (the next `job_status`).
    fn wake_source(&self, job: &JobId) -> Option<WakeReceiver> {
        let events = self.events.as_deref()?;
        let (entry, id) = self.job_target(job).ok()?;
        events.register(&entry.url, id)
    }

    /// The configured poll interval (2 s by default).
    fn recommended_poll_interval(&self) -> Option<Duration> {
        Some(self.config.poll_interval)
    }

    /// A WS-Print printer aborts a job that got no document within 60 seconds of `CreatePrintJob`
    /// (`DocumentTimeoutError`); five seconds more for the printer's own clock.
    fn open_job_timeout(&self) -> Option<Duration> {
        Some(Duration::from_secs(65))
    }

    /// One `GetJobElements`. A job the printer does not list is [`ErrorKind::NotFound`] and the
    /// message says what that usually means: printers forget finished jobs, so a job that was
    /// accepted and is now unknown most likely finished.
    ///
    /// When the app opted in to push status ([`WsdBackend::with_events`]) the call first does the
    /// WS-Eventing housekeeping that is due ([`WsdBackend::maintain_events`]): at most one bounded
    /// `Subscribe`, `Renew` or `Unsubscribe` request per subscription, none when nothing is due, and
    /// it returns at once when another call is already doing it. That is time spent on the polling
    /// thread before the `GetJobElements`, so poll from a worker thread; without events nothing extra
    /// is sent.
    fn job_status(&self, job: &JobId) -> Result<JobStatus, PrintError> {
        self.maintain_events();
        let (entry, id) = self.job_target(job)?;
        match self.client(&entry).job_status(id) {
            Ok(status) => Ok(neutral_status(&status)),
            Err(WsdError::Fault(f)) if f.code == FaultCode::ClientErrorJobIdNotFound => Err(forgotten(&entry, id, &WsdError::Fault(f))),
            Err(e) => {
                let mut error = e.print_error(Op::Status);
                if error.kind() == ErrorKind::NotFound {
                    // Only the printer's own "no such job" means forgotten; anything else is a failure.
                    error = PrintError::new(ErrorKind::Failed, error.message().to_owned())
                        .with_phase(crate::Phase::Poll)
                        .with_safety(crate::RetrySafety::Idempotent);
                }
                Err(error)
            }
        }
    }

    /// `CancelJob`, treating "too late" as success (see [`job::cancel_job_tolerant`]).
    fn cancel_job(&self, job: &JobId) -> Result<(), PrintError> {
        let (entry, id) = self.job_target(job)?;
        job::cancel_job_tolerant(&self.client(&entry), id).map_err(|e| e.print_error(Op::Cancel))
    }

    /// The tag is a suffix of the job name in the ticket (`"title [ref abc123de]"`, cut so that it
    /// fits the 255 characters), and `GetActiveJobs` / `GetJobHistory` return the name. A printer
    /// may store a shorter name than it was sent and lose the suffix, so the tag is carried but a
    /// listing is not relied on to return it ([`TagSupport::Partial`]): a reconcile matches by
    /// title (a cut-short title included), user and time, never by the tag alone, which would
    /// report a printed job as not found.
    /// An empty ticket (`TicketShape::Empty`) names no job, so then nothing carries the tag.
    fn tag_support(&self) -> TagSupport {
        if self.config.ticket_shape.carries_name() { TagSupport::Partial(TagPlacement::JobNameSuffix) } else { TagSupport::None }
    }

    /// Active jobs and history, with the user; no creation time (job summaries do not carry one).
    fn job_query_caps(&self) -> JobQueryCaps {
        JobQueryCaps { list: true, created_time: false, user: true, tag: self.config.ticket_shape.carries_name(), history: true }
    }

    /// `GetActiveJobs` and `GetJobHistory` (two idempotent reads; a service without
    /// `GetJobHistory` gives the active jobs alone, any other fault is an error), and for up to a few summaries that lack a
    /// name one `GetJobElements` to fill in the name and user. Newest first (highest job id).
    fn list_jobs(&self, printer_native: &str, filter: &JobFilter) -> Result<Vec<RemoteJob>, PrintError> {
        let entry = self.entry(printer_native)?;
        Ok(filter_jobs(self.remote_jobs(&entry)?, filter))
    }

    /// Reconcile by tag (the tag is in the job name), else by title and this backend's user name.
    fn find_job(&self, printer_native: &str, fingerprint: &Fingerprint) -> Result<FoundJobs, PrintError> {
        let shape = self.config.ticket_shape;
        if !shape.carries_name() {
            return Err(unmatchable(shape));
        }
        let mut fingerprint = fingerprint.clone();
        // Compared as it was sent: cut to the name's length and as the printer can store it.
        fingerprint.title = comparable(&ticket_name(&fingerprint.title, None));
        if shape.carries_user() {
            fingerprint.user.get_or_insert_with(|| self.config.user_name.clone());
        } else {
            // The ticket named no user, so the printer filled in its own.
            fingerprint.user = None;
        }
        let support = self.tag_support();
        // The tag may have survived or not: list everything the tag could mark and let the
        // reconcile try it first and the title, user and time after.
        let jobs = self.list_jobs(printer_native, &fingerprint.filter(support.carries()))?;
        Ok(reconcile_jobs(jobs, &fingerprint, support))
    }
}

/// The backend's job handle: `<printer url>#<job id>`. A printer URL never contains `#` (the policy
/// refuses fragments), so the last `#` separates.
fn job_native(url: &str, job_id: u32) -> String {
    format!("{url}#{job_id}")
}

/// A job a printer no longer lists. Printers drop finished jobs quickly, so "unknown" after an
/// accepted job means finished, cancelled or lost; the neutral kind is [`ErrorKind::NotFound`] and a
/// [`crate::JobWatch`] reports it as forgotten.
fn forgotten(entry: &Entry, job_id: u32, cause: &WsdError) -> PrintError {
    let phase = cause.print_error(Op::Status).phase().unwrap_or(crate::Phase::Poll);
    PrintError::job_not_found(format!(
        "the printer at {} does not list job {job_id}: printers forget finished jobs, so it most likely finished (or was cancelled)",
        entry.url.as_url()
    ))
    .with_phase(phase)
    .with_safety(crate::RetrySafety::Safe)
}

/// The neutral status for a WS-Print job status.
fn neutral_status(status: &model::JobStatus) -> JobStatus {
    let has = |r: JobReason| status.reasons.contains(&r);
    let state = match &status.state {
        model::JobState::Pending => crate::JobState::Pending,
        model::JobState::PendingHeld => crate::JobState::Held,
        model::JobState::Processing | model::JobState::Started | model::JobState::Terminating => crate::JobState::Processing,
        model::JobState::ProcessingStopped => crate::JobState::NeedsAttention,
        model::JobState::Completed if has(JobReason::JobCompletedWithErrors) => crate::JobState::CompletedWithErrors,
        model::JobState::Completed => crate::JobState::Completed,
        model::JobState::Canceled => crate::JobState::Cancelled,
        model::JobState::Aborted => crate::JobState::Aborted,
        model::JobState::Other(_) => crate::JobState::Unknown,
    };
    let mut out = JobStatus::new(state);
    out.reasons = status.reasons.iter().filter(|r| **r != JobReason::None).map(|r| r.name().to_owned()).collect();
    out.sheets_done = Some(u32::try_from(status.sheets_completed).unwrap_or(u32::MAX));
    // "Completed with warnings" is a completed job plus the printer's words about it.
    if state == crate::JobState::Completed && has(JobReason::JobCompletedWithWarnings) {
        out.warnings.push("the printer completed the job with warnings".to_owned());
    }
    out
}

/// The result of [`WsdBackend::reconcile`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reconciled {
    /// The printer lists no job with this name from this backend's user: as far as it can tell, the
    /// document did not arrive, and submitting again will not duplicate it.
    NotFound,
    /// Exactly one job matches: the earlier attempt did create it.
    Found(JobMatch),
    /// Several match (the same title was printed before and the printer still lists it). Newest first
    /// (highest job id); look at the states to choose.
    Ambiguous(Vec<JobMatch>),
}

/// A job found by [`WsdBackend::reconcile`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct JobMatch {
    /// The registry's id for the job (`wsd:<printer url>#<id>`), usable with `job_status` and
    /// `cancel_job` of a [`crate::Registry`] that holds this backend.
    pub job: JobId,
    pub name: String,
    pub status: JobStatus,
}

/// The job name a ticket carries for this title and tag: the tag is the suffix and survives the
/// length cap.
fn ticket_name(title: &str, tag: Option<&Tag>) -> String {
    match tag {
        Some(tag) => tag.apply_to(title, Some(MAX_NAME)),
        None => title.chars().take(MAX_NAME).collect(),
    }
}

/// Fill the name and the user of a job summary that lacks them from the job's own ticket (one
/// `GetJobElements`). Best effort: when the lookup fails, or the ticket lacks a field too, what was
/// empty stays empty. A field the summary did carry is kept.
fn complete_from_ticket(client: &Client<SharedTransport>, id: u32, name: &mut String, user: &mut String) {
    if let Ok(elements) = client.get_job_elements(id, &[JobSection::Ticket])
        && let Some(ticket) = elements.ticket
    {
        if name.is_empty() {
            *name = ticket.job_name;
        }
        if user.is_empty() {
            *user = ticket.user_name;
        }
    }
}

/// A job name as it can come back from the printer: XML drops the control characters it cannot
/// carry and turns line breaks into `\n`, and a printer may trim the ends. Both sides of a match go
/// through this.
fn comparable(name: &str) -> String {
    let kept: String = name
        .replace("\r\n", "\n")
        .chars()
        .map(|c| if c == '\r' { '\n' } else { c })
        .filter(|&c| !(u32::from(c) < 0x20 && !matches!(c, '\t' | '\n')) && c != '\u{fffe}' && c != '\u{ffff}')
        .collect();
    kept.trim().to_owned()
}

/// A ticket shape that names no job: nothing on the printer can be matched to the print.
fn unmatchable(shape: TicketShape) -> PrintError {
    PrintError::new(ErrorKind::Unsupported, format!("a {} ticket names no job, so the printer's jobs cannot be matched to this print", shape.token()))
}

/// The printer's job history, or none when it does not offer one (`wsa:ActionNotSupported`: the
/// service has no GetJobHistory). Any other fault, a busy or broken printer included, is an error:
/// read as "no history", it would make a finished job look unknown and invite a second print.
fn history_or_none(history: Result<Vec<model::JobSummary>, WsdError>) -> Result<Vec<model::JobSummary>, PrintError> {
    match history {
        Ok(rows) => Ok(rows),
        Err(WsdError::Fault(f)) if f.code == FaultCode::ActionNotSupported => Ok(Vec::new()),
        Err(e) => Err(e.print_error(Op::Query)),
    }
}

impl WsdBackend {
    /// Every job the printer lists, neutral, newest first.
    fn remote_jobs(&self, entry: &Entry) -> Result<Vec<RemoteJob>, PrintError> {
        let client = self.client(entry);
        let active = client.get_active_jobs().map_err(|e| e.print_error(Op::Query))?;
        let history = history_or_none(client.get_job_history())?;
        let url = entry.url.as_url();
        let mut seen: Vec<u32> = Vec::new();
        let mut lookups = 0usize;
        let mut jobs: Vec<(u32, RemoteJob)> = Vec::new();
        for row in active.into_iter().chain(history) {
            let id = row.status.job_id;
            if seen.contains(&id) {
                continue;
            }
            seen.push(id);
            let (mut name, mut user) = (row.job_name.clone(), row.user_name.clone());
            if name.is_empty() && lookups < MAX_NAME_LOOKUPS {
                lookups += 1;
                complete_from_ticket(&client, id, &mut name, &mut user);
            }
            let status = neutral_status(&row.status);
            let mut remote = RemoteJob::new(JobId::new(NAME, job_native(&url, id)), comparable(&name), status.state).with_reasons(status.reasons);
            if !user.is_empty() {
                remote = remote.with_user(user);
            }
            remote.sheets_done = status.sheets_done;
            jobs.push((id, remote));
        }
        jobs.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        Ok(jobs.into_iter().map(|(_, job)| job).collect())
    }

    /// The printer and job number of a job handle made by this backend. The printer must still be
    /// added: a request only ever reaches what the app chose. A handle that is not one of ours, or
    /// whose printer was removed, is [`ErrorKind::InvalidRequest`] (not `NotFound`: that means a
    /// forgotten job).
    fn job_target(&self, job: &JobId) -> Result<(Entry, u32), PrintError> {
        let bad = |why: &str| {
            PrintError::new(ErrorKind::InvalidRequest, format!("{job} is not a job of the wsd backend: {why}")).with_phase(crate::Phase::Prepare)
        };
        if job.backend() != NAME {
            return Err(bad("it belongs to another backend"));
        }
        let WsdJob { printer_url: url, job_number: number } = Self::parse_native(job.native()).map_err(|e| bad(e.message()))?;
        let url = url.as_str();
        let entry = self.entry(url).map_err(|e| if e.kind() == ErrorKind::NotFound { bad("its printer is not added to this backend") } else { e })?;
        Ok((entry, number))
    }

    /// After an error that says the outcome is unknown ([`PrintError::outcome_unknown`], for
    /// example a connection that closed with no reply to `SendDocument`), find out whether the
    /// printer made the job **before** sending again. It reads the printer's active jobs and its
    /// history (`GetActiveJobs`, `GetJobHistory`: idempotent reads) and looks for jobs named
    /// `job_title` (the title of the [`crate::Job`], cut to the 255 characters this backend sends)
    /// from this backend's user name.
    ///
    /// WS-Print job summaries carry no creation time, so the window is whatever the printer still
    /// lists; a title that was printed earlier may match too ([`Reconciled::Ambiguous`]). A job that
    /// was created but never received its document shows as pending: cancel it with the registry's
    /// `cancel_job`. Nothing is sent that changes a job.
    pub fn reconcile(&self, printer: &str, job_title: &str) -> Result<Reconciled, PrintError> {
        self.reconcile_tagged(printer, job_title, None)
    }

    /// [`Self::reconcile`] for a request that carried a [`Tag`]: the job was named with the tag as a
    /// suffix of the title (cut to fit), so that is the name to look for. Without the tag here, a
    /// tagged job is never found and sending again would print it twice. A printer that stored the
    /// name shorter and lost the tag suffix is matched by title (a cut-short title included) and
    /// user; a listed name that carries a different tag is not a match.
    pub fn reconcile_tagged(&self, printer: &str, job_title: &str, tag: Option<&Tag>) -> Result<Reconciled, PrintError> {
        let entry = self.entry(printer)?;
        let shape = self.config.ticket_shape;
        if !shape.carries_name() {
            return Err(unmatchable(shape));
        }
        let client = self.client(&entry);
        let name = comparable(&ticket_name(job_title, tag));
        let title = comparable(job_title);
        // Without a user in the ticket the printer filled in its own, so the user cannot narrow it.
        let user = shape.carries_user().then_some(self.config.user_name.as_str());
        let mut lookups = 0usize;
        let active = client.get_active_jobs().map_err(|e| e.print_error(Op::Query))?;
        let history = history_or_none(client.get_job_history())?;
        let url = entry.url.as_url();
        // (whether the name is the one sent, the match, its job id)
        let mut found: Vec<(bool, JobMatch, u32)> = Vec::new();
        for row in active.into_iter().chain(history) {
            if found.iter().any(|(_, _, id)| *id == row.status.job_id) {
                continue;
            }
            // A summary without a name, or without the user we filter on, is completed from the
            // job's own ticket (a bounded number of lookups) before anything is decided from it.
            let (mut listed_name, mut listed_user) = (row.job_name.clone(), row.user_name.clone());
            if (listed_name.is_empty() || (user.is_some() && listed_user.is_empty())) && lookups < MAX_NAME_LOOKUPS {
                lookups += 1;
                complete_from_ticket(&client, row.status.job_id, &mut listed_name, &mut listed_user);
            }
            // The name as sent, or one the printer stored shorter without the tag suffix (see
            // `tag_support`): then the title decides, as in a listing. A name that carries a tag of
            // its own is another print's, whatever its title.
            let listed = comparable(&listed_name);
            let exact = listed == name;
            if !exact && !(Tag::find_in(&listed).is_none() && crate::remote::title_matches(&listed, &title)) {
                continue;
            }
            // Another user's job is not ours. A user the printer never told us (the summary and the
            // ticket both lack it) cannot rule out a job that carries the name as sent.
            if user.is_some_and(|user| if listed_user.is_empty() { !exact } else { listed_user != user }) {
                continue;
            }
            let job = JobId::new(NAME, job_native(&url, row.status.job_id));
            found.push((exact, JobMatch { job, name: listed_name, status: neutral_status(&row.status) }, row.status.job_id));
        }
        // A job that still carries the name as sent is the answer, whatever else shares its title;
        // the untagged rows decide only when none does.
        if found.iter().any(|(exact, ..)| *exact) {
            found.retain(|(exact, ..)| *exact);
        }
        found.sort_by_key(|(_, _, id)| std::cmp::Reverse(*id));
        let mut matches: Vec<JobMatch> = found.into_iter().map(|(_, m, _)| m).collect();
        Ok(match matches.len() {
            0 => Reconciled::NotFound,
            1 => matches.pop().map_or(Reconciled::NotFound, Reconciled::Found),
            _ => Reconciled::Ambiguous(matches),
        })
    }
}
