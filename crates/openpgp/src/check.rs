//! Checking one Digital signature over bytes, and what came of it.
//!
//! Only what a signature's hashed area says is believed: its issuer there may be blamed for a
//! signature that doesn't match, and its unhashed area (which anybody can change) is a hint to
//! find a key that then has to fit, never a reason for a result. What was signed is hashed once
//! for each hash, kind and salt among the signatures, whatever the number of keys tried.

use std::cell::Cell;

use pgp::composed::{Deserializable, DetachedSignature};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::line_writer::LineBreak;
use pgp::normalize_lines::NormalizedReader;
use pgp::packet::{Signature, SignatureConfig, SignatureType, SignatureVersion, SignatureVersionSpecific, Subpacket, SubpacketData};
use pgp::types::{KeyVersion, SignatureBytes, VerifyingKey};
use sha2::digest::{Digest, DynDigest};

use crate::keys::{self, Fingerprint, KeyId, KeyRing, PublicKey, Signing};

/// How many signatures over one text are looked at.
pub(crate) const MAX_SIGNATURES: usize = 8;
/// How many bytes a block of signatures may hold, decoded.
pub(crate) const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
/// How many keys one signature is tried with.
pub(crate) const MAX_CANDIDATES: usize = 8;
/// How many signature checks (public-key operations) one message may cost.
pub(crate) const MAX_VERIFICATIONS: usize = 24;

/// Who made a Digital signature and when, for the details.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Signer {
    /// The primary key's fingerprint: what the owner compares and marks Verified.
    pub fingerprint: Fingerprint,
    /// The fingerprint of the key that signed, when it is a subkey.
    pub subkey: Option<Fingerprint>,
    /// The key id of the key that signed.
    pub key_id: KeyId,
    /// The key's addresses (from its own User IDs), lower case.
    pub addresses: Vec<String>,
    /// When the signature says it was made, in seconds since 1970.
    pub signed: u64,
    /// When the primary key was made, in seconds since 1970.
    pub key_created: u64,
    /// The signing key's algorithm ("RSA", "Ed25519", …).
    pub algorithm: &'static str,
    /// The signature's hash ("SHA-256", …).
    pub hash: &'static str,
}

/// Who a signature that couldn't be checked names as its signer in its hashed area, if it does.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Issuer {
    pub fingerprint: Option<Fingerprint>,
    pub key_id: Option<KeyId>,
}

/// Why a valid Digital signature isn't Verified: one or more of these.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct NotVerified {
    /// The owner hasn't Verified this key for the From address (marked it after comparing the
    /// fingerprint, or their GnuPG keyring trusts it fully).
    pub key_unverified: bool,
    /// The From address isn't one of the key's own (when the message was signed).
    pub address_differs: bool,
    /// The signature covers only part of the message ([`Coverage`]).
    pub partly_signed: bool,
}

/// Why a Digital signature can't be checked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CantCheck {
    /// No Public key here for the key it names, nor any for the From address.
    NoPublicKey,
    /// None of the Public keys here for the From address made it, and the key it names (if
    /// any) isn't here.
    NoMatchingKey,
    /// Made with an algorithm that is no longer safe: a hash (MD5, SHA-1, RIPEMD-160) or a key
    /// (DSA, RSA under 2048 bits). Its name.
    WeakAlgorithm(&'static str),
    /// An algorithm or signature version this program doesn't know (or one before 4).
    UnknownAlgorithm,
    /// The signing key (or its primary key) was revoked by its owner, for this signature.
    KeyRevoked,
    /// The signing key (or its primary key) had expired when the signature was made.
    KeyExpired,
    /// The key that matches isn't one that may sign (or its subkey didn't sign back that it
    /// belongs to the key).
    NotASigningKey,
    /// The signature itself has expired.
    SignatureExpired,
    /// The message is larger than any looked at: it is signed, but isn't checked.
    TooLarge,
    /// The signature, or what it is over, can't be read, or is malformed.
    Unreadable,
}

/// What came of checking a message's OpenPGP Digital signature: the four results.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Outcome {
    /// Valid, the key Verified for the From address, which is the key's, over the whole message.
    Verified(Signer),
    /// Valid, but not Verified: why.
    Valid { signer: Signer, why: NotVerified },
    /// The message was changed after it was signed: the key its hashed area names is here and
    /// doesn't match it.
    Changed(Signer),
    /// It can't be checked, and why; who its hashed area names as its signer.
    CantCheck { why: CantCheck, issuer: Issuer },
}

impl Outcome {
    /// Which outcome to show of several signatures over one text: the best.
    fn rank(&self) -> u8 {
        match self {
            Outcome::Verified(_) => 3,
            Outcome::Valid { .. } => 2,
            Outcome::Changed(_) => 1,
            Outcome::CantCheck { .. } => 0,
        }
    }

    pub(crate) fn cant(why: CantCheck) -> Outcome {
        Outcome::CantCheck { why, issuer: Issuer::default() }
    }
}

/// What of a message a Digital signature covers.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Coverage {
    /// Whether it is the whole message: a PGP/MIME message at the top, or a plain text body that
    /// is one cleartext-signed block with nothing but white space around it. Only then can a
    /// message be Verified.
    pub whole: bool,
    /// The MIME part signed, as the place of each part on the way down from the message (empty:
    /// the message itself; `[0]`: its first part).
    pub path: Vec<usize>,
    /// For a cleartext-signed text: where the block is in that part's text (decoded from its
    /// transfer encoding), from its first line to the end of its last.
    pub span: Option<(usize, usize)>,
}

/// A message's Digital signature checked: what came of it, and what it covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Checked {
    pub outcome: Outcome,
    pub coverage: Coverage,
}

/// What the caller hands in to check against, and what checking has cost so far.
pub(crate) struct Context<'a> {
    /// The From address, lower case; empty when the From field doesn't hold exactly one.
    pub(crate) from: String,
    pub(crate) keys: &'a KeyRing,
    pub(crate) now: u64,
    pub(crate) verified: &'a dyn Fn(&Fingerprint, &str) -> bool,
    /// Public-key operations made.
    pub(crate) verifications: Cell<usize>,
    /// Times what was signed was hashed.
    pub(crate) data_passes: Cell<usize>,
}

/// A hash over what was signed, before a signature's own fields: cloned for each signature.
#[derive(Clone)]
enum Base {
    Sha224(sha2::Sha224),
    Sha256(sha2::Sha256),
    Sha384(sha2::Sha384),
    Sha512(sha2::Sha512),
    Sha3_256(sha3::Sha3_256),
    Sha3_512(sha3::Sha3_512),
}

impl Base {
    fn new(hash: HashAlgorithm) -> Option<Base> {
        Some(match hash {
            HashAlgorithm::Sha224 => Base::Sha224(Default::default()),
            HashAlgorithm::Sha256 => Base::Sha256(Default::default()),
            HashAlgorithm::Sha384 => Base::Sha384(Default::default()),
            HashAlgorithm::Sha512 => Base::Sha512(Default::default()),
            HashAlgorithm::Sha3_256 => Base::Sha3_256(Default::default()),
            HashAlgorithm::Sha3_512 => Base::Sha3_512(Default::default()),
            _ => return None,
        })
    }

    fn update(&mut self, bytes: &[u8]) {
        match self {
            Base::Sha224(h) => Digest::update(h, bytes),
            Base::Sha256(h) => Digest::update(h, bytes),
            Base::Sha384(h) => Digest::update(h, bytes),
            Base::Sha512(h) => Digest::update(h, bytes),
            Base::Sha3_256(h) => Digest::update(h, bytes),
            Base::Sha3_512(h) => Digest::update(h, bytes),
        }
    }

    fn boxed(&self) -> Box<dyn DynDigest + Send> {
        match self.clone() {
            Base::Sha224(h) => Box::new(h),
            Base::Sha256(h) => Box::new(h),
            Base::Sha384(h) => Box::new(h),
            Base::Sha512(h) => Box::new(h),
            Base::Sha3_256(h) => Box::new(h),
            Base::Sha3_512(h) => Box::new(h),
        }
    }
}

/// The hashes over what was signed, one for each hash, kind (binary or text) and salt.
type Bases = Vec<((HashAlgorithm, bool, Vec<u8>), Base)>;

/// Checks the signature packets `packets` (binary) over `data`; the best outcome of them.
pub(crate) fn check(packets: &[u8], data: &[u8], cx: &Context<'_>) -> Outcome {
    if packets.len() > MAX_SIGNATURE_BYTES {
        return Outcome::cant(CantCheck::Unreadable);
    }
    let parsed = crate::guard(|| DetachedSignature::from_bytes_many(packets).map(|it| it.take(MAX_SIGNATURES).collect::<Vec<_>>()));
    let Some(Ok(signatures)) = parsed else { return Outcome::cant(CantCheck::Unreadable) };
    let mut bases: Bases = Vec::new();
    let mut best: Option<Outcome> = None;
    for signature in signatures.into_iter().filter_map(Result::ok) {
        let outcome = crate::guard(|| one(&signature.signature, data, cx, &mut bases)).unwrap_or_else(|| Outcome::cant(CantCheck::Unreadable));
        if best.as_ref().is_none_or(|b| outcome.rank() > b.rank()) {
            best = Some(outcome);
        }
    }
    best.unwrap_or_else(|| Outcome::cant(CantCheck::Unreadable))
}

/// The issuers a list of subpackets names.
fn issuers(subpackets: &[Subpacket]) -> (Vec<Fingerprint>, Vec<KeyId>) {
    let mut fps = Vec::new();
    let mut ids = Vec::new();
    for p in subpackets.iter().take(64) {
        match &p.data {
            SubpacketData::IssuerFingerprint(fp) => fps.push(Fingerprint::of(fp)),
            SubpacketData::IssuerKeyId(id) => ids.push(KeyId::of(id)),
            _ => {}
        }
    }
    (fps, ids)
}

/// The digest a signature signs: what was signed (hashed once per hash, kind and salt), then
/// its own fields and trailer (RFC 9580 §5.2.4). `None` when it can't be made.
fn digest(config: &SignatureConfig, data: &[u8], cx: &Context<'_>, bases: &mut Bases) -> Option<Vec<u8>> {
    let text = config.typ == SignatureType::Text;
    let salt = match &config.version_specific {
        SignatureVersionSpecific::V6 { salt } => {
            if config.hash_alg.salt_len() != Some(salt.len()) {
                return None;
            }
            salt.clone()
        }
        _ => Vec::new(),
    };
    let key = (config.hash_alg, text, salt);
    let base = match bases.iter().find(|(k, _)| *k == key) {
        Some((_, base)) => base.clone(),
        None => {
            let mut base = Base::new(config.hash_alg)?;
            base.update(&key.2);
            if text {
                // RFC 9580 §5.2.1.2: a text signature is over the text with CR LF line ends.
                let mut reader = NormalizedReader::new(data, LineBreak::Crlf);
                let mut chunk = [0u8; 8192];
                loop {
                    let n = std::io::Read::read(&mut reader, &mut chunk).ok()?;
                    if n == 0 {
                        break;
                    }
                    base.update(chunk.get(..n)?);
                }
            } else {
                base.update(data);
            }
            cx.data_passes.set(cx.data_passes.get().saturating_add(1));
            bases.push((key, base.clone()));
            base
        }
    };
    let mut hasher = base.boxed();
    let len = config.hash_signature_data(&mut hasher).ok()?;
    hasher.update(&config.trailer(len).ok()?);
    Some(hasher.finalize().to_vec())
}

fn one(sig: &Signature, data: &[u8], cx: &Context<'_>, bases: &mut Bases) -> Outcome {
    let Some(config) = sig.config() else { return Outcome::cant(CantCheck::UnknownAlgorithm) };
    let (fps, ids) = issuers(&config.hashed_subpackets);
    let (hint_fps, hint_ids) = issuers(&config.unhashed_subpackets);
    let issuer = Issuer { fingerprint: fps.first().cloned(), key_id: ids.first().copied() };
    let cant = |why: CantCheck| Outcome::CantCheck { why, issuer: issuer.clone() };
    // Version 3 signatures (and earlier) hash too little of themselves to be trusted.
    if !matches!(config.version(), SignatureVersion::V4 | SignatureVersion::V6) {
        return cant(CantCheck::UnknownAlgorithm);
    }
    if !matches!(config.typ, SignatureType::Binary | SignatureType::Text) {
        return cant(CantCheck::Unreadable);
    }
    if keys::is_weak(config.hash_alg) {
        return cant(CantCheck::WeakAlgorithm(keys::hash_name(config.hash_alg)));
    }
    if config.pub_alg == PublicKeyAlgorithm::DSA {
        return cant(CantCheck::WeakAlgorithm("DSA"));
    }
    if !known_signature_algorithm(config.pub_alg) {
        return cant(CantCheck::UnknownAlgorithm);
    }
    // RFC 9580 §5.2.3.11: a signature says when it was made, in its hashed area.
    let Some(signed) = config.created().map(|t| u64::from(t.as_secs())) else { return cant(CantCheck::Unreadable) };
    // A signature whose value isn't of the algorithm's shape is malformed, never "changed".
    let Some(sig_bytes) = sig.signature().filter(|b| well_shaped(config.pub_alg, b)) else { return cant(CantCheck::Unreadable) };
    let Some(digest) = digest(config, data, cx, bases) else { return cant(CantCheck::UnknownAlgorithm) };
    // The first two bytes of the digest are in the signature: a quick test that it is over this.
    let fits = sig.signed_hash_value().is_some_and(|v| digest.get(..2) == Some(&v[..]));

    // The keys to try, at most MAX_CANDIDATES: those the hashed area names; then those the
    // unhashed area names (a hint: only a key that fits counts), before any other key of the
    // From address, so that keys claiming the address can't push the signer's out; then the
    // From address's keys.
    let mut candidates: Vec<(&PublicKey, &Signing, bool)> = Vec::new();
    for (k, s) in cx.keys.signers(&fps, &ids) {
        push(&mut candidates, k, s, true);
    }
    for (k, s) in cx.keys.signers(&hint_fps, &hint_ids) {
        push(&mut candidates, k, s, false);
    }
    let from_keys: Vec<&PublicKey> = cx.keys.for_address(&cx.from).collect();
    for k in &from_keys {
        for s in &k.signing {
            push(&mut candidates, k, s, false);
        }
    }

    let mut blamed: Option<Signer> = None;
    let mut weak: Option<&'static str> = None;
    let mut starved = false;
    for (key, signing, named) in candidates {
        // A version 6 signature is made by a version 6 key and only by one (RFC 9580 §5.2.3).
        if (signing.version == KeyVersion::V6) != (config.version() == SignatureVersion::V6) || signing.algorithm != config.pub_alg {
            continue;
        }
        let signer = Signer {
            fingerprint: key.fingerprint().clone(),
            subkey: signing.subkey.map(|_| signing.fingerprint.clone()),
            key_id: signing.key_id,
            addresses: key.addresses().to_vec(),
            signed,
            key_created: key.created(),
            algorithm: keys::algorithm_name(signing.algorithm),
            hash: keys::hash_name(config.hash_alg),
        };
        if let Some(why) = signing.weak {
            if named {
                weak = Some(why);
            }
            continue;
        }
        if !fits {
            if named {
                blamed.get_or_insert(signer);
            }
            continue;
        }
        if cx.verifications.get() >= MAX_VERIFICATIONS {
            // Keys left untried: what came of it isn't known.
            starved = true;
            break;
        }
        cx.verifications.set(cx.verifications.get().saturating_add(1));
        if verify_with(config.hash_alg, &digest, sig_bytes, key, signing) {
            return judged(sig, key, signing, signer, cx).unwrap_or_else(&cant);
        }
        if named {
            blamed.get_or_insert(signer);
        }
    }
    if let Some(why) = weak {
        return cant(CantCheck::WeakAlgorithm(why));
    }
    match blamed {
        Some(signer) => Outcome::Changed(signer),
        None if starved => cant(CantCheck::Unreadable),
        None if !from_keys.is_empty() => cant(CantCheck::NoMatchingKey),
        None => cant(CantCheck::NoPublicKey),
    }
}

/// Adds a key to try, once, while there is room.
fn push<'a>(list: &mut Vec<(&'a PublicKey, &'a Signing, bool)>, key: &'a PublicKey, signing: &'a Signing, named: bool) {
    if list.len() < MAX_CANDIDATES && !list.iter().any(|(_, s, _)| s.fingerprint == signing.fingerprint) {
        list.push((key, signing, named));
    }
}

/// A signature that checks: whether its key (and the primary key over it) could make it then,
/// and whether it is Verified.
fn judged(sig: &Signature, key: &PublicKey, signing: &Signing, signer: Signer, cx: &Context<'_>) -> Result<Outcome, CantCheck> {
    let when = signer.signed;
    let primary = key.primary();
    if signing.revoked.is_some_and(|r| r.covers(Some(when))) || primary.and_then(|p| p.revoked).is_some_and(|r| r.covers(Some(when))) {
        return Err(CantCheck::KeyRevoked);
    }
    if !signing.may_sign {
        return Err(CantCheck::NotASigningKey);
    }
    // A signature that says it was made before its key was is not one the key made.
    if when < u64::from(signing.created) {
        return Err(CantCheck::Unreadable);
    }
    if signing.expired_at(when) || primary.is_some_and(|p| p.expired_at(when)) {
        return Err(CantCheck::KeyExpired);
    }
    if let Some(after) = sig.signature_expiration_time().map(|d| u64::from(d.as_secs()))
        && after > 0
        && cx.now >= when.saturating_add(after)
    {
        return Err(CantCheck::SignatureExpired);
    }
    let address_on_key = !cx.from.is_empty() && key.has_address_at(&cx.from, when);
    let why = NotVerified {
        key_unverified: cx.from.is_empty() || !(cx.verified)(&signer.fingerprint, &cx.from),
        address_differs: !address_on_key,
        partly_signed: false,
    };
    Ok(if why.key_unverified || why.address_differs { Outcome::Valid { signer, why } } else { Outcome::Verified(signer) })
}

fn verify_with(hash: HashAlgorithm, digest: &[u8], sig: &SignatureBytes, key: &PublicKey, signing: &Signing) -> bool {
    let inner = key.inner();
    match signing.subkey {
        None => inner.primary_key.verify(hash, digest, sig).is_ok(),
        Some(i) => inner.public_subkeys.get(i).is_some_and(|sub| sub.key.verify(hash, digest, sig).is_ok()),
    }
}

/// Whether a signature value has the shape its algorithm gives it (RFC 9580 §5.2.3): one MPI
/// for RSA, two for ECDSA and legacy EdDSA, 64 or 114 bytes for Ed25519 and Ed448.
fn well_shaped(algorithm: PublicKeyAlgorithm, bytes: &SignatureBytes) -> bool {
    match (algorithm, bytes) {
        (PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign, SignatureBytes::Mpis(m)) => m.len() == 1,
        (PublicKeyAlgorithm::ECDSA | PublicKeyAlgorithm::EdDSALegacy, SignatureBytes::Mpis(m)) => m.len() == 2,
        (PublicKeyAlgorithm::Ed25519, SignatureBytes::Native(b)) => b.len() == 64,
        (PublicKeyAlgorithm::Ed448, SignatureBytes::Native(b)) => b.len() == 114,
        _ => false,
    }
}

/// Algorithms a Digital signature can be made with that are checked (DSA is refused before).
fn known_signature_algorithm(algorithm: PublicKeyAlgorithm) -> bool {
    matches!(
        algorithm,
        PublicKeyAlgorithm::RSA
            | PublicKeyAlgorithm::RSASign
            | PublicKeyAlgorithm::ECDSA
            | PublicKeyAlgorithm::EdDSALegacy
            | PublicKeyAlgorithm::Ed25519
            | PublicKeyAlgorithm::Ed448
    )
}
