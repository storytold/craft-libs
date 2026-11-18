//! Checking Digital signatures end to end: keys and signatures made here, and RFC 9580's samples.

use pgp::crypto::hash::HashAlgorithm;

use crate::fixtures::*;
use crate::{CantCheck, Coverage, Fingerprint, KeyRing, NotVerified, Outcome, PublicKey, keys_in_message};

const ALICE: &str = "Alice <alice@example.org>";

fn ring(keys: &[&pgp::composed::SignedSecretKey]) -> KeyRing {
    keys.iter().map(|k| public(k)).collect()
}

fn yes(_: &Fingerprint, _: &str) -> bool {
    true
}

fn no(_: &Fingerprint, _: &str) -> bool {
    false
}

/// The outcome alone, for the tests that don't look at what is covered.
fn verify(message: &[u8], from: &str, keys: &KeyRing, now: u64, verified: impl Fn(&Fingerprint, &str) -> bool) -> Option<Outcome> {
    crate::verify(message, from, keys, now, verified).map(|c| c.outcome)
}

#[test]
fn pgp_mime_by_a_verified_key_from_its_address_is_verified() {
    let alice = secret(ALICE, Make::default());
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, "Alice <alice@example.org>", &ring(&[&alice]), LATER, yes);
    let Some(Outcome::Verified(signer)) = outcome else { panic!("{outcome:?}") };
    assert_eq!(&signer.fingerprint, public(&alice).fingerprint());
    assert_eq!(signer.subkey, None);
    assert_eq!(signer.addresses, vec!["alice@example.org".to_string()]);
    assert_eq!(signer.hash, "SHA-256");
    assert_eq!(signer.algorithm, "EdDSA");
    assert!(signer.signed > 0);
}

#[test]
fn a_key_not_verified_is_valid_but_not_verified() {
    let alice = secret(ALICE, Make::default());
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, "alice@example.org", &ring(&[&alice]), LATER, no);
    let Some(Outcome::Valid { why, .. }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(why, NotVerified { key_unverified: true, ..NotVerified::default() });
}

#[test]
fn a_from_address_not_on_the_key_is_valid_but_not_verified() {
    let alice = secret(ALICE, Make::default());
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, "Mallory <mallory@example.org>", &ring(&[&alice]), LATER, yes);
    let Some(Outcome::Valid { why, .. }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(why, NotVerified { address_differs: true, ..NotVerified::default() });
}

#[test]
fn a_signed_part_changed_after_signing_is_changed() {
    let alice = secret(ALICE, Make::default());
    let armour = detached(&alice, SIGNED_PART.as_bytes(), HashAlgorithm::Sha256, false);
    let message = pgp_mime("alice@example.org", &SIGNED_PART.replace("attached", "lost"), &armour);
    let outcome = verify(&message, ALICE, &ring(&[&alice]), LATER, yes);
    assert!(matches!(outcome, Some(Outcome::Changed(_))), "{outcome:?}");
}

#[test]
fn a_changed_header_of_the_signed_part_is_changed() {
    let alice = secret(ALICE, Make::default());
    let armour = detached(&alice, SIGNED_PART.as_bytes(), HashAlgorithm::Sha256, false);
    let message = pgp_mime("alice@example.org", &SIGNED_PART.replace("utf-8", "us-ascii"), &armour);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Changed(_))));
}

#[test]
fn no_public_key_cant_be_checked_and_names_the_signer() {
    let alice = secret(ALICE, Make::default());
    let bob = secret("Bob <bob@example.org>", Make { seed: 2, ..Make::default() });
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, ALICE, &ring(&[&bob]), LATER, yes);
    let Some(Outcome::CantCheck { why: CantCheck::NoPublicKey, issuer }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(issuer.fingerprint.as_ref(), Some(public(&alice).fingerprint()));
    // rPGP writes the key id in the unhashed area, which anybody can change: not believed.
    assert_eq!(issuer.key_id, None);
}

#[test]
fn sha1_and_md5_are_algorithms_no_longer_safe() {
    let alice = secret(ALICE, Make { rsa: Some(2048), ..Make::default() });
    for (hash, name) in [(HashAlgorithm::Sha1, "SHA-1"), (HashAlgorithm::Md5, "MD5")] {
        let armour = detached(&alice, SIGNED_PART.as_bytes(), hash, false);
        let message = pgp_mime("alice@example.org", SIGNED_PART, &armour);
        let outcome = verify(&message, ALICE, &ring(&[&alice]), LATER, yes);
        assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::WeakAlgorithm(n), .. }) if n == name), "{outcome:?}");
    }
}

#[test]
fn a_signing_subkey_is_found_by_its_own_id() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let message = signed_message(&alice, HashAlgorithm::Sha512);
    let outcome = verify(&message, ALICE, &ring(&[&alice]), LATER, yes);
    let Some(Outcome::Verified(signer)) = outcome else { panic!("{outcome:?}") };
    assert!(signer.subkey.is_some());
    assert_ne!(signer.subkey.as_ref(), Some(&signer.fingerprint));
    assert_eq!(signer.hash, "SHA-512");
}

#[test]
fn version_6_keys_check() {
    let alice = secret(ALICE, Make { v6: true, seed: 3, ..Make::default() });
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, ALICE, &ring(&[&alice]), LATER, yes);
    let Some(Outcome::Verified(signer)) = outcome else { panic!("{outcome:?}") };
    assert_eq!(signer.fingerprint.as_bytes().len(), 32);
    assert_eq!(signer.algorithm, "Ed25519");
}

#[test]
fn a_message_kept_with_bare_line_feeds_is_checked_in_canonical_form() {
    let alice = secret(ALICE, Make::default());
    let message = String::from_utf8(signed_message(&alice, HashAlgorithm::Sha256)).unwrap().replace("\r\n", "\n");
    assert!(matches!(verify(message.as_bytes(), ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Verified(_))));
}

#[test]
fn a_text_mode_signature_checks() {
    let alice = secret(ALICE, Make::default());
    let armour = detached(&alice, SIGNED_PART.as_bytes(), HashAlgorithm::Sha256, true);
    let message = pgp_mime("alice@example.org", SIGNED_PART, &armour);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Verified(_))));
}

#[test]
fn a_signed_message_inside_a_mailing_list_wrapper_is_found_but_only_partly_signed() {
    let alice = secret(ALICE, Make::default());
    let inner = String::from_utf8(signed_message(&alice, HashAlgorithm::Sha256)).unwrap();
    // The signed message's own Content-Type becomes the first part's header.
    let (head, body) = inner.split_once("\r\n\r\n").unwrap();
    let content_type = head.split("\r\n").skip_while(|l| !l.starts_with("Content-Type")).collect::<Vec<_>>().join("\r\n");
    let wrapped = format!(
        "From: list@example.org\r\nContent-Type: multipart/mixed; boundary=\"outer\"\r\n\r\n--outer\r\n{content_type}\r\n\r\n{body}\r\n--outer\r\nContent-Type: text/plain\r\n\r\nList footer\r\n--outer--\r\n"
    );
    // The footer isn't signed: the signature is good, but the message isn't Verified.
    let checked = crate::verify(wrapped.as_bytes(), ALICE, &ring(&[&alice]), LATER, yes).unwrap();
    assert_eq!(checked.coverage, Coverage { whole: false, path: vec![0], span: None });
    let Outcome::Valid { why, .. } = checked.outcome else { panic!("{checked:?}") };
    assert_eq!(why, NotVerified { partly_signed: true, ..NotVerified::default() });
}

#[test]
fn inline_cleartext_signed_text_checks() {
    let alice = secret(ALICE, Make::default());
    // (Trailing white space, which a reader drops, is in `armour_tests`: rPGP signs it as it is.)
    let text = "Hello Bob,\n- a line that starts with a dash\n-----\nAlice\n";
    let message = plain(ALICE, "7bit", &cleartext(&alice, text));
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Verified(_))));
}

#[test]
fn inline_text_changed_after_signing_is_changed() {
    let alice = secret(ALICE, Make::default());
    let signed = cleartext(&alice, "Pay 10 EUR to Bob.\n").replace("10 EUR", "99 EUR");
    let message = plain(ALICE, "7bit", &signed);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Changed(_))));
}

#[test]
fn inline_text_in_quoted_printable_and_base64_checks() {
    let alice = secret(ALICE, Make::default());
    let signed = cleartext(&alice, "Gr=C3=BC=C3=9Fe and a long line\n");
    // Quoted-printable: the `=` of the text is written as `=3D`, and a soft break is added.
    let qp = signed.replace('=', "=3D").replacen("Gr=3DC3", "Gr=\r\n=3DC3", 1);
    let message = plain(ALICE, "quoted-printable", &qp);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Verified(_))), "qp");
    let b64 = base64(signed.as_bytes());
    let message = plain(ALICE, "base64", &b64);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice]), LATER, yes), Some(Outcome::Verified(_))), "base64");
}

#[test]
fn rfc9580_sample_cleartext_message_checks_against_the_sample_certificate() {
    let keys: KeyRing = PublicKey::parse(RFC9580_A3_CERT.as_bytes()).unwrap().into_iter().collect();
    assert_eq!(keys.keys()[0].fingerprint().to_hex(), RFC9580_A3_FINGERPRINT);
    // The sample certificate has no User ID, so no address is its own.
    assert!(keys.keys()[0].addresses().is_empty());
    let message = plain("someone@example.org", "7bit", RFC9580_A6_CLEARTEXT);
    let outcome = verify(&message, "someone@example.org", &keys, LATER, yes);
    let Some(Outcome::Valid { signer, why }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(why, NotVerified { address_differs: true, ..NotVerified::default() });
    assert_eq!(signer.algorithm, "Ed25519");
    // A changed grocery list is a changed message.
    let changed = plain("someone@example.org", "7bit", &RFC9580_A6_CLEARTEXT.replace("tofu", "tempeh"));
    assert!(matches!(verify(&changed, "someone@example.org", &keys, LATER, yes), Some(Outcome::Changed(_))));
}

#[test]
fn messages_without_an_openpgp_signature_are_none() {
    let keys = KeyRing::new();
    assert_eq!(verify(&plain(ALICE, "7bit", "Just text.\r\n"), ALICE, &keys, LATER, yes), None);
    let smime = b"From: a@example.org\r\nContent-Type: multipart/signed; protocol=\"application/pkcs7-signature\"; boundary=b\r\n\r\n--b\r\n\r\nx\r\n--b\r\nContent-Type: application/pkcs7-signature\r\n\r\nAAAA\r\n--b--\r\n";
    assert_eq!(verify(smime, ALICE, &keys, LATER, yes), None);
    let encrypted =
        b"Content-Type: multipart/encrypted; protocol=\"application/pgp-encrypted\"; boundary=b\r\n\r\n--b\r\n\r\nVersion: 1\r\n--b--\r\n";
    assert_eq!(verify(encrypted, ALICE, &keys, LATER, yes), None);
}

#[test]
fn a_broken_pgp_mime_layout_cant_be_checked() {
    let keys = KeyRing::new();
    let one_part = b"Content-Type: multipart/signed; protocol=\"application/pgp-signature\"; boundary=b\r\n\r\n--b\r\n\r\nx\r\n--b--\r\n";
    assert!(matches!(verify(one_part, ALICE, &keys, LATER, yes), Some(Outcome::CantCheck { why: CantCheck::Unreadable, .. })));
    let junk = pgp_mime("alice@example.org", SIGNED_PART, "-----BEGIN PGP SIGNATURE-----\r\n\r\nnot base64 at all!\r\n-----END PGP SIGNATURE-----");
    assert!(matches!(verify(&junk, ALICE, &keys, LATER, yes), Some(Outcome::CantCheck { why: CantCheck::Unreadable, .. })));
}

#[test]
fn a_signature_by_another_key_with_the_same_address_is_changed_only_when_named() {
    // Two keys for one address: the signature names the first, which doesn't match the text.
    let alice = secret(ALICE, Make::default());
    let other = secret(ALICE, Make { seed: 5, ..Make::default() });
    let armour = detached(&alice, b"something else", HashAlgorithm::Sha256, false);
    let message = pgp_mime("alice@example.org", SIGNED_PART, &armour);
    assert!(matches!(verify(&message, ALICE, &ring(&[&alice, &other]), LATER, yes), Some(Outcome::Changed(_))));
    // Without the named key, the other key of the address isn't blamed.
    assert!(matches!(verify(&message, ALICE, &ring(&[&other]), LATER, yes), Some(Outcome::CantCheck { why: CantCheck::NoMatchingKey, .. })));
}

#[test]
fn the_clock_matters_only_for_what_expires() {
    let alice = secret(ALICE, Make::default());
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    // Neither key nor signature expires, so any time the caller gives checks the same.
    let outcome = verify(&message, ALICE, &ring(&[&alice]), 0, yes);
    assert!(matches!(outcome, Some(Outcome::Verified(_))), "{outcome:?}");
}

#[test]
fn keys_are_read_armoured_or_binary_and_round_trip() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let armoured = armoured_public(&alice);
    let keys = PublicKey::parse(armoured.as_bytes()).unwrap();
    assert_eq!(keys.len(), 1);
    let key = &keys[0];
    assert_eq!(key.addresses(), ["alice@example.org".to_string()]);
    assert!(key.has_address("ALICE@example.org"));
    assert!(!key.revoked());
    assert_eq!(key.expires(), None);
    let again = PublicKey::parse(&key.to_bytes()).unwrap();
    assert_eq!(again[0].fingerprint(), key.fingerprint());
    assert_eq!(PublicKey::parse(b"not a key").unwrap_err(), crate::Error::Unreadable);
}

#[test]
fn a_key_whose_user_id_was_altered_has_no_address() {
    let alice = secret(ALICE, Make::default());
    let mut public_key = pgp::composed::SignedPublicKey::from(alice.clone());
    let user = public_key.details.users[0].clone();
    let forged = pgp::packet::UserId::from_str(Default::default(), "Mallory <mallory@example.org>").unwrap();
    public_key.details.users[0] = pgp::types::SignedUser::new(forged, user.signatures.clone());
    let bytes = pgp::ser::Serialize::to_bytes(&public_key).unwrap();
    // No self-signature checks, so it is no key at all.
    assert!(PublicKey::parse(&bytes).is_err());
}

#[test]
fn the_ring_keeps_one_copy_per_fingerprint_and_finds_keys_by_address() {
    let alice = secret(ALICE, Make::default());
    let bob = secret("Bob <bob@example.org>", Make { seed: 2, ..Make::default() });
    let mut keys = KeyRing::new();
    assert!(keys.insert(public(&alice)));
    assert!(!keys.insert(public(&alice)));
    assert!(keys.insert(public(&bob)));
    assert_eq!(keys.len(), 2);
    assert_eq!(keys.for_address("Bob <BOB@example.org>").count(), 1);
    assert_eq!(keys.for_address("nobody@example.org").count(), 0);
    assert!(keys.get(public(&bob).fingerprint()).is_some());
}

#[test]
fn fingerprints_read_and_write_hex() {
    let fp = Fingerprint::from_hex(RFC9580_A3_FINGERPRINT).unwrap();
    assert_eq!(fp.to_hex(), RFC9580_A3_FINGERPRINT);
    assert_eq!(Fingerprint::from_hex(&fp.grouped()), Some(fp.clone()));
    assert!(fp.grouped().starts_with("CB18 6C4F "));
    assert_eq!(Fingerprint::from_hex("0x1234"), None);
    assert_eq!(Fingerprint::from_hex(&"G".repeat(40)), None);
}

#[test]
fn attached_keys_and_the_autocrypt_header_are_found() {
    let alice = secret(ALICE, Make::default());
    let carol = secret("Carol <carol@example.org>", Make { seed: 4, ..Make::default() });
    let keydata = base64(&pgp::ser::Serialize::to_bytes(&pgp::composed::SignedPublicKey::from(alice.clone())).unwrap()).replace("\r\n", "\r\n ");
    let message = format!(
        "From: Alice <alice@example.org>\r\nAutocrypt: addr=alice@example.org; prefer-encrypt=mutual; keydata=\r\n {keydata}\r\nContent-Type: multipart/mixed; boundary=m\r\n\r\n--m\r\nContent-Type: text/plain\r\n\r\nCarol's key, as asked.\r\n--m\r\nContent-Type: application/pgp-keys; name=\"carol.asc\"\r\n\r\n{}\r\n--m--\r\n",
        armoured_public(&carol)
    );
    let found = keys_in_message(message.as_bytes());
    let autocrypt = found.autocrypt.expect("autocrypt");
    assert_eq!(autocrypt.address, "alice@example.org");
    assert!(autocrypt.prefer_encrypt_mutual);
    assert_eq!(autocrypt.key.fingerprint(), public(&alice).fingerprint());
    assert_eq!(found.attached.len(), 1);
    assert_eq!(found.attached[0].addresses(), ["carol@example.org".to_string()]);
}

#[test]
fn an_autocrypt_header_for_another_address_or_twice_is_ignored() {
    let alice = secret(ALICE, Make::default());
    let keydata = base64(&pgp::ser::Serialize::to_bytes(&pgp::composed::SignedPublicKey::from(alice)).unwrap()).replace("\r\n", "");
    let header = format!("Autocrypt: addr=alice@example.org; keydata={keydata}\r\n");
    let other = format!("From: mallory@example.org\r\n{header}\r\nHi\r\n");
    assert!(keys_in_message(other.as_bytes()).autocrypt.is_none());
    let twice = format!("From: alice@example.org\r\n{header}{header}\r\nHi\r\n");
    assert!(keys_in_message(twice.as_bytes()).autocrypt.is_none());
    let once = format!("From: alice@example.org\r\n{header}\r\nHi\r\n");
    let found = keys_in_message(once.as_bytes()).autocrypt.expect("one header");
    assert!(!found.prefer_encrypt_mutual);
}

#[test]
fn a_revoked_key_cant_be_checked() {
    use pgp::packet::{SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::{KeyDetails, Password, Timestamp};
    use rand::SeedableRng;
    let alice = secret(ALICE, Make::default());
    let mut public_key = pgp::composed::SignedPublicKey::from(alice.clone());
    let mut config = SignatureConfig::from_key(rand::rngs::StdRng::seed_from_u64(11), &alice.primary_key, SignatureType::KeyRevocation).unwrap();
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::now())).unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(alice.primary_key.fingerprint())).unwrap(),
    ];
    let revocation = config.sign_key(&alice.primary_key, &Password::empty(), &public_key.primary_key).unwrap();
    public_key.details.revocation_signatures.push(revocation);
    let keys: KeyRing = PublicKey::parse(&pgp::ser::Serialize::to_bytes(&public_key).unwrap()).unwrap().into_iter().collect();
    assert!(keys.keys()[0].revoked());
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    let outcome = verify(&message, ALICE, &keys, LATER, yes);
    assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::KeyRevoked, .. })), "{outcome:?}");
}

/// Binary signature packets by `secret`'s signing key over `data`, made at `at`.
fn packets(secret: &pgp::composed::SignedSecretKey, data: &[u8], at: u32) -> Vec<u8> {
    let key = signer(secret);
    let sig = config(&key, pgp::packet::SignatureType::Binary, at, vec![]).sign(&key, &pgp::types::Password::empty(), data).unwrap();
    pgp::ser::Serialize::to_bytes(&pgp::composed::DetachedSignature::new(sig)).unwrap()
}

fn context(keys: &KeyRing) -> crate::check::Context<'_> {
    crate::check::Context {
        from: "alice@example.org".into(),
        keys,
        now: LATER,
        verified: &|_, _| true,
        verifications: Default::default(),
        data_passes: Default::default(),
    }
}

#[test]
fn keys_tried_for_one_signature_and_one_message_are_capped_and_the_text_hashed_once() {
    // Twenty keys claim the address; the signature is by another, not here.
    let keys: KeyRing = (10..30).map(|seed| public(&secret(ALICE, Make { seed, ..Make::default() }))).collect();
    let stranger = secret(ALICE, Make { seed: 40, ..Make::default() });
    let t = made(&stranger);
    let cx = context(&keys);
    let outcome = crate::check::check(&packets(&stranger, b"data", t), b"data", &cx);
    assert!(matches!(outcome, Outcome::CantCheck { why: CantCheck::NoMatchingKey, .. }), "{outcome:?}");
    assert_eq!(cx.verifications.get(), 8, "eight keys a signature");
    // Eight such signatures together: the message's budget, and one pass over the text.
    let cx = context(&keys);
    let many: Vec<u8> = (0..8).flat_map(|i| packets(&stranger, b"data", t + i)).collect();
    let _ = crate::check::check(&many, b"data", &cx);
    assert_eq!(cx.verifications.get(), 24, "twenty-four checks a message");
    assert_eq!(cx.data_passes.get(), 1);
}

#[test]
fn a_key_is_verified_only_for_the_address_it_was_verified_for() {
    let alice = secret(ALICE, Make { alias: true, ..Make::default() });
    let fingerprint = public(&alice).fingerprint().clone();
    let keys = ring(&[&alice]);
    let only_alice = |fp: &Fingerprint, address: &str| *fp == fingerprint && address == "alice@example.org";
    let message = signed_message(&alice, HashAlgorithm::Sha256);
    assert!(matches!(verify(&message, ALICE, &keys, LATER, only_alice), Some(Outcome::Verified(_))));
    // The key's other address (a User ID added later, say) wasn't the one compared.
    let outcome = verify(&message, "alias@example.org", &keys, LATER, only_alice);
    let Some(Outcome::Valid { why, .. }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(why, NotVerified { key_unverified: true, ..NotVerified::default() });
}

/// A PGP/MIME message whose signature is `sig`, over [`SIGNED_PART`].
fn with_signature(sig: pgp::packet::Signature) -> Vec<u8> {
    let armour = pgp::composed::DetachedSignature::new(sig).to_armored_string(Default::default()).unwrap();
    pgp_mime("alice@example.org", SIGNED_PART, &armour)
}

fn two_mpis() -> pgp::types::SignatureBytes {
    pgp::types::SignatureBytes::Mpis(vec![pgp::types::Mpi::from_slice(&[1; 32]), pgp::types::Mpi::from_slice(&[1; 32])])
}

#[test]
fn dsa_and_version_3_signatures_are_refused() {
    use pgp::crypto::public_key::PublicKeyAlgorithm;
    use pgp::packet::{Signature, SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::{KeyDetails, Timestamp};
    let alice = secret(ALICE, Make::default());
    let keys = ring(&[&alice]);
    let mut dsa = SignatureConfig::v4(SignatureType::Binary, PublicKeyAlgorithm::DSA, HashAlgorithm::Sha256);
    dsa.hashed_subpackets = vec![Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(made(&alice)))).unwrap()];
    let outcome = verify(&with_signature(Signature::from_config(dsa, [0, 0], two_mpis()).unwrap()), ALICE, &keys, LATER, yes);
    assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::WeakAlgorithm("DSA"), .. })), "{outcome:?}");
    let v3 = SignatureConfig::v3(
        SignatureType::Binary,
        PublicKeyAlgorithm::EdDSALegacy,
        HashAlgorithm::Sha256,
        Timestamp::from_secs(made(&alice)),
        alice.primary_key.legacy_key_id(),
    );
    let outcome = verify(&with_signature(Signature::from_config(v3, [0, 0], two_mpis()).unwrap()), ALICE, &keys, LATER, yes);
    assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::UnknownAlgorithm, .. })), "{outcome:?}");
}

#[test]
fn the_unhashed_area_never_decides_who_is_blamed() {
    use pgp::packet::{Subpacket, SubpacketData};
    use pgp::types::{KeyDetails, Password};
    let mallory = secret("Mallory <mallory@example.org>", Make { seed: 6, ..Make::default() });
    let bob = secret("Bob <bob@example.org>", Make { seed: 2, ..Make::default() });
    let keys = ring(&[&bob]);
    let t = made(&mallory);
    let sign = |unhashed: Vec<SubpacketData>| {
        let mut config = config(&mallory.primary_key, pgp::packet::SignatureType::Binary, t, vec![]);
        config.unhashed_subpackets = unhashed.into_iter().map(|d| Subpacket::regular(d).unwrap()).collect();
        with_signature(config.sign(&mallory.primary_key, &Password::empty(), SIGNED_PART.as_bytes()).unwrap())
    };
    // Mallory's signature, its unhashed area naming Bob: Bob isn't blamed for it.
    let framed = verify(&sign(vec![SubpacketData::IssuerFingerprint(bob.primary_key.fingerprint())]), ALICE, &keys, LATER, yes);
    assert!(matches!(framed, Some(Outcome::CantCheck { why: CantCheck::NoPublicKey, .. })), "{framed:?}");
    assert_eq!(framed, verify(&sign(vec![]), ALICE, &keys, LATER, yes));

    // An issuer only in the unhashed area finds a key, which counts because it fits.
    let alice = secret(ALICE, Make::default());
    let alice_created = made(&alice);
    let mut config = config(&alice.primary_key, pgp::packet::SignatureType::Binary, alice_created, vec![]);
    config.hashed_subpackets.retain(|p| matches!(p.data, SubpacketData::SignatureCreationTime(_)));
    config.unhashed_subpackets = vec![Subpacket::regular(SubpacketData::IssuerFingerprint(alice.primary_key.fingerprint())).unwrap()];
    let sig = config.sign(&alice.primary_key, &Password::empty(), SIGNED_PART.as_bytes()).unwrap();
    let outcome = verify(&with_signature(sig), "Carol <carol@example.org>", &ring(&[&alice, &bob]), LATER, yes);
    assert!(matches!(outcome, Some(Outcome::Valid { .. })), "{outcome:?}");
}

#[test]
fn a_malformed_signature_is_unreadable_not_changed() {
    use pgp::packet::Signature;
    let alice = secret(ALICE, Make::default());
    let config = config(&alice.primary_key, pgp::packet::SignatureType::Binary, made(&alice), vec![]);
    // Legacy EdDSA has two MPIs; one is no signature of its kind.
    let one = pgp::types::SignatureBytes::Mpis(vec![pgp::types::Mpi::from_slice(&[1; 32])]);
    let outcome = verify(&with_signature(Signature::from_config(config, [0, 0], one).unwrap()), ALICE, &ring(&[&alice]), LATER, yes);
    assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::Unreadable, .. })), "{outcome:?}");
}

#[test]
fn autocrypt_in_a_report_is_ignored() {
    let alice = secret(ALICE, Make::default());
    let keydata = base64(&pgp::ser::Serialize::to_bytes(&pgp::composed::SignedPublicKey::from(alice)).unwrap()).replace("\r\n", "");
    let message = |media: &str| {
        format!(
            "From: alice@example.org\r\nAutocrypt: addr=alice@example.org; keydata={keydata}\r\nContent-Type: {media}; boundary=r\r\n\r\n--r\r\n\r\nHi\r\n--r--\r\n"
        )
    };
    assert!(keys_in_message(message("multipart/mixed").as_bytes()).autocrypt.is_some());
    assert!(keys_in_message(message("multipart/report").as_bytes()).autocrypt.is_none());
}

#[test]
fn a_from_field_with_two_mailboxes_matches_no_address() {
    let alice = secret(ALICE, Make::default());
    let two = "alice@example.org, Mallory <mallory@example.org>";
    let outcome = verify(&signed_message(&alice, HashAlgorithm::Sha256), two, &ring(&[&alice]), LATER, yes);
    let Some(Outcome::Valid { why, .. }) = outcome else { panic!("{outcome:?}") };
    assert_eq!(why, NotVerified { key_unverified: true, address_differs: true, partly_signed: false });
    let keydata = base64(&pgp::ser::Serialize::to_bytes(&pgp::composed::SignedPublicKey::from(alice)).unwrap()).replace("\r\n", "");
    let message = format!("From: {two}\r\nAutocrypt: addr=alice@example.org; keydata={keydata}\r\n\r\nHi\r\n");
    assert!(keys_in_message(message.as_bytes()).autocrypt.is_none());
}

#[test]
fn a_signature_value_of_the_wrong_shape_is_unreadable() {
    // rPGP reads an Ed448 signature as whatever bytes are left; 10 aren't one.
    use pgp::crypto::public_key::PublicKeyAlgorithm;
    use pgp::packet::{Signature, SignatureConfig, SignatureType, Subpacket, SubpacketData};
    use pgp::types::{KeyDetails, Timestamp};
    let alice = secret(ALICE, Make::default());
    let mut config = SignatureConfig::v4(SignatureType::Binary, PublicKeyAlgorithm::Ed448, HashAlgorithm::Sha512);
    config.hashed_subpackets = vec![
        Subpacket::regular(SubpacketData::SignatureCreationTime(Timestamp::from_secs(made(&alice)))).unwrap(),
        Subpacket::regular(SubpacketData::IssuerFingerprint(alice.primary_key.fingerprint())).unwrap(),
    ];
    let sig = Signature::from_config(config, [0, 0], pgp::types::SignatureBytes::Native(vec![1u8; 10].into())).unwrap();
    let outcome = verify(&with_signature(sig), ALICE, &KeyRing::new(), LATER, yes);
    assert!(matches!(outcome, Some(Outcome::CantCheck { why: CantCheck::Unreadable, .. })), "{outcome:?}");
}

#[test]
fn keys_claiming_the_address_cant_push_out_the_key_the_signature_hints_at() {
    use pgp::packet::{Subpacket, SubpacketData};
    use pgp::types::{KeyDetails, Password};
    let alice = secret(ALICE, Make::default());
    // Eight keys claim Alice's address, and come before hers.
    let mut keys: KeyRing = (50..58).map(|seed| public(&secret(ALICE, Make { seed, ..Make::default() }))).collect();
    keys.insert(public(&alice));
    // Her signature names her only in the unhashed area, as some writers do.
    let mut config = config(&alice.primary_key, pgp::packet::SignatureType::Binary, made(&alice), vec![]);
    config.hashed_subpackets.retain(|p| matches!(p.data, SubpacketData::SignatureCreationTime(_)));
    config.unhashed_subpackets = vec![Subpacket::regular(SubpacketData::IssuerFingerprint(alice.primary_key.fingerprint())).unwrap()];
    let sig = config.sign(&alice.primary_key, &Password::empty(), SIGNED_PART.as_bytes()).unwrap();
    let outcome = verify(&with_signature(sig), ALICE, &keys, LATER, yes);
    assert!(matches!(outcome, Some(Outcome::Verified(_))), "{outcome:?}");
}

#[test]
fn running_out_of_checks_before_the_signers_key_is_tried_is_unreadable() {
    // Earlier signatures of the message used up its checks: whether this one is good isn't known.
    let alice = secret(ALICE, Make::default());
    let keys = ring(&[&alice]);
    let cx = context(&keys);
    cx.verifications.set(crate::check::MAX_VERIFICATIONS);
    let outcome = crate::check::check(&packets(&alice, b"data", made(&alice)), b"data", &cx);
    assert!(matches!(outcome, Outcome::CantCheck { why: CantCheck::Unreadable, .. }), "{outcome:?}");
}
