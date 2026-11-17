use craft_anything_formats::pcap::decode;
use craft_anything_testkit::pcap::{eth, ipv4_pkt};

fn raw_packets() -> (Vec<u8>, Vec<u8>) {
    let ipv4 = ipv4_pkt(17, [1; 4], [2; 4], &[0; 20]);
    let mut ipv6 = vec![0u8; 40];
    ipv6[0] = 0x60;
    ipv6[6] = 59;
    (ipv4, ipv6)
}

#[test]
fn explicit_raw_link_types_reject_the_other_ip_version() {
    let (ipv4, ipv6) = raw_packets();
    for (link_type, packet) in [(228, &ipv6), (229, &ipv4)] {
        let decoded = decode(link_type, packet);
        assert!(decoded.ip.is_none(), "link type {link_type}");
        assert!(decoded.transport.is_none(), "link type {link_type}");
        assert!(decoded.note.is_some(), "link type {link_type}");
    }
}

#[test]
fn matching_raw_link_types_and_generic_raw_packets_remain_readable() {
    let (ipv4, ipv6) = raw_packets();
    assert_eq!(decode(228, &ipv4).ip.unwrap().version, 4);
    assert_eq!(decode(229, &ipv6).ip.unwrap().version, 6);
    for link_type in [12, 14, 101] {
        assert_eq!(decode(link_type, &ipv4).ip.unwrap().version, 4);
        assert_eq!(decode(link_type, &ipv6).ip.unwrap().version, 6);
    }
}

#[test]
fn an_ethernet_type_does_not_replace_the_ipv6_version_check() {
    for version in [0, 4, 5, 7, 15] {
        let mut packet = [0u8; 40];
        packet[0] = version << 4;
        packet[6] = 59;
        let decoded = decode(1, &eth(0x86dd, &packet));
        assert!(decoded.ip.is_none(), "version {version}");
        assert!(decoded.transport.is_none());
        assert!(decoded.note.is_some());
    }
}

#[test]
fn ipv6_version_keeps_traffic_class_bits() {
    let mut packet = [0u8; 40];
    packet[0] = 0x6f;
    packet[6] = 59;
    assert_eq!(decode(1, &eth(0x86dd, &packet)).ip.unwrap().version, 6);
}
