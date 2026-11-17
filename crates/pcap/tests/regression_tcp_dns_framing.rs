use craft_anything_formats::pcap::{Decoded, decode};
use craft_anything_testkit::pcap::{dns_query, dns_response, ipv4_pkt, tcp};

fn decoded(payload: &[u8]) -> Decoded {
    decode(101, &ipv4_pkt(6, [192, 0, 2, 1], [192, 0, 2, 2], &tcp(53, 51000, 0x18, payload)))
}

fn framed(message: &[u8], declared: usize) -> Vec<u8> {
    let mut payload = (declared as u16).to_be_bytes().to_vec();
    payload.extend(message);
    payload
}

#[test]
fn an_unfinished_dns_message_is_not_reported_complete() {
    let message = dns_response();
    let result = decoded(&framed(&message, message.len() + 1));
    assert!(result.dns.is_none());
    assert!(result.note.is_some());
    assert_eq!(result.transport.unwrap().src_port, Some(53));
}

#[test]
fn dns_records_cannot_borrow_bytes_past_the_message_length() {
    let result = decoded(&framed(&dns_response(), 12));
    let dns = result.dns.unwrap();
    assert!(dns.questions.is_empty() && dns.answers.is_empty());
    assert!(dns.incomplete);
}

#[test]
fn a_length_smaller_than_the_dns_header_is_rejected() {
    for length in 0..12 {
        let result = decoded(&framed(&dns_response(), length));
        assert!(result.dns.is_none(), "length {length}");
        assert!(result.note.is_some(), "length {length}");
    }
}

#[test]
fn a_partial_dns_length_prefix_is_reported() {
    let result = decoded(&[0]);
    assert!(result.dns.is_none());
    assert!(result.note.is_some());
}

#[test]
fn an_empty_tcp_ack_does_not_claim_a_missing_dns_message() {
    let result = decoded(&[]);
    assert!(result.dns.is_none());
    assert!(result.note.is_none());
    assert_eq!(result.transport.unwrap().payload_len, 0);
}

#[test]
fn the_first_complete_dns_message_is_decoded_without_the_next_one() {
    let first = dns_query();
    let second = dns_response();
    let mut payload = framed(&first, first.len());
    payload.extend(framed(&second, second.len()));
    let result = decoded(&payload);
    let dns = result.dns.unwrap();
    assert!(!dns.response && !dns.incomplete);
    assert_eq!(dns.questions[0].name, "example.com");
    assert!(dns.answers.is_empty());
    assert!(result.note.is_none());
}
