use craft_anything_formats::pcap::{Dns, decode};
use craft_anything_testkit::pcap::{ipv4_pkt, udp};

fn response(data: &[u8]) -> Dns {
    let mut msg = vec![0, 1, 0x81, 0x80, 0, 0, 0, 1, 0, 0, 0, 0];
    msg.extend([0, 0, 16, 0, 1, 0, 0, 0, 60]);
    msg.extend((data.len() as u16).to_be_bytes());
    msg.extend(data);
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(53, 100, &msg))).dns.unwrap()
}

fn prerequisites(flags: u16, class: u16, ttl: u32, data: &[u8]) -> Dns {
    let mut msg = vec![0, 1];
    msg.extend(flags.to_be_bytes());
    msg.extend([0, 1, 0, 2, 0, 0, 0, 0]);
    // UPDATE's zone section has the question layout: example.test, SOA, IN.
    msg.extend(b"\x07example\x04test\x00\x00\x06\x00\x01");
    msg.extend([0xc0, 12, 0, 16]);
    msg.extend(class.to_be_bytes());
    msg.extend(ttl.to_be_bytes());
    msg.extend((data.len() as u16).to_be_bytes());
    msg.extend(data);
    // Different owners let NONE and value-dependent prerequisites hold together.
    // The following TXT record must also survive.
    msg.extend(b"\x05after");
    msg.extend([0xc0, 12, 0, 16, 0, 1, 0, 0, 0, 0, 0, 3, 2, b'o', b'k']);
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(100, 53, &msg))).dns.unwrap()
}

fn named_txt(owner: &[&[u8]], data: &[u8], port: u16, flags: u16, class: u16, ttl: u32, compressed: bool) -> Dns {
    let mut name = Vec::new();
    for label in owner {
        name.push(u8::try_from(label.len()).unwrap());
        name.extend(*label);
    }
    name.push(0);
    let mut msg = vec![0, 1];
    msg.extend(flags.to_be_bytes());
    msg.extend([0, 1, 0, 2, 0, 0, 0, 0]);
    msg.extend(&name);
    msg.extend([0, 16, 0, 1]);
    if compressed {
        msg.extend([0xc0, 12]);
    } else {
        msg.extend(&name);
    }
    msg.extend([0, 16]);
    msg.extend(class.to_be_bytes());
    msg.extend(ttl.to_be_bytes());
    msg.extend(u16::try_from(data.len()).unwrap().to_be_bytes());
    msg.extend(data);
    // The next answer must survive an empty DNS-SD TXT record.
    msg.extend([0xc0, 12, 0, 1, 0, 1, 0, 0, 0, 60, 0, 4, 127, 0, 0, 1]);
    decode(101, &ipv4_pkt(17, [1; 4], [2; 4], &udp(port, 100, &msg))).dns.unwrap()
}

fn assert_empty_txt_and_following_answer(dns: Dns) {
    assert!(!dns.incomplete);
    assert_eq!(dns.answers.len(), 2);
    assert_eq!(dns.answers[0].rtype, "TXT");
    assert_eq!(dns.answers[0].value, "");
    assert_eq!(dns.answers[1].value, "127.0.0.1");
}

#[test]
fn empty_dns_sd_txt_preserves_following_answers() {
    let multicast: &[&[u8]] = &[b"Printer", b"_ipp", b"_tcp", b"local"];
    let unicast: &[&[u8]] = &[b"Printer", b"_ipp", b"_tcp", b"example", b"test"];
    for (port, owner) in [(5353, multicast), (53, unicast)] {
        assert_empty_txt_and_following_answer(named_txt(owner, &[], port, 0x8400, 1, 60, false));
    }
}

#[test]
fn empty_dns_sd_txt_is_valid_in_mdns_known_answers_and_cache_flush_records() {
    let owner: &[&[u8]] = &[b"Printer", b"_ipp", b"_tcp", b"local"];
    // Queries may contain known answers; the cache-flush bit belongs to CLASS.
    for (flags, class) in [(0, 1), (0x8400, 0x8001)] {
        assert_empty_txt_and_following_answer(named_txt(owner, &[], 5353, flags, class, 60, false));
    }
}

#[test]
fn dns_sd_owner_preserves_wire_labels_case_and_compression() {
    let owner: &[&[u8]] = &[b"Printer.with dots", b"_Ab-12", b"_UdP", b"example", b"test"];
    for compressed in [false, true] {
        assert_empty_txt_and_following_answer(named_txt(owner, &[], 53, 0x8180, 1, 60, compressed));
    }
}

#[test]
fn other_empty_txt_owners_are_not_dns_sd_instances() {
    let owners: &[&[&[u8]]] = &[
        &[b"Printer", b"local"],
        &[b"Printer._ipp._tcp", b"local"],
        &[b"Printer", b"_ipp._tcp", b"_tcp", b"local"],
        &[b"_ipp", b"_tcp", b"local"],
        &[b"Printer", b"_ipp", b"_tcp"],
        &[b"Printer", b"_ipp", b"_other", b"local"],
        &[b"Printer", b"_123", b"_tcp", b"local"],
        &[b"Printer", b"_bad--name", b"_tcp", b"local"],
    ];
    for owner in owners {
        let dns = named_txt(owner, &[], 5353, 0x8400, 1, 60, false);
        assert!(dns.incomplete, "owner {owner:?}");
        assert!(dns.answers.is_empty());
    }
}

#[test]
fn dns_sd_txt_still_rejects_truncated_nonempty_strings() {
    let owner: &[&[u8]] = &[b"Printer", b"_ipp", b"_tcp", b"local"];
    for data in [&b"\x02x"[..], &b"\x00\x04x"[..]] {
        let dns = named_txt(owner, data, 5353, 0x8400, 0x8001, 60, true);
        assert!(dns.incomplete);
        assert!(dns.answers.is_empty());
    }
}

#[test]
fn dns_sd_owner_does_not_relax_update_prerequisite_classes() {
    let owner: &[&[u8]] = &[b"Printer", b"_ipp", b"_tcp", b"example", b"test"];
    let dns = named_txt(owner, &[], 53, 5 << 11, 1, 0, true);
    assert!(dns.incomplete);
    assert!(dns.answers.is_empty());
    for class in [254, 255] {
        assert_empty_txt_and_following_answer(named_txt(owner, &[], 53, 5 << 11, class, 0, true));
    }
}

#[test]
fn empty_update_prerequisites_preserve_following_records() {
    for class in [254, 255] {
        let dns = prerequisites(5 << 11, class, 0, &[]);
        assert!(!dns.incomplete, "class {class}");
        assert_eq!(dns.answers.len(), 2);
        assert_eq!(dns.answers[0].name, "example.test");
        assert_eq!(dns.answers[0].value, "");
        assert_eq!(dns.answers[1].name, "after.example.test");
        assert_eq!(dns.answers[1].value, "ok");
    }
}

#[test]
fn empty_txt_rdata_requires_update_and_a_prerequisite_class() {
    assert!(response(&[]).incomplete);
    for (flags, class, ttl) in [(0x8180, 254, 0), (0x8180, 255, 0), (5 << 11, 1, 0), (5 << 11, 254, 60), (5 << 11, 255, 60)] {
        let dns = prerequisites(flags, class, ttl, &[]);
        assert!(dns.incomplete, "flags {flags:#x}, class {class}, ttl {ttl}");
        assert!(dns.answers.is_empty());
    }
    for opcode in [1, 2, 3, 4, 6] {
        for class in [254, 255] {
            let dns = prerequisites(opcode << 11, class, 0, &[]);
            assert!(dns.incomplete, "opcode {opcode}, class {class}");
            assert!(dns.answers.is_empty());
        }
    }
}

#[test]
fn update_prerequisite_classes_do_not_allow_truncated_strings() {
    for class in [254, 255] {
        let dns = prerequisites(5 << 11, class, 0, b"\x02x");
        assert!(dns.incomplete);
        assert!(dns.answers.is_empty());
    }
}

#[test]
fn truncated_txt_strings_are_not_complete_answers() {
    for data in [&b"\x05abc"[..], &b"\x02ok\x04x"[..], &b"\x01"[..]] {
        let dns = response(data);
        assert!(dns.incomplete);
        assert!(dns.answers.is_empty());
    }
}

#[test]
fn empty_and_multiple_complete_strings_remain_valid() {
    for (data, expected) in [(&b"\x00"[..], ""), (&b"\x00\x02ok\x01!"[..], "ok!")] {
        let dns = response(data);
        assert!(!dns.incomplete);
        assert_eq!(dns.answers[0].value, expected);
    }
}

#[test]
fn value_dependent_update_accepts_an_empty_character_string() {
    let dns = prerequisites(5 << 11, 1, 0, b"\x00");
    assert!(!dns.incomplete);
    assert_eq!(dns.answers.len(), 2);
    assert_eq!(dns.answers[0].value, "");
    assert_eq!(dns.answers[1].name, "after.example.test");
    assert_eq!(dns.answers[1].value, "ok");
}
