//! Benign wire parity and boundary tests; no signature/trust decisions are made by these views.
use craft_pkcs7::der::{self, tag};
use craft_pkcs7::inspection::{CertificateView, Cursor, InspectionError, MAX_CHILDREN, MAX_DEPTH};

#[test]
fn cursor_keeps_exact_bytes_and_length_notes() {
    for (encoded, note) in [(vec![4, 3, 1, 2, 3], false), (vec![4, 0x81, 3, 1, 2, 3], true), (vec![4, 0x82, 0, 3, 1, 2, 3], true)] {
        let mut cursor = Cursor::new(&encoded);
        let element = cursor.next_element().unwrap().unwrap();
        assert_eq!(element.raw(), encoded);
        assert_eq!(element.raw().as_ptr(), encoded.as_ptr());
        assert_eq!(element.value(), &[1, 2, 3]);
        assert_eq!(element.non_minimal_length(), note);
        assert_eq!(cursor.next_element(), Ok(None));
    }
    let canonical = der::octets(&[7; 128]);
    assert!(!Cursor::new(&canonical).next_element().unwrap().unwrap().non_minimal_length());
}

#[test]
fn cursor_distinguishes_completion_and_sticky_partial_results() {
    let bytes = [5, 0, 4]; // One complete NULL followed by an incomplete header.
    let mut cursor = Cursor::new(&bytes);
    assert_eq!(cursor.next_element().unwrap().unwrap().tag(), tag::NULL);
    assert_eq!(cursor.next_element(), Err(InspectionError::Malformed));
    assert_eq!(cursor.next_element(), Err(InspectionError::Malformed));
    assert_eq!(cursor.remaining(), &[4]);
    assert_eq!(Cursor::new(&[0x30, 0x80, 0, 0]).next_element(), Err(InspectionError::Malformed));
    assert_eq!(Cursor::new(&[0x30, 0]).next_element().unwrap().unwrap().raw(), &[0x30, 0]);
}

#[test]
fn sibling_limit_allows_exact_completion_without_allocating_a_child_list() {
    let mut bytes = [5, 0].repeat(MAX_CHILDREN);
    let mut cursor = Cursor::new(&bytes);
    for _ in 0..MAX_CHILDREN {
        assert!(cursor.next_element().unwrap().is_some());
    }
    assert_eq!(cursor.next_element(), Ok(None));
    bytes.extend_from_slice(&[5, 0]);
    let mut cursor = Cursor::new(&bytes);
    for _ in 0..MAX_CHILDREN {
        cursor.next_element().unwrap();
    }
    assert_eq!(cursor.next_element(), Err(InspectionError::ChildLimit));
    assert_eq!(cursor.next_element(), Err(InspectionError::ChildLimit));
    assert_eq!(cursor.remaining(), &[5, 0]);
}

#[test]
fn nested_cursor_depth_is_explicitly_bounded() {
    let mut bytes = der::seq(&[]);
    for _ in 0..MAX_DEPTH {
        bytes = der::seq(&[&bytes]);
    }
    let mut element = Cursor::new(&bytes).next_element().unwrap().unwrap();
    for _ in 0..MAX_DEPTH {
        element = element.children().unwrap().next_element().unwrap().unwrap();
    }
    assert!(matches!(element.children(), Err(InspectionError::DepthLimit)));
}

fn certificate(version: bool, tail: &[u8]) -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>) {
    // Unknown algorithms are still inspectable. These are wire fixtures, not signed certificates.
    let algorithm = der::seq(&[&der::oid("1.2.3.4"), &der::tlv(tag::NULL, &[])]);
    let name = der::seq(&[]);
    let spki = der::seq(&[&algorithm, &der::bit_string(&[1, 2, 3])]);
    let version_field = if version { der::explicit(0, &der::int(2)) } else { vec![] };
    let tbs = der::seq(&[&version_field, &der::int(7), &algorithm, &name, &der::seq(&[]), &name, &spki, tail]);
    let cert = der::seq(&[&tbs, &algorithm, &der::bit_string(&[9, 8, 7])]);
    (cert, tbs, algorithm, spki)
}

#[test]
fn certificate_fields_are_borrowed_exact_and_do_not_consume_trailing_input() {
    for version in [false, true] {
        let (mut bytes, tbs, algorithm, spki) = certificate(version, &[]);
        let len = bytes.len();
        bytes.extend_from_slice(&[5, 0]);
        let (mut view, rest) = CertificateView::parse(&bytes).unwrap();
        assert_eq!(view.certificate.raw(), &bytes[..len]);
        assert_eq!(view.certificate.raw().as_ptr(), bytes.as_ptr());
        assert_eq!(view.tbs.raw(), tbs);
        assert_eq!(view.signed_algorithm.raw(), algorithm);
        assert_eq!(view.signature_algorithm.raw(), algorithm);
        assert_eq!(view.spki.raw(), spki);
        assert_eq!(view.serial.value(), &[7]);
        assert_eq!(view.signature_bits.value(), &[0, 9, 8, 7]);
        assert_eq!(view.version.is_some(), version);
        assert_eq!(rest, &[5, 0]);
        assert_eq!(view.remaining_tbs_fields.next_element(), Ok(None));
        assert_eq!(view.remaining_certificate_fields.next_element(), Ok(None));
    }
}

#[test]
fn required_fields_do_not_hide_optional_tail_diagnostics() {
    let (bytes, _, _, _) = certificate(false, &[0xa3]);
    let (mut view, _) = CertificateView::parse(&bytes).unwrap();
    assert_eq!(view.remaining_tbs_fields.remaining(), &[0xa3]);
    assert_eq!(view.remaining_tbs_fields.next_element(), Err(InspectionError::Malformed));
    // This partial wire view is deliberately not accepted as a validated certificate.
    assert!(craft_pkcs7::Certificate::parse(&bytes).is_err());
}

#[test]
fn existing_certificate_fixture_retains_signed_components() {
    let cert = craft_pkcs7::x509::load_certificates(include_bytes!("../data/ca.crt.pem")).unwrap().remove(0);
    let (view, rest) = CertificateView::parse(&cert.raw).unwrap();
    assert!(rest.is_empty());
    let parts = cert.clone().into_parts();
    assert_eq!(view.tbs.raw(), parts.tbs);
    assert_eq!(view.signature_algorithm.raw(), parts.sig_alg);
    assert_eq!(view.signature_bits.value().first(), Some(&0));
    assert_eq!(&view.signature_bits.value()[1..], parts.signature);
    assert_eq!(view.issuer.raw(), parts.issuer.raw);
    assert_eq!(view.subject.raw(), parts.subject.raw);
    assert_eq!(view.spki.raw(), parts.public_key.spki());
}
