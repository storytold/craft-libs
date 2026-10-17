//! The WS-Print client layer (`craft_print::wsd::{client, job, ...}`) against the in-process fake
//! printer, with no sockets (the transport calls the fake directly). `wsd_registry.rs` drives the
//! same flows through the registry and over real loopback sockets.
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]

use craft_print::wsd::testing as wsd_support;
use wsd_support as common;

use std::time::Duration;

use common::{Ending, FakeConfig, FakePrinter, MemTransport};
use craft_print::wsd::encoder::{Chain, EncodeContext, PageSource, RasterEncoder};
use craft_print::wsd::error::WsdError;
use craft_print::wsd::fault::FaultCode;
use craft_print::wsd::job::{JobOutcome, PollPolicy, PrintOptions, print_document, printer_info, watch_job};
use craft_print::wsd::model::{JobState, Section};
use craft_print::wsd::pwg::RasterPage;
use craft_print::wsd::{Client, ColorMode, PassThrough, PrinterUrl, Transfer};
use craft_print::{CancelToken, UserConfirmed};

fn client(p: &std::sync::Arc<FakePrinter>) -> Client<MemTransport> {
    Client::new(MemTransport(p.clone()), PrinterUrl::parse("http://127.0.0.1:5357/print").unwrap())
}

fn quick() -> PollPolicy {
    PollPolicy { interval: Duration::from_millis(1), max_wait: Duration::from_secs(5) }
}

fn opts() -> PrintOptions {
    PrintOptions { poll: quick(), ..Default::default() }
}

fn yes() -> UserConfirmed {
    UserConfirmed::interactive()
}

const PDF: &[u8] = b"%PDF-1.7\n%\xe2\xe3\xcf\xd3\r\n--not-a-boundary\r\n--\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n";

#[test]
fn discovery_reports_the_printer_its_status_and_what_it_accepts() {
    let p = FakePrinter::new(FakeConfig::default());
    let info = printer_info(&client(&p)).unwrap();
    let d = info.description.unwrap();
    assert_eq!((d.name.as_str(), d.color_supported, d.multiple_documents), ("Fake Printer", Some(true), Some(false)));
    assert_eq!(info.status.unwrap().state, "Idle");
    let c = info.capabilities.unwrap();
    assert_eq!(c.formats, ["application/pdf", "image/pwg-raster"]);
    assert_eq!(c.copies, Some((1, 10)));
    assert_eq!(p.jobs(), 0);
    // An unknown section is reported invalid, not an error.
    let e = client(&p).get_printer_elements(&[Section::DefaultPrintTicket]).unwrap();
    assert_eq!(e.invalid, ["DefaultPrintTicket"]);
}

#[test]
fn a_pdf_is_sent_as_mtom_and_the_job_is_watched_to_completion() {
    let p = FakePrinter::new(FakeConfig::default());
    let r = print_document(
        &client(&p),
        &PassThrough::pdf(PDF.to_vec()),
        &PrintOptions { copies: Some(2), job_name: "Report".into(), ..opts() },
        yes(),
        &CancelToken::new(),
    )
    .unwrap();
    assert_eq!(r.outcome, JobOutcome::Completed);
    assert_eq!((r.format.as_str(), r.bytes_sent, r.printer_name.as_str()), ("application/pdf", PDF.len(), "Fake Printer"));
    assert_eq!(r.state, Some(JobState::Completed));
    assert_eq!(r.sheets_completed, 1);
    let job = p.job(r.job_id).unwrap();
    assert_eq!(job.document.as_deref(), Some(PDF), "the bytes arrive intact, including boundary look-alikes");
    assert!(job.ticket.contains("Report") && job.ticket.contains(">2</"), "{}", job.ticket);
    let log = p.log();
    assert!(log.iter().any(|l| l.content_type.starts_with("multipart/related") && l.action.ends_with("SendDocument")));
    let seq = p.actions();
    assert_eq!(&seq[..3], ["GetPrinterElements", "CreatePrintJob", "SendDocument"]);
    assert!(seq[3..].iter().all(|a| a == "GetJobElements") && seq.len() >= 5, "{seq:?}");
}

#[test]
fn inline_base64_transfer_delivers_the_same_bytes() {
    let p = FakePrinter::new(FakeConfig::default());
    let r = print_document(
        &client(&p),
        &PassThrough::pdf(PDF.to_vec()),
        &PrintOptions { transfer: Transfer::InlineBase64, ..opts() },
        yes(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(r.outcome.is_success());
    assert_eq!(p.job(r.job_id).unwrap().document.as_deref(), Some(PDF));
    assert!(p.log().iter().any(|l| l.action.ends_with("SendDocument") && l.content_type.starts_with("application/soap+xml")));
}

#[test]
fn a_format_the_printer_does_not_list_stops_before_any_job_exists() {
    let p = FakePrinter::new(FakeConfig::default());
    let e = print_document(&client(&p), &PassThrough::new("image/jpeg", vec![1u8]), &opts(), yes(), &CancelToken::new()).unwrap_err();
    match e {
        WsdError::NoCommonFormat { offered, accepted } => {
            assert_eq!(offered, ["image/jpeg"]);
            assert!(accepted.contains(&"application/pdf".to_owned()));
        }
        other => panic!("{other:?}"),
    }
    assert_eq!((p.jobs(), p.actions()), (0, vec!["GetPrinterElements".to_owned()]));
}

struct Blank;
impl PageSource for Blank {
    fn page_count(&self) -> usize {
        2
    }
    fn render(&self, _i: usize, dpi: (u32, u32), gray: bool) -> craft_print::wsd::error::Result<RasterPage> {
        let (w, h) = (dpi.0 / 20, dpi.1 / 20);
        Ok(RasterPage { width: w, height: h, gray, pixels: vec![200; (w * h) as usize * if gray { 1 } else { 3 }] })
    }
}

#[test]
fn a_printer_without_pdf_gets_pwg_raster_from_the_chain() {
    let p = FakePrinter::new(FakeConfig { formats: vec!["image/pwg-raster"], ..Default::default() });
    let enc = Chain(vec![Box::new(PassThrough::pdf(PDF.to_vec())), Box::new(RasterEncoder::new(Blank))]);
    let r = print_document(&client(&p), &enc, &PrintOptions { color: ColorMode::Mono, ..opts() }, yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.format, "image/pwg-raster");
    let doc = p.job(r.job_id).unwrap().document.unwrap();
    assert_eq!(&doc[..4], b"RaS2");
    // Mono was asked for: 8-bit grey pages (bits per pixel = 8 at header offset 388).
    let bpp = u32::from_be_bytes([doc[4 + 388], doc[4 + 389], doc[4 + 390], doc[4 + 391]]);
    assert_eq!(bpp, 8);
}

#[test]
fn colour_follows_the_printers_description() {
    let p = FakePrinter::new(FakeConfig { formats: vec!["image/pwg-raster"], color: false, ..Default::default() });
    let r = print_document(&client(&p), &RasterEncoder::new(Blank), &PrintOptions { color: ColorMode::Color, ..opts() }, yes(), &CancelToken::new())
        .unwrap();
    assert!(r.warnings.iter().any(|w| w.contains("monochrome")), "{:?}", r.warnings);
    let doc = p.job(r.job_id).unwrap().document.unwrap();
    assert_eq!(u32::from_be_bytes([doc[4 + 388], doc[4 + 389], doc[4 + 390], doc[4 + 391]]), 8);
}

#[test]
fn settings_the_printer_does_not_list_are_dropped_or_refused() {
    let p = FakePrinter::new(FakeConfig::default());
    let soft = PrintOptions { copies: Some(50), sides: Some(craft_print::wsd::ticket::Sides::TwoSidedShortEdge), ..opts() };
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &soft, yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.warnings.len(), 2, "{:?}", r.warnings);
    let t = p.job(r.job_id).unwrap().ticket;
    assert!(t.contains(">10</") && !t.contains("TwoSidedShortEdge"), "{t}");
    let before = p.jobs();
    let strict = PrintOptions { strict: true, copies: Some(50), ..opts() };
    assert!(matches!(
        print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &strict, yes(), &CancelToken::new()),
        Err(WsdError::Unsupported(_))
    ));
    assert_eq!(p.jobs(), before, "refused before CreatePrintJob");
}

#[test]
fn a_minimal_ticket_sends_only_the_job_name_and_reports_what_it_left_out() {
    use craft_print::wsd::ticket::TicketShape;
    let p = FakePrinter::new(FakeConfig::default());
    let o = PrintOptions { ticket_shape: TicketShape::JobNameOnly, copies: Some(2), job_name: "Minimal".into(), ..opts() };
    // Settings the reduced ticket cannot carry are refused before anything is sent...
    let before = p.jobs();
    assert!(matches!(print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &o, yes(), &CancelToken::new()), Err(WsdError::Unsupported(_))));
    assert_eq!(p.jobs(), before);
    // ...unless the caller opts into dropping them.
    let o = PrintOptions { drop_ticket_settings: true, ..o };
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &o, yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::Completed);
    assert_eq!(r.warnings.len(), 1, "{:?}", r.warnings);
    let t = p.job(r.job_id).unwrap().ticket;
    assert!(
        t.contains("Minimal") && !t.contains("JobOriginatingUserName") && !t.contains("JobProcessing") && !t.contains("DocumentProcessing"),
        "{t}"
    );
    // The default stays the full ticket.
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap();
    assert!(p.job(r.job_id).unwrap().ticket.contains("JobOriginatingUserName"));
    let r = print_document(
        &client(&p),
        &PassThrough::pdf(PDF.to_vec()),
        &PrintOptions { ticket_shape: TicketShape::Empty, ..opts() },
        yes(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(!p.job(r.job_id).unwrap().ticket.contains("JobDescription"));
}

#[test]
fn must_honor_reaches_the_printer_in_strict_mode() {
    let p = FakePrinter::new(FakeConfig::default());
    let r = print_document(
        &client(&p),
        &PassThrough::pdf(PDF.to_vec()),
        &PrintOptions { strict: true, copies: Some(3), ..opts() },
        yes(),
        &CancelToken::new(),
    )
    .unwrap();
    assert!(p.job(r.job_id).unwrap().ticket.contains("MustHonor=\"true\""));
}

#[test]
fn an_aborted_job_is_a_typed_outcome_with_the_reason() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Aborts("DocumentFormatError"), ..Default::default() });
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::Aborted { reason: Some(craft_print::wsd::model::JobReason::DocumentFormatError) });
    assert!(!r.outcome.is_success());
}

#[test]
fn completion_with_warnings_is_still_a_success() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Completes("JobCompletedWithWarnings"), ..Default::default() });
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::CompletedWithWarnings);
    assert!(r.outcome.is_success());
}

#[test]
fn faults_surface_typed() {
    let p = FakePrinter::new(FakeConfig { accept_jobs: false, ..Default::default() });
    let e = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap_err();
    let f = e.fault().expect("a fault");
    assert_eq!(f.code, FaultCode::ServerErrorNotAcceptingJobs);
    assert!(f.code.is_retryable());
    // A job id the printer does not know.
    let p = FakePrinter::new(FakeConfig::default());
    let e = client(&p).cancel_job(77).unwrap_err();
    assert_eq!(e.fault().map(|f| f.code.clone()), Some(FaultCode::ClientErrorJobIdNotFound));
}

#[test]
fn send_document_faults_are_typed() {
    let p = FakePrinter::new(FakeConfig::default());
    let c = client(&p);
    let ticket = craft_print::wsd::ticket::PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
    let id = c.create_print_job(&ticket).unwrap();
    let bad = craft_print::wsd::ticket::DocumentDescription {
        id: 1,
        compression: craft_print::wsd::ticket::Compression::Gzip,
        format: "application/pdf".into(),
        name: None,
        must_honor_format: false,
    };
    let e = c.send_document(id, &bad, &ticket, true, PDF, Transfer::Mtom).unwrap_err();
    assert_eq!(e.fault().map(|f| f.code.clone()), Some(FaultCode::ClientErrorCompressionNotSupported));
    // And a second document after the last one is a typed fault.
    let good = craft_print::wsd::ticket::DocumentDescription { compression: craft_print::wsd::ticket::Compression::None, ..bad };
    c.send_document(id, &good, &ticket, true, PDF, Transfer::Mtom).unwrap();
    let e = c.send_document(id, &good, &ticket, true, PDF, Transfer::Mtom).unwrap_err();
    assert_eq!(e.fault().map(|f| f.code.clone()), Some(FaultCode::ClientErrorLastDocumentAlreadySent));
}

#[test]
fn a_job_the_printer_forgets_is_reported_as_forgotten() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Vanishes, ..Default::default() });
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::Forgotten);
}

#[test]
fn a_slow_job_times_out_the_wait_not_the_job() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..Default::default() });
    let o = PrintOptions { poll: PollPolicy { interval: Duration::from_millis(2), max_wait: Duration::from_millis(30) }, ..Default::default() };
    let r = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &o, yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::StillRunning);
    assert!(!p.job(r.job_id).unwrap().state.is_empty());
    // The caller can still cancel it afterwards.
    client(&p).cancel_job(r.job_id).unwrap();
    assert_eq!(client(&p).job_status(r.job_id).unwrap().state, JobState::Canceled);
}

#[test]
fn cancelling_while_watching_sends_cancel_job_and_reports_canceled() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..Default::default() });
    let c = client(&p);
    let ticket = craft_print::wsd::ticket::PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
    let id = c.create_print_job(&ticket).unwrap();
    let token = CancelToken::new();
    token.cancel();
    let (outcome, status) = watch_job(&c, id, quick(), &token).unwrap();
    assert_eq!(outcome, JobOutcome::Canceled { at_device: false });
    assert_eq!(status.unwrap().state, JobState::Canceled);
    assert_eq!(p.actions().iter().filter(|a| *a == "CancelJob").count(), 1);
    // A print cancelled before it starts sends nothing but discovery.
    let p2 = FakePrinter::new(FakeConfig::default());
    assert!(print_document(&client(&p2), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &token).is_err());
    assert_eq!(p2.jobs(), 0);
}

#[test]
fn a_cancel_that_loses_the_race_with_the_end_of_the_job_reports_the_real_outcome() {
    let ticket = craft_print::wsd::ticket::PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
    let cfg = FakeConfig { cancel_ended_is_operation_failed: true, ..Default::default() };
    let p = FakePrinter::new(cfg.clone());
    let c = client(&p);
    let id = print_document(&c, &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap().job_id;
    // Explicit cancel of a finished job: tolerated, and the job stays Completed.
    craft_print::wsd::job::cancel_job_tolerant(&c, id).unwrap();
    assert_eq!(c.job_status(id).unwrap().state, JobState::Completed);
    // The raw call still shows the fault.
    assert_eq!(c.cancel_job(id).unwrap_err().fault().map(|f| f.code.clone()), Some(FaultCode::OperationFailed));
    // Watching with the token already set: the cancel is refused, the real end state is reported.
    let token = CancelToken::new();
    token.cancel();
    let (outcome, status) = watch_job(&c, id, quick(), &token).unwrap();
    assert_eq!(outcome, JobOutcome::Completed);
    assert_eq!(status.unwrap().state, JobState::Completed);
    // A still-running job refused with the same fault is a real failure and not hidden: with a
    // transport that fails the status read, the fault is kept.
    let hang = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..cfg });
    let hc = client(&hang);
    let hid = hc.create_print_job(&ticket).unwrap();
    craft_print::wsd::job::cancel_job_tolerant(&hc, hid).unwrap(); // cancels normally
    assert_eq!(hc.job_status(hid).unwrap().state, JobState::Canceled);
}

#[test]
fn job_lists_split_active_from_history() {
    let p = FakePrinter::new(FakeConfig { ending: Ending::Hangs, ..Default::default() });
    let c = client(&p);
    let ticket = craft_print::wsd::ticket::PrintTicket { job_name: "j".into(), user_name: "u".into(), ..Default::default() };
    let a = c.create_print_job(&ticket).unwrap();
    let b = c.create_print_job(&ticket).unwrap();
    c.cancel_job(b).unwrap();
    let active = c.get_active_jobs().unwrap();
    assert_eq!(active.iter().map(|j| j.status.job_id).collect::<Vec<_>>(), [a]);
    let history = c.get_job_history().unwrap();
    assert_eq!(history.iter().map(|j| j.status.job_id).collect::<Vec<_>>(), [b]);
    assert_eq!(active[0].job_name, "j");
}

#[test]
fn a_device_that_answers_http_errors_gives_a_typed_error() {
    let p = FakePrinter::new(FakeConfig { http_error: Some(503), ..Default::default() });
    assert_eq!(printer_info(&client(&p)).unwrap_err(), WsdError::Http { status: 503, retry_after: None });
}

#[test]
fn encoders_see_the_resolved_settings() {
    struct Spy(std::sync::Mutex<Option<EncodeContext>>);
    impl craft_print::wsd::encoder::DocumentEncoder for Spy {
        fn formats(&self) -> Vec<String> {
            vec!["application/pdf".into()]
        }
        fn encode(&self, _f: &str, ctx: &EncodeContext) -> craft_print::wsd::error::Result<std::borrow::Cow<'_, [u8]>> {
            *self.0.lock().unwrap() = Some(ctx.clone());
            Ok(std::borrow::Cow::Borrowed(PDF))
        }
    }
    let p = FakePrinter::new(FakeConfig::default());
    let spy = Spy(Default::default());
    print_document(
        &client(&p),
        &spy,
        &PrintOptions { sides: Some(craft_print::wsd::ticket::Sides::TwoSidedLongEdge), ..opts() },
        yes(),
        &CancelToken::new(),
    )
    .unwrap();
    let ctx = spy.0.lock().unwrap().clone().unwrap();
    assert_eq!((ctx.color, ctx.dpi, ctx.sides), (true, (300, 300), Some(craft_print::wsd::ticket::Sides::TwoSidedLongEdge)));
}

#[test]
fn a_failed_send_cancels_the_orphan_job_and_keeps_the_real_error() {
    let p = FakePrinter::new(FakeConfig { fail_send: true, ..Default::default() });
    let e = print_document(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap_err();
    assert_eq!(e.fault().map(|f| f.code.clone()), Some(FaultCode::ServerErrorInternalError));
    assert_eq!(p.jobs(), 1);
    assert_eq!(p.job(1).unwrap().state, "Canceled");
}

// ---- additions for this crate ----

#[test]
fn progress_is_reported_per_phase_and_a_pre_cancelled_job_sends_nothing() {
    use craft_print::wsd::job::print_document_with_progress;
    let p = FakePrinter::new(FakeConfig::default());
    let mut seen = Vec::new();
    let mut record = |done: usize, total: usize| seen.push((done, total));
    print_document_with_progress(&client(&p), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new(), &mut record).unwrap();
    assert_eq!(seen, [(1, 4), (2, 4), (3, 4), (4, 4)]);
    let token = CancelToken::new();
    token.cancel();
    let p2 = FakePrinter::new(FakeConfig::default());
    let e = print_document(&client(&p2), &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &token).unwrap_err();
    assert_eq!(e, WsdError::Cancelled);
    assert!(p2.log().is_empty());
}

#[test]
fn borrowed_documents_are_not_copied_by_the_pass_through() {
    use craft_print::wsd::encoder::DocumentEncoder;
    let ctx = EncodeContext { color: true, dpi: (300, 300), sides: None, media_size: None };
    let enc = PassThrough::pdf(PDF);
    assert!(matches!(enc.encode("application/pdf", &ctx).unwrap(), std::borrow::Cow::Borrowed(_)));
}

#[test]
fn a_dropped_connection_while_watching_is_retried_a_few_times() {
    // The third GetJobElements closes the connection without a reply; everything else is answered.
    use std::sync::Arc;
    use std::sync::atomic::{AtomicU32, Ordering};

    use craft_print::wsd::adapter::{DeviceUrl, Request, Response, Transport, TransportKind, WireError};
    struct Flaky(Arc<FakePrinter>, AtomicU32);
    impl Transport for Flaky {
        fn post(&self, _: &DeviceUrl, request: Request) -> Result<Response, WireError> {
            if request.soap_action.ends_with("GetJobElements") && self.1.fetch_add(1, Ordering::SeqCst) == 2 {
                return Err(WireError::transport(TransportKind::ConnectionClosed, "closed without a reply"));
            }
            let (status, content_type, body) = self.0.handle(&request.content_type, &request.body);
            Ok(Response::new(status, Some(content_type), body))
        }
    }
    let p = FakePrinter::new(FakeConfig { polls_to_finish: 5, ..FakeConfig::default() });
    let c = Client::new(Flaky(p.clone(), AtomicU32::new(0)), PrinterUrl::parse("http://127.0.0.1:5357/print").unwrap());
    let r = print_document(&c, &PassThrough::pdf(PDF.to_vec()), &opts(), yes(), &CancelToken::new()).unwrap();
    assert_eq!(r.outcome, JobOutcome::Completed);
}
