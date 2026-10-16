//! The IPP client against a fake CUPS server (a unix socket or a loopback port): discovery,
//! Print-Job, and the malformed replies it must refuse without a panic.

use super::*;
use crate as spool;
use crate::ipp::{self, Server};
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
    fn on_unix(replies: Vec<Vec<u8>>) -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("cups.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        // The socket must outlive this function; the folder is left for the process to reap.
        std::mem::forget(dir);
        let requests = listen_unix(listener, replies);
        FakeCups { server: Server::Unix(path), requests }
    }

    fn on_tcp(replies: Vec<Vec<u8>>) -> Self {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = Server::Tcp(listener.local_addr().unwrap());
        let requests = listen_tcp(listener, replies);
        FakeCups { server, requests }
    }

    fn request(&self) -> Vec<u8> {
        self.requests.recv_timeout(Duration::from_secs(5)).expect("the fake server took the request")
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
    let _ = requests.send(request);
    // A list out of replies is a server that hangs up before answering.
    if let Some(reply) = replies.lock().unwrap().pop_front() {
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
fn ipp_body(request: &[u8]) -> &[u8] {
    let at = request.windows(4).position(|w| w == b"\r\n\r\n").expect("an HTTP request has a head");
    &request[at + 4..]
}

fn entry(name: &str, default: bool) -> spool::Printer {
    spool::Printer { name: name.into(), default, two_sided: None }
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
    ]);
    let printers = ipp::printers_at(fake.endpoint()).unwrap();
    assert_eq!(printers, [entry("Office_Laser", true), entry("Label_Writer", false)], "each queue once, the default marked");
    let taken = fake.request();
    let queues = ipp_body(&taken);
    assert_eq!(&queues[..4], &[1, 1, 0x40, 0x02], "IPP 1.1, CUPS-Get-Printers");
    assert!(has(queues, b"ipp://localhost/"));
    assert!(has(queues, &attr(0x44, "requested-attributes", b"printer-name")));
    assert!(has(queues, b"printer-is-accepting-jobs"), "only the attributes the dialog needs are asked for");
    let taken = fake.request();
    let default = ipp_body(&taken);
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
    ))]);
    let message = ipp::print_job_at(fake.endpoint(), &pdf, &job).unwrap();
    assert_eq!(message, "request id is Office-12", "the same message lp prints");
    let taken = fake.request();
    let request = ipp_body(&taken);
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
    ]);
    let message = ipp::print_job_at(fake.endpoint(), b"%PDF-1.7\n", &Job::default()).unwrap();
    assert_eq!(message, "request id is Office-7");
    let default = fake.request();
    assert_eq!(&ipp_body(&default)[..4], &[1, 1, 0x40, 0x01], "the default comes from CUPS-Get-Default");
    let taken = fake.request();
    assert!(has(ipp_body(&taken), b"ipp://localhost/printers/Office"), "the resolved default is the target");
}

#[test]
fn a_default_outside_the_queue_list_still_gets_its_marker() {
    let fake = FakeCups::on_unix(vec![
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Office_Laser")])),
        http_reply(&ipp_reply(0x0000, &[&group(0x01), &group(0x04), &printer_name("Label_Writer")])),
    ]);
    let printers = ipp::printers_at(fake.endpoint()).unwrap();
    assert_eq!(
        printers,
        [entry("Office_Laser", false), entry("Label_Writer", true)],
        "the default joins the list, so the dialog has a selection to restore"
    );
}

#[test]
fn no_default_printer_is_an_error_when_the_job_names_none() {
    let fake = FakeCups::on_unix(vec![http_reply(&ipp_reply(0x0000, &[&group(0x01)]))]);
    let err = ipp::print_job_at(fake.endpoint(), b"%PDF-1.7\n", &Job::default()).unwrap_err();
    assert!(err.to_string().contains("no default printer"), "{err}");
}

#[test]
fn a_printer_name_that_cannot_be_a_uri_path_is_refused() {
    let fake = FakeCups::on_tcp(vec![]);
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
    let fake = FakeCups::on_tcp(vec![http_reply(&ipp_reply(0x0506, &[&group(0x01)]))]);
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
        let fake = FakeCups::on_unix(vec![http_reply(&body)]);
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
        let fake = FakeCups::on_unix(vec![reply]);
        let result = ipp::post(&[fake.server], "/", b"ignored", TIMEOUT, true);
        assert!(result.is_err(), "{why}: the reply must be an error, not a panic or a success");
    }
    // A server that takes the request and hangs up.
    let fake = FakeCups::on_unix(vec![]);
    assert!(ipp::post(&[fake.server], "/", b"ignored", TIMEOUT, true).is_err(), "a hang-up is an error");
}

#[test]
fn a_server_that_answers_is_reachable_and_silence_is_not() {
    let fake = FakeCups::on_unix(vec![]);
    assert!(ipp::reachable(&fake.server), "a unix socket that accepts connections is reachable");
    let fake = FakeCups::on_tcp(vec![]);
    assert!(ipp::reachable(&fake.server), "a loopback port that accepts connections is reachable");
    let absent = Server::Unix("/nonexistent/craft-print-test.sock".into());
    assert!(!ipp::reachable(&absent), "a socket nothing listens on is not reachable");
    let addr = {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        listener.local_addr().unwrap()
    };
    assert!(!ipp::reachable(&Server::Tcp(addr)), "a port nothing listens on is not reachable");
}
