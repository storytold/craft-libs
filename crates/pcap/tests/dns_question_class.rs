use craft_anything_formats::pcap::{decode, summarize};
use craft_anything_testkit::pcap::{dns_query, eth, ipv4_pkt, pcap, udp};

fn packet(message: &[u8]) -> Vec<u8> {
    eth(0x0800, &ipv4_pkt(17, [192, 0, 2, 1], [192, 0, 2, 2], &udp(51000, 53, message)))
}

/// Neither a missing class nor a one-byte class can complete a DNS question.
#[test]
fn a_question_missing_any_class_byte_is_incomplete() {
    let complete = dns_query();
    for missing in [1, 2] {
        let message = &complete[..complete.len() - missing];
        let dns = decode(1, &packet(message)).dns.unwrap();
        assert!(dns.incomplete, "missing {missing} class bytes");
        assert!(dns.questions.is_empty(), "a question needs both type and class");
    }
}

#[test]
fn complete_questions_survive_before_a_truncated_question() {
    let mut message = dns_query();
    message[4..6].copy_from_slice(&2u16.to_be_bytes());
    message.extend([0xc0, 12, 0, 28, 0]); // compressed name, AAAA, one class byte

    let dns = decode(1, &packet(&message)).dns.unwrap();
    assert!(dns.incomplete);
    assert_eq!(dns.questions.len(), 1);
    assert_eq!(dns.questions[0].name, "example.com");
    assert_eq!(dns.questions[0].qtype, "A");
}

#[test]
fn incomplete_questions_do_not_enter_the_query_inventory() {
    let mut message = dns_query();
    message.pop();
    let summary = summarize(&pcap(&[packet(&message)]), &|| false).unwrap();
    assert_eq!(summary.packets, 1);
    assert!(summary.dns_queries.is_empty());
}

/// Class validation requires both bytes without restricting valid class values.
#[test]
fn complete_class_fields_are_accepted() {
    for class in [1u16, 3, 255] {
        let mut message = dns_query();
        message.truncate(message.len() - 2);
        message.extend(class.to_be_bytes());
        let dns = decode(1, &packet(&message)).dns.unwrap();
        assert!(!dns.incomplete);
        assert_eq!(dns.questions.len(), 1);
        assert_eq!(dns.questions[0].name, "example.com");
        assert_eq!(dns.questions[0].qtype, "A");
    }
}
