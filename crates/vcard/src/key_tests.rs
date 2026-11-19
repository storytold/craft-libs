//! `KEY`: certificates and OpenPGP public keys read in every version's form, known by their bytes,
//! secret key material never kept, written back as 3.0, capped, and never a panic.

use crate::{Contact, ContactKey, KeyKind};
use craft_mime::base64::STANDARD;
use proptest::prelude::*;

use crate::{parse, write};

/// Made-up bytes with the outline of a DER certificate: SEQUENCE { SEQUENCE { INTEGER 7 },
/// SEQUENCE {}, BIT STRING }.
const DER: &[u8] = &[0x30, 0x0a, 0x30, 0x03, 0x02, 0x01, 0x07, 0x30, 0x00, 0x03, 0x01, 0x00];
/// Made-up OpenPGP packets: an old-format public-key packet (tag 6) of six bytes.
const PGP: &[u8] = &[0x99, 0x00, 0x06, 0x04, 0x5e, 0x00, 0x00, 0x01, 0x16];

/// One DER element, its length in the short or the long form.
fn der(tag: u8, content: &[u8]) -> Vec<u8> {
    let mut out = vec![tag];
    match content.len() {
        n if n < 0x80 => out.push(n as u8),
        n if n < 0x100 => out.extend([0x81, n as u8]),
        n => out.extend([0x82, (n >> 8) as u8, n as u8]),
    }
    out.extend(content);
    out
}

/// A certificate's outline whose signature is `signature`.
fn certificate(serial: &[u8], signature: &[u8]) -> Vec<u8> {
    let signed = der(0x30, &der(0x02, serial));
    der(0x30, &[signed, der(0x30, &[]), der(0x03, signature)].concat())
}

/// An ASCII armour of OpenPGP packets, with a header and a checksum line.
fn armoured(packets: &[u8]) -> String {
    format!("-----BEGIN PGP PUBLIC KEY BLOCK-----\nComment: made up\n\n{}\n=AAAA\n-----END PGP PUBLIC KEY BLOCK-----", STANDARD.encode(packets))
}

fn pem(der: &[u8]) -> String {
    format!("-----BEGIN CERTIFICATE-----\n{}\n-----END CERTIFICATE-----\n", STANDARD.encode(der))
}

/// `text` with `\`, `;`, `,` and line breaks escaped as a vCard value.
fn escaped(text: &str) -> String {
    crate::text::escape(text)
}

fn keys(version: &str, properties: &str) -> Vec<ContactKey> {
    let got = parse(&format!("BEGIN:VCARD\r\nVERSION:{version}\r\nFN:Ada\r\n{properties}\r\nEND:VCARD\r\n"));
    assert_eq!(got.len(), 1, "{properties}");
    got.into_iter().next().map(|c| c.keys).unwrap_or_default()
}

fn x509(data: &[u8]) -> ContactKey {
    ContactKey { kind: KeyKind::X509, data: data.to_vec(), ..Default::default() }
}

fn openpgp(data: &[u8]) -> ContactKey {
    ContactKey { kind: KeyKind::OpenPgp, data: data.to_vec(), ..Default::default() }
}

fn at(uri: &str, media_type: &str) -> ContactKey {
    ContactKey { kind: KeyKind::Uri, uri: uri.into(), media_type: media_type.into(), ..Default::default() }
}

#[test]
fn reads_2_1_keys() {
    let (der, pgp) = (STANDARD.encode(DER), STANDARD.encode(PGP));
    let (head, tail) = der.split_at(4);
    let got = keys("2.1", &format!("KEY;X509;ENCODING=BASE64:{head}\r\n  {tail}\r\n\r\nKEY;PGP;BASE64:{pgp}"));
    assert_eq!(got, [x509(DER), openpgp(PGP)]);
}

#[test]
fn reads_3_0_keys() {
    let (der, pgp) = (STANDARD.encode(DER), STANDARD.encode(PGP));
    let (head, tail) = pgp.split_at(5);
    let armour = armoured(PGP);
    let got = keys(
        "3.0",
        &format!(
            "KEY;ENCODING=b;TYPE=X509:{der}\r\nkey;type=pgp;encoding=B:{head}\r\n {tail}\r\nKEY;TYPE=PGP:{}\r\nKEY;VALUE=uri;TYPE=X509:ldap://ldap.example.org/cn=Ada",
            escaped(&format!("{armour}\nWhat follows the armour is not kept"))
        ),
    );
    assert_eq!(got, [x509(DER), openpgp(PGP), openpgp(armour.as_bytes()), at("ldap://ldap.example.org/cn=Ada", ContactKey::X509_MEDIA_TYPE)]);
}

#[test]
fn reads_4_0_keys() {
    let (der, pgp) = (STANDARD.encode(DER), STANDARD.encode(PGP));
    let got = keys(
        "4.0",
        &format!(
            "KEY:data:application/pgp-keys;base64,{pgp}\r\nKEY:data:application/pkix-cert;base64\\,{der}\r\nKEY:data:application/pgp-keys,%99%00%06%04%5E%00%00%01%16\r\n\
KEY;MEDIATYPE=application/pgp-keys:https://keys.example.org/ada.asc\r\nKEY;TYPE=work:openpgp4fpr:ABCDEF0123456789ABCDEF0123456789ABCDEF01\r\nKEY:http://example.org/ada.cer"
        ),
    );
    assert_eq!(
        got,
        [
            openpgp(PGP),
            x509(DER),
            // (The same key twice is kept once.)
            at("https://keys.example.org/ada.asc", ContactKey::OPENPGP_MEDIA_TYPE),
            at("openpgp4fpr:ABCDEF0123456789ABCDEF0123456789ABCDEF01", ""),
            at("http://example.org/ada.cer", ""),
        ]
    );
}

#[test]
fn a_key_without_a_type_is_known_by_its_bytes() {
    let got = keys(
        "3.0",
        &format!(
            "KEY;ENCODING=b:{}\r\nKEY;ENCODING=b:{}\r\nKEY:{}\r\nKEY:data:,{}\r\nKEY;ENCODING=b:{}",
            STANDARD.encode(DER),
            STANDARD.encode(PGP),
            escaped(&pem(DER)),
            armoured(PGP).replace('\n', "%0A"),
            STANDARD.encode(b"neither")
        ),
    );
    assert_eq!(got, [x509(DER), openpgp(PGP), openpgp(armoured(PGP).as_bytes())]);
}

#[test]
fn secret_key_material_is_never_kept() {
    let pkcs8 = der(0x30, &[der(0x02, &[0]), der(0x30, &der(0x06, &[0x2a, 0x86, 0x48])), der(0x04, &[1, 2, 3, 4])].concat());
    let pkcs12 = der(0x30, &[der(0x02, &[3]), der(0x30, &der(0x06, &[0x2a])), der(0x30, &[])].concat());
    let encrypted_pkcs8 = der(0x30, &[der(0x30, &der(0x06, &[0x2a])), der(0x04, &[9, 9])].concat());
    // A public-key packet followed by a secret-subkey packet (old format, tag 7).
    let with_secret_subkey = [PGP, &[0x9d, 0x00, 0x01, 0x00]].concat();
    for (what, line) in [
        ("an armoured secret key labelled PGP", format!("KEY;TYPE=PGP:{}", escaped(&armoured(PGP).replace("PUBLIC", "PRIVATE")))),
        (
            "a PEM private key labelled X509",
            format!("KEY;TYPE=X509:{}", escaped(&format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----", STANDARD.encode(&pkcs8)))),
        ),
        ("a PEM RSA key", format!("KEY:{}", escaped("-----BEGIN RSA PRIVATE KEY-----\nMIIB\n-----END RSA PRIVATE KEY-----"))),
        (
            "a certificate with a key after it",
            format!(
                "KEY;TYPE=X509:{}",
                escaped(&format!("{}-----BEGIN ENCRYPTED PRIVATE KEY-----\nMIIB\n-----END ENCRYPTED PRIVATE KEY-----", pem(DER)))
            ),
        ),
        ("a secret-key packet, old format (lQAA)", "KEY;TYPE=PGP;ENCODING=b:lQAA".to_string()),
        ("a secret-key packet, new format", format!("KEY;ENCODING=b:{}", STANDARD.encode([0xc5, 0x01, 0x04]))),
        ("a secret-subkey packet, new format", format!("KEY;ENCODING=b:{}", STANDARD.encode([PGP, &[0xc7, 0x00]].concat()))),
        ("a secret subkey after a public key", format!("KEY;TYPE=PGP;ENCODING=b:{}", STANDARD.encode(&with_secret_subkey))),
        ("a secret subkey inside a public armour", format!("KEY;TYPE=PGP:{}", escaped(&armoured(&with_secret_subkey)))),
        ("PKCS #8 labelled X509", format!("KEY;TYPE=X509;ENCODING=b:{}", STANDARD.encode(&pkcs8))),
        ("PKCS #8 without a type", format!("KEY;ENCODING=b:{}", STANDARD.encode(&pkcs8))),
        ("encrypted PKCS #8", format!("KEY:data:application/pkix-cert;base64,{}", STANDARD.encode(&encrypted_pkcs8))),
        ("PKCS #12", format!("KEY;TYPE=X509;ENCODING=b:{}", STANDARD.encode(&pkcs12))),
        ("PKCS #8 in PEM as a certificate", format!("KEY:{}", escaped(&pem(&pkcs8)))),
    ] {
        assert!(keys("3.0", &line).is_empty(), "{what}");
    }
    // And a mailbox file that holds such bytes never has them written to a card.
    let c =
        Contact { display_name: "Ada".into(), keys: vec![x509(&pkcs8), openpgp(&with_secret_subkey), openpgp(&[0x95, 0, 0])], ..Default::default() };
    assert!(!write(&[&c]).contains("KEY"));
}

#[test]
fn text_is_never_taken_for_binary() {
    for line in [
        "KEY;VALUE=text:0abc",
        "KEY;TYPE=X509;VALUE=text:0\\n\\u{3}",
        "KEY;TYPE=X509;ENCODING=QUOTED-PRINTABLE:0hello",
        "KEY;TYPE=PGP;VALUE=text:ABCD 1234 EF56",
        "KEY;TYPE=PGP:ABCD1234EF56",
        // Text that looks like an address is text, not the address of a key.
        "KEY;VALUE=text:https://keys.example.org/ada.asc",
        "KEY;TYPE=PGP;VALUE=TEXT:openpgp4fpr:ABCDEF01",
    ] {
        assert!(keys("3.0", line).is_empty(), "{line}");
    }
}

#[test]
fn an_indeterminate_length_is_no_keys() {
    // Old format, tag 6, length type 3 (the rest of the bytes): only data packets may have it.
    for header in [0x9b_u8, 0xbb] {
        let bytes = [header, 0x04, 0x5e, 0x00, 0x00, 0x01, 0x16];
        assert!(keys("3.0", &format!("KEY;TYPE=PGP;ENCODING=b:{}", STANDARD.encode(bytes))).is_empty(), "{header:#x}");
        assert!(keys("3.0", &format!("KEY;ENCODING=b:{}", STANDARD.encode([PGP, &bytes].concat()))).is_empty(), "{header:#x}");
    }
}

#[test]
fn armour_headers_without_a_space_are_headers() {
    let armour = format!(
        "-----BEGIN PGP PUBLIC KEY BLOCK-----\nVersion:GnuPG v2\nComment:made up\n\n{}\n=AAAA\n-----END PGP PUBLIC KEY BLOCK-----",
        STANDARD.encode(PGP)
    );
    assert_eq!(keys("3.0", &format!("KEY;TYPE=PGP:{}", escaped(&armour))), [openpgp(armour.as_bytes())]);
}

#[test]
fn every_spelling_of_a_certificate_is_kept_as_der_and_the_bytes_say_the_kind() {
    let quoted: String = pem(DER).bytes().map(|b| format!("={b:02X}")).collect();
    let percent: String = pem(DER).bytes().map(|b| format!("%{b:02X}")).collect();
    let got = keys(
        "3.0",
        &format!(
            "KEY;TYPE=X509;ENCODING=QUOTED-PRINTABLE:{quoted}\r\nKEY:data:application/pkix-cert,{percent}\r\nKEY;TYPE=X509;ENCODING=b:{}\r\nKEY;ENCODING=b:{}",
            STANDARD.encode(pem(DER)),
            STANDARD.encode(pem(&certificate(&[8], &[0])))
        ),
    );
    // The first three are the same certificate, kept once.
    assert_eq!(got, [x509(DER), x509(&certificate(&[8], &[0]))]);
    // A label the bytes don't bear out is dropped.
    for line in [
        format!("KEY;TYPE=PGP:{}", escaped(&pem(DER))),
        format!("KEY;TYPE=PGP;ENCODING=b:{}", STANDARD.encode(DER)),
        format!("KEY;TYPE=X509;ENCODING=b:{}", STANDARD.encode(PGP)),
        format!("KEY:data:application/pkix-cert;base64,{}", STANDARD.encode(PGP)),
        format!("KEY;MEDIATYPE=application/pgp-keys:{}", escaped(&pem(DER))),
    ] {
        assert!(keys("4.0", &line).is_empty(), "{line}");
    }
}

#[test]
fn what_is_not_a_key_is_dropped() {
    for junk in [
        "KEY;TYPE=PGP;ENCODING=b:!!!",
        "KEY;TYPE=PGP;ENCODING=b:",
        "KEY;TYPE=X509:",
        "KEY:javascript:alert(1)",
        "KEY:file:///etc/passwd",
        "KEY:cid:part1@example.org",
        "KEY;VALUE=uri:https://exa mple.org/key",
        "KEY:https:",
        "KEY:no scheme at all",
        "KEY:data:application/pgp-keys;base64",
        "KEY;TYPE=X509:-----BEGIN CERTIFICATE-----\\n!!!\\n-----END CERTIFICATE-----",
        "KEY;TYPE=X509:-----BEGIN CERTIFICATE-----\\nMAA=",
        "KEY;TYPE=X-SOMETHING;ENCODING=b:AAEC",
        // A packet that runs past the end, a partial length, a header that is none.
        "KEY;ENCODING=b:mQAJBA==",
        "KEY;ENCODING=b:xuAA",
        "KEY;ENCODING=b:BgAA",
    ] {
        assert!(keys("3.0", junk).is_empty(), "{junk}");
    }
    let long = format!("KEY:https://example.org/{}", "a".repeat(ContactKey::MAX_URI));
    assert!(keys("4.0", &long).is_empty());
}

#[test]
fn an_address_with_format_characters_is_dropped() {
    for hidden in ['\u{202e}', '\u{200b}', '\u{2066}', '\u{feff}', '\u{ad}'] {
        assert!(keys("4.0", &format!("KEY:https://keys.example.org/{hidden}ada.asc")).is_empty(), "{:?}", hidden);
    }
    assert_eq!(keys("4.0", "KEY:https://keys.example.org/ada.asc"), [at("https://keys.example.org/ada.asc", "")]);
}

#[test]
fn the_size_counted_is_the_keys_own() {
    // A quoted-printable certificate just under the cap is three times as long written, and kept.
    let big = certificate(&[1], &vec![0; ContactKey::MAX_BYTES - 20]);
    assert!(big.len() <= ContactKey::MAX_BYTES && big.len() > ContactKey::MAX_BYTES - 32);
    let quoted: String = big.iter().map(|b| format!("={b:02X}")).collect();
    assert_eq!(keys("2.1", &format!("KEY;X509;QUOTED-PRINTABLE:{quoted}")), [x509(&big)]);
    let percent: String = big.iter().map(|b| format!("%{b:02X}")).collect();
    assert_eq!(keys("4.0", &format!("KEY:data:application/pkix-cert,{percent}")), [x509(&big)]);
    // One byte more is not.
    let over = certificate(&[1], &vec![0; ContactKey::MAX_BYTES - 20 + (ContactKey::MAX_BYTES - big.len()) + 1]);
    assert_eq!(over.len(), ContactKey::MAX_BYTES + 1);
    assert!(keys("3.0", &format!("KEY;TYPE=X509;ENCODING=b:{}", STANDARD.encode(&over))).is_empty());
    let quoted: String = over.iter().map(|b| format!("={b:02X}")).collect();
    assert!(keys("2.1", &format!("KEY;X509;QUOTED-PRINTABLE:{quoted}")).is_empty());
}

#[test]
fn keys_are_capped_per_card_and_per_text() {
    let many: String = (0..Contact::MAX_KEYS + 10)
        .map(|i| format!("KEY;TYPE=X509;ENCODING=b:{}\r\n", STANDARD.encode(certificate(&[u8::try_from(i).unwrap()], &[0]))))
        .collect();
    assert_eq!(keys("3.0", &many).len(), Contact::MAX_KEYS);
    // What one text keeps of keys in all: the cards after the allowance is spent keep none.
    let card = |n: u8| format!("BEGIN:VCARD\r\nFN:C{n}\r\nKEY;TYPE=X509;ENCODING=b:{}\r\nEND:VCARD\r\n", STANDARD.encode(certificate(&[n], &[0])));
    let text: String = (0..5).map(card).collect();
    let got = crate::read::parse_keeping(&text, 25);
    assert_eq!(got.iter().map(|c| c.keys.len()).collect::<Vec<_>>(), [1, 1, 0, 0, 0]);
    assert_eq!(parse(&text).iter().map(|c| c.keys.len()).sum::<usize>(), 5);
}

#[test]
fn a_card_of_nothing_but_keys_is_skipped() {
    let only = format!(
        "BEGIN:VCARD\r\nVERSION:3.0\r\nKEY;TYPE=X509;ENCODING=b:{}\r\nKEY:https://keys.example.org/a.asc\r\nEND:VCARD\r\n",
        STANDARD.encode(DER)
    );
    assert!(parse(&only).is_empty());
    assert_eq!(parse(&only.replace("VERSION:3.0", "VERSION:3.0\r\nFN:Ada"))[0].keys.len(), 2);
}

#[test]
fn writes_keys_as_3_0() {
    let c = Contact {
        display_name: "Ada".into(),
        keys: vec![
            x509(DER),
            openpgp(PGP),
            openpgp(armoured(PGP).as_bytes()),
            // Never written: addresses (3.0 has no form for them), nothing in it, bytes that aren't the kind they say.
            at("https://keys.example.org/ada.asc", ContactKey::OPENPGP_MEDIA_TYPE),
            at("openpgp4fpr:ABCDEF01", ""),
            openpgp(&[]),
            openpgp(DER),
            ContactKey { uri: "https://example.org/a".into(), ..x509(DER) },
        ],
        ..Default::default()
    };
    let out = write(&[&c]);
    let lines: Vec<&str> = out.split("\r\n").filter(|l| l.starts_with("KEY")).collect();
    assert_eq!(lines.len(), 3, "{out}");
    assert_eq!(lines[0], format!("KEY;TYPE=X509;ENCODING=b:{}", STANDARD.encode(DER)));
    assert_eq!(lines[1], format!("KEY;TYPE=PGP;ENCODING=b:{}", STANDARD.encode(PGP)));
    assert!(lines[2].starts_with("KEY;TYPE=PGP;ENCODING=b:"));
    assert!(!out.contains("VALUE=uri") && !out.contains("keys.example.org"));
    assert_eq!(parse(&out)[0].keys, c.keys[..3]);
}

#[test]
fn large_keys_fold_and_read_back() {
    let signature: Vec<u8> = (0..ContactKey::MAX_BYTES - 32).map(|i| (i % 251) as u8).collect();
    let cert = certificate(&[1], &signature);
    let body: Vec<u8> = (0..60_000u32).map(|i| (i % 251) as u8).collect();
    let pgp = [&[0x9a, 0x00, 0x00, 0xea, 0x60][..], &body].concat();
    let c = Contact { display_name: "Ada".into(), keys: vec![x509(&cert), openpgp(&pgp)], ..Default::default() };
    let out = write(&[&c]);
    assert!(out.split("\r\n").all(|line| line.len() <= 75));
    assert_eq!(parse(&out)[0].keys, c.keys);
    assert_eq!(crate::parse_bytes(out.as_bytes())[0].keys, c.keys);
}

/// A certificate's outline or an OpenPGP public key (binary or armoured), from random bytes.
fn key() -> impl Strategy<Value = ContactKey> {
    let bytes = || proptest::collection::vec(any::<u8>(), 1..200);
    let user_ids = proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..60), 0..3);
    let packets = (bytes(), user_ids).prop_map(|(body, ids)| {
        let mut out = vec![0x99, 0, body.len() as u8];
        out.extend(body);
        for id in ids {
            // An old-format user ID packet (tag 13) with a one-byte length.
            out.extend([0xb4, id.len() as u8]);
            out.extend(id);
        }
        out
    });
    prop_oneof![
        (bytes(), bytes()).prop_map(|(serial, signature)| x509(&certificate(&serial[..serial.len().min(100)], &signature))),
        packets.clone().prop_map(|p| openpgp(&p)),
        packets.prop_map(|p| openpgp(armoured(&p).as_bytes())),
    ]
}

proptest! {
    #[test]
    fn any_keys_round_trip(list in proptest::collection::vec(key(), 0..6)) {
        let mut unique: Vec<ContactKey> = Vec::new();
        for k in list {
            if !unique.contains(&k) {
                unique.push(k);
            }
        }
        let c = Contact { display_name: "Ada".into(), keys: unique, ..Default::default() };
        let back = parse(&write(&[&c]));
        prop_assert_eq!(back.len(), 1);
        prop_assert_eq!(&back[0].keys, &c.keys);
    }

    #[test]
    fn arbitrary_key_values_never_panic(
        params in proptest::sample::select(vec!["", ";ENCODING=b", ";TYPE=PGP", ";TYPE=X509;ENCODING=BASE64", ";X509;QUOTED-PRINTABLE", ";MEDIATYPE=application/pgp-keys", ";VALUE=text", ";VALUE=uri"]),
        head in proptest::sample::select(vec!["", "data:", "data:application/pkix-cert;base64,", "data:,%", "-----BEGIN CERTIFICATE-----", "-----BEGIN PGP PUBLIC KEY BLOCK-----", "https:", "openpgp4fpr:", "0", "\u{99}"]),
        value in any::<String>(),
        bytes in proptest::collection::vec(any::<u8>(), 0..512),
    ) {
        let text = format!("BEGIN:VCARD\r\nFN:x\r\nKEY{params}:{head}{value}\r\nKEY{params}:{head}{}\r\nEND:VCARD\r\n", STANDARD.encode(&bytes));
        let got = parse(&text);
        for c in &got {
            prop_assert!(c.keys.iter().all(|k| k.data.len() <= ContactKey::MAX_BYTES && k.uri.len() <= ContactKey::MAX_URI));
            // What was read writes and reads back as itself (addresses are not written to 3.0).
            let again = parse(&write(&[c]));
            let written: Vec<&ContactKey> = c.keys.iter().filter(|k| k.kind != KeyKind::Uri).collect();
            prop_assert_eq!(again.first().map(|a| a.keys.iter().collect::<Vec<_>>()), Some(written));
        }
        let mut raw = format!("BEGIN:VCARD\r\nFN:x\r\nKEY{params}:{head}").into_bytes();
        raw.extend(&bytes);
        raw.extend(b"\r\nEND:VCARD\r\n");
        let _ = crate::parse_bytes(&raw);
    }

    #[test]
    fn random_bytes_are_never_kept_as_secret_material(body in proptest::collection::vec(any::<u8>(), 0..300)) {
        // Whatever follows a secret-key or secret-subkey header, nothing is kept.
        for header in [[0x95u8, 0x00], [0x94, 0x00], [0x9d, 0x00], [0xc5, 0x00], [0xc7, 0x00]] {
            let mut packet = header.to_vec();
            if let Some(len) = packet.get_mut(1) {
                *len = body.len().min(150) as u8;
            }
            let mut bytes = packet;
            bytes.extend(body.iter().take(150));
            let line = format!("KEY;TYPE=PGP;ENCODING=b:{}", STANDARD.encode([PGP, &bytes].concat()));
            prop_assert!(keys("3.0", &line).is_empty());
        }
    }
}
