use craft_pkcs7::der::{self, tag};
use craft_pkcs7::inspection::InspectionError;
use craft_pkcs7::inspection::cms::{SignedDataView, SignerInfoView};

fn fixture(key_id: bool, attrs: bool) -> (Vec<u8>, Vec<u8>) {
    let digest = der::seq(&[&der::oid("2.16.840.1.101.3.4.2.1")]);
    let sig_alg = der::seq(&[&der::oid("1.2.840.113549.1.1.1"), &der::tlv(tag::NULL, &[])]);
    let identifier = if key_id { der::tlv(tag::ctx_prim(0), &[7, 8]) } else { der::seq(&[&der::seq(&[]), &der::int(7)]) };
    let attributes = if attrs {
        let content = der::seq(&[&der::oid("1.2.840.113549.1.9.3"), &der::set_of(&[&der::oid("1.2.840.113549.1.7.1")])]);
        let digest_attr = der::seq(&[&der::oid("1.2.840.113549.1.9.4"), &der::set_of(&[&der::octets(&[6; 32])])]);
        der::tlv(tag::ctx(0), &[content, digest_attr].concat())
    } else {
        vec![]
    };
    let signer = der::seq(&[&der::int(if key_id { 3 } else { 1 }), &identifier, &digest, &attributes, &sig_alg, &der::octets(&[1, 2, 3])]);
    let content = der::seq(&[&der::oid("1.2.840.113549.1.7.1"), &der::explicit(0, &der::octets(b"hello"))]);
    let signed_data = der::seq(&[&der::int(1), &der::set_of(&[&digest]), &content, &der::set_of(&[&signer])]);
    (der::seq(&[&der::oid("1.2.840.113549.1.7.2"), &der::explicit(0, &signed_data)]), signer)
}

#[test]
fn cms_fields_retain_encodings_and_both_identifier_forms() {
    for key_id in [false, true] {
        for attrs in [false, true] {
            let (cms, signer) = fixture(key_id, attrs);
            let (mut view, rest) = SignedDataView::parse(&cms).unwrap();
            assert!(rest.is_empty());
            assert_eq!(view.content_info.raw().as_ptr(), cms.as_ptr());
            assert_eq!(view.version.value(), &[1]);
            let signer_set = view.remaining_fields.next_element().unwrap().unwrap();
            let (mut si, rest) = SignerInfoView::parse(signer_set.value()).unwrap();
            assert_eq!(si.signer_info.raw(), signer);
            assert!(rest.is_empty());
            assert_eq!(si.identifier.tag(), if key_id { 0x80 } else { 0x30 });
            assert_eq!(si.signed_attributes.is_some(), attrs);
            assert_eq!(si.signature.value(), &[1, 2, 3]);
            assert_eq!(si.remaining_fields.next_element(), Ok(None));
            assert_eq!(view.remaining_fields.next_element(), Ok(None));
            assert_eq!(view.remaining_explicit_fields.next_element(), Ok(None));
            assert_eq!(view.remaining_content_info_fields.next_element(), Ok(None));
        }
    }
}

#[test]
fn signer_prefix_does_not_hide_optional_tail_or_consume_next_signer() {
    let (_, signer) = fixture(false, true);
    let parsed = der::Tlv::parse_all(&signer).unwrap();
    let with_tail = der::seq(&[parsed.value, &[0xa1]]);
    let joined = [with_tail, signer.clone()].concat();
    let (mut view, rest) = SignerInfoView::parse(&joined).unwrap();
    assert_eq!(rest, signer);
    assert_eq!(view.remaining_fields.next_element(), Err(InspectionError::Malformed));
    assert_eq!(view.remaining_fields.remaining(), &[0xa1]);
}

// Ordinary optional sets and complete trailing TLVs, never interpreted as trust evidence here.
fn optional_fixture(certificate_count: usize, with_crl: bool, tails: bool) -> (Vec<u8>, Vec<u8>, Vec<u8>) {
    let certificate = craft_pkcs7::x509::load_certificates(include_bytes!("../data/ca.crt.pem")).unwrap().remove(0).raw;
    let (base, signer) = fixture(false, true);
    let (view, _) = SignedDataView::parse(&base).unwrap();
    let certificates = der::tlv(tag::ctx(0), &certificate.repeat(certificate_count));
    let crl = if with_crl { der::tlv(tag::ctx(1), &der::seq(&[])) } else { vec![] };
    let tail = if tails { der::tlv(tag::NULL, &[]) } else { vec![] };
    let signed_data = der::seq(&[
        view.version.raw(),
        view.digest_algorithms.raw(),
        view.encapsulated_content.raw(),
        &certificates,
        &crl,
        &der::set_of(&[&signer]),
        &tail,
    ]);
    let wrapped = der::tlv(tag::ctx(0), &[signed_data, tail.clone()].concat());
    let content = der::seq(&[&der::oid("1.2.840.113549.1.7.2"), &wrapped, &tail]);
    ([content, tail].concat(), certificate, crl)
}

#[test]
fn optional_certificate_crl_sets_and_all_tails_remain_visible() {
    for with_crl in [false, true] {
        let (bytes, cert, crl) = optional_fixture(2, with_crl, true);
        let (mut view, rest) = SignedDataView::parse(&bytes).unwrap();
        assert_eq!(rest, &[5, 0]);
        let certs = view.remaining_fields.next_element().unwrap().unwrap();
        assert_eq!(certs.tag(), tag::ctx(0));
        assert_eq!(certs.value(), cert.repeat(2));
        if with_crl {
            assert_eq!(view.remaining_fields.next_element().unwrap().unwrap().raw(), crl);
        }
        assert_eq!(view.remaining_fields.next_element().unwrap().unwrap().tag(), tag::SET);
        for cursor in [&mut view.remaining_fields, &mut view.remaining_explicit_fields, &mut view.remaining_content_info_fields] {
            assert_eq!(cursor.next_element().unwrap().unwrap().raw(), &[5, 0]);
            assert_eq!(cursor.next_element(), Ok(None));
        }
    }
}

#[test]
fn signer_unsigned_attributes_remain_raw_and_separate_from_signed_attributes() {
    let (_, signer) = fixture(true, true);
    let fields = der::Tlv::parse_all(&signer).unwrap();
    let unsigned = der::tlv(tag::ctx(1), &der::seq(&[]));
    let bytes = der::seq(&[fields.value, &unsigned]);
    let (mut view, rest) = SignerInfoView::parse(&bytes).unwrap();
    assert!(rest.is_empty());
    assert_eq!(view.signed_attributes.unwrap().tag(), tag::ctx(0));
    assert_eq!(view.remaining_fields.next_element().unwrap().unwrap().raw(), unsigned);
    assert_eq!(view.remaining_fields.next_element(), Ok(None));
}
