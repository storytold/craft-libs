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
