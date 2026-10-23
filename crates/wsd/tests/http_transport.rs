//! The real blocking transport against loopback servers: the policy behaviours (redirects refused,
//! replies capped, timeouts) and the headers a request carries.
#![cfg(not(target_arch = "wasm32"))]
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::indexing_slicing)]

use std::sync::{Arc, Mutex};
use std::time::Duration;

use craft_wsd::http::HttpTransport;
use craft_wsd::testing::{Server, raw_response};
use craft_wsd::{DeviceUrl, Error, Request, RetrySafety, Stage, Transport, TransportKind};

/// The `/ws` endpoint of `server`.
fn url(server: &Server) -> DeviceUrl {
    DeviceUrl::parse(&server.url("/ws")).unwrap()
}

/// A small SOAP request with a five-second timeout and a 64 KiB reply cap.
fn request() -> Request {
    Request {
        content_type: "application/soap+xml".into(),
        soap_action: "urn:example/Action".into(),
        body: b"<x/>".to_vec(),
        timeout: Duration::from_secs(5),
        max_response: 64 << 10,
    }
}

#[test]
fn a_reply_comes_back_with_status_type_and_body() {
    let server = Server::start(|_| raw_response(200, "application/soap+xml", b"<ok/>"));
    let r = HttpTransport::new().post(&url(&server), request()).unwrap();
    assert_eq!((r.status, r.content_type.as_deref(), r.body.as_slice()), (200, Some("application/soap+xml"), &b"<ok/>"[..]));
    // A non-success status is data, not an error: the caller decides (it may carry a SOAP fault).
    let busy = Server::start(|_| raw_response(503, "text/plain", b"busy"));
    assert_eq!(HttpTransport::new().post(&url(&busy), request()).unwrap().status, 503);
}

#[test]
fn redirects_are_refused_not_followed() {
    let target = Server::start(|_| raw_response(200, "text/plain", b"you should never get here"));
    let target_url = target.url("/elsewhere");
    let redirecting = Server::start(move |_| {
        format!("HTTP/1.1 302 Found\r\nLocation: {target_url}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").into_bytes()
    });
    let e = HttpTransport::new().post(&url(&redirecting), request()).unwrap_err();
    assert!(matches!(e, Error::Transport { kind: TransportKind::Redirect, ref message, .. } if message.contains("redirect")), "{e:?}");
    assert!(!e.is_retryable());
    assert_eq!(*target.hits.lock().unwrap(), 0, "the redirect target was never contacted");
}

#[test]
fn oversized_replies_are_cut_off() {
    let big = Server::start(|_| raw_response(200, "application/soap+xml", &vec![b'<'; 3 << 20]));
    let e = HttpTransport::new().post(&url(&big), request()).unwrap_err();
    assert!(matches!(e, Error::TooLarge(..)), "{e:?}");
}

#[test]
fn a_silent_device_times_out() {
    let silent = Server::start(|_| {
        std::thread::sleep(Duration::from_millis(1500));
        Vec::new()
    });
    let started = std::time::Instant::now();
    let e = HttpTransport::new().post(&url(&silent), Request { timeout: Duration::from_millis(300), ..request() }).unwrap_err();
    assert_eq!(e.transport_kind(), Some(TransportKind::Timeout), "{e:?}");
    assert!(e.is_retryable() && e.retry_safety(false) == RetrySafety::OutcomeUnknown, "a silent device may have the job: {e:?}");
    assert!(started.elapsed() < Duration::from_millis(1400), "{:?}", started.elapsed());
}

#[test]
fn requests_carry_the_soap_headers() {
    let seen = Arc::new(Mutex::new(Vec::<(String, String, String, Vec<u8>)>::new()));
    let s = seen.clone();
    let server = Server::start(move |req| {
        s.lock().unwrap().push((
            req.header("soapaction").unwrap_or("").to_owned(),
            req.header("content-type").unwrap_or("").to_owned(),
            req.header("host").unwrap_or("").to_owned(),
            req.body.clone(),
        ));
        raw_response(200, "text/plain", b"")
    });
    HttpTransport::new().post(&url(&server), request()).unwrap();
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0].0, "\"urn:example/Action\"");
    assert_eq!(seen[0].1, "application/soap+xml");
    assert!(seen[0].2.starts_with("127.0.0.1:"));
    assert_eq!(seen[0].3, b"<x/>");
}

#[test]
fn a_server_that_closes_without_replying_fails_fast() {
    let server = Server::start(|_| Vec::new());
    let mut r = request();
    r.timeout = Duration::from_secs(20);
    let started = std::time::Instant::now();
    let e = HttpTransport::new().post(&url(&server), r).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}: {e:?}", started.elapsed());
    assert_eq!(e.transport_kind(), Some(TransportKind::ConnectionClosed), "{e:?}");
    assert!(e.is_retryable());
}

/// One connection, kept alive: the first request is answered, the second is read and the socket
/// closed without a reply (what an embedded server does with a request it dislikes).
#[test]
fn a_reused_connection_closed_without_a_reply_fails_fast() {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let handle = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut buf = [0u8; 4096];
        let _ = s.read(&mut buf).unwrap();
        s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: text/plain\r\nContent-Length: 2\r\n\r\nok").unwrap();
        let _ = s.read(&mut buf).unwrap();
        drop(s);
        // Anything that arrives on a fresh connection is also dropped.
        for s in listener.incoming().take(3).flatten() {
            let mut b = [0u8; 4096];
            let mut s = s;
            let _ = s.read(&mut b);
        }
    });
    let u = DeviceUrl::parse(&format!("http://127.0.0.1:{port}/ws")).unwrap();
    let t = HttpTransport::new();
    let mut r = request();
    r.timeout = Duration::from_secs(20);
    assert_eq!(t.post(&u, r.clone()).unwrap().status, 200);
    let started = std::time::Instant::now();
    let e = t.post(&u, r).unwrap_err();
    assert!(started.elapsed() < Duration::from_secs(5), "took {:?}: {e:?}", started.elapsed());
    assert_eq!(e.transport_kind(), Some(TransportKind::ConnectionClosed), "{e:?}");
    assert!(e.is_retryable());
    drop(handle);
}

/// Run one request against a loopback server that misbehaves as `serve` says; the error and how long it took.
fn behaviour(serve: impl Fn(std::net::TcpStream) + Send + 'static) -> (std::time::Duration, Error) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    std::thread::spawn(move || {
        for s in listener.incoming().take(4).flatten() {
            serve(s);
        }
    });
    let u = DeviceUrl::parse(&format!("http://127.0.0.1:{port}/ws")).unwrap();
    let mut r = request();
    r.timeout = Duration::from_secs(8);
    let started = std::time::Instant::now();
    let e = HttpTransport::new().post(&u, r).unwrap_err();
    (started.elapsed(), e)
}

#[test]
fn a_peer_that_closes_early_or_half_closes_fails_fast_as_connection_closed() {
    use std::io::Read;
    use std::net::Shutdown;
    // Reads only part of the request, then closes (a reset on most stacks).
    let (took, e) = behaviour(|mut s| {
        let mut b = [0u8; 20];
        let _ = s.read(&mut b);
        drop(s);
    });
    assert!(took < Duration::from_secs(4) && e.transport_kind() == Some(TransportKind::ConnectionClosed), "{took:?} {e:?}");
    // Reads the request, shuts down its sending side and keeps the socket open: still an immediate failure.
    let (took, e) = behaviour(|mut s| {
        let mut b = [0u8; 4096];
        let _ = s.read(&mut b);
        let _ = s.shutdown(Shutdown::Write);
        std::thread::sleep(Duration::from_secs(10));
    });
    assert!(took < Duration::from_secs(4) && e.transport_kind() == Some(TransportKind::ConnectionClosed), "{took:?} {e:?}");
}

#[test]
fn nothing_listening_is_refused_at_the_connect_stage() {
    let port = {
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        l.local_addr().unwrap().port()
    };
    let u = DeviceUrl::parse(&format!("http://127.0.0.1:{port}/ws")).unwrap();
    let e = HttpTransport::new().post(&u, request()).unwrap_err();
    assert_eq!((e.transport_kind(), e.stage()), (Some(TransportKind::Refused), Stage::Connect), "{e:?}");
    assert!(e.is_retryable());
    assert_eq!(e.retry_safety(false), RetrySafety::Safe, "nothing was sent, so even a submit may be repeated");
}

/// A one-byte `status` reply with the extra raw `headers` (each ending in CRLF).
fn reply_with(status: u16, headers: &str) -> Vec<u8> {
    format!("HTTP/1.1 {status} X\r\n{headers}Content-Type: text/plain\r\nContent-Length: 1\r\nConnection: close\r\n\r\nx").into_bytes()
}

/// The `Retry-After` wait the transport reads from a 503 reply carrying `headers`.
fn retry_after_of(headers: &'static str) -> Option<Duration> {
    let server = Server::start(move |_| reply_with(503, headers));
    HttpTransport::new().post(&url(&server), request()).unwrap().retry_after
}

#[test]
fn retry_after_comes_back_parsed_and_capped_and_hostile_values_are_ignored() {
    assert_eq!(retry_after_of("Retry-After: 12\r\n"), Some(Duration::from_secs(12)));
    assert_eq!(retry_after_of("Retry-After: 99999999999999999999999999\r\n"), Some(craft_wsd::MAX_RETRY_AFTER));
    assert_eq!(retry_after_of("Retry-After: Fri, 31 Dec 9999 23:59:59 GMT\r\n"), Some(craft_wsd::MAX_RETRY_AFTER));
    assert_eq!(retry_after_of("Retry-After: Sun, 06 Nov 1994 08:49:37 GMT\r\n"), Some(Duration::ZERO), "a date in the past is no wait");
    for hostile in [
        "Retry-After: -5\r\n",
        "Retry-After: 1.5\r\n",
        "Retry-After: soon\r\n",
        "Retry-After: 5, 10\r\n",
        "Retry-After:\r\n",
        "Retry-After: 4\r\nRetry-After: 9\r\n",
    ] {
        assert_eq!(retry_after_of(hostile), None, "{hostile:?}");
    }
    assert_eq!(retry_after_of(""), None);
    assert_eq!(retry_after_of("Retry-After: 4\r\nRetry-After: 4\r\n"), Some(Duration::from_secs(4)));
    // The value reaches the Response whatever the status; the reader decides what it means.
    let ok = Server::start(|_| reply_with(200, "Retry-After: 3\r\n"));
    assert_eq!(HttpTransport::new().post(&url(&ok), request()).unwrap().retry_after, Some(Duration::from_secs(3)));
}

#[test]
fn a_peer_that_does_not_speak_http_is_an_invalid_response() {
    use std::io::{Read, Write};
    for garbage in [&b"\x00\x01\x02 this is not http at all\r\n\r\n"[..], b"HTTP/9.9 banana\r\n\r\n", b"SSH-2.0-OpenSSH_9.0\r\n"] {
        let (took, e) = behaviour(move |mut s| {
            let mut b = [0u8; 4096];
            let _ = s.read(&mut b);
            let _ = s.write_all(garbage);
            let _ = s.flush();
            std::thread::sleep(Duration::from_millis(300));
        });
        assert!(took < Duration::from_secs(5), "{took:?} {e:?}");
        assert_eq!((e.transport_kind(), e.stage()), (Some(TransportKind::InvalidResponse), Stage::Reply), "{e:?}");
        assert!(!e.is_retryable());
        assert_eq!(e.retry_safety(false), RetrySafety::OutcomeUnknown);
        assert_eq!(e.retry_safety(true), RetrySafety::Idempotent);
    }
}

#[test]
fn an_oversized_header_block_is_an_invalid_response() {
    use std::io::{Read, Write};
    let (_, e) = behaviour(|mut s| {
        let mut b = [0u8; 4096];
        let _ = s.read(&mut b);
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nX-Junk: ");
        let _ = s.write_all(&vec![b'a'; 200_000]);
        let _ = s.write_all(b"\r\n\r\n");
        std::thread::sleep(Duration::from_millis(300));
    });
    assert_eq!(e.transport_kind(), Some(TransportKind::InvalidResponse), "{e:?}");
}

/// The header promises more body than arrives and the socket is then reset: the device went away
/// mid-reply, which is a close (or, if the stack reports it otherwise, an I/O error), never HTTP nonsense.
#[test]
fn a_reset_in_the_middle_of_the_body_is_a_close_or_an_io_error() {
    use std::io::{Read, Write};
    let (took, e) = behaviour(|mut s| {
        let mut b = [0u8; 4096];
        let _ = s.read(&mut b);
        let _ = s.write_all(b"HTTP/1.1 200 OK\r\nContent-Type: application/soap+xml\r\nContent-Length: 100000\r\n\r\n<partial");
        let _ = s.flush();
        std::thread::sleep(Duration::from_millis(100));
        drop(s);
    });
    assert!(took < Duration::from_secs(5), "{took:?} {e:?}");
    assert!(matches!(e.transport_kind(), Some(TransportKind::ConnectionClosed | TransportKind::Io)), "{e:?}");
    assert_eq!(e.stage(), Stage::Reply);
    assert!(e.is_retryable());
    assert_eq!(e.retry_safety(false), RetrySafety::OutcomeUnknown);
}
