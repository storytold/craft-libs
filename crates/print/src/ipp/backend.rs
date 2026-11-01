//! [`IppBackend`]: the IPP client as a registry backend.

use std::fmt;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, PoisonError, RwLock};
use std::time::{Duration, SystemTime};

use super::adapter::{SharedTransport, Transport, default_transport};
use super::client::{Client, IppVersion, VersionMemory, Which};
use super::error::{IppError, Op};
use super::model::{self, JobAttrs, PrinterAttrs};
use super::plan::{self, Plan};
use super::policy::{NetworkLimits, PrinterUri};
use super::protocol::{Operation, StatusCode};
use crate::backends::{Backend, BackendClass, DocumentFormat, JobHint, PrinterDetails, PrinterRef, PrinterStatus, SubmitRequest};
use crate::{
    ErrorKind, Fingerprint, FingerprintExt as _, JobFilter, JobId, JobQueryCaps, JobStatus, NativeCode, PrintError, Progress, Reconciled, RemoteJob,
    Resolution, RetrySafety, Submitted, TagExt as _, TagPlacement, TagSupport, TagSupportExt as _, WatchSupport, filter_jobs, reconcile_jobs,
};

/// The registry name and id prefix.
const NAME: &str = "ipp";

/// The longest `job-name` in bytes (`name(MAX)`, RFC 8011 section 5.1.3).
const MAX_NAME_BYTES: usize = 255;

/// What a printer's `multiple-operation-time-out` is assumed to be when it publishes none: the
/// upper bound RFC 8011 recommends (240 seconds).
const DEFAULT_OPEN_JOB: Duration = Duration::from_secs(240);

/// Added to a printer's own `multiple-operation-time-out`.
const OPEN_JOB_MARGIN: Duration = Duration::from_secs(5);

/// What the registry may be handed: every format this crate names. Whether one printer takes one of
/// them is decided against that printer's own `document-format-supported`.
const FORMATS: &[DocumentFormat] = &DocumentFormat::ALL;

/// The `Job` and `JobOptions` settings this backend applies: copies, duplex, colour, media,
/// orientation, quality and resolution. Collation has no job attribute used here, and driver
/// keywords are a CUPS notion. Scaling (`print-scaling`) is added per printer, see [`honoured_for`].
const HONOURED: &[JobHint] =
    &[JobHint::Copies, JobHint::Duplex, JobHint::Colour, JobHint::Media, JobHint::Orientation, JobHint::Quality, JobHint::Resolution];

/// How a document gets to the printer.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum JobFlow {
    /// `Print-Job` for documents up to [`IppBackend::with_large_document`], `Create-Job` and
    /// `Send-Document` above that (and for a printer that does not list `Print-Job`).
    #[default]
    Auto,
    /// Always `Print-Job`: the job and its document in one request.
    PrintJob,
    /// Always `Create-Job` and `Send-Document`.
    CreateAndSend,
}

#[derive(Clone, Debug)]
struct Entry {
    uri: PrinterUri,
    name: Option<String>,
}

#[derive(Clone, Debug)]
struct Config {
    strict: bool,
    flow: JobFlow,
    poll_interval: Duration,
    user_name: String,
    limits: NetworkLimits,
    /// `None`: IPP 2.0 with one fallback to 1.1; `Some`: exactly that version.
    version: Option<IppVersion>,
    version_fallback: bool,
    validate_first: bool,
    large_document: usize,
    list_limit: u32,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            strict: false,
            flow: JobFlow::Auto,
            poll_interval: Duration::from_secs(2),
            user_name: "craft".into(),
            limits: NetworkLimits::default(),
            version: None,
            version_fallback: false,
            validate_first: false,
            large_document: 8 << 20,
            list_limit: 100,
        }
    }
}

/// An IPP backend. Registers as `ipp`; its printers are the ones added to it and are named
/// `ipp:<printer uri>`. See the [module documentation](super) for what it does and, above all, does
/// not do on its own.
///
/// The backend is cheap to clone and clones share the printer list: register one clone and keep the
/// other to [`add`](Self::add) or [`remove`](Self::remove) printers later. Configuration is set on a
/// backend before it is cloned; a `with_*` method makes an independent variant that still prints to
/// the same printers.
#[derive(Clone)]
pub struct IppBackend {
    printers: Arc<RwLock<Vec<Entry>>>,
    transport: SharedTransport,
    config: Arc<Config>,
    counter: Arc<AtomicU32>,
    /// The largest `multiple-operation-time-out` any printer published, in seconds; 0 when none did.
    open_job_secs: Arc<AtomicU64>,
    /// The IPP version each printer speaks, learned from its advertisement or from a rejection.
    versions: Arc<VersionMemory>,
}

impl fmt::Debug for IppBackend {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("IppBackend").field("printers", &self.urls()).field("config", &self.config).finish_non_exhaustive()
    }
}

impl Default for IppBackend {
    fn default() -> Self {
        Self::new()
    }
}

/// The printer and job number an `ipp` job id names; see [`IppBackend::parse_native`].
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct IppJob {
    /// The canonical printer URI, the native id of the printer.
    pub printer_uri: String,
    pub job_number: u32,
}

impl IppBackend {
    /// A backend with no printers and the plain-HTTP transport. Opens no connection.
    pub fn new() -> Self {
        Self {
            printers: Arc::default(),
            transport: default_transport(),
            config: Arc::default(),
            counter: Arc::new(AtomicU32::new(1)),
            open_job_secs: Arc::default(),
            versions: Arc::default(),
        }
    }

    /// A backend with these printer URIs added. A URI the policy refuses is an
    /// [`ErrorKind::InvalidRequest`] naming it, and no backend is made.
    pub fn with_printers<I>(printers: I) -> Result<Self, PrintError>
    where
        I: IntoIterator,
        I::Item: AsRef<str>,
    {
        let backend = Self::new();
        for uri in printers {
            backend.add(uri.as_ref())?;
        }
        Ok(backend)
    }

    /// Add a printer by URI (`ipp://<numeric local address>[:port]/path`, or `http://`) and return its
    /// native id, the canonical URI (the registry id is `ipp:<that>`). Adding a printer twice keeps
    /// one entry. Opens no connection. `ipps://` is refused: there is no TLS.
    pub fn add(&self, uri: &str) -> Result<String, PrintError> {
        self.add_entry(uri, None)
    }

    /// [`Self::add`] with the name a person sees in the printer list (until the printer reports its own).
    pub fn add_named(&self, uri: &str, name: &str) -> Result<String, PrintError> {
        self.add_entry(uri, Some(name.to_owned()))
    }

    /// Add the printer at `uri` (or, if it is already added, update its name when one is given) and
    /// return its native id, the canonical URI. No network.
    fn add_entry(&self, uri: &str, name: Option<String>) -> Result<String, PrintError> {
        let uri = PrinterUri::parse(uri).map_err(PrintError::from)?;
        let id = uri.canonical();
        let mut printers = self.printers.write().unwrap_or_else(PoisonError::into_inner);
        match printers.iter_mut().find(|e| e.uri == uri) {
            Some(existing) => {
                if name.is_some() {
                    existing.name = name;
                }
            }
            None => printers.push(Entry { uri, name }),
        }
        Ok(id)
    }

    /// Remove a printer; `false` when it was not added (or the URI is not one).
    pub fn remove(&self, uri: &str) -> bool {
        let Ok(uri) = PrinterUri::parse(uri) else { return false };
        let mut printers = self.printers.write().unwrap_or_else(PoisonError::into_inner);
        let before = printers.len();
        printers.retain(|e| e.uri != uri);
        printers.len() != before
    }

    /// The added printers' native ids (canonical URIs).
    pub fn urls(&self) -> Vec<String> {
        self.snapshot().iter().map(|e| e.uri.canonical()).collect()
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

    /// Use another transport (a recorder, a wrapper that adds logging). The default is the
    /// plain-HTTP transport of `craft-wsd`. The transport is only ever given validated URIs.
    #[must_use]
    pub fn with_transport(mut self, transport: impl Transport + Send + Sync + 'static) -> Self {
        self.transport = Arc::new(transport);
        self
    }

    /// Strict mode: refuse a setting the printer lists as unsupported, and send
    /// `ipp-attribute-fidelity` so that the printer refuses rather than substitutes. Off by default
    /// (such settings are left out or lowered, with a warning).
    #[must_use]
    pub fn with_strict(self, strict: bool) -> Self {
        self.configured(|c| c.strict = strict)
    }

    /// How the document travels; see [`JobFlow`].
    #[must_use]
    pub fn with_job_flow(self, flow: JobFlow) -> Self {
        self.configured(|c| c.flow = flow)
    }

    /// The size above which [`JobFlow::Auto`] uses `Create-Job` and `Send-Document` (default 8 MiB).
    #[must_use]
    pub fn with_large_document(self, bytes: usize) -> Self {
        self.configured(|c| c.large_document = bytes)
    }

    /// Ask the printer to `Validate-Job` before the job is sent (one more request; off by default).
    /// What it would change comes back as warnings.
    #[must_use]
    pub fn with_validate_first(self, validate: bool) -> Self {
        self.configured(|c| c.validate_first = validate)
    }

    /// The poll interval this backend recommends for [`Backend::job_status`] (default two seconds).
    #[must_use]
    pub fn with_poll_interval(self, interval: Duration) -> Self {
        self.configured(|c| c.poll_interval = interval)
    }

    /// The `requesting-user-name` sent with requests. Defaults to a fixed `craft`, never the account name.
    #[must_use]
    pub fn with_user_name(self, name: &str) -> Self {
        self.configured(|c| c.user_name = name.to_owned())
    }

    /// Timeout and size caps per request.
    #[must_use]
    pub fn with_limits(self, limits: NetworkLimits) -> Self {
        self.configured(|c| c.limits = limits)
    }

    /// Pin the IPP version of the requests. By default the backend speaks IPP 2.0 (every IPP
    /// Everywhere printer supports it) and falls back to 1.1 once, per printer, when a printer
    /// answers `server-error-version-not-supported`; see [`Self::version_of`]. A version set here is
    /// fixed: it is never downgraded unless the app also calls [`Self::with_version_fallback`].
    #[must_use]
    pub fn with_ipp_version(self, version: IppVersion) -> Self {
        self.configured(|c| c.version = Some(version))
    }

    /// Let a version pinned with [`Self::with_ipp_version`] fall back to IPP 1.1 (once per request,
    /// remembered per printer) when the printer answers `server-error-version-not-supported`. Without
    /// a pinned version the fallback is always on and this changes nothing. A version-rejected
    /// request creates nothing on the printer, so repeating it at 1.1 is safe even for `Print-Job`,
    /// `Create-Job` and `Send-Document`.
    #[must_use]
    pub fn with_version_fallback(self, fallback: bool) -> Self {
        self.configured(|c| c.version_fallback = fallback)
    }

    /// The IPP version the next request to this printer is sent at: the pinned version, or 2.0, in
    /// either case lowered to what is known of the printer (its `ipp-versions-supported`, or an
    /// earlier rejection) when fallback is allowed. `None` when the printer is not added.
    pub fn version_of(&self, uri: &str) -> Option<IppVersion> {
        let entry = self.entry(uri).ok()?;
        Some(self.client(&entry).version())
    }

    /// The most jobs one `Get-Jobs` asks for (default 100; two requests per listing).
    #[must_use]
    pub fn with_list_limit(self, limit: u32) -> Self {
        self.configured(|c| c.list_limit = limit.max(1))
    }

    /// The listed printer for a native id: it must be an added printer, not just a valid URI, so a
    /// request can only ever reach what the app chose.
    fn entry(&self, native_id: &str) -> Result<Entry, PrintError> {
        let uri = PrinterUri::parse(native_id).map_err(PrintError::from)?;
        self.snapshot()
            .into_iter()
            .find(|e| e.uri == uri)
            .ok_or_else(|| PrintError::printer_not_found(format!("{native_id} is not an IPP printer that was added to this backend")))
    }

    /// An IPP client for `entry` with this backend's transport, limits, user name and version (2.0
    /// unless set, falling back when allowed), sharing the remembered versions and request ids.
    fn client(&self, entry: &Entry) -> Client<SharedTransport> {
        Client::new(Arc::clone(&self.transport), entry.uri.clone())
            .with_limits(self.config.limits)
            .with_user(&self.config.user_name)
            .with_version(self.config.version.unwrap_or(IppVersion::V2_0))
            .with_fallback(self.config.version.is_none() || self.config.version_fallback)
            .with_memory(Arc::clone(&self.versions))
            .with_counter(Arc::clone(&self.counter))
    }

    /// Remember a printer's `multiple-operation-time-out` for [`Backend::open_job_timeout`], and the
    /// highest IPP version it advertises (2.0, then 1.1) when this backend may lower its version.
    fn learn(&self, entry: &Entry, attrs: &PrinterAttrs) {
        if (self.config.version.is_none() || self.config.version_fallback)
            && let Some(best) = IppVersion::highest_common(&attrs.versions)
        {
            self.versions.lower(&entry.uri.canonical(), best);
        }
        if let Some(secs) = attrs.multiple_operation_timeout.filter(|s| *s > 0) {
            self.open_job_secs.fetch_max(u64::from(secs), Ordering::Relaxed);
        }
    }

    /// The job id of job number `job_number` on the printer at `printer_uri`: the typed way to name a
    /// job this backend made (or one a person typed in), so nothing concatenates strings. The URI is
    /// canonicalised and must pass the printer URI policy ([`ErrorKind::InvalidRequest`] otherwise),
    /// and the job number must be a valid IPP `job-id` (1 to 2147483647, RFC 8011), so every id made
    /// here is one [`Self::parse_native`] accepts; the printer need not be added to any backend. The
    /// inverse is [`Self::parse_native`] (or [`Self::parse_job_id`]).
    pub fn job_id(printer_uri: &str, job_number: u32) -> Result<JobId, PrintError> {
        if job_number == 0 || i32::try_from(job_number).is_err() {
            return Err(PrintError::new(ErrorKind::InvalidRequest, format!("{job_number} is not a valid IPP job id (1 to 2147483647)"))
                .with_phase(crate::Phase::Prepare));
        }
        let uri = PrinterUri::parse(printer_uri).map_err(PrintError::from)?;
        Ok(JobId::new(NAME, job_native(&uri.canonical(), i64::from(job_number))))
    }

    /// Split the native part of an `ipp` [`JobId`] (`<printer uri>#<job number>`) into its printer
    /// URI (canonical) and job number. A malformed handle is [`ErrorKind::InvalidRequest`].
    pub fn parse_native(native: &str) -> Result<IppJob, PrintError> {
        let bad = |why: &str| {
            PrintError::new(ErrorKind::InvalidRequest, format!("{native:?} is not an ipp job handle: {why}")).with_phase(crate::Phase::Prepare)
        };
        let (uri, number) = native.rsplit_once('#').ok_or_else(|| bad("no job number"))?;
        let number: u32 = number.parse().map_err(|_| bad("the job number is not a number"))?;
        if i32::try_from(number).is_err() || number == 0 {
            return Err(bad("the job number is out of range"));
        }
        let uri = PrinterUri::parse(uri).map_err(|e| bad(&PrintError::from(e).to_string()))?;
        Ok(IppJob { printer_uri: uri.canonical(), job_number: number })
    }

    /// [`Self::parse_native`] for a whole [`JobId`]; one of another backend is [`ErrorKind::InvalidRequest`].
    pub fn parse_job_id(job: &JobId) -> Result<IppJob, PrintError> {
        if job.backend() != NAME {
            return Err(
                PrintError::new(ErrorKind::InvalidRequest, format!("{job} is not a job of the ipp backend")).with_phase(crate::Phase::Prepare)
            );
        }
        Self::parse_native(job.native())
    }

    /// The printer and job number of a job handle made by this backend. The printer must still be added.
    fn job_target(&self, job: &JobId) -> Result<(Entry, i32), PrintError> {
        let bad = |why: &str| {
            PrintError::new(ErrorKind::InvalidRequest, format!("{job} is not a job of the ipp backend: {why}")).with_phase(crate::Phase::Prepare)
        };
        if job.backend() != NAME {
            return Err(bad("it belongs to another backend"));
        }
        let IppJob { printer_uri, job_number } = Self::parse_native(job.native()).map_err(|e| bad(e.message()))?;
        let entry = self
            .entry(&printer_uri)
            .map_err(|e| if e.kind() == ErrorKind::NotFound { bad("its printer is not added to this backend") } else { e })?;
        let number = i32::try_from(job_number).map_err(|_| bad("the job number is out of range"))?;
        Ok((entry, number))
    }

    /// The job name sent: the title without control characters, then the tag as a suffix, within
    /// 255 bytes. A title that is empty once cleaned is the default title.
    pub(super) fn job_name(&self, title: &str, tag: Option<&crate::Tag>) -> String {
        let clean = sanitize_name(title);
        let mut chars = MAX_NAME_BYTES;
        loop {
            let name: String = match tag {
                Some(tag) => tag.apply_to(&clean, Some(chars)),
                None => clean.chars().take(chars).collect(),
            };
            if name.len() <= MAX_NAME_BYTES || chars == 0 {
                return name;
            }
            chars -= 1;
        }
    }
}

/// The title sent in a name: control characters (C0 including NUL, tab and line breaks, DEL and
/// C1) are removed, not refused; nothing left means the default title.
fn sanitize_name(title: &str) -> String {
    let clean: String = title.chars().filter(|c| !c.is_control()).collect();
    if clean.is_empty() { crate::Job::default().title } else { clean }
}

/// `<printer uri>#<job id>`. A printer URI never contains `#` (the policy refuses fragments).
fn job_native(uri: &str, job_id: i64) -> String {
    format!("{uri}#{job_id}")
}

/// `n` as a `u32`, saturating.
fn clamp(n: usize) -> u32 {
    u32::try_from(n).unwrap_or(u32::MAX)
}

/// The settings applied for one printer: [`HONOURED`], plus [`JobHint::Scaling`] when the printer
/// publishes `print-scaling-supported` with a keyword this backend sends (see
/// [`plan::scaling_keyword`]). A printer that does not publish it is `Unsupported`: the attribute is
/// never sent to it, and a shrink-to-fit request is reported as not applied.
fn honoured_for(attrs: &PrinterAttrs) -> Vec<JobHint> {
    let mut hints = HONOURED.to_vec();
    if plan::scaling_advertised(attrs) {
        hints.push(JobHint::Scaling);
    }
    hints
}

/// Neutral printer details from Get-Printer-Attributes: the name (printer's, given, info, or the
/// URI), formats, raster types, duplex, colour, media, copies, resolutions and state with reasons.
fn details_of(entry: &Entry, attrs: &PrinterAttrs) -> PrinterDetails {
    let id = entry.uri.canonical();
    let name =
        [attrs.name.as_deref(), entry.name.as_deref(), attrs.info.as_deref()].into_iter().flatten().find(|n| !n.is_empty()).unwrap_or(&id).to_owned();
    let mut details = PrinterDetails { id: id.clone(), name, default: false, honoured: Some(honoured_for(attrs)), ..PrinterDetails::default() };
    for format in attrs.formats.iter().filter_map(|f| DocumentFormat::from_mime(f)) {
        if !details.formats.contains(&format) {
            details.formats.push(format);
        }
    }
    for kind in attrs.raster_types.iter().flatten().filter_map(|k| crate::pwg::RasterType::from_keyword(k)) {
        if !details.raster_types.contains(&kind) {
            details.raster_types.push(kind);
        }
    }
    details.two_sided = attrs.sides.as_ref().map(|s| s.iter().any(|k| k.starts_with("two-sided")));
    details.colour = match (&attrs.color_modes, attrs.color_supported) {
        (Some(modes), _) if modes.iter().any(|m| m == "color") => Some(true),
        (_, Some(supported)) => Some(supported),
        (Some(modes), None) if !modes.is_empty() => Some(false),
        _ => None,
    };
    details.paper = media_names(attrs);
    details.max_copies = attrs.copies.and_then(|(_, max)| u32::try_from(max).ok());
    details.resolutions = attrs.resolutions.iter().flatten().map(|(x, y)| Resolution::new(*x, *y)).collect();
    details.status = model::neutral_printer_state(attrs).map(|state| {
        let mut reasons: Vec<String> = attrs.state_reasons.iter().filter(|r| r.as_str() != "none").cloned().collect();
        if attrs.accepting_jobs == Some(false) {
            reasons.push("not-accepting-jobs".to_owned());
        }
        PrinterStatus { state, reasons }
    });
    details
}

/// The paper names to show: `media-supported` entries that spell a size, then the sizes of
/// `media-size-supported` that no name covers (`custom_WxHmm`).
fn media_names(attrs: &PrinterAttrs) -> Vec<String> {
    let mut names: Vec<String> = attrs.media.iter().flatten().filter(|m| model::media_name_size(m).is_some()).cloned().collect();
    for size in attrs.media_sizes.iter().flatten() {
        let (w, h) = (size.width, size.height);
        if w.0 != w.1 || h.0 != h.1 {
            continue;
        }
        let covered = names.iter().filter_map(|n| model::media_name_size(n)).any(|(nw, nh)| size.fits(nw, nh));
        if !covered {
            let mm = |n: i32| {
                let text = format!("{:.2}", f64::from(n) / 100.0);
                text.trim_end_matches('0').trim_end_matches('.').to_owned()
            };
            names.push(format!("custom_{}x{}mm", mm(w.0), mm(h.0)));
        }
    }
    names.truncate(256);
    names
}

/// The neutral status of a job reading.
fn neutral_status(job: &JobAttrs) -> JobStatus {
    let state = model::neutral_state(job);
    let mut out = JobStatus::new(state);
    out.reasons = job.reasons.iter().filter(|r| r.as_str() != "none").cloned().collect();
    let count = |n: Option<i32>| n.and_then(|n| u32::try_from(n).ok());
    out.sheets_done = count(job.sheets_completed);
    out.pages_done = count(job.impressions_completed);
    out.sheets_total = count(job.sheets).filter(|n| *n > 0);
    out.pages_total = count(job.impressions).filter(|n| *n > 0);
    if state == crate::JobState::Completed && job.has_reason("job-completed-with-warnings") {
        out.warnings.push("the printer completed the job with warnings".to_owned());
    }
    out
}

/// A job a printer no longer lists: most likely finished, so the neutral kind is
/// [`ErrorKind::NotFound`] and a [`crate::JobWatch`] reports it as forgotten.
fn forgotten(entry: &Entry, job_id: i32, cause: &IppError) -> PrintError {
    let phase = cause.print_error(Op::Status).phase().unwrap_or(crate::Phase::Poll);
    PrintError::job_not_found(format!(
        "the printer at {} does not list job {job_id}: printers forget finished jobs, so it most likely finished (or was cancelled)",
        entry.uri.canonical()
    ))
    .with_phase(phase)
    .with_safety(RetrySafety::Safe)
}

/// The printer (or its path) answering "not found": `client-error-not-found`, `client-error-gone`,
/// or the HTTP 404/410 of a server that does not know the path.
fn is_not_found(error: &IppError) -> bool {
    matches!(error.status(), Some(StatusCode::NOT_FOUND | StatusCode::GONE)) || matches!(error, IppError::Http { status: 404 | 410, .. })
}

/// What one extra `Get-Printer-Attributes` (`printer-state` only) says about a "not found" that a
/// job request got.
enum Probe {
    /// The printer answers normally: it is the job that it does not know.
    JobForgotten,
    /// The printer is not found at that address or path either.
    PrinterMissing,
    /// The extra read failed for another reason; this is that failure.
    Unknown(IppError),
}

/// The native code of the IPP `client-error-not-found` status.
fn not_found_native() -> NativeCode {
    NativeCode { domain: "ipp", code: i64::from(StatusCode::NOT_FOUND.0), name: StatusCode::NOT_FOUND.name() }
}

/// The printer is not at that address: a "not found" for the printer, not for a job.
fn printer_missing(entry: &Entry, what: &str, op: Op) -> PrintError {
    let phase = if op == Op::Status { crate::Phase::Poll } else { crate::Phase::Reply };
    PrintError::printer_not_found(
        format!(
            "the printer was not found at {}: it answers not-found to a printer query too, so the address or path is wrong (this is not a forgotten {what})",
            entry.uri.canonical()
        ),
    )
    .with_phase(phase)
    .with_safety(RetrySafety::Safe)
    .with_native(not_found_native())
}

/// The printer said not-found, and the follow-up read failed, so the cause is unknown. Reading
/// again is harmless, so this is retryable as a read.
fn could_not_tell(entry: &Entry, what: &str, original: &IppError, probe: &IppError, op: Op) -> PrintError {
    let phase = original.print_error(op).phase().unwrap_or(crate::Phase::Reply);
    PrintError::new(
        ErrorKind::Failed,
        format!(
            "the printer at {} answered not-found for {what}, but could not tell whether the {what} is forgotten or the printer address is wrong: the follow-up printer read failed ({probe})",
            entry.uri.canonical()
        ),
    )
    .with_phase(phase)
    .with_safety(RetrySafety::Idempotent)
    .with_native(not_found_native())
}

impl IppBackend {
    /// One cheap `Get-Printer-Attributes` (`printer-state`) at the same printer URI. Asked once per
    /// "not found", never repeated.
    fn probe(&self, client: &Client<SharedTransport>) -> Probe {
        match client.get_printer_attributes(&["printer-state"]) {
            Ok(_) => Probe::JobForgotten,
            Err(e) if is_not_found(&e) => Probe::PrinterMissing,
            Err(e) => Probe::Unknown(e),
        }
    }
}

impl Backend for IppBackend {
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

    /// Each printer lists its own formats (`document-format-supported`); the registry reads them
    /// with one `Get-Printer-Attributes` when it has to decide on a conversion.
    fn formats_per_printer(&self) -> bool {
        true
    }

    /// The printers the app added; no discovery, no connection.
    fn list_printers(&self) -> Result<Vec<PrinterRef>, PrintError> {
        Ok(self
            .snapshot()
            .into_iter()
            .map(|e| {
                let id = e.uri.canonical();
                PrinterRef { name: e.name.unwrap_or_else(|| id.clone()), id, default: false, two_sided: None }
            })
            .collect())
    }

    /// One `Get-Printer-Attributes`.
    fn printer_details(&self, native_id: &str) -> Result<PrinterDetails, PrintError> {
        let entry = self.entry(native_id)?;
        let attrs = self.client(&entry).get_printer_attributes(&[]).map_err(|e| e.print_error(Op::Query))?;
        self.learn(&entry, &attrs);
        Ok(details_of(&entry, &attrs))
    }

    /// Read the printer's attributes, plan the job against them, and hand the printer the job:
    /// `Print-Job`, or `Create-Job` and `Send-Document` (see [`JobFlow`]); returns as soon as the
    /// printer has **accepted** it, never waiting for it to print. At most five bounded requests
    /// (attributes, an optional `Validate-Job`, the job, and a cleanup `Cancel-Job` when a
    /// `Send-Document` is refused). `request.job.printer` must name an added printer; there is no
    /// default. Progress events are the protocol phases (attributes read, job created, document
    /// accepted: `Progress::of(done, 3)`), not sheets.
    ///
    /// Failures say whether sending again is safe: a refusal is [`RetrySafety::Safe`]; a connection
    /// that closed or timed out **after** the request was written is [`RetrySafety::OutcomeUnknown`]
    /// (the job may exist) and the backend then leaves any created job alone; find it with the
    /// registry's `find_job` before submitting again.
    fn submit(&self, request: SubmitRequest<'_>) -> Result<Submitted, PrintError> {
        let SubmitRequest { document, job, options: per_job, consent, cancel, mut progress, tag } = request;
        if consent.is_none() {
            return Err(PrintError::new(
                ErrorKind::ConsentRequired,
                "the ipp backend prints over a network and needs the person's confirmation (UserConfirmed)",
            ));
        }
        let Some(id) = job.printer.as_deref() else {
            return Err(PrintError::printer_not_found("the ipp backend has no default printer: name one (ipp:<uri>)"));
        };
        let entry = self.entry(id)?;
        let client = self.client(&entry);
        let mut report = |done: usize, total: usize| {
            if let Some(callback) = progress.as_deref_mut() {
                callback(Progress::of(clamp(done), clamp(total)));
            }
        };
        let check_cancel = || {
            if cancel.is_cancelled() {
                Err(PrintError::new(ErrorKind::Cancelled, "the print job was cancelled").with_safety(RetrySafety::Safe))
            } else {
                Ok(())
            }
        };
        check_cancel()?;
        let attrs = client.get_printer_attributes(&[]).map_err(|e| e.print_error(Op::Query))?;
        self.learn(&entry, &attrs);
        report(1, 3);
        check_cancel()?;
        let name = self.job_name(&job.title, tag.as_ref());
        let Plan { request: job_request, mut warnings } =
            plan::plan(name, document.format.mime(), &job, &per_job, self.config.strict, &attrs).map_err(|e| e.print_error(Op::Create))?;
        if self.config.validate_first {
            let reply = client.validate_job(&job_request).map_err(|e| e.print_error(Op::Query))?;
            warnings.extend(reply.warnings);
            check_cancel()?;
        }
        let create_and_send = match self.config.flow {
            JobFlow::PrintJob => false,
            JobFlow::CreateAndSend => true,
            JobFlow::Auto => {
                let can_split = attrs.supports_operation(Operation::CreateJob.code()) != Some(false)
                    && attrs.supports_operation(Operation::SendDocument.code()) != Some(false);
                (document.bytes.len() > self.config.large_document || attrs.supports_operation(Operation::PrintJob.code()) == Some(false))
                    && can_split
            }
        };
        let accepted = if create_and_send {
            let created = client.create_job(&job_request).map_err(|e| e.print_error(Op::Create))?;
            report(2, 3);
            if let Err(error) = check_cancel() {
                // The job exists and has no document; do not leave it open.
                let _ = client.cancel_job(created.job_id);
                return Err(error);
            }
            let format = document.format.mime();
            match client.send_document(created.job_id, Some(format), true, document.bytes) {
                Ok(reply) => {
                    let mut accepted = created;
                    accepted.warnings.extend(reply.warnings);
                    accepted
                }
                Err(e) => {
                    let error = e.print_error(Op::Send);
                    // A definite refusal leaves a half-made job: remove it. After a lost reply the
                    // document may have arrived, so the job is left for a reconcile.
                    if error.retry_safety() == RetrySafety::Safe {
                        let _ = client.cancel_job(created.job_id);
                    }
                    return Err(error);
                }
            }
        } else {
            client.print_job(&job_request, document.bytes).map_err(|e| e.print_error(Op::Send))?
        };
        report(3, 3);
        warnings.extend(accepted.warnings.iter().cloned());
        let url = entry.uri.canonical();
        let mut message = format!("job {} on {url} was accepted", accepted.job_id);
        if !warnings.is_empty() {
            message.push_str(&format!(" (warnings: {})", warnings.join("; ")));
        }
        let effective = format!(
            "ipp {}.{} {}; job name {:?}",
            client.version().parts().0,
            client.version().parts().1,
            if create_and_send { "create-job + send-document" } else { "print-job" },
            job_request.job_name
        );
        let mut submitted = Submitted::new(NAME, message)
            .with_job(JobId::new(NAME, job_native(&url, i64::from(accepted.job_id))))
            .with_warnings(warnings)
            .with_effective(effective);
        if accepted.job.state.is_some() {
            submitted = submitted.with_status(neutral_status(&accepted.job));
        }
        Ok(submitted)
    }

    /// There are no event subscriptions here: callers poll.
    fn watch_support(&self) -> WatchSupport {
        WatchSupport::Poll
    }

    /// The configured poll interval (2 s by default).
    fn recommended_poll_interval(&self) -> Option<Duration> {
        Some(self.config.poll_interval)
    }

    /// A printer ends a job that got no document within its `multiple-operation-time-out`
    /// (`submission-interrupted`). The largest value a printer of this backend published, plus a
    /// margin; 240 seconds (RFC 8011's recommended upper bound) when none did.
    fn open_job_timeout(&self) -> Option<Duration> {
        match self.open_job_secs.load(Ordering::Relaxed) {
            0 => Some(DEFAULT_OPEN_JOB),
            secs => Some(Duration::from_secs(secs) + OPEN_JOB_MARGIN),
        }
    }

    /// One `Get-Job-Attributes`. A job the printer does not list is [`ErrorKind::NotFound`] and the
    /// message says what that usually means: printers forget finished jobs.
    fn job_status(&self, job: &JobId) -> Result<JobStatus, PrintError> {
        let (entry, id) = self.job_target(job)?;
        let client = self.client(&entry);
        match client.get_job_attributes(id) {
            Ok(attrs) => Ok(neutral_status(&attrs)),
            Err(e) if is_not_found(&e) => Err(match self.probe(&client) {
                Probe::JobForgotten => forgotten(&entry, id, &e),
                Probe::PrinterMissing => printer_missing(&entry, &format!("job {id}"), Op::Status),
                Probe::Unknown(probe) => could_not_tell(&entry, &format!("job {id}"), &e, &probe, Op::Status),
            }),
            Err(e) => {
                let mut error = e.print_error(Op::Status);
                if error.kind() == ErrorKind::NotFound {
                    error = PrintError::new(ErrorKind::Failed, error.message().to_owned())
                        .with_phase(crate::Phase::Poll)
                        .with_safety(RetrySafety::Idempotent);
                }
                Err(error)
            }
        }
    }

    /// `Cancel-Job`. A job that already ended, or that the printer no longer lists, is not an error:
    /// `client-error-not-possible` is confirmed with a `Get-Job-Attributes` and accepted when the job
    /// is terminal.
    fn cancel_job(&self, job: &JobId) -> Result<(), PrintError> {
        let (entry, id) = self.job_target(job)?;
        let client = self.client(&entry);
        match client.cancel_job(id) {
            Ok(()) => Ok(()),
            Err(e) if is_not_found(&e) => match self.probe(&client) {
                Probe::JobForgotten => Ok(()),
                Probe::PrinterMissing => Err(printer_missing(&entry, &format!("job {id}"), Op::Cancel)),
                Probe::Unknown(probe) => Err(could_not_tell(&entry, &format!("job {id}"), &e, &probe, Op::Cancel)),
            },
            Err(e) if e.status() == Some(StatusCode::NOT_POSSIBLE) => match client.get_job_attributes(id) {
                Ok(attrs) if model::neutral_state(&attrs).is_terminal() => Ok(()),
                Err(g) if matches!(g.status(), Some(StatusCode::NOT_FOUND | StatusCode::GONE)) => Ok(()),
                _ => Err(e.print_error(Op::Cancel)),
            },
            Err(e) => Err(e.print_error(Op::Cancel)),
        }
    }

    /// The tag is a suffix of `job-name` (`"title [ref abc123de]"`, cut so that it fits 255 bytes),
    /// and `Get-Jobs` returns the name, so a listing can match it.
    fn tag_support(&self) -> TagSupport {
        TagSupport::Carried(TagPlacement::JobNameSuffix)
    }

    /// Active jobs and history, with the user and the creation time (`date-time-at-creation`, else
    /// `time-at-creation` against `printer-up-time`).
    fn job_query_caps(&self) -> JobQueryCaps {
        JobQueryCaps { list: true, created_time: true, user: true, tag: true, history: true }
    }

    /// `Get-Jobs` for `not-completed` and for `completed` jobs (a printer that does not keep the
    /// completed ones gives the active jobs alone), plus a `Get-Printer-Attributes` for
    /// `printer-up-time` when a creation time has to be worked out from the printer's uptime.
    /// Newest first (highest job id).
    fn list_jobs(&self, printer_native: &str, filter: &JobFilter) -> Result<Vec<RemoteJob>, PrintError> {
        let entry = self.entry(printer_native)?;
        Ok(filter_jobs(self.remote_jobs(&entry, filter.user.as_deref())?, filter))
    }

    /// Reconcile by tag (the tag is in the job name), else by title, this backend's user name and time.
    fn find_job(&self, printer_native: &str, fingerprint: &Fingerprint) -> Result<Reconciled, PrintError> {
        let mut fingerprint = fingerprint.clone();
        fingerprint.user.get_or_insert_with(|| self.config.user_name.clone());
        let support = self.tag_support();
        let jobs = self.list_jobs(printer_native, &fingerprint.filter(support.matchable()))?;
        Ok(reconcile_jobs(jobs, &fingerprint, support))
    }
}

impl IppBackend {
    /// The printer's jobs, active and finished, newest first and without duplicates; `user` narrows
    /// the listing to that user's jobs. A printer that keeps no history lists only active jobs.
    /// Creation times come from `printer-up-time` when a job only has a relative time.
    fn remote_jobs(&self, entry: &Entry, user: Option<&str>) -> Result<Vec<RemoteJob>, PrintError> {
        let client = self.client(entry);
        let limit = self.config.list_limit;
        let my_jobs = user.is_some();
        let active = match client.get_jobs(Which::NotCompleted, user, my_jobs, limit) {
            Ok(rows) => rows,
            Err(e) if is_not_found(&e) => {
                return Err(match self.probe(&client) {
                    Probe::JobForgotten => e.print_error(Op::Query),
                    Probe::PrinterMissing => printer_missing(entry, "job listing", Op::Query),
                    Probe::Unknown(probe) => could_not_tell(entry, "job listing", &e, &probe, Op::Query),
                });
            }
            Err(e) => return Err(e.print_error(Op::Query)),
        };
        let history = match client.get_jobs(Which::Completed, user, my_jobs, limit) {
            Ok(rows) => rows,
            // A printer that keeps no history refuses the request itself (a client error such as
            // attributes-or-values-not-supported, or operation-not-supported); the active jobs are the
            // answer. A busy, unavailable, temporary or internal error says nothing about history, and
            // an empty one here could let a finished job look lost, so it is the caller's error.
            Err(e) if e.status().is_some_and(refuses_history) => Vec::new(),
            Err(e) => return Err(e.print_error(Op::Query)),
        };
        let now = SystemTime::now();
        let rows: Vec<JobAttrs> = {
            let mut seen: Vec<i32> = Vec::new();
            active
                .into_iter()
                .chain(history)
                .filter(|row| match row.id {
                    Some(id) if !seen.contains(&id) => {
                        seen.push(id);
                        true
                    }
                    _ => false,
                })
                .collect()
        };
        let needs_clock = rows.iter().any(|r| r.created.is_none() && r.time_at_creation.is_some());
        let up_time = if needs_clock {
            // Best effort: without it the jobs are listed without a creation time.
            client.get_printer_attributes(&["printer-up-time"]).ok().and_then(|a| a.up_time)
        } else {
            None
        };
        let url = entry.uri.canonical();
        let mut jobs: Vec<(i32, RemoteJob)> = rows
            .iter()
            .filter_map(|row| {
                let id = row.id?;
                let status = neutral_status(row);
                let mut remote =
                    RemoteJob::new(JobId::new(NAME, job_native(&url, i64::from(id))), row.name.clone().unwrap_or_default(), status.state)
                        .with_reasons(status.reasons);
                if let Some(user) = row.user.as_deref().filter(|u| !u.is_empty()) {
                    remote = remote.with_user(user);
                }
                if let Some(created) = row.created_at(up_time, now) {
                    remote = remote.with_created(created);
                }
                remote.sheets_done = status.sheets_done;
                remote.pages_done = status.pages_done;
                Some((id, remote))
            })
            .collect();
        jobs.sort_by_key(|(id, _)| std::cmp::Reverse(*id));
        Ok(jobs.into_iter().map(|(_, job)| job).collect())
    }
}

/// Does this status mean "this printer will not list its history" (so the active jobs are all there
/// is)? Only the statuses that establish a refusal of the history request itself: the request form
/// (`which-jobs=completed`) is not supported, or the operation is not. Everything else says nothing
/// about the history and is the caller's error: not-authorized, forbidden and not-found (RFC 8011
/// defines them apart from "unsupported values"), a timeout, a bad request, busy, unavailable,
/// temporary or internal errors. A wrong "no history" lets a finished job look lost and be sent twice.
fn refuses_history(code: StatusCode) -> bool {
    matches!(code, StatusCode::ATTRIBUTES_OR_VALUES_NOT_SUPPORTED | StatusCode::OPERATION_NOT_SUPPORTED)
}

#[cfg(test)]
mod history_status_tests {
    use super::{StatusCode, refuses_history};

    #[test]
    fn only_a_refusal_of_the_request_means_no_history() {
        for refused in [StatusCode::ATTRIBUTES_OR_VALUES_NOT_SUPPORTED, StatusCode::OPERATION_NOT_SUPPORTED] {
            assert!(refuses_history(refused), "{refused:?}");
        }
        for other in [
            StatusCode::BUSY,
            StatusCode::SERVICE_UNAVAILABLE,
            StatusCode::TEMPORARY_ERROR,
            StatusCode::INTERNAL_ERROR,
            StatusCode::DEVICE_ERROR,
            StatusCode::TIMEOUT,
            StatusCode::NOT_AUTHORIZED,
            StatusCode::NOT_AUTHENTICATED,
            StatusCode::FORBIDDEN,
            StatusCode::NOT_FOUND,
            StatusCode::NOT_POSSIBLE,
            StatusCode::BAD_REQUEST,
        ] {
            assert!(!refuses_history(other), "{other:?}");
        }
    }
}
