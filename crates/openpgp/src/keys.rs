//! Public keys: read from their bytes (armoured or not), their self-signatures checked once
//! when they are read, and kept in a [`KeyRing`] that finds them by fingerprint, key id or
//! address. A key's addresses are those of the User IDs it signed itself (RFC 9580 §5.2.1,
//! §10.1); nothing here is a web of trust: whether a key is Verified is the caller's to say.

use pgp::composed::{Deserializable, SignedKeyDetails, SignedPublicKey, SignedPublicSubKey};
use pgp::crypto::hash::HashAlgorithm;
use pgp::crypto::public_key::PublicKeyAlgorithm;
use pgp::packet::{KeyFlags, RevocationCode, Signature, SignatureType, SignatureVersion, SubpacketData};
use pgp::ser::Serialize;
use pgp::types::{KeyDetails, KeyVersion, PublicParams, SignedUser, Tag};

use crate::armour::{self, Kind};
use crate::{Error, mime};

/// How many bytes a block of Public keys may hold, decoded.
pub(crate) const MAX_KEY_BYTES: usize = 1 << 20;
/// How many keys one block (a file, an attachment) is read for.
pub(crate) const MAX_KEYS_PER_BLOCK: usize = 16;
/// How many User IDs and subkeys one key may have: more is not a key anybody sends.
const MAX_USERS: usize = 64;
const MAX_SUBKEYS: usize = 32;
/// How long a User ID may be: a name and an address are far shorter. (User attributes, such as
/// pictures, are never checked or kept, so cost nothing.)
const MAX_USER_ID: usize = 4 * 1024;
/// How many signature checks reading one key may cost (third-party certifications are never
/// checked, so a real key needs a handful). Room for two copies of the largest key kept, merged:
/// two self-signatures for each User ID, and two with a back-signature each for each subkey.
const MAX_CHECKS: usize = 512;
/// How many packets one block is read for before it is given up on.
const MAX_PACKETS: usize = 4_096;

/// A key's fingerprint (RFC 9580 §5.5.4): 20 bytes for a version 4 key, 32 for version 6.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct Fingerprint(Vec<u8>);

impl Fingerprint {
    /// The fingerprint written as hexadecimal digits (spaces and a `0x` in front allowed).
    pub fn from_hex(text: &str) -> Option<Fingerprint> {
        let text = text.trim();
        let text = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X")).unwrap_or(text);
        let digits: Vec<u32> = text.chars().filter(|c| !c.is_whitespace()).map(|c| c.to_digit(16)).collect::<Option<_>>()?;
        if !matches!(digits.len(), 40 | 64) {
            return None;
        }
        Some(Fingerprint(digits.chunks(2).map(|p| p.iter().fold(0u8, |acc, d| (acc << 4) | (*d as u8))).collect()))
    }

    /// The fingerprint's bytes.
    pub fn as_bytes(&self) -> &[u8] {
        &self.0
    }

    /// Upper-case hexadecimal, no spaces: what [`Fingerprint::from_hex`] reads back.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02X}")).collect()
    }

    /// Upper-case hexadecimal in groups of four, for an owner to compare by eye.
    pub fn grouped(&self) -> String {
        let hex = self.to_hex();
        let groups: Vec<&str> = (0..hex.len()).step_by(4).filter_map(|i| hex.get(i..i.saturating_add(4).min(hex.len()))).collect();
        groups.join(" ")
    }

    pub(crate) fn of(fp: &pgp::types::Fingerprint) -> Fingerprint {
        Fingerprint(fp.as_bytes().to_vec())
    }
}

impl std::fmt::Display for Fingerprint {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A key id (RFC 9580 §5.5.4): eight bytes that a signature names its signer by.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeyId(pub [u8; 8]);

impl KeyId {
    /// Upper-case hexadecimal, sixteen digits.
    pub fn to_hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02X}")).collect()
    }

    pub(crate) fn of(id: &pgp::types::KeyId) -> KeyId {
        let mut bytes = [0u8; 8];
        for (to, from) in bytes.iter_mut().zip(id.as_ref()) {
            *to = *from;
        }
        KeyId(bytes)
    }
}

impl std::fmt::Display for KeyId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.to_hex())
    }
}

/// A public-key algorithm's name, as an owner may see it in the details.
pub(crate) fn algorithm_name(algorithm: PublicKeyAlgorithm) -> &'static str {
    match algorithm {
        PublicKeyAlgorithm::RSA | PublicKeyAlgorithm::RSASign | PublicKeyAlgorithm::RSAEncrypt => "RSA",
        PublicKeyAlgorithm::DSA => "DSA",
        PublicKeyAlgorithm::ECDSA => "ECDSA",
        PublicKeyAlgorithm::EdDSALegacy => "EdDSA",
        PublicKeyAlgorithm::Ed25519 => "Ed25519",
        PublicKeyAlgorithm::Ed448 => "Ed448",
        PublicKeyAlgorithm::ECDH => "ECDH",
        PublicKeyAlgorithm::X25519 => "X25519",
        PublicKeyAlgorithm::X448 => "X448",
        PublicKeyAlgorithm::Elgamal | PublicKeyAlgorithm::ElgamalEncrypt => "Elgamal",
        _ => "unknown",
    }
}

/// A hash algorithm's name, as an owner may see it in the details.
pub(crate) fn hash_name(hash: HashAlgorithm) -> &'static str {
    match hash {
        HashAlgorithm::Md5 => "MD5",
        HashAlgorithm::Sha1 => "SHA-1",
        HashAlgorithm::Ripemd160 => "RIPEMD-160",
        HashAlgorithm::Sha224 => "SHA-224",
        HashAlgorithm::Sha256 => "SHA-256",
        HashAlgorithm::Sha384 => "SHA-384",
        HashAlgorithm::Sha512 => "SHA-512",
        HashAlgorithm::Sha3_256 => "SHA3-256",
        HashAlgorithm::Sha3_512 => "SHA3-512",
        _ => "unknown",
    }
}

/// Hashes no longer safe for a signature: MD5, SHA-1 and RIPEMD-160 (RFC 9580 §9.5).
pub(crate) fn is_weak(hash: HashAlgorithm) -> bool {
    matches!(hash, HashAlgorithm::Md5 | HashAlgorithm::Sha1 | HashAlgorithm::Ripemd160)
}

/// Why a key may not be trusted to make a Digital signature at all, whatever its signature
/// says: DSA, and RSA under 2048 bits (RFC 9580 §12.4, §12.5).
pub(crate) fn weak_key(key: &impl KeyDetails) -> Option<&'static str> {
    match key.public_params() {
        PublicParams::DSA(_) => Some("DSA"),
        PublicParams::RSA(params) => {
            // An MPI starts with its length in bits (RFC 9580 §3.2): n comes first.
            let bytes = params.to_bytes().unwrap_or_default();
            let bits = u16::from_be_bytes([bytes.first().copied().unwrap_or(0), bytes.get(1).copied().unwrap_or(0)]);
            (bits < 2048).then_some("RSA under 2048 bits")
        }
        _ => None,
    }
}

/// A revocation of a key or subkey (RFC 9580 §5.2.3.31): a hard one (compromised, or no reason
/// a reader knows) undoes every signature the key made; a soft one (superseded, retired) only
/// those made at or after it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Revoked {
    pub(crate) hard: bool,
    pub(crate) at: u64,
}

impl Revoked {
    fn of(sig: &Signature) -> Revoked {
        let soft = matches!(sig.revocation_reason_code(), Some(RevocationCode::KeySuperseded | RevocationCode::KeyRetired));
        Revoked { hard: !soft, at: u64::from(created(sig)) }
    }

    /// Whether it undoes a signature made at `when` (`None`: unknown, so yes).
    pub(crate) fn covers(&self, when: Option<u64>) -> bool {
        self.hard || when.is_none_or(|t| t >= self.at)
    }
}

/// One key of a [`PublicKey`] that may make Digital signatures: the primary key or a subkey.
#[derive(Clone, Debug)]
pub(crate) struct Signing {
    /// `None` for the primary key, else the subkey's place in the key.
    pub(crate) subkey: Option<usize>,
    pub(crate) fingerprint: Fingerprint,
    pub(crate) key_id: KeyId,
    pub(crate) version: KeyVersion,
    pub(crate) algorithm: PublicKeyAlgorithm,
    pub(crate) created: u32,
    /// When it stops being valid, in seconds since 1970 (its own expiry, or that of the
    /// self-signature that binds it); `None` when never.
    pub(crate) expires: Option<u64>,
    /// Its own revocation; a subkey is also undone by the primary key's.
    pub(crate) revoked: Option<Revoked>,
    /// Whether its self-signature lets it sign (subkeys say so; a primary key that says nothing
    /// may, as RFC 9580 §5.2.3.29 leaves it).
    pub(crate) may_sign: bool,
    /// Why it isn't strong enough to be trusted, if it isn't.
    pub(crate) weak: Option<&'static str>,
}

impl Signing {
    /// Whether it was valid at `when`: not before it was made, not after it expired.
    pub(crate) fn expired_at(&self, when: u64) -> bool {
        self.expires.is_some_and(|end| when >= end)
    }
}

/// A Public key (an OpenPGP certificate, RFC 9580 §10.1) whose self-signatures check.
#[derive(Clone, Debug)]
pub struct PublicKey {
    key: SignedPublicKey,
    fingerprint: Fingerprint,
    addresses: Vec<String>,
    /// For each address, when its User ID's self-signature stops being valid.
    address_until: Vec<Option<u64>>,
    /// The primary key first, then each subkey that may sign.
    pub(crate) signing: Vec<Signing>,
}

/// How many signature checks one call that reads keys may cost in all ([`PublicKey::parse`],
/// [`crate::keys_in_message`]).
pub(crate) const MAX_CHECKS_PER_CALL: usize = 1_024;
/// How many revocations of one key are checked: never cut short; a key with more is refused.
/// Room for two of each part of two merged copies of the largest key kept.
const MAX_REVOCATION_CHECKS: usize = 256;

/// How many signature checks some reading may cost.
pub(crate) struct Budget {
    left: usize,
    /// A check was wanted and refused.
    pub(crate) starved: bool,
}

impl Budget {
    pub(crate) fn new(left: usize) -> Budget {
        Budget { left, starved: false }
    }

    /// Whether one more check may be made, counting it.
    fn spend(&mut self) -> bool {
        if self.left == 0 {
            self.starved = true;
            return false;
        }
        self.left -= 1;
        true
    }
}

impl PublicKey {
    /// Every Public key in `bytes`: armoured (`-----BEGIN PGP PUBLIC KEY BLOCK-----`, as many
    /// blocks as there are) or binary, at most 16 in all. A key whose self-signatures don't
    /// check, of a version before 4, or larger than a key anybody sends, is left out.
    /// [`Error::Unreadable`] when no key at all could be read.
    pub fn parse(bytes: &[u8]) -> Result<Vec<PublicKey>, Error> {
        Self::parse_with(bytes, &mut Budget::new(MAX_CHECKS_PER_CALL))
    }

    pub(crate) fn parse_with(bytes: &[u8], total: &mut Budget) -> Result<Vec<PublicKey>, Error> {
        let mut out = Vec::new();
        let blocks = armour::blocks(bytes, Kind::PublicKey, MAX_KEYS_PER_BLOCK, MAX_KEY_BYTES);
        if blocks.is_empty() {
            if bytes.len() > MAX_KEY_BYTES {
                return Err(Error::TooLarge);
            }
            out.extend(Self::binary(bytes, MAX_KEYS_PER_BLOCK, total));
        } else {
            for block in blocks {
                let room = MAX_KEYS_PER_BLOCK.saturating_sub(out.len());
                if room == 0 {
                    break;
                }
                out.extend(Self::binary(&block, room, total));
            }
        }
        if out.is_empty() { Err(Error::Unreadable) } else { Ok(out) }
    }

    /// At most `room` keys in binary packets.
    pub(crate) fn binary(bytes: &[u8], room: usize, total: &mut Budget) -> Vec<PublicKey> {
        if room == 0 || count_packets(bytes) > MAX_PACKETS {
            return Vec::new();
        }
        let Some(Ok(keys)) = crate::guard(|| SignedPublicKey::from_bytes_many(bytes).map(|it| it.take(room).collect::<Vec<_>>())) else {
            return Vec::new();
        };
        keys.into_iter().filter_map(Result::ok).filter_map(|key| crate::guard(|| PublicKey::checked(key, total)).flatten()).collect()
    }

    /// The key, once its self-signatures check: its addresses from the User IDs it certified,
    /// its revocations, and the keys of it that may sign. `None` when it is no key to trust:
    /// nothing certifies it, it is too large, its version is before 4, or a budget ran out
    /// before all of it was checked. What is kept of it is what checked and is used: for the
    /// key, each User ID and each subkey, its newest self-signature and its strongest
    /// revocation; nothing that anybody but the key's owner could have added.
    pub(crate) fn checked(key: SignedPublicKey, total: &mut Budget) -> Option<PublicKey> {
        if !matches!(key.primary_key.version(), KeyVersion::V4 | KeyVersion::V6)
            || key.details.users.len() > MAX_USERS
            || key.public_subkeys.len() > MAX_SUBKEYS
        {
            return None;
        }
        let primary = &key.primary_key;
        let mut local = Budget::new(MAX_CHECKS);
        let mut revocations = Budget::new(MAX_REVOCATION_CHECKS);
        let spend = |budget: &mut Budget, total: &mut Budget| budget.spend() && total.spend();

        // Revocations first, on a budget of their own, so that none is ever left unchecked.
        let mut key_revocation: Option<&Signature> = None;
        for sig in key.details.revocation_signatures.iter().chain(&key.details.direct_signatures) {
            if sig.typ() == Some(SignatureType::KeyRevocation)
                && usable(sig)
                && is_self(sig, primary)
                && spend(&mut revocations, total)
                && sig.verify_key(primary).is_ok()
            {
                key_revocation = stronger(key_revocation, sig);
            }
        }

        // The newest valid self-signature on the key or one of its User IDs: its key flags and
        // expiry are the primary key's (RFC 9580 §5.2.3.10, §10.1.1).
        let mut newest: Option<&Signature> = None;
        let mut direct: Option<&Signature> = None;
        for sig in &key.details.direct_signatures {
            if sig.typ() == Some(SignatureType::Key)
                && usable(sig)
                && is_self(sig, primary)
                && spend(&mut local, total)
                && sig.verify_key(primary).is_ok()
            {
                consider(sig, &mut direct);
            }
        }
        if let Some(sig) = direct {
            consider(sig, &mut newest);
        }
        let (mut addresses, mut address_until, mut users) = (Vec::new(), Vec::new(), Vec::new());
        for user in &key.details.users {
            // Every check of a User ID hashes all of it: a longer one than anybody's is left
            // out with its signatures, unhashed.
            if user.id.id().len() > MAX_USER_ID {
                continue;
            }
            let mut certified: Option<&Signature> = None;
            let mut user_revocation: Option<&Signature> = None;
            for sig in &user.signatures {
                let Some(typ) = sig.typ() else { continue };
                if !usable(sig) || !is_self(sig, primary) {
                    continue;
                }
                if typ == SignatureType::CertRevocation {
                    if spend(&mut revocations, total) && sig.verify_certification(primary, Tag::UserId, &user.id).is_ok() {
                        consider(sig, &mut user_revocation);
                    }
                    continue;
                }
                let certification =
                    matches!(typ, SignatureType::CertGeneric | SignatureType::CertPersona | SignatureType::CertCasual | SignatureType::CertPositive);
                if certification && spend(&mut local, total) && sig.verify_certification(primary, Tag::UserId, &user.id).is_ok() {
                    consider(sig, &mut certified);
                }
            }
            if certified.is_some() || user_revocation.is_some() {
                users.push(SignedUser::new(user.id.clone(), certified.into_iter().chain(user_revocation).cloned().collect()));
            }
            let Some(cert) = certified else { continue };
            consider(cert, &mut newest);
            // RFC 9580 §5.2.3.31: the newest self-signature on a User ID is what holds, so a
            // certification after its revocation takes it back (one at the same second doesn't).
            let revoked = user_revocation.is_some_and(|r| created(r) >= created(cert));
            let address = mime::address(&String::from_utf8_lossy(user.id.id()));
            if !revoked && !address.is_empty() && !addresses.contains(&address) {
                addresses.push(address);
                address_until.push(signature_until(cert));
            }
        }
        let newest = newest?;
        let primary_created = primary.created_at().as_secs();
        let mut signing = vec![Signing {
            subkey: None,
            fingerprint: Fingerprint::of(&primary.fingerprint()),
            key_id: KeyId::of(&primary.legacy_key_id()),
            version: primary.version(),
            algorithm: primary.algorithm(),
            created: primary_created,
            expires: earlier(expiry(primary_created, newest), signature_until(newest)),
            revoked: key_revocation.map(Revoked::of),
            may_sign: flags(newest).is_none_or(|f| f.sign()),
            weak: weak_key(primary),
        }];
        let mut subkeys = Vec::new();
        for sub in &key.public_subkeys {
            if !matches!(sub.key.version(), KeyVersion::V4 | KeyVersion::V6) {
                continue;
            }
            let mut binding: Option<&Signature> = None;
            let mut sub_revocation: Option<&Signature> = None;
            for sig in &sub.signatures {
                if !usable(sig) || !is_self(sig, primary) {
                    continue;
                }
                match sig.typ() {
                    Some(SignatureType::SubkeyRevocation) => {
                        if spend(&mut revocations, total) && sig.verify_subkey_binding(primary, &sub.key).is_ok() {
                            sub_revocation = stronger(sub_revocation, sig);
                        }
                    }
                    Some(SignatureType::SubkeyBinding) if spend(&mut local, total) && sig.verify_subkey_binding(primary, &sub.key).is_ok() => {
                        consider(sig, &mut binding);
                    }
                    _ => {}
                }
            }
            let Some(binding) = binding else { continue };
            let place = subkeys.len();
            subkeys.push(SignedPublicSubKey::new(sub.key.clone(), std::iter::once(binding).chain(sub_revocation).cloned().collect()));
            if !flags(binding).is_some_and(|f| f.sign()) {
                continue;
            }
            // A subkey that signs must sign back that it belongs to this key (§5.2.1): else
            // anybody's key could claim somebody else's subkey.
            let backed = binding
                .embedded_signature()
                .is_some_and(|back| spend(&mut local, total) && back.verify_primary_key_binding(&sub.key, primary).is_ok());
            let sub_created = sub.key.created_at().as_secs();
            signing.push(Signing {
                subkey: Some(place),
                fingerprint: Fingerprint::of(&sub.key.fingerprint()),
                key_id: KeyId::of(&sub.key.legacy_key_id()),
                version: sub.key.version(),
                algorithm: sub.key.algorithm(),
                created: sub_created,
                expires: earlier(expiry(sub_created, binding), signature_until(binding)),
                revoked: sub_revocation.map(Revoked::of),
                may_sign: backed,
                weak: weak_key(&sub.key),
            });
        }
        // A revocation that couldn't be checked, or a key not checked whole, is no key at all:
        // never a key with a part of it left out.
        if revocations.starved || local.starved || total.starved {
            return None;
        }
        let details = SignedKeyDetails::new(key_revocation.into_iter().cloned().collect(), direct.into_iter().cloned().collect(), users, Vec::new());
        let kept = SignedPublicKey::new(key.primary_key.clone(), details, subkeys);
        Some(PublicKey { fingerprint: Fingerprint::of(&key.fingerprint()), key: kept, addresses, address_until, signing })
    }

    /// The primary key's fingerprint: what the owner compares, and marks Verified.
    pub fn fingerprint(&self) -> &Fingerprint {
        &self.fingerprint
    }

    /// The primary key's key id.
    pub fn key_id(&self) -> KeyId {
        KeyId::of(&self.key.primary_key.legacy_key_id())
    }

    /// The addresses of the User IDs the key certified itself (lower case), in its order.
    pub fn addresses(&self) -> &[String] {
        &self.addresses
    }

    /// Whether `address` (an address, or a From field's value) is one of the key's own.
    pub fn has_address(&self, address: &str) -> bool {
        let address = mime::address(address);
        !address.is_empty() && self.addresses.contains(&address)
    }

    /// Whether `address` was one of the key's own at `when`: its self-signature hadn't expired.
    pub(crate) fn has_address_at(&self, address: &str, when: u64) -> bool {
        let address = mime::address(address);
        !address.is_empty() && self.addresses.iter().zip(&self.address_until).any(|(a, until)| *a == address && until.is_none_or(|end| when < end))
    }

    /// When the primary key was made, in seconds since 1970.
    pub fn created(&self) -> u64 {
        u64::from(self.key.primary_key.created_at().as_secs())
    }

    /// The primary key's algorithm, by name ("RSA", "Ed25519", …).
    pub fn algorithm(&self) -> &'static str {
        algorithm_name(self.key.primary_key.algorithm())
    }

    /// Whether the key's owner revoked it.
    pub fn revoked(&self) -> bool {
        self.primary().is_some_and(|s| s.revoked.is_some())
    }

    /// When the primary key stops being valid, in seconds since 1970; `None` when never.
    pub fn expires(&self) -> Option<u64> {
        self.primary().and_then(|s| s.expires)
    }

    pub(crate) fn primary(&self) -> Option<&Signing> {
        self.signing.first()
    }

    /// The key as binary OpenPGP packets, to be kept and read back with [`PublicKey::parse`].
    pub fn to_bytes(&self) -> Vec<u8> {
        self.key.to_bytes().unwrap_or_default()
    }

    pub(crate) fn inner(&self) -> &SignedPublicKey {
        &self.key
    }
}

/// The earlier of two optional times.
fn earlier(a: Option<u64>, b: Option<u64>) -> Option<u64> {
    match (a, b) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (a, b) => a.or(b),
    }
}

/// When a signature stops being valid by its own expiry (RFC 9580 §5.2.3.18).
fn signature_until(sig: &Signature) -> Option<u64> {
    let after = sig.signature_expiration_time()?.as_secs();
    (after > 0).then(|| u64::from(created(sig)).saturating_add(u64::from(after)))
}

/// Makes `sig` the `newest` when it is at least as new.
fn consider<'a>(sig: &'a Signature, newest: &mut Option<&'a Signature>) {
    if newest.is_none_or(|n| created(sig) >= created(n)) {
        *newest = Some(sig);
    }
}

/// A signature's creation time, 0 when it has none.
fn created(sig: &Signature) -> u32 {
    sig.created().map_or(0, |t| t.as_secs())
}

/// A self-signature with a hash that may still be trusted for one: everything but MD5 (SHA-1
/// self-signatures are still on many keys, and rPGP checks SHA-1 for collision attacks), and
/// of version 4 or 6.
fn usable(sig: &Signature) -> bool {
    sig.hash_alg().is_some_and(|h| h != HashAlgorithm::Md5) && matches!(sig.version(), SignatureVersion::V4 | SignatureVersion::V6)
}

/// Whether `sig` names `key` as its issuer, or names nobody (a hint only: the check decides).
fn is_self(sig: &Signature, key: &impl KeyDetails) -> bool {
    let (ids, fps) = (sig.issuer_key_id(), sig.issuer_fingerprint());
    (ids.is_empty() && fps.is_empty()) || ids.iter().any(|id| **id == key.legacy_key_id()) || fps.iter().any(|fp| **fp == key.fingerprint())
}

/// The key flags a self-signature states, if it states any.
fn flags(sig: &Signature) -> Option<KeyFlags> {
    sig.config()?.hashed_subpackets.iter().find_map(|p| match &p.data {
        SubpacketData::KeyFlags(flags) => Some(flags.clone()),
        _ => None,
    })
}

/// When a key made at `created` stops being valid by self-signature `sig`.
fn expiry(created: u32, sig: &Signature) -> Option<u64> {
    let after = sig.key_expiration_time()?.as_secs();
    (after > 0).then(|| u64::from(created).saturating_add(u64::from(after)))
}

/// How many packet headers `bytes` holds, counted without reading them (RFC 9580 §4.2): a
/// cap on work for a block of many tiny packets.
fn count_packets(bytes: &[u8]) -> usize {
    packet_tags(bytes, MAX_PACKETS.saturating_add(1)).len()
}

/// The tags of the first `max` packets of `bytes` (RFC 9580 §4.2), read from their headers
/// alone: as far as a header reads, and a packet of partial length as the last.
pub(crate) fn packet_tags(bytes: &[u8], max: usize) -> Vec<u8> {
    packets(bytes, max).0
}

/// [`packet_tags`], with where the packets read end: the length of `bytes` when they are all
/// there is, short of it when something follows that isn't a packet (or a packet of partial or
/// indeterminate length, or more than `max`), past it when the last says more than is there.
pub(crate) fn packets(bytes: &[u8], max: usize) -> (Vec<u8>, usize) {
    let mut at = 0usize;
    let mut tags = Vec::new();
    while let Some(&tag) = bytes.get(at) {
        if tag & 0x80 == 0 || tags.len() >= max {
            break;
        }
        tags.push(if tag & 0x40 != 0 { tag & 0x3f } else { (tag >> 2) & 0x0f });
        let len = |o: usize| bytes.get(at.saturating_add(o)).copied().map_or(0usize, usize::from);
        let (header, body) = if tag & 0x40 != 0 {
            match len(1) {
                0..=191 => (2, len(1)),
                192..=223 => (3, ((len(1) - 192) << 8) + len(2) + 192),
                255 => (6, (len(2) << 24) | (len(3) << 16) | (len(4) << 8) | len(5)),
                // A partial length: the rest is one stream, counted as one.
                _ => break,
            }
        } else {
            match tag & 0x03 {
                0 => (2, len(1)),
                1 => (3, (len(1) << 8) | len(2)),
                2 => (5, (len(1) << 24) | (len(2) << 16) | (len(3) << 8) | len(4)),
                _ => break,
            }
        };
        at = at.saturating_add(header).saturating_add(body);
    }
    (tags, at)
}

/// Of two revocations of one thing, the one that counts for more: a hard one (it undoes
/// everything), else the earlier.
fn stronger<'a>(a: Option<&'a Signature>, b: &'a Signature) -> Option<&'a Signature> {
    Some(match a {
        Some(a) if Revoked::of(a).hard => a,
        _ if Revoked::of(b).hard => b,
        Some(a) if created(a) <= created(b) => a,
        _ => b,
    })
}

/// `a`, then what of `b` isn't in it: each signature once, `a`'s first. Both are what a check
/// kept of a key (signatures its owner made, at most two of a kind for each part), so nothing
/// is cut, and no date anybody wrote decides what stays.
fn union(a: &[Signature], b: &[Signature]) -> Vec<Signature> {
    let mut all = a.to_vec();
    for sig in b {
        if !all.contains(sig) {
            all.push(sig.clone());
        }
    }
    all
}

/// Two checked copies of one certificate as one (RFC 9580 §10.1): every User ID, subkey and
/// self-signature of either, the old copy's first, so that a new copy can only add to it.
fn merge(old: &SignedPublicKey, new: &SignedPublicKey) -> SignedPublicKey {
    let mut merged = old.clone();
    merged.details.revocation_signatures = union(&old.details.revocation_signatures, &new.details.revocation_signatures);
    merged.details.direct_signatures = union(&old.details.direct_signatures, &new.details.direct_signatures);
    for user in &new.details.users {
        let users = &mut merged.details.users;
        match users.iter().position(|u| u.id == user.id) {
            Some(i) => {
                if let Some(mine) = users.get_mut(i) {
                    mine.signatures = union(&mine.signatures, &user.signatures);
                }
            }
            None if users.len() < MAX_USERS => users.push(user.clone()),
            None => {}
        }
    }
    for sub in &new.public_subkeys {
        let fingerprint = sub.key.fingerprint();
        let subs = &mut merged.public_subkeys;
        match subs.iter().position(|s| s.key.fingerprint() == fingerprint) {
            Some(i) => {
                if let Some(mine) = subs.get_mut(i) {
                    mine.signatures = union(&mine.signatures, &sub.signatures);
                }
            }
            None if subs.len() < MAX_SUBKEYS => subs.push(sub.clone()),
            None => {}
        }
    }
    merged
}

/// `key` with nothing but its revocations: what a merge must keep when all of it can't be.
fn revocations_only(key: &SignedPublicKey) -> SignedPublicKey {
    let mut only = key.clone();
    only.details.direct_signatures.clear();
    only.details.users.clear();
    only.details.user_attributes.clear();
    for sub in &mut only.public_subkeys {
        sub.signatures.retain(|s| s.typ() == Some(SignatureType::SubkeyRevocation));
    }
    only
}

/// The Public keys the caller holds: by address, fingerprint and key id. Linear look-ups; the
/// caller decides how many keys it keeps.
#[derive(Clone, Debug, Default)]
pub struct KeyRing {
    keys: Vec<PublicKey>,
}

impl KeyRing {
    /// An empty ring.
    pub fn new() -> KeyRing {
        KeyRing::default()
    }

    /// Adds `key`. A key already here with the same fingerprint is merged with it: every User
    /// ID, subkey and self-signature of both is kept and checked again, so that a copy that
    /// lacks a revocation never undoes it and a copy that brings one always counts. Whether
    /// the ring changed.
    pub fn insert(&mut self, key: PublicKey) -> bool {
        let Some(old) = self.keys.iter_mut().find(|k| k.fingerprint == key.fingerprint) else {
            self.keys.push(key);
            return true;
        };
        let recheck = |merged: SignedPublicKey| crate::guard(|| PublicKey::checked(merged, &mut Budget::new(MAX_CHECKS_PER_CALL))).flatten();
        let merged = recheck(merge(&old.key, &key.key)).or_else(|| recheck(merge(&old.key, &revocations_only(&key.key))));
        match merged {
            Some(merged) if merged.to_bytes() != old.to_bytes() => {
                *old = merged;
                true
            }
            _ => false,
        }
    }

    /// Every key in the ring.
    pub fn keys(&self) -> &[PublicKey] {
        &self.keys
    }

    /// The keys that have `address` (an address, or a From field's value) as one of their own.
    pub fn for_address<'a>(&'a self, address: &str) -> impl Iterator<Item = &'a PublicKey> + 'a {
        let address = mime::address(address);
        self.keys.iter().filter(move |k| !address.is_empty() && k.addresses.contains(&address))
    }

    /// The key with primary fingerprint `fingerprint`.
    pub fn get(&self, fingerprint: &Fingerprint) -> Option<&PublicKey> {
        self.keys.iter().find(|k| &k.fingerprint == fingerprint)
    }

    /// How many keys the ring holds.
    pub fn len(&self) -> usize {
        self.keys.len()
    }

    /// Whether the ring holds no key.
    pub fn is_empty(&self) -> bool {
        self.keys.is_empty()
    }

    /// The signing keys (primary or subkey) named by fingerprint or key id.
    pub(crate) fn signers<'a>(&'a self, fps: &'a [Fingerprint], ids: &'a [KeyId]) -> impl Iterator<Item = (&'a PublicKey, &'a Signing)> + 'a {
        self.keys
            .iter()
            .flat_map(move |k| k.signing.iter().filter(move |s| fps.contains(&s.fingerprint) || ids.contains(&s.key_id)).map(move |s| (k, s)))
    }
}

impl FromIterator<PublicKey> for KeyRing {
    fn from_iter<I: IntoIterator<Item = PublicKey>>(iter: I) -> KeyRing {
        let mut ring = KeyRing::new();
        for key in iter {
            ring.insert(key);
        }
        ring
    }
}
