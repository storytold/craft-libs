//! A key and a PGP/MIME signature put together byte by byte from RFC 9580 (§4.2 packet headers,
//! §5.2 signatures, §5.5 keys, §5.2.4 what is hashed), with Ed25519 and SHA-256 and nothing of
//! rPGP's writing: so the checks here don't only agree with the library that made the samples.

use ed25519_dalek::Signer;
use sha1::Digest as _;

use crate::fixtures::{LATER, SIGNED_PART, base64, pgp_mime};
use crate::{KeyRing, Outcome, PublicKey, verify};

/// When the key and the signatures were made (2024-01-01).
const MADE: u32 = 1_704_067_200;
const USER_ID: &[u8] = b"Hand <hand@example.org>";

/// A packet: a new-format header (§4.2.1) with the tag, its length, then `body`.
fn packet(tag: u8, body: &[u8]) -> Vec<u8> {
    let mut out = vec![0xC0 | tag];
    match body.len() {
        n @ 0..=191 => out.push(n as u8),
        n => {
            let n = n - 192;
            out.extend_from_slice(&[(n >> 8) as u8 + 192, (n & 0xFF) as u8]);
        }
    }
    out.extend_from_slice(body);
    out
}

struct Hand {
    secret: ed25519_dalek::SigningKey,
    /// The version 4 public-key packet's body (§5.5.2): version, time, algorithm 27, the key.
    key_body: Vec<u8>,
    fingerprint: [u8; 20],
}

impl Hand {
    fn new() -> Hand {
        let secret = ed25519_dalek::SigningKey::from_bytes(&[7; 32]);
        let mut key_body = vec![4];
        key_body.extend_from_slice(&MADE.to_be_bytes());
        key_body.push(27);
        key_body.extend_from_slice(&secret.verifying_key().to_bytes());
        // §5.5.4.2: SHA-1 over 0x99, the two-octet length and the body.
        let mut sha1 = sha1::Sha1::new();
        sha1.update([0x99, 0, key_body.len() as u8]);
        sha1.update(&key_body);
        Hand { secret, fingerprint: sha1.finalize().into(), key_body }
    }

    /// A version 4 signature packet of `typ` over `prefix` (what comes before the signature's
    /// own fields in the hash), SHA-256, its time and issuer fingerprint hashed, `extra` too.
    fn signature(&self, typ: u8, prefix: &[u8], extra: &[u8]) -> Vec<u8> {
        let mut hashed = vec![5, 2];
        hashed.extend_from_slice(&MADE.to_be_bytes());
        hashed.extend_from_slice(&[22, 33, 4]);
        hashed.extend_from_slice(&self.fingerprint);
        hashed.extend_from_slice(extra);
        let mut fields = vec![4, typ, 27, 8, 0, hashed.len() as u8];
        fields.extend_from_slice(&hashed);
        // §5.2.4: the fields, then 0x04 0xFF and their length in four octets.
        let mut sha = sha2::Sha256::new();
        sha.update(prefix);
        sha.update(&fields);
        sha.update([4, 0xFF]);
        sha.update((fields.len() as u32).to_be_bytes());
        let digest = sha.finalize();
        let mut body = fields;
        body.extend_from_slice(&[0, 0]);
        body.extend_from_slice(&digest[..2]);
        // §5.2.3.4: Ed25519 signs the digest, and the signature is its 64 octets as they are.
        body.extend_from_slice(&self.secret.sign(&digest).to_bytes());
        packet(2, &body)
    }

    /// The certificate: key, User ID, and a positive certification that may sign and certify.
    fn certificate(&self) -> String {
        let mut prefix = vec![0x99, 0, self.key_body.len() as u8];
        prefix.extend_from_slice(&self.key_body);
        prefix.push(0xB4);
        prefix.extend_from_slice(&(USER_ID.len() as u32).to_be_bytes());
        prefix.extend_from_slice(USER_ID);
        let mut bytes = packet(6, &self.key_body);
        bytes.extend(packet(13, USER_ID));
        bytes.extend(self.signature(0x13, &prefix, &[2, 27, 0x03]));
        format!("-----BEGIN PGP PUBLIC KEY BLOCK-----\n\n{}\n-----END PGP PUBLIC KEY BLOCK-----\n", base64(&bytes))
    }

    /// A PGP/MIME message whose signature is over [`SIGNED_PART`] (a binary signature, §5.2.1.1).
    fn message(&self) -> Vec<u8> {
        let sig = self.signature(0x00, SIGNED_PART.as_bytes(), &[]);
        pgp_mime("hand@example.org", SIGNED_PART, &format!("-----BEGIN PGP SIGNATURE-----\n\n{}\n-----END PGP SIGNATURE-----", base64(&sig)))
    }
}

#[test]
fn a_key_and_signature_made_by_hand_from_the_specification_check() {
    let hand = Hand::new();
    let keys: KeyRing = PublicKey::parse(hand.certificate().as_bytes()).unwrap().into_iter().collect();
    let key = &keys.keys()[0];
    assert_eq!(key.fingerprint().as_bytes(), hand.fingerprint);
    assert_eq!(key.addresses(), ["hand@example.org".to_string()]);
    let yes = |_: &crate::Fingerprint, _: &str| true;
    let message = hand.message();
    let checked = verify(&message, "hand@example.org", &keys, LATER, yes).unwrap();
    let Outcome::Verified(signer) = checked.outcome else { panic!("{checked:?}") };
    assert_eq!((signer.signed, signer.algorithm, signer.hash), (u64::from(MADE), "Ed25519", "SHA-256"));
    // Kept with bare line feeds, it is checked in canonical form all the same.
    let lf = String::from_utf8(message.clone()).unwrap().replace("\r\n", "\n");
    assert!(matches!(verify(lf.as_bytes(), "hand@example.org", &keys, LATER, yes).unwrap().outcome, Outcome::Verified(_)));
    let changed = String::from_utf8(message).unwrap().replace("minutes", "invoice");
    assert!(matches!(verify(changed.as_bytes(), "hand@example.org", &keys, LATER, yes).unwrap().outcome, Outcome::Changed(_)));
}
