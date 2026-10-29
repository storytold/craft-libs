//! Against a real `fakeprint` server, only when `FAKEPRINT_URL` names its WS-Print service on a
//! loopback address (for example `http://127.0.0.1:5357/print`). The tests are `#[ignore]`d, so
//! nothing here runs unless asked for, and the URL goes through the same policy as any printer URL.
//!
//! Run: `FAKEPRINT_URL=http://127.0.0.1:PORT/PATH cargo test -p craft-print --test wsd_live -- --ignored`
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]

use std::time::Duration;

use craft_print::backends::Backend;
use craft_print::wsd::{PrinterUrl, WsdBackend};
use craft_print::{Document, ErrorKind, Job, Registry, SubmitRequest, UserConfirmed, WaitPolicy, namespaced_id, wait_for_job};

fn live() -> (String, Registry) {
    let url = std::env::var("FAKEPRINT_URL").ok().filter(|u| !u.is_empty()).expect("set FAKEPRINT_URL to run the ignored live tests");
    let parsed = PrinterUrl::parse(&url).expect("FAKEPRINT_URL must be a numeric loopback or private http URL");
    assert!(parsed.host().is_loopback(), "the live test only talks to loopback");
    let backend = WsdBackend::new().with_poll_interval(Duration::from_millis(200));
    let native = backend.add(&url).expect("the URL is allowed");
    let registry = Registry::new();
    registry.register(Box::new(backend)).expect("register");
    (namespaced_id("wsd", &native), registry)
}

#[test]
#[ignore = "needs FAKEPRINT_URL"]
fn the_printer_describes_itself() {
    let (id, registry) = live();
    let listing = registry.list_printers().expect("listing");
    assert_eq!(listing.items.len(), 1);
    let details = registry.printer_details(&id).expect("GetPrinterElements");
    assert!(!details.formats.is_empty(), "a conforming printer lists its document formats");
}

#[test]
#[ignore = "needs FAKEPRINT_URL"]
fn a_document_prints_to_an_end_state() {
    let (id, registry) = live();
    let details = registry.printer_details(&id).expect("GetPrinterElements");
    let format = *details.formats.first().expect("a listed format");
    let bytes: &[u8] = b"%PDF-1.4\n1 0 obj<</Type/Catalog>>endobj\ntrailer<</Root 1 0 R>>\n%%EOF\n";
    let job = Job { printer: Some(id), title: "craft-print live test".into(), ..Job::default() };
    let request = SubmitRequest::new(Document::new(format, bytes), job).with_consent(UserConfirmed::interactive());
    match registry.submit_job(request) {
        Ok(submitted) => {
            assert_eq!(submitted.backend, "wsd");
            // Accepted is not printed: follow it for a bounded time.
            let job = submitted.job.expect("a job handle");
            let outcome = wait_for_job(&registry, &job, WaitPolicy::default().with_max_wait(Duration::from_secs(30))).expect("routable");
            assert!(!matches!(outcome, craft_print::WatchOutcome::Failed(_)), "{outcome:?}");
        }
        // The fake may judge a stub document of another format harshly; a typed refusal is fine.
        Err(e) => assert!(matches!(e.kind(), ErrorKind::UnsupportedFormat | ErrorKind::Rejected | ErrorKind::Failed), "{e}"),
    }
}

#[test]
fn the_backend_is_a_network_backend() {
    assert_eq!(WsdBackend::new().class(), craft_print::BackendClass::Network);
}
