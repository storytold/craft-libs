// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Checking S/MIME Digital signatures (#30): fixed vectors signed with RFC 9216's example
//! certificate for Alice (by OpenSSL, `testdata/`), and certificates of example.org authorities
//! and signers made here; and never a panic on any bytes.

use cms::cert::IssuerAndSerialNumber;
use cms::content_info::{CmsVersion, ContentInfo};
use cms::signed_data::{CertificateSet, EncapsulatedContentInfo, SignedData, SignerIdentifier, SignerInfo, SignerInfos};
use const_oid::ObjectIdentifier;
use der::asn1::{OctetString, SetOfVec};
use der::{Any, Decode, Encode};
use mailcraft_model::digital_signature::{Coverage, Verdict, Why};
use proptest::prelude::*;
use spki::AlgorithmIdentifierOwned;
use x509_cert::Certificate;
use x509_cert::attr::Attribute;

use crate::{Anchors, Checked, Context, Signed, check};

const CA: &[u8] = include_bytes!("../testdata/ca.rsa.der");
const ALICE: &[u8] = include_bytes!("../testdata/alice.sign.der");
const DETACHED: &[u8] = include_bytes!("../testdata/detached.eml");
const OPAQUE: &[u8] = include_bytes!("../testdata/opaque.eml");
const OPAQUE_BER: &[u8] = include_bytes!("../testdata/opaque-ber.eml");
const SHA1: &[u8] = include_bytes!("../testdata/sha1.eml");
const NO_ATTRIBUTES: &[u8] = include_bytes!("../testdata/noattr.eml");
const PSS: &[u8] = include_bytes!("../testdata/pss.eml");
const NO_CERTS: &[u8] = include_bytes!("../testdata/nocerts.eml");

/// 2026-11-09, within the RFC 9216 certificates' years.
const NOW: i64 = 1_794_182_400;

fn base64(text: &[u8]) -> Vec<u8> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = Vec::new();
    let (mut acc, mut bits) = (0u32, 0u32);
    for c in text {
        let Some(v) = ALPHABET.iter().position(|a| a == c) else { continue };
        acc = (acc << 6) | v as u32;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((acc >> bits) as u8);
        }
    }
    out
}

/// A message's body: what follows its first empty line.
fn body(message: &[u8]) -> &[u8] {
    let at = message.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    &message[at + 4..]
}

/// The two halves of a detached fixture: the signed part as it came, and the CMS.
fn halves(message: &[u8]) -> (Vec<u8>, Vec<u8>) {
    let text = String::from_utf8_lossy(message);
    let boundary = text.split("boundary=\"").nth(1).unwrap().split('"').next().unwrap().to_string();
    let delimiter = format!("\r\n--{boundary}");
    let body = String::from_utf8_lossy(body(message)).into_owned();
    let pieces: Vec<&str> = body.split(delimiter.as_str()).collect();
    // pieces[0] is the preamble up to the first delimiter line; the content follows its CRLF.
    let content = pieces[1].strip_prefix("\r\n").unwrap().as_bytes().to_vec();
    let signature_part = pieces[2].strip_prefix("\r\n").unwrap();
    (content, base64(body_of(signature_part.as_bytes())))
}

fn body_of(part: &[u8]) -> &[u8] {
    let at = part.windows(4).position(|w| w == b"\r\n\r\n").unwrap();
    &part[at + 4..]
}

fn anchors() -> Anchors {
    Anchors::new([CA])
}

fn checked_detached(message: &[u8], anchors: &Anchors, from: &str, now: i64, known: &[&[u8]]) -> Checked {
    let (content, signature) = halves(message);
    check(Signed::Detached { content: &content, signature: &signature }, &Context { anchors, from, now, known, coverage: Coverage::Whole }).unwrap()
}

fn checked_opaque(message: &[u8], anchors: &Anchors, from: &str) -> Checked {
    let cms = base64(body(message));
    check(Signed::Opaque(&cms), &Context { anchors, from, now: NOW, known: &[], coverage: Coverage::Whole }).unwrap()
}

#[test]
fn alices_detached_signature_is_verified() {
    let c = checked_detached(DETACHED, &anchors(), "alice@smime.example", NOW, &[]);
    assert_eq!(c.signature.verdict, Verdict::Verified, "{c:?}");
    assert_eq!(c.signature.why, None);
    let signer = c.signature.signer.clone().unwrap();
    assert_eq!(signer.name, "Alice Lovelace");
    assert_eq!(signer.addresses, vec!["alice@smime.example".to_string()]);
    assert_eq!(signer.issuer, "Sample LAMPS RSA Certification Authority");
    assert_eq!(signer.fingerprint.len(), 32 * 3 - 1);
    assert!(signer.not_before < NOW && NOW < signer.not_after);
    assert!(c.signature.signed_at.is_some(), "OpenSSL gives a signing time");
    assert!(!c.signature.revocation_checked);
    assert_eq!(c.certificate.as_deref(), Some(ALICE), "the certificate is worth keeping");
}

#[test]
fn the_address_is_compared_without_regard_to_case() {
    let c = checked_detached(DETACHED, &anchors(), "Alice@SMIME.example", NOW, &[]);
    assert_eq!(c.signature.verdict, Verdict::Verified);
}

#[test]
fn opaque_ber_pss_and_bare_signatures_are_verified() {
    for message in [OPAQUE, OPAQUE_BER] {
        let c = checked_opaque(message, &anchors(), "alice@smime.example");
        assert_eq!(c.signature.verdict, Verdict::Verified, "{c:?}");
    }
    for message in [NO_ATTRIBUTES, PSS] {
        let c = checked_detached(message, &anchors(), "alice@smime.example", NOW, &[]);
        assert_eq!(c.signature.verdict, Verdict::Verified, "{c:?}");
    }
}

#[test]
fn sha1_is_no_longer_safe() {
    let c = checked_detached(SHA1, &anchors(), "alice@smime.example", NOW, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Unchecked, Some(Why::WeakAlgorithm)));
    assert_eq!(c.certificate, None);
}

#[test]
fn a_signature_without_its_certificate_needs_one_kept_here() {
    let c = checked_detached(NO_CERTS, &anchors(), "alice@smime.example", NOW, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Unchecked, Some(Why::NoPublicKey)));
    let c = checked_detached(NO_CERTS, &anchors(), "alice@smime.example", NOW, &[ALICE]);
    assert_eq!(c.signature.verdict, Verdict::Verified);
}

#[test]
fn an_unknown_issuer_another_address_and_an_expired_certificate_are_not_verified() {
    let c = checked_detached(DETACHED, &Anchors::default(), "alice@smime.example", NOW, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::NotVerified, Some(Why::UnknownIssuer)));
    assert_eq!(c.certificate, None, "only a certificate that is trusted and valid is kept (#30 review 7)");
    let c = checked_detached(DETACHED, &anchors(), "bob@smime.example", NOW, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::NotVerified, Some(Why::AddressMismatch)));
    assert_eq!(c.certificate, None, "not kept for an address it isn't for");
    // 2060: past 2052.
    let c = checked_detached(DETACHED, &anchors(), "alice@smime.example", 2_840_140_800, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::NotVerified, Some(Why::Expired)));
    assert_eq!(c.certificate, None, "nor one out of date");
    let c = checked_detached(DETACHED, &anchors(), "alice@smime.example", NOW, &[]);
    assert!(c.certificate.is_some(), "a trusted one, valid now, for the sender, is");
}

#[test]
fn a_change_after_signing_is_seen() {
    let (mut content, signature) = halves(DETACHED);
    let at = content.windows(7).position(|w| w == b"Tuesday").unwrap();
    content[at] = b'W';
    let c = check(
        Signed::Detached { content: &content, signature: &signature },
        &Context { anchors: &anchors(), from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Changed, Some(Why::Altered)));
    // Opaque: the content inside the CMS.
    let mut cms = base64(body(OPAQUE));
    let at = cms.windows(7).position(|w| w == b"Tuesday").unwrap();
    cms[at] = b'W';
    let c =
        check(Signed::Opaque(&cms), &Context { anchors: &anchors(), from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Whole })
            .unwrap();
    assert_eq!(c.signature.verdict, Verdict::Changed);
    // The signature itself.
    let (content, mut signature) = halves(DETACHED);
    let last = signature.len() - 1;
    signature[last] ^= 1;
    let c = check(
        Signed::Detached { content: &content, signature: &signature },
        &Context { anchors: &anchors(), from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!(c.signature.verdict, Verdict::Changed);
}

#[test]
fn content_in_lf_lines_is_checked_in_its_canonical_form() {
    let (content, signature) = halves(DETACHED);
    let lf: Vec<u8> = String::from_utf8_lossy(&content).replace("\r\n", "\n").into_bytes();
    let c = check(
        Signed::Detached { content: &lf, signature: &signature },
        &Context { anchors: &anchors(), from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!(c.signature.verdict, Verdict::Verified);
}

#[test]
fn enveloped_data_is_no_digital_signature_and_junk_is_unreadable() {
    // ContentInfo { envelopedData, [0] { SEQUENCE {} } }.
    let enveloped = [0x30, 0x0f, 0x06, 0x09, 0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x03, 0xa0, 0x02, 0x30, 0x00];
    let context = Context { anchors: &Anchors::default(), from: "a@example.org", now: NOW, known: &[], coverage: Coverage::Whole };
    assert_eq!(check(Signed::Opaque(&enveloped), &context), None);
    let junk = check(Signed::Opaque(b"not a CMS"), &context).unwrap();
    assert_eq!((junk.signature.verdict, junk.signature.why), (Verdict::Unchecked, Some(Why::Unreadable)));
}

// ---- certificates made here ----

const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const ECDSA_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.2");
const ECDSA_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.10045.4.3.3");
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");

struct Made {
    der: Vec<u8>,
    key: rcgen::KeyPair,
    params: rcgen::CertificateParams,
}

fn params(name: &str) -> rcgen::CertificateParams {
    let mut p = rcgen::CertificateParams::new(Vec::<String>::new()).unwrap();
    p.distinguished_name = rcgen::DistinguishedName::new();
    p.distinguished_name.push(rcgen::DnType::CommonName, name);
    p.not_before = rcgen::date_time_ymd(2020, 1, 1);
    p.not_after = rcgen::date_time_ymd(2050, 1, 1);
    p
}

fn ca_params(name: &str) -> rcgen::CertificateParams {
    let mut p = params(name);
    p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    p.key_usages = vec![rcgen::KeyUsagePurpose::KeyCertSign, rcgen::KeyUsagePurpose::CrlSign];
    p
}

fn signer_params(name: &str, address: &str) -> rcgen::CertificateParams {
    let mut p = params(name);
    p.subject_alt_names = vec![rcgen::SanType::Rfc822Name(address.try_into().unwrap())];
    p.key_usages = vec![rcgen::KeyUsagePurpose::DigitalSignature];
    p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection];
    p
}

fn key(alg: &'static rcgen::SignatureAlgorithm) -> rcgen::KeyPair {
    rcgen::KeyPair::generate_for(alg).unwrap()
}

fn root(name: &str) -> Made {
    let key = key(&rcgen::PKCS_ECDSA_P256_SHA256);
    let params = ca_params(name);
    let der = params.self_signed(&key).unwrap().der().to_vec();
    Made { der, key, params }
}

fn issued(params: rcgen::CertificateParams, key: rcgen::KeyPair, by: &Made) -> Made {
    let issuer = rcgen::Issuer::new(by.params.clone(), &by.key);
    let der = params.signed_by(&key, &issuer).unwrap().der().to_vec();
    Made { der, key, params }
}

/// A detached CMS SignedData over `content` by `signer`, carrying `certs`.
fn sign(content: &[u8], signer: &Made, certs: &[&Made], p384: bool) -> Vec<u8> {
    sign_with(content, signer, certs, p384, |_| {})
}

/// [`sign`], with the signer's information changed by `tweak` after it is made.
fn sign_with(content: &[u8], signer: &Made, certs: &[&Made], p384: bool, tweak: impl Fn(&mut SignerInfo)) -> Vec<u8> {
    sign_data_with(content, signer, certs, p384, tweak, |_| {})
}

/// [`sign_with`], with the SignedData changed by `tweak_data` too.
fn sign_data_with(
    content: &[u8],
    signer: &Made,
    certs: &[&Made],
    p384: bool,
    tweak: impl Fn(&mut SignerInfo),
    tweak_data: impl Fn(&mut SignedData),
) -> Vec<u8> {
    let cert = Certificate::from_der(&signer.der).unwrap();
    let (digest_oid, sig_oid) = if p384 { (SHA384, ECDSA_SHA384) } else { (SHA256, ECDSA_SHA256) };
    let digest = if p384 { ring::digest::digest(&ring::digest::SHA384, content) } else { ring::digest::digest(&ring::digest::SHA256, content) };
    let attr = |oid: &str, value: Any| Attribute { oid: ObjectIdentifier::new_unwrap(oid), values: SetOfVec::try_from(vec![value]).unwrap() };
    let attrs: SetOfVec<Attribute> = SetOfVec::try_from(vec![
        attr("1.2.840.113549.1.9.3", Any::encode_from(&ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1")).unwrap()),
        attr("1.2.840.113549.1.9.4", Any::encode_from(&OctetString::new(digest.as_ref().to_vec()).unwrap()).unwrap()),
    ])
    .unwrap();
    let to_sign = attrs.to_der().unwrap();
    let alg = if p384 { &ring::signature::ECDSA_P384_SHA384_ASN1_SIGNING } else { &ring::signature::ECDSA_P256_SHA256_ASN1_SIGNING };
    let rng = ring::rand::SystemRandom::new();
    let pair = ring::signature::EcdsaKeyPair::from_pkcs8(alg, &signer.key.serialize_der(), &rng).unwrap();
    let signature = pair.sign(&rng, &to_sign).unwrap();
    let info = SignerInfo {
        version: CmsVersion::V1,
        sid: SignerIdentifier::IssuerAndSerialNumber(IssuerAndSerialNumber {
            issuer: cert.tbs_certificate().issuer().clone(),
            serial_number: cert.tbs_certificate().serial_number().clone(),
        }),
        digest_alg: AlgorithmIdentifierOwned { oid: digest_oid, parameters: None },
        signed_attrs: Some(attrs),
        signature_algorithm: AlgorithmIdentifierOwned { oid: sig_oid, parameters: None },
        signature: OctetString::new(signature.as_ref().to_vec()).unwrap(),
        unsigned_attrs: None,
    };
    let mut info = info;
    tweak(&mut info);
    let set: Vec<cms::cert::CertificateChoices> =
        certs.iter().map(|c| cms::cert::CertificateChoices::Certificate(Certificate::from_der(&c.der).unwrap())).collect();
    let mut data = SignedData {
        version: CmsVersion::V1,
        digest_algorithms: SetOfVec::try_from(vec![AlgorithmIdentifierOwned { oid: digest_oid, parameters: None }]).unwrap(),
        encap_content_info: EncapsulatedContentInfo { econtent_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1"), econtent: None },
        certificates: Some(CertificateSet::try_from(set).unwrap()),
        crls: None,
        signer_infos: SignerInfos::try_from(vec![info]).unwrap(),
    };
    tweak_data(&mut data);
    ContentInfo { content_type: ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.2"), content: Any::encode_from(&data).unwrap() }.to_der().unwrap()
}

fn verdict(cms: &[u8], content: &[u8], anchors: &Anchors, from: &str) -> (Verdict, Option<Why>) {
    let c = check(Signed::Detached { content, signature: cms }, &Context { anchors, from, now: NOW, known: &[], coverage: Coverage::Whole }).unwrap();
    (c.signature.verdict, c.signature.why)
}

const CONTENT: &[u8] = b"Content-Type: text/plain\r\n\r\nThe accounts are ready.\r\n";

#[test]
fn a_path_through_an_intermediate_authority_is_verified() {
    let root = root("Example Root");
    let intermediate = issued(ca_params("Example Mail CA"), key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    let ada = issued(signer_params("Ada", "ada@example.org"), key(&rcgen::PKCS_ECDSA_P256_SHA256), &intermediate);
    let cms = sign(CONTENT, &ada, &[&ada, &intermediate], false);
    let anchors = Anchors::new([&root.der]);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "ada@example.org"), (Verdict::Verified, None));
    // Without the intermediate authority's certificate there is no path.
    let alone = sign(CONTENT, &ada, &[&ada], false);
    assert_eq!(verdict(&alone, CONTENT, &anchors, "ada@example.org"), (Verdict::NotVerified, Some(Why::UnknownIssuer)));
    // A P-384 signer.
    let grace = issued(signer_params("Grace", "grace@example.org"), key(&rcgen::PKCS_ECDSA_P384_SHA384), &intermediate);
    let cms = sign(CONTENT, &grace, &[&grace, &intermediate], true);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "grace@example.org"), (Verdict::Verified, None));
}

#[test]
fn a_certificate_not_for_mail_or_signed_by_itself_is_not_verified() {
    let root = root("Example Root");
    let anchors = Anchors::new([&root.der]);
    let mut web = signer_params("Web", "web@example.org");
    web.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth];
    let web = issued(web, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    let cms = sign(CONTENT, &web, &[&web], false);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "web@example.org"), (Verdict::NotVerified, Some(Why::NotForEmail)));
    let own_key = key(&rcgen::PKCS_ECDSA_P256_SHA256);
    let own_params = signer_params("Self", "self@example.org");
    let own = Made { der: own_params.self_signed(&own_key).unwrap().der().to_vec(), key: own_key, params: own_params };
    let cms = sign(CONTENT, &own, &[&own], false);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "self@example.org"), (Verdict::NotVerified, Some(Why::SelfSigned)));
    // Unless the host trusts it itself.
    assert_eq!(verdict(&cms, CONTENT, &Anchors::new([&own.der]), "self@example.org"), (Verdict::Verified, None));
}

#[test]
fn an_expired_link_is_not_verified() {
    let root = root("Example Root");
    let mut old = signer_params("Old", "old@example.org");
    old.not_after = rcgen::date_time_ymd(2021, 1, 1);
    let old = issued(old, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    let cms = sign(CONTENT, &old, &[&old], false);
    assert_eq!(verdict(&cms, CONTENT, &Anchors::new([&root.der]), "old@example.org"), (Verdict::NotVerified, Some(Why::Expired)));
}

#[test]
fn a_certificate_from_another_authority_of_the_same_name_is_not_trusted() {
    let real = root("Example Root");
    let fake = root("Example Root");
    let ada = issued(signer_params("Ada", "ada@example.org"), key(&rcgen::PKCS_ECDSA_P256_SHA256), &fake);
    let cms = sign(CONTENT, &ada, &[&ada], false);
    assert_eq!(verdict(&cms, CONTENT, &Anchors::new([&real.der]), "ada@example.org"), (Verdict::NotVerified, Some(Why::UnknownIssuer)));
}

/// A certificate of the signer's issuer and serial number put in the message (anybody can put
/// one there) is not the signer's unless its key made the signature: it changes nothing.
#[test]
fn a_certificate_that_only_claims_the_signers_name_changes_nothing() {
    let real = root("Example Root");
    let fake = root("Example Root");
    let serial = |mut p: rcgen::CertificateParams| {
        p.serial_number = Some(rcgen::SerialNumber::from(vec![0x30, 0x01, 0x02]));
        p
    };
    let ada = issued(serial(signer_params("Ada", "ada@example.org")), key(&rcgen::PKCS_ECDSA_P256_SHA256), &real);
    let impostor = issued(serial(signer_params("Ada", "ada@example.org")), key(&rcgen::PKCS_ECDSA_P256_SHA256), &fake);
    let anchors = Anchors::new([&real.der]);
    // The impostor first: the signer is still found, and verified.
    let cms = sign(CONTENT, &ada, &[&impostor, &ada], false);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "ada@example.org"), (Verdict::Verified, None));
    // Changed content is put down to the signer that fits, and named.
    let changed = b"Content-Type: text/plain\r\n\r\nThe accounts are empty.\r\n";
    let c = check(
        Signed::Detached { content: changed, signature: &cms },
        &Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Changed, Some(Why::Altered)));
    let named = c.signature.signer.unwrap();
    assert_eq!(named.fingerprint, crate::cert::Cert::from_der(&ada.der).unwrap().fingerprint(), "the real signer, not the impostor");
    // With only the impostor's certificate, nobody is named as having signed it.
    let alone = sign(CONTENT, &ada, &[&impostor], false);
    let c = check(
        Signed::Detached { content: CONTENT, signature: &alone },
        &Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!(c.signature.verdict, Verdict::Changed);
    assert!(c.signature.signer.is_none());
    assert!(c.certificate.is_none(), "nothing of it is kept");
}

/// A signature that can't be read is "can't be checked", never "changed".
#[test]
fn a_malformed_signature_cant_be_checked() {
    let root = root("Example Root");
    let ada = issued(signer_params("Ada", "ada@example.org"), key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    let junk = sign_with(CONTENT, &ada, &[&ada], false, |info| info.signature = OctetString::new(vec![0xff; 12]).unwrap());
    assert_eq!(verdict(&junk, CONTENT, &Anchors::new([&root.der]), "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
    // An RSA signature algorithm for an elliptic-curve key: of different kinds.
    let crossed = sign_with(CONTENT, &ada, &[&ada], false, |info| {
        info.signature_algorithm = AlgorithmIdentifierOwned { oid: ObjectIdentifier::new_unwrap("1.2.840.113549.1.1.11"), parameters: None }
    });
    assert_eq!(verdict(&crossed, CONTENT, &Anchors::new([&root.der]), "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
}

/// A valid signature by a trusted signer over only part of a message (a wrapper's first part)
/// is never verified for the message.
#[test]
fn a_signature_over_part_of_a_message_is_not_verified_for_it() {
    let (content, signature) = halves(DETACHED);
    let anchors = anchors();
    let c = check(
        Signed::Detached { content: &content, signature: &signature },
        &Context { anchors: &anchors, from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Part },
    )
    .unwrap();
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::NotVerified, Some(Why::PartlySigned)));
    assert_eq!(c.signature.coverage, Coverage::Part);
    // A message from several people is nobody's alone: no address matches.
    let c = checked_detached(DETACHED, &anchors, "", NOW, &[]);
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::NotVerified, Some(Why::AddressMismatch)));
}

// ---- #30 review: what a path must keep (RFC 5280 §4.2, §6.1) ----

/// An authority's certificate for `name` issued by `by`, changed by `change` first.
fn authority(name: &str, by: &Made, change: impl FnOnce(&mut rcgen::CertificateParams)) -> Made {
    let mut p = ca_params(name);
    change(&mut p);
    issued(p, key(&rcgen::PKCS_ECDSA_P256_SHA256), by)
}

fn signer_by(name: &str, address: &str, by: &Made) -> Made {
    issued(signer_params(name, address), key(&rcgen::PKCS_ECDSA_P256_SHA256), by)
}

/// The verdict for a message by `leaf` carrying `chain`, against `root`.
fn path_verdict(leaf: &Made, chain: &[&Made], root: &Made, from: &str) -> (Verdict, Option<Why>) {
    let mut certs = vec![leaf];
    certs.extend_from_slice(chain);
    let cms = sign(CONTENT, leaf, &certs, false);
    verdict(&cms, CONTENT, &Anchors::new([&root.der]), from)
}

#[test]
fn name_constraints_on_mail_addresses_are_kept() {
    let root = root("Example Root");
    let corp = authority("Corp CA", &root, |p| {
        p.name_constraints = Some(rcgen::NameConstraints {
            permitted_subtrees: vec![rcgen::GeneralSubtree::Rfc822Name("corp.example".into())],
            excluded_subtrees: vec![rcgen::GeneralSubtree::Rfc822Name("ceo@corp.example".into())],
        })
    });
    let inside = signer_by("Ada", "ada@corp.example", &corp);
    assert_eq!(path_verdict(&inside, &[&corp], &root, "ada@corp.example"), (Verdict::Verified, None));
    let outside = signer_by("CEO", "ceo@bank.example", &corp);
    assert_eq!(path_verdict(&outside, &[&corp], &root, "ceo@bank.example"), (Verdict::NotVerified, Some(Why::Constrained)));
    let excluded = signer_by("CEO", "ceo@corp.example", &corp);
    assert_eq!(path_verdict(&excluded, &[&corp], &root, "ceo@corp.example"), (Verdict::NotVerified, Some(Why::Constrained)));
    // A leading dot: any host under the domain, not the domain itself.
    let sub = authority("Sub CA", &root, |p| {
        p.name_constraints = Some(rcgen::NameConstraints {
            permitted_subtrees: vec![rcgen::GeneralSubtree::Rfc822Name(".corp.example".into())],
            excluded_subtrees: vec![],
        })
    });
    let mail = signer_by("Ada", "ada@mail.corp.example", &sub);
    assert_eq!(path_verdict(&mail, &[&sub], &root, "ada@mail.corp.example"), (Verdict::Verified, None));
    let bare = signer_by("Ada", "ada@corp.example", &sub);
    assert_eq!(path_verdict(&bare, &[&sub], &root, "ada@corp.example"), (Verdict::NotVerified, Some(Why::Constrained)));
    assert!(crate::cert::mail_within("Ada@Corp.Example", "corp.example"));
    assert!(!crate::cert::mail_within("ada@evilcorp.example", ".corp.example"));
}

#[test]
fn name_constraints_on_directory_names_are_kept() {
    let root = root("Example Root");
    let mut corp_dn = rcgen::DistinguishedName::new();
    corp_dn.push(rcgen::DnType::OrganizationName, "Corp");
    let corp = authority("Corp CA", &root, |p| {
        p.name_constraints = Some(rcgen::NameConstraints {
            permitted_subtrees: vec![rcgen::GeneralSubtree::DirectoryName(corp_dn.clone())],
            excluded_subtrees: vec![],
        })
    });
    let mut inside = signer_params("Ada", "ada@corp.example");
    inside.distinguished_name = rcgen::DistinguishedName::new();
    inside.distinguished_name.push(rcgen::DnType::OrganizationName, "Corp");
    inside.distinguished_name.push(rcgen::DnType::CommonName, "Ada");
    let inside = issued(inside, key(&rcgen::PKCS_ECDSA_P256_SHA256), &corp);
    assert_eq!(path_verdict(&inside, &[&corp], &root, "ada@corp.example"), (Verdict::Verified, None));
    let outside = signer_by("Mallory", "mallory@corp.example", &corp);
    assert_eq!(path_verdict(&outside, &[&corp], &root, "mallory@corp.example"), (Verdict::NotVerified, Some(Why::Constrained)));
}

#[test]
fn an_excluded_directory_name_written_another_way_is_still_excluded() {
    let root = root("Example Root");
    let mut rogue = rcgen::DistinguishedName::new();
    rogue.push(rcgen::DnType::OrganizationName, "Rogue Unit");
    let ca = authority("Corp CA", &root, |p| {
        p.name_constraints =
            Some(rcgen::NameConstraints { permitted_subtrees: vec![], excluded_subtrees: vec![rcgen::GeneralSubtree::DirectoryName(rogue)] })
    });
    // The same name in another string type, case and spacing (RFC 5280 §7.1).
    let mut dressed = signer_params("Mallory", "mallory@corp.example");
    dressed.distinguished_name = rcgen::DistinguishedName::new();
    dressed.distinguished_name.push(rcgen::DnType::OrganizationName, rcgen::DnValue::PrintableString(" ROGUE   unit ".try_into().unwrap()));
    dressed.distinguished_name.push(rcgen::DnType::CommonName, "Mallory");
    let dressed = issued(dressed, key(&rcgen::PKCS_ECDSA_P256_SHA256), &ca);
    assert_eq!(path_verdict(&dressed, &[&ca], &root, "mallory@corp.example"), (Verdict::NotVerified, Some(Why::Constrained)));
    let other = signer_by("Ada", "ada@corp.example", &ca);
    assert_eq!(path_verdict(&other, &[&ca], &root, "ada@corp.example"), (Verdict::Verified, None));
}

#[test]
fn an_unknown_critical_extension_is_refused() {
    let root = root("Example Root");
    let mut odd = signer_params("Ada", "ada@example.org");
    let mut ext = rcgen::CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 1], vec![0x05, 0x00]);
    ext.set_criticality(true);
    odd.custom_extensions = vec![ext];
    let odd = issued(odd, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    assert_eq!(path_verdict(&odd, &[], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::Constrained)));
    // On an authority on the way too.
    let ca = authority("Odd CA", &root, |p| {
        let mut ext = rcgen::CustomExtension::from_oid_content(&[1, 3, 6, 1, 4, 1, 99999, 2], vec![0x05, 0x00]);
        ext.set_criticality(true);
        p.custom_extensions = vec![ext];
    });
    let ada = signer_by("Ada", "ada@example.org", &ca);
    assert_eq!(path_verdict(&ada, &[&ca], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::Constrained)));
}

#[test]
fn a_path_length_constraint_is_kept() {
    let root = root("Example Root");
    let ca = authority("Mail CA", &root, |p| p.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Constrained(0)));
    let rogue = authority("Rogue CA", &ca, |_| {});
    let leaf = signer_by("Ada", "ada@example.org", &rogue);
    assert_eq!(path_verdict(&leaf, &[&rogue, &ca], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::Constrained)));
    // Directly below it, a signer is fine.
    let direct = signer_by("Ada", "ada@example.org", &ca);
    assert_eq!(path_verdict(&direct, &[&ca], &root, "ada@example.org"), (Verdict::Verified, None));
}

#[test]
fn an_authority_not_for_mail_cant_vouch_for_a_mail_certificate() {
    let root = root("Example Root");
    let web = authority("Web CA", &root, |p| p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::ServerAuth]);
    let leaf = signer_by("Ada", "ada@example.org", &web);
    assert_eq!(path_verdict(&leaf, &[&web], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::Constrained)));
    let mail = authority("Mail CA", &root, |p| p.extended_key_usages = vec![rcgen::ExtendedKeyUsagePurpose::EmailProtection]);
    let leaf = signer_by("Ada", "ada@example.org", &mail);
    assert_eq!(path_verdict(&leaf, &[&mail], &root, "ada@example.org"), (Verdict::Verified, None));
}

#[test]
fn another_path_is_tried_when_the_first_doesnt_hold() {
    let root = root("Example Root");
    let shared = key(&rcgen::PKCS_ECDSA_P256_SHA256);
    let again = || rcgen::KeyPair::try_from(shared.serialize_der().as_slice()).unwrap();
    // The same authority, its key and name, in two certificates: one ran out, one is current.
    let mut old = ca_params("Mail CA");
    old.not_after = rcgen::date_time_ymd(2021, 1, 1);
    let old = issued(old, again(), &root);
    let current = issued(ca_params("Mail CA"), again(), &root);
    let leaf = signer_by("Ada", "ada@example.org", &current);
    assert_eq!(path_verdict(&leaf, &[&old, &current], &root, "ada@example.org"), (Verdict::Verified, None));
    assert_eq!(path_verdict(&leaf, &[&old], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::Expired)));
}

#[test]
fn the_search_for_a_path_keeps_to_its_budget() {
    let root = root("Example Root");
    let a = authority("A CA", &root, |_| {});
    let b = authority("B CA", &a, |_| {});
    let leaf = signer_by("Ada", "ada@example.org", &b);
    let pool: Vec<crate::cert::Cert> = [&b, &a].iter().map(|m| crate::cert::Cert::from_der(&m.der).unwrap()).collect();
    let leaf = crate::cert::Cert::from_der(&leaf.der).unwrap();
    let anchors = Anchors::new([&root.der]);
    let mut budget = 3;
    assert_eq!(crate::chain::trust(&leaf, &pool, &anchors, NOW, &mut budget), Ok(()));
    assert_eq!(budget, 0, "three links, three signatures");
    let mut budget = 2;
    assert_eq!(crate::chain::trust(&leaf, &pool, &anchors, NOW, &mut budget), Err(Why::TooMuch), "stops when it runs out, and says so");
    assert_eq!(crate::MAX_VERIFICATIONS, 32);
}

/// `cms` with its signers' information put in place of its own, in this order.
fn with_signers(cms: &[u8], infos: &[&[u8]]) -> Vec<u8> {
    let (info, _) = mailcraft_ber::read(cms).unwrap();
    let fields = mailcraft_ber::children(info.value).unwrap();
    let (sd, _) = mailcraft_ber::read(fields[1].value).unwrap();
    let parts = mailcraft_ber::children(sd.value).unwrap();
    let mut inner: Vec<u8> = parts[..parts.len() - 1].iter().flat_map(|p| p.whole.to_vec()).collect();
    inner.extend(tlv(0x31, &infos.concat()));
    let mut ci = fields[0].whole.to_vec();
    ci.extend(tlv(0xa0, &tlv(0x30, &inner)));
    tlv(0x30, &ci)
}

/// The first signer's information in `cms`, as it is written.
fn first_signer(cms: &[u8]) -> Vec<u8> {
    let (info, _) = mailcraft_ber::read(cms).unwrap();
    let fields = mailcraft_ber::children(info.value).unwrap();
    let (sd, _) = mailcraft_ber::read(fields[1].value).unwrap();
    let parts = mailcraft_ber::children(sd.value).unwrap();
    mailcraft_ber::children(parts.last().unwrap().value).unwrap()[0].whole.to_vec()
}

/// The second try of a signature without signed attributes, over the content in its canonical
/// form, is a verification of its own, paid from the budget (#30 review 6).
#[test]
fn the_canonical_try_is_paid_from_the_budget() {
    const LF: &[u8] = b"Content-Type: text/plain\n\nThe accounts are ready.\n";
    let root = root("Example Root");
    let mail = authority("Mail CA", &root, |_| {});
    let ada = signer_by("Ada", "ada@example.org", &mail);
    // Without its signed attributes, the signature (made over them) fails as it came and in the
    // canonical form alike.
    let cms = sign_with(LF, &ada, &[&ada], false, |i| i.signed_attrs = None);
    let anchors = Anchors::default();
    let context = Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &[], coverage: Coverage::Whole };
    let signed = Signed::Detached { content: LF, signature: &cms };
    let one = crate::verify::check_within(signed, &context, 1).unwrap();
    assert_eq!((one.signature.verdict, one.signature.why), (Verdict::Unchecked, Some(Why::TooMuch)), "one verification: no canonical try");
    let two = crate::verify::check_within(signed, &context, 2).unwrap();
    assert_eq!(two.signature.verdict, Verdict::Changed);
}

/// A signer's name and issuer are shown without format characters: no bidirectional override
/// reorders them and no zero-width space hides in them (#30 review 6).
#[test]
fn a_certificates_names_lose_their_format_characters() {
    let root = root("Example Root");
    let mail = authority("Mail\u{202e} CA\u{200b}", &root, |_| {});
    let ada = signer_by("Ada\u{202e}gpj.exe\u{200b}", "ada@example.org", &mail);
    let cms = sign(CONTENT, &ada, &[&ada], false);
    let anchors = Anchors::default();
    let context = Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &[], coverage: Coverage::Whole };
    let c = check(Signed::Detached { content: CONTENT, signature: &cms }, &context).unwrap();
    let signer = c.signature.signer.unwrap();
    assert_eq!(signer.name, "Adagpj.exe");
    assert!(signer.issuer.contains("Mail CA") && !signer.issuer.contains(['\u{202e}', '\u{200b}']), "{:?}", signer.issuer);
}

#[test]
fn a_message_with_too_much_to_check_is_said_so() {
    let root = root("Example Root");
    // Thirty authorities of the right name with the wrong keys come before the one that signed:
    // the first signer's path takes all the budget, and the second signer is left with none.
    let mut known: Vec<Made> = (0..30).map(|_| authority("Mail CA", &root, |_| {})).collect();
    let mail = authority("Mail CA", &root, |_| {});
    let ada = signer_by("Ada", "ada@example.org", &mail);
    // A signer this computer trusts itself: no link to check, only its signature.
    let trusted = issued(signer_params("Ada", "ada@example.org"), key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    known.push(mail);
    let first = sign(CONTENT, &ada, &[&ada], false);
    let second = sign(CONTENT, &trusted, &[], false);
    let cms = with_signers(&first, &[&first_signer(&first), &first_signer(&second)]);
    let known: Vec<&[u8]> = known.iter().map(|m| m.der.as_slice()).chain([trusted.der.as_slice()]).collect();
    let anchors = Anchors::new([&root.der, &trusted.der]);
    let c = check(
        Signed::Detached { content: CONTENT, signature: &cms },
        &Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &known, coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Unchecked, Some(Why::TooMuch)));
    // With the budget to spare, the trusted signer alone is verified.
    let alone = with_signers(&first, &[&first_signer(&second)]);
    let c = check(
        Signed::Detached { content: CONTENT, signature: &alone },
        &Context { anchors: &anchors, from: "ada@example.org", now: NOW, known: &known, coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!((c.signature.verdict, c.signature.why), (Verdict::Verified, None));
}

#[test]
fn an_authoritys_certificate_never_signs_mail_and_the_matching_address_is_said() {
    let root = root("Example Root");
    let mut ca_signer = signer_params("Ada", "ada@example.org");
    ca_signer.is_ca = rcgen::IsCa::Ca(rcgen::BasicConstraints::Unconstrained);
    let ca_signer = issued(ca_signer, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    assert_eq!(path_verdict(&ca_signer, &[], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::NotForEmail)));
    let mut two = signer_params("Ada", "other@example.org");
    two.subject_alt_names.push(rcgen::SanType::Rfc822Name("ada@example.org".try_into().unwrap()));
    let two = issued(two, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    let cms = sign(CONTENT, &two, &[&two], false);
    let anchors = Anchors::new([&root.der]);
    let c = check(
        Signed::Detached { content: CONTENT, signature: &cms },
        &Context { anchors: &anchors, from: "ADA@example.org", now: NOW, known: &[], coverage: Coverage::Whole },
    )
    .unwrap();
    assert_eq!(c.signature.verdict, Verdict::Verified);
    assert_eq!(c.signature.signer.unwrap().addresses.first().map(String::as_str), Some("ada@example.org"));
}

#[test]
fn an_address_is_compared_as_it_is_written() {
    let root = root("Example Root");
    let mut spaced = signer_params("Ada", "ada@example.org");
    spaced.subject_alt_names = vec![rcgen::SanType::Rfc822Name("ada@example.org ".try_into().unwrap())];
    let spaced = issued(spaced, key(&rcgen::PKCS_ECDSA_P256_SHA256), &root);
    assert_eq!(path_verdict(&spaced, &[], &root, "ada@example.org"), (Verdict::NotVerified, Some(Why::AddressMismatch)));
    let ada = crate::cert::Cert::from_der(&signer_by("Ada", "ada@example.org", &root).der).unwrap();
    assert!(ada.is_for("ADA@EXAMPLE.ORG"));
    assert!(!ada.is_for("ada@example.org\n"));
    assert!(!ada.is_for(""));
}

// ---- #30 review: SignedData read strictly ----

/// A DER element of `tag` holding `content`.
fn tlv(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    let n = content.len();
    if n < 0x80 {
        out.push(n as u8);
    } else if n < 0x100 {
        out.extend([0x81, n as u8]);
    } else {
        out.extend([0x82, (n >> 8) as u8, n as u8]);
    }
    out.extend_from_slice(content);
    out
}

/// `cms` with `extra` put into its SignedData just before the signers.
fn smuggled(cms: &[u8], extra: &[u8]) -> Vec<u8> {
    let (info, _) = mailcraft_ber::read(cms).unwrap();
    let fields = mailcraft_ber::children(info.value).unwrap();
    let (sd, _) = mailcraft_ber::read(fields[1].value).unwrap();
    let parts = mailcraft_ber::children(sd.value).unwrap();
    let mut inner = Vec::new();
    for (i, p) in parts.iter().enumerate() {
        if i == parts.len() - 1 {
            inner.extend_from_slice(extra);
        }
        inner.extend_from_slice(p.whole);
    }
    let sd = tlv(0x30, &inner);
    let mut ci = fields[0].whole.to_vec();
    ci.extend(tlv(0xa0, &sd));
    tlv(0x30, &ci)
}

#[test]
fn signed_data_with_anything_else_in_it_cant_be_checked() {
    let root = root("Example Root");
    let ada = signer_by("Ada", "ada@example.org", &root);
    let cms = sign(CONTENT, &ada, &[&ada], false);
    let anchors = Anchors::new([&root.der]);
    assert_eq!(verdict(&cms, CONTENT, &anchors, "ada@example.org"), (Verdict::Verified, None));
    // A field smuggled in, an OCTET STRING with other content, is refused, not passed over.
    let forged = smuggled(&cms, &tlv(0x04, b"<p>Wire 10000 EUR to Mallory</p>"));
    assert_eq!(verdict(&forged, CONTENT, &anchors, "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
    // And after the signers, where nothing may follow.
    let signers = first_signer(&cms);
    let mut after = with_signers(&cms, &[&signers]);
    assert_eq!(verdict(&after, CONTENT, &anchors, "ada@example.org"), (Verdict::Verified, None));
    after = {
        let (info, _) = mailcraft_ber::read(&after).unwrap();
        let fields = mailcraft_ber::children(info.value).unwrap();
        let (sd, _) = mailcraft_ber::read(fields[1].value).unwrap();
        let mut inner = sd.value.to_vec();
        inner.extend(tlv(0x04, b"after"));
        let mut ci = fields[0].whole.to_vec();
        ci.extend(tlv(0xa0, &tlv(0x30, &inner)));
        tlv(0x30, &ci)
    };
    assert_eq!(verdict(&after, CONTENT, &anchors, "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
    // A version that isn't one, and a detached signature that carries content too.
    let versioned = sign_data_with(CONTENT, &ada, &[&ada], false, |_| {}, |d| d.version = CmsVersion::V2);
    assert_eq!(verdict(&versioned, CONTENT, &anchors, "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
    let carrying = sign_data_with(
        CONTENT,
        &ada,
        &[&ada],
        false,
        |_| {},
        |d| d.encap_content_info.econtent = Some(Any::encode_from(&OctetString::new(b"other".to_vec()).unwrap()).unwrap()),
    );
    assert_eq!(verdict(&carrying, CONTENT, &anchors, "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
    // What mail signs is id-data.
    let other_type = sign_data_with(
        CONTENT,
        &ada,
        &[&ada],
        false,
        |_| {},
        |d| d.encap_content_info.econtent_type = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.16.1.4"),
    );
    assert_eq!(verdict(&other_type, CONTENT, &anchors, "ada@example.org"), (Verdict::Unchecked, Some(Why::Unreadable)));
}

#[test]
fn an_opaque_signatures_content_is_given_for_showing() {
    let cms = base64(body(OPAQUE));
    let content = mailcraft_ber::signed_content(&cms).unwrap();
    assert!(String::from_utf8_lossy(&content).contains("Hello Bob"), "the signed MIME entity");
    let ber = base64(body(OPAQUE_BER));
    assert_eq!(mailcraft_ber::signed_content(&ber), Some(content));
    let (_, detached) = halves(DETACHED);
    assert_eq!(mailcraft_ber::signed_content(&detached), None);
}

#[test]
fn the_ber_reader_keeps_der_as_it_is_and_joins_pieces() {
    assert_eq!(mailcraft_ber::to_der(CA).as_deref(), Some(CA));
    // SEQUENCE (indefinite) { OCTET STRING in pieces "ab" "c" }.
    let ber = [0x30, 0x80, 0x24, 0x80, 0x04, 0x02, b'a', b'b', 0x04, 0x01, b'c', 0x00, 0x00, 0x00, 0x00];
    assert_eq!(mailcraft_ber::to_der(&ber), Some(vec![0x30, 0x05, 0x04, 0x03, b'a', b'b', b'c']));
    // A definite length inside, written longer than it need be (a certificate's, say, which its
    // signature covers), is kept as it was written.
    let ber = [0x30, 0x80, 0x30, 0x81, 0x03, 0x02, 0x01, 0x05, 0x00, 0x00];
    assert_eq!(mailcraft_ber::to_der(&ber), Some(vec![0x30, 0x06, 0x30, 0x81, 0x03, 0x02, 0x01, 0x05]));
    // Nesting past the bound is refused, not followed.
    let deep: Vec<u8> = std::iter::repeat_n([0x30u8, 0x80], 200).flatten().chain(std::iter::repeat_n(0u8, 400)).collect();
    assert_eq!(mailcraft_ber::to_der(&deep), None);
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(512))]

    #[test]
    fn any_bytes_never_panic(bytes in proptest::collection::vec(any::<u8>(), 0..4096)) {
        let anchors = Anchors::new([CA]);
        let context = Context { anchors: &anchors, from: "alice@smime.example", now: NOW, known: &[&bytes], coverage: Coverage::Whole };
        let _ = check(Signed::Opaque(&bytes), &context);
        let _ = check(Signed::Detached { content: b"x", signature: &bytes }, &context);
        let _ = mailcraft_ber::to_der(&bytes);
        let _ = Anchors::new([&bytes]);
    }

    #[test]
    fn a_damaged_cms_never_panics(flip in 0usize..3000, byte in any::<u8>(), cut in 0usize..3000) {
        let mut cms = base64(body(OPAQUE_BER));
        if let Some(b) = cms.get_mut(flip) {
            *b = byte;
        }
        cms.truncate(cut);
        let anchors = Anchors::new([CA]);
        let context = Context { anchors: &anchors, from: "alice@smime.example", now: NOW, known: &[], coverage: Coverage::Whole };
        let _ = check(Signed::Opaque(&cms), &context);
        let (content, signature) = halves(DETACHED);
        let mut signature = signature;
        if let Some(b) = signature.get_mut(flip) {
            *b = byte;
        }
        let _ = check(Signed::Detached { content: &content, signature: &signature }, &context);
    }
}
