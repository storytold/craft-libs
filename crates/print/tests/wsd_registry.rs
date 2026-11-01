//! The WS-Print backend through the public registry API, against the in-process fake printer
//! (no sockets: the backend is given an in-memory transport) and, for the hostile cases, loopback
//! servers that play a misbehaving device.
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]

use craft_print::wsd::testing as wsd_support;

use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_print::backends::Backend;
use craft_print::wsd::{Reconciled, TicketShape, Transfer, WsdBackend};
use craft_print::{
    BackendClass, CancelToken, Clock, Document, DocumentFormat, Duplex, ErrorKind, Job, JobHint, JobId, JobOptions, JobState, JobWatch, ManualClock,
    Phase, PrinterState, Progress, Quality, Registry, Resolution, RetryPolicy, RetrySafety, Scaling, SubmitRequest, UserConfirmed, WaitPolicy,
    WatchOutcome, WatchStep, WatchSupport, namespaced_id, wait_for_job,
};
use wsd_support::{Ending, FakeConfig, FakePrinter, MemTransport, printer_server, raw_response};

const URL: &str = "http://127.0.0.1:5357/print";
const PDF: &[u8] = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\r\n--not-a-boundary\r\n--\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n";

fn quick() -> Duration {
    Duration::from_millis(1)
}

/// A watch that polls every millisecond and gives up after five seconds.
fn fast() -> WaitPolicy {
    WaitPolicy::every(Duration::from_millis(1)).with_max_wait(Duration::from_secs(5))
}

fn backend(p: &Arc<FakePrinter>) -> WsdBackend {
    WsdBackend::new().with_transport(MemTransport(p.clone())).with_poll_interval(quick())
}

fn registry(backend: &WsdBackend) -> Registry {
    let registry = Registry::new();
    registry.register(Box::new(backend.clone())).unwrap();
    registry
}

fn id(url: &str) -> String {
    namespaced_id("wsd", url)
}

fn job(url: &str) -> Job {
    Job { printer: Some(id(url)), title: "Report".into(), ..Job::default() }
}

fn request<'a>(job: Job) -> SubmitRequest<'a> {
    SubmitRequest::new(Document::pdf(PDF), job).with_consent(UserConfirmed::interactive())
}

/// A backend over the fake with the printer added, and a registry that holds it.
fn setup(config: FakeConfig) -> (Arc<FakePrinter>, WsdBackend, Registry) {
    let p = FakePrinter::new(config);
    let b = backend(&p);
    b.add(URL).unwrap();
    let r = registry(&b);
    (p, b, r)
}

// ---- identity, listing, policy ----

#[test]
fn the_backend_is_a_network_backend_named_wsd_and_lists_only_what_was_added() {
    let b = WsdBackend::new();
    assert_eq!((b.name(), b.class()), ("wsd", BackendClass::Network));
    assert!(b.accepted_formats().contains(&DocumentFormat::Pdf) && b.accepted_formats().contains(&DocumentFormat::PwgRaster));
    assert!(b.list_printers().unwrap().is_empty(), "nothing is listed until the app adds a printer");
    let r = registry(&b);
    assert!(r.list_printers().unwrap().items.is_empty());
    let native = b.add_named(URL, "Front office").unwrap();
    assert_eq!(native, URL);
    b.add("http://127.0.0.1:5358/print").unwrap();
    assert_eq!(b.add(URL).unwrap(), URL, "adding twice keeps one entry");
    let listing = r.list_printers().unwrap();
    assert!(listing.is_complete());
    let ids: Vec<_> = listing.items.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["wsd:http://127.0.0.1:5357/print", "wsd:http://127.0.0.1:5358/print"]);
    assert_eq!(listing.items[0].name, "Front office");
    assert_eq!(listing.items[1].name, "http://127.0.0.1:5358/print");
    assert!(listing.items.iter().all(|p| !p.default && p.two_sided.is_none()));
    assert!(b.remove(URL) && !b.remove(URL) && !b.remove("not a url"));
    assert_eq!(b.urls(), ["http://127.0.0.1:5358/print"]);
}

#[test]
fn building_listing_and_adding_open_no_connection() {
    let p = FakePrinter::new(FakeConfig::default());
    let server = printer_server(p.clone());
    let b = WsdBackend::with_printers([server.url("/print")]).unwrap();
    let r = registry(&b);
    r.list_printers().unwrap();
    b.add(&server.url("/other")).unwrap();
    r.list_printers().unwrap();
    assert_eq!(*server.hits.lock().unwrap(), 0);
    assert!(p.log().is_empty());
}

#[test]
fn printer_urls_go_through_the_policy() {
    let b = WsdBackend::new();
    for bad in [
        "http://printer.example/print",
        "http://localhost:5357/print",
        "https://127.0.0.1/print",
        "http://192.0.2.10:5357/print",
        "http://198.51.100.20/print",
        "http://127.0.0.1:5357/print?x=1",
        "http://user@127.0.0.1/print",
        "127.0.0.1",
        "",
    ] {
        let e = b.add(bad).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{bad}");
        assert!(!e.is_retryable());
    }
    assert!(b.urls().is_empty());
    assert!(WsdBackend::with_printers([URL, "http://printer.example/print"]).is_err());
    assert_eq!(WsdBackend::with_printers(["http://10.0.0.5/print", "http://[::1]:5357/print"]).unwrap().urls().len(), 2);
}

#[test]
fn only_added_printers_can_be_reached() {
    let (p, _b, r) = setup(FakeConfig::default());
    // A valid URL that was never added: not found, and nothing was sent.
    let other = "http://127.0.0.1:5999/print";
    assert_eq!(r.printer_details(&id(other)).unwrap_err().kind(), ErrorKind::NotFound);
    assert_eq!(r.submit(request(job(other))).unwrap_err().kind(), ErrorKind::NotFound);
    // A URL the policy refuses is refused before it is looked up.
    let e = r.submit(request(job("http://printer.example/print"))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest);
    // There is no default printer.
    let e = r.submit(request(Job { printer: None, ..Job::default() })).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert!(p.log().is_empty());
    // A backend without the wsd prefix id is not guessed.
    assert_eq!(r.submit(request(Job { printer: Some(URL.into()), ..Job::default() })).unwrap_err().kind(), ErrorKind::UnknownBackend);
}

// ---- details ----

#[test]
fn details_map_the_printers_capabilities_and_status() {
    let config = FakeConfig {
        formats: vec!["application/pdf", "image/pwg-raster", "application/vnd.vendor-pcl", "APPLICATION/POSTSCRIPT", "application/pdf"],
        sides: vec!["OneSided", "TwoSidedLongEdge"],
        max_copies: 25,
        color: true,
        state: "Stopped",
        state_reasons: vec!["MediaEmpty", "DoorOpen"],
        media: vec!["iso_a4_210x297mm", "na_letter_8.5x11in"],
        ..FakeConfig::default()
    };
    let (p, _b, r) = setup(config);
    let d = r.printer_details(&id(URL)).unwrap();
    assert_eq!(d.id, "wsd:http://127.0.0.1:5357/print");
    assert_eq!(d.name, "Fake Printer");
    assert!(!d.default);
    assert_eq!(
        d.formats,
        [DocumentFormat::Pdf, DocumentFormat::PwgRaster, DocumentFormat::PostScript],
        "listed order, unknown formats left out, no duplicates"
    );
    assert_eq!((d.two_sided, d.colour, d.max_copies), (Some(true), Some(true), Some(25)));
    assert_eq!(d.paper, ["iso_a4_210x297mm", "na_letter_8.5x11in"]);
    assert_eq!(
        d.honoured,
        Some(vec![JobHint::Copies, JobHint::Duplex, JobHint::Colour, JobHint::Media, JobHint::Orientation, JobHint::Quality, JobHint::Resolution,])
    );
    let status = d.status.unwrap();
    assert_eq!(status.state, PrinterState::Stopped);
    assert_eq!(status.reasons, ["MediaEmpty", "DoorOpen"]);
    assert!(d.options.is_empty());
    assert_eq!(p.actions(), ["GetPrinterElements"], "details cost one round trip and create no job");
    assert_eq!(p.jobs(), 0);
}

#[test]
fn details_say_unknown_when_the_printer_does_not() {
    let (_p, _b, r) = setup(FakeConfig { sides: vec!["OneSided"], color: false, state: "VendorBusy", ..FakeConfig::default() });
    let d = r.printer_details(&id(URL)).unwrap();
    assert_eq!((d.two_sided, d.colour), (Some(false), Some(false)));
    assert!(d.status.is_none(), "a vendor state is not guessed");
    let (_p, _b, r) = setup(FakeConfig { sides: vec![], ..FakeConfig::default() });
    assert_eq!(r.printer_details(&id(URL)).unwrap().two_sided, None);
}

// ---- routing, consent, formats ----

#[test]
fn a_job_is_accepted_through_the_registry_and_named_without_waiting_for_the_print() {
    let (p, _b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let phases = Mutex::new(Vec::new());
    let mut record = |event: Progress| phases.lock().unwrap().push((event.done, event.total));
    let started = std::time::Instant::now();
    let submitted = r.submit_job(request(Job { copies: 2, ..job(URL) }).with_progress(&mut record)).unwrap();
    assert!(started.elapsed() < Duration::from_secs(5), "submit returns when the printer has accepted the document, not when it prints");
    assert_eq!(submitted.backend, "wsd");
    let job_ref = submitted.job.clone().unwrap();
    assert_eq!(job_ref.to_string(), format!("wsd:{URL}#1"));
    assert_eq!(job_ref.native(), format!("{URL}#1"));
    assert_eq!(WsdBackend::job_id(URL, 1).unwrap(), job_ref, "the typed constructor names the same job");
    assert!(submitted.message.contains("accepted") && submitted.message.contains("127.0.0.1:5357"), "{}", submitted.message);
    assert_eq!((submitted.watch, submitted.poll_interval), (WatchSupport::Poll, Some(Duration::from_millis(1))));
    assert_eq!(*phases.lock().unwrap(), [(1, Some(3)), (2, Some(3)), (3, Some(3))]);
    let stored = p.job(1).unwrap();
    assert_eq!(stored.document.as_deref(), Some(PDF), "the bytes arrive intact");
    assert_eq!(stored.format.as_deref(), Some("application/pdf"));
    assert!(stored.ticket.contains("Report") && stored.ticket.contains(">2</"), "{}", stored.ticket);
    assert_eq!(p.actions(), ["GetPrinterElements", "CreatePrintJob", "SendDocument"], "nothing is asked after the printer accepted the document");
    // The older receipt entry point is the same call.
    assert_eq!(r.submit(request(job(URL))).unwrap().job_id.as_deref(), Some(format!("{URL}#2").as_str()));
}

#[test]
fn an_accepted_job_is_followed_to_completion_with_the_blocking_convenience() {
    let (p, _b, r) = setup(FakeConfig::default());
    let submitted = r.submit_job(request(job(URL))).unwrap();
    let outcome = wait_for_job(&r, &submitted.job.unwrap(), fast()).unwrap();
    assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.state == JobState::Completed), "{outcome:?}");
    assert!(outcome.is_success());
    let seq = p.actions();
    assert_eq!(&seq[..3], ["GetPrinterElements", "CreatePrintJob", "SendDocument"]);
    assert!(seq[3..].iter().all(|a| a == "GetJobElements"));
}

#[test]
fn zero_or_too_few_copies_are_refused_before_anything_is_sent_and_never_become_the_maximum() {
    // The fake lists copies 1..=99.
    let (p, _b, r) = setup(FakeConfig { max_copies: 99, ..FakeConfig::default() });
    let e = r.submit(request(Job { copies: 0, ..job(URL) })).unwrap_err();
    assert_eq!((e.kind(), e.phase(), e.retry_safety()), (ErrorKind::InvalidRequest, Some(Phase::Prepare), RetrySafety::Safe));
    assert!(!e.is_retryable());
    assert_eq!(p.jobs(), 0, "no job was created");
    assert!(!p.actions().iter().any(|a| a == "CreatePrintJob" || a == "SendDocument"), "{:?}", p.actions());
    // Above the maximum keeps the lowering, with the warning.
    let receipt = r.submit(request(Job { copies: 120, ..job(URL) })).unwrap();
    assert!(p.job(1).unwrap().ticket.contains(">99</"));
    assert!(receipt.message.contains("warnings"), "{}", receipt.message);
    // One copy and an in-range count are accepted.
    assert!(r.submit(request(Job { copies: 1, ..job(URL) })).is_ok());
}

#[test]
fn network_jobs_need_consent_and_nothing_is_sent_without_it() {
    let (p, _b, r) = setup(FakeConfig::default());
    let e = r.submit(SubmitRequest::new(Document::pdf(PDF), job(URL))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::ConsentRequired);
    assert!(p.log().is_empty());
    assert_eq!(UserConfirmed::attest(None).unwrap_err().kind(), ErrorKind::ConsentRequired);
    // A caller holding the backend directly gets the same answer.
    let direct = backend(&p);
    direct.add(URL).unwrap();
    let e = direct.submit(SubmitRequest::new(Document::pdf(PDF), Job { printer: Some(URL.into()), ..Job::default() })).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::ConsentRequired);
    assert!(p.log().is_empty());
    // The attested path works.
    let ok = SubmitRequest::new(Document::pdf(PDF), job(URL)).with_consent(UserConfirmed::attest(Some(true)).unwrap());
    assert!(r.submit(ok).is_ok());
}

#[test]
fn a_format_the_printer_does_not_list_is_refused_before_a_job_exists() {
    let (p, _b, r) = setup(FakeConfig::default());
    let ps = SubmitRequest::new(Document::new(DocumentFormat::PostScript, b"%!PS\n"), job(URL)).with_consent(UserConfirmed::interactive());
    let e = r.submit(ps).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::UnsupportedFormat);
    assert!(!e.is_retryable());
    assert_eq!((p.jobs(), p.actions()), (0, vec!["GetPrinterElements".to_owned()]));
    // The raster format the printer lists is sent as that format.
    let raster = SubmitRequest::new(Document::new(DocumentFormat::PwgRaster, b"RaS2...."), job(URL)).with_consent(UserConfirmed::interactive());
    let submitted = r.submit_job(raster).unwrap();
    let number: u32 = submitted.job.unwrap().native().rsplit('#').next().unwrap_or("").parse().unwrap();
    assert_eq!(p.job(number).unwrap().format.as_deref(), Some("image/pwg-raster"));
}

#[test]
fn the_printers_own_spelling_of_the_format_is_used() {
    let (p, _b, r) = setup(FakeConfig { formats: vec!["Application/PDF"], ..FakeConfig::default() });
    r.submit(request(job(URL))).unwrap();
    assert_eq!(p.job(1).unwrap().format.as_deref(), Some("Application/PDF"));
}

#[test]
fn a_token_cancelled_before_the_job_starts_sends_nothing() {
    let (p, _b, r) = setup(FakeConfig::default());
    let token = CancelToken::new();
    token.cancel();
    let e = r.submit(request(job(URL)).with_cancel(token.clone())).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Cancelled);
    // The backend checks too, for callers that hold it directly.
    let direct = backend(&p);
    direct.add(URL).unwrap();
    let e = direct.submit(request(Job { printer: Some(URL.into()), ..Job::default() }).with_cancel(token)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Cancelled);
    assert!(p.log().is_empty());
}

// ---- job settings and ticket shapes ----

#[test]
fn copies_duplex_and_colour_reach_the_ticket_and_the_rest_is_reported() {
    let (p, _b, r) = setup(FakeConfig::default());
    let j = Job {
        copies: 3,
        duplex: Some(Duplex::LongEdge),
        grayscale: true,
        scaling: Scaling::ShrinkToFit,
        options: vec![("Tray".into(), "2".into())],
        ..job(URL)
    };
    let receipt = r.submit(request(j)).unwrap();
    let t = p.job(1).unwrap().ticket;
    assert!(t.contains(">3</") && t.contains("TwoSidedLongEdge"), "{t}");
    assert!(receipt.message.contains("scaling") && receipt.message.contains("driver options"), "{}", receipt.message);
    // Simplex is forced with `Some(Off)`, and the defaults write neither copies nor sides.
    r.submit(request(Job { duplex: Some(Duplex::Off), ..job(URL) })).unwrap();
    let t2 = p.job(2).unwrap().ticket;
    assert!(t2.contains("OneSided") && !t2.contains("Copies"), "{t2}");
    r.submit(request(job(URL))).unwrap();
    let t3 = p.job(3).unwrap().ticket;
    assert!(!t3.contains("Copies") && !t3.contains("Sides"), "{t3}");
}

#[test]
fn settings_the_printer_does_not_list_are_lowered_with_a_warning_or_refused_in_strict_mode() {
    let (p, _b, r) = setup(FakeConfig { max_copies: 10, ..FakeConfig::default() });
    let receipt = r.submit(request(Job { copies: 50, duplex: Some(Duplex::ShortEdge), ..job(URL) })).unwrap();
    let t = p.job(1).unwrap().ticket;
    assert!(t.contains(">10</") && !t.contains("TwoSidedShortEdge"), "{t}");
    assert!(receipt.message.contains("warnings"), "{}", receipt.message);
    let p2 = FakePrinter::new(FakeConfig::default());
    let strict = backend(&p2).with_strict(true);
    strict.add(URL).unwrap();
    let e = registry(&strict).submit(request(Job { copies: 50, ..job(URL) })).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest);
    assert_eq!(p2.jobs(), 0, "refused before CreatePrintJob");
    let ok = registry(&strict).submit(request(Job { copies: 3, ..job(URL) }));
    assert!(ok.is_ok());
    assert!(p2.job(1).unwrap().ticket.contains("MustHonor=\"true\""));
}

#[test]
fn a_reduced_ticket_refuses_settings_it_cannot_carry_by_default() {
    for shape in [TicketShape::Description, TicketShape::JobNameOnly, TicketShape::Empty] {
        let p = FakePrinter::new(FakeConfig::default());
        let b = backend(&p).with_ticket_shape(shape);
        b.add(URL).unwrap();
        let r = registry(&b);
        let e = r.submit(request(Job { copies: 2, ..job(URL) })).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{shape:?}");
        assert!(e.message().contains("copies"), "{}", e.message());
        let e = r.submit(request(Job { duplex: Some(Duplex::LongEdge), ..job(URL) })).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{shape:?}");
        assert_eq!(p.jobs(), 0, "{shape:?}: refused before anything is created");
        // A plain job needs no settings and goes through.
        r.submit(request(job(URL))).unwrap();
        assert_eq!(p.jobs(), 1);
    }
}

#[test]
fn a_reduced_ticket_can_drop_settings_with_a_warning() {
    let p = FakePrinter::new(FakeConfig::default());
    let b = backend(&p).with_ticket_shape(TicketShape::JobNameOnly).with_drop_ticket_settings(true);
    b.add(URL).unwrap();
    let receipt = registry(&b).submit(request(Job { copies: 2, duplex: Some(Duplex::LongEdge), ..job(URL) })).unwrap();
    assert!(receipt.message.contains("leaves out copies") && receipt.message.contains("leaves out sides"), "{}", receipt.message);
    let t = p.job(1).unwrap().ticket;
    assert!(
        t.contains("Report") && !t.contains("JobOriginatingUserName") && !t.contains("JobProcessing") && !t.contains("DocumentProcessing"),
        "{t}"
    );
}

#[test]
fn the_default_ticket_is_the_full_one_and_the_shapes_differ() {
    let ticket_for = |shape: Option<TicketShape>| {
        let p = FakePrinter::new(FakeConfig::default());
        let mut b = backend(&p);
        if let Some(shape) = shape {
            b = b.with_ticket_shape(shape);
        }
        b.add(URL).unwrap();
        registry(&b).submit(request(job(URL))).unwrap();
        p.job(1).unwrap().ticket
    };
    let default = ticket_for(None);
    assert_eq!(default, ticket_for(Some(TicketShape::Full)));
    assert!(default.contains("JobOriginatingUserName") && default.contains(">craft<"), "{default}");
    assert!(ticket_for(Some(TicketShape::Description)).contains("JobOriginatingUserName"));
    let name_only = ticket_for(Some(TicketShape::JobNameOnly));
    assert!(name_only.contains("Report") && !name_only.contains("JobOriginatingUserName"));
    assert!(!ticket_for(Some(TicketShape::Empty)).contains("JobDescription"));
    // The user name is configurable, and the transfer too.
    let p = FakePrinter::new(FakeConfig::default());
    let b = backend(&p).with_user_name("kiosk").with_transfer(Transfer::InlineBase64);
    b.add(URL).unwrap();
    registry(&b).submit(request(job(URL))).unwrap();
    assert!(p.job(1).unwrap().ticket.contains(">kiosk<"));
    assert!(p.log().iter().any(|l| l.action.ends_with("SendDocument") && l.content_type.starts_with("application/soap+xml")));
    assert_eq!(p.job(1).unwrap().document.as_deref(), Some(PDF));
}

// ---- cancellation and progress ----

#[test]
fn cancelling_after_the_job_is_created_cancels_it_on_the_printer_and_sends_no_document() {
    let (p, _b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let token = CancelToken::new();
    let t = token.clone();
    let mut progress = move |event: Progress| {
        if event.done == 2 {
            t.cancel();
        }
    };
    let e = r.submit(request(job(URL)).with_cancel(token).with_progress(&mut progress)).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Cancelled);
    assert_eq!(p.job(1).unwrap().state, "Canceled");
    assert!(!p.actions().iter().any(|a| a == "SendDocument"), "{:?}", p.actions());
}

#[test]
fn cancelling_a_running_job_sends_cancel_job_once_and_reports_cancelled() {
    let (p, _b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let submitted = r.submit_job(request(job(URL))).unwrap();
    assert_eq!(p.job(1).unwrap().state, "Pending");
    // A caller that wants it stopped: its token goes to the watch, which sends the cancel.
    let token = CancelToken::new();
    let mut watch = r.job_watch(&submitted.job.unwrap(), fast()).unwrap().with_cancel(token.clone()).with_clock(Arc::new(ManualClock::new()));
    assert!(matches!(watch.tick(&r), WatchStep::Update(s) if s.state == JobState::Pending));
    token.cancel();
    let outcome = watch.run(&r, &mut |_| {});
    let e = outcome.error().unwrap();
    assert_eq!(e.kind(), ErrorKind::Cancelled);
    assert!(!e.is_retryable());
    assert_eq!(p.job(1).unwrap().state, "Canceled");
    assert_eq!(p.actions().iter().filter(|a| *a == "CancelJob").count(), 1);
    // A direct cancel through the registry works too, and cancelling an ended job is not an error.
    let second = r.submit_job(request(job(URL))).unwrap().job.unwrap();
    r.cancel_job(&second).unwrap();
    r.cancel_job(&second).unwrap();
    assert_eq!(r.job_status(&second).unwrap().state, JobState::Cancelled);
}

#[test]
fn a_slow_job_is_accepted_and_the_watch_gives_up_without_touching_it() {
    let (p, _b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let submitted = r.submit_job(request(job(URL))).unwrap();
    let job_ref = submitted.job.unwrap();
    let log_after_submit = p.log().len();
    assert_eq!(r.job_status(&job_ref).unwrap().state, JobState::Pending);
    let outcome = wait_for_job(&r, &job_ref, WaitPolicy::every(Duration::from_millis(2)).with_max_wait(Duration::from_millis(40))).unwrap();
    assert!(matches!(outcome, WatchOutcome::TimedOut { last: Some(_) }), "{outcome:?}");
    assert!(!p.actions().iter().any(|a| a == "CancelJob"), "giving up does not cancel the job");
    assert!(p.log().len() > log_after_submit);
}

#[test]
fn the_status_of_a_job_maps_state_reasons_and_counters() {
    let (p, _b, r) = setup(FakeConfig { ending: Ending::Completes("JobCompletedWithWarnings"), polls_to_finish: 3, ..FakeConfig::default() });
    let id = r.submit_job(request(job(URL))).unwrap().job.unwrap();
    let first = r.job_status(&id).unwrap();
    assert_eq!((first.state, first.reasons.as_slice(), first.sheets_done), (JobState::Processing, ["JobPrinting".to_owned()].as_slice(), Some(0)));
    assert!(!first.terminal());
    r.job_status(&id).unwrap();
    let last = r.job_status(&id).unwrap();
    assert_eq!((last.state, last.sheets_done), (JobState::Completed, Some(1)));
    assert_eq!(last.warnings.len(), 1, "completed with warnings is Completed plus a warning: {:?}", last.warnings);
    assert!(last.terminal() && last.state.is_success());
    assert_eq!(last.sheets_total, None, "WS-Print does not say how many sheets there will be");
    drop(p);
    // The other end states.
    let state_of = |ending| {
        let (_p, _b, r) = setup(FakeConfig { ending, ..FakeConfig::default() });
        let id = r.submit_job(request(job(URL))).unwrap().job.unwrap();
        r.job_status(&id).unwrap()
    };
    assert_eq!(state_of(Ending::Aborts("DocumentFormatError")).state, JobState::Aborted);
    assert_eq!(state_of(Ending::Aborts("DocumentFormatError")).reasons, ["DocumentFormatError"]);
}

#[test]
fn the_status_of_a_forgotten_job_is_not_found_and_says_what_that_means() {
    let (_p, _b, r) = setup(FakeConfig { ending: Ending::Vanishes, ..FakeConfig::default() });
    let job_ref = r.submit_job(request(job(URL))).unwrap().job.unwrap();
    r.job_status(&job_ref).unwrap();
    r.job_status(&job_ref).unwrap();
    let e = r.job_status(&job_ref).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::NotFound);
    assert_eq!(e.subject(), craft_print::NotFoundSubject::Job, "the printer answered; it is the job it does not know");
    assert!(e.message().contains("forget") && e.message().contains("most likely finished"), "{}", e.message());
    assert!(!e.is_retryable());
    assert_eq!(e.phase(), Some(Phase::Poll));
    // A watch reports it as forgotten (success: it most likely finished), with the last status it saw.
    let (_p, _b, r) = setup(FakeConfig { ending: Ending::Vanishes, ..FakeConfig::default() });
    let job_ref = r.submit_job(request(job(URL))).unwrap().job.unwrap();
    let outcome = wait_for_job(&r, &job_ref, fast()).unwrap();
    assert!(matches!(outcome, WatchOutcome::Forgotten { last: Some(_) }), "{outcome:?}");
    assert!(outcome.is_success());
}

#[test]
fn job_handles_that_are_not_ours_are_invalid_requests_not_forgotten_jobs() {
    let (p, b, r) = setup(FakeConfig::default());
    let log = p.log().len();
    for bad in [
        format!("wsd:{URL}"),                            // no job number
        format!("wsd:{URL}#x"),                          // not a number
        "wsd:http://127.0.0.1:5999/print#1".to_owned(),  // a printer that was never added
        "wsd:http://printer.example/print#1".to_owned(), // a printer the policy refuses
    ] {
        let bad: JobId = bad.parse().unwrap();
        let e = r.job_status(&bad).unwrap_err();
        assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{bad}: {e}");
        assert_eq!(r.cancel_job(&bad).unwrap_err().kind(), ErrorKind::InvalidRequest, "{bad}");
    }
    assert_eq!(p.log().len(), log, "nothing was sent for them");
    // A job of a printer that is removed afterwards cannot be reached any more.
    let id = r.submit_job(request(job(URL))).unwrap().job.unwrap();
    assert!(b.remove(URL));
    assert_eq!(r.job_status(&id).unwrap_err().kind(), ErrorKind::InvalidRequest);
}

// ---- how a job can end ----

#[test]
fn job_end_states_map_to_outcomes_and_errors() {
    let run = |ending| {
        let (_p, _b, r) = setup(FakeConfig { ending, ..FakeConfig::default() });
        let job_ref = r.submit_job(request(job(URL))).unwrap().job.unwrap();
        wait_for_job(&r, &job_ref, fast()).unwrap()
    };
    let warn = run(Ending::Completes("JobCompletedWithWarnings"));
    assert!(matches!(warn, WatchOutcome::Terminal(ref s) if s.state == JobState::Completed && !s.warnings.is_empty()));
    assert!(warn.is_success() && warn.error().is_none());
    let errors = run(Ending::Completes("JobCompletedWithErrors"));
    assert!(matches!(errors, WatchOutcome::Terminal(ref s) if s.state == JobState::CompletedWithErrors));
    assert_eq!(errors.error().unwrap().kind(), ErrorKind::Failed);
    let aborted = run(Ending::Aborts("DocumentFormatError"));
    let e = aborted.error().unwrap();
    assert_eq!(e.kind(), ErrorKind::Rejected);
    assert!(e.message().contains("DocumentFormatError"), "{}", e.message());
    let other = run(Ending::Aborts("JobPasswordWait"));
    assert_eq!((other.error().unwrap().kind(), other.error().unwrap().is_retryable()), (ErrorKind::Rejected, false));
    let gone = run(Ending::Vanishes);
    assert!(matches!(gone, WatchOutcome::Forgotten { .. }));
}

#[test]
fn faults_map_to_neutral_kinds_and_keep_their_code() {
    let (_p, _b, r) = setup(FakeConfig { accept_jobs: false, ..FakeConfig::default() });
    let e = r.submit(request(job(URL))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Busy);
    assert!(e.is_retryable());
    let n = e.native().unwrap();
    assert_eq!((n.domain, n.name), ("wsd-fault", Some("ServerErrorNotAcceptingJobs")));
    assert_eq!(n.code, 5);
    // The printer declined to create the job: nothing was accepted, and it says how long to wait.
    assert_eq!((e.retry_safety(), e.phase(), e.retry_after()), (RetrySafety::Safe, Some(Phase::Reply), Some(Duration::from_secs(15))));
    let (p, _b, r) = setup(FakeConfig { fail_send: true, ..FakeConfig::default() });
    let e = r.submit(request(job(URL))).unwrap_err();
    assert_eq!((e.kind(), e.is_retryable()), (ErrorKind::Failed, false));
    assert_eq!(e.native().unwrap().name, Some("ServerErrorInternalError"));
    assert_eq!(p.job(1).unwrap().state, "Canceled", "a failed send cancels the orphan job");
    // A fault on details too, and a details failure is an idempotent query.
    let (_p, _b, r) = setup(FakeConfig { http_error: Some(503), ..FakeConfig::default() });
    let e = r.printer_details(&id(URL)).unwrap_err();
    assert_eq!((e.kind(), e.is_retryable(), e.retry_safety()), (ErrorKind::Unavailable, true, RetrySafety::Safe));
    assert_eq!(e.retry_after(), Some(Duration::from_secs(5)));
}

// ---- hostile devices, over real loopback sockets ----

type Responder = Box<dyn Fn(&wsd_support::HttpRequest) -> Vec<u8> + Send>;

fn hostile(respond: impl Fn(&wsd_support::HttpRequest) -> Vec<u8> + Send + 'static) -> (wsd_support::Server, Registry) {
    let server = wsd_support::Server::start(respond);
    let b = WsdBackend::new().with_poll_interval(quick());
    b.add(&server.url("/print")).unwrap();
    let r = registry(&b);
    (server, r)
}

fn details_error(respond: impl Fn(&wsd_support::HttpRequest) -> Vec<u8> + Send + 'static) -> craft_print::PrintError {
    let (server, r) = hostile(respond);
    r.printer_details(&id(&server.url("/print"))).unwrap_err()
}

#[test]
fn hostile_replies_are_typed_errors_and_never_a_panic() {
    let soap = "application/soap+xml";
    let cases: Vec<(&str, Responder, ErrorKind)> = vec![
        ("html 503", Box::new(|_| raw_response(503, "text/html", b"<html><body>maintenance</body></html>")), ErrorKind::Unavailable),
        ("html 500 with a doctype", Box::new(|_| raw_response(500, "text/html", b"\n<!DOCTYPE html><html><body>bad</body>")), ErrorKind::Unavailable),
        ("429", Box::new(|_| raw_response(429, "text/plain", b"slow down")), ErrorKind::Busy),
        ("408", Box::new(|_| raw_response(408, "text/plain", b"too slow")), ErrorKind::Timeout),
        ("501", Box::new(|_| raw_response(501, "text/plain", b"no")), ErrorKind::Failed),
        ("404", Box::new(|_| raw_response(404, "text/plain", b"nope")), ErrorKind::Failed),
        ("html 200", Box::new(|_| raw_response(200, "text/html", b"<html><body>router login</body></html>")), ErrorKind::Failed),
        ("malformed xml", Box::new(move |_| raw_response(200, soap, b"<s:Envelope><s:Body>")), ErrorKind::Failed),
        ("empty 200", Box::new(move |_| raw_response(200, soap, b"")), ErrorKind::Failed),
        ("binary junk", Box::new(move |_| raw_response(200, soap, &[b'<', 0, 159, 146, 150, 0xff, 0xfe])), ErrorKind::Failed),
        (
            "wrong namespace",
            Box::new(move |_| {
                raw_response(
                    200,
                    soap,
                    b"<s:Envelope xmlns:s=\"urn:not-soap\"><s:Header/><s:Body><x:GetPrinterElementsResponse xmlns:x=\"urn:other\"/></s:Body></s:Envelope>",
                )
            }),
            ErrorKind::Failed,
        ),
        (
            "a DTD",
            Box::new(move |_| {
                raw_response(200, soap, b"<!DOCTYPE s [<!ENTITY a \"aaaa\">]><s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\"/>")
            }),
            ErrorKind::Failed,
        ),
        ("garbage multipart", Box::new(|_| raw_response(200, "multipart/related; boundary=b", b"<html>")), ErrorKind::Failed),
        ("not http at all", Box::new(|_| b"not http at all".to_vec()), ErrorKind::Failed),
        ("connection closed with no reply", Box::new(|_| Vec::new()), ErrorKind::Unavailable),
    ];
    for (what, respond, kind) in cases {
        let e = details_error(respond);
        assert_eq!(e.kind(), kind, "{what}: {e}");
        assert_eq!(e.is_retryable(), kind.is_retryable(), "{what}");
        assert!(!e.message().is_empty());
    }
}

#[test]
fn a_retry_after_from_the_printer_reaches_the_neutral_error() {
    let with_wait = |status: u16, wait: &str| {
        let wait = wait.to_owned();
        move |_: &wsd_support::HttpRequest| {
            format!("HTTP/1.1 {status} X\r\nContent-Type: text/plain\r\nRetry-After: {wait}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                .into_bytes()
        }
    };
    let e = details_error(with_wait(503, "17"));
    assert_eq!((e.kind(), e.retry_after()), (ErrorKind::Unavailable, Some(Duration::from_secs(17))), "{e}");
    // Without a usable header the default wait stays.
    let e = details_error(with_wait(503, "soon"));
    assert_eq!(e.retry_after(), Some(Duration::from_secs(5)), "{e}");
}

#[test]
fn a_connection_closed_with_no_reply_is_retryable() {
    let e = details_error(|_| Vec::new());
    assert_eq!(e.kind(), ErrorKind::Unavailable);
    assert!(e.is_retryable());
}

#[test]
fn a_huge_reply_is_cut_off() {
    let server = wsd_support::Server::start(|_| raw_response(200, "application/soap+xml", &vec![b'<'; 3 << 20]));
    let b = WsdBackend::new().with_limits(craft_print::wsd::NetworkLimits { max_response: 64 << 10, ..Default::default() });
    b.add(&server.url("/print")).unwrap();
    let e = registry(&b).printer_details(&id(&server.url("/print"))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Failed);
    assert!(e.message().contains("larger than the limit"), "{}", e.message());
}

#[test]
fn a_redirect_is_refused_not_followed_and_not_retryable() {
    let target = wsd_support::Server::start(|_| raw_response(200, "text/plain", b"never reached"));
    let location = target.url("/elsewhere");
    let (server, r) =
        hostile(move |_| format!("HTTP/1.1 302 Found\r\nLocation: {location}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes());
    let e = r.printer_details(&id(&server.url("/print"))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Failed);
    assert!(!e.is_retryable(), "a redirect will not go away by waiting");
    assert_eq!(e.retry_safety(), RetrySafety::Idempotent, "a query");
    assert_eq!(*target.hits.lock().unwrap(), 0, "the redirect target was never contacted");
    // A policy refusal is not retryable either, and nothing was sent.
    let policy = WsdBackend::new().add("http://printer.example/print").unwrap_err();
    assert_eq!((policy.kind(), policy.is_retryable(), policy.retry_safety()), (ErrorKind::InvalidRequest, false, RetrySafety::Safe));
}

#[test]
fn a_silent_printer_times_out_and_a_query_may_be_repeated() {
    let server = wsd_support::Server::start(|_| {
        std::thread::sleep(Duration::from_millis(1500));
        Vec::new()
    });
    let b = WsdBackend::new().with_limits(craft_print::wsd::NetworkLimits { timeout: Duration::from_millis(300), ..Default::default() });
    b.add(&server.url("/print")).unwrap();
    let started = std::time::Instant::now();
    let e = registry(&b).printer_details(&id(&server.url("/print"))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::Timeout, "{e}");
    assert!(e.is_retryable());
    assert_eq!(e.retry_safety(), RetrySafety::Idempotent);
    assert!(started.elapsed() < Duration::from_millis(1400), "{:?}", started.elapsed());
}

#[test]
fn a_connection_that_cannot_be_made_is_safe_to_retry() {
    // A port that was open a moment ago and is not any more.
    let port = std::net::TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
    let url = format!("http://127.0.0.1:{port}/print");
    let b = WsdBackend::new().with_limits(craft_print::wsd::NetworkLimits { timeout: Duration::from_secs(5), ..Default::default() });
    b.add(&url).unwrap();
    let r = registry(&b);
    let e = r.submit(request(job(&url))).unwrap_err();
    assert_eq!((e.kind(), e.is_retryable()), (ErrorKind::Unavailable, true), "{e}");
    // The first request of a submit is the capabilities read: a query that never connected.
    assert_eq!((e.phase(), e.retry_safety()), (Some(Phase::Connect), RetrySafety::Safe), "{e}");
}

#[test]
fn a_hostile_reply_in_the_middle_of_a_job_leaves_a_typed_error() {
    // The printer answers discovery and job creation properly and then goes bad on SendDocument.
    let p = FakePrinter::new(FakeConfig::default());
    let server = wsd_support::Server::start(move |req| {
        let ct = req.header("content-type").unwrap_or("");
        let action = req.header("soapaction").unwrap_or("").to_owned();
        if action.contains("SendDocument") {
            return raw_response(502, "text/html", b"<html>bad gateway</html>");
        }
        let (status, ct, body) = p.handle(ct, &req.body);
        raw_response(status, &ct, &body)
    });
    let b = WsdBackend::new().with_poll_interval(quick());
    b.add(&server.url("/print")).unwrap();
    let e = registry(&b).submit(request(job(&server.url("/print")))).unwrap_err();
    assert_eq!((e.kind(), e.is_retryable()), (ErrorKind::Unavailable, true), "{e}");
    // A gateway error after the document went out: it may have been processed.
    assert_eq!((e.phase(), e.retry_safety()), (Some(Phase::Reply), RetrySafety::OutcomeUnknown));
}

#[test]
fn a_full_print_over_a_loopback_socket() {
    let printer = FakePrinter::new(FakeConfig::default());
    let server = printer_server(printer.clone());
    let b = WsdBackend::new().with_poll_interval(quick());
    b.add(&server.url("/print")).unwrap();
    let r = registry(&b);
    let pdf = b"%PDF-1.4\n\r\n--x\r\n binary \xff\x00\xfe\n%%EOF".to_vec();
    let submitted =
        r.submit_job(SubmitRequest::new(Document::pdf(&pdf), job(&server.url("/print"))).with_consent(UserConfirmed::interactive())).unwrap();
    assert!(submitted.message.contains("accepted"));
    let outcome = wait_for_job(&r, &submitted.job.unwrap(), fast()).unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(printer.job(1).unwrap().document, Some(pdf));
    let d = r.printer_details(&id(&server.url("/print"))).unwrap();
    assert_eq!(d.name, "Fake Printer");
}

// ---- watching on an injected clock, retries and reconciling ----

use std::sync::atomic::{AtomicU32, Ordering};

use craft_print::wsd::adapter::{DeviceUrl, Request, Response, Transport, TransportKind, WireError};
use craft_print::{Jitter, retry};

/// Delivers every request to the fake printer and, for the named operation, loses the reply a number
/// of times: the printer did the work and the client saw the connection close with no answer.
struct LossyReply {
    printer: Arc<FakePrinter>,
    lose: &'static str,
    remaining: AtomicU32,
}

impl LossyReply {
    fn new(printer: &Arc<FakePrinter>, lose: &'static str, times: u32) -> Self {
        Self { printer: Arc::clone(printer), lose, remaining: AtomicU32::new(times) }
    }
}

impl Transport for LossyReply {
    fn post(&self, _: &DeviceUrl, request: Request) -> Result<Response, WireError> {
        let (status, content_type, body) = self.printer.handle(&request.content_type, &request.body);
        if request.soap_action.ends_with(self.lose) && self.remaining.load(Ordering::SeqCst) > 0 {
            self.remaining.fetch_sub(1, Ordering::SeqCst);
            return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
        }
        Ok(Response::new(status, Some(content_type), body))
    }
}

fn lossy(config: FakeConfig, lose: &'static str, times: u32) -> (Arc<FakePrinter>, WsdBackend, Registry) {
    let p = FakePrinter::new(config);
    let b = WsdBackend::new().with_transport(LossyReply::new(&p, lose, times)).with_poll_interval(quick());
    b.add(URL).unwrap();
    let r = registry(&b);
    (p, b, r)
}

fn count(p: &FakePrinter, action: &str) -> usize {
    p.actions().iter().filter(|a| *a == action).count()
}

#[test]
fn a_watch_on_an_injected_clock_follows_the_job_to_a_terminal_state() {
    let (p, _b, r) = setup(FakeConfig { polls_to_finish: 3, ..FakeConfig::default() });
    let submitted = r.submit_job(request(job(URL))).unwrap();
    let clock = ManualClock::new();
    let policy = WaitPolicy { interval: Some(Duration::from_secs(2)), growth_percent: 100, ..WaitPolicy::default() };
    let mut watch = JobWatch::for_submitted(&submitted, policy).unwrap().with_clock(Arc::new(clock.clone()));
    let started = std::time::Instant::now();
    let mut states = Vec::new();
    let outcome = watch.run(&r, &mut |step| {
        if let WatchStep::Update(s) = step {
            states.push(s.state);
        }
    });
    assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.state == JobState::Completed), "{outcome:?}");
    assert_eq!(states, [JobState::Processing, JobState::Processing]);
    assert_eq!(clock.now(), Duration::from_secs(4), "two gaps of two seconds, on the injected clock");
    assert!(started.elapsed() < Duration::from_secs(2), "and no real waiting");
    assert_eq!(count(&p, "GetJobElements"), 3);
}

#[test]
fn a_dropped_connection_while_following_is_retried_because_a_status_read_is_idempotent() {
    let (p, _b, r) = lossy(FakeConfig { polls_to_finish: 2, ..FakeConfig::default() }, "GetJobElements", 2);
    let submitted = r.submit_job(request(job(URL))).unwrap();
    let mut watch = JobWatch::for_submitted(&submitted, fast()).unwrap().with_clock(Arc::new(ManualClock::new()));
    let mut retries = Vec::new();
    let outcome = watch.run(&r, &mut |step| {
        if let WatchStep::Retrying { error, .. } = step {
            retries.push((error.kind(), error.phase(), error.retry_safety()));
        }
    });
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(retries, [(ErrorKind::Unavailable, Some(Phase::Poll), RetrySafety::Idempotent); 2]);
    assert_eq!(count(&p, "CreatePrintJob"), 1);
}

#[test]
fn a_lost_reply_to_send_document_is_outcome_unknown_and_reconcile_finds_the_job() {
    let (p, b, r) = lossy(FakeConfig::default(), "SendDocument", 1);
    let e = r.submit_job(request(job(URL))).unwrap_err();
    assert_eq!((e.kind(), e.is_retryable()), (ErrorKind::Unavailable, true), "{e}");
    assert_eq!(e.retry_safety(), RetrySafety::OutcomeUnknown, "the document went out; the reply did not come back");
    assert!(e.outcome_unknown());
    assert_eq!(e.phase(), Some(Phase::Reply));
    // The printer has the job and its document; the backend did not cancel a job that may be printing.
    assert_eq!(p.jobs(), 1);
    assert!(p.job(1).unwrap().document.is_some());
    assert_eq!(count(&p, "CancelJob"), 0);
    // Reconcile before sending again: it reads the lists and finds exactly one job of that name.
    let Reconciled::Found(found) = b.reconcile(URL, "Report").unwrap() else { panic!("the job should be found") };
    assert_eq!(found.job.to_string(), format!("wsd:{URL}#1"));
    assert_eq!(found.name, "Report");
    // It is a normal job handle from here on.
    let outcome = wait_for_job(&r, &found.job, fast()).unwrap();
    assert!(outcome.is_success(), "{outcome:?}");
    assert_eq!(count(&p, "CreatePrintJob"), 1, "nothing was submitted a second time");
    // Reconcile only reads.
    assert!(p.actions().iter().all(|a| !matches!(a.as_str(), "CancelJob")));
}

#[test]
fn a_lost_reply_to_create_print_job_leaves_an_orphan_that_reconcile_shows_as_pending() {
    let (p, b, r) = lossy(FakeConfig::default(), "CreatePrintJob", 1);
    let e = r.submit_job(request(job(URL))).unwrap_err();
    assert_eq!(e.retry_safety(), RetrySafety::OutcomeUnknown);
    assert_eq!(count(&p, "SendDocument"), 0, "the id never came back, so there was nothing to send to");
    let Reconciled::Found(found) = b.reconcile(URL, "Report").unwrap() else { panic!("the orphan should be found") };
    assert_eq!(found.status.state, JobState::Pending);
    assert!(p.job(1).unwrap().document.is_none());
    // The caller can clear it.
    r.cancel_job(&found.job).unwrap();
    assert_eq!(p.job(1).unwrap().state, "Canceled");
}

#[test]
fn reconcile_reports_not_found_and_ambiguity_and_only_matches_this_backends_user() {
    let (p, b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    assert_eq!(b.reconcile(URL, "Report").unwrap(), Reconciled::NotFound, "an empty printer");
    r.submit_job(request(Job { title: "Other".into(), ..job(URL) })).unwrap();
    assert_eq!(b.reconcile(URL, "Report").unwrap(), Reconciled::NotFound, "other titles do not match");
    r.submit_job(request(job(URL))).unwrap();
    r.submit_job(request(job(URL))).unwrap();
    let Reconciled::Ambiguous(all) = b.reconcile(URL, "Report").unwrap() else { panic!("two jobs share the title") };
    let ids: Vec<_> = all.iter().map(|m| m.job.native().rsplit('#').next().unwrap_or("").to_owned()).collect();
    assert_eq!(ids, ["3", "2"], "newest first");
    // A backend with another user name does not see them as its own.
    let other = WsdBackend::new().with_transport(MemTransport(p.clone())).with_user_name("someone-else");
    other.add(URL).unwrap();
    assert_eq!(other.reconcile(URL, "Report").unwrap(), Reconciled::NotFound);
    // The title is compared as the backend sent it (cut to 255 characters).
    let long: String = "x".repeat(300);
    r.submit_job(request(Job { title: long.clone(), ..job(URL) })).unwrap();
    assert!(matches!(b.reconcile(URL, &long).unwrap(), Reconciled::Found(_)));
    // Only added printers can be asked.
    assert_eq!(b.reconcile("http://127.0.0.1:5999/print", "Report").unwrap_err().kind(), ErrorKind::NotFound);
}

#[test]
fn reconcile_matches_what_each_ticket_shape_sends() {
    // A name-only ticket sends no user, so the printer's own user name cannot rule the job out.
    let p = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let b = backend(&p).with_ticket_shape(TicketShape::JobNameOnly);
    b.add(URL).unwrap();
    let r = registry(&b);
    r.submit_job(request(job(URL))).unwrap();
    assert!(matches!(b.reconcile(URL, "Report").unwrap(), Reconciled::Found(_)), "a name-only ticket's job is found");
    let fingerprint = craft_print::Fingerprint::new("Report", craft_print::Timestamp::default());
    assert!(matches!(r.find_job(&id(URL), &fingerprint).unwrap(), craft_print::Reconciled::Found(_)), "through the registry too");

    // A title XML cannot carry as it is comes back without the control characters.
    let (_p, b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let odd = "Re\u{1}port\r\n";
    r.submit_job(request(Job { title: odd.into(), ..job(URL) })).unwrap();
    assert!(matches!(b.reconcile(URL, odd).unwrap(), Reconciled::Found(_)), "matched as the printer stored it");

    // An empty ticket names no job: matching is refused rather than reported as nothing found.
    let p = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    let b = backend(&p).with_ticket_shape(TicketShape::Empty);
    b.add(URL).unwrap();
    let r = registry(&b);
    r.submit_job(request(job(URL))).unwrap();
    assert_eq!(b.reconcile(URL, "Report").unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(r.find_job(&id(URL), &fingerprint).unwrap_err().kind(), ErrorKind::Unsupported);
    assert_eq!(b.tag_support(), craft_print::TagSupport::None, "nothing carries a tag");
}

#[test]
fn a_history_the_printer_cannot_give_right_now_is_an_error_not_an_empty_history() {
    // A finished job is only in the history. If a busy printer's fault there read as "no history",
    // reconcile would call the job unknown and a caller would print it again.
    for busy in ["wprt:ServerErrorTemporaryError", "wprt:ServerErrorInternalError", "wprt:OperationFailed"] {
        let (_p, b, r) = setup(FakeConfig { history_fault: Some(busy), ..FakeConfig::default() });
        let submitted = r.submit_job(request(job(URL))).unwrap();
        let outcome = wait_for_job(&r, &submitted.job.unwrap(), fast()).unwrap();
        assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.state == JobState::Completed), "{outcome:?}");
        assert!(b.reconcile(URL, "Report").is_err(), "{busy}: an error, not NotFound");
    }
    // A service with no GetJobHistory at all: the active jobs are all there is to look at.
    let (_p, b, r) = setup(FakeConfig { history_fault: Some("wsa:ActionNotSupported"), ending: Ending::Hangs, ..FakeConfig::default() });
    r.submit_job(request(job(URL))).unwrap();
    assert!(matches!(b.reconcile(URL, "Report").unwrap(), Reconciled::Found(_)), "the active job is still found");
    assert_eq!(b.reconcile(URL, "Other").unwrap(), Reconciled::NotFound);
}

#[test]
fn a_history_longer_than_the_cap_still_finds_the_job_just_sent() {
    // A busy printer lists far more finished jobs than the backend will hold. The reply to
    // SendDocument is lost, so the job can only be found by listing; the list must not be an error.
    let cfg = FakeConfig { history_filler: 300, first_job_id: 1000, ending: Ending::Hangs, ..FakeConfig::default() };
    let (_p, b, r) = setup(cfg.clone());
    let submitted = r.submit_job(request(job(URL))).unwrap();
    // Finish the job so that it is in the history, behind 300 older rows.
    let job_id = submitted.job.unwrap();
    r.cancel_job(&job_id).unwrap();
    assert!(matches!(b.reconcile(URL, "Report").unwrap(), Reconciled::Found(_)), "found among the newest 256");
    let fingerprint = craft_print::Fingerprint::new("Report", craft_print::Timestamp::default());
    assert!(matches!(r.find_job(&id(URL), &fingerprint).unwrap(), craft_print::Reconciled::Found(_)));
    assert!(r.list_jobs(&id(URL), &craft_print::JobFilter::new()).unwrap().iter().any(|j| j.id == job_id));
}

#[test]
fn an_unknown_outcome_submit_is_never_retried_automatically_but_a_safe_refusal_is() {
    let policy = RetryPolicy::default();
    let cancel = CancelToken::new();
    let mut jitter = Jitter::new(11);
    // The reply to SendDocument is lost: one attempt, then the error, and not a second job.
    let (p, _b, r) = lossy(FakeConfig::default(), "SendDocument", 3);
    let clock = ManualClock::new();
    let got = retry(&policy, &clock, &cancel, &mut jitter, |_| r.submit_job(request(job(URL))));
    assert!(got.unwrap_err().outcome_unknown());
    assert_eq!(count(&p, "CreatePrintJob"), 1);
    assert!(clock.sleeps().is_empty(), "no waiting, no second try");
    // A printer that is not taking jobs refuses before accepting anything: safe, so it is retried,
    // waiting at least as long as the printer's hint, until it takes the job.
    let (p, _b, r) = setup(FakeConfig { accept_jobs: false, ..FakeConfig::default() });
    let clock = ManualClock::new();
    let got = retry(&policy, &clock, &cancel, &mut jitter, |attempt| {
        if attempt == 3 {
            p.config.lock().unwrap().accept_jobs = true;
        }
        r.submit_job(request(job(URL)))
    });
    assert!(got.is_ok());
    assert_eq!(count(&p, "CreatePrintJob"), 3, "two refusals, then the job");
    assert_eq!(p.jobs(), 1);
    assert!(
        clock.sleeps().iter().sum::<Duration>() >= Duration::from_secs(30),
        "the printer asked for fifteen seconds each time: {:?}",
        clock.sleeps()
    );
    // And the number of tries is bounded.
    let (p, _b, r) = setup(FakeConfig { accept_jobs: false, ..FakeConfig::default() });
    let clock = ManualClock::new();
    assert!(retry(&policy, &clock, &cancel, &mut jitter, |_| r.submit_job(request(job(URL)))).is_err());
    assert_eq!(count(&p, "CreatePrintJob"), policy.max_attempts as usize);
}

#[test]
fn the_backend_declares_polling_with_a_configurable_interval() {
    let b = WsdBackend::new();
    assert_eq!((b.watch_support(), b.recommended_poll_interval()), (WatchSupport::Poll, Some(Duration::from_secs(2))));
    let b = b.with_poll_interval(Duration::from_millis(750));
    assert_eq!(b.recommended_poll_interval(), Some(Duration::from_millis(750)));
}

// ---- per-job options and typed job ids ----

#[test]
fn per_job_options_reach_the_ticket_and_unlisted_ones_are_dropped_with_a_warning() {
    use craft_print::{ColourMode, JobOptions, Orientation, Quality, Resolution};
    let (p, _b, r) = setup(FakeConfig::default());
    let options = JobOptions::new()
        .with_media("iso_a4_210x297mm")
        .with_orientation(Orientation::Landscape)
        .with_quality(Quality::High)
        .with_resolution(Resolution::new(600, 600))
        .with_colour(ColourMode::Mono)
        .with_expected_pages(4);
    let submitted = r.submit_job(request(job(URL)).with_options(options)).unwrap();
    let t = p.job(1).unwrap().ticket;
    assert!(t.contains("iso_a4_210x297mm") && t.contains(">Landscape<") && t.contains(">High<"), "{t}");
    assert_eq!(submitted.expected.pages, Some(4));
    // A media size the printer does not list is lowered (dropped) with a warning, the rest stays.
    let letter = r
        .submit_job(request(job(URL)).with_options(JobOptions::new().with_media("na_letter_8.5x11in").with_orientation(Orientation::Landscape)))
        .unwrap();
    let t2 = p.job(2).unwrap().ticket;
    assert!(!t2.contains("na_letter") && t2.contains(">Landscape<"), "{t2}");
    assert!(letter.warnings.iter().any(|w| w.contains("na_letter")), "{:?}", letter.warnings);
    // Nothing asked, nothing written.
    r.submit_job(request(job(URL))).unwrap();
    let t3 = p.job(3).unwrap().ticket;
    assert!(!t3.contains("Orientation") && !t3.contains("MediaSizeName") && !t3.contains("PrintQuality"), "{t3}");
}

#[test]
fn an_explicit_colour_choice_overrides_the_grey_flag_and_strict_mode_refuses_what_cannot_be_done() {
    use craft_print::{ColourMode, JobOptions};
    // The fake is a colour printer; colour is chosen when the picture is made, so the bytes are the
    // pass-through PDF either way. What reaches the backend is observable through the plan.
    let (_p, b, r) = setup(FakeConfig { color: false, ..FakeConfig::default() });
    let ok = r.submit_job(request(Job { grayscale: true, ..job(URL) }).with_options(JobOptions::new().with_colour(ColourMode::Mono)));
    assert!(ok.is_ok());
    let strict = b.clone().with_strict(true);
    let e = registry(&strict).submit_job(request(job(URL)).with_options(JobOptions::new().with_colour(ColourMode::Colour))).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{e}");
}

#[test]
fn a_reduced_ticket_refuses_or_drops_per_job_settings_as_before() {
    use craft_print::{JobOptions, Orientation};
    let p = FakePrinter::new(FakeConfig::default());
    let reduced = backend(&p).with_ticket_shape(TicketShape::Description);
    reduced.add(URL).unwrap();
    let options = JobOptions::new().with_orientation(Orientation::Landscape);
    let e = registry(&reduced).submit_job(request(job(URL)).with_options(options.clone())).unwrap_err();
    assert_eq!(e.kind(), ErrorKind::InvalidRequest, "{e}");
    assert_eq!(p.jobs(), 0, "refused before CreatePrintJob");
    let dropping = reduced.clone().with_drop_ticket_settings(true);
    let submitted = registry(&dropping).submit_job(request(job(URL)).with_options(options)).unwrap();
    assert!(submitted.warnings.iter().any(|w| w.contains("orientation")), "{:?}", submitted.warnings);
    assert!(!p.job(1).unwrap().ticket.contains("Orientation"));
}

#[test]
fn a_variant_backend_per_invocation_shares_the_printers_but_not_the_configuration() {
    let p = FakePrinter::new(FakeConfig::default());
    let base = backend(&p);
    base.add(URL).unwrap();
    let variant = base.clone().with_ticket_shape(TicketShape::Empty).with_strict(true);
    assert_eq!(variant.urls(), base.urls(), "the printer list is shared");
    registry(&variant).submit_job(request(job(URL))).unwrap();
    assert_eq!(p.job(1).unwrap().ticket, "<wprt:PrintTicket/>");
    // The base backend still sends the full ticket.
    registry(&base).submit_job(request(job(URL))).unwrap();
    assert!(p.job(2).unwrap().ticket.contains("JobOriginatingUserName"));
}

#[test]
fn wsd_job_ids_are_built_and_taken_apart_without_string_handling() {
    let id = WsdBackend::job_id("http://127.0.0.1:5357/print", 12).unwrap();
    assert_eq!((id.backend(), id.native()), ("wsd", "http://127.0.0.1:5357/print#12"));
    let job = WsdBackend::parse_job_id(&id).unwrap();
    assert_eq!((job.printer_url.as_str(), job.job_number), ("http://127.0.0.1:5357/print", 12));
    assert_eq!(WsdBackend::parse_native(id.native()).unwrap(), job);
    assert_eq!(id.to_string().parse::<JobId>().unwrap(), id, "the text form round-trips");
    // Refusals are typed values.
    assert_eq!(WsdBackend::job_id("http://printer.example/p", 1).unwrap_err().kind(), ErrorKind::InvalidRequest);
    for bad in ["", "no-number", "http://127.0.0.1:5357/print#x", "http://127.0.0.1:5357/print#-1", "http://printer.example/p#1", "#1"] {
        assert_eq!(WsdBackend::parse_native(bad).unwrap_err().kind(), ErrorKind::InvalidRequest, "{bad:?}");
    }
    let foreign = JobId::new("lp", "q-1");
    assert_eq!(WsdBackend::parse_job_id(&foreign).unwrap_err().kind(), ErrorKind::InvalidRequest);
    // A backend asked about a job of another backend refuses it without sending anything.
    let (p, b, _r) = setup(FakeConfig::default());
    let log = p.log().len();
    assert_eq!(b.job_status(&foreign).unwrap_err().kind(), ErrorKind::InvalidRequest);
    assert_eq!(b.cancel_job(&foreign).unwrap_err().kind(), ErrorKind::InvalidRequest);
    assert_eq!(p.log().len(), log);
}

#[test]
fn a_typed_wsd_job_id_is_accepted_by_status_cancel_and_watch() {
    let (_p, _b, r) = setup(FakeConfig { ending: Ending::Hangs, ..FakeConfig::default() });
    r.submit_job(request(job(URL))).unwrap();
    let id = WsdBackend::job_id(URL, 1).unwrap();
    assert_eq!(r.job_status(&id).unwrap().state, JobState::Pending);
    assert!(r.job_watch(&id, fast()).is_ok());
    r.cancel_job(&id).unwrap();
    assert_eq!(r.job_status(&id).unwrap().state, JobState::Cancelled);
}

#[test]
fn expected_pages_give_the_sheet_counter_a_total_that_the_printer_does_not() {
    use craft_print::JobOptions;
    let (_p, _b, r) = setup(FakeConfig::default());
    let submitted = r.submit_job(request(job(URL)).with_options(JobOptions::new().with_expected_pages(1))).unwrap();
    let mut watch = JobWatch::for_submitted(&submitted, fast()).unwrap();
    let mut seen = Vec::new();
    let outcome = watch.run(&r, &mut |step| {
        if let WatchStep::Update(s) = step {
            seen.push((s.sheets_done, s.sheets_total, s.sheet_fraction()));
        }
    });
    assert!(outcome.is_success(), "{outcome:?}");
    assert!(seen.iter().all(|(_, total, _)| *total == Some(1)), "the printer reports no total, the caller's page count stands in: {seen:?}");
    assert!(matches!(outcome, WatchOutcome::Terminal(ref s) if s.sheet_fraction() == Some(1.0)));
}

// ---- Photo quality, more formats, resolutions and registry conversion ----

#[test]
fn details_name_the_newer_formats_and_list_the_resolutions() {
    let config = FakeConfig {
        formats: vec![
            "image/jpeg",
            "IMAGE/PNG",
            "image/tiff",
            "text/plain",
            "application/vnd.ms-xpsdocument",
            "application/oxps",
            "application/vnd.vendor-pcl",
            "image/pwg-raster",
        ],
        resolutions: vec![(300, 300), (600, 600), (1200, 600)],
        ..FakeConfig::default()
    };
    let (_p, _b, r) = setup(config);
    let d = r.printer_details(&id(URL)).unwrap();
    use DocumentFormat::*;
    assert_eq!(d.formats, [Jpeg, Png, Tiff, PlainText, Xps, OpenXps, PwgRaster]);
    assert_eq!(d.resolutions, [Resolution::square(300), Resolution::square(600), Resolution::new(1200, 600)]);
    // Nothing published, nothing listed.
    let (_p, _b, r) = setup(FakeConfig::default());
    assert!(r.printer_details(&id(URL)).unwrap().resolutions.is_empty());
}

#[test]
fn photo_quality_reaches_the_ticket_when_the_printer_lists_it() {
    let (p, _b, r) = setup(FakeConfig { qualities: vec!["Normal", "High", "Photo"], ..FakeConfig::default() });
    let submitted = r.submit_job(request(job(URL)).with_options(JobOptions::new().with_quality(Quality::Photo))).unwrap();
    assert!(p.job(1).unwrap().ticket.contains(">Photo<"), "{}", p.job(1).unwrap().ticket);
    assert!(submitted.warnings.is_empty(), "{:?}", submitted.warnings);
    // A printer that publishes no quality list is not second-guessed.
    let (p, _b, r) = setup(FakeConfig::default());
    r.submit_job(request(job(URL)).with_options(JobOptions::new().with_quality(Quality::Photo))).unwrap();
    assert!(p.job(1).unwrap().ticket.contains(">Photo<"));
}

#[test]
fn photo_quality_is_dropped_with_a_warning_when_unlisted_and_refused_in_strict_mode() {
    let (p, b, r) = setup(FakeConfig { qualities: vec!["Draft", "Normal", "High"], ..FakeConfig::default() });
    let submitted = r.submit_job(request(job(URL)).with_options(JobOptions::new().with_quality(Quality::Photo))).unwrap();
    assert!(!p.job(1).unwrap().ticket.contains("Photo"), "{}", p.job(1).unwrap().ticket);
    assert!(submitted.warnings.iter().any(|w| w.contains("Photo")), "{:?}", submitted.warnings);
    // High, which the same printer lists, is kept.
    r.submit_job(request(job(URL)).with_options(JobOptions::new().with_quality(Quality::High))).unwrap();
    assert!(p.job(2).unwrap().ticket.contains(">High<"));
    // Strict mode refuses before anything is created.
    let strict = registry(&b.clone().with_strict(true));
    let error = strict.submit_job(request(job(URL)).with_options(JobOptions::new().with_quality(Quality::Photo))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::InvalidRequest, "{error}");
    assert_eq!(p.jobs(), 2);
}

#[test]
fn a_document_in_a_listed_format_goes_to_the_printer_unchanged() {
    let formats =
        vec!["image/jpeg", "image/png", "image/tiff", "text/plain", "application/vnd.ms-xpsdocument", "application/oxps", "application/postscript"];
    let (p, _b, r) = setup(FakeConfig { formats, ..FakeConfig::default() });
    let samples = [
        (DocumentFormat::Jpeg, &b"\xff\xd8\xff\xe0jpeg"[..]),
        (DocumentFormat::Png, b"\x89PNG\r\n\x1a\npng"),
        (DocumentFormat::Tiff, b"II*\0tiff"),
        (DocumentFormat::PlainText, b"hello\n"),
        (DocumentFormat::Xps, b"PK\x03\x04xps"),
        (DocumentFormat::OpenXps, b"PK\x03\x04oxps"),
        (DocumentFormat::PostScript, b"%!PS\n"),
    ];
    for (n, (format, bytes)) in samples.into_iter().enumerate() {
        let submitted = r.submit_job(SubmitRequest::new(Document::new(format, bytes), job(URL)).with_consent(UserConfirmed::interactive())).unwrap();
        assert!(submitted.conversion.is_none());
        let sent = p.job(u32::try_from(n).unwrap() + 1).unwrap();
        assert_eq!((sent.format.as_deref(), sent.document.as_deref()), (Some(format.mime()), Some(bytes)), "{format:?}");
    }
    // PDF is not in this printer's list: refused by the backend, no job made.
    let before = p.jobs();
    let error = r.submit_job(request(job(URL))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert_eq!(p.jobs(), before);
}

/// Renders a blank page per request and packs it with the public PWG encoder.
struct Blank;

impl craft_print::DocumentConverter for Blank {
    fn targets(&self, from: DocumentFormat) -> Vec<DocumentFormat> {
        if from == DocumentFormat::Pdf { vec![DocumentFormat::Jpeg, DocumentFormat::PwgRaster] } else { Vec::new() }
    }
    fn convert(
        &self,
        _document: Document<'_>,
        target: DocumentFormat,
        context: &craft_print::ConvertContext,
    ) -> Result<craft_print::OwnedDocument, craft_print::PrintError> {
        let (w, h) = (context.resolution.x_dpi / 10, context.resolution.y_dpi / 10);
        let channels = if context.colour { 3 } else { 1 };
        let page = craft_print::pwg::RasterPage { width: w, height: h, gray: !context.colour, pixels: vec![255; (w * h * channels) as usize] };
        let options =
            craft_print::pwg::PwgOptions { dpi: (context.resolution.x_dpi, context.resolution.y_dpi), duplex: craft_print::pwg::Duplex::Off };
        let bytes = craft_print::pwg::encode(&[page], &options).map_err(|e| craft_print::PrintError::new(ErrorKind::Failed, e.to_string()))?;
        Ok(craft_print::OwnedDocument::new(target, bytes))
    }
}

#[test]
fn the_registry_converts_for_a_wsd_printer_that_lists_only_raster_and_picks_its_resolution() {
    let config = FakeConfig { formats: vec!["image/pwg-raster"], resolutions: vec![(150, 150), (600, 600), (1200, 1200)], ..FakeConfig::default() };
    let (p, _b, r) = setup(config);
    r.set_converter(Arc::new(Blank));
    let submitted = r.submit_job(request(job(URL))).unwrap();
    let conversion = submitted.conversion.unwrap();
    assert_eq!((conversion.from, conversion.to, conversion.resolution), (DocumentFormat::Pdf, DocumentFormat::PwgRaster, Resolution::square(150)));
    let sent = p.job(1).unwrap();
    assert_eq!(sent.format.as_deref(), Some("image/pwg-raster"));
    assert_eq!(&sent.document.unwrap()[..4], b"RaS2");
    assert!(sent.ticket.contains("150"), "the raster's resolution is in the ticket: {}", sent.ticket);
    // One read-only request for the registry to choose, one inside the backend's submit.
    assert_eq!(count(&p, "GetPrinterElements"), 2);
}

#[test]
fn details_the_caller_already_has_save_the_registrys_request() {
    let config = FakeConfig { formats: vec!["image/pwg-raster"], resolutions: vec![(300, 300)], ..FakeConfig::default() };
    let (p, _b, r) = setup(config);
    r.set_converter(Arc::new(Blank));
    let details = r.printer_details(&id(URL)).unwrap();
    assert_eq!(count(&p, "GetPrinterElements"), 1);
    r.submit_job_with_details(request(job(URL)), Some(details)).unwrap();
    assert_eq!(count(&p, "GetPrinterElements"), 2, "only the backend's own read remains");
    assert_eq!(p.jobs(), 1);
}

#[test]
fn a_printer_that_lists_pdf_is_sent_pdf_even_with_a_converter_registered() {
    let (p, _b, r) = setup(FakeConfig::default());
    r.set_converter(Arc::new(Blank));
    let submitted = r.submit_job(request(job(URL))).unwrap();
    assert!(submitted.conversion.is_none());
    assert_eq!(p.job(1).unwrap().format.as_deref(), Some("application/pdf"));
}

#[test]
fn without_a_common_format_or_a_converter_nothing_is_sent_to_the_printer() {
    let (p, _b, r) = setup(FakeConfig { formats: vec!["image/tiff"], ..FakeConfig::default() });
    let error = r.submit_job(request(job(URL))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    r.set_converter(Arc::new(Blank));
    let error = r.submit_job(request(job(URL))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::UnsupportedFormat);
    assert!(error.message().contains("image/tiff") && error.message().contains("image/pwg-raster"), "{}", error.message());
    assert_eq!(p.jobs(), 0);
}

#[test]
fn a_network_printer_is_not_asked_for_a_conversion_without_consent() {
    let (p, _b, r) = setup(FakeConfig { formats: vec!["image/pwg-raster"], ..FakeConfig::default() });
    r.set_converter(Arc::new(Blank));
    let error = r.submit_job(SubmitRequest::new(Document::pdf(PDF), job(URL))).unwrap_err();
    assert_eq!(error.kind(), ErrorKind::ConsentRequired);
    assert!(p.log().is_empty());
}

#[test]
fn the_old_wsd_encoder_paths_still_work_and_share_the_neutral_encoder() {
    use craft_print::wsd::pwg;
    let page = craft_print::pwg::RasterPage { width: 3, height: 2, gray: true, pixels: vec![9; 6] };
    let options = craft_print::pwg::PwgOptions { dpi: (300, 300), duplex: craft_print::pwg::Duplex::Off };
    assert_eq!(pwg::encode(std::slice::from_ref(&page), &options).unwrap(), craft_print::pwg::encode(&[page], &options).unwrap());
}
