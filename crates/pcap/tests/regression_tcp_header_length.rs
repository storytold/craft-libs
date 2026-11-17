use craft_anything_formats::pcap::{Decoded, decode};
use craft_anything_testkit::pcap::{ipv4_pkt, tcp};

fn decoded(segment: &[u8]) -> Decoded {
    decode(101, &ipv4_pkt(6, [192, 0, 2, 1], [192, 0, 2, 2], segment))
}

#[test]
fn a_tcp_header_shorter_than_twenty_bytes_is_not_payload() {
    for words in 0..5u8 {
        let mut segment = tcp(51000, 80, 0x18, b"GET / HTTP/1.1\r\n\r\n");
        segment[12] = words << 4;
        let result = decoded(&segment);
        let transport = result.transport.unwrap();
        assert_eq!(transport.payload_len, 0, "offset {words}");
        assert_eq!((transport.src_port, transport.dst_port), (Some(51000), Some(80)));
        assert!(result.http.is_none() && result.tls.is_none());
        assert!(result.note.is_some());
    }
}

#[test]
fn a_cut_tcp_header_keeps_ports_but_reports_the_cut() {
    let whole = tcp(51000, 80, 0x18, b"request");
    for end in 4..20 {
        let result = decoded(&whole[..end]);
        let transport = result.transport.unwrap();
        assert_eq!((transport.src_port, transport.dst_port), (Some(51000), Some(80)));
        assert_eq!(transport.payload_len, 0);
        assert!(result.note.is_some(), "cut at {end}");
    }
    for words in 6..=15u8 {
        let mut segment = tcp(51000, 80, 0x18, &[]);
        segment[12] = words << 4;
        let result = decoded(&segment);
        assert_eq!(result.transport.unwrap().payload_len, 0);
        assert!(result.note.is_some(), "missing options for offset {words}");
    }
}

#[test]
fn a_valid_tcp_header_with_options_finds_its_payload() {
    let mut segment = tcp(51000, 80, 0x18, &[]);
    segment[12] = 7 << 4;
    segment.extend([1; 8]);
    let payload = b"GET /resource HTTP/1.1\r\nHost: example.test\r\n\r\n";
    segment.extend(payload);
    let result = decoded(&segment);
    assert_eq!(result.transport.unwrap().payload_len, payload.len());
    assert_eq!(result.http.unwrap().target.as_deref(), Some("/resource"));
    assert!(result.note.is_none());
}
