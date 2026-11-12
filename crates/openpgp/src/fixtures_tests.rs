//! Keys, signatures and messages made in the tests: synthetic `example.org` identities only.

use pgp::composed::{
    ArmorOptions, CleartextSignedMessage, DetachedSignature, EncryptionCaps, KeyType, SecretKeyParamsBuilder, SignedPublicKey, SignedSecretKey,
    SubkeyParamsBuilder,
};
use pgp::crypto::hash::HashAlgorithm;
use pgp::packet::{SignatureConfig, SignatureType, Subpacket, SubpacketData};
use pgp::ser::Serialize;
use pgp::types::{KeyDetails, KeyVersion, Password, SigningKey, Timestamp};
use rand::SeedableRng;
use rand::rngs::StdRng;

use crate::PublicKey;

/// RFC 9580 Appendix A.3: the sample version 6 certificate (no User ID).
pub(crate) const RFC9580_A3_CERT: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----

xioGY4d/4xsAAAAg+U2nu0jWCmHlZ3BqZYfQMxmZu52JGggkLq2EVD34laPCsQYf
GwoAAABCBYJjh3/jAwsJBwUVCg4IDAIWAAKbAwIeCSIhBssYbE8GCaaX5NUt+mxy
KwwfHifBilZwj2Ul7Ce62azJBScJAgcCAAAAAK0oIBA+LX0ifsDm185Ecds2v8lw
gyU2kCcUmKfvBXbAf6rhRYWzuQOwEn7E/aLwIwRaLsdry0+VcallHhSu4RN6HWaE
QsiPlR4zxP/TP7mhfVEe7XWPxtnMUMtf15OyA51YBM4qBmOHf+MZAAAAIIaTJINn
+eUBXbki+PSAld2nhJh/LVmFsS+60WyvXkQ1wpsGGBsKAAAALAWCY4d/4wKbDCIh
BssYbE8GCaaX5NUt+mxyKwwfHifBilZwj2Ul7Ce62azJAAAAAAQBIKbpGG2dWTX8
j+VjFM21J0hqWlEg+bdiojWnKfA5AQpWUWtnNwDEM0g12vYxoWM8Y81W+bHBw805
I8kWVkXU6vFOi+HWvv/ira7ofJu16NnoUkhclkUrk0mXubZvyl4GBg==
-----END PGP PUBLIC KEY BLOCK-----
";

/// RFC 9580 Appendix A.6: the sample cleartext-signed message, signed by [`RFC9580_A3_CERT`].
pub(crate) const RFC9580_A6_CLEARTEXT: &str = "-----BEGIN PGP SIGNED MESSAGE-----

What we need from the grocery store:

- - tofu
- - vegetables
- - noodles

-----BEGIN PGP SIGNATURE-----

wpgGARsKAAAAKQWCY5ijYyIhBssYbE8GCaaX5NUt+mxyKwwfHifBilZwj2Ul7Ce6
2azJAAAAAGk2IHZJX1AhiJD39eLuPBgiUU9wUA9VHYblySHkBONKU/usJ9BvuAqo
/FvLFuGWMbKAdA+epq7V4HOtAPlBWmU8QOd6aud+aSunHQaaEJ+iTFjP2OMW0KBr
NK2ay45cX1IVAQ==
-----END PGP SIGNATURE-----
";

/// The primary fingerprint of [`RFC9580_A3_CERT`].
pub(crate) const RFC9580_A3_FINGERPRINT: &str = "CB186C4F0609A697E4D52DFA6C722B0C1F1E27C18A56708F6525EC27BAD9ACC9";

/// A key for the property tests to look signatures up in.
pub(crate) static ALICE_FOR_PROPS: std::sync::LazyLock<Vec<PublicKey>> =
    std::sync::LazyLock::new(|| vec![public(&secret("A <a@example.org>", Make::default()))]);

/// A time after every signature made in the tests (2100-01-01).
pub(crate) const LATER: u64 = 4_102_444_800;

/// How a test key is made.
#[derive(Clone, Copy)]
pub(crate) struct Make {
    pub(crate) seed: u64,
    /// A subkey signs, not the primary key.
    pub(crate) subkey: bool,
    pub(crate) v6: bool,
    /// RSA of this many bits rather than EdDSA: rPGP makes MD5 and SHA-1 signatures only with RSA.
    pub(crate) rsa: Option<usize>,
    /// A second User ID, `Alias <alias@example.org>`.
    pub(crate) alias: bool,
    /// An RSA encryption subkey of 2048 bits besides.
    pub(crate) rsa_encryption: bool,
}

impl Default for Make {
    fn default() -> Make {
        Make { seed: 1, subkey: false, v6: false, rsa: None, alias: false, rsa_encryption: false }
    }
}

/// A secret key for `user_id` (an `example.org` identity).
pub(crate) fn secret(user_id: &str, make: Make) -> SignedSecretKey {
    let rng = StdRng::seed_from_u64(make.seed);
    let version = if make.v6 { KeyVersion::V6 } else { KeyVersion::V4 };
    let key_type = match (make.rsa, make.v6) {
        (Some(bits), _) => KeyType::Rsa(bits as u32),
        (None, true) => KeyType::Ed25519,
        (None, false) => KeyType::Ed25519Legacy,
    };
    let mut params = SecretKeyParamsBuilder::default();
    params.version(version).key_type(key_type.clone()).can_certify(true).can_sign(!make.subkey).primary_user_id(user_id.into());
    if make.alias {
        params.user_ids(vec!["Alias <alias@example.org>".into()]);
    }
    let mut subkeys = Vec::new();
    if make.subkey {
        let mut sub = SubkeyParamsBuilder::default();
        sub.version(version).key_type(key_type).can_sign(true).can_encrypt(EncryptionCaps::None);
        subkeys.push(sub.build().unwrap());
    }
    if make.rsa_encryption {
        let mut sub = SubkeyParamsBuilder::default();
        sub.version(version).key_type(KeyType::Rsa(2048)).can_sign(false).can_encrypt(EncryptionCaps::All);
        subkeys.push(sub.build().unwrap());
    }
    if !subkeys.is_empty() {
        params.subkeys(subkeys);
    }
    params.build().unwrap().generate(rng).unwrap()
}

/// The public half of `secret`, read back as the crate reads keys.
pub(crate) fn public(secret: &SignedSecretKey) -> PublicKey {
    let bytes = SignedPublicKey::from(secret.clone()).to_bytes().unwrap();
    PublicKey::parse(&bytes).unwrap().remove(0)
}

/// The armoured public half of `secret`.
pub(crate) fn armoured_public(secret: &SignedSecretKey) -> String {
    SignedPublicKey::from(secret.clone()).to_armored_string(ArmorOptions::default()).unwrap()
}

/// The key that signs: rPGP takes either kind through this box alone.
#[allow(clippy::redundant_allocation)]
pub(crate) fn signer(secret: &SignedSecretKey) -> Box<&dyn SigningKey> {
    match secret.secret_subkeys.first() {
        Some(sub) => Box::new(&sub.key),
        None => Box::new(&secret.primary_key),
    }
}

/// An armoured detached signature over `data` by `secret` (its signing subkey when it has one).
pub(crate) fn detached(secret: &SignedSecretKey, data: &[u8], hash: HashAlgorithm, text: bool) -> String {
    let rng = StdRng::seed_from_u64(7);
    let key = signer(secret);
    let sig = if text {
        DetachedSignature::sign_text_data(rng, &key, &Password::empty(), hash, data).unwrap()
    } else {
        DetachedSignature::sign_binary_data(rng, &key, &Password::empty(), hash, data).unwrap()
    };
    sig.to_armored_string(ArmorOptions::default()).unwrap()
}

/// When `secret`'s primary key was made, in seconds since 1970.
pub(crate) fn made(secret: &SignedSecretKey) -> u32 {
    secret.primary_key.created_at().as_secs()
}

/// A signature of `typ` by `key` made at `at`, its issuer's fingerprint and `extra` in its
/// hashed area: to be signed.
pub(crate) fn config(key: &impl SigningKey, typ: SignatureType, at: u32, extra: Vec<SubpacketData>) -> SignatureConfig {
    let mut config = SignatureConfig::from_key(StdRng::seed_from_u64(11), key, typ).unwrap();
    config.hashed_subpackets = [SubpacketData::SignatureCreationTime(Timestamp::from_secs(at)), SubpacketData::IssuerFingerprint(key.fingerprint())]
        .into_iter()
        .chain(extra)
        .map(|d| Subpacket::regular(d).unwrap())
        .collect();
    config
}

/// A PGP/MIME message from alice@example.org over [`SIGNED_PART`], signed by `secret`'s
/// signing key at `at`.
pub(crate) fn signed_at(secret: &SignedSecretKey, at: u32) -> Vec<u8> {
    let key = signer(secret);
    let sig = config(&key, SignatureType::Binary, at, vec![]).sign(&key, &Password::empty(), SIGNED_PART.as_bytes()).unwrap();
    pgp_mime("alice@example.org", SIGNED_PART, &DetachedSignature::new(sig).to_armored_string(ArmorOptions::default()).unwrap())
}

/// The keys in `key`, read as the crate reads them, as a ring.
pub(crate) fn ring_of(key: &SignedPublicKey) -> crate::KeyRing {
    PublicKey::parse(&key.to_bytes().unwrap()).unwrap().into_iter().collect()
}

/// A cleartext-signed `text` by `secret`.
pub(crate) fn cleartext(secret: &SignedSecretKey, text: &str) -> String {
    let rng = StdRng::seed_from_u64(9);
    let key = signer(secret);
    CleartextSignedMessage::sign(rng, text, &key, &Password::empty()).unwrap().to_armored_string(ArmorOptions::default()).unwrap()
}

/// The part a PGP/MIME message signs, CR LF throughout.
pub(crate) const SIGNED_PART: &str = "Content-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 7bit\r\n\r\nHello Bob,\r\n\r\nthe minutes are attached.\r\n-- \r\nAlice\r\n";

/// A PGP/MIME message from `from` whose first part is `part` and whose signature is `armour`.
pub(crate) fn pgp_mime(from: &str, part: &str, armour: &str) -> Vec<u8> {
    format!(
        "From: Alice <{from}>\r\nTo: bob@example.org\r\nSubject: Minutes\r\nMIME-Version: 1.0\r\nContent-Type: multipart/signed; micalg=pgp-sha256;\r\n protocol=\"application/pgp-signature\"; boundary=\"=-b1\"\r\n\r\nThis is an OpenPGP/MIME signed message.\r\n--=-b1\r\n{part}\r\n--=-b1\r\nContent-Type: application/pgp-signature; name=\"signature.asc\"\r\nContent-Disposition: attachment; filename=\"signature.asc\"\r\n\r\n{armour}\r\n--=-b1--\r\n"
    )
    .into_bytes()
}

/// A PGP/MIME message from alice@example.org signed by `secret` over [`SIGNED_PART`].
pub(crate) fn signed_message(secret: &SignedSecretKey, hash: HashAlgorithm) -> Vec<u8> {
    let armour = detached(secret, SIGNED_PART.as_bytes(), hash, false);
    pgp_mime("alice@example.org", SIGNED_PART, &armour)
}

/// A one-part message from `from` with `body` as its text, `encoding` its transfer encoding.
pub(crate) fn plain(from: &str, encoding: &str, body: &str) -> Vec<u8> {
    format!("From: {from}\r\nSubject: Note\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: {encoding}\r\n\r\n{body}")
        .into_bytes()
}

/// `bytes` in base64, a line break after every 76 characters.
pub(crate) fn base64(bytes: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for (n, chunk) in bytes.chunks(3).enumerate() {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let v = (u32::from(b[0]) << 16) | (u32::from(b[1]) << 8) | u32::from(b[2]);
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(ALPHABET[((v >> (18 - 6 * i)) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
        if n % 19 == 18 {
            out.push_str("\r\n");
        }
    }
    out
}
