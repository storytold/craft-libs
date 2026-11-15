//! Keys over time: revocations that are never lost, expiry of the primary key and of
//! self-signatures, and the cost of reading keys.

use pgp::composed::{SignedPublicKey, SignedSecretKey};
use pgp::packet::{RevocationCode, SignatureType, SubpacketData};
use pgp::ser::Serialize;
use pgp::types::{Duration, Password, Tag};

use crate::fixtures::*;
use crate::keys::Budget;
use crate::{CantCheck, Fingerprint, KeyRing, NotVerified, Outcome, PublicKey, verify};

const ALICE: &str = "Alice <alice@example.org>";

fn yes(_: &Fingerprint, _: &str) -> bool {
    true
}

fn outcome(message: &[u8], from: &str, keys: &KeyRing) -> Outcome {
    verify(message, from, keys, LATER, yes).unwrap().outcome
}

fn revoked(outcome: &Outcome) -> bool {
    matches!(outcome, Outcome::CantCheck { why: CantCheck::KeyRevoked, .. })
}

/// `secret`'s public key revoked at `at`, for `reason` (none: no reason given).
fn key_revoked(secret: &SignedSecretKey, at: u32, reason: Option<RevocationCode>) -> SignedPublicKey {
    let mut key = SignedPublicKey::from(secret.clone());
    let extra = reason.map(|r| SubpacketData::RevocationReason(r, Vec::new().into())).into_iter().collect();
    let sig = config(&secret.primary_key, SignatureType::KeyRevocation, at, extra)
        .sign_key(&secret.primary_key, &Password::empty(), &key.primary_key)
        .unwrap();
    key.details.revocation_signatures.push(sig);
    key
}

/// `secret`'s public key with its first subkey revoked at `at`.
fn subkey_revoked(secret: &SignedSecretKey, at: u32) -> SignedPublicKey {
    let mut key = SignedPublicKey::from(secret.clone());
    let sig = config(&secret.primary_key, SignatureType::SubkeyRevocation, at, vec![])
        .sign_subkey_binding(&secret.primary_key, &key.primary_key, &Password::empty(), &key.public_subkeys[0].key)
        .unwrap();
    key.public_subkeys[0].signatures.push(sig);
    key
}

/// `secret`'s public key with a new self-signature on its User ID number `user`, made at `at`.
fn recertified(secret: &SignedSecretKey, key: &mut SignedPublicKey, user: usize, at: u32, extra: Vec<SubpacketData>) {
    let id = key.details.users[user].id.clone();
    let sig = config(&secret.primary_key, SignatureType::CertPositive, at, extra)
        .sign_certification(&secret.primary_key, &key.primary_key, &Password::empty(), Tag::UserId, &id)
        .unwrap();
    key.details.users[user].signatures.push(sig);
}

#[test]
fn a_copy_without_the_revocation_never_undoes_it() {
    let alice = secret(ALICE, Make::default());
    let mut keys = ring_of(&key_revoked(&alice, made(&alice), None));
    assert!(!keys.insert(public(&alice)), "nothing new in the copy");
    assert!(keys.keys()[0].revoked());
    assert!(revoked(&outcome(&signed_message(&alice, pgp::crypto::hash::HashAlgorithm::Sha256), ALICE, &keys)));
}

#[test]
fn a_copy_that_brings_a_revocation_always_counts() {
    let alice = secret(ALICE, Make::default());
    let mut keys = ring_of(&SignedPublicKey::from(alice.clone()));
    let copy = PublicKey::parse(&key_revoked(&alice, made(&alice), None).to_bytes().unwrap()).unwrap().remove(0);
    assert!(keys.insert(copy));
    assert!(keys.keys()[0].revoked());
}

#[test]
fn a_copy_with_a_subkey_revocation_stripped_never_undoes_it() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let mut keys = ring_of(&subkey_revoked(&alice, made(&alice)));
    let message = signed_at(&alice, made(&alice) + 100);
    assert!(revoked(&outcome(&message, ALICE, &keys)));
    keys.insert(public(&alice));
    assert!(revoked(&outcome(&message, ALICE, &keys)));
}

#[test]
fn the_primary_keys_expiry_and_revocation_hold_for_its_subkeys() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let t = made(&alice);
    // The newest self-signature says the key expires 50 seconds after it was made.
    let mut key = SignedPublicKey::from(alice.clone());
    recertified(&alice, &mut key, 0, t + 10, vec![SubpacketData::KeyExpirationTime(Duration::from_secs(50))]);
    let keys = ring_of(&key);
    assert!(matches!(outcome(&signed_at(&alice, t + 20), ALICE, &keys), Outcome::Verified(_)));
    assert!(matches!(outcome(&signed_at(&alice, t + 100), ALICE, &keys), Outcome::CantCheck { why: CantCheck::KeyExpired, .. }));
    // A revoked primary key undoes its subkey's signatures.
    assert!(revoked(&outcome(&signed_at(&alice, t + 20), ALICE, &ring_of(&key_revoked(&alice, t + 50, None)))));
}

#[test]
fn a_soft_revocation_undoes_only_what_was_signed_after_it() {
    let alice = secret(ALICE, Make::default());
    let t = made(&alice);
    for reason in [RevocationCode::KeySuperseded, RevocationCode::KeyRetired] {
        let keys = ring_of(&key_revoked(&alice, t + 50, Some(reason)));
        assert!(matches!(outcome(&signed_at(&alice, t + 20), ALICE, &keys), Outcome::Verified(_)));
        assert!(revoked(&outcome(&signed_at(&alice, t + 100), ALICE, &keys)));
    }
    // Compromised, or no reason: everything it ever signed.
    for reason in [Some(RevocationCode::KeyCompromised), None] {
        let keys = ring_of(&key_revoked(&alice, t + 50, reason));
        assert!(revoked(&outcome(&signed_at(&alice, t + 20), ALICE, &keys)));
    }
}

#[test]
fn an_expired_user_id_self_signature_takes_its_address_away() {
    let alice = secret(ALICE, Make { alias: true, ..Make::default() });
    let t = made(&alice);
    let mut key = SignedPublicKey::from(alice.clone());
    let alias = key.details.users.iter().position(|u| u.id.id().starts_with(b"Alias")).unwrap();
    // The alias's newest self-signature expires 50 seconds after it was made; then a newer one
    // on the primary User ID that doesn't, so that the key itself stays valid.
    recertified(&alice, &mut key, alias, t + 10, vec![SubpacketData::SignatureExpirationTime(Duration::from_secs(50))]);
    recertified(&alice, &mut key, 1 - alias, t + 20, vec![]);
    let keys = ring_of(&key);
    let from_alias = |at| outcome(&signed_at(&alice, at), "alias@example.org", &keys);
    assert!(matches!(from_alias(t + 30), Outcome::Verified(_)));
    let Outcome::Valid { why, .. } = from_alias(t + 100) else { panic!() };
    assert_eq!(why, NotVerified { address_differs: true, ..NotVerified::default() });
}

#[test]
fn an_expired_subkey_binding_stops_the_subkey() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let t = made(&alice);
    let mut key = SignedPublicKey::from(alice.clone());
    let back = key.public_subkeys[0].signatures[0].embedded_signature().unwrap().clone();
    let mut flags = pgp::packet::KeyFlags::default();
    flags.set_sign(true);
    let extra = vec![
        SubpacketData::KeyFlags(flags),
        SubpacketData::SignatureExpirationTime(Duration::from_secs(50)),
        SubpacketData::EmbeddedSignature(Box::new(back)),
    ];
    let binding = config(&alice.primary_key, SignatureType::SubkeyBinding, t + 10, extra)
        .sign_subkey_binding(&alice.primary_key, &key.primary_key, &Password::empty(), &key.public_subkeys[0].key)
        .unwrap();
    key.public_subkeys[0].signatures.push(binding);
    let keys = ring_of(&key);
    assert!(matches!(outcome(&signed_at(&alice, t + 30), ALICE, &keys), Outcome::Verified(_)));
    assert!(matches!(outcome(&signed_at(&alice, t + 100), ALICE, &keys), Outcome::CantCheck { why: CantCheck::KeyExpired, .. }));
}

#[test]
fn rsa_under_2048_bits_is_too_weak_to_trust() {
    use pgp::composed::SignedKeyDetails;
    use pgp::crypto::public_key::PublicKeyAlgorithm;
    use pgp::packet::{PubKeyInner, PublicKey as Packet, SecretKey, UserId};
    use pgp::types::{KeyVersion, PlainSecretParams, PublicParams, RsaPublicParams, SecretParams, SignedUser, Timestamp};
    use rand::SeedableRng;
    // rPGP won't make a key this small, so it is put together from one the rsa crate makes.
    let private = rsa::RsaPrivateKey::new(&mut rand::rngs::StdRng::seed_from_u64(3), 1024).unwrap();
    let t = 1_704_067_200;
    let public_params = PublicParams::RSA(RsaPublicParams { key: private.to_public_key() });
    let inner = PubKeyInner::new(KeyVersion::V4, PublicKeyAlgorithm::RSA, Timestamp::from_secs(t), None, public_params).unwrap();
    let packet = Packet::from_inner(inner).unwrap();
    let secret = SecretKey::new(packet.clone(), SecretParams::Plain(PlainSecretParams::RSA(private.into()))).unwrap();
    let id = UserId::from_str(Default::default(), ALICE).unwrap();
    let mut flags = pgp::packet::KeyFlags::default();
    flags.set_sign(true);
    flags.set_certify(true);
    let cert = config(&secret, SignatureType::CertPositive, t, vec![SubpacketData::KeyFlags(flags)])
        .sign_certification(&secret, &packet, &Password::empty(), Tag::UserId, &id)
        .unwrap();
    let key = SignedPublicKey::new(packet, SignedKeyDetails::new(vec![], vec![], vec![SignedUser::new(id, vec![cert])], vec![]), vec![]);
    let keys = ring_of(&key);
    let sig = config(&secret, SignatureType::Binary, t + 1, vec![]).sign(&secret, &Password::empty(), SIGNED_PART.as_bytes()).unwrap();
    let armour = pgp::composed::DetachedSignature::new(sig).to_armored_string(Default::default()).unwrap();
    let out = outcome(&pgp_mime("alice@example.org", SIGNED_PART, &armour), ALICE, &keys);
    assert!(matches!(out, Outcome::CantCheck { why: CantCheck::WeakAlgorithm("RSA under 2048 bits"), .. }), "{out:?}");
}

#[test]
fn reading_keys_stops_when_its_budget_or_room_runs_out() {
    let block = armoured_public(&secret(ALICE, Make::default()));
    let many = block.repeat(20);
    // At most 16 keys from one call, however many blocks.
    assert_eq!(PublicKey::parse(many.as_bytes()).unwrap().len(), 16);
    // A budget of checks shared by everything one call reads: the key that would overspend it is
    // left out, whole, and so is every key after it.
    let mut budget = Budget::new(2);
    let read = PublicKey::parse_with(block.repeat(4).as_bytes(), &mut budget).unwrap();
    assert_eq!(read.len(), 2);
    assert!(budget.starved);
}

/// A signature of `typ` that names Alice's key as its issuer but is made by Mallory's: junk
/// anybody can add to a copy of Alice's key.
fn forged(
    alice: &SignedSecretKey,
    mallory: &SignedSecretKey,
    typ: SignatureType,
    at: u32,
    extra: Vec<SubpacketData>,
) -> pgp::packet::SignatureConfig {
    use pgp::types::KeyDetails;
    let mut config = config(&mallory.primary_key, typ, at, extra);
    for p in &mut config.hashed_subpackets {
        if matches!(p.data, SubpacketData::IssuerFingerprint(_)) {
            *p = pgp::packet::Subpacket::regular(SubpacketData::IssuerFingerprint(alice.primary_key.fingerprint())).unwrap();
        }
    }
    config
}

/// Inserts `bytes` as a copy of a key, when they read as one.
fn offer(keys: &mut KeyRing, bytes: &[u8]) {
    for key in PublicKey::parse(bytes).unwrap_or_default() {
        keys.insert(key);
    }
}

#[test]
fn a_copy_with_junk_subkey_bindings_from_the_future_never_pushes_out_the_real_one() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let mallory = secret("Mallory <mallory@example.org>", Make { seed: 6, ..Make::default() });
    let mut copy = SignedPublicKey::from(alice.clone());
    let mut flags = pgp::packet::KeyFlags::default();
    flags.set_sign(true);
    for i in 0..64 {
        // Dated 2096.
        let junk = forged(&alice, &mallory, SignatureType::SubkeyBinding, 4_000_000_000 + i, vec![SubpacketData::KeyFlags(flags.clone())])
            .sign_subkey_binding(&mallory.primary_key, &copy.primary_key, &Password::empty(), &copy.public_subkeys[0].key)
            .unwrap();
        copy.public_subkeys[0].signatures.push(junk);
    }
    let junk = copy.to_bytes().unwrap();
    let message = signed_at(&alice, made(&alice) + 10);
    // Either way round, the real key stays whole, and nothing of the junk is kept.
    for first_real in [true, false] {
        let mut keys = KeyRing::new();
        if first_real {
            keys.insert(public(&alice));
            offer(&mut keys, &junk);
        } else {
            offer(&mut keys, &junk);
            keys.insert(public(&alice));
        }
        assert!(matches!(outcome(&message, ALICE, &keys), Outcome::Verified(_)), "real first: {first_real}");
        assert_eq!(keys.keys()[0].to_bytes(), public(&alice).to_bytes());
    }
}

#[test]
fn a_copy_that_costs_more_checks_than_a_key_may_is_refused_not_cut_short() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let mallory = secret("Mallory <mallory@example.org>", Make { seed: 6, ..Make::default() });
    let mut copy = SignedPublicKey::from(alice.clone());
    // Junk User IDs with junk certifications before the subkey: checking them all would leave
    // no checks for the subkey's binding.
    for n in 0..5 {
        let id = pgp::packet::UserId::from_str(Default::default(), format!("Alice {n} <alice@example.org>")).unwrap();
        let sigs = (0..110)
            .map(|i| {
                forged(&alice, &mallory, SignatureType::CertPositive, made(&alice) + i, vec![])
                    .sign_certification(&mallory.primary_key, &copy.primary_key, &Password::empty(), Tag::UserId, &id)
                    .unwrap()
            })
            .collect();
        copy.details.users.push(pgp::types::SignedUser::new(id, sigs));
    }
    let junk = copy.to_bytes().unwrap();
    assert!(PublicKey::parse(&junk).is_err(), "a key not checked whole is no key");
    let mut keys = KeyRing::new();
    keys.insert(public(&alice));
    offer(&mut keys, &junk);
    assert!(matches!(outcome(&signed_at(&alice, made(&alice) + 10), ALICE, &keys), Outcome::Verified(_)));
}

#[test]
fn a_key_whose_revocations_or_the_calls_budget_run_out_is_refused() {
    let alice = secret(ALICE, Make { subkey: true, ..Make::default() });
    let mallory = secret("Mallory <mallory@example.org>", Make { seed: 6, ..Make::default() });
    // More revocations than are checked: one left unchecked could be the real one.
    let mut copy = SignedPublicKey::from(alice.clone());
    for i in 0..257 {
        let junk = forged(&alice, &mallory, SignatureType::KeyRevocation, made(&alice) + i, vec![])
            .sign_key(&mallory.primary_key, &Password::empty(), &copy.primary_key)
            .unwrap();
        copy.details.revocation_signatures.push(junk);
    }
    assert!(PublicKey::parse(&copy.to_bytes().unwrap()).is_err());
    // The call's budget runs out after the User ID, before the subkey.
    let bytes = SignedPublicKey::from(alice).to_bytes().unwrap();
    assert!(PublicKey::parse_with(&bytes, &mut Budget::new(1)).is_err());
    assert!(PublicKey::parse_with(&bytes, &mut Budget::new(3)).is_ok());
}

#[test]
fn a_user_id_certified_again_after_its_revocation_is_the_keys_again() {
    let alice = secret(ALICE, Make::default());
    let t = made(&alice);
    let mut key = SignedPublicKey::from(alice.clone());
    let id = key.details.users[0].id.clone();
    let revocation = config(&alice.primary_key, SignatureType::CertRevocation, t + 10, vec![])
        .sign_certification(&alice.primary_key, &key.primary_key, &Password::empty(), Tag::UserId, &id)
        .unwrap();
    key.details.users[0].signatures.push(revocation);
    let revoked = ring_of(&key);
    assert!(revoked.keys()[0].addresses().is_empty(), "revoked");
    // Certified again in the same second as the revocation: which came last isn't known, so it
    // stays revoked.
    let mut tie = key.clone();
    recertified(&alice, &mut tie, 0, t + 10, vec![]);
    assert!(ring_of(&tie).keys()[0].addresses().is_empty(), "same second");
    recertified(&alice, &mut key, 0, t + 20, vec![]);
    assert_eq!(ring_of(&key).keys()[0].addresses(), ["alice@example.org".to_string()]);
}

#[test]
fn a_user_id_longer_than_any_real_one_is_left_out_unhashed() {
    let alice = secret(ALICE, Make::default());
    let mut key = SignedPublicKey::from(alice.clone());
    let long = format!("{} <long@example.org>", "x".repeat(5_000));
    let id = pgp::packet::UserId::from_str(Default::default(), &long).unwrap();
    let cert = config(&alice.primary_key, SignatureType::CertPositive, made(&alice), vec![])
        .sign_certification(&alice.primary_key, &key.primary_key, &Password::empty(), Tag::UserId, &id)
        .unwrap();
    key.details.users.push(pgp::types::SignedUser::new(id, vec![cert]));
    let keys = ring_of(&key);
    assert_eq!(keys.keys()[0].addresses(), ["alice@example.org".to_string()]);
}
