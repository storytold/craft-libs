use craft_pcap::decode;

/// Build a raw IPv6 packet with the given payload and Next Header field.
fn packet(next: u8, payload: &[u8]) -> Vec<u8> {
    let mut packet = vec![0; 40];
    packet[0] = 0x60;
    packet[4..6].copy_from_slice(&u16::try_from(payload.len()).unwrap().to_be_bytes());
    packet[6] = next;
    packet[7] = 64;
    packet.extend_from_slice(payload);
    packet
}

/// Both an empty packet and bytes after No Next Header have no transport record.
#[test]
fn no_next_header_has_no_transport() {
    for payload in [b"".as_slice(), b"padding"] {
        let decoded = decode(229, &packet(59, payload));
        assert_eq!(decoded.ip.unwrap().protocol, 59);
        assert_eq!(decoded.protocols, ["raw", "ipv6"]);
        assert!(decoded.transport.is_none());
    }
}

/// Destination Options can also end the header chain.
#[test]
fn no_next_header_after_an_extension() {
    let decoded = decode(229, &packet(60, &[59, 0, 0, 0, 0, 0, 0, 0, 1, 2]));
    assert_eq!(decoded.ip.unwrap().protocol, 59);
    assert!(decoded.transport.is_none());
}

/// UDP after the same extension still yields its transport and payload.
#[test]
fn udp_after_an_extension_still_decodes() {
    let decoded = decode(229, &packet(60, &[17, 0, 0, 0, 0, 0, 0, 0, 0, 10, 0, 11, 0, 9, 0, 0, 42]));
    let transport = decoded.transport.unwrap();
    assert_eq!(transport.protocol, "udp");
    assert_eq!((transport.src_port, transport.dst_port, transport.payload_len), (Some(10), Some(11), 1));
}
