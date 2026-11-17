use craft_anything_formats::pcap::{Dns, decode};
use craft_anything_testkit::pcap::{ipv4_pkt, udp};

fn query(last: usize, compressed: bool) -> Dns {
    let mut msg = vec![0, 1, 1, 0, 0, if compressed { 2 } else { 1 }, 0, 0, 0, 0, 0, 0];
    for len in [63, 63, 63, last] {
        msg.push(len as u8);
        msg.extend(vec![b'a'; len]);
    }
    msg.extend([0, 0, 1, 0, 1]);
    if compressed {
        msg.extend([0xc0, 12, 0, 1, 0, 1]);
    }
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(100, 53, &msg))).dns.unwrap()
}

#[test]
fn label_lengths_and_the_root_octet_count_towards_the_dns_limit() {
    for last in [62, 63] {
        let dns = query(last, false);
        assert!(dns.incomplete);
        assert!(dns.questions.is_empty());
    }
}

#[test]
fn a_255_octet_name_is_valid_in_plain_and_compressed_form() {
    for compressed in [false, true] {
        let dns = query(61, compressed);
        assert!(!dns.incomplete);
        assert_eq!(dns.questions.len(), if compressed { 2 } else { 1 });
        assert_eq!(dns.questions[0].name.len(), 253);
    }
}

fn prefixed_pointer(last: usize, answer_type: Option<u16>) -> Dns {
    let mut msg = vec![0, 1, 0x81, 0x80, 0, if answer_type.is_some() { 1 } else { 2 }, 0, u8::from(answer_type.is_some()), 0, 0, 0, 0];
    for len in [63, 63, 63, last] {
        msg.push(len as u8);
        msg.extend(vec![b'a'; len]);
    }
    msg.extend([0, 0, 1, 0, 1]);
    if let Some(rtype) = answer_type {
        msg.extend([0xc0, 12]);
        msg.extend(rtype.to_be_bytes());
        msg.extend([0, 1, 0, 0, 0, 0, 0, if rtype == 15 { 6 } else { 4 }]);
        if rtype == 15 {
            msg.extend([0, 10]);
        }
    }
    msg.extend([1, b'x', 0xc0, 12]);
    if answer_type.is_none() {
        msg.extend([0, 1, 0, 1]);
    }
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(100, 53, &msg))).dns.unwrap()
}

#[test]
fn a_literal_prefix_and_pointer_cannot_exceed_the_question_name_limit() {
    let dns = prefixed_pointer(61, None);
    assert!(dns.incomplete);
    assert_eq!(dns.questions.len(), 1);
}

#[test]
fn invalid_name_rdata_is_incomplete_instead_of_an_empty_answer() {
    for rtype in [2, 5, 12, 15] {
        let dns = prefixed_pointer(61, Some(rtype));
        assert!(dns.incomplete, "record type {rtype}");
        assert_eq!(dns.questions.len(), 1);
        assert!(dns.answers.is_empty(), "record type {rtype}");
    }
}

#[test]
fn a_prefixed_pointer_at_255_octets_remains_valid() {
    let dns = prefixed_pointer(59, None);
    assert!(!dns.incomplete);
    assert_eq!(dns.questions.len(), 2);
    for rtype in [2, 5, 12, 15] {
        let dns = prefixed_pointer(59, Some(rtype));
        assert!(!dns.incomplete, "record type {rtype}");
        assert_eq!(dns.answers.len(), 1);
        assert_eq!(dns.answers[0].value.len(), 253);
    }
}

fn name_rdata_then_a(flags: u16, class: u16, ttl: u32, rtype: u16, rdata: &[u8]) -> Dns {
    let mut msg = vec![0, 1];
    msg.extend(flags.to_be_bytes());
    msg.extend([0, 1, 0, 2, 0, 0, 0, 0]);
    msg.extend(b"\x07example\x04test\0");
    msg.extend([0, 6, 0, 1]); // Zone: SOA/IN for UPDATE requests.
    msg.extend([3, b'o', b'n', b'e', 0xc0, 12]);
    msg.extend(rtype.to_be_bytes());
    msg.extend(class.to_be_bytes());
    msg.extend(ttl.to_be_bytes());
    msg.extend((rdata.len() as u16).to_be_bytes());
    msg.extend(rdata);
    // A different owner keeps the two UPDATE prerequisites independent.
    msg.extend([4, b'n', b'e', b'x', b't', 0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 0, 0, 4, 203, 0, 113, 1]);
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(100, 53, &msg))).dns.unwrap()
}

#[test]
fn empty_update_name_prerequisites_preserve_the_following_record() {
    for rtype in [2, 5, 12, 15] {
        for class in [254, 255] {
            let dns = name_rdata_then_a(0x2800, class, 0, rtype, &[]);
            assert!(!dns.incomplete, "type {rtype}, class {class}");
            assert_eq!(dns.answers.len(), 2);
            assert_eq!(dns.answers[0].value, "");
            assert_eq!(dns.answers[1].value, "203.0.113.1");
        }
    }
}

#[test]
fn empty_name_rdata_is_rejected_outside_legal_update_prerequisites() {
    let cases = [(0x8180, 254, 0), (0x8180, 255, 0), (0x2000, 254, 0), (0x2000, 255, 0), (0x2800, 1, 0), (0x2800, 254, 1), (0x2800, 255, 1)];
    for rtype in [2, 5, 12, 15] {
        for (flags, class, ttl) in cases {
            let dns = name_rdata_then_a(flags, class, ttl, rtype, &[]);
            assert!(dns.incomplete, "type {rtype}, flags {flags:#x}, class {class}, TTL {ttl}");
            assert!(dns.answers.is_empty());
        }
    }
}

#[test]
fn a_cut_rdata_pointer_cannot_borrow_the_following_owner() {
    for rtype in [2, 5, 12, 15] {
        let mut rdata = if rtype == 15 { vec![0, 10] } else { Vec::new() };
        rdata.push(0xc0); // The next owner's length would complete a pointer to the header's zero byte.
        let dns = name_rdata_then_a(0x8180, 1, 0, rtype, &rdata);
        assert!(dns.incomplete, "type {rtype}");
        assert!(dns.answers.is_empty());
    }
}

#[test]
fn root_name_rdata_remains_valid() {
    for rtype in [2, 5, 12, 15] {
        let rdata = if rtype == 15 { vec![0, 0, 0] } else { vec![0] };
        let dns = name_rdata_then_a(0x8180, 1, 0, rtype, &rdata);
        assert!(!dns.incomplete, "type {rtype}");
        assert_eq!(dns.answers.len(), 2);
        assert_eq!(dns.answers[0].value, ".");
        assert_eq!(dns.answers[1].value, "203.0.113.1");
    }
}

#[test]
fn dns_label_escapes_distinguish_wire_names_in_question_and_answer_owners() {
    let mut message = vec![0, 1, 0x81, 0x80, 0, 2, 0, 1, 0, 0, 0, 0];
    message.extend(b"\x03a.b\x03\\\x00\xff\x00\x00\x01\x00\x01");
    message.extend(b"\x01a\x01b\x03\\\x00\xff\x00\x00\x01\x00\x01");
    message.extend([0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 0, 0, 4, 1, 2, 3, 4]);
    let dns = decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(53, 100, &message))).dns.unwrap();
    assert!(!dns.incomplete);
    assert_eq!(dns.questions[0].name, r"a\.b.\\\000\255");
    assert_eq!(dns.questions[1].name, r"a.b.\\\000\255");
    assert_eq!(dns.answers[0].name, dns.questions[0].name);
}
