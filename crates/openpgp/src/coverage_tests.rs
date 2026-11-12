//! What a signature covers: only a whole message can be Verified, and words around a signed
//! part never borrow its signature.

use pgp::crypto::hash::HashAlgorithm;

use crate::fixtures::*;
use crate::{CantCheck, Checked, Coverage, Fingerprint, KeyRing, MAX_MESSAGE, NotVerified, Outcome, verify};

const ALICE: &str = "Alice <alice@example.org>";

fn yes(_: &Fingerprint, _: &str) -> bool {
    true
}

fn alice() -> (pgp::composed::SignedSecretKey, KeyRing) {
    let alice = secret(ALICE, Make::default());
    let ring = [public(&alice)].into_iter().collect();
    (alice, ring)
}

/// A PGP/MIME message by `alice` as a part: its Content-Type field and its body.
fn signed_part(alice: &pgp::composed::SignedSecretKey) -> (String, String) {
    let inner = String::from_utf8(signed_message(alice, HashAlgorithm::Sha256)).unwrap();
    let (head, body) = inner.split_once("\r\n\r\n").unwrap();
    let content_type = head.split("\r\n").skip_while(|l| !l.starts_with("Content-Type")).collect::<Vec<_>>().join("\r\n");
    (content_type, body.to_string())
}

fn partly(checked: &Checked) -> bool {
    matches!(&checked.outcome, Outcome::Valid { why: NotVerified { partly_signed: true, .. }, .. })
}

#[test]
fn a_pgp_mime_message_at_the_top_covers_the_whole_message() {
    let (alice, keys) = alice();
    let checked = verify(&signed_message(&alice, HashAlgorithm::Sha256), ALICE, &keys, LATER, yes).unwrap();
    assert_eq!(checked.coverage, Coverage { whole: true, path: vec![], span: None });
    assert!(matches!(checked.outcome, Outcome::Verified(_)));
}

#[test]
fn a_signed_alternative_beside_an_unsigned_one_is_not_a_signed_message() {
    // The HTML alternative is what most readers show, and it isn't signed.
    let (alice, keys) = alice();
    let (content_type, body) = signed_part(&alice);
    let message = format!(
        "From: {ALICE}\r\nContent-Type: multipart/alternative; boundary=\"alt\"\r\n\r\n--alt\r\n{content_type}\r\n\r\n{body}\r\n--alt\r\nContent-Type: text/html\r\n\r\n<p>Wire 10000 EUR to Mallory</p>\r\n--alt--\r\n"
    );
    assert_eq!(verify(message.as_bytes(), ALICE, &keys, LATER, yes), None);
}

#[test]
fn a_signed_part_beside_an_unsigned_one_is_only_partly_signed() {
    let (alice, keys) = alice();
    let (content_type, body) = signed_part(&alice);
    let message = format!(
        "From: {ALICE}\r\nContent-Type: multipart/mixed; boundary=\"mix\"\r\n\r\n--mix\r\n{content_type}\r\n\r\n{body}\r\n--mix\r\nContent-Type: text/plain\r\n\r\nWire 10000 EUR to Mallory\r\n--mix--\r\n"
    );
    let checked = verify(message.as_bytes(), ALICE, &keys, LATER, yes).unwrap();
    assert!(!checked.coverage.whole);
    assert_eq!(checked.coverage.path, vec![0]);
    assert!(partly(&checked), "{checked:?}");
}

#[test]
fn an_inline_signed_body_alone_covers_the_whole_message_and_says_where() {
    let (alice, keys) = alice();
    let block = cleartext(&alice, "Pay 10 EUR to Bob.\n");
    let body = format!("\r\n{block}\r\n\r\n");
    let checked = verify(&plain(ALICE, "7bit", &body), ALICE, &keys, LATER, yes).unwrap();
    assert!(matches!(checked.outcome, Outcome::Verified(_)), "{checked:?}");
    assert!(checked.coverage.whole);
    let (start, end) = checked.coverage.span.unwrap();
    assert_eq!(&body[start..end].trim_end(), &block.trim_end());
}

#[test]
fn words_before_or_after_an_inline_signed_block_are_not_covered() {
    let (alice, keys) = alice();
    let block = cleartext(&alice, "Pay 10 EUR to Bob.\n");
    for body in [format!("Wire 10000 EUR to Mallory\r\n{block}"), format!("{block}\r\nPS: wire 10000 EUR to Mallory\r\n")] {
        let checked = verify(&plain(ALICE, "7bit", &body), ALICE, &keys, LATER, yes).unwrap();
        assert!(!checked.coverage.whole);
        assert!(partly(&checked), "{checked:?}");
    }
}

#[test]
fn two_inline_signed_blocks_are_each_only_part() {
    let (alice, keys) = alice();
    let body = format!("{}\r\n{}", cleartext(&alice, "First.\n"), cleartext(&alice, "Second.\n"));
    let checked = verify(&plain(ALICE, "7bit", &body), ALICE, &keys, LATER, yes).unwrap();
    assert!(partly(&checked), "{checked:?}");
}

#[test]
fn an_armour_header_other_than_hash_or_an_unescaped_dash_line_is_unreadable() {
    // Both would show words in the block that aren't signed.
    let (alice, keys) = alice();
    let block = cleartext(&alice, "Pay 10 EUR to Bob.\n");
    let header = block.replacen("-----BEGIN PGP SIGNED MESSAGE-----\n", "-----BEGIN PGP SIGNED MESSAGE-----\nNote: and 10000 EUR to Mallory\n", 1);
    assert_ne!(header, block);
    let dash = block.replacen("Pay 10 EUR to Bob.", "-----BEGIN PGP SIGNATURE-----", 1);
    // A `Hash:` header with more than hash names after it.
    let with_hash = block.replacen(
        "-----BEGIN PGP SIGNED MESSAGE-----\n",
        "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256, and pay Mallory 10000 EUR today\n",
        1,
    );
    assert_ne!(with_hash, block);
    // Hash names alone, as many writers give them, are fine.
    let names = block.replacen("-----BEGIN PGP SIGNED MESSAGE-----\n", "-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256, sha512 ,SHA3-256\n", 1);
    let checked = verify(&plain(ALICE, "7bit", &names), ALICE, &keys, LATER, yes).unwrap();
    assert!(matches!(checked.outcome, Outcome::Verified(_)), "{checked:?}");
    for body in [header, dash, with_hash] {
        let checked = verify(&plain(ALICE, "7bit", &body), ALICE, &keys, LATER, yes).unwrap();
        assert!(matches!(checked.outcome, Outcome::CantCheck { why: CantCheck::Unreadable, .. }), "{checked:?}");
    }
}

#[test]
fn a_signed_message_too_large_to_check_says_so() {
    let mut big = signed_message(&secret(ALICE, Make::default()), HashAlgorithm::Sha256);
    big.resize(MAX_MESSAGE + 1, b'a');
    let checked = verify(&big, ALICE, &KeyRing::new(), LATER, yes).unwrap();
    assert!(matches!(checked.outcome, Outcome::CantCheck { why: CantCheck::TooLarge, .. }));
    // Inline-signed, the block found anywhere in it.
    let mut big = plain(ALICE, "7bit", &"a".repeat(1 << 20));
    big.extend_from_slice(b"\r\n-----BEGIN PGP SIGNED MESSAGE-----\r\n");
    big.resize(MAX_MESSAGE + 1, b'a');
    assert!(verify(&big, ALICE, &KeyRing::new(), LATER, yes).is_some());
    // Not signed at all: nothing to say.
    let mut big = plain(ALICE, "7bit", "Just text.\r\n");
    big.resize(MAX_MESSAGE + 1, b'a');
    assert_eq!(verify(&big, ALICE, &KeyRing::new(), LATER, yes), None);
}
