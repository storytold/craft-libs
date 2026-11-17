use craft_anything_formats::pcap::{Decoded, decode};
use craft_anything_testkit::pcap::{client_hello, ipv4_pkt, tcp};

fn decoded(record: &[u8]) -> Decoded {
    decode(101, &ipv4_pkt(6, [192, 0, 2, 1], [192, 0, 2, 2], &tcp(51000, 443, 0x18, record)))
}

fn assert_no_complete_sni(record: &[u8]) {
    let result = decoded(record);
    assert!(result.protocols.contains(&"tls"));
    assert!(result.tls.is_none_or(|hello| hello.sni.is_none() && hello.partial));
}

#[test]
fn a_client_hello_cannot_read_past_its_tls_record() {
    let mut record = client_hello("example.test");
    record[3..5].copy_from_slice(&4u16.to_be_bytes());
    assert_no_complete_sni(&record);
}

#[test]
fn a_client_hello_cannot_read_past_its_handshake() {
    let mut record = client_hello("example.test");
    record[6..9].copy_from_slice(&[0, 0, 34]);
    assert_no_complete_sni(&record);
}

#[test]
fn extensions_cannot_read_past_the_declared_extension_list() {
    let mut record = client_hello("example.test");
    record[50..52].copy_from_slice(&4u16.to_be_bytes());
    assert_no_complete_sni(&record);
}

#[test]
fn a_truncated_record_reports_partial_even_when_the_hello_fields_fit() {
    let mut record = client_hello("example.test");
    let larger = (record.len() + 20) as u16;
    record[3..5].copy_from_slice(&larger.to_be_bytes());
    assert!(decoded(&record).tls.unwrap().partial);
}

#[test]
fn server_names_stay_inside_the_name_list() {
    let mut record = client_hello("example.test");
    record[56..58].copy_from_slice(&3u16.to_be_bytes());
    assert_no_complete_sni(&record);
}

#[test]
fn alpn_names_stay_inside_the_protocol_list() {
    let mut record = client_hello("example.test");
    let alpn_list = 52 + 4 + 5 + "example.test".len() + 4;
    record[alpn_list..alpn_list + 2].copy_from_slice(&3u16.to_be_bytes());
    let hello = decoded(&record).tls.unwrap();
    assert_eq!(hello.alpn, ["h2"]);
    assert!(hello.partial);
}

#[test]
fn supported_versions_stay_inside_the_version_list() {
    let mut record = client_hello("example.test");
    let versions = 52 + 4 + 5 + "example.test".len() + 4 + 14 + 4;
    record[versions] = 2;
    let hello = decoded(&record).tls.unwrap();
    assert_eq!(hello.supported_versions, ["TLS 1.3"]);
    assert!(hello.partial);
}

#[test]
fn a_complete_legacy_hello_can_omit_extensions() {
    let mut record = client_hello("example.test");
    record.truncate(50);
    record[3..5].copy_from_slice(&45u16.to_be_bytes());
    record[6..9].copy_from_slice(&[0, 0, 41]);
    record[46..48].copy_from_slice(&0x002fu16.to_be_bytes());
    let hello = decoded(&record).tls.unwrap();
    assert!(hello.sni.is_none() && hello.alpn.is_empty() && hello.supported_versions.is_empty());
    assert!(!hello.partial);
}

#[test]
fn a_following_handshake_does_not_extend_the_first_hello() {
    let mut record = client_hello("example.test");
    record.extend([14, 0, 0, 0]);
    let record_len = (record.len() - 5) as u16;
    record[3..5].copy_from_slice(&record_len.to_be_bytes());
    let hello = decoded(&record).tls.unwrap();
    assert_eq!(hello.sni.as_deref(), Some("example.test"));
    assert_eq!(hello.alpn, ["h2", "http/1.1"]);
    assert!(!hello.partial);
}

#[test]
fn complete_hello_metadata_is_unchanged() {
    let record = client_hello("example.test");
    let hello = decoded(&record).tls.unwrap();
    assert_eq!(hello.sni.as_deref(), Some("example.test"));
    assert_eq!(hello.alpn, ["h2", "http/1.1"]);
    assert_eq!(hello.supported_versions, ["TLS 1.3", "TLS 1.2"]);
    assert!(!hello.partial);
}

fn hello_with_vectors(session: &[u8], suites: &[u8], compression: &[u8], extensions: &[(u16, &[u8])]) -> craft_anything_formats::pcap::TlsHello {
    let mut record = client_hello("unused.test");
    record.truncate(43);
    record.push(session.len() as u8);
    record.extend(session);
    record.extend((suites.len() as u16).to_be_bytes());
    record.extend(suites);
    record.push(compression.len() as u8);
    record.extend(compression);
    let mut ext = Vec::new();
    for (kind, bytes) in extensions {
        ext.extend(kind.to_be_bytes());
        ext.extend((bytes.len() as u16).to_be_bytes());
        ext.extend(*bytes);
    }
    record.extend((ext.len() as u16).to_be_bytes());
    record.extend(ext);
    let len = record.len();
    record[3..5].copy_from_slice(&((len - 5) as u16).to_be_bytes());
    record[6..9].copy_from_slice(&((len - 9) as u32).to_be_bytes()[1..]);
    decoded(&record).tls.unwrap()
}

#[test]
fn hello_vectors_must_have_protocol_lengths() {
    assert!(!hello_with_vectors(&[0; 32], &[0, 47], &[0], &[]).partial);
    assert!(hello_with_vectors(&[0; 33], &[0, 47], &[0], &[]).partial);
    for suites in [&[][..], &[0, 47, 0][..]] {
        assert!(hello_with_vectors(&[], suites, &[0], &[]).partial);
    }
    assert!(hello_with_vectors(&[], &[0, 47], &[], &[]).partial);
    for (kind, empty) in [(0, &[0, 0][..]), (16, &[0, 0][..]), (43, &[0][..])] {
        assert!(hello_with_vectors(&[], &[0, 47], &[0], &[(kind, empty)]).partial);
    }
}

#[test]
fn only_registered_grease_patterns_are_removed() {
    let hello = hello_with_vectors(&[], &[0, 47], &[0], &[(43, &[6, 0x1a, 0x2a, 0x4a, 0x4a, 3, 4])]);
    assert!(!hello.partial);
    assert_eq!(hello.supported_versions, ["0x1a2a", "TLS 1.3"]);
}

#[test]
fn duplicate_extensions_and_repeated_host_names_are_incomplete() {
    let hello = hello_with_vectors(&[], &[0, 47], &[0], &[(43, &[2, 3, 4]), (43, &[2, 3, 3])]);
    assert!(hello.partial);
    assert_eq!(hello.supported_versions, ["TLS 1.3"]);
    let hello = hello_with_vectors(&[], &[0, 47], &[0], &[(0, &[0, 8, 0, 0, 1, b'a', 0, 0, 1, b'b'])]);
    assert!(hello.partial);
    assert_eq!(hello.sni.as_deref(), Some("a"));
}

#[test]
fn the_alpn_display_limit_does_not_hide_a_damaged_tail() {
    let mut protocols = vec![0, 33];
    for _ in 0..16 {
        protocols.extend([1, b'x']);
    }
    protocols.push(3);
    let hello = hello_with_vectors(&[], &[0, 47], &[0], &[(16, &protocols)]);
    assert_eq!(hello.alpn.len(), 16);
    assert!(hello.partial);
    protocols[1] = 34;
    protocols.pop();
    protocols.extend([1, b'y']);
    let hello = hello_with_vectors(&[], &[0, 47], &[0], &[(16, &protocols)]);
    assert_eq!(hello.alpn.len(), 16);
    assert!(!hello.partial);
}
