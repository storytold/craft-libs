use craft_anything_formats::pcap::decode;
use craft_anything_testkit::pcap::{eth, udp};

fn packet(offset: u16, next: u8, payload: &[u8]) -> Vec<u8> {
    let mut ip = vec![0u8; 40];
    ip[0] = 0x60;
    ip[4..6].copy_from_slice(&((8 + payload.len()) as u16).to_be_bytes());
    ip[6] = 44;
    ip.extend([next, 0]);
    ip.extend(offset.to_be_bytes());
    ip.extend([0; 4]);
    ip.extend(payload);
    eth(0x86dd, &ip)
}

#[test]
fn a_noninitial_fragment_cannot_reset_its_offset_from_payload_bytes() {
    let mut payload = vec![17, 0, 0, 0, 0, 0, 0, 0];
    payload.extend(udp(100, 200, b"body"));
    let decoded = decode(1, &packet(8, 44, &payload));
    assert!(decoded.ip.unwrap().fragment);
    assert!(decoded.transport.is_none());
    assert!(!decoded.protocols.contains(&"udp"));
}

#[test]
fn noninitial_fragment_bytes_cannot_change_the_recorded_protocol() {
    let decoded = decode(1, &packet(8, 60, &[17, 0, 0, 0, 0, 0, 0, 0]));
    assert_eq!(decoded.ip.unwrap().protocol, 60);
    assert!(decoded.transport.is_none());
}

#[test]
fn initial_and_atomic_fragments_still_decode_transport_headers() {
    for offset in [0, 1] {
        let decoded = decode(1, &packet(offset, 17, &udp(100, 200, b"body")));
        assert!(!decoded.ip.unwrap().fragment);
        assert_eq!(decoded.transport.unwrap().payload_len, 4);
    }
}
