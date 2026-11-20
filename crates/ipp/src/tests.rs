//! The client against an in-process fake CUPS: TCP and Unix socket, content-length and chunked
//! replies, every operation, error statuses and broken replies.

use super::*;
use std::io::{BufRead, BufReader};
use std::sync::{Arc, Mutex, mpsc};

#[derive(Default)]
struct ConnectTrace {
    calls: Vec<(SocketAddr, Duration)>,
    active: usize,
    peak: usize,
}

impl ConnectTrace {
    fn start(&mut self, addr: SocketAddr, timeout: Duration) {
        self.calls.push((addr, timeout));
        self.active += 1;
        self.peak = self.peak.max(self.active);
    }

    fn finish(&mut self) {
        self.active -= 1;
    }

    fn assert_addresses(&self, expected: &[SocketAddr]) {
        // Callback entry order depends on worker scheduling, not resolver order.
        let mut actual: Vec<_> = self.calls.iter().map(|(addr, _)| *addr).collect();
        let mut expected = expected.to_vec();
        actual.sort_unstable();
        expected.sort_unstable();
        assert_eq!(actual, expected);
    }
}

/// What the fake received: op, attributes, document.
type Seen = (u16, Vec<Attribute>, Vec<u8>);

/// A fake server answering one request with `reply_body` (IPP bytes), chunked or not, under an
/// HTTP status. Returns the endpoint and a channel with what it received.
fn fake(reply: Vec<u8>, chunked: bool, http_status: u16) -> (Endpoint, mpsc::Receiver<Seen>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        // This fake serves one request only: later optional probes must be refused, not queued.
        drop(listener);
        serve(Stream::Tcp(stream), reply, chunked, http_status, &tx);
    });
    (Endpoint::Tcp("127.0.0.1".into(), port), rx)
}

fn serve(stream: Stream, reply: Vec<u8>, chunked: bool, http_status: u16, tx: &mpsc::Sender<Seen>) {
    let mut reader = BufReader::new(stream);
    let mut line = String::new();
    let mut content_length = 0usize;
    let mut first = String::new();
    loop {
        line.clear();
        reader.read_line(&mut line).unwrap();
        if first.is_empty() {
            first = line.clone();
        }
        if line == "\r\n" || line.is_empty() {
            break;
        }
        if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
            content_length = v.trim().parse().unwrap();
        }
    }
    if first.is_empty() {
        return; // a reachability probe: connect and close
    }
    assert!(first.starts_with("POST "), "{first}");
    let mut body = vec![0u8; content_length];
    reader.read_exact(&mut body).unwrap();
    let _ = tx.send(parse_request(&body).unwrap());
    let mut out = reader.into_inner();
    let status_text = if http_status == 200 { "OK" } else { "Nope" };
    if chunked {
        let mut resp =
            format!("HTTP/1.1 {http_status} {status_text}\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n").into_bytes();
        for chunk in reply.chunks(7) {
            resp.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
            resp.extend_from_slice(chunk);
            resp.extend_from_slice(b"\r\n");
        }
        resp.extend_from_slice(b"0\r\n\r\n");
        out.write_all(&resp).unwrap();
    } else {
        out.write_all(
            format!("HTTP/1.1 {http_status} {status_text}\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", reply.len()).as_bytes(),
        )
        .unwrap();
        out.write_all(&reply).unwrap();
    }
    out.flush().unwrap();
}

/// An attribute to write: tag, name, values.
type Attr<'a> = (u8, &'a str, &'a [&'a [u8]]);
/// A group to write: tag and attributes.
type Group<'a> = (u8, &'a [Attr<'a>]);

/// An IPP reply body.
fn ipp(status: u16, groups: &[Group<'_>]) -> Vec<u8> {
    let mut b = vec![2, 0];
    b.extend_from_slice(&status.to_be_bytes());
    b.extend_from_slice(&1u32.to_be_bytes());
    for (group, attrs) in groups {
        b.push(*group);
        for (tag, name, values) in *attrs {
            for (i, v) in values.iter().enumerate() {
                b.push(*tag);
                let n = if i == 0 { name.as_bytes() } else { b"" };
                b.extend_from_slice(&(n.len() as u16).to_be_bytes());
                b.extend_from_slice(n);
                b.extend_from_slice(&(v.len() as u16).to_be_bytes());
                b.extend_from_slice(v);
            }
        }
    }
    b.push(TAG_END);
    b
}

fn ok_header() -> Group<'static> {
    (TAG_OPERATION_GROUP, &[(TAG_CHARSET, "attributes-charset", &[b"utf-8"]), (TAG_LANGUAGE, "attributes-natural-language", &[b"en"])])
}

#[test]
fn printers_are_listed_with_state_and_default() {
    let three = ipp(
        0,
        &[
            ok_header(),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_NAME, "printer-name", &[b"Office"]),
                    (TAG_TEXT, "printer-info", &[b"HP in the office"]),
                    (TAG_ENUM, "printer-state", &[&3u32.to_be_bytes()]),
                    (TAG_BOOLEAN, "printer-is-accepting-jobs", &[&[1]]),
                ],
            ),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_NAME, "printer-name", &[b"Stopped"]),
                    (TAG_ENUM, "printer-state", &[&5u32.to_be_bytes()]),
                    (TAG_BOOLEAN, "printer-is-accepting-jobs", &[&[0]]),
                ],
            ),
        ],
    );
    for chunked in [false, true] {
        let (ep, rx) = fake(three.clone(), chunked, 200);
        // `printers` asks for the default too; that second request reaches a server that has
        // closed, so the default is simply unknown here.
        let list = printers(&ep).unwrap();
        let (op, attrs, doc) = rx.recv().unwrap();
        assert_eq!(op, OP_CUPS_GET_PRINTERS);
        assert!(attrs.iter().any(|a| a.name == "requested-attributes" && a.values.len() == 4 && a.tag == TAG_KEYWORD));
        assert!(doc.is_empty());
        assert_eq!(list.len(), 2, "chunked {chunked}");
        assert_eq!((list[0].name.as_str(), list[0].info.as_str(), list[0].state, list[0].accepting_jobs), ("Office", "HP in the office", 3, true));
        assert_eq!((list[1].name.as_str(), list[1].state, list[1].accepting_jobs), ("Stopped", 5, false));
        assert!(!list[0].is_default && !list[1].is_default);
    }
}

#[test]
fn printer_attributes_before_name_are_preserved() {
    let state = 5u32.to_be_bytes();
    let base: [Attr<'_>; 4] = [
        (TAG_TEXT, "printer-info", &[b"Studio Printer"]),
        (TAG_ENUM, "printer-state", &[&state]),
        (TAG_BOOLEAN, "printer-is-accepting-jobs", &[b"\0"]),
        (TAG_NAME, "printer-name", &[b"studio"]),
    ];
    let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &base)]);
    for chunked in [false, true] {
        let (ep, _) = fake(body.clone(), chunked, 200);
        let list = printers(&ep).unwrap();
        assert_eq!(list.len(), 1, "chunked {chunked}");
        assert_eq!((list[0].name.as_str(), list[0].info.as_str(), list[0].state, list[0].accepting_jobs), ("studio", "Studio Printer", 5, false));
        assert!(!list[0].is_class);
    }
}

#[test]
fn printer_groups_do_not_share_metadata() {
    let state_a = 4u32.to_be_bytes();
    let state_b = 5u32.to_be_bytes();
    let body = ipp(
        0,
        &[
            ok_header(),
            (
                TAG_PRINTER_GROUP,
                &[(TAG_NAME, "printer-name", &[b"first"]), (TAG_TEXT, "printer-info", &[b"First printer"]), (TAG_ENUM, "printer-state", &[&state_a])],
            ),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_TEXT, "printer-info", &[b"Second printer"]),
                    (TAG_ENUM, "printer-state", &[&state_b]),
                    (TAG_NAME, "printer-name", &[b"second"]),
                ],
            ),
        ],
    );
    let (ep, _) = fake(body, false, 200);
    let list = queues(&ep, OP_CUPS_GET_PRINTERS, false).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].name.as_str(), list[0].info.as_str(), list[0].state), ("first", "First printer", 4));
    assert_eq!((list[1].name.as_str(), list[1].info.as_str(), list[1].state), ("second", "Second printer", 5));
}

#[test]
fn printer_attribute_order_does_not_change_results() {
    let state = 5u32.to_be_bytes();
    let fields: [Attr<'_>; 4] = [
        (TAG_TEXT, "printer-info", &[b"Studio Printer"]),
        (TAG_ENUM, "printer-state", &[&state]),
        (TAG_BOOLEAN, "printer-is-accepting-jobs", &[b"\0"]),
        (TAG_NAME, "printer-name", &[b"studio"]),
    ];
    let expected =
        Printer { name: "studio".into(), info: "Studio Printer".into(), state: 5, accepting_jobs: false, is_default: false, is_class: false };
    let mut permutations = 0;
    for first in 0..4 {
        for second in 0..4 {
            for third in 0..4 {
                for fourth in 0..4 {
                    let order = [first, second, third, fourth];
                    if order[0] == order[1]
                        || order[0] == order[2]
                        || order[0] == order[3]
                        || order[1] == order[2]
                        || order[1] == order[3]
                        || order[2] == order[3]
                    {
                        continue;
                    }
                    permutations += 1;
                    let ordered = order.iter().map(|index| fields[*index]).collect::<Vec<_>>();
                    let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &ordered)]);
                    let (ep, _) = fake(body, false, 200);
                    assert_eq!(queues(&ep, OP_CUPS_GET_PRINTERS, false).unwrap(), vec![expected.clone()], "order {order:?}");
                }
            }
        }
    }
    assert_eq!(permutations, 24);
}

#[test]
fn class_attributes_before_name_are_preserved() {
    let state = 5u32.to_be_bytes();
    let body = ipp(
        0,
        &[
            ok_header(),
            (
                TAG_PRINTER_GROUP,
                &[
                    (TAG_BOOLEAN, "printer-is-accepting-jobs", &[b"\0"]),
                    (TAG_TEXT, "printer-info", &[b"Shared studio"]),
                    (TAG_ENUM, "printer-state", &[&state]),
                    (TAG_NAME, "printer-name", &[b"studio-class"]),
                ],
            ),
        ],
    );
    let (ep, rx) = fake(body, true, 200);
    let classes = queues(&ep, OP_CUPS_GET_CLASSES, true).unwrap();
    assert_eq!(rx.recv().unwrap().0, OP_CUPS_GET_CLASSES);
    assert_eq!(classes.len(), 1);
    assert_eq!(
        (classes[0].name.as_str(), classes[0].info.as_str(), classes[0].state, classes[0].accepting_jobs, classes[0].is_class),
        ("studio-class", "Shared studio", 5, false, true)
    );
}

#[test]
fn empty_or_unnamed_groups_do_not_leak_metadata() {
    let body = ipp(
        0,
        &[
            ok_header(),
            (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"named"])]),
            (TAG_PRINTER_GROUP, &[]),
            (TAG_PRINTER_GROUP, &[(TAG_TEXT, "printer-info", &[b"orphan metadata"])]),
            (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"next"]), (TAG_ENUM, "printer-state", &[&3u32.to_be_bytes()])]),
        ],
    );
    let (ep, _) = fake(body, false, 200);
    let list = queues(&ep, OP_CUPS_GET_PRINTERS, false).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!((list[0].name.as_str(), list[0].info.as_str()), ("named", ""));
    assert_eq!((list[1].name.as_str(), list[1].info.as_str()), ("next", ""));
}

#[test]
fn additional_values_do_not_cross_group_boundaries() {
    let valid = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_KEYWORD, "supported", &[b"one", b"two"])])]);
    let reply = Reply::parse(&valid).unwrap();
    assert_eq!(reply.attributes.iter().find(|a| a.name == "supported").map(|a| a.values.as_slice()), Some(&[b"one".to_vec(), b"two".to_vec()][..]));

    let mut body = vec![2, 0, 0, 0, 0, 0, 0, 1, TAG_PRINTER_GROUP];
    body.extend_from_slice(&[TAG_NAME, 0, 12]);
    body.extend_from_slice(b"printer-name");
    body.extend_from_slice(&[0, 5]);
    body.extend_from_slice(b"first");
    body.extend_from_slice(&[TAG_TEXT, 0, 12]);
    body.extend_from_slice(b"printer-info");
    body.extend_from_slice(&[0, 6]);
    body.extend_from_slice(b"before");
    body.push(TAG_PRINTER_GROUP);
    body.extend_from_slice(&[TAG_TEXT, 0, 0, 0, 5]);
    body.extend_from_slice(b"after");
    body.extend_from_slice(&[TAG_NAME, 0, 12]);
    body.extend_from_slice(b"printer-name");
    body.extend_from_slice(&[0, 6]);
    body.extend_from_slice(b"second");
    body.push(TAG_END);
    let (ep, _) = fake(body, true, 200);
    assert!(matches!(queues(&ep, OP_CUPS_GET_PRINTERS, false), Err(Error::Protocol(_))));
}

#[test]
fn default_printer_is_read_and_absence_is_not_an_error() {
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]), false, 200);
    assert_eq!(default_printer(&ep).unwrap(), Some("Office".into()));
    assert_eq!(rx.recv().unwrap().0, OP_CUPS_GET_DEFAULT);
    let (ep, _rx) = fake(ipp(0x0406, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"No default printer"])])]), false, 200);
    assert_eq!(default_printer(&ep).unwrap(), None);
}

#[test]
fn a_print_job_carries_the_document_and_returns_its_id() {
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&42u32.to_be_bytes()])])]), true, 200);
    let pdf = b"%PDF-1.4 fake".to_vec();
    let id = print_job(
        &ep,
        &Job { printer: "Office", is_class: false, title: "photo.psd", user: "diego", copies: 2, document: &pdf, format: "application/pdf" },
    )
    .unwrap();
    assert_eq!(id, 42);
    let (op, attrs, doc) = rx.recv().unwrap();
    assert_eq!(op, OP_PRINT_JOB);
    assert_eq!(doc, pdf);
    let get = |n: &str| attrs.iter().find(|a| a.name == n).cloned().unwrap_or_else(|| panic!("{n}"));
    let uri = String::from_utf8(get("printer-uri").values[0].clone()).unwrap();
    // Spelled out here rather than echoed back from host(), so the printer-uri the server receives is
    // pinned down even when the authority is an IP literal.
    assert!(uri.starts_with("ipp://127.0.0.1:") && uri.ends_with("/printers/Office"), "{uri}");
    assert_eq!(get("job-name").values[0], b"photo.psd");
    assert_eq!(get("requesting-user-name").values[0], b"diego");
    assert_eq!(get("document-format").values[0], b"application/pdf");
    let copies = get("copies");
    assert_eq!((copies.group, copies.tag, copies.int()), (TAG_JOB_GROUP, TAG_INTEGER, Some(2)));
    // One copy: no job attributes group at all.
    let (ep, rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&7u32.to_be_bytes()])])]), false, 200);
    print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" }).unwrap();
    assert!(!rx.recv().unwrap().1.iter().any(|a| a.name == "copies"));
}

#[test]
fn error_statuses_and_broken_replies_are_errors() {
    let (ep, _rx) =
        fake(ipp(0x0506, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"Printer is not accepting jobs"])])]), false, 200);
    let e = print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" })
        .unwrap_err();
    assert_eq!(e, Error::Status(0x0506, "Printer is not accepting jobs".into()));
    assert!(e.to_string().contains("0x0506"));
    // A CUPS with no printer at all answers "not found": that is an empty list, not an error.
    let (ep, _rx) =
        fake(ipp(0x0406, &[ok_header(), (TAG_OPERATION_GROUP, &[(TAG_TEXT, "status-message", &[b"No destinations added."])])]), false, 200);
    assert_eq!(printers(&ep).unwrap(), Vec::<Printer>::new());
    // HTTP 401, a reply cut short, garbage.
    let (ep, _rx) = fake(ipp(0, &[ok_header()]), false, 401);
    assert!(matches!(printers(&ep), Err(Error::Protocol(m)) if m.contains("401")));
    let (ep, _rx) = fake(vec![2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP, TAG_NAME, 0, 5], true, 200);
    assert!(matches!(printers(&ep), Err(Error::Protocol(_))));
    let (ep, _rx) = fake(b"not ipp at all".to_vec(), false, 200);
    assert!(matches!(printers(&ep), Err(Error::Protocol(_))));
    // Nobody reachable. A port is never guaranteed to be free, let alone to stay free: that is what
    // made the old port-1 case fail on a FreeBSD CI VM. A name that can never resolve (RFC 2606)
    // reaches the same exit without owning or releasing anything.
    let e = printers(&Endpoint::Tcp("printer.invalid".into(), 631)).unwrap_err();
    assert!(matches!(e, Error::Unreachable(_)), "{e}");
    assert!(e.to_string().contains("printer.invalid"), "{e} does not name the server it tried");
}

#[test]
fn a_refused_connection_is_unreachable_and_keeps_why() {
    // The refused-connect path, with the machine no longer deciding what is closed: the connect call
    // is a parameter, so this test cannot race for an ephemeral port with anything.
    let addr: SocketAddr = "192.0.2.9:631".parse().unwrap();
    let asked_for = Arc::new(Mutex::new(None));
    let record = Arc::clone(&asked_for);
    let open = move |seen: SocketAddr, _timeout: Duration| -> std::io::Result<TcpStream> {
        *record.lock().unwrap() = Some(seen);
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "nobody is listening on this port"))
    };
    let e = match open_tcp("printserver", 631, [addr], open) {
        Ok(_) => panic!("nothing opens {addr}"),
        Err(e) => e,
    };
    assert_eq!(*asked_for.lock().unwrap(), Some(addr), "the connector is handed the resolved address");
    match &e {
        Error::Unreachable(m) => {
            assert!(m.contains("printserver:631"), "{e} does not name the endpoint");
            assert!(m.contains("nobody is listening on this port"), "{e} loses why the connect failed");
        }
        other => panic!("expected Unreachable, got {other}"),
    }
}

#[test]
fn reply_parsing_is_bounded() {
    assert!(Reply::parse(&[]).is_err());
    assert!(Reply::parse(&[2, 0, 0, 0, 0, 0, 0, 1]).is_err(), "no end tag");
    let mut many = vec![2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP];
    for _ in 0..MAX_ATTRIBUTES + 1 {
        many.extend_from_slice(&[TAG_NAME, 0, 1, b'a', 0, 0]);
    }
    many.push(TAG_END);
    assert!(matches!(Reply::parse(&many), Err(Error::Protocol(m)) if m.contains("too many")));
    // An additional value before any attribute.
    assert!(Reply::parse(&[2, 0, 0, 0, 0, 0, 0, 1, TAG_OPERATION_GROUP, TAG_NAME, 0, 0, 0, 1, b'x', TAG_END]).is_err());
    assert!(dechunk(b"zz\r\n").is_err() && dechunk(b"5\r\nab").is_err());
    assert_eq!(dechunk(b"3\r\nabc\r\n0\r\n\r\n").unwrap(), b"abc");
}

#[cfg(unix)]
#[test]
fn the_unix_socket_endpoint_works_and_detect_honours_cups_server() {
    let dir = std::env::temp_dir().join(format!("pc-ipp-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let sock = dir.join("cups.sock");
    let listener = std::os::unix::net::UnixListener::bind(&sock).unwrap();
    let (tx, rx) = mpsc::channel();
    let reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Sock"])])]);
    std::thread::spawn(move || {
        for _ in 0..3 {
            let (stream, _) = listener.accept().unwrap();
            serve(Stream::Unix(stream), reply.clone(), false, 200, &tx);
        }
    });
    let ep = Endpoint::Unix(sock.clone());
    assert!(ep.reachable());
    assert_eq!(default_printer(&ep).unwrap(), Some("Sock".into()));
    assert_eq!(rx.recv().unwrap().0, OP_CUPS_GET_DEFAULT);
    assert!(!Endpoint::Unix(dir.join("missing.sock")).reachable());
    assert_eq!(ep.to_string(), sock.display().to_string());
    let _ = std::fs::remove_dir_all(&dir);
}

/// An IPv6 print server is not a `host:port` split: a bare address has more than one ':', and the
/// brackets of `[address]:port` must not end up inside the host, where no resolver can read them.
#[test]
fn an_ipv6_print_server_is_parsed_as_a_host_not_as_host_port() {
    assert_eq!(Endpoint::parse("::1"), Endpoint::Tcp("::1".into(), 631));
    assert_eq!(Endpoint::parse("fe80::1"), Endpoint::Tcp("fe80::1".into(), 631));
    assert_eq!(Endpoint::parse("2001:db8::1"), Endpoint::Tcp("2001:db8::1".into(), 631));
    assert_eq!(Endpoint::parse("[::1]"), Endpoint::Tcp("::1".into(), 631));
    assert_eq!(Endpoint::parse("[::1]:631"), Endpoint::Tcp("::1".into(), 631));
    assert_eq!(Endpoint::parse("[fe80::1]:6310"), Endpoint::Tcp("fe80::1".into(), 6310));
    // A zone id carries no ':', so a port can still follow it without brackets.
    assert_eq!(Endpoint::parse("fe80::1%eth0:631"), Endpoint::Tcp("fe80::1%eth0".into(), 631));
    // What already worked is unchanged.
    assert_eq!(Endpoint::parse("cups.example"), Endpoint::Tcp("cups.example".into(), 631));
    assert_eq!(Endpoint::parse("cups.example:6310"), Endpoint::Tcp("cups.example".into(), 6310));
    assert_eq!(Endpoint::parse("127.0.0.1:6310"), Endpoint::Tcp("127.0.0.1".into(), 6310));
    assert_eq!(Endpoint::parse("/var/run/cups/cups.sock"), Endpoint::Unix("/var/run/cups/cups.sock".into()));
    // A mistyped bracketed endpoint is not quietly the default one: the whole string stays the host,
    // so nothing can resolve it and detect() reports the endpoint unreachable.
    assert_eq!(Endpoint::parse("[::1]:abc"), Endpoint::Tcp("[::1]:abc".into(), 631));
    assert_eq!(Endpoint::parse("[::1]:70000"), Endpoint::Tcp("[::1]:70000".into(), 631));
    assert_eq!(Endpoint::parse("[::1]junk"), Endpoint::Tcp("[::1]junk".into(), 631));
    assert!(!Endpoint::parse("[::1]:abc").reachable());
}

/// The authority a request carries (the `Host:` header and the `ipp://` printer-uri) has to bracket an
/// IPv6 host, or its last colon reads as the port separator and CUPS sees a nonsense host.
#[test]
fn an_ipv6_endpoint_writes_its_authority_in_brackets() {
    assert_eq!(Endpoint::parse("::1").host(), "[::1]:631");
    assert_eq!(Endpoint::parse("[::1]:631").host(), "[::1]:631");
    assert_eq!(Endpoint::parse("[fe80::1]:6310").host(), "[fe80::1]:6310");
    assert_eq!(Endpoint::parse("cups.example").host(), "cups.example:631");
    assert_eq!(Endpoint::parse("cups.example:6310").host(), "cups.example:6310");
    assert_eq!(Endpoint::parse("127.0.0.1:6310").host(), "127.0.0.1:6310");
    // A scoped address keeps its zone id, but the authority writes it "%25"-escaped: a bare "%" is
    // not valid inside an authority, and the escaped form is what CUPS and curl-style ipp:// URIs
    // use. The host itself stays raw, because that is what the resolver gets.
    assert_eq!(Endpoint::parse("fe80::1%3:631").host(), "[fe80::1%253]:631");
    assert_eq!(Endpoint::parse("[fe80::1%3]:6310").host(), "[fe80::1%253]:6310");
    assert_eq!(Endpoint::parse("fe80::1%eth0:631").host(), "[fe80::1%25eth0]:631");
    assert_eq!(Endpoint::parse("fe80::1%3:631"), Endpoint::Tcp("fe80::1%3".into(), 631));
    // What the user is shown is written the same way, with the zone exactly as it was typed.
    assert_eq!(Endpoint::parse("::1").to_string(), "[::1]:631");
    assert_eq!(Endpoint::parse("cups.example:6310").to_string(), "cups.example:6310");
    assert_eq!(Endpoint::parse("fe80::1%3:631").to_string(), "[fe80::1%3]:631");
}

#[test]
fn requests_start_with_the_mandatory_attributes() {
    let r = Request::new(OP_PRINT_JOB, 9).end();
    let (op, attrs, doc) = parse_request(&r).unwrap();
    assert_eq!(op, OP_PRINT_JOB);
    assert_eq!(Reply::parse(&r).unwrap().request_id, 9);
    assert_eq!(attrs[0].name, "attributes-charset");
    assert_eq!(attrs[1].name, "attributes-natural-language");
    assert!(doc.is_empty());
}

/// A server that keeps the connection open after its reply (a printer reached directly, a proxy)
/// must not hold the client until the read times out: the reply ends at Content-Length, or at
/// the last chunk.
#[test]
fn replies_end_at_content_length_or_the_last_chunk_without_waiting_for_close() {
    for chunked in [false, true] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let one = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
        let none = ipp(0x0406, &[ok_header()]);
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { return };
                let mut reader = BufReader::new(stream);
                let (mut line, mut len) = (String::new(), 0usize);
                loop {
                    line.clear();
                    if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap_or(0);
                    }
                }
                if len == 0 {
                    continue;
                }
                let mut body = vec![0u8; len];
                if reader.read_exact(&mut body).is_err() {
                    continue;
                }
                let op = parse_request(&body).map(|r| r.0).unwrap_or(0);
                let reply = if op == OP_CUPS_GET_PRINTERS { one.clone() } else { none.clone() };
                let mut out = reader.into_inner();
                let mut resp = Vec::new();
                if chunked {
                    resp.extend_from_slice(b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n");
                    for chunk in reply.chunks(5) {
                        resp.extend_from_slice(format!("{:x}\r\n", chunk.len()).as_bytes());
                        resp.extend_from_slice(chunk);
                        resp.extend_from_slice(b"\r\n");
                    }
                    resp.extend_from_slice(b"0\r\n\r\n");
                } else {
                    resp.extend_from_slice(
                        format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", reply.len()).as_bytes(),
                    );
                    resp.extend_from_slice(&reply);
                }
                if out.write_all(&resp).and_then(|()| out.flush()).is_err() {
                    continue;
                }
                // Keep this connection open for a while, on its own thread, and go on serving.
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs(10));
                    drop(out);
                });
            }
        });
        let ep = Endpoint::Tcp("127.0.0.1".into(), port);
        let started = std::time::Instant::now();
        let list = printers(&ep).unwrap();
        assert_eq!(list.len(), 1, "chunked {chunked}");
        assert!(started.elapsed() < Duration::from_secs(5), "waited for the server to close (chunked {chunked}): {:?}", started.elapsed());
    }
}

/// A chunked reply whose payload contains the bytes of a last chunk (`\r\n0\r\n\r\n`, here inside
/// a printer name) and arrives split right after them: the reader must wait for the real last
/// chunk instead of returning a cut reply.
#[test]
fn a_last_chunk_pattern_inside_a_value_does_not_end_the_reply() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Of\r\n0\r\n\r\nfice"])])]);
    let cut = reply.windows(7).position(|w| w == b"\r\n0\r\n\r\n").unwrap() + 7;
    std::thread::spawn(move || {
        let (stream, _) = listener.accept().unwrap();
        let mut reader = BufReader::new(stream);
        let (mut line, mut len) = (String::new(), 0usize);
        loop {
            line.clear();
            if reader.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                break;
            }
            if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                len = v.trim().parse().unwrap_or(0);
            }
        }
        let mut body = vec![0u8; len];
        reader.read_exact(&mut body).unwrap();
        let mut out = reader.into_inner();
        let (first, second) = reply.split_at(cut);
        // Part one ends exactly on the pattern, inside the first chunk's payload.
        let mut a = b"HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
        a.extend_from_slice(format!("{:x}\r\n", first.len()).as_bytes());
        a.extend_from_slice(first);
        out.write_all(&a).and_then(|()| out.flush()).unwrap();
        std::thread::sleep(Duration::from_millis(400));
        let mut b = b"\r\n".to_vec();
        b.extend_from_slice(format!("{:x}\r\n", second.len()).as_bytes());
        b.extend_from_slice(second);
        b.extend_from_slice(b"\r\n0\r\n\r\n");
        out.write_all(&b).and_then(|()| out.flush()).unwrap();
    });
    let ep = Endpoint::Tcp("127.0.0.1".into(), port);
    let list = printers(&ep).unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].name, "Of\r\n0\r\n\r\nfice");
    // The boundary walk itself: complete only at a zero-size chunk, never on payload bytes.
    assert!(!complete(b"5\r\nab\r\n0"));
    assert!(!complete(b"8\r\n\r\n0\r\n\r\nx\r\n"));
    assert!(complete(b"8\r\n\r\n0\r\n\r\nx\r\n0\r\n\r\n"));
    assert!(complete(b"zz\r\n") && dechunk(b"zz\r\n").is_err());
}

/// [`chunked_complete`] on a whole body at once.
fn complete(body: &[u8]) -> bool {
    chunked_complete(body, &mut 0)
}

/// Fed a byte at a time, as a server dripping the reply would, the walk resumes where it stopped
/// and agrees with the one-shot answer at every length.
#[test]
fn the_chunk_walk_resumes_instead_of_starting_over() {
    let body = b"3\r\nabc\r\n10\r\n0123456789abcdef\r\n0\r\n\r\n";
    let mut at = 0;
    for len in 0..=body.len() {
        let prefix = &body[..len];
        assert_eq!(chunked_complete(prefix, &mut at), complete(prefix), "{len}");
        assert!(at <= len);
    }
    assert!(at > 0, "it moved past the chunks already walked");
}

/// A chunk size that cannot be satisfied (`usize::MAX`, `usize::MAX - 1`, anything past the
/// reply cap) is a protocol error, through the real reader, never an overflow panic.
#[test]
fn an_impossible_chunk_size_is_a_protocol_error_not_a_panic() {
    for size_line in ["ffffffffffffffff", "fffffffffffffffe", "1000001"] {
        assert!(complete(format!("{size_line}\r\n").as_bytes()));
        assert!(dechunk(format!("{size_line}\r\nab\r\n").as_bytes()).is_err());
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let line = size_line.to_string();
        std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (mut l, mut len) = (String::new(), 0usize);
            loop {
                l.clear();
                if reader.read_line(&mut l).unwrap_or(0) == 0 || l == "\r\n" {
                    break;
                }
                if let Some(v) = l.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = v.trim().parse().unwrap_or(0);
                }
            }
            let mut body = vec![0u8; len];
            reader.read_exact(&mut body).unwrap();
            let mut out = reader.into_inner();
            let resp = format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nTransfer-Encoding: chunked\r\n\r\n{line}\r\n");
            let _ = out.write_all(resp.as_bytes()).and_then(|()| out.flush());
        });
        let ep = Endpoint::Tcp("127.0.0.1".into(), port);
        match printers(&ep) {
            Err(Error::Protocol(_)) => {}
            other => panic!("{size_line}: expected a protocol error, got {other:?}"),
        }
    }
}

#[test]
fn printer_names_are_encoded_as_one_request_target_segment() {
    assert_eq!(queue_path("office printer/1", false).unwrap(), "/printers/office%20printer%2F1");
    assert!(queue_path("printer\r\nX-Injected: yes", false).is_err());
}

/// A reply arriving a byte at a time, its blank line split across reads, is still found: the
/// header search resumes where it stopped instead of rescanning everything read so far.
#[test]
fn a_header_trickled_a_byte_at_a_time_is_found() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        s.set_nodelay(true).unwrap();
        for b in b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\nokEXTRA" {
            s.write_all(&[*b]).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    });
    let mut stream = Stream::Tcp(TcpStream::connect(addr).unwrap());
    let raw = read_response(&mut stream).unwrap();
    assert!(raw.ends_with(b"\r\n\r\nok"), "{:?}", String::from_utf8_lossy(&raw));
    server.join().unwrap();
}

#[test]
fn chunk_data_must_end_with_crlf() {
    for body in [b"1\r\naXX0\r\n\r\n", b"1\r\na\n\r0\r\n\r\n"] {
        assert!(matches!(dechunk(body), Err(Error::Protocol(_))), "accepted malformed chunk: {body:?}");
    }
}

#[test]
fn malformed_chunk_ends_complete_the_reply() {
    for body in [b"1\r\naXX", b"1\r\na\n\r", b"1\r\na\rX"] {
        assert!(chunked_complete(body, &mut 0), "malformed separator must reach dechunk: {body:?}");
    }
    for body in [b"1\r\na".as_slice(), b"1\r\na\r"] {
        assert!(!chunked_complete(body, &mut 0), "partial separator still needs bytes: {body:?}");
    }
    let body = b"1\r\na\r\n2\r\nbc\r\n0\r\n\r\n";
    let mut resume = 0;
    for end in 0..=body.len() {
        assert_eq!(chunked_complete(&body[..end], &mut resume), chunked_complete(&body[..end], &mut 0), "prefix {end}");
    }
}

#[test]
fn a_malformed_chunk_reply_finishes_without_waiting_for_close() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (release, held) = mpsc::channel();
    let (ready, written) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\naXX").unwrap();
        ready.send(()).unwrap();
        held.recv().unwrap();
    });
    let socket = TcpStream::connect(addr).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    written.recv().unwrap();
    let result = read_response(&mut Stream::Tcp(socket));
    release.send(()).unwrap();
    server.join().unwrap();
    let raw = result.expect("a malformed chunk must finish while the server keeps the socket open");
    let (_, _, body) = http_response(&raw).unwrap();
    assert!(matches!(dechunk(body), Err(Error::Protocol(message)) if message == "invalid chunk end"));
}

#[test]
fn malformed_http_status_lines_cannot_report_a_default_printer() {
    for (status_line, valid) in [
        ("garbage 200 OK", false),
        ("HTTP/1.1 +200 OK", false),
        ("HTTP/1.1 0200 OK", false),
        ("http/1.1 200 OK", false),
        ("HTTP/1.a 200 OK", false),
        ("HTTP/1.10 200 OK", false),
        ("HTTP/1.0 200 OK", true),
        ("HTTP/1.1 200 ", true),
        ("HTTP/1.9 200 OK", true),
    ] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (mut line, mut len) = (String::new(), 0usize);
            loop {
                line.clear();
                if reader.read_line(&mut line).unwrap() == 0 || line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; len]).unwrap();
            let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"P"])])]);
            let mut reply = format!("{status_line}\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
            reply.extend_from_slice(&body);
            let _ = reader.into_inner().write_all(&reply);
        });
        let result = default_printer(&Endpoint::Tcp("127.0.0.1".into(), port));
        server.join().unwrap();
        if valid {
            assert_eq!(result, Ok(Some("P".into())), "{status_line}");
        } else {
            assert!(matches!(&result, Err(Error::Protocol(message)) if message.starts_with("bad status line")), "{status_line}: {result:?}");
        }
    }
}

#[test]
fn print_job_requires_a_positive_job_id() {
    for id in [i32::MIN, -1, 0] {
        let (ep, _rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&id.to_be_bytes()])])]), false, 200);
        let result =
            print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" });
        assert!(matches!(result, Err(Error::Protocol(_))), "job-id {id}: {result:?}");
    }
    for id in [1i32, i32::MAX] {
        let (ep, _rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(TAG_INTEGER, "job-id", &[&id.to_be_bytes()])])]), false, 200);
        let result =
            print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" });
        assert_eq!(result.unwrap(), id as u32);
    }
}

#[test]
fn a_content_length_body_cut_short_is_an_error() {
    let valid = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    for body in [Vec::new(), b"o".to_vec(), valid] {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (mut line, mut len) = (String::new(), 0usize);
            loop {
                line.clear();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    len = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0u8; len]).unwrap();
            let mut stream = reader.into_inner();
            stream
                .write_all(format!("HTTP/1.1 200 OK\r\nContent-Type: application/ipp\r\nContent-Length: {}\r\n\r\n", body.len() + 1).as_bytes())
                .unwrap();
            stream.write_all(&body).unwrap();
        });
        let result = printers(&Endpoint::Tcp(addr.ip().to_string(), addr.port()));
        server.join().unwrap();
        assert!(matches!(result, Err(Error::Protocol(_))), "accepted incomplete HTTP body: {result:?}");
    }
}

#[test]
fn a_failed_address_does_not_hide_a_reachable_print_server() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let reachable = listener.local_addr().unwrap();
    let refused: SocketAddr = "[::1]:631".parse().unwrap();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let result = open_tcp("printserver", 631, [refused, reachable], move |addr, timeout| {
        record.lock().unwrap().start(addr, timeout);
        let result = if addr == refused {
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "first address refused"))
        } else {
            TcpStream::connect_timeout(&addr, timeout)
        };
        record.lock().unwrap().finish();
        result
    });
    assert!(result.is_ok(), "the second address was not tried");
    let trace = trace.lock().unwrap();
    trace.assert_addresses(&[refused, reachable]);
    assert!(trace.calls.iter().all(|(_, timeout)| !timeout.is_zero() && *timeout <= CONNECT));
    listener.accept().unwrap();
}

#[test]
fn failed_addresses_keep_the_last_error_and_empty_resolution_stays_clear() {
    let first: SocketAddr = "[::1]:631".parse().unwrap();
    let last: SocketAddr = "127.0.0.1:631".parse().unwrap();
    let (release_first, wait_first) = mpsc::channel();
    let wait_first = Mutex::new(wait_first);
    let completed = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&completed);
    let result = open_tcp("printserver", 631, [first, last], move |addr, timeout| {
        if addr == first {
            wait_first.lock().unwrap().recv_timeout(timeout).unwrap();
            record.lock().unwrap().push(addr);
        } else {
            record.lock().unwrap().push(addr);
            release_first.send(()).unwrap();
        }
        Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, if addr == last { "last refusal" } else { "first refusal" }))
    });
    assert!(matches!(result, Err(Error::Unreachable(message)) if message == "printserver:631: last refusal"));
    assert_eq!(*completed.lock().unwrap(), [last, first], "completion order must not choose the final error");
    let result = open_tcp("printserver", 631, [], |_, _| panic!("an empty resolution must not connect"));
    assert!(matches!(result, Err(Error::Unreachable(message)) if message == "printserver:631: no address"));
}

#[test]
fn a_shared_deadline_does_not_report_an_earlier_connection_refusal() {
    let refused: SocketAddr = "192.0.2.1:631".parse().unwrap();
    let stalled: SocketAddr = "192.0.2.2:631".parse().unwrap();
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let (finished, done) = mpsc::channel();
    let result = open_tcp("printserver", 631, [refused, stalled], move |addr, _timeout| {
        if addr == refused {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "earlier address refused"));
        }
        // Keep the later attempt pending until the coordinator has returned its deadline error.
        let _ = held.lock().unwrap().recv_timeout(CONNECT + Duration::from_secs(1));
        let _ = finished.send(());
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "later address did not answer"))
    });
    let _ = release.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(result.err(), Some(Error::Unreachable("printserver:631: connection timed out".into())));
}

#[test]
fn a_shared_deadline_keeps_a_received_error_from_the_last_attempt() {
    let stalled: SocketAddr = "192.0.2.1:631".parse().unwrap();
    let refused: SocketAddr = "192.0.2.2:631".parse().unwrap();
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let (finished, done) = mpsc::channel();
    let result = open_tcp("printserver", 631, [stalled, refused], move |addr, _timeout| {
        if addr == refused {
            return Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "last address refused"));
        }
        let _ = held.lock().unwrap().recv_timeout(CONNECT + Duration::from_secs(1));
        let _ = finished.send(());
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "earlier address did not answer"))
    });
    let _ = release.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert_eq!(result.err(), Some(Error::Unreachable("printserver:631: last address refused".into())));
}

#[test]
fn a_slow_address_leaves_time_for_a_reachable_print_server() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let reachable = listener.local_addr().unwrap();
    let blackholed: SocketAddr = "[::1]:631".parse().unwrap();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let (finished, done) = mpsc::channel();
    let started = std::time::Instant::now();
    let result = open_tcp("printserver", 631, [blackholed, reachable], move |addr, timeout| {
        record.lock().unwrap().start(addr, timeout);
        let result = if addr == blackholed {
            let _ = held.lock().unwrap().recv_timeout(timeout);
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "first address did not answer"))
        } else {
            TcpStream::connect_timeout(&addr, timeout)
        };
        record.lock().unwrap().finish();
        if addr == blackholed {
            let _ = finished.send(());
        }
        result
    });
    let elapsed = started.elapsed();
    let _ = release.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result.is_ok(), "a timed-out first address hid a reachable second address");
    assert!(elapsed < Duration::from_secs(2), "the winner waited for a blocked losing worker: {elapsed:?}");
    let trace = trace.lock().unwrap();
    trace.assert_addresses(&[blackholed, reachable]);
    assert!(trace.calls.iter().all(|(_, timeout)| !timeout.is_zero() && *timeout <= CONNECT));
    assert_eq!(trace.active, 0);
    listener.accept().unwrap();
}

fn fake_probe_responses(responses: Vec<(Vec<u8>, bool)>) -> (Endpoint, std::thread::JoinHandle<()>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || {
        for (body, truncated) in responses {
            let (stream, _) = listener.accept().unwrap();
            let mut reader = BufReader::new(stream);
            let (mut line, mut length) = (String::new(), 0usize);
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n", body.len() + usize::from(truncated)).into_bytes();
            response.extend_from_slice(&body);
            reader.into_inner().write_all(&response).unwrap();
        }
    });
    (Endpoint::Tcp("127.0.0.1".into(), port), server)
}

#[test]
fn a_truncated_optional_class_reply_is_not_hidden() {
    let printers_reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes_reply = ipp(0, &[ok_header()]);
    let (endpoint, server) = fake_probe_responses(vec![(printers_reply, false), (classes_reply, true)]);
    let result = printers(&endpoint);
    server.join().unwrap();
    assert!(matches!(result, Err(Error::Protocol(_))), "accepted truncated optional classes: {result:?}");
}

#[test]
fn a_truncated_optional_default_reply_is_not_hidden() {
    let printers_reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes_reply = ipp(0, &[ok_header()]);
    let default_reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let (endpoint, server) = fake_probe_responses(vec![(printers_reply, false), (classes_reply, false), (default_reply, true)]);
    let result = printers(&endpoint);
    server.join().unwrap();
    assert!(matches!(result, Err(Error::Protocol(_))), "accepted truncated optional default: {result:?}");
}

#[test]
fn optional_unsupported_operations_still_allow_listing_printers() {
    let printers_reply = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let unsupported_classes = ipp(0x0501, &[ok_header()]);
    let no_default = ipp(0x0406, &[ok_header()]);
    let (endpoint, server) = fake_probe_responses(vec![(printers_reply, false), (unsupported_classes, false), (no_default, false)]);
    let result = printers(&endpoint);
    server.join().unwrap();
    let printers = result.unwrap();
    assert_eq!(printers.len(), 1);
    assert_eq!(printers[0].name, "Office");
    assert!(!printers[0].is_default);
}

fn http_probe_reply(status: u16, body: &[u8]) -> Vec<u8> {
    let mut response = format!("HTTP/1.1 {status} Reply\r\nContent-Length: {}\r\n\r\n", body.len()).into_bytes();
    response.extend_from_slice(body);
    response
}

fn fake_raw_probe_responses(responses: Vec<Vec<u8>>) -> (Endpoint, mpsc::Sender<()>, std::thread::JoinHandle<usize>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let port = listener.local_addr().unwrap().port();
    let (done, caller_done) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let mut requests = 0;
        for response in responses {
            let stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        if caller_done.try_recv().is_ok() {
                            return requests;
                        }
                        std::thread::sleep(Duration::from_millis(2));
                    }
                    Err(error) => panic!("accepting fake probe: {error}"),
                }
            };
            stream.set_nonblocking(false).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut reader = BufReader::new(stream);
            let (mut line, mut length) = (String::new(), 0usize);
            loop {
                line.clear();
                assert_ne!(reader.read_line(&mut line).unwrap(), 0);
                if line == "\r\n" {
                    break;
                }
                if let Some(value) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                    length = value.trim().parse().unwrap();
                }
            }
            reader.read_exact(&mut vec![0; length]).unwrap();
            reader.into_inner().write_all(&response).unwrap();
            requests += 1;
        }
        requests
    });
    (Endpoint::Tcp("127.0.0.1".into(), port), done, server)
}

#[test]
fn optional_http_failures_still_allow_listing_printers() {
    let primary = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes = ipp(0, &[ok_header()]);
    let default = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let mut failures = Vec::new();
    for probe in [1, 2] {
        for status in [401, 403, 500] {
            let mut responses = vec![http_probe_reply(200, &primary), http_probe_reply(200, &classes), http_probe_reply(200, &default)];
            responses[probe] = http_probe_reply(status, b"denied");
            let (endpoint, done, server) = fake_raw_probe_responses(responses);
            let result = printers(&endpoint);
            let _ = done.send(());
            let requests = server.join().unwrap();
            match result {
                Ok(printers) => {
                    assert_eq!(requests, 3);
                    assert_eq!(printers.len(), 1);
                    assert_eq!(printers[0].name, "Office");
                    assert_eq!(printers[0].is_default, probe == 1);
                }
                Err(error) => failures.push(format!("probe {probe}, HTTP {status}: {error}")),
            }
        }
    }
    assert!(failures.is_empty(), "optional HTTP failures hid available printers: {failures:?}");
}

#[test]
fn optional_empty_connection_closes_still_allow_listing_printers() {
    let primary = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes = ipp(0, &[ok_header()]);
    let default = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let mut failures = Vec::new();
    for probe in [1, 2] {
        let mut responses = vec![http_probe_reply(200, &primary), http_probe_reply(200, &classes), http_probe_reply(200, &default)];
        responses[probe].clear();
        let (endpoint, done, server) = fake_raw_probe_responses(responses);
        let result = printers(&endpoint);
        let _ = done.send(());
        let requests = server.join().unwrap();
        match result {
            Ok(printers) => {
                assert_eq!(requests, 3);
                assert_eq!(printers.len(), 1);
                assert_eq!(printers[0].name, "Office");
                assert_eq!(printers[0].is_default, probe == 1);
            }
            Err(error) => failures.push(format!("probe {probe}: {error}")),
        }
    }
    assert!(failures.is_empty(), "optional connection closes hid available printers: {failures:?}");
}

#[test]
fn standalone_http_and_transport_failures_keep_their_public_error_messages() {
    for (status, expected) in
        [(401, "the print server asks for authentication (HTTP 401)"), (403, "the print server forbids this request (HTTP 403)"), (500, "HTTP 500")]
    {
        let (endpoint, done, server) = fake_raw_probe_responses(vec![http_probe_reply(status, b"denied")]);
        let result = default_printer(&endpoint);
        let _ = done.send(());
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(result, Err(Error::Protocol(expected.into())));
    }
    let (endpoint, done, server) = fake_raw_probe_responses(vec![Vec::new()]);
    let result = default_printer(&endpoint);
    let _ = done.send(());
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(result, Err(Error::Protocol("no HTTP header".into())));
}

#[test]
fn invalid_content_lengths_are_not_accepted_as_eof_delimited_bodies() {
    let primary = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes = ipp(0, &[ok_header()]);
    let default = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let bodies = [primary, classes, default];
    let mut failures = Vec::new();
    for probe in [0, 1, 2] {
        for invalid in [String::new(), "garbage".into(), "18446744073709551616".into(), format!("+{}", bodies[probe].len())] {
            let mut responses: Vec<_> = bodies.iter().map(|body| http_probe_reply(200, body)).collect();
            responses[probe] = format!("HTTP/1.1 200 OK\r\nContent-Length: {invalid}\r\n\r\n").into_bytes();
            responses[probe].extend_from_slice(&bodies[probe]);
            let (endpoint, done, server) = fake_raw_probe_responses(responses);
            let result = printers(&endpoint);
            let _ = done.send(());
            server.join().unwrap();
            if !matches!(result, Err(Error::Protocol(_))) {
                failures.push(format!("probe {probe}, length {invalid:?}: {result:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "accepted invalid present Content-Length: {failures:?}");
}

#[test]
fn absent_content_length_and_chunked_precedence_remain_supported() {
    let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let mut eof_delimited = b"HTTP/1.1 200 OK\r\n\r\n".to_vec();
    eof_delimited.extend_from_slice(&body);
    let mut chunked =
        b"HTTP/1.1 200 OK\r\nContent-Length: invalid\r\nContent-Length: 18446744073709551616\r\nTransfer-Encoding: chunked\r\n\r\n".to_vec();
    chunked.extend_from_slice(format!("{:x}\r\n", body.len()).as_bytes());
    chunked.extend_from_slice(&body);
    chunked.extend_from_slice(b"\r\n0\r\n\r\n");
    for response in [eof_delimited, chunked] {
        let (endpoint, done, server) = fake_raw_probe_responses(vec![response]);
        let result = default_printer(&endpoint);
        let _ = done.send(());
        assert_eq!(server.join().unwrap(), 1);
        assert_eq!(result, Ok(Some("Office".into())));
    }
}

fn language_value(language: &str, text: &[u8]) -> Vec<u8> {
    let mut value = (language.len() as u16).to_be_bytes().to_vec();
    value.extend_from_slice(language.as_bytes());
    value.extend_from_slice(&(text.len() as u16).to_be_bytes());
    value.extend_from_slice(text);
    value
}

#[test]
fn language_tagged_text_uses_byte_lengths_and_keeps_plain_text_lossy() {
    for tag in [0x35, 0x36] {
        let value = language_value("de", "Büro 東京".as_bytes());
        let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![value, b"ignored".to_vec()] };
        assert_eq!(attribute.text(), "Büro 東京");
        let attribute = Attribute { values: vec![language_value("en", b"bad\xfftext")], ..attribute };
        assert_eq!(attribute.text(), "bad\u{fffd}text");
    }
    for tag in [TAG_TEXT, TAG_NAME] {
        let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![b"plain\xfftext".to_vec()] };
        assert_eq!(attribute.text(), "plain\u{fffd}text");
        assert_eq!(Attribute { values: Vec::new(), ..attribute }.text(), "");
    }
}

#[test]
fn malformed_language_lengths_do_not_become_printer_text() {
    let malformed: &[&[u8]] = &[
        b"",
        b"\0",
        b"\xff\xff",
        b"\x80\0",
        b"\0\x02e",
        b"\0\x02en",
        b"\0\x02en\0",
        b"\0\x02en\xff\xff",
        b"\0\x02en\x80\0",
        b"\0\x02en\0\x02x",
        b"\0\x02en\0\x01xy",
        b"\x7f\xffen\0\0",
        b"\0\x02en\x7f\xffx",
    ];
    for tag in [0x35, 0x36] {
        for value in malformed {
            let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![value.to_vec()] };
            assert_eq!(attribute.text(), "", "tag {tag:#x}: {value:?}");
        }
        let attribute = Attribute { group: TAG_PRINTER_GROUP, tag, name: "text".into(), values: vec![language_value("en", b"")] };
        assert_eq!(attribute.text(), "");
    }
}

#[test]
fn duplicate_content_lengths_must_be_valid_and_identical() {
    let primary = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes = ipp(0, &[ok_header()]);
    let default = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let bodies = [primary, classes, default];
    let mut failures = Vec::new();
    for probe in [0, 1, 2] {
        let length = bodies[probe].len();
        for later in [String::new(), "garbage".into(), "18446744073709551616".into(), format!("+{length}"), (length + 1).to_string()] {
            let mut responses: Vec<_> = bodies.iter().map(|body| http_probe_reply(200, body)).collect();
            responses[probe] = format!("HTTP/1.1 200 OK\r\nContent-Length: {length}\r\nContent-Length: {later}\r\n\r\n").into_bytes();
            responses[probe].extend_from_slice(&bodies[probe]);
            let (endpoint, done, server) = fake_raw_probe_responses(responses);
            let result = printers(&endpoint);
            let _ = done.send(());
            server.join().unwrap();
            if !matches!(result, Err(Error::Protocol(_))) {
                failures.push(format!("probe {probe}, later length {later:?}: {result:?}"));
            }
        }
    }
    assert!(failures.is_empty(), "accepted invalid or conflicting later Content-Length: {failures:?}");
}

#[test]
fn identical_duplicate_content_lengths_remain_supported() {
    let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let mut response = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nContent-Length: {}\r\n\r\n", body.len(), body.len()).into_bytes();
    response.extend_from_slice(&body);
    let (endpoint, done, server) = fake_raw_probe_responses(vec![response]);
    let result = default_printer(&endpoint);
    let _ = done.send(());
    assert_eq!(server.join().unwrap(), 1);
    assert_eq!(result, Ok(Some("Office".into())));
}

/// A wrong first CRLF byte is already malformed even while the server keeps the socket open.
#[test]
fn a_wrong_first_chunk_separator_byte_finishes_without_a_second_byte() {
    for body in [b"1\r\nA".as_slice(), b"1\r\nA\r"] {
        assert!(!chunked_complete(body, &mut 0), "a valid separator prefix needs more bytes: {body:?}");
    }
    for body in [b"1\r\nAX".as_slice(), b"1\r\nA\n", b"1\r\nA\0"] {
        assert!(chunked_complete(body, &mut 0), "a wrong first separator byte cannot recover: {body:?}");
        assert!(matches!(dechunk(body), Err(Error::Protocol(message)) if message == "invalid chunk end"));
    }
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let (release, held) = mpsc::channel();
    let (ready, written) = mpsc::channel();
    let server = std::thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n1\r\nAX").unwrap();
        ready.send(()).unwrap();
        held.recv().unwrap();
    });
    let socket = TcpStream::connect(addr).unwrap();
    socket.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    written.recv().unwrap();
    let result = read_response(&mut Stream::Tcp(socket));
    release.send(()).unwrap();
    server.join().unwrap();
    let raw = result.expect("a wrong first chunk terminator byte must not wait for the server to close");
    let (_, _, body) = http_response(&raw).unwrap();
    assert!(matches!(dechunk(body), Err(Error::Protocol(message)) if message == "invalid chunk end"));
}

#[test]
fn print_job_refuses_an_enum_instead_of_an_integer_job_id() {
    for tag in [TAG_ENUM, TAG_NAME, TAG_TEXT] {
        let (ep, _rx) = fake(ipp(0, &[ok_header(), (TAG_JOB_GROUP, &[(tag, "job-id", &[&42i32.to_be_bytes()])])]), false, 200);
        let result =
            print_job(&ep, &Job { printer: "P", is_class: false, title: "t", user: "u", copies: 1, document: b"x", format: "application/pdf" });
        assert!(matches!(result, Err(Error::Protocol(_))), "job-id tag {tag:#x}: {result:?}");
    }
}

#[test]
fn http_status_separators_use_http_octets_not_unicode_whitespace() {
    for separator in [" ", "\t", "\u{0b}", "\u{0c}", "\r", " \t\r "] {
        let raw = format!("{separator}HTTP/1.1{separator}200{separator}Ready\r\nContent-Length: 0\r\n\r\n");
        assert_eq!(http_response(raw.as_bytes()).unwrap().0, 200, "ASCII separator {separator:?}");
    }
    for separator in ["\u{00a0}", "\u{1680}", "\u{2003}", "\u{2028}", "\u{202f}", "\u{3000}", "\n"] {
        for status in [format!("HTTP/1.1{separator}200 Ready"), format!("HTTP/1.1 200{separator}Ready"), format!("{separator}HTTP/1.1 200 Ready")] {
            let raw = format!("{status}\r\nContent-Length: 0\r\n\r\n");
            assert!(matches!(http_response(raw.as_bytes()), Err(Error::Protocol(_))), "accepted non-HTTP separator: {status:?}");
        }
    }
    assert_eq!(http_response("HTTP/1.1 200 Prêt\r\nContent-Length: 0\r\n\r\n".as_bytes()).unwrap().0, 200);
}

#[test]
fn content_length_lists_normalize_only_identical_decimal_members() {
    let primary = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let classes = ipp(0, &[ok_header()]);
    let default = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(TAG_NAME, "printer-name", &[b"Office"])])]);
    let bodies = [primary, classes, default];
    for probe in [0, 1, 2] {
        let length = bodies[probe].len();
        let lists = [
            (format!("{length},{length}"), true),
            (format!("{length}, \t{length}, 0{length}"), true),
            (format!("{length},{}", length + 1), false),
            (format!("{length},,{length}"), false),
            (format!(",{length}"), false),
            (format!("{length},"), false),
            (format!("{length},+{length}"), false),
            (format!("{length},garbage"), false),
            (format!("{length},18446744073709551616"), false),
            (format!("{length},\u{00a0}{length},{length}"), false),
        ];
        for (list, valid) in lists {
            let mut responses: Vec<_> = bodies.iter().map(|body| http_probe_reply(200, body)).collect();
            responses[probe] = format!("HTTP/1.1 200 OK\r\nContent-Length: {list}\r\nContent-Length: {length}\r\n\r\n").into_bytes();
            responses[probe].extend_from_slice(&bodies[probe]);
            let (endpoint, done, server) = fake_raw_probe_responses(responses);
            let result = printers(&endpoint);
            let _ = done.send(());
            server.join().unwrap();
            if valid {
                let printers = result.unwrap_or_else(|error| panic!("probe {probe}, valid identical list {list:?}: {error}"));
                assert_eq!(printers.len(), 1);
                assert_eq!(printers[0].name, "Office");
                assert!(printers[0].is_default);
            } else {
                assert!(matches!(result, Err(Error::Protocol(_))), "probe {probe}, invalid list {list:?}: {result:?}");
            }
        }
    }
}

#[test]
fn a_slow_reachable_first_address_keeps_its_full_connection_window() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let first = listener.local_addr().unwrap();
    let refused: SocketAddr = "192.0.2.7:631".parse().unwrap();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let result = open_tcp("printserver", 631, [first, refused], move |addr, timeout| {
        record.lock().unwrap().start(addr, timeout);
        let result = if addr == first {
            let needed = Duration::from_secs(2);
            std::thread::sleep(timeout.min(needed));
            if timeout < needed {
                Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "the first address needs two seconds"))
            } else {
                TcpStream::connect_timeout(&addr, timeout.saturating_sub(needed).max(Duration::from_nanos(1)))
            }
        } else {
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "second address refused"))
        };
        record.lock().unwrap().finish();
        result
    });
    assert!(result.is_ok(), "the viable first address lost its original timeout window");
    let trace = trace.lock().unwrap();
    trace.assert_addresses(&[first, refused]);
    assert_eq!(trace.active, 0);
    listener.accept().unwrap();
}

#[test]
fn failed_attempts_immediately_advance_to_the_next_resolved_address() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let reachable = listener.local_addr().unwrap();
    let blocked: SocketAddr = "192.0.2.1:631".parse().unwrap();
    let refused: SocketAddr = "192.0.2.2:631".parse().unwrap();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let (first_started, wait_started) = mpsc::channel();
    let wait_started = Mutex::new(wait_started);
    let (finished, done) = mpsc::channel();
    let result = open_tcp("printserver", 631, [blocked, refused, reachable], move |addr, timeout| {
        if addr == refused {
            wait_started.lock().unwrap().recv_timeout(timeout).unwrap();
        }
        record.lock().unwrap().start(addr, timeout);
        let result = if addr == blocked {
            first_started.send(()).unwrap();
            let _ = held.lock().unwrap().recv_timeout(timeout);
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "blocked first address"))
        } else if addr == refused {
            Err(std::io::Error::new(std::io::ErrorKind::ConnectionRefused, "refused second address"))
        } else {
            TcpStream::connect_timeout(&addr, timeout)
        };
        record.lock().unwrap().finish();
        if addr == blocked {
            let _ = finished.send(());
        }
        result
    });
    let _ = release.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result.is_ok());
    let trace = trace.lock().unwrap();
    trace.assert_addresses(&[blocked, refused, reachable]);
    assert_eq!(trace.peak, 2, "a quick refusal must advance without another stagger delay");
    assert_eq!(trace.active, 0);
    listener.accept().unwrap();
}

#[test]
fn overlapping_attempts_share_one_deadline() {
    let addresses: Vec<_> = (1..=16).map(|last| SocketAddr::from(([192, 0, 2, last], 631))).collect();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let (finished, done) = mpsc::channel();
    let started = std::time::Instant::now();
    let result = open_tcp("printserver", 631, addresses.clone(), move |addr, timeout| {
        record.lock().unwrap().start(addr, timeout);
        std::thread::sleep(timeout);
        record.lock().unwrap().finish();
        finished.send(()).unwrap();
        Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "address did not answer"))
    });
    let elapsed = started.elapsed();
    let count = trace.lock().unwrap().calls.len();
    for _ in 0..count {
        done.recv_timeout(Duration::from_secs(1)).unwrap();
    }
    assert!(matches!(result, Err(Error::Unreachable(_))));
    assert!(elapsed < CONNECT + Duration::from_secs(1), "each worker received a separate budget: {elapsed:?}");
    let trace = trace.lock().unwrap();
    assert!(count >= 3, "later addresses never received a chance");
    assert!(count <= 12, "the shared deadline and stagger must not fan out indefinitely");
    assert!(trace.calls.iter().all(|(addr, _)| addresses.contains(addr)));
    let mut called: Vec<_> = trace.calls.iter().map(|(addr, _)| *addr).collect();
    called.sort_unstable();
    called.dedup();
    assert_eq!(called.len(), count, "a resolved address was attempted more than once");
    assert!(trace.peak <= 12);
    assert_eq!(trace.active, 0);
}

#[test]
fn two_blocked_addresses_do_not_hide_a_reachable_third_address() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let third = listener.local_addr().unwrap();
    let first: SocketAddr = "192.0.2.1:631".parse().unwrap();
    let second: SocketAddr = "192.0.2.2:631".parse().unwrap();
    let trace = Arc::new(Mutex::new(ConnectTrace::default()));
    let record = Arc::clone(&trace);
    let (release_first, held_first) = mpsc::channel();
    let held_first = Mutex::new(held_first);
    let (release_second, held_second) = mpsc::channel();
    let held_second = Mutex::new(held_second);
    let (finished, done) = mpsc::channel();
    let (entered, wait_entered) = mpsc::channel();
    let wait_entered = Mutex::new(wait_entered);
    let started = std::time::Instant::now();
    let result = open_tcp("printserver", 631, [first, second, third], move |addr, timeout| {
        if addr == third {
            for _ in 0..2 {
                wait_entered.lock().unwrap().recv_timeout(timeout).unwrap();
            }
        }
        record.lock().unwrap().start(addr, timeout);
        let result = if addr == third {
            TcpStream::connect_timeout(&addr, timeout)
        } else {
            entered.send(()).unwrap();
            if addr == first {
                let _ = held_first.lock().unwrap().recv_timeout(timeout);
            } else {
                let _ = held_second.lock().unwrap().recv_timeout(timeout);
            }
            Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "address did not answer"))
        };
        record.lock().unwrap().finish();
        if addr != third {
            let _ = finished.send(());
        }
        result
    });
    let elapsed = started.elapsed();
    let _ = release_first.send(());
    let _ = release_second.send(());
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    done.recv_timeout(Duration::from_secs(1)).unwrap();
    assert!(result.is_ok(), "two stalled addresses hid the reachable third address");
    assert!(elapsed < Duration::from_secs(2), "the third address waited for the blocked attempts: {elapsed:?}");
    let trace = trace.lock().unwrap();
    trace.assert_addresses(&[first, second, third]);
    assert_eq!(trace.peak, 3);
    assert_eq!(trace.active, 0);
    listener.accept().unwrap();
}

#[test]
fn a_socket_connecting_after_another_attempt_wins_is_closed() {
    let loser = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let loser_addr = loser.local_addr().unwrap();
    let winner = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let winner_addr = winner.local_addr().unwrap();
    let (release, held) = mpsc::channel();
    let held = Mutex::new(held);
    let result = open_tcp("printserver", 631, [loser_addr, winner_addr], move |addr, timeout| {
        if addr == loser_addr {
            held.lock().unwrap().recv_timeout(timeout).unwrap();
        }
        TcpStream::connect_timeout(&addr, timeout)
    });
    let _ = release.send(());
    assert!(result.is_ok());
    winner.accept().unwrap();
    loser.set_nonblocking(true).unwrap();
    let deadline = std::time::Instant::now() + CONNECT;
    let mut late = loop {
        match loser.accept() {
            Ok((stream, _)) => break stream,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock && std::time::Instant::now() < deadline => {
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(error) => panic!("the losing connection did not arrive before the deadline: {error}"),
        }
    };
    late.set_nonblocking(false).unwrap();
    late.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
    let mut byte = [0];
    assert_eq!(late.read(&mut byte).unwrap(), 0, "the losing socket was retained after the coordinator returned");
}

#[test]
fn localized_queue_default_and_status_text_reach_callers() {
    let name = language_value("en", b"Office");
    let info = language_value("fr", "Imprimante générale".as_bytes());
    let body = ipp(0, &[ok_header(), (TAG_PRINTER_GROUP, &[(0x36, "printer-name", &[&name]), (0x35, "printer-info", &[&info])])]);
    for chunked in [false, true] {
        let (ep, _rx) = fake(body.clone(), chunked, 200);
        let list = queues(&ep, OP_CUPS_GET_PRINTERS, false).unwrap();
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "Office");
        assert_eq!(list[0].info, "Imprimante générale");
        assert_eq!(queue_path(&list[0].name, false).unwrap(), "/printers/Office");
        let (ep, _rx) = fake(body.clone(), chunked, 200);
        assert_eq!(default_printer(&ep).unwrap().as_deref(), Some("Office"));
        let reply = ipp(0x0506, &[ok_header(), (TAG_OPERATION_GROUP, &[(0x35, "status-message", &[&info])])]);
        let (ep, _rx) = fake(reply, chunked, 200);
        assert_eq!(queues(&ep, OP_CUPS_GET_PRINTERS, false), Err(Error::Status(0x0506, "Imprimante générale".into())));
    }
}
