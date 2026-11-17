use craft_anything_formats::pcap::{Dns, decode};
use craft_anything_testkit::pcap::{dns_query, eth, ipv4_pkt, udp};

fn answer(kind: u16, rd: &[u8]) -> Dns {
    let mut msg = dns_query();
    msg[2..4].copy_from_slice(&0x8180u16.to_be_bytes());
    msg[6..8].copy_from_slice(&1u16.to_be_bytes());
    msg.extend([0xc0, 12]);
    msg.extend(kind.to_be_bytes());
    msg.extend([0, 1, 0, 0, 0, 60]);
    msg.extend((rd.len() as u16).to_be_bytes());
    msg.extend(rd);
    decode(1, &eth(0x0800, &ipv4_pkt(17, [192, 0, 2, 2], [192, 0, 2, 1], &udp(53, 51000, &msg)))).dns.unwrap()
}

#[test]
fn svc_binding_reports_target_and_port() {
    for kind in [64, 65] {
        let msg = answer(kind, b"\0\x01\x03api\0\0\x03\0\x02\x20\xfb");
        assert_eq!(msg.answers[0].value, "1 api port=8443");
        assert_eq!(answer(kind, b"\0\x01\0\0\x01\0\x03\x02h2\0\x04\0\x04\xc0\0\x02\x01").answers[0].value, "1 . alpn=h2 ipv4hint=192.0.2.1");
        assert!(answer(kind, b"\0\x01\0\0\x01\0\x01\x00").incomplete);
        assert!(answer(kind, b"\0\x01\xc0\x0c").incomplete);
        assert!(answer(kind, b"\0\x01\0\0\x03\0\x01\x00").incomplete);
    }
}
#[test]
fn sshfp_preserves_algorithm_and_complete_digest() {
    let mut d = vec![4, 2];
    d.extend([0x5a; 32]);
    assert_eq!(answer(44, &d).answers[0].value, format!("4 2 {}", "5a".repeat(32)));
    assert!(answer(44, &d[..15]).incomplete);
}
#[test]
fn tlsa_keeps_selector_and_certificate_association() {
    let mut d = vec![3, 1, 1];
    d.extend([0xaa; 32]);
    assert_eq!(answer(52, &d).answers[0].value, format!("3 1 1 {}", "aa".repeat(32)));
    assert!(answer(52, &d[..4]).incomplete);
}
#[test]
fn naptr_keeps_order_preference_strings_and_replacement() {
    let rd = b"\0\x0a\0\x02\x01s\x03sip\0\x03srv\0";
    assert_eq!(answer(35, rd).answers[0].value, r#"10 2 "s" "sip" "" srv"#);
    assert!(answer(35, &rd[..rd.len() - 1]).incomplete);
}
#[test]
fn uri_keeps_priority_weight_and_target() {
    assert_eq!(answer(256, b"\0\x02\0\x05https://x.test/s").answers[0].value, r#"2 5 "https://x.test/s""#);
    assert!(answer(256, b"\0\x02\0\x05").incomplete);
}
#[test]
fn dname_only_accepts_a_complete_uncompressed_target() {
    assert_eq!(answer(39, b"\x04host\0").answers[0].value, "host");
    assert!(answer(39, b"\xc0\x0c").incomplete);
    assert!(answer(39, b"\x04host\0extra").incomplete);
}
