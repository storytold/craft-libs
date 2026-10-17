//! Push status for the WS-Print backend: the listener against forged, malformed, oversized,
//! duplicated and flooding senders (this test is a client of OUR listener, over a loopback socket,
//! sending static bytes), the subscription manager against canned WS-Eventing replies from a small
//! scripted transport, renew / expiry / `SubscriptionEnd` flows on an injected clock, the fall back to
//! polling, and a `JobWatch` that is woken. No fake printer sends anything here: the real-device-shaped
//! integration lives in the pdfcraft-fakeprint harness (outside this repository).
#![cfg(all(feature = "wsd", not(target_arch = "wasm32")))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::io::{Read, Write};
use std::net::{SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_print::backends::Backend;
use craft_print::wsd::adapter::{DeviceUrl, Request, Response, Transport, WireError};
use craft_print::wsd::testing::{FakeConfig, FakePrinter, MemTransport};
use craft_print::wsd::{EventTuning, NotifyConfig, PrinterUrl, WsdBackend};
use craft_print::{
    Document, Job, JobId, JobWatch, ManualClock, Registry, SubmitRequest, UserConfirmed, WaitPolicy, WatchOutcome, WatchStep, WatchSupport,
};

const URL: &str = "http://127.0.0.1:5357/print";
const MANAGER: &str = "http://127.0.0.1:5357/print";
const PDF: &[u8] = b"%PDF-1.7\n1 0 obj\n<<>>\nendobj\ntrailer\n<<>>\n%%EOF\n";
const WSE: &str = "http://schemas.xmlsoap.org/ws/2004/08/eventing";
const WPRT: &str = "http://schemas.microsoft.com/windows/2006/08/wdp/print";

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// ---------------------------------------------------------------- the scripted printer-side transport

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Ok,
    Fault,
    Down,
}

/// Canned WS-Eventing replies for the subscription operations; every other operation goes to the
/// in-memory printer (so a job can be submitted and polled).
struct Script {
    printer: Arc<FakePrinter>,
    log: Mutex<Vec<(String, String)>>,
    subscribe: Mutex<Mode>,
    renew: Mutex<Mode>,
    granted: Mutex<String>,
    fail_reads: AtomicBool,
    reads: AtomicU32,
}

impl Script {
    fn new(printer: Arc<FakePrinter>) -> Arc<Self> {
        Arc::new(Self {
            printer,
            log: Mutex::default(),
            subscribe: Mutex::new(Mode::Ok),
            renew: Mutex::new(Mode::Ok),
            granted: Mutex::new("PT600S".into()),
            fail_reads: AtomicBool::new(false),
            reads: AtomicU32::new(0),
        })
    }

    /// The eventing operations received so far, by name.
    fn ops(&self) -> Vec<String> {
        self.log.lock().unwrap().iter().map(|(op, _)| op.clone()).collect()
    }

    fn count(&self, op: &str) -> usize {
        self.ops().iter().filter(|o| *o == op).count()
    }

    fn body_of(&self, op: &str) -> String {
        self.log.lock().unwrap().iter().rev().find(|(o, _)| o == op).map(|(_, b)| b.clone()).unwrap_or_default()
    }

    fn set(&self, which: &Mutex<Mode>, mode: Mode) {
        *which.lock().unwrap() = mode;
    }
}

struct Wire(Arc<Script>);

fn envelope(action: &str, relates: &str, body: &str) -> Vec<u8> {
    format!(
        "<s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" xmlns:wse=\"{WSE}\"><s:Header><wsa:Action>{action}</wsa:Action><wsa:RelatesTo>{relates}</wsa:RelatesTo></s:Header><s:Body>{body}</s:Body></s:Envelope>"
    )
    .into_bytes()
}

fn wse_fault(relates: &str, sub: &str) -> Response {
    let body = format!(
        "<s:Fault><s:Code><s:Value>s:Receiver</s:Value><s:Subcode><s:Value xmlns:wse=\"{WSE}\">wse:{sub}</s:Value></s:Subcode></s:Code><s:Reason><s:Text xml:lang=\"en\">no</s:Text></s:Reason></s:Fault>"
    );
    Response::new(500, Some("application/soap+xml".into()), envelope("http://schemas.xmlsoap.org/ws/2004/08/addressing/fault", relates, &body))
}

impl Transport for Wire {
    fn post(&self, url: &DeviceUrl, request: Request) -> Result<Response, WireError> {
        let script = &self.0;
        let text = String::from_utf8_lossy(&request.body).into_owned();
        let Some(op) = request.soap_action.strip_prefix(&format!("{WSE}/")).map(str::to_owned) else {
            if request.soap_action.ends_with("GetJobElements") {
                script.reads.fetch_add(1, Ordering::SeqCst);
                if script.fail_reads.load(Ordering::SeqCst) {
                    return Ok(Response::new(503, Some("text/plain".into()), Vec::new()));
                }
            }
            return MemTransport(Arc::clone(&script.printer)).post(url, request);
        };
        script.log.lock().unwrap().push((op.clone(), text.clone()));
        let doc = roxmltree::Document::parse(&text).unwrap();
        let relates =
            doc.descendants().find(|n| n.is_element() && n.tag_name().name() == "MessageID").and_then(|n| n.text()).unwrap_or_default().to_owned();
        let mode = match op.as_str() {
            "Subscribe" => *script.subscribe.lock().unwrap(),
            "Renew" => *script.renew.lock().unwrap(),
            _ => Mode::Ok,
        };
        match mode {
            Mode::Down => return Ok(Response::new(503, Some("text/plain".into()), Vec::new())),
            Mode::Fault => return Ok(wse_fault(&relates, if op == "Renew" { "UnableToRenew" } else { "EventSourceUnableToProcess" })),
            Mode::Ok => {}
        }
        let granted = script.granted.lock().unwrap().clone();
        let body = match op.as_str() {
            "Subscribe" => format!(
                "<wse:SubscribeResponse><wse:SubscriptionManager><wsa:Address>{MANAGER}</wsa:Address><wsa:ReferenceParameters><wse:Identifier>urn:uuid:sub-{}</wse:Identifier></wsa:ReferenceParameters></wse:SubscriptionManager><wse:Expires>{granted}</wse:Expires></wse:SubscribeResponse>",
                script.count("Subscribe")
            ),
            "Renew" => format!("<wse:RenewResponse><wse:Expires>{granted}</wse:Expires></wse:RenewResponse>"),
            "Unsubscribe" => "<wse:UnsubscribeResponse/>".to_owned(),
            _ => String::new(),
        };
        Ok(Response::new(200, Some("application/soap+xml".into()), envelope(&format!("{WSE}/{op}Response"), &relates, &body)))
    }
}

// ---------------------------------------------------------------- the rig

struct Rig {
    backend: WsdBackend,
    script: Arc<Script>,
    clock: ManualClock,
    url: PrinterUrl,
}

fn rig_with(tuning: EventTuning, config: FakeConfig) -> Rig {
    let printer = FakePrinter::new(config);
    let script = Script::new(printer);
    let clock = ManualClock::new();
    let notify = NotifyConfig::loopback().unwrap().with_tuning(tuning);
    let backend = WsdBackend::new()
        .with_transport(Wire(Arc::clone(&script)))
        .with_clock(Arc::new(clock.clone()))
        .with_poll_interval(Duration::from_secs(2))
        .with_events(notify)
        .unwrap();
    backend.add(URL).unwrap();
    Rig { backend, script, clock, url: PrinterUrl::parse(URL).unwrap() }
}

fn rig() -> Rig {
    rig_with(EventTuning::default(), FakeConfig { polls_to_finish: 2, ..FakeConfig::default() })
}

impl Rig {
    fn addr(&self) -> SocketAddr {
        self.backend.events().unwrap().local_addr()
    }

    fn token(&self) -> String {
        self.backend.events().unwrap().token_for(&self.url).unwrap()
    }

    /// Register interest in a job and let the manager subscribe.
    fn subscribed(&self, job: u32) -> craft_print::WakeReceiver {
        let rx = self.backend.events().unwrap().register(&self.url, job).unwrap();
        self.backend.maintain_events();
        assert_eq!(self.script.count("Subscribe"), 1, "{:?}", self.script.ops());
        rx
    }

    fn stats(&self) -> craft_print::wsd::EventStats {
        self.backend.events().unwrap().stats()
    }

    /// Deliver a valid event of this kind for `job` (a message id makes it unique).
    fn send_event(&self, element: &str, job: u32, id: &str) -> Option<u16> {
        let body = event_body(&self.token(), element, job, id);
        post(self.addr(), &format!("/n/{}", self.token()), "application/soap+xml; charset=utf-8", body.as_bytes())
    }
}

fn event_body(token: &str, element: &str, job: u32, id: &str) -> String {
    let payload = match element {
        "JobStatusEvent" | "JobEndStateEvent" => {
            let inner = if element == "JobStatusEvent" { "JobStatus" } else { "JobEndState" };
            format!("<wprt:{element}><wprt:{inner}><wprt:JobId>{job}</wprt:JobId></wprt:{inner}></wprt:{element}>")
        }
        other => format!("<wprt:{other}/>"),
    };
    format!(
        "<s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" xmlns:wse=\"{WSE}\" xmlns:wprt=\"{WPRT}\"><s:Header><wsa:To>x</wsa:To><wsa:Action>{WPRT}/{element}</wsa:Action><wsa:MessageID>urn:uuid:{id}</wsa:MessageID><wse:Identifier>{token}</wse:Identifier></s:Header><s:Body>{payload}</s:Body></s:Envelope>"
    )
}

/// POST over a socket to the listener and read the status; `None` when it closed without answering.
fn post(addr: SocketAddr, path: &str, content_type: &str, body: &[u8]) -> Option<u16> {
    let head =
        format!("POST {path} HTTP/1.1\r\nHost: x\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len());
    let mut bytes = head.into_bytes();
    bytes.extend_from_slice(body);
    raw(addr, &bytes)
}

fn raw(addr: SocketAddr, bytes: &[u8]) -> Option<u16> {
    let mut stream = TcpStream::connect(addr).ok()?;
    stream.set_read_timeout(Some(secs(5))).ok()?;
    let _ = stream.write_all(bytes);
    let mut reply = Vec::new();
    let _ = stream.read_to_end(&mut reply);
    let text = String::from_utf8_lossy(&reply).into_owned();
    text.strip_prefix("HTTP/1.1 ")?.get(..3)?.parse().ok()
}

fn job_id(n: u32) -> JobId {
    WsdBackend::job_id(URL, n).unwrap()
}

// ---------------------------------------------------------------- opt in

#[test]
fn nothing_listens_and_nothing_is_sent_unless_events_are_configured() {
    let printer = FakePrinter::new(FakeConfig::default());
    let script = Script::new(Arc::clone(&printer));
    let backend = WsdBackend::new().with_transport(Wire(Arc::clone(&script)));
    backend.add(URL).unwrap();
    assert!(backend.events().is_none());
    assert_eq!(backend.watch_support(), WatchSupport::Poll);
    assert!(backend.wake_source(&job_id(1)).is_none());
    backend.maintain_events();
    backend.shutdown_events();
    assert!(script.ops().is_empty());
}

#[test]
fn a_bad_bind_address_is_an_error_and_a_public_advertise_address_is_refused() {
    let busy = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let config = NotifyConfig::new(busy.local_addr().unwrap(), "http://127.0.0.1:9/").unwrap();
    assert!(WsdBackend::new().with_events(config).is_err());
    assert!(NotifyConfig::new("127.0.0.1:0".parse().unwrap(), "http://203.0.113.9:80/").is_err());
}

#[test]
fn a_wildcard_bind_is_refused_before_any_socket_is_opened() {
    // Pick a free port, release it, then ask for a wildcard listener on exactly that port.
    let port = {
        let probe = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        probe.local_addr().unwrap().port()
    };
    for bind in [format!("0.0.0.0:{port}"), format!("[::]:{port}"), format!("[::ffff:0.0.0.0]:{port}")] {
        let refused = NotifyConfig::parse(&bind, "http://127.0.0.1:1/").unwrap_err();
        assert_eq!(refused.kind(), craft_print::ErrorKind::InvalidRequest, "{bind}");
        assert!(refused.message().contains("wildcard"), "{bind}: {refused}");
    }
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "nothing listens after the refusals");
    // The same through a config whose public fields were changed after it was made: with_events refuses.
    let mut config = NotifyConfig::new(format!("127.0.0.1:{port}").parse().unwrap(), "http://127.0.0.1:1/").unwrap();
    config.bind = format!("0.0.0.0:{port}").parse().unwrap();
    let error = WsdBackend::new().with_events(config).map(|_| ()).unwrap_err();
    assert_eq!(error.kind(), craft_print::ErrorKind::InvalidRequest);
    assert!(TcpStream::connect(("127.0.0.1", port)).is_err(), "nothing listens after the refusal");
    // A concrete interface address is accepted and listens there, advertised as itself.
    let config = NotifyConfig::new(format!("127.0.0.1:{port}").parse().unwrap(), "http://127.0.0.1:1/").unwrap();
    let backend = WsdBackend::new().with_events(config).unwrap();
    assert!(TcpStream::connect(("127.0.0.1", port)).is_ok());
    assert_eq!(backend.events_status().unwrap().advertise, "http://127.0.0.1:1");
    backend.shutdown_events();
}

#[test]
fn getting_a_wake_source_sends_nothing_and_the_next_status_call_subscribes() {
    let rig = rig();
    let rx = rig.backend.wake_source(&job_id(1)).unwrap();
    assert!(!rx.is_healthy());
    assert!(rig.script.ops().is_empty());
    // A job that is not ours has no channel.
    assert!(rig.backend.wake_source(&WsdBackend::job_id("http://127.0.0.1:9999/other", 1).unwrap()).is_none());

    let _ = rig.backend.job_status(&job_id(1)); // the job does not exist; housekeeping still ran
    assert_eq!(rig.script.ops(), ["Subscribe"]);
    let body = rig.script.body_of("Subscribe");
    let token = rig.token();
    let port = rig.addr().port();
    assert!(body.contains(&format!("http://127.0.0.1:{port}/n/{token}")), "{body}");
    assert!(body.contains(&format!("http://127.0.0.1:{port}/e/{token}")), "{body}");
    assert!(body.contains("PT600S"));
    assert!(body.contains(&format!(">{token}</")), "the identifier travels as a reference parameter");
    for wanted in
        ["JobStatusEvent", "JobEndStateEvent", "PrinterStatusSummaryEvent", "PrinterStatusConditionEvent", "PrinterStatusConditionClearedEvent"]
    {
        assert!(body.contains(&format!("{WPRT}/{wanted}")), "{wanted}");
    }
    assert!(!body.contains("PrinterElementsChangeEvent"), "capability changes are not job news");
    // Subscribed is not yet trusted: no event has proven that the path works.
    assert!(!rx.is_healthy());
    assert_eq!(rig.backend.watch_support(), WatchSupport::Poll);
}

#[test]
fn the_status_is_plain_data() {
    let rig = rig();
    assert!(WsdBackend::new().events_status().is_none());
    assert!(NotifyConfig::parse("not an address", "http://127.0.0.1:1/").is_err());
    let _rx = rig.subscribed(1);
    let status = rig.backend.events_status().unwrap();
    assert_eq!(status.listen, rig.addr().to_string());
    assert_eq!(status.subscriptions.len(), 1);
    let sub = &status.subscriptions[0];
    assert_eq!((sub.printer.as_str(), sub.subscribed, sub.healthy, sub.watchers, sub.seconds_since_event), (URL, true, false, 1, None));
    rig.send_event("JobStatusEvent", 1, "s1");
    rig.clock.advance(secs(5));
    let sub = &rig.backend.events_status().unwrap().subscriptions[0];
    assert_eq!((sub.healthy, sub.seconds_since_event), (true, Some(5)));
    assert_eq!(rig.backend.events_status().unwrap().stats.wakes, 1);
}

// ---------------------------------------------------------------- a valid event

#[test]
fn a_verified_job_event_wakes_that_job_and_earns_trust() {
    let rig = rig();
    let one = rig.subscribed(1);
    let two = rig.backend.events().unwrap().register(&rig.url, 2).unwrap();
    assert_eq!(rig.send_event("JobStatusEvent", 1, "m1"), Some(202));
    assert!(one.is_pending() && one.is_healthy());
    assert!(!two.is_pending(), "an event about job 1 does not wake job 2");
    assert_eq!(rig.backend.watch_support(), WatchSupport::Events);
    assert_eq!(rig.stats().wakes, 1);

    // Printer-level news wakes every job on the printer.
    one.take();
    assert_eq!(rig.send_event("PrinterStatusSummaryEvent", 0, "m2"), Some(202));
    assert!(one.is_pending() && two.is_pending());
    // The end state of another user's job wakes nobody.
    one.take();
    two.take();
    assert_eq!(rig.send_event("JobEndStateEvent", 99, "m3"), Some(202));
    assert!(!one.is_pending() && !two.is_pending());
}

#[test]
fn a_duplicate_is_acknowledged_but_wakes_once_and_a_burst_is_one_counter() {
    let rig = rig();
    let rx = rig.subscribed(1);
    for _ in 0..5 {
        assert_eq!(rig.send_event("JobStatusEvent", 1, "same-id"), Some(202));
    }
    assert_eq!(rx.take(), 1);
    assert_eq!(rig.stats().duplicates, 4);
    for n in 0..40 {
        assert_eq!(rig.send_event("JobStatusEvent", 1, &format!("burst-{n}")), Some(202));
    }
    assert_eq!(rx.take(), 40);
}

// ---------------------------------------------------------------- forged and malformed

#[test]
fn forged_malformed_and_oversized_messages_are_ignored() {
    let rig = rig();
    let rx = rig.subscribed(1);
    let token = rig.token();
    let addr = rig.addr();
    let good = event_body(&token, "JobStatusEvent", 1, "g1");
    let path = format!("/n/{token}");
    let soap = "application/soap+xml";

    // Wrong or missing token, wrong shape of path.
    let other = "a".repeat(64);
    assert_eq!(post(addr, &format!("/n/{other}"), soap, good.as_bytes()), Some(404));
    assert_eq!(post(addr, "/n/", soap, good.as_bytes()), Some(404));
    assert_eq!(post(addr, "/", soap, good.as_bytes()), Some(404));
    assert_eq!(post(addr, &format!("{path}/x"), soap, good.as_bytes()), Some(404));
    // Right token, wrong identifier header (or none).
    let wrong_id = event_body(&other, "JobStatusEvent", 1, "g2");
    assert_eq!(post(addr, &path, soap, wrong_id.as_bytes()), Some(403));
    let no_id = good.replace(&format!("<wse:Identifier>{token}</wse:Identifier>"), "");
    assert_eq!(post(addr, &path, soap, no_id.as_bytes()), Some(403));
    // Not XML, not SOAP, a fault, no action.
    assert_eq!(post(addr, &path, soap, b"not xml at all"), Some(400));
    assert_eq!(post(addr, &path, soap, b"<a/>"), Some(400));
    assert_eq!(post(addr, &path, soap, good.replace("wsa:Action", "wsa:Nothing").as_bytes()), Some(400));
    // A DTD (entity expansion) is refused by the bounded parser.
    let dtd = format!("<!DOCTYPE x [<!ENTITY a \"aaaa\">]>{good}");
    assert_eq!(post(addr, &path, soap, dtd.as_bytes()), Some(400));
    // Wrong namespace for the body, wrong element for the action, an event type we did not ask for.
    assert_eq!(post(addr, &path, soap, good.replace(&format!("xmlns:wprt=\"{WPRT}\""), "xmlns:wprt=\"urn:evil\"").as_bytes()), Some(400));
    let mismatched = good.replace("<wprt:JobStatusEvent>", "<wprt:JobEndStateEvent>").replace("</wprt:JobStatusEvent>", "</wprt:JobEndStateEvent>");
    assert_eq!(post(addr, &path, soap, mismatched.as_bytes()), Some(400));
    let elements = event_body(&token, "PrinterElementsChangeEvent", 0, "g3");
    assert_eq!(post(addr, &path, soap, elements.as_bytes()), Some(400));
    let unknown_action = good.replace(&format!("{WPRT}/JobStatusEvent"), "urn:something:else");
    assert_eq!(post(addr, &path, soap, unknown_action.as_bytes()), Some(400));
    // A job event with no usable job number.
    let no_job = good.replace("<wprt:JobId>1</wprt:JobId>", "");
    assert_eq!(post(addr, &path, soap, no_job.as_bytes()), Some(400));
    let bad_job = good.replace("<wprt:JobId>1</wprt:JobId>", "<wprt:JobId>one</wprt:JobId>");
    assert_eq!(post(addr, &path, soap, bad_job.as_bytes()), Some(400));
    // Wrong content type, wrong method.
    assert_eq!(post(addr, &path, "text/html", good.as_bytes()), Some(415));
    assert_eq!(raw(addr, format!("GET {path} HTTP/1.1\r\nContent-Length: 0\r\n\r\n").as_bytes()), Some(405));
    // Framing: no length, chunked, two lengths, a huge length, a garbage request line, too many headers.
    assert_eq!(raw(addr, format!("POST {path} HTTP/1.1\r\nContent-Type: {soap}\r\n\r\n").as_bytes()), Some(411));
    assert_eq!(
        raw(addr, format!("POST {path} HTTP/1.1\r\nContent-Type: {soap}\r\nTransfer-Encoding: chunked\r\n\r\n0\r\n\r\n").as_bytes()),
        Some(411)
    );
    assert_eq!(raw(addr, format!("POST {path} HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx").as_bytes()), Some(400));
    assert_eq!(raw(addr, format!("POST {path} HTTP/1.1\r\nContent-Type: {soap}\r\nContent-Length: 99999999999\r\n\r\n").as_bytes()), Some(413));
    assert_eq!(raw(addr, b"hello\r\n\r\n"), Some(400));
    assert_eq!(raw(addr, format!("POST {path} HTTP/2 \r\n\r\n").as_bytes()), Some(400));
    let many: String = (0..40).map(|i| format!("X-{i}: y\r\n")).collect();
    assert_eq!(raw(addr, format!("POST {path} HTTP/1.1\r\n{many}Content-Length: 0\r\n\r\n").as_bytes()), Some(431));
    // A body over the cap, actually sent.
    let big = vec![b' '; 70 * 1024];
    assert_eq!(post(addr, &path, soap, &big), Some(413));
    // Oversized headers.
    let long = "x".repeat(9000);
    assert_eq!(raw(addr, format!("POST {path} HTTP/1.1\r\nX-Long: {long}\r\n\r\n").as_bytes()), Some(431));

    assert!(!rx.is_pending(), "not one of them woke anybody");
    assert_eq!(rx.total(), 0);
    assert_eq!(rig.stats().wakes, 0);
    assert!(!rx.is_healthy());
    // And the listener is still serving.
    assert_eq!(post(addr, &path, soap, good.as_bytes()), Some(202));
    assert!(rx.is_pending());
}

#[test]
fn a_connection_from_another_address_is_closed_unread_and_a_token_is_bound_to_its_printer() {
    let rig = rig();
    // A subscription for a printer at another loopback address: we connect from 127.0.0.1.
    let far = PrinterUrl::parse("http://127.0.0.9:5357/print").unwrap();
    let rx_far = rig.backend.events().unwrap().register(&far, 1).unwrap();
    let rx = rig.backend.events().unwrap().register(&rig.url, 1).unwrap();
    let far_token = rig.backend.events().unwrap().token_for(&far).unwrap();
    // 127.0.0.1 is a printer we know, but this token belongs to 127.0.0.9.
    let body = event_body(&far_token, "JobStatusEvent", 1, "f1");
    assert_eq!(post(rig.addr(), &format!("/n/{far_token}"), "application/soap+xml", body.as_bytes()), Some(404));
    assert!(!rx_far.is_pending() && !rx.is_pending());
    assert_eq!(rig.stats().forged, 1);

    // Only the far printer is known now: our own address is not a printer at all.
    drop(rx);
    rig.backend.maintain_events();
    let before = rig.stats().rejected_source;
    assert_eq!(post(rig.addr(), &format!("/n/{far_token}"), "application/soap+xml", body.as_bytes()), None);
    assert_eq!(rig.stats().rejected_source, before + 1);
    assert!(!rx_far.is_pending());
}

#[test]
fn a_flood_is_rate_limited_and_a_slow_sender_does_not_stop_the_listener() {
    let tuning = EventTuning { burst: 5, per_second: 0, connection_deadline: Duration::from_millis(300), ..EventTuning::default() };
    let rig = rig_with(tuning, FakeConfig::default());
    let rx = rig.subscribed(1);
    // A sender that connects and says nothing is cut off at the deadline.
    let mut idle = TcpStream::connect(rig.addr()).unwrap();
    idle.set_read_timeout(Some(secs(5))).unwrap();
    let mut sink = Vec::new();
    let _ = idle.read_to_end(&mut sink);
    assert!(String::from_utf8_lossy(&sink).starts_with("HTTP/1.1 408"), "{}", String::from_utf8_lossy(&sink));

    let mut statuses = Vec::new();
    for n in 0..12 {
        statuses.push(rig.send_event("JobStatusEvent", 1, &format!("flood-{n}")));
    }
    let accepted = statuses.iter().filter(|s| **s == Some(202)).count();
    let limited = statuses.iter().filter(|s| **s == Some(429)).count();
    assert_eq!(accepted + limited, 12, "{statuses:?}");
    assert!(limited >= 7, "{statuses:?}");
    assert_eq!(rx.take(), u32::try_from(accepted).unwrap());
    assert!(rig.stats().rejected_rate >= 7);
}

// ---------------------------------------------------------------- lease, trust and fallback

#[test]
fn the_lease_is_renewed_at_half_and_trust_lapses_with_silence() {
    let rig = rig();
    let rx = rig.subscribed(1);
    assert_eq!(rig.send_event("JobStatusEvent", 1, "a"), Some(202));
    assert!(rx.is_healthy());

    rig.clock.advance(secs(299));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 0);
    assert!(!rx.is_healthy(), "299 seconds without an event is beyond the 2 minute quiet window");
    assert_eq!(rig.backend.watch_support(), WatchSupport::Poll);

    rig.clock.advance(secs(1));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 1);
    // The renewal moved the expiry, not the trust: the next verified event restores it.
    assert!(!rx.is_healthy());
    assert_eq!(rig.send_event("JobEndStateEvent", 1, "b"), Some(202));
    assert!(rx.is_healthy());

    // The next renewal is half a lease after the last.
    rig.clock.advance(secs(299));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 1);
    rig.clock.advance(secs(1));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 2);
    assert_eq!(rig.script.count("Subscribe"), 1);
}

#[test]
fn a_printer_that_grants_a_short_lease_is_renewed_at_half_of_what_it_granted() {
    let rig = rig();
    *rig.script.granted.lock().unwrap() = "PT60S".into();
    let _rx = rig.subscribed(1);
    rig.clock.advance(secs(29));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 0);
    rig.clock.advance(secs(1));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 1);
}

#[test]
fn a_renewal_the_printer_refuses_starts_over_and_one_it_cannot_answer_is_retried_until_the_lease_ends() {
    let rig = rig();
    let rx = rig.subscribed(1);
    rig.send_event("JobStatusEvent", 1, "a");
    assert!(rx.is_healthy());

    // The printer forgot us (a restart): Renew faults, the next housekeeping subscribes again.
    rig.script.set(&rig.script.renew, Mode::Fault);
    rig.clock.advance(secs(300));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 1);
    assert!(!rx.is_healthy());
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 2);

    // Now it cannot be reached: trust is withdrawn at once, renewals are retried within 15 seconds,
    // and when the lease runs out the subscription is dropped and made again.
    rig.send_event("JobStatusEvent", 1, "b");
    assert!(rx.is_healthy());
    rig.script.set(&rig.script.renew, Mode::Down);
    rig.clock.advance(secs(300));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 2);
    assert!(!rx.is_healthy());
    rig.clock.advance(secs(15));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Renew"), 3);
    rig.clock.advance(secs(300));
    rig.backend.maintain_events(); // expired on our clock: dropped
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 3);
}

#[test]
fn a_failed_subscribe_means_polling_with_backoff_and_recovers_when_the_printer_does() {
    let rig = rig();
    rig.script.set(&rig.script.subscribe, Mode::Fault);
    let rx = rig.backend.events().unwrap().register(&rig.url, 1).unwrap();
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 1);
    assert!(!rx.is_healthy());
    assert_eq!(rig.backend.watch_support(), WatchSupport::Poll);

    // Backoff: nothing for a minute, then one try, then two minutes.
    rig.backend.maintain_events();
    rig.clock.advance(secs(59));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 1);
    rig.clock.advance(secs(1));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 2);
    rig.clock.advance(secs(119));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 2);
    rig.script.set(&rig.script.subscribe, Mode::Ok);
    rig.clock.advance(secs(1));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 3);
    // Subscribed again, and trusted once an event proves the path.
    assert!(!rx.is_healthy());
    rig.send_event("JobStatusEvent", 1, "x");
    assert!(rx.is_healthy());
}

#[test]
fn a_printer_that_is_down_fails_the_subscribe_without_breaking_job_status() {
    let rig = rig();
    rig.script.set(&rig.script.subscribe, Mode::Down);
    let rx = rig.backend.events().unwrap().register(&rig.url, 1).unwrap();
    // The job does not exist, so this is the printer's "no such job"; the housekeeping before it failed quietly.
    let error = rig.backend.job_status(&job_id(1)).unwrap_err();
    assert_eq!(error.kind(), craft_print::backends::ErrorKind::NotFound);
    assert_eq!(rig.script.count("Subscribe"), 1);
    assert!(!rx.is_healthy());
}

#[test]
fn a_subscription_end_withdraws_trust_wakes_the_watch_and_backs_off() {
    let rig = rig();
    let rx = rig.subscribed(1);
    rig.send_event("JobStatusEvent", 1, "a");
    rx.take();
    assert!(rx.is_healthy());

    let token = rig.token();
    let end = format!(
        "<s:Envelope xmlns:s=\"http://www.w3.org/2003/05/soap-envelope\" xmlns:wsa=\"http://schemas.xmlsoap.org/ws/2004/08/addressing\" xmlns:wse=\"{WSE}\"><s:Header><wsa:Action>{WSE}/SubscriptionEnd</wsa:Action><wsa:MessageID>urn:uuid:e1</wsa:MessageID></s:Header><s:Body><wse:SubscriptionEnd><wse:SubscriptionManager><wsa:Address>{MANAGER}</wsa:Address><wsa:ReferenceParameters><wse:Identifier>IDENT</wse:Identifier></wsa:ReferenceParameters></wse:SubscriptionManager><wse:Status>{WSE}/SourceShuttingDown</wse:Status><wse:Reason>bye</wse:Reason></wse:SubscriptionEnd></s:Body></s:Envelope>"
    );
    // An end that names another subscription's manager is not ours.
    let wrong = end.replace("IDENT", "urn:uuid:someone-else");
    assert_eq!(post(rig.addr(), &format!("/e/{token}"), "application/soap+xml", wrong.as_bytes()), Some(403));
    assert!(rx.is_healthy());
    // An end with the wrong token is nothing.
    assert_eq!(post(rig.addr(), &format!("/e/{}", "b".repeat(64)), "application/soap+xml", end.as_bytes()), Some(404));
    assert!(rx.is_healthy());

    let ours = end.replace("IDENT", "urn:uuid:sub-1");
    assert_eq!(post(rig.addr(), &format!("/e/{token}"), "application/soap+xml", ours.as_bytes()), Some(202));
    assert!(!rx.is_healthy());
    assert!(rx.is_pending(), "the watch polls now");
    assert_eq!(rig.backend.watch_support(), WatchSupport::Poll);
    assert_eq!(rig.stats().ended, 1);
    // Resubscription waits out the backoff.
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 1);
    rig.clock.advance(secs(60));
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Subscribe"), 2);
}

// ---------------------------------------------------------------- cleanup

#[test]
fn a_subscription_nobody_watches_any_more_is_released() {
    let rig = rig();
    let a = rig.subscribed(1);
    let b = rig.backend.events().unwrap().register(&rig.url, 2).unwrap();
    let token = rig.token();
    drop(a);
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Unsubscribe"), 0, "job 2 still needs it");
    drop(b);
    rig.backend.maintain_events();
    assert_eq!(rig.script.count("Unsubscribe"), 1);
    assert!(rig.backend.events().unwrap().token_for(&rig.url).is_none());
    // The old token is dead: a late or replayed event reaches nobody.
    let body = event_body(&token, "JobStatusEvent", 1, "late");
    let status = post(rig.addr(), &format!("/n/{token}"), "application/soap+xml", body.as_bytes());
    assert!(matches!(status, None | Some(404)), "{status:?}");
}

#[test]
fn shutdown_unsubscribes_stops_and_joins_and_dropping_the_backend_does_the_same() {
    let rig = rig();
    let _rx = rig.subscribed(1);
    let addr = rig.addr();
    rig.backend.shutdown_events();
    assert_eq!(rig.script.count("Unsubscribe"), 1);
    assert!(TcpStream::connect(addr).is_err(), "the listener is gone");
    rig.backend.shutdown_events(); // again: nothing happens
    assert_eq!(rig.script.count("Unsubscribe"), 1);
    assert!(rig.backend.events().unwrap().register(&rig.url, 1).is_none());

    let second = self::rig();
    let _rx = second.subscribed(1);
    let (script, addr) = (Arc::clone(&second.script), second.addr());
    let Rig { backend, .. } = second;
    let clone = backend.clone();
    drop(backend);
    assert_eq!(script.count("Unsubscribe"), 0, "a clone still holds the listener");
    drop(clone);
    assert_eq!(script.count("Unsubscribe"), 1);
    assert!(TcpStream::connect(addr).is_err());
}

// ---------------------------------------------------------------- a watch that is woken

fn submit(rig: &Rig) -> JobId {
    let registry = Registry::new();
    registry.register(Box::new(rig.backend.clone())).unwrap();
    let job = Job { printer: Some(format!("wsd:{URL}")), title: "t".into(), ..Job::default() };
    let accepted = registry.submit_job(SubmitRequest::new(Document::pdf(PDF), job).with_consent(UserConfirmed::interactive())).unwrap();
    accepted.job.unwrap()
}

fn watch_over(rig: &Rig, job: &JobId) -> (Registry, JobWatch) {
    let registry = Registry::new();
    registry.register(Box::new(rig.backend.clone())).unwrap();
    let watch = registry.job_watch(job, WaitPolicy::default().with_max_wait(secs(3600))).unwrap().with_clock(Arc::new(rig.clock.clone()));
    (registry, watch)
}

#[test]
fn a_woken_watch_polls_now_and_the_lost_event_is_caught_by_the_safety_poll() {
    // The printer finishes the job at its fourth status read, announcing nothing.
    let rig = rig_with(EventTuning::default(), FakeConfig { polls_to_finish: 4, ..FakeConfig::default() });
    let job = submit(&rig);
    let (registry, mut watch) = watch_over(&rig, &job);
    // Not trusted yet: ordinary fast polling (this poll also subscribes).
    assert!(matches!(watch.tick(&registry), WatchStep::Update(_)));
    assert_eq!(rig.script.count("Subscribe"), 1);
    assert_eq!(watch.next_poll_in(), Some(secs(2)));

    // The first verified event proves the path: the idle gap becomes the 30 second safety poll.
    assert_eq!(rig.send_event("JobStatusEvent", 1, "e1"), Some(202));
    assert_eq!(rig.backend.watch_support(), WatchSupport::Events);
    assert!(watch.is_pushed());
    rig.clock.advance(secs(1));
    assert!(matches!(watch.tick(&registry), WatchStep::Update(_)), "the wake brought the poll forward");
    assert_eq!(watch.wake_polls(), 1);
    assert_eq!(watch.next_poll_in(), Some(secs(30)));

    // Nothing is announced; 29 seconds later nothing has happened, at 30 the safety poll runs.
    rig.clock.advance(secs(29));
    assert!(matches!(watch.tick(&registry), WatchStep::Pending { .. }));
    rig.clock.advance(secs(1));
    let before = watch.polls();
    assert!(matches!(watch.tick(&registry), WatchStep::Update(_)));
    assert_eq!(watch.polls(), before + 1);
    // The fourth read is the printer's final word: only a poll can end the job.
    rig.clock.advance(secs(30));
    assert!(matches!(watch.tick(&registry), WatchStep::Done(WatchOutcome::Terminal(_))));
}

#[test]
fn events_never_decide_an_outcome_even_when_they_claim_the_job_ended() {
    let rig = rig_with(EventTuning::default(), FakeConfig { ending: craft_print::wsd::testing::Ending::Hangs, ..FakeConfig::default() });
    let job = submit(&rig);
    let (registry, mut watch) = watch_over(&rig, &job);
    watch.tick(&registry);
    for n in 0..5 {
        assert_eq!(rig.send_event("JobEndStateEvent", 1, &format!("end-{n}")), Some(202));
        rig.clock.advance(secs(1));
        // The end-state event only brought a poll forward; the printer still says the job is not done.
        assert!(matches!(watch.tick(&registry), WatchStep::Update(_)), "{:?}", watch.outcome());
    }
    assert!(!watch.is_done());
}

#[test]
fn losing_the_subscription_returns_the_watch_to_fast_polling() {
    let rig = rig_with(EventTuning::default(), FakeConfig { ending: craft_print::wsd::testing::Ending::Hangs, ..FakeConfig::default() });
    let job = submit(&rig);
    let (registry, mut watch) = watch_over(&rig, &job);
    watch.tick(&registry);
    rig.send_event("JobStatusEvent", 1, "a");
    rig.clock.advance(secs(1));
    watch.tick(&registry);
    assert_eq!(watch.next_poll_in(), Some(secs(30)));
    // Two quiet minutes: the next housekeeping (inside a poll) withdraws trust.
    rig.clock.advance(secs(30));
    watch.tick(&registry);
    rig.clock.advance(secs(30));
    watch.tick(&registry);
    rig.clock.advance(secs(30));
    watch.tick(&registry);
    rig.clock.advance(secs(31));
    let step = watch.tick(&registry);
    assert!(!watch.is_pushed(), "{step:?}");
    assert!(watch.next_poll_in().unwrap() <= secs(15));
}
