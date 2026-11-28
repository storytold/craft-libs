use craft_pcap::decode;

/// Decode a raw IPv4 packet with a TCP header and the specified control bits.
fn flags_on_wire(bits: u8) -> String {
    let mut packet = [0u8; 40];
    packet[0] = 0x45;
    packet[2..4].copy_from_slice(&40u16.to_be_bytes());
    packet[8] = 64;
    packet[9] = 6;
    packet[20..24].copy_from_slice(&[0xc0, 0, 0x01, 0xbb]);
    packet[32] = 0x50;
    packet[33] = bits;
    decode(228, &packet).transport.unwrap().tcp_flags.unwrap()
}

/// An ECN-capable SYN shows both classic ECN flags.
#[test]
fn reports_ecn_setup_on_syn() {
    assert_eq!(flags_on_wire(0xc2), "SYN,ECE,CWR");
}

/// The receiver and sender can signal ECE and CWR independently on ACKs.
#[test]
fn distinguishes_ecn_echo_and_window_reduction() {
    assert_eq!(flags_on_wire(0x50), "ACK,ECE");
    assert_eq!(flags_on_wire(0x90), "ACK,CWR");
}

/// Existing non-ECN packets keep their flag strings.
#[test]
fn keeps_ordinary_ack_flags() {
    assert_eq!(flags_on_wire(0x12), "SYN,ACK");
}
