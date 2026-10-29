//! The backend abstraction, driven only through the public API with test-only fake backends:
//! a CUPS-like one (driver options, no status), an IPP-like one (state up front), a WSD-like one
//! (several formats, consent required) and one that always fails.

use craft_print::testing::FakeBackend;
use craft_print::{
    BackendClass, CancelToken, ColourMode, Document, DocumentFormat, ErrorKind, Job, JobHint, JobOptions, NativeCode, Orientation, PrintError,
    PrinterDetails, PrinterOption, PrinterRef, PrinterState, PrinterStatus, Progress, Quality, RegisterError, Registry, Resolution, SubmitRequest,
    UserConfirmed, namespaced_id, split_id,
};

/// Options and driver keywords, no status, PDF only, local.
fn cups_like() -> FakeBackend {
    FakeBackend::new("cups")
        .with_printer("Office", true)
        .with_printer("Label:Writer", false)
        .with_details(PrinterDetails {
            formats: vec![DocumentFormat::Pdf],
            options: vec![PrinterOption {
                key: "InputSlot".into(),
                label: "Tray".into(),
                group: String::new(),
                choices: vec![("Tray1".into(), "Tray 1".into())],
                default: "Tray1".into(),
            }],
            honoured: Some(vec![JobHint::Copies, JobHint::Duplex, JobHint::DriverOptions]),
            status: None,
            ..PrinterDetails::default()
        })
        .with_message("request id is Office-1")
        .with_job_handle(|_, _| Some("Office-1".into()))
}

/// Reports state up front, ignores duplex and colour, reports progress, hands out no job.
fn ipp_like() -> FakeBackend {
    FakeBackend::new("ipp")
        .with_printer_ref(PrinterRef { two_sided: Some(true), ..printer("net-1", false) })
        .with_details(PrinterDetails {
            formats: vec![DocumentFormat::Pdf, DocumentFormat::PwgRaster],
            honoured: Some(vec![JobHint::Copies]),
            status: Some(PrinterStatus { state: PrinterState::Stopped, reasons: vec!["paper empty".into()] }),
            ..PrinterDetails::default()
        })
        .with_progress_events(vec![Progress::of(0, 2), Progress::of(2, 2)])
        .with_job_handle(|_, _| None)
}

/// Network class: several formats, consent required, always busy.
fn wsd_like() -> FakeBackend {
    FakeBackend::new("wsd")
        .with_class(BackendClass::Network)
        .with_formats(vec![DocumentFormat::PwgRaster, DocumentFormat::OctetStream])
        .with_printer("device-a", false)
        .failing_submit(PrintError::new(ErrorKind::Busy, "the device is busy").with_native(NativeCode {
            domain: "wsd-fault",
            code: 7,
            name: Some("ServerErrorBusy"),
        }))
}

/// Lists nothing but errors.
fn broken() -> FakeBackend {
    let down = || PrintError::new(ErrorKind::Unavailable, "the service is down");
    FakeBackend::new("broken")
        .failing_list(down().with_native(NativeCode { domain: "win32", code: 1722, name: None }))
        .failing_details(down())
        .failing_submit(down())
}

fn printer(id: &str, default: bool) -> PrinterRef {
    PrinterRef { id: id.into(), name: format!("Printer {id}"), default, two_sided: None }
}

struct Fakes {
    cups: FakeBackend,
    wsd: FakeBackend,
}

type Seen = Vec<(Option<String>, DocumentFormat, usize)>;

impl Fakes {
    /// What the CUPS-like and the WSD-like backend were handed, in that order.
    fn seen(&self) -> Seen {
        self.cups.submissions().into_iter().chain(self.wsd.submissions()).map(|s| (s.printer, s.format, s.bytes.len())).collect()
    }
}

fn all() -> (Registry, Fakes) {
    let fakes = Fakes { cups: cups_like(), wsd: wsd_like() };
    let registry = Registry::new();
    registry.register(Box::new(fakes.cups.clone())).unwrap();
    registry.register(Box::new(ipp_like())).unwrap();
    registry.register(Box::new(fakes.wsd.clone())).unwrap();
    (registry, fakes)
}

fn job_for(id: Option<&str>) -> Job {
    Job { printer: id.map(str::to_string), ..Job::default() }
}

#[test]
fn an_empty_registry_is_unsupported() {
    let registry = Registry::new();
    assert!(!registry.supported());
    assert_eq!(registry.list_printers().unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(registry.printer_details("cups:a").unwrap_err().kind(), ErrorKind::Unsupported);
    let request = SubmitRequest::new(Document::pdf(b"%PDF"), Job::default());
    assert_eq!(registry.submit(request).unwrap_err().kind(), ErrorKind::Unsupported);
}

#[test]
fn listing_aggregates_every_backend_with_namespaced_ids() {
    let (registry, _) = all();
    assert_eq!(registry.backend_names(), ["cups", "ipp", "wsd"]);
    let listing = registry.list_printers().unwrap();
    assert!(listing.is_complete());
    let ids: Vec<&str> = listing.items.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["cups:Office", "cups:Label:Writer", "ipp:net-1", "wsd:device-a"]);
    assert_eq!(listing.items[2].two_sided, Some(true));
    assert_eq!(namespaced_id("cups", "Office"), "cups:Office");
    assert_eq!(split_id("cups:Label:Writer"), Some(("cups", "Label:Writer")), "only the first colon separates the prefix");
}

#[test]
fn a_failing_backend_does_not_hide_the_others() {
    let (registry, _) = all();
    registry.register(Box::new(broken())).unwrap();
    let listing = registry.list_printers().unwrap();
    assert!(!listing.is_complete());
    assert_eq!(listing.items.len(), 4, "the three working backends' printers are all there");
    assert_eq!(listing.errors.len(), 1);
    let failure = &listing.errors[0];
    assert_eq!(failure.backend, "broken");
    assert_eq!(failure.error.kind(), ErrorKind::Unavailable);
    assert!(failure.error.is_retryable());
    assert_eq!(failure.error.native().map(|n| (n.domain, n.code)), Some(("win32", 1722)));
}

#[test]
fn only_failing_backends_list_no_printers_and_report_each_error() {
    let registry = Registry::new();
    registry.register(Box::new(broken())).unwrap();
    let listing = registry.list_printers().unwrap();
    assert!(listing.items.is_empty());
    assert_eq!(listing.errors.len(), 1);
}

#[test]
fn registering_a_name_twice_is_an_error_value() {
    let (registry, _) = all();
    let err = registry.register(Box::new(ipp_like())).unwrap_err();
    assert_eq!(err, RegisterError::DuplicateName { name: "ipp" });
    assert!(err.to_string().contains("already registered"));
    assert_eq!(registry.backend_names().len(), 3, "the first registration stays");
}

#[test]
fn details_route_by_prefix_and_come_back_namespaced() {
    let (registry, _) = all();
    let cups = registry.printer_details("cups:Office").unwrap();
    assert_eq!(cups.id, "cups:Office");
    assert_eq!(cups.options.len(), 1);
    assert!(cups.status.is_none(), "a CUPS-like backend cannot say");
    assert_eq!(cups.honoured, Some(vec![JobHint::Copies, JobHint::Duplex, JobHint::DriverOptions]));
    let colon = registry.printer_details("cups:Label:Writer").unwrap();
    assert_eq!(colon.id, "cups:Label:Writer");

    let ipp = registry.printer_details("ipp:net-1").unwrap();
    assert_eq!(ipp.id, "ipp:net-1");
    assert_eq!(ipp.status.as_ref().map(|s| s.state), Some(PrinterState::Stopped));
    assert_eq!(ipp.formats, [DocumentFormat::Pdf, DocumentFormat::PwgRaster]);
    assert_eq!(ipp.honoured, Some(vec![JobHint::Copies]), "duplex and colour are hints this backend ignores");

    let unknown = registry.printer_details("cups:Nope").unwrap_err();
    assert_eq!(unknown.kind(), ErrorKind::NotFound);
}

#[test]
fn an_unknown_or_missing_prefix_names_the_registered_backends() {
    let (registry, _) = all();
    for id in ["lpd:Office", "Office"] {
        let err = registry.printer_details(id).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::UnknownBackend, "{id}");
        assert!(err.message().contains("cups, ipp, wsd"), "{}", err.message());
        let submit = registry.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(Some(id)))).unwrap_err();
        assert_eq!(submit.kind(), ErrorKind::UnknownBackend, "{id}");
    }
}

#[test]
fn a_job_routes_to_its_backend_with_the_native_id() {
    let (registry, fakes) = all();
    let receipt = registry.submit(SubmitRequest::new(Document::pdf(b"%PDF-1.7"), job_for(Some("cups:Label:Writer")))).unwrap();
    assert_eq!(receipt.backend, "cups");
    assert_eq!(receipt.job_id.as_deref(), Some("Office-1"));
    assert_eq!(fakes.seen(), [(Some("Label:Writer".to_string()), DocumentFormat::Pdf, 8)]);

    let mut events = Vec::new();
    let mut progress = |event: Progress| events.push((event.done, event.total));
    let request = SubmitRequest::new(Document::pdf(b"%PDF"), job_for(Some("ipp:net-1"))).with_progress(&mut progress);
    assert_eq!(registry.submit(request).unwrap().backend, "ipp");
    assert_eq!(events, [(0, Some(2)), (2, Some(2))]);
}

#[test]
fn no_printer_chosen_goes_to_the_backend_with_a_default() {
    let (registry, fakes) = all();
    registry.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(None))).unwrap();
    assert_eq!(fakes.seen()[0].0, None, "the backend gets no printer and picks its own default");

    let no_default = Registry::new();
    no_default.register(Box::new(ipp_like())).unwrap();
    no_default.register(Box::new(broken())).unwrap();
    let err = no_default.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(None))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::NotFound, "several backends and none lists a default");

    let single = Registry::new();
    single.register(Box::new(ipp_like())).unwrap();
    assert!(single.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(None))).is_ok(), "one backend takes the default job");
}

#[test]
fn a_format_the_backend_does_not_accept_is_refused_before_it_is_called() {
    let (registry, fakes) = all();
    let doc = Document::new(DocumentFormat::PostScript, b"%!PS");
    let err = registry.submit(SubmitRequest::new(doc, job_for(Some("cups:Office")))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFormat);
    assert!(err.message().contains("application/postscript"));
    let request = SubmitRequest::new(Document::pdf(b"x"), job_for(Some("wsd:device-a"))).with_consent(UserConfirmed::interactive());
    let err = registry.submit(request).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::UnsupportedFormat, "the WSD-like backend wants raster or its own language, not PDF");
    assert!(fakes.seen().is_empty(), "no backend saw a refused document");
}

#[test]
fn a_network_backend_needs_consent_and_a_local_one_does_not() {
    let (registry, fakes) = all();
    let raster = Document::new(DocumentFormat::PwgRaster, b"RaS2");

    let err = registry.submit(SubmitRequest::new(raster, job_for(Some("wsd:device-a")))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::ConsentRequired);
    assert!(fakes.seen().is_empty(), "the backend was not reached");

    let consent = UserConfirmed::attest(Some(true)).unwrap();
    let err = registry.submit(SubmitRequest::new(raster, job_for(Some("wsd:device-a"))).with_consent(consent)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Busy, "with consent the backend ran and its error came back");
    assert!(err.is_retryable());
    assert_eq!(err.native(), Some(NativeCode { domain: "wsd-fault", code: 7, name: Some("ServerErrorBusy") }));
    assert_eq!(fakes.seen().len(), 1);

    // A local backend does not need consent.
    assert!(registry.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(Some("cups:Office")))).is_ok());
}

#[test]
fn attest_wants_an_explicit_true() {
    assert!(UserConfirmed::attest(Some(true)).is_ok());
    for value in [None, Some(false)] {
        let err = UserConfirmed::attest(value).unwrap_err();
        assert_eq!(err.kind(), ErrorKind::ConsentRequired);
        assert!(err.message().contains("user_confirmed: true"), "{err}");
    }
}

#[test]
fn a_cancelled_token_stops_the_job_before_any_backend_runs() {
    let (registry, fakes) = all();
    let cancel = CancelToken::new();
    cancel.cancel();
    assert!(cancel.clone().is_cancelled(), "clones share the flag");
    let err = registry.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(Some("cups:Office"))).with_cancel(cancel)).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Cancelled);
    assert!(!err.is_retryable());
    assert!(fakes.seen().is_empty());
}

#[test]
fn a_backends_job_error_comes_back_unchanged() {
    let registry = Registry::new();
    registry.register(Box::new(broken())).unwrap();
    let err = registry.submit(SubmitRequest::new(Document::pdf(b"x"), job_for(Some("broken:any")))).unwrap_err();
    assert_eq!(err.kind(), ErrorKind::Unavailable);
}

#[test]
fn names_are_validated_and_kinds_say_whether_to_retry() {
    let registry = Registry::new();
    for bad in ["", "Upper", "has:colon", "sp ace"] {
        assert_eq!(registry.register(Box::new(FakeBackend::new(bad))).unwrap_err(), RegisterError::InvalidName { name: bad });
    }
    assert!(registry.register(Box::new(FakeBackend::new("my-backend_2"))).is_ok());
    for kind in [ErrorKind::Unavailable, ErrorKind::Busy, ErrorKind::Timeout] {
        assert!(kind.is_retryable());
    }
    for kind in [
        ErrorKind::Unsupported,
        ErrorKind::NotFound,
        ErrorKind::UnsupportedFormat,
        ErrorKind::ConsentRequired,
        ErrorKind::Rejected,
        ErrorKind::Failed,
    ] {
        assert!(!kind.is_retryable());
    }
}

#[cfg(any(windows, all(unix, not(target_arch = "wasm32"))))]
#[test]
fn the_process_wide_registry_holds_the_platform_backend() {
    let names = craft_print::registry().backend_names();
    assert_eq!(names, [if cfg!(windows) { "spooler" } else { "cups" }]);
    // The built-in name cannot be registered a second time.
    assert!(matches!(craft_print::register(Box::new(craft_print::SystemSpooler)), Err(RegisterError::DuplicateName { .. })));
}

#[test]
fn per_job_options_reach_the_backend_through_the_registry() {
    let echo = FakeBackend::new("echo").with_printer("p", true).with_job_handle(|_, _| Some("1".into()));
    let registry = Registry::new();
    registry.register(Box::new(echo.clone())).unwrap();
    let options = JobOptions::new()
        .with_media("iso_a4_210x297mm")
        .with_orientation(Orientation::Landscape)
        .with_quality(Quality::Draft)
        .with_resolution(Resolution::new(300, 300))
        .with_colour(ColourMode::Mono)
        .with_expected_pages(2);
    let submitted = registry.submit_job(SubmitRequest::new(Document::pdf(b"%PDF"), job_for(None)).with_options(options.clone())).unwrap();
    assert_eq!(echo.submissions()[0].options, options);
    assert_eq!(submitted.job.map(|j| j.to_string()).as_deref(), Some("echo:1"));
    assert_eq!(submitted.expected.pages, Some(2));
    // Without options the backend sees the defaults.
    registry.submit_job(SubmitRequest::new(Document::pdf(b"%PDF"), job_for(None))).unwrap();
    assert_eq!(echo.submissions()[1].options, JobOptions::default());
}
