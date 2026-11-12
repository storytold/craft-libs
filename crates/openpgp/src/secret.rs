//! A transferable secret key (RFC 9580 §10.2) read for the window to import: what it
//! is (its fingerprint and addresses, its public half for the key ring), whether every secret
//! part of it is protected by a passphrase, whether any part is RSA, and whether it can sign.
//! Nothing secret is used here (an rPGP advisory concerns using an RSA private key, which this
//! never does), and the secret bytes are not kept. The one thing decrypted is a trial: each
//! secret part of a key with no RSA part is tried with an empty passphrase (the passphrase's key
//! derivation and a symmetric decryption, within caps), so that a key "protected" by an empty
//! one is told as unprotected. The file is hostile: the caps of the Public keys' reader hold, a
//! file holds exactly one secret key, and nothing here panics on any bytes.

use pgp::composed::{Deserializable, SignedSecretKey};
use pgp::types::{KeyDetails, Password, S2kParams, SecretParams, StringToKey};

use crate::Error;
use crate::armour::{self, Kind};
use crate::keys::{Budget, Fingerprint, MAX_CHECKS_PER_CALL, MAX_KEY_BYTES, PublicKey, packet_tags, packets};

/// How many packets a secret key may be made of.
const MAX_PACKETS: usize = 4_096;
/// The packet tags of a secret key and a secret subkey (RFC 9580 §5.5.1.3, §5.5.1.4).
const SECRET_KEY: u8 = 5;
const SECRET_SUBKEY: u8 = 7;
/// The most secret parts (the primary key and its subkeys) a key to import may have: each is
/// tried with an empty passphrase, which costs a key derivation.
pub const MAX_SECRET_PARTS: usize = 16;
/// The most memory an Argon2 derivation of that trial may ask for, as RFC 9580's exponent of
/// KiB (2^18 KiB: 256 MiB), and the most passes. A part asking for more isn't tried, and is
/// taken as protected: nobody sets so costly a derivation for an empty passphrase.
const MAX_ARGON2_MEMORY: u8 = 18;
const MAX_ARGON2_PASSES: u8 = 8;

/// What a secret key is, read without using anything secret.
#[derive(Clone, Debug)]
pub struct SecretKeyInfo {
    /// The primary key's fingerprint.
    pub fingerprint: Fingerprint,
    /// The addresses of the User IDs it certified itself, lower case.
    pub addresses: Vec<String>,
    /// Its public half, its self-signatures checked: for the key ring, and to keep for its
    /// addresses.
    pub public: PublicKey,
    /// Whether every secret part of it (the primary key's and each subkey's) is protected by a
    /// passphrase that isn't empty.
    pub protected: bool,
    /// Whether any secret part of it (the primary key or a subkey, whatever it is for) is RSA,
    /// whose private-key operations never run in this process.
    pub any_rsa: bool,
    /// Whether it can make Digital signatures: a key of it that may sign, isn't revoked, isn't
    /// of an algorithm no longer safe, and has its secret part here.
    pub can_sign: bool,
    /// The algorithm of the key it would sign with (the first that can): whether it may be
    /// used here. `None` when it can't sign.
    pub signing_kind: Option<SigningKind>,
}

/// What kind of key signs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SigningKind {
    Rsa,
    /// EdDSA: Ed25519 (either way of writing it) or Ed448.
    EdDsa,
    Ecdsa,
    /// Any other (DSA, or one not known).
    Other,
}

impl SigningKind {
    fn of(algorithm: pgp::crypto::public_key::PublicKeyAlgorithm) -> SigningKind {
        use pgp::crypto::public_key::PublicKeyAlgorithm as A;
        match algorithm {
            A::RSA | A::RSASign | A::RSAEncrypt => SigningKind::Rsa,
            A::EdDSALegacy | A::Ed25519 | A::Ed448 => SigningKind::EdDsa,
            A::ECDSA => SigningKind::Ecdsa,
            _ => SigningKind::Other,
        }
    }
}

/// Reads the transferable secret key in `bytes`: armoured (`-----BEGIN PGP PRIVATE KEY
/// BLOCK-----`) or binary. [`Error::Unreadable`] when there is no secret key, or its
/// self-signatures don't check; [`Error::TooLarge`] past the size of any key or past
/// [`MAX_SECRET_PARTS`]; [`Error::SeveralKeys`] when the file holds more than one key (two
/// armour blocks, two secret key packets, or anything after the key's packets): the whole file
/// is what is kept, so the key checked must be all there is in it.
pub fn read(bytes: &[u8]) -> Result<SecretKeyInfo, Error> {
    if bytes.windows(15).filter(|w| *w == b"-----BEGIN PGP ").nth(1).is_some() {
        return Err(Error::SeveralKeys);
    }
    let block = match armour::blocks(bytes, Kind::PrivateKey, 1, MAX_KEY_BYTES).into_iter().next() {
        Some(block) => block,
        None if bytes.len() > MAX_KEY_BYTES => return Err(Error::TooLarge),
        None => bytes.to_vec(),
    };
    let (tags, end) = packets(&block, MAX_PACKETS.saturating_add(1));
    if tags.len() > MAX_PACKETS || tags.first() != Some(&SECRET_KEY) {
        return Err(Error::Unreadable);
    }
    if tags.iter().filter(|t| **t == SECRET_KEY).count() > 1 || end != block.len() {
        return Err(Error::SeveralKeys);
    }
    if tags.iter().filter(|t| matches!(**t, SECRET_KEY | SECRET_SUBKEY)).count() > MAX_SECRET_PARTS {
        return Err(Error::TooLarge);
    }
    let parsed = crate::guard(|| SignedSecretKey::from_bytes_many(block.as_slice()).map(|mut it| (it.next(), it.next().is_some())));
    let key = match parsed {
        Some(Ok((Some(Ok(key)), false))) => key,
        Some(Ok((Some(Ok(_)), true))) => return Err(Error::SeveralKeys),
        _ => return Err(Error::Unreadable),
    };
    crate::guard(|| info(&key)).flatten().ok_or(Error::Unreadable)
}

fn info(key: &SignedSecretKey) -> Option<SecretKeyInfo> {
    let public = PublicKey::checked(key.to_public_key(), &mut Budget::new(MAX_CHECKS_PER_CALL))?;
    let encrypted = key.primary_key.secret_params().is_encrypted() && key.secret_subkeys.iter().all(|s| s.key.secret_params().is_encrypted());
    let any_rsa = SigningKind::of(key.primary_key.algorithm()) == SigningKind::Rsa
        || key.secret_subkeys.iter().any(|s| SigningKind::of(s.key.algorithm()) == SigningKind::Rsa);
    // (An RSA key is refused whatever its passphrase: its secret parts are never decrypted here.)
    let protected = encrypted && (any_rsa || !opens_without_passphrase(key));
    let mut secret: Vec<Fingerprint> = vec![Fingerprint::of(&key.primary_key.fingerprint())];
    secret.extend(key.secret_subkeys.iter().map(|s| Fingerprint::of(&s.key.fingerprint())));
    let signer = public.signing.iter().find(|s| s.may_sign && s.revoked.is_none() && s.weak.is_none() && secret.contains(&s.fingerprint));
    let can_sign = signer.is_some();
    let signing_kind = signer.map(|s| SigningKind::of(s.algorithm));
    Some(SecretKeyInfo {
        fingerprint: public.fingerprint().clone(),
        addresses: public.addresses().to_vec(),
        public,
        protected,
        any_rsa,
        can_sign,
        signing_kind,
    })
}

/// Whether a secret part of `key` opens with an empty passphrase: its derivation and symmetric
/// decryption run (within [`MAX_ARGON2_MEMORY`] and [`MAX_ARGON2_PASSES`]; a part asking for more
/// isn't tried) and its checksum or authentication tag checks. Only for a key with no RSA part.
fn opens_without_passphrase(key: &SignedSecretKey) -> bool {
    let empty = Password::empty();
    let tried = |params: &SecretParams| matches!(params, SecretParams::Encrypted(e) if bounded(e.string_to_key_params()));
    let primary = tried(key.primary_key.secret_params()) && matches!(key.primary_key.unlock(&empty, |_, _| Ok(())), Ok(Ok(())));
    primary || key.secret_subkeys.iter().any(|s| tried(s.key.secret_params()) && matches!(s.key.unlock(&empty, |_, _| Ok(())), Ok(Ok(()))))
}

/// Whether a passphrase's derivation is cheap enough to try.
fn bounded(params: &S2kParams) -> bool {
    let s2k = match params {
        S2kParams::Unprotected => return false,
        S2kParams::LegacyCfb { .. } => return true,
        S2kParams::Aead { s2k, .. } | S2kParams::Cfb { s2k, .. } | S2kParams::MalleableCfb { s2k, .. } => s2k,
    };
    match s2k {
        StringToKey::Argon2 { t, m_enc, .. } => *m_enc <= MAX_ARGON2_MEMORY && *t <= MAX_ARGON2_PASSES,
        _ => true,
    }
}

/// Whether `bytes` hold OpenPGP secret key material: an armoured `PRIVATE KEY BLOCK`, or binary
/// packets among which is a secret key or subkey. So that a secret key is never taken for a
/// Public key to import.
pub fn holds_secret(bytes: &[u8]) -> bool {
    if bytes.windows(25).any(|w| w == b"BEGIN PGP PRIVATE KEY BLO") {
        return true;
    }
    let secret = |packets: &[u8]| packet_tags(packets, MAX_PACKETS).iter().any(|t| matches!(*t, SECRET_KEY | SECRET_SUBKEY));
    secret(bytes) || armour::blocks(bytes, Kind::PublicKey, crate::keys::MAX_KEYS_PER_BLOCK, MAX_KEY_BYTES).iter().any(|b| secret(b))
}
