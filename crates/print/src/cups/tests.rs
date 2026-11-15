//! The IPP client against a fake CUPS server (a unix socket or a loopback port): discovery,
//! Print-Job, and the malformed replies it must refuse without a panic.

use super::*;
use crate as spool;
use crate::cups::{self as ipp, Server};
use std::collections::VecDeque;
use std::io::{Read, Write};
use std::sync::Mutex;
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(2);

/// A fake CUPS server: each connection carries one request; the next canned reply goes back
/// (nothing, when the list is out — the client then sees a hang-up). Every request is recorded
/// for the assertions.
struct FakeCups {
    server: Server,
    requests: Receiver<Vec<u8>>,
}

impl FakeCups {
    fn on_unix(replies: Vec<Vec<u8>>) -> std::io::Result<Self> {
        let dir = tempfile::tempdir()?;
        let path = dir.path().join("cups.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path)?;
        // The socket must outlive this function; the folder is left for the process to reap.
        std::mem::forget(dir);
        let requests = listen_unix(listener, replies);
        Ok(FakeCups { server: Server::Unix(path), requests })
    }

    fn on_tcp(replies: Vec<Vec<u8>>) -> std::io::Result<Self> {
        let listener = std::net::TcpListener::bind("127.0.0.1:0")?;
        let server = Server::Tcp(listener.local_addr()?);
        let requests = listen_tcp(listener, replies);
        Ok(FakeCups { server, requests })
    }

    fn request(&self) -> Result<Vec<u8>, mpsc::RecvTimeoutError> {
        self.requests.recv_timeout(Duration::from_secs(5))
    }

    /// The server as the one-endpoint list the client functions take.
    fn endpoint(&self) -> &[Server] {
        std::slice::from_ref(&self.server)
    }
}

fn listen_unix(listener: std::os::unix::net::UnixListener, replies: Vec<Vec<u8>>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let replies = Mutex::new(VecDeque::from(replies));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &replies, &tx);
        }
    });
    rx
}

fn listen_tcp(listener: std::net::TcpListener, replies: Vec<Vec<u8>>) -> Receiver<Vec<u8>> {
    let (tx, rx) = mpsc::channel();
    let replies = Mutex::new(VecDeque::from(replies));
    std::thread::spawn(move || {
        for stream in listener.incoming().flatten() {
            serve(stream, &replies, &tx);
        }
    });
    rx
}

fn serve(mut stream: impl Read + Write, replies: &Mutex<VecDeque<Vec<u8>>>, requests: &mpsc::Sender<Vec<u8>>) {
    let Some(request) = read_request(&mut stream) else { return };
    let Some(body) = ipp_body(&request) else { return };
    let request_id = body.get(4..8).map(|id| id.to_vec());
    let _ = requests.send(request);
    // A list out of replies is a server that hangs up before answering.
    let Ok(mut replies) = replies.lock() else { return };
    if let Some(mut reply) = replies.pop_front() {
        if let Some(body) = reply.windows(4).position(|w| w == b"\r\n\r\n").map(|at| at + 4)
            && reply.get(body..body + 2) == Some(&[1, 1])
            && let Some(id) = request_id
            && let Some(destination) = reply.get_mut(body + 4..body + 8)
        {
            destination.copy_from_slice(&id);
        }
        let _ = stream.write_all(&reply);
        let _ = stream.flush();
    }
}

/// One HTTP request: the headers (bounded), then the `Content-Length` body it announces.
fn read_request(stream: &mut impl Read) -> Option<Vec<u8>> {
    let mut buf = Vec::new();
    let mut byte = [0u8; 1];
    loop {
        match stream.read(&mut byte) {
            Ok(0) | Err(_) => return None,
            Ok(_) => buf.push(byte[0]),
        }
        if buf.ends_with(b"\r\n\r\n") {
            break;
        }
        if buf.len() > 1 << 16 {
            return None;
        }
    }
    let head = String::from_utf8_lossy(&buf);
    let len: usize = head.lines().find_map(|l| l.strip_prefix("Content-Length:")).and_then(|v| v.trim().parse().ok())?;
    let head_len = buf.len();
    buf.resize(head_len + len, 0);
    stream.read_exact(&mut buf[head_len..]).ok()?;
    Some(buf)
}

/// An HTTP reply carrying `body`, as CUPS would send it.
fn http_reply(body: &[u8]) -> Vec<u8> {
    let mut out = format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    out.extend(body);
    out
}

/// An IPP reply body: `status`, then the attribute groups as raw bytes, then end of attributes.
fn ipp_reply(status: u16, groups: &[&[u8]]) -> Vec<u8> {
    let mut out = vec![1, 1];
    out.extend(status.to_be_bytes());
    out.extend(1u32.to_be_bytes());
    for &group in groups {
        out.extend(group);
    }
    out.push(0x03);
    out
}

fn group(tag: u8) -> Vec<u8> {
    vec![tag]
}

fn printer_name(name: &str) -> Vec<u8> {
    attr(0x42, "printer-name", name.as_bytes())
}

/// One attribute, framed the way the client frames them, so `has` finds it in a request.
fn attr(tag: u8, name: &str, value: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    out.extend((name.len() as u16).to_be_bytes());
    out.extend(name.as_bytes());
    out.extend((value.len() as u16).to_be_bytes());
    out.extend(value);
    out
}

fn has(bytes: &[u8], needle: &[u8]) -> bool {
    !needle.is_empty() && bytes.windows(needle.len()).any(|w| w == needle)
}

/// The IPP part of a recorded request: everything after the HTTP head.
fn ipp_body(request: &[u8]) -> Option<&[u8]> {
    let at = request.windows(4).position(|w| w == b"\r\n\r\n")?;
    request.get(at + 4..)
}

fn entry(name: &str, default: bool) -> spool::Printer {
    spool::Printer { description: String::new(), name: name.into(), default, two_sided: None }
}

#[test]
fn discovery_finds_queues_and_the_default_over_a_unix_socket() {
    let fake = FakeCups::on_unix(vec![
        http_reply(&ipp_reply(
            0x0000,
            &[
                &group(0x01),
                &printer_name("Office_Laser"),
                &group(0x04),
                &printer_name("Office_Laser"),
                &group(0x04),
                &printer_name("Label_Writer"),
                &group(0x04),
                &printer_name("Office_Laser"),
            ],
        )),
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Office_Laser")])),
    ])
    .unwrap();
    let printers = ipp::printers_at(fake.endpoint()).unwrap();
    assert_eq!(printers, [entry("Office_Laser", true), entry("Label_Writer", false)], "each queue once, the default marked");
    let taken = fake.request().unwrap();
    let queues = ipp_body(&taken).unwrap();
    assert_eq!(&queues[..4], &[1, 1, 0x40, 0x02], "IPP 1.1, CUPS-Get-Printers");
    assert!(has(queues, b"ipp://localhost/"));
    assert!(has(queues, &attr(0x44, "requested-attributes", b"printer-name")));
    assert!(has(queues, b"printer-is-accepting-jobs"), "only the attributes the dialog needs are asked for");
    let taken = fake.request().unwrap();
    let default = ipp_body(&taken).unwrap();
    assert_eq!(&default[..4], &[1, 1, 0x40, 0x01], "CUPS-Get-Default");
}

#[test]
fn a_print_job_carries_the_pdf_and_the_options() {
    let pdf: Vec<u8> = b"%PDF-1.7 fake print job\n".iter().copied().cycle().take(1 << 20).collect();
    let job = spool::Job {
        printer: Some("Office".into()),
        copies: 2,
        collate: false,
        duplex: Some(spool::Duplex::LongEdge),
        grayscale: true,
        scaling: spool::Scaling::ShrinkToFit,
        title: "memo.pdf".into(),
        ..Job::default()
    };
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(
        0x0000,
        &[&group(0x01), &group(0x02), &attr(0x21, "job-id", &12u32.to_be_bytes()), &attr(0x23, "job-state", &[3u8])],
    ))])
    .unwrap();
    let message = ipp::print_job_at(fake.endpoint(), &pdf, &job).unwrap();
    assert_eq!(message, "request id is Office-12", "the same message lp prints");
    let taken = fake.request().unwrap();
    let request = ipp_body(&taken).unwrap();
    assert_eq!(&request[..4], &[1, 1, 0x00, 0x02], "IPP 1.1, Print-Job");
    assert!(has(request, b"ipp://localhost/printers/Office"));
    let mut copies = group(0x02);
    copies.extend(attr(0x21, "copies", &2u32.to_be_bytes()));
    assert!(has(request, &copies), "copies are a job attribute");
    assert!(has(request, &attr(0x22, "collate", &[0])));
    assert!(has(request, &attr(0x44, "sides", b"two-sided-long-edge")));
    assert!(has(request, &attr(0x44, "print-color-mode", b"monochrome")));
    assert!(has(request, &attr(0x44, "print-scaling", b"auto-fit")));
    assert!(has(request, &attr(0x49, "document-format", b"application/pdf")));
    assert!(has(request, &attr(0x42, "job-name", b"memo.pdf")));
    assert!(request.ends_with(&pdf), "the whole job is the document data");
}

#[test]
fn the_default_printer_is_used_when_the_job_has_none() {
    let fake = FakeCups::on_tcp(vec![
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Office")])),
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x02), &attr(0x21, "job-id", &7u32.to_be_bytes())])),
    ])
    .unwrap();
    let message = ipp::print_job_at(fake.endpoint(), b"%PDF-1.7\n", &Job::default()).unwrap();
    assert_eq!(message, "request id is Office-7");
    let default = fake.request().unwrap();
    assert_eq!(&ipp_body(&default).unwrap()[..4], &[1, 1, 0x40, 0x01], "the default comes from CUPS-Get-Default");
    let taken = fake.request().unwrap();
    assert!(has(ipp_body(&taken).unwrap(), b"ipp://localhost/printers/Office"), "the resolved default is the target");
}

#[test]
fn a_default_outside_the_queue_list_still_gets_its_marker() {
    let fake = FakeCups::on_unix(vec![
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Office_Laser")])),
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Label_Writer")])),
    ])
    .unwrap();
    let printers = ipp::printers_at(fake.endpoint()).unwrap();
    assert_eq!(
        printers,
        [entry("Office_Laser", false), entry("Label_Writer", true)],
        "the default joins the list, so the dialog has a selection to restore"
    );
}

#[test]
fn no_default_printer_is_an_error_when_the_job_names_none() {
    let fake = FakeCups::on_unix(vec![http_reply(&ipp_reply(0x0000, &[&group(0x01)]))]).unwrap();
    let err = ipp::print_job_at(fake.endpoint(), b"%PDF-1.7\n", &Job::default()).unwrap_err();
    assert!(err.to_string().contains("no default printer"), "{err}");
}

#[test]
fn a_printer_name_that_cannot_be_a_uri_path_is_refused() {
    let fake = FakeCups::on_tcp(vec![]).unwrap();
    let mut long = "x".repeat(257);
    long.push('y');
    for name in ["Office (east)", "../../admin", "", "a b", "Brother/HL", "Копир", long.as_str()] {
        let job = spool::Job { printer: Some(name.into()), ..Job::default() };
        assert_eq!(
            ipp::print_job_at(fake.endpoint(), b"pdf", &job),
            Err(spool::Error::Spool(format!("not a printer name: {name}"))),
            "{name:?} cannot go in the printer URI's path"
        );
    }
}

#[test]
fn an_ipp_refusal_is_an_error_that_names_the_problem() {
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0x0506, &[&group(0x01)]))]).unwrap();
    let job = spool::Job { printer: Some("Office".into()), ..Job::default() };
    assert_eq!(
        ipp::print_job_at(fake.endpoint(), b"%PDF-1.7\n", &job),
        Err(spool::Error::Spool("the print server refused the request (the printer is not accepting jobs)".into()))
    );
}

#[test]
fn malformed_replies_are_errors_never_a_panic() {
    let bodies: Vec<(&str, Vec<u8>)> = vec![
        // Shorter than an IPP header.
        ("shorter than an IPP header", vec![1, 1, 0]),
        // A reply that ends inside an attribute: the name is announced, the value never comes.
        ("cut inside an attribute", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0x42, 0x00, 0x20, b'p']),
        // A value tag the registry does not know.
        ("unknown value tag", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0xFF, 0x00, 0x00, 0x00, 0x00]),
        // A collection, which the attributes asked for never are.
        ("collection", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0x34, 0x00, 0x00, 0x00, 0x00]),
        // A group tag that is no delimiter in the registry.
        ("reserved delimiter", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x06]),
        // A length within the cap that runs past the reply, and one past the cap.
        ("length past the reply", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0x42, 0x00, 0xFE, b'p']),
        ("length past the cap", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0x42, 0xFF, 0xFF, b'p']),
        // No end of attributes before the body runs out.
        ("no end of attributes", vec![1, 1, 0, 0, 0, 0, 0, 1, 0x04, 0x42, 0x00, 0x01, b'x']),
    ];
    for (why, body) in bodies {
        let fake = FakeCups::on_unix(vec![http_reply(&body)]).unwrap();
        let result = ipp::printers_at(fake.endpoint());
        assert!(result.is_err(), "{why}: the reply must be an error, not a panic or a success");
    }
}

#[test]
fn http_replies_that_cannot_carry_an_answer_are_refused() {
    let replies: Vec<(&str, Vec<u8>)> = vec![
        ("requires authentication", b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\n\r\n".to_vec()),
        ("demands a TLS upgrade", b"HTTP/1.1 426 Upgrade Required\r\nContent-Length: 0\r\n\r\n".to_vec()),
        ("chunked", b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n25\r\nthe body is chunked\r\n0\r\n\r\n".to_vec()),
        ("no Content-Length", b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\n\r\n".to_vec()),
        ("Content-Length past the cap", b"HTTP/1.1 200 OK\r\nContent-Length: 99999999\r\n\r\n".to_vec()),
        ("unreadable status line", b"not HTTP at all\r\n\r\n".to_vec()),
    ];
    for (why, reply) in replies {
        let fake = FakeCups::on_unix(vec![reply]).unwrap();
        let result = ipp::post(&[fake.server], "/", b"ignored", TIMEOUT, true);
        assert!(result.is_err(), "{why}: the reply must be an error, not a panic or a success");
    }
    // A server that takes the request and hangs up.
    let fake = FakeCups::on_unix(vec![]).unwrap();
    assert!(ipp::post(&[fake.server], "/", b"ignored", TIMEOUT, true).is_err(), "a hang-up is an error");
}

#[test]
fn a_server_that_answers_is_reachable_and_silence_is_not() {
    let fake = FakeCups::on_unix(vec![]).unwrap();
    assert!(ipp::reachable(&fake.server), "a unix socket that accepts connections is reachable");
    let fake = FakeCups::on_tcp(vec![]).unwrap();
    assert!(ipp::reachable(&fake.server), "a loopback port that accepts connections is reachable");
    let absent = Server::Unix("/nonexistent/craft-print-test.sock".into());
    assert!(!ipp::reachable(&absent), "a socket nothing listens on is not reachable");
    let addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    assert!(!ipp::reachable(&Server::Tcp(addr)), "a port nothing listens on is not reachable");
}

#[test]
fn chunked_reply_is_bounded_and_rejects_ambiguous_framing() {
    let good = b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2\r\nde\r\n0\r\n\r\n";
    assert_eq!(super::read_reply(&mut &good[..]).unwrap(), b"abcde");
    for bad in [
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nContent-Length: 0\r\n\r\n0\r\n\r\n".as_slice(),
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n100001\r\n",
        b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\na",
        b"HTTP/1.1 200 OK\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
    ] {
        assert!(super::read_reply(&mut &bad[..]).is_err());
    }
}

#[test]
fn explicit_server_rejects_header_injection_and_malformed_port() {
    for value in ["host\r\nInjected: yes", "host:0", "host:99999", "http://host", "user@host", ""] {
        assert!(Server::parse(value).is_err(), "{value:?}");
    }
    assert_eq!(Server::parse("[::1]:631").unwrap().authority(), "[::1]:631");
    assert_eq!(Server::parse("print.example:8631").unwrap().authority(), "print.example:8631");
}

#[test]
fn class_submission_preserves_authority_user_and_numeric_job_id() {
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0, &[&group(2), &attr(0x21, "job-id", &42u32.to_be_bytes())]))]).unwrap();
    let destination = super::Destination { name: "Studio".into(), is_class: true };
    assert_eq!(super::submit_as_at(fake.endpoint(), &destination, b"pdf", &Job::default(), "artist").unwrap(), 42);
    let request = fake.request().unwrap();
    assert!(has(&request, b"POST /classes/Studio HTTP/1.1"));
    assert!(has(&request, format!("Host: {}", fake.server.authority()).as_bytes()));
    assert!(has(ipp_body(&request).unwrap(), format!("ipp://{}/classes/Studio", fake.server.authority()).as_bytes()));
    assert!(has(ipp_body(&request).unwrap(), &attr(0x42, "requesting-user-name", b"artist")));
}

#[test]
fn uncertain_submission_never_tries_the_second_endpoint() {
    let first = FakeCups::on_tcp(vec![]).unwrap();
    let second = FakeCups::on_tcp(vec![]).unwrap();
    let destination = super::Destination { name: "Studio".into(), is_class: false };
    assert!(super::submit_at(&[first.server.clone(), second.server.clone()], &destination, b"pdf", &Job::default()).is_err());
    let _ = first.request().unwrap();
    assert!(second.requests.recv_timeout(Duration::from_millis(100)).is_err());
}

#[test]
fn queue_metadata_stays_with_its_printer_group() {
    let bytes = ipp_reply(
        0,
        &[
            &group(4),
            &attr(0x41, "printer-info", b"Studio printer"),
            &printer_name("First"),
            &attr(0x23, "printer-state", &5u32.to_be_bytes()),
            &attr(0x22, "printer-is-accepting-jobs", &[0]),
            &group(4),
            &printer_name("Second"),
        ],
    );
    let reply = super::parse_reply(&bytes).unwrap();
    assert_eq!(reply.queues.len(), 2);
    assert_eq!(reply.queues[0].info, "Studio printer");
    assert_eq!(reply.queues[0].state, 5);
    assert!(!reply.queues[0].accepting_jobs);
    assert!(reply.queues[1].info.is_empty());
    assert!(reply.queues[1].accepting_jobs);
}

#[test]
fn reply_identity_and_metadata_request_are_checked() {
    let request = super::discovery_request(0x4002);
    assert!(has(&request, b"printer-info"));
    let mut reply = vec![1, 1, 0, 0];
    reply.extend_from_slice(&request[4..8]);
    reply.push(3);
    assert!(super::validate_reply_identity(&request, &reply).is_ok());
    reply[7] ^= 1;
    assert!(super::validate_reply_identity(&request, &reply).is_err());
    reply[4..8].copy_from_slice(&request[4..8]);
    reply[0] = 9;
    assert!(super::validate_reply_identity(&request, &reply).is_err());
}

#[test]
fn slow_reply_cannot_reset_the_absolute_deadline() {
    let (client, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let producer = std::thread::spawn(move || {
        // Finite work and a joined handle: no abandoned timeout worker.
        for _ in 0..20 {
            if peer.write_all(b"x").is_err() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
    });
    let start = std::time::Instant::now();
    let mut stream = super::Stream::Unix(client);
    let mut reader = super::DeadlineReader { stream: &mut stream, end: start + Duration::from_millis(80) };
    let mut bytes = [0; 20];
    assert!(reader.read_exact(&mut bytes).is_err(), "trickle reads must not extend the absolute deadline");
    drop(stream);
    producer.join().unwrap();
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn blocked_upload_obeys_the_same_absolute_deadline() {
    let (client, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let start = std::time::Instant::now();
    let mut stream = super::Stream::Unix(client);
    let mut writer = super::DeadlineReader { stream: &mut stream, end: start + Duration::from_millis(80) };
    let chunk = [0u8; 64 * 1024];
    let mut timed_out = false;
    // A peer that never drains cannot turn each partial write into a fresh timeout budget.
    for _ in 0..512 {
        if writer.write_all(&chunk).is_err() {
            timed_out = true;
            break;
        }
    }
    assert!(timed_out, "blocked upload must terminate without a reader");
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn recorded_request_body_requires_complete_http_headers() {
    assert!(ipp_body(b"POST / HTTP/1.1\r\n").is_none());
    assert_eq!(ipp_body(b"POST / HTTP/1.1\r\n\r\nbody"), Some(b"body".as_slice()));
}

#[test]
fn named_destinations_encode_one_path_segment_on_both_wire_fields() {
    for is_class in [false, true] {
        let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0, &[&group(2), &attr(0x21, "job-id", &42u32.to_be_bytes())]))]).unwrap();
        let destination = super::Destination { name: "Office café%20?#".into(), is_class };
        assert_eq!(super::submit_as_at(fake.endpoint(), &destination, b"pdf", &Job::default(), "artist").unwrap(), 42);
        let kind = if is_class { "classes" } else { "printers" };
        let path = format!("/{kind}/Office%20caf%C3%A9%2520%3F%23");
        let request = fake.request().unwrap();
        assert!(has(&request, format!("POST {path} HTTP/1.1").as_bytes()));
        assert!(has(ipp_body(&request).unwrap(), format!("ipp://{}{path}", fake.server.authority()).as_bytes()));
    }
}

#[test]
fn destination_path_rejects_routing_syntax_before_connecting() {
    for name in ["", ".", "..", "../admin", "a/b", "a\\b", "a\r\nb", "a\0b"] {
        let destination = super::Destination { name: name.into(), is_class: false };
        assert!(super::destination_path(&destination).is_err(), "{name:?}");
    }
    let destination = super::Destination { name: "x".repeat(257), is_class: false };
    assert!(super::destination_path(&destination).is_err());
}

#[test]
fn separate_queue_query_preserves_optional_discovery_errors() {
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0x0403, &[&group(1)]))]).unwrap();
    assert!(super::queues_at(fake.endpoint(), true).is_err(), "authorization refusal is not an empty class list");
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0x0501, &[&group(1)]))]).unwrap();
    assert!(super::queues_at(fake.endpoint(), true).unwrap().is_empty(), "unsupported class operation is absent");
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0, &[&group(1), &group(4), &printer_name("Pool")]))]).unwrap();
    let queues = super::queues_at(fake.endpoint(), true).unwrap();
    assert_eq!(queues.len(), 1);
    assert!(queues[0].is_class);
    assert!(!queues[0].is_default, "default resolution is caller-owned for this API");
    let request = fake.request().unwrap();
    assert_eq!(&ipp_body(&request).unwrap()[..4], &[1, 1, 0x40, 0x05]);
}

#[test]
fn ipv6_endpoint_keeps_scope_for_resolution_and_escapes_wire_authority() {
    assert_eq!(Server::parse("[::1]").unwrap().authority(), "[::1]:631");
    assert_eq!(Server::parse("::1").unwrap().authority(), "[::1]:631");
    let scoped = Server::parse("[fe80::1%en0]:8631").unwrap();
    assert!(matches!(&scoped, Server::Named(host, 8631) if host == "fe80::1%en0"));
    assert_eq!(scoped.authority(), "[fe80::1%25en0]:8631");
    assert_eq!(Server::parse("fe80::1%en0:8631").unwrap().authority(), scoped.authority());
    assert_eq!(Server::parse("fe80::1%en0").unwrap().authority(), "[fe80::1%25en0]:631");
    assert!(Server::parse("[host]:631").is_err());
    assert_eq!(super::connect(&Server::Named("/invalid".into(), 631)).err().unwrap().kind(), std::io::ErrorKind::InvalidInput);
    for bad in ["[::1]junk", "[::1]:0", "[::1]:abc", "[fe80::1%]:631", "[fe80::1%x%y]:631", "host%zone"] {
        assert!(Server::parse(bad).is_err(), "{bad}");
    }
}

#[test]
fn long_printer_descriptions_are_not_limited_like_queue_names() {
    let description = "Studio ".repeat(200);
    let body = ipp_reply(0, &[&group(1), &group(4), &printer_name("Studio"), &attr(0x41, "printer-info", description.as_bytes())]);
    let parsed = super::parse_reply(&body).unwrap();
    assert_eq!(parsed.queues[0].info, description);
}

#[test]
fn destination_limit_is_explicit_and_never_silently_truncates() {
    let mut body = vec![1, 1, 0, 0, 0, 0, 0, 1];
    for index in 0..super::MAX_PRINTERS {
        body.push(4);
        body.extend(printer_name(&format!("Printer{index}")));
    }
    body.push(3);
    assert_eq!(super::parse_reply(&body).unwrap().queues.len(), super::MAX_PRINTERS);
    assert_eq!(body.pop(), Some(3));
    body.push(4);
    body.extend(printer_name("OneTooMany"));
    body.push(3);
    assert!(super::parse_reply(&body).err().unwrap().to_string().contains("too many CUPS destinations"));
}

#[test]
fn closed_unix_peer_keeps_its_buffered_reply_readable_with_a_deadline() {
    let (client, mut peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let body = ipp_reply(0, &[&group(1), &group(4), &printer_name("Studio")]);
    peer.write_all(&http_reply(&body)).unwrap();
    drop(peer);
    let mut stream = super::Stream::Unix(client);
    let mut reader = super::DeadlineReader { stream: &mut stream, end: std::time::Instant::now() + Duration::from_secs(1) };
    assert_eq!(super::read_reply(&mut reader).unwrap(), body);
    let mut byte = [0];
    assert_eq!(reader.read(&mut byte).unwrap(), 0);
    reader.end = std::time::Instant::now();
    assert_eq!(reader.read(&mut byte).unwrap_err().kind(), std::io::ErrorKind::TimedOut);
}

#[test]
fn stalled_unix_peer_still_fails_within_the_absolute_read_deadline() {
    let (client, _peer) = std::os::unix::net::UnixStream::pair().unwrap();
    let start = std::time::Instant::now();
    let mut stream = super::Stream::Unix(client);
    let mut reader = super::DeadlineReader { stream: &mut stream, end: start + Duration::from_millis(80) };
    let mut byte = [0];
    let error = reader.read(&mut byte).unwrap_err();
    assert!(matches!(error.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock));
    assert!(start.elapsed() < Duration::from_secs(3));
}

#[test]
fn fleet_discovery_opt_in_preserves_all_257_queues_and_default_limit() {
    let mut body = vec![1, 1, 0, 0, 0, 0, 0, 1];
    for index in 0..257 {
        body.push(4);
        body.extend(printer_name(&format!("Printer{index}")));
    }
    body.push(3);
    assert!(parse_reply(&body).is_err());
    assert_eq!(parse_reply_with_limit(&body, 257).unwrap().queues.len(), 257);
    assert!(parse_reply_with_limit(&body, 256).is_err());
    for is_class in [false, true] {
        let server = FakeCups::on_tcp(vec![http_reply(&body)]).unwrap();
        let queues = queues_at_with_limit(server.endpoint(), is_class, 257).unwrap();
        assert_eq!(queues.len(), 257);
        assert_eq!(queues[256].name, "Printer256");
        assert!(queues.iter().all(|q| q.is_class == is_class));
        let request = server.request().unwrap();
        assert!(has(&request, b"requested-attributes"));
    }
}

#[test]
fn fleet_discovery_keeps_absolute_resource_limits() {
    for limit in [0, MAX_DISCOVERY_QUEUES + 1, usize::MAX] {
        assert!(queues_at_with_limit(&[], false, limit).is_err());
    }
    assert!(parse_reply_with_limit(&vec![0; MAX_BODY + 1], MAX_DISCOVERY_QUEUES).is_err());
    let mut body = vec![1, 1, 0, 0, 0, 0, 0, 1, 4];
    body.extend(attr(0x42, &"x".repeat(MAX_NAME + 1), b"value"));
    body.push(3);
    assert!(parse_reply_with_limit(&body, MAX_DISCOVERY_QUEUES).is_err());
}
