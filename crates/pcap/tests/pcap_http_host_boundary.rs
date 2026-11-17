use craft_anything_formats::pcap::{Http, decode};
use craft_anything_testkit::pcap::{ipv4_pkt, tcp};

fn request(data: &[u8]) -> Http {
    decode(101, &ipv4_pkt(6, [1; 4], [2; 4], &tcp(100, 80, 0x18, data))).http.unwrap()
}

#[test]
fn a_host_shaped_line_in_the_body_is_not_a_request_header() {
    let http = request(b"POST / HTTP/1.1\r\nContent-Length: 22\r\n\r\nbody\r\nHost: forged.test\r\n");
    assert!(http.host.is_none());
}

#[test]
fn invalid_utf8_before_host_does_not_shift_header_byte_offsets() {
    let http = request(b"GET / HTTP/1.1\r\nX-Data: \xff\r\nHoSt:\tReal.test \t\r\n\r\n");
    assert_eq!(http.host.as_deref(), Some("Real.test"));
}

#[test]
fn host_requires_a_complete_header_line_and_keeps_its_spelling() {
    assert!(request(b"GET / HTTP/1.1\r\nHost: cut").host.is_none());
    assert_eq!(request(b"GET / HTTP/1.1\r\nHost: Real.test\r\n\r\n").host.as_deref(), Some("Real.test"));
}

#[test]
fn http_evidence_needs_a_complete_start_line() {
    for input in [
        b"GET / HTTP/1.1".as_slice(),
        b"GET / HTTP/1.1x\r\n",
        b"GET / HTTP/1.1 extra\r\n",
        b"GET  HTTP/1.0\r\n",
        b"GET /bad\tpath HTTP/1.1\r\n",
        b"HTTP/1.garbage\r\n",
        b"HTTP/1.1 20 OK\r\n",
        b"HTTP/1.1 999 OK\r\n",
        b"HTTP/1.1 200 OK",
        b"HTTP/1.garbage 204\r\n\r\n",
        b"HTTP/1.1 20\r\n\r\n",
        b"HTTP/1.1 2040\r\n\r\n",
        b"HTTP/1.1 999\r\n\r\n",
        b"HTTP/1.1 2x4\r\n\r\n",
        b"HTTP/1.1 204",
        b"HTTP/1.1 204 bad\0reason\r\n\r\n",
    ] {
        assert!(decode(101, &ipv4_pkt(6, [1; 4], [2; 4], &tcp(100, 80, 0x18, input))).http.is_none(), "{input:?}");
    }
    assert_eq!(request(b"GET / HTTP/1.0\r\n").version, "HTTP/1.0");
    assert_eq!(request(b"HTTP/1.1 204 \r\n").status, Some(204));
    assert_eq!(request(b"HTTP/1.1 403 Access denied\r\n").status, Some(403));
}

#[test]
fn responses_with_no_reason_phrase_still_keep_the_status_code() {
    for (input, version, code) in
        [(b"HTTP/1.1 204\r\n\r\n".as_slice(), "HTTP/1.1", 204), (b"HTTP/1.0 200\r\nContent-Length: 0\r\n\r\n".as_slice(), "HTTP/1.0", 200)]
    {
        let decoded = decode(101, &ipv4_pkt(6, [1; 4], [2; 4], &tcp(80, 100, 0x18, input)));
        assert!(decoded.protocols.contains(&"http"));
        let http = decoded.http.unwrap();
        assert_eq!(http.kind, "response");
        assert_eq!(http.version, version);
        assert_eq!(http.status, Some(code));
        assert!(http.method.is_none());
        assert!(http.target.is_none());
    }
}
