use craft_anything_formats::pcap::decode;
use craft_anything_testkit::pcap::{ipv4_pkt, udp};

#[test]
fn incomplete_ipv4_base_headers_explain_why_decoding_stopped() {
    let packet = ipv4_pkt(17, [1; 4], [2; 4], &[]);
    for captured in 10..20 {
        let decoded = decode(101, &packet[..captured]);
        assert!(decoded.ip.is_none(), "captured {captured} bytes");
        assert!(decoded.transport.is_none(), "captured {captured} bytes");
        assert_eq!(decoded.note.as_deref(), Some("IPv4 header cut short"), "captured {captured} bytes");
    }
}

#[test]
fn an_ipv4_total_length_cannot_be_shorter_than_its_header() {
    for total in [0u16, 19, 23] {
        let mut packet = ipv4_pkt(17, [1, 2, 3, 4], [5, 6, 7, 8], &udp(100, 200, b"body"));
        packet[0] = 0x46;
        packet[2..4].copy_from_slice(&total.to_be_bytes());
        let decoded = decode(101, &packet);
        assert!(decoded.ip.is_none());
        assert!(decoded.transport.is_none());
        assert!(decoded.note.is_some());
    }
}

#[test]
fn missing_ipv4_options_do_not_form_a_complete_ip_header() {
    let mut packet = ipv4_pkt(17, [1; 4], [2; 4], &[]);
    packet[0] = 0x46;
    packet[2..4].copy_from_slice(&24u16.to_be_bytes());
    let decoded = decode(101, &packet);
    assert!(decoded.ip.is_none());
    assert!(decoded.note.is_some());
}

#[test]
fn complete_options_and_truncated_transport_keep_their_valid_fields() {
    let body = udp(100, 200, b"body");
    let mut payload = vec![0; 4];
    payload.extend(body);
    let mut packet = ipv4_pkt(17, [1; 4], [2; 4], &payload);
    packet[0] = 0x46;
    assert_eq!(decode(101, &packet).transport.unwrap().payload_len, 4);
    packet.truncate(34);
    let decoded = decode(101, &packet);
    assert!(decoded.ip.is_some());
    let transport = decoded.transport.unwrap();
    assert_eq!(transport.src_port, Some(100));
    assert_eq!(transport.dst_port, Some(200));
    assert_eq!(transport.payload_len, 2);
    packet.truncate(28);
    let decoded = decode(101, &packet);
    assert!(decoded.ip.is_some());
    assert_eq!(decoded.transport.unwrap().src_port, Some(100));
}
