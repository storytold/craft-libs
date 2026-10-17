# craft-print

Submit a print-ready PDF to the system spooler. Used by PdfCraft and DesignCraft;
there are no dependencies on either app or on a UI framework. Requires Rust 1.95.

```rust,no_run
let job = craft_print::Job { title: "Report".into(), ..Default::default() };
let receipt = craft_print::submit(&pdf_bytes, &job)?;
```

The caller generates the PDF, checks document permissions, and handles page selection,
imposition, marks and bleed. This crate discovers printers, exposes CUPS driver options,
opens Windows printer preferences and submits jobs with copies, collation, duplex and colour.
`command_preview` returns diagnostics without creating files or starting a process.

## Backends and the registry

`submit`, `printers` and the rest of the functions above are the unchanged single-spooler API.
Beside them the crate has a `backends::Backend` trait and a `Registry` that holds several
backends at once (the system spooler plus, later, network protocols such as WS-Print or IPP):

```rust,no_run
use craft_print::{Document, Job, SubmitRequest};

let listing = craft_print::list_printers()?;          // every backend; ids look like "cups:Office"
for failure in &listing.errors { eprintln!("{}: {}", failure.backend, failure.error); }
let job = Job { printer: Some("cups:Office".into()), ..Job::default() };
let receipt = craft_print::submit_to_backend(SubmitRequest::new(Document::pdf(&pdf_bytes), job))?;
# Ok::<(), Box<dyn std::error::Error>>(())
```

* **Built-in backend.** `SystemSpooler` is an adapter over the existing functions (`printers`,
  `printer_options`, `submit_with_progress`), so behaviour is the same. It registers as `cups` on
  macOS and Linux and as `spooler` on Windows (the `winprint` code stays behind `cfg(windows)`);
  the process-wide registry (`craft_print::registry()`) holds it from the start. The older
  `Backend` enum and `backend()` still describe that built-in kind; the trait lives at
  `craft_print::backends::Backend`.
* **Ids** are `"<backend>:<native id>"`; only the first colon separates the prefix. An id
  without a registered prefix is `ErrorKind::UnknownBackend`. A job with no printer goes to the
  only backend, or to the first one that lists a default printer.
* **Several backends, partial failure.** `list_printers` asks every backend and returns what
  answered plus one `BackendError` per backend that failed; it is `Err(Unsupported)` only when
  none is registered. `Registry::register` returns `RegisterError` for a duplicate or invalid name,
  never panics. Make a standalone `Registry::new()` for isolation or tests.
* **Neutral errors.** `PrintError::kind()` (`ErrorKind`) and `is_retryable()` are the contract;
  `native()` is an optional `NativeCode { domain, code, name }` with domains such as `"win32"`,
  `"ipp"`, `"wsd-fault"` or `"cups"`. The legacy `Error::Spool` converts with its message kept.
* **Capabilities may be unknown.** `PrinterDetails` lists formats, driver options, paper and the
  `JobHint`s the backend honours (`honoured`); an empty list or `None` means "not published".
  Duplex, colour, scaling and driver options in a `Job` are hints a backend may ignore, and
  `honoured` tells a UI which ones it applies. `status` is `None` when the backend cannot say
  before a job (CUPS and the Windows spooler report after the fact).
* **Documents** are bytes plus a `DocumentFormat`: `Pdf`, `PwgRaster`, `PostScript`, `OctetStream`,
  `Jpeg`, `Png`, `Tiff`, `PlainText`, `Xps` (`application/vnd.ms-xpsdocument`) and `OpenXps`
  (`application/oxps`). `mime()`, `from_mime()` (parameters and case ignored), `extension()`,
  `from_extension()`, `sniff(bytes)` (magic numbers; plain text by content; XPS is a ZIP and is never
  sniffed) and `detect(file_name, bytes)` (bytes first, then the extension) turn files into
  formats; `DocumentFormat::negotiate(&preference, &printer_formats)` picks the first preferred
  format a printer lists (`negotiate_default` uses `DEFAULT_PREFERENCE`: PDF, PWG raster, Open XPS,
  XPS, PostScript, TIFF, PNG, JPEG, text, then the printer's own language). A backend declares
  `accepted_formats()` and, when printers differ, `formats_per_printer()` plus
  `PrinterDetails::formats`. The built-in spooler stays **PDF only**: CUPS is handed the PDF and the
  Windows job is drawn from it by the in-box PDF renderer, and the crate has no image, text or XPS
  renderer to offer instead; an app that wants to print a JPEG on the spooler registers a converter
  that makes PDF from it.

### Formats and conversion

This crate has no renderer, so the app that owns one says how to make a format a printer takes:

* **`DocumentConverter`** (`targets(from)`, `convert(document, target, &ConvertContext) ->
  OwnedDocument`), registered with `Registry::set_converter` (or `craft_print::set_converter`).
  `ConvertContext` carries the chosen `resolution`, whether to render in `colour`, and the media,
  orientation, duplex, page count and cancel token of the request.
* **`craft_print::pwg`** is the neutral PWG raster **encoder** for pages the converter has already
  rendered (`RasterPage`, `PwgOptions`, `encode`). It needs no feature and builds for wasm32;
  `craft_print::wsd::pwg` re-exports it.
* **Raster pixel formats.** `pwg::RasterType` is a colour space and a depth: sGray, sRGB or Adobe RGB, at 8 or 16 bits
  (`sgray_8`, `srgb_16`, `adobe-rgb_16`...; header colour spaces 18, 19 and 20 of PWG 5102.4, 16-bit samples
  most significant byte first), written by `pwg::encode_typed` (`TypedPage`); `encode` is the 8-bit sGray/sRGB case.
  An IPP printer lists what it takes in `pwg-raster-document-type-supported`, which is read into
  `PrinterDetails::raster_types` (the types this crate can write, either spelling, no duplicates; empty when not
  published, taken as `RasterType::BASELINE`: sRGB and sGray at 8 bits). A converter lists the types it can make, best
  first, in `DocumentConverter::raster_types` (default: the baseline), and the registry gives it the first the printer
  lists in `ConvertContext::raster` (grey for a monochrome job, else colour; the baseline when nothing is in common;
  `choose_raster_type`), reporting it in `Conversion::raster`. WS-Print does not publish raster types, so it gets the
  baseline. CMYK, device colour and one-bit types are not written, and a printer's ICC profile
  (`printer-icc-profiles`) is not read yet.
* **The registry decides.** A document in a format the printer accepts is sent as it is and the
  converter is not asked. Otherwise the best target the converter makes and the printer lists is
  chosen (`ConversionPolicy::preference`, default `DocumentFormat::DEFAULT_PREFERENCE`), the
  resolution is the request's `JobOptions::resolution` if set, else the printer's listed
  `PrinterDetails::resolutions` entry nearest `ConversionPolicy::target_dpi` (default 300; distance
  is summed over both axes, ties go to the higher one, nothing listed means 300 square; see
  `choose_resolution`), the converter runs and its output is sent. For a raster target the chosen
  resolution is also put into the job so the ticket and the raster agree.
  `Submitted::conversion` records what was done. With no converter, no format in common, or an
  unsupported source the call fails with `ErrorKind::UnsupportedFormat` and a message that lists the
  document's format and what the printer accepts (and what the converter can make); a converter
  failure is reported as it was, marked `Phase::Prepare` and `RetrySafety::Safe` (nothing was
  sent). A network backend is not contacted for any of this without a `UserConfirmed`.
* **Cost.** For a backend with `formats_per_printer()` (WS-Print) and a converter registered, the
  decision needs the printer's list: one read-only `printer_details` call per print. An app that
  has just shown the printer's details passes them to `Registry::submit_job_with_details` and no
  extra request is made. A backend with an exact list (the spooler) costs nothing.
* **Backend-wide settings stay backend-wide.** The ticket shape, strict mode, transfer and limits
  of a network backend are configuration of the backend, not of a job. An app that wants different
  ones for one print builds a backend for that print (`backend.clone().with_*`, clones share the
  printer list) and registers it in a registry of its own, as pdfcraft does for each invocation.
* **Consent.** `UserConfirmed` is a marker with no policy of its own (`attest(Option<bool>)`
  errors unless `Some(true)`; `interactive()` for a Print button the person just pressed). Local
  backends (`BackendClass::Local`, the default) ignore it; a `BackendClass::Network` backend only
  ever receives requests that carry one, which the registry enforces.

### Job lifecycle

Submitting and watching are separate. `Registry::submit_job` returns a `Submitted` as soon as the
printer has **accepted** the document (a bounded call; it never waits for the paper). What happens
next is asked for, one request at a time, with the job's id:

```rust,no_run
use craft_print::{Document, Job, JobWatch, SubmitRequest, WaitPolicy, WatchStep};

let registry = craft_print::registry();
let job = Job { printer: Some("wsd:http://192.0.2.1:5357/p".into()), ..Job::default() };
let request = SubmitRequest::new(Document::pdf(&pdf_bytes), job).with_consent(craft_print::UserConfirmed::interactive());
let submitted = registry.submit_job(request)?;            // accepted; not printed yet
let Some(mut watch) = JobWatch::for_submitted(&submitted, WaitPolicy::default()) else { return Ok(()) };
// In a UI frame or a worker loop; each tick makes at most one request, and only when one is due:
match watch.tick(registry) {
    WatchStep::Pending { next_in } => { /* come back in next_in */ }
    WatchStep::Update(status) => { /* status.state, status.sheet_fraction() */ }
    WatchStep::Retrying { error, next_in, .. } => { /* a poll failed transiently */ }
    WatchStep::Done(outcome) => { /* Terminal, Forgotten, NoStatus, TimedOut or Failed */ }
    _ => {}
}
# Ok::<(), Box<dyn std::error::Error>>(())
```

* `Submitted` carries a typed `JobId` (`backend()` and `native()`; text form `"wsd:<printer>#7"` via
  `Display`/`FromStr`, `JobId::new(backend, native)` to build one, a typed `ParseIdError` for bad
  text, never a panic), `accepted_at`, warnings, a message, and what the backend can do afterwards
  (`watch: WatchSupport`, `poll_interval`). The job is `None` for a spooler that gives no handle.
* `WatchSupport::None` means the backend cannot report (a fire-and-forget spooler): the status is
  `JobState::Unknown`, "accepted, outcome unknown", which is not terminal and not a failure. Do not
  poll it. `Poll` means ask every `poll_interval`; `Events` is reserved (nothing listens yet).
* `JobStatus` has a `JobState` (Pending, Held, Processing, NeedsAttention, Completed,
  CompletedWithErrors, Cancelled, Aborted, Unknown), reasons, `warnings` (a completed job the printer
  qualified: "completed with warnings" is `Completed` plus this list), optional sheet and page counters
  and `terminal()`. A job the device has forgotten is `ErrorKind::NotFound`; a watch reports it as
  `WatchOutcome::Forgotten` (most likely finished).
  A `NotFound` says what is missing (`PrintError::subject()`): `Job` is forgotten as above, `Printer` (a wrong
  address or queue path) ends the watch `Failed` at once with no retry and no "most likely finished", and
  `Unknown` (a backend that could not tell) keeps the forgotten reading.
* `Registry::job_status(&JobId)` and `cancel_job(&JobId)` are single bounded requests routed by the
  id's backend. Keep the `JobId` (or its string and `.parse()` it later); do not build ids by hand.
* `JobWatch` is a state machine **you** advance: no thread, no timer. It has an interval that backs
  off while nothing changes, a `max_wait`, a `CancelToken` (one cancel is sent, then it keeps watching
  for the end state), failure retries under a `RetryPolicy`, and an injectable `Clock` (`ManualClock`
  in tests). `poll_status` forces a read now. `wait_for_job(registry, &job, WaitPolicy)` is the
  blocking convenience for a CLI, built on the same ticks; `max_wait` is finite (two minutes by
  default) and the job is left alone when it runs out.
* **Retry.** `PrintError::is_retryable()` says the condition is transient; `retry_safety()` says
  whether sending again is safe: `Safe` (nothing was accepted), `Idempotent` (a query or cancel) or
  `OutcomeUnknown` (the request may have arrived: reconcile before resubmitting). `phase()` and
  `retry_after()` complete it, and `RetryPolicy` (`delay`, `advise_error`, the `retry` helper) is a
  bounded exponential backoff with jitter; `advise` answers `CheckDeviceFirst` for an `OutcomeUnknown`
  error and nothing here repeats it. The neutral
  defaults per `ErrorKind` are tabulated in [docs/job-lifecycle.md](docs/job-lifecycle.md), which also
  has the survey of how CUPS/IPP, WS-Print and the Windows spooler separate accepting from printing.
* `Registry::submit` and `submit_to_backend` still return a `Receipt`; it now means "accepted".

### Tracked print jobs (`PrintJob`)

`submit_job` plus a `JobWatch` follows one remote job. A person pressed Print once and expects one
sheet, however many connections, retries and printer-side restarts that takes, so `PrintJob` is the
lifecycle record of **one user-visible print**, stable across retries. You advance it; it owns no thread,
timer or runtime, waits on an injectable `Clock` and keeps everything in memory.

```rust,no_run
use craft_print::{BytesSource, DocumentFormat, Job, PrintJob, UserConfirmed};

let registry = craft_print::registry();
let job = Job { printer: Some("wsd:http://192.0.2.1:5357/p".into()), title: "Quarterly report".into(), ..Job::default() };
let mut print = PrintJob::new(job, BytesSource::new(DocumentFormat::Pdf, pdf_bytes)).with_consent(UserConfirmed::interactive());
// In a worker loop or a frame loop. Each advance makes at most ONE bounded request, and only when one is due:
let step = print.advance(registry);          // step.next_in: when to come back; None: nothing happens on its own
let view = print.snapshot();                 // cheap and immutable: for the UI, every frame
// view.state, view.simple, view.progress, view.reasons, view.last_error, view.advice, view.attempts(), view.trail()
# Ok::<(), Box<dyn std::error::Error>>(())
```

* **Lifecycle** (separate from the printer's `JobState`): `Preparing`, `Submitting`, `Accepted`,
  `Watching(JobState)`, `RetryScheduled { attempt, next_at }`, `NeedsAttention` (the outcome of a send is
  unknown) and `Terminal(Completed | CompletedWithWarnings | Failed | Cancelled | Aborted | Abandoned)` (the lifecycle
  keeps `CompletedWithWarnings`; the printer's `JobState` does not; `view()` shows `Abandoned` as `craft_job::JobState::Abandoned`).
* **Retry.** Safe, transient failures are retried automatically under a `RetryPolicy` (on by default, at
  most 3 attempts, the device's `retry_after` honoured; `with_retry(None)` turns it off). An
  `OutcomeUnknown` failure is **never** retried automatically: the print waits in `NeedsAttention`, and
  `reconcile` asks the backend whether the earlier attempt created the job (`Found`, `Ambiguous`,
  `NotFound`); then `accept(job)` adopts it or `retry()` sends again. A printer-side abort or cancel is a
  manual `retry()` as well: a new attempt, a new remote `JobId`, the history kept.
* **`retry_advice()`** is the one question a UI has: `RetryNow`, `RetryAfter(duration)`,
  `CheckDeviceFirst` or `DoNotRetry`. `cancel()` works in every state.
* **`view()`** is the print as a `craft_job::JobView` (`PrintJob::view()`, `Snapshot::view()`), the same
  record a scan job fills: id (the printer's job id once accepted, a `local:print-<n>` id before),
  `JobKind::Print`, title, a `craft_job::JobState`, reasons (printer reasons, then the print's warnings),
  progress, wall-clock times, the document sent, the attempt count, the last error with its retry advice
  and native code, the tag and the printer id. No I/O. The field-by-field mapping is in
  [docs/job-tracking.md](docs/job-tracking.md) section 3.
* **The idempotency tag.** Each `PrintJob` makes a short random tag (`abc123de`) once and puts it on every
  attempt where the backend can carry it (`SubmitRequest::with_tag`, `Backend::tag_support()`: a
  job-name suffix `"title [ref abc123de]"`, a document name, a native attribute, or nothing), so that
  reconcile finds the job a lost reply created instead of guessing from the title. **On by default for
  network backends, off for the local spooler**; `TagPolicy::{Auto, Always, Never}` overrides it per job.
* **Document source.** The job keeps the document's length, format and a content hash, never the bytes:
  a `DocumentSource` (`BytesSource`, `FnSource`) supplies them again for each attempt.
* **Common base for other job sources.** `RemoteJob { id, title, user, created, state, reasons, ours, .. }`,
  `JobFilter`, `Fingerprint`, `Reconciled` and `JobQueryCaps` are backend-neutral; a backend opts in with
  `Backend::list_jobs`, `find_job` (defaults to the former), `tag_support` and `job_query_caps`, and the
  registry routes `Registry::list_jobs` and `find_job` by printer id. Foreign jobs are listed too
  (`ours == false`). The WS-Print backend implements them; the built-in CUPS and Windows-spooler adapters
  answer `Unsupported` for `list_jobs` in this version. The mapping of CUPS, IPP, the Windows spooler and
  WS-Print onto these types, and the reasoning, are in [docs/job-tracking.md](docs/job-tracking.md).

### Shared vocabulary: `craft-job`

The names that printing and scanning have in common live in the small `craft-job` crate and are
re-exported here, so `craft_print::JobId`, `NativeCode`, `RetrySafety`, `RetryPolicy`, `RetryAdvice`,
`Jitter`, `JobState`, `Progress`, `ExpectedTotals`, `Tag`, `TagSupport`, `TagPlacement`, `Fingerprint`
and `JobView` keep resolving. What stays here: `PrintJob` and its lifecycle, `Attempt` and the trail,
the reconcile engine, `RemoteJob`/`JobFilter`/`JobQueryCaps`, `ErrorKind`/`PrintError`, `Clock`. The
domain additions to the shared types are extension traits, re-exported from the crate root and
imported where used: `TagExt` (`Tag::random()`, `apply_to`, `strip`, `find_in`), `TagSupportExt`
(`carries`, `matchable`), `FingerprintExt` (`since`, `filter`), `ExpectedTotalsExt` (`for_job`,
`apply`) and `JitterExt` (`from_clock`).

Renamed relative to the earlier drafts of this crate: `JobTag` is `Tag` (and is `Copy`),
`ParseJobIdError` is `ParseIdError`, `RetryAdvice::CheckPrinterFirst` is `CheckDeviceFirst`,
`JobState::Stopped` is `NeedsAttention`, `JobState::CompletedWithWarnings` is gone (see above),
`TagPlacement::{JobName, Native}` are `{JobNameSuffix, NativeAttribute}`, `TagSupport` is an enum
(`None`, `Carried(placement)`, `Partial(placement)`) instead of a struct with `matchable`, and
`RetryPolicy::next_delay` is replaced by `advise` / `advise_error`.

### Per-job options and totals

* `SubmitRequest::with_options(JobOptions)` carries per-job **hints** beside the spooler's own `Job`:
  `media` (a PWG token such as `iso_a4_210x297mm`), `orientation`, `quality`, `resolution`
  (`Resolution::new(x_dpi, y_dpi)`), an explicit `colour` (`ColourMode::Auto`, `Colour`, `Mono`; it
  overrides `Job::grayscale` unless `Auto`) and `expected_pages`. Build with `JobOptions::new()` and
  the `with_*` methods; the struct is `#[non_exhaustive]`. A backend applies what it can;
  `PrinterDetails::honoured` lists the `JobHint`s it applies (`Media`, `Orientation`, `Quality`,
  `Resolution` and `Colour` joined `Copies`, ...). The built-in spooler honours the colour choice only.
  `Quality` is `Draft`, `Normal`, `High` or `Photo` (hint only; `Quality::from_name`/`ALL`). A backend
  maps it to the printer's own setting when the printer offers a matching value and drops it with a
  warning otherwise.
* `Registry::submit_job` accepts them, so an app needs no backend-specific entry point. Settings that
  are backend-wide configuration (a network backend's ticket shape, strict mode, transfer) stay on
  the backend: an app that wants different ones per invocation builds a backend for that invocation
  (or its own `Registry`) with them.
* **Totals are optional.** `Progress { done, total: Option<u32> }` is what the progress callback gets
  (`fraction()` is `None` while the total is unknown), and `JobStatus`'s counters were already
  optional. `expected_pages` is the page count the app knows: the registry turns it into an
  estimated sheet total (halved for two-sided jobs, times the copies; `Submitted::expected`) and
  uses it as the total of progress events and, through `JobWatch::for_submitted` (or
  `JobWatch::with_expected`), of status readings, **only where the backend reports none**; a
  backend-reported total always wins. `JobStatus::sheet_fraction()` is then determinate.

### Writing a backend (the extension points for a network backend)

* The trait is object safe and **blocking but bounded**: `Backend: Send + Sync`, stored as
  `Arc<dyn Backend>`. There is no async runtime here; a network backend blocks on its own transport
  for at most its per-request timeouts, never for the length of a print (see "Job lifecycle").
  Cancellation is a cloneable `CancelToken` in the `SubmitRequest` that the backend polls between
  round trips (the registry also checks it before calling); progress is a callback of
  `Progress { done, total: Option<u32> }` events (see "Per-job options and totals").
* Implement `name`, `list_printers`, `printer_details` and `submit` (which returns `Submitted` once the
  printer has accepted the document). Override `watch_support`, `job_status` and `cancel_job` if the
  protocol can tell what happened to a job afterwards; the defaults say "accepted, outcome unknown".
  Override `class` (network backends return `BackendClass::Network`) and `accepted_formats` (for
  example raster or the printer's own language) as needed. Use native ids; the registry adds the
  prefix, to printers and to jobs. A job is `JobId::new(self.name(), native)`.
* Register once at start-up: `craft_print::register(Box::new(MyBackend::new()))?`, or on a
  `Registry` you own. A backend that needs extra dependencies can live behind a cargo feature
  that registers it; the base crate is complete without it.

`tests/registry.rs` drives the public API with `craft_print::testing::FakeBackend` configured as a
CUPS-like backend (options, no status), an IPP-like one (state up front), a WSD-like one (formats
and a consent requirement) and one that always fails: aggregation, routing, partial failure,
duplicate registration, unknown prefix, consent and format refusals.

### Testing your app (cargo feature `testing`)

`testing` is off by default, has no effect on a default build and adds no dependency of its own
(with `wsd` it turns on `craft-wsd`'s own `testing` feature, which is a loopback server). Enable
it in a dev-dependency only, never in a shipped binary:

```toml
[dev-dependencies]
craft-print = { version = "0.1", features = ["testing"] }
```

* `craft_print::testing::FakeBackend` is a configurable `Backend` for a `Registry` of your own:
  printers, details (including `with_printer_formats`, `with_resolutions`,
  `with_formats_per_printer`; `detail_calls()` counts the reads), accepted formats, class (network
  needs consent), progress events, watch
  support, a scripted `job_status` (the last answer repeats), failing list/details/submit, and a job
  handle of your choosing. Clones share state, so register one and keep one:
  `submissions()` (printer, format, bytes, the `Job`, the `JobOptions`, whether consent came),
  `status_calls()` and `cancels()` say what your app did.
* `FakeBackend::cups_like(name)` (local, no status, no tag support, no job listing) and
  `FakeBackend::ipp_like(name)` (network, pollable, a job listing with creation time, user and history,
  the tag in a native attribute) are presets for the two shapes of backend a `PrintJob` meets;
  `with_submit_script(vec![SubmitStep::LoseReply(error), ..])` makes a send whose reply is lost (the job
  exists, the call fails), `seed_job` adds a foreign job and `remote_jobs()` shows the fake's queue.
* `craft_print::wsd::testing` (needs `wsd` too) is an in-memory WS-Print printer (there is no IPP equivalent: the fake IPP printer is fakeprint):
  `FakePrinter::new(FakeConfig { .. })` (formats, sides, copies, colour, media, qualities,
  resolutions, how jobs end
  with `Ending`, faults, HTTP errors), `MemTransport(printer)` for
  `WsdBackend::with_transport` (no sockets), `printer_server(printer)` for a loopback HTTP server,
  and the stored `FakeJob`s (ticket and document) and the request log (`actions()`) to assert on.
  The crate's own tests use exactly these types.

## IPP backend (cargo feature `ipp`)

`craft_print::ipp::IppBackend` prints to network printers over the Internet Printing Protocol
(RFC 8010/8011, PWG 5100.x) on plain HTTP. It is a `BackendClass::Network` backend registered as `ipp`;
its printers are `ipp:<printer uri>` and its jobs `ipp:<printer uri>#<job id>` (typed:
`IppBackend::job_id(uri, n)` and `parse_job_id`).

```no_run
use craft_print::ipp::IppBackend;
use craft_print::{Document, Job, SubmitRequest, UserConfirmed};

let ipp = IppBackend::new();
ipp.add_named("ipp://10.0.0.30/ipp/print", "Front office")?; // the app chose this printer; port 631
craft_print::register(Box::new(ipp.clone()))?;
let job = Job { printer: Some("ipp:ipp://10.0.0.30:631/ipp/print".into()), ..Job::default() };
let request = SubmitRequest::new(Document::pdf(b"%PDF-1.7"), job).with_consent(UserConfirmed::interactive());
let submitted = craft_print::submit_job(request)?; // returns when the printer has accepted the job
# Ok::<(), Box<dyn std::error::Error>>(())
```

* **Safety stance.** As for `wsd`: nothing discovers or connects until the app adds a printer and asks;
  every job needs a `UserConfirmed`; a request can only reach an added printer. URIs are `ipp://` or
  `http://` with a numeric loopback, link-local or private address (port 631 unless named); host names,
  public addresses, redirects and proxies are refused (`craft-wsd`'s policy), and `ipps://` and
  `https://` are refused with a clear error because there is no TLS. Replies are read by a strict,
  bounded codec (size, group, attribute, value and nesting caps). `ipp/adapter.rs` is the only file that
  names `craft-wsd`.
* **Operations.** `Get-Printer-Attributes`, `Validate-Job` (optional, `with_validate_first`), `Print-Job`,
  `Create-Job` + `Send-Document` (large documents, or printers without `Print-Job`; `JobFlow`),
  `Get-Job-Attributes`, `Get-Jobs` and `Cancel-Job`. `submit` returns once the printer has accepted the job;
  following it is `job_status` (one `Get-Job-Attributes`), polled every 2 s by default.
* **Settings.** Copies, two-sided, orientation, quality (draft/normal/high; photo is high plus the
  printer's `photo` content optimisation where it lists one), resolution, media and colour become job
  attributes. A setting the printer lists as unsupported is dropped with a warning in `Submitted::warnings`, or
  refused in strict mode (`with_strict`, which also sends `ipp-attribute-fidelity`); attributes the printer
  itself ignores come back as warnings too. The tables both ways are in [docs/ipp.md](docs/ipp.md).
* **Tracking.** The idempotency tag is a suffix of `job-name` (the title is sent with control characters removed); `list_jobs` / `find_job` read `Get-Jobs`
  (title, user, state, creation time), so a lost reply is reconciled by tag before anything is resent.
* **Errors** carry `NativeCode { domain: "ipp", code, name }` (HTTP failures `"http"`) and the retry facts of
  the status table in [docs/ipp.md](docs/ipp.md), pinned by the table tests in `ipp::error`.
* **IPP version.** Requests go out as IPP 2.0 (every IPP Everywhere printer speaks it). A printer that
  answers `server-error-version-not-supported` gets the same request once more at 1.1 (a rejected request
  creates nothing, so this is safe for `Print-Job`, `Create-Job` and `Send-Document` too), and a printer that
  answers a request at 1.1 with any status other than version-not-supported is remembered as speaking 1.1, for the life
  of the backend (clones share it). The printer's own
  `ipp-versions-supported` is used when it was read (highest of 2.0 and 1.1). `with_ipp_version` pins a version
  that is never downgraded unless the app also calls `with_version_fallback(true)`; `version_of(uri)` says what
  the next request to a printer will use.
* **Not found.** When a job read or cancel gets `client-error-not-found`, one `Get-Printer-Attributes`
  (`printer-state`) tells a forgotten job (the printer answers) from a printer at a wrong address or path (it
  answers not-found too: `ErrorKind::NotFound` saying the printer was not found) or "could not tell" (the extra
  read failed: `Failed`, `Idempotent`). Never repeated; see [docs/ipp.md](docs/ipp.md).
* **Not implemented:** TLS, authentication, IPP subscriptions (events) and discovery.
* **Tests.** Printer-free: the codec on static byte vectors (round trips, truncations, hostile input), request
  builders against expected bytes, canned replies through a small scripted transport (a mock transport,
  not a printer), the status and retry tables, and registry-level tests (routing, consent, format
  negotiation, conversion, tracking and reconcile). The fake printer is fakeprint; protocol-level
  verification against it is done through the Python bindings in a separate harness (outside this repo).
  There is deliberately no in-memory IPP printer in this crate.

## WS-Print backend (cargo feature `wsd`)

`craft_print::wsd::WsdBackend` prints to network printers that speak the WS-Print service
(Web Services for Devices). It is a `BackendClass::Network` backend registered as `wsd`; its
printers are `wsd:<url>`.

```rust,no_run
use craft_print::wsd::WsdBackend;
use craft_print::{Document, Job, SubmitRequest, UserConfirmed};

let wsd = WsdBackend::new();
wsd.add_named("http://10.0.0.20:5357/print", "Front office")?; // the app chose this printer
craft_print::register(Box::new(wsd.clone()))?;               // clones share the printer list

let details = craft_print::printer_details("wsd:http://10.0.0.20:5357/print")?; // one round trip
let job = Job { printer: Some("wsd:http://10.0.0.20:5357/print".into()), ..Job::default() };
let request = SubmitRequest::new(Document::pdf(&pdf_bytes), job).with_consent(UserConfirmed::interactive());
let submitted = craft_print::submit_job(request)?;   // returns when the printer has accepted the document
let job = submitted.job.expect("WS-Print gives every accepted job a handle"); // "wsd:<url>#<job id>"
let status = craft_print::job_status(&job)?;          // one GetJobElements; or follow it with a JobWatch
# Ok::<(), Box<dyn std::error::Error>>(())
```

| Feature | Default | Adds | Notes |
| --- | --- | --- | --- |
| `ipp` | on | `craft-wsd` (the same git dependency as `wsd`; its plain-HTTP transport and URL policy) and nothing else: the IPP codec is this crate's own | Not built for `wasm32`. No TLS (`ipps://` is refused), no authentication, no discovery. See [docs/ipp.md](docs/ipp.md). |
| `wsd` | on | `craft-wsd` (git, see below) and `roxmltree`; through `craft-wsd`: `thiserror` and `ureq` (plain HTTP, no TLS, no proxy, no redirects) | Not built for `wasm32`. `--no-default-features` is the CUPS/Windows crate with none of them. |

* **Safety stance.** Nothing in this feature discovers or connects until the app adds a printer.
  The backend lists only printers the app added (`add`, `add_named`, `with_printers`); it does no
  network discovery and sends no multicast (finding printers is a separate decision). Making a
  backend, adding a printer and listing open no connection; a connection is made by
  `printer_details` (one `GetPrinterElements`) and by a job. Every job needs consent: the
  registry refuses a request without a `UserConfirmed` (`ErrorKind::ConsentRequired`), and the
  backend checks again if it is called directly. A request can only reach a printer that was added.
  URLs are `http://` with a numeric loopback, link-local or private address; host names, public
  addresses, redirects and proxies are refused (`craft-wsd`'s policy). Replies, lists and XML are
  size-capped and every reply is checked (action, relation to the request, namespaces).
* **Printer details** map from `GetPrinterElements`: accepted formats (every MIME type the printer
  lists that has a `DocumentFormat`: PDF, PWG raster, PostScript, octet-stream, JPEG, PNG, TIFF,
  plain text, XPS and Open XPS, in the printer's order; other formats are not named), its
  **resolutions** (`PrinterDetails::resolutions`, from the same request, so choosing a raster dpi
  needs no extra read), two-sided, colour, paper names, maximum copies, status (idle, processing, stopped, with the reasons the
  printer gives) and the hints it applies (copies, duplex, colour, and the per-job media size, orientation, quality and resolution). Unlisted
  dimensions are `None` or empty, not "unsupported".
* **Formats and conversion.** `formats_per_printer()` is `true`: with an app `DocumentConverter`
  registered the registry reads the printer's list (one read-only `GetPrinterElements`, which is
  skipped when the app passes the details it has to `Registry::submit_job_with_details`) and, when
  the printer does not list the document's format, converts to the best format it does list at the
  resolution nearest 300 dpi from its own list (see "Formats and conversion" above). Without a
  converter nothing is read first: the backend answers `UnsupportedFormat` itself. A document in a
  format the printer lists goes through unchanged, whatever the format (JPEG, PNG, TIFF, text, XPS,
  Open XPS, PostScript and raster are sent as they are; the printer's spelling of the MIME type is
  used). `wsd::pwg` re-exports the neutral `craft_print::pwg` encoder; `wsd::DocumentEncoder`,
  `RasterEncoder` and `negotiate_format` are unchanged for apps that call them directly.
* **Quality.** `Quality::Photo` is sent as `PrintQuality` `Photo` when the printer lists it (or
  lists no qualities), left out with a warning when it lists qualities without it, and refused in
  strict mode, like any other setting; `Draft`, `Normal` and `High` follow the same rule.
* **Per-invocation settings** (ticket shape, strict mode, transfer, user name, limits) are
  backend-wide: build `backend.clone().with_*` for the invocation (clones share the printers).
* **Jobs.** A document is bytes in a `DocumentFormat` the printer lists (case-insensitively; the
  printer's spelling is sent), otherwise `ErrorKind::UnsupportedFormat` before a job exists. The
  job is created and the document sent as an MTOM part (or inline base64, `with_transfer`);
  **`submit` returns when the printer has accepted the document** (the reply to `SendDocument`),
  after at most capabilities, create, send (and a cleanup cancel when the printer refused the
  document), each bounded by the request timeout. It never waits for the print. The accepted job
  is `Submitted::job`, a `JobId` whose text form is `wsd:<printer url>#<job id>`. Copies, two-sided
  and grey come from the `Job`; media size, orientation, quality, resolution and an explicit
  colour choice come from the request's `JobOptions` and go into that job's ticket (the printer's
  capability list still decides: an unlisted value is dropped with a warning, or refused in strict
  mode; a reduced ticket shape refuses or drops them as before); collation, scaling and driver options have no WS-Print setting (the PrintTicket schema has no scaling element, so none is invented; shrink-to-fit on WS-Print needs a `DocumentConverter` that rescales to the page size, which `ConvertContext` does not yet carry and which is never called for a printer that takes PDF: a follow-up) and are reported as
  warnings in `Submitted::warnings`. Progress events are `Progress::of(done, 3)` protocol phases (capabilities read,
  job created, document accepted), not sheets; the sheets come from `job_status`. The cancel token is polled between round trips;
  cancelling before the document is sent cancels the created job and returns
  `ErrorKind::Cancelled`. A failed send whose outcome is unknown (the connection closed or timed
  out after the document was written) leaves the created job alone: cancelling could abort a job
  that is printing.
* **Following a job.** `watch_support()` is `Poll` (push status is opt-in, see "Push status" below; nothing listens unless the app asks) and
  `recommended_poll_interval()` is two seconds (`with_poll_interval`). `job_status` is one
  `GetJobElements`: the nine WS-Print job states map to `Pending`, `Held`, `Processing` (also
  `Started` and `Terminating`), `NeedsAttention` (a stopped job), `Completed` (with
  `JobStatus::warnings` when the reasons say `JobCompletedWithWarnings`),
  `CompletedWithErrors` (from the state reasons), `Cancelled` and `Aborted` (`Other` is `Unknown`),
  with the state reasons and `sheets_done` (`MediaSheetsCompleted`; WS-Print does not give a total).
  A job the printer does not list is `ErrorKind::NotFound` whose message says printers forget finished
  jobs, so it most likely finished; a watch reports it as `Forgotten`. `cancel_job` is `CancelJob`,
  treating "too late" as success (`wsd::job::cancel_job_tolerant`). A job handle must name an added
  printer; anything else is `ErrorKind::InvalidRequest`, never `NotFound`. Apps never build the
  `wsd:<url>#<n>` string: `WsdBackend::job_id(printer_url, number)` makes the `JobId` and
  `WsdBackend::parse_job_id` / `parse_native` read it back (`WsdJob { printer_url, job_number }`).
  `with_expected` / `Submitted::expected` give the sheet counter a total (WS-Print reports none)
  from `JobOptions::expected_pages`.
* **Reconcile.** After an error with `retry_safety() == OutcomeUnknown` (for example the connection
  closed with no reply to `SendDocument`), `WsdBackend::reconcile(printer, title)` reads
  `GetActiveJobs` and `GetJobHistory` (reads only) and returns the jobs with that title from this
  backend's user: `Found`, `Ambiguous` (newest first) or `NotFound`. Job summaries have no creation
  time, so the window is what the printer still lists. A job created without a document shows as
  pending and can be cancelled through the registry.
* **Ticket shapes.** Some printers' firmware does not answer tickets containing `JobProcessing` or
  `DocumentProcessing`. `with_ticket_shape(TicketShape::Description | JobNameOnly | Empty)` sends a
  smaller ticket (`Full`, the default, writes everything set). A smaller ticket has no room for
  copies, two-sided and the like: such a job is refused before anything is sent
  (`ErrorKind::InvalidRequest`) unless `with_drop_ticket_settings(true)` drops them with a
  warning each. `with_strict(true)` refuses settings the printer lists as unsupported and sends
  every setting as `MustHonor`; otherwise unsupported settings are lowered or left out, with a warning.
* **Per invocation.** Ticket shape, strict mode, transfer, user name and limits are backend-wide.
  To use others for one invocation, build a variant: `backend.clone().with_ticket_shape(..)`
  shares the printer list but not the configuration (register it in a `Registry` of its own, or
  call `Backend::submit` on it, which checks consent itself). Everything else is per job.
* **Errors** are neutral `PrintError`s. Faults carry `NativeCode { domain: "wsd-fault", code,
  name }` (codes 1 to 16 in the order of the service description, 100 for
  `ActionNotSupported`, 0 for a vendor code) and temporary refusals (not accepting jobs,
  temporary error, document active) are `ErrorKind::Busy` with a suggested `retry_after`. Beyond
  the kind, every error says whether the condition is transient (`is_retryable`), whether sending
  again is safe (`retry_safety`) and where it failed (`phase`). The rules, in short: a failure that
  proves the printer did not act is `Safe` (policy, connect failure, 408, 425, 429, 503, any SOAP
  fault); a failure after the request may have arrived is `OutcomeUnknown` for a create or a send
  (connection closed or reset, timeout waiting for the reply, 500, 502, 504, an unreadable reply)
  and `Idempotent` for a query or a cancel. A refused redirect, a refused address, 501, 505 and
  other 4xx are not retryable. The full table, per error kind and operation, is in
  [docs/job-lifecycle.md](docs/job-lifecycle.md) and pinned by the table tests in `wsd::error`.
  `craft-wsd` reports transport failures as a typed kind and stage (connect failures, timeouts,
  redirects, a closed connection), so nothing is read from text; a `Retry-After` the printer sends
  becomes the error's `retry_after`.
* **Push status (opt-in).** `WsdBackend::with_events(NotifyConfig)` starts one inbound listener and
  subscribes (WS-Eventing) to job and printer events, so a watch can poll *now* instead of waiting.
  **Nothing listens unless you call it**, and there is no default address: `NotifyConfig::new(bind,
  advertise)` takes the socket to listen on and the numeric local URL the printers are told
  (`NotifyConfig::loopback()` is for tests). `bind` must be one concrete local interface address (loopback,
  link-local or private): `0.0.0.0`, `::` and the v4-mapped wildcard are refused with a typed error before
  any socket is opened. `advertise` must be that same address, or a private address the app names for NAT. Events are only a **wake-up**: the state of a job always
  comes from `job_status`, so a lost, duplicated or forged event can cost a request or some latency
  and can never produce a wrong terminal state. While a subscription is healthy (a verified event
  arrived recently) a `JobWatch` polls every 30 seconds as a safety net plus once per wake; when it
  is not (subscribe failed, renewal failed, `SubscriptionEnd`, silence) it polls as before.
  The listener accepts only connections from a subscribed printer's address, requires an
  unguessable per-subscription path and `wse:Identifier`, caps sizes, time and rate, and answers a
  bare status. Housekeeping (`Subscribe`, `Renew`, `Unsubscribe`) is caller-driven on an injectable
  clock: `Backend::job_status` does it, or call `maintain_events()`; `shutdown_events()` (or dropping
  the last clone) unsubscribes, stops and joins the listener thread, the only background thread.
  `events_status()` returns plain data (listen address, per-printer health, counters). It needs the
  printer to reach the client: a firewall or NAT in between silently leaves you with polling. Design
  and exposure rules: [docs/events.md](docs/events.md). Verified without a printer and against
  `fakeprint` through a separate harness; not yet against real printers.
* **Lower layers** are public for apps that need them: `wsd::client::Client` (one method per
  operation: `GetPrinterElements`, `CreatePrintJob`, `SendDocument`, `GetJobElements`,
  `GetActiveJobs`, `GetJobHistory`, `CancelJob`), `wsd::job::submit_document` (the accepted
  half of a print) and `wsd::job::print_document` (accept, then a blocking watch) with a
  `DocumentEncoder` (`PassThrough`, `Chain`, `RasterEncoder` over a caller-supplied page renderer
  that writes PWG raster) and `cancel_job_tolerant`. `wsd::adapter` is the only file that names
  `craft-wsd`.
* **Dependency.** `craft-wsd` is a workspace crate (`crates/wsd`); nothing else is needed.
* **Tests.** `tests/wsd_registry.rs` (through the registry, on `wsd::testing`: listing, policy, consent, format
  refusal, ticket shapes, accepted-not-printed submits, watching on an injected clock, cancellation,
  progress, outcomes, faults, retries, reconcile after a lost reply, and hostile replies from
  loopback servers: huge, malformed, wrong namespace, HTML 5xx, redirects, silence, a closed port,
  connection closed), `tests/wsd_client.rs` (the client layer against an in-memory fake printer) and the unit
  tests of each module. `tests/wsd_live.rs` runs only when `FAKEPRINT_URL` names a loopback
  WS-Print service of a test server; it is skipped otherwise.

- **macOS/Linux:** CUPS `lpstat`, `lpoptions` and `lp`. Jobs stream through stdin. Exact-size
  sheets use `fit-to-page=false`; shrink-to-fit uses IPP `print-scaling=auto-fit`. The IPP backend sends `print-scaling` (`auto-fit` for shrink-to-fit, `none` for actual size when the printer's default differs) only to a printer whose `print-scaling-supported` lists it, and then lists `JobHint::Scaling` as honoured; otherwise shrink-to-fit is a warning and the document goes as it is.
- **Windows:** in this process, with no PowerShell, script or other program started, so
  printing works where App Control for Business or AppLocker enforce script rules. Printers
  come from `EnumPrinters` (local queues and `\\server\queue` connections) and the default from
  `GetDefaultPrinter`. A job is drawn by the in-box `Windows.Data.Pdf` with Direct2D
  into the XPS print API (the `winprint` crate), and copies, collation, duplex, colour, paper,
  orientation and "no scaling" travel in a PrintTicket that the driver validates first; a
  driver that would change the paper, orientation or scaling is refused before anything is
  sent. Paper is the driver's own option for the sheet's size (to 0.25 mm), preferring the one
  named like the standard size ("ARCH D" over a custom 24 × 36 in entry). This backend comes
  from storytold/pdfcraft#599 and #1132.
  **Resolution.** Vector content stays vector. `Windows.Data.Pdf` draws a page's images at the
  page's destination size, by default 96 dpi whatever the images in the PDF are. The spooler
  backend gives each page a destination at the printer's own resolution instead (read from its
  device context with the ticket applied, else 300 dpi; no ceiling; a page whose bitmap would pass
  256 MiB or 16384 px is drawn in bands at the same resolution, never lowered), so an image reaches the driver at
  its own resolution, up to the renderer's limit of about 4 million pixels per image (an A4
  page's image arrives at most at about 211 dpi; measured, and tiling does not lift it). A request's `JobOptions::resolution` is a requirement (the higher axis, at most 2400
  dpi); `JobOptions::quality` is a wish that never goes above the printer's resolution (draft
  150 dpi, normal 300, high 600, photo 1200); a resolution wins over a quality. Both are in
  `honoured` on Windows. A difference between what was asked for and what was drawn, a hint
  that was merged, lowered or ignored, and the media and orientation hints (the job's pages
  decide those) are `Submitted::warnings`; CUPS also warns for the four hints it leaves to the
  driver's own options. The driver may still re-encode the bitmap (the Microsoft class driver
  stores JPEG q90): that is not controllable from here.
  A job has one ticket, so one paper and orientation. Sheets of the other orientation are
  turned back with `/Rotate`, so portrait and landscape sheets still print as one job. Exact
  size refuses sheets of different sizes and paper the driver lacks. Shrink-to-fit uses the
  driver's matching paper, else its default paper, and puts every sheet on it: a sheet that
  doesn't fit inside the paper's widest hardware margin is shrunk, a smaller one keeps its
  size, and both are centred. Both are done by appending an incremental update to the PDF
  (new versions of the pages that need it, the original bytes untouched); the crate reads
  only cross-reference sections, object streams and the page tree for this, leniently and
  with every size bounded. Encrypted PDFs can be turned but not reframed. Annotations that print
  nothing (Link, Popup, Hidden, or without the Print flag) are dropped from a reframed page; a
  printable one is refused, so print-ready PDFs have those flattened. Nondefault or invalid
  `/UserUnit` is refused because the native renderer does not reliably honor physical units.
  Parsing caps cumulative cross-reference/object-stream records at 500,000, decoded streams
  at 16 MiB each and 32 MiB per job, and parsed nodes/tokens at 250,000/16 MiB per job and
  32,000/1 MiB per object. Files exceeding these limits return a clear error.
  Long-edge duplex is `TwoSidedLongEdge` and short-edge `TwoSidedShortEdge`; `duplex: None`
  leaves duplex out of the ticket. Driver-specific defaults remain in Windows printer
  preferences (`rundll32 printui.dll`, the one process this crate starts, and only when asked).
- **Browser/other targets:** discovery is empty and submission reports an unsupported
  platform; callers can offer PDF export instead.

Windows jobs briefly use a file in a private temporary folder (random name, this user's only),
named after the job's title because the spooler shows it as the document name; the folder is
removed once the spooler has the job, on success and failure. Failed and canceled terminal
package statuses are errors, including failures received after every sheet was processed.
A missing completion notification times out after two minutes after the native print control
is closed and requests cancellation. Calls into a driver and interactive output-file dialogs
are not covered by this notification timeout. Windows answers when the job is in
the queue ("spooled 3 sheet(s)"): a paused queue or an offline printer keeps it there. CUPS jobs
never use a PDF temporary file. Printing is synchronous; UI callers should submit on a worker.
`submit_with_progress` reports `(processed, sheets)` before the first Windows sheet and after
each sheet is successfully added to the native print job. These are source PDF sheets, excluding
copies and duplex; the events describe sheet processing, and do not confirm physical output.
The callback runs on the thread calling submission, can capture non-`Send` state, and should
return promptly. It is never retained. Keep waiting for the final `Result` even at full progress:
the job can still fail. The API does not provide cancellation. CUPS reports no progress.
UI callers must forward events to their own progress state; adopting the API alone does not
change an application's UI.
Receipts are backend messages, not portable job IDs. `Job::duplex = None` preserves the queue's
duplex default; `Some(Duplex::Off)` forces simplex. `Job::scaling` defaults to `Scaling::Exact`;
callers exporting arbitrary document sizes can choose `Scaling::ShrinkToFit`. Exact mode reports
a driver paper-size mismatch. On Windows `command_preview` describes the in-process job
(`xps-print`, then the printer and options as `key=value`) instead of a command line. The crate
does not provide PostScript output, separations, trapping or colour-managed device rendering.

Annotations are sorted into silent (they print nothing: Link or Popup, flagged Hidden, or
without the Print flag) and printable. Windows shrink-to-fit drops the annotations of a page
whose annotations are all silent (hyperlinks do not stop a document from being fitted) and
refuses a page with a printable one, naming its subtype, because its appearance lives outside
the page content and would stay at the old coordinates; callers flatten such annotations
first. Exact-size printing and rotation retain the annotations. The Windows renderer uses
viewer visibility (not Hidden, not NoView) and ignores the Print flag. Native submission
therefore accepts annotations that print nothing and draw nothing (Link and Popup, Hidden,
NoView without Print) and plain printable ones (`F=4` with the Print flag and no NoZoom or
NoRotate); an annotation the file says not to print but a viewer shows (no Print flag), one
with NoView and Print, or one with NoZoom/NoRotate is refused for caller flattening. This
prevents printing comments that were marked nonprinting or omitting print-only marks. A
visible Square appearance with `F=4` was rendered in the Windows qualification fixture; other
appearance types still need their own fidelity checks.

Native Windows PDF submission checks the driver's validated copy count and explicit
simplex/duplex and grayscale requests before sending. Collation must be retained when both
the page count and copy count exceed one. A driver that drops or changes these options is
refused; select supported options or flatten grayscale into the print-ready page content.
Without an explicit duplex request, the queue default applies. Normal color output retains
the driver's color policy. Single-page or single-copy jobs have no collating order to check.

`cargo test -p craft-print` tests CUPS transport (including pipe-sized jobs and refusal),
printer parsing, PPD options and argument mapping, and on every platform the Windows pieces that
need no printer: the PrintTicket, the checks on the driver's validated ticket and capabilities,
paper matching, job file names, and the PDF sheet reader and its update (classic and compressed
cross-references, turning, reframing, refusal, and thousands of truncated and corrupted files
that must fail without a panic). On Windows it also submits a mixed-orientation exact-size job
and a shrink-to-fit job to a temporary paused queue (inbox "Generic / Text Only" driver on
`LPT1:`) and checks each arrives as one job with all its sheets. The queue portion explicitly
skips if the driver or administrative rights are absent. Windows CI installs the inbox test
driver and sets `CRAFT_PRINT_REQUIRE_QUEUE=1`, making queue preparation/submission mandatory on
its runner. Two ignored tests check a real printer by hand: `an_installed_driver_validates_the_ticket`
(`CRAFT_PRINT_TEST_PRINTER`; sends no job) and `an_installed_printer_prints_a_file`
(`CRAFT_PRINT_TEST_PRINTER`, `CRAFT_PRINT_TEST_FILE`, optional `CRAFT_PRINT_TEST_SCALING=shrink`
and `CRAFT_PRINT_TEST_DUPLEX=long`). Physical printer fidelity remains a hardware check.

The cross-platform GitHub Actions workflow is defined in `.github/workflows/print.yml`.
It runs formatting, Clippy and this crate's own tests on Windows, Linux and macOS when print
code changes, and supports manual runs. The Windows job executes the spooler regression above;
physical printer fidelity still requires hardware verification.

Windows discovery reports driver duplex support in `Printer::two_sided` (a two-sided option in
the driver's PrintCapabilities); CUPS leaves it unknown. Explicit simplex/duplex choices that the driver
drops are refused before submission.

Windows dependencies: the maintained [`winprint` fork](https://github.com/storytold/winprint.rs)
(BSD-3-Clause, pinned to an immutable Git revision, its PDFium-downloading `pdfium` feature off) and `winsafe` (MIT) hold every Win32 and WinRT call, so the crate keeps `unsafe` forbidden;
`roxmltree` reads PrintTickets and `miniz_oxide` inflates PDF streams. `winprint` 0.2.1 lists
reqwest (with rustls) and sigstore crates as non-optional build-dependencies, used only by its
`pdfium` feature: they compile into its build script on Windows but not into the library. Apps
running `cargo deny` need a licence exception for `webpki-root-certs` (CDLA-Permissive-2.0, CA
list data reqwest brings) and an advisory ignore for RUSTSEC-2024-0370 (`proc-macro-error`
unmaintained, a compile-time proc macro of winprint's `fmt-derive`).

`craft_print::rgb` exposes the safe `craft-winprint-gdi` API for native driver settings,
printable geometry, RGB pages and caller-composed vector strokes. Applications retain ICC
conversion, document layout, labels and print-mark composition. The shared transport validates
single-copy DEVMODE settings before emitting manual copies and checks GDI errors. Color policy
is explicit: application-managed/unmanaged RGB disables DC and old-style halftone correction
with `ICM_DONE_OUTSIDEDC`, reapplied after `StartPage`; driver-managed RGB uses `ICM_OFF`.
Vendor driver automatic color correction remains a driver setting and requires qualification.
Other platforms return `rgb::PrintError::Unsupported`. The Windows PDF backend and its existing
completion dependency pin are independent of this additional MIT OR Apache-2.0 API.
