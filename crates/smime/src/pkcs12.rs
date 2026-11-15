// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! A PKCS #12 file (`.p12`, `.pfx`; RFC 7292) opened with its passphrase (#30 R13): the
//! certificate and private key an owner's IT department hands out, for the window to put in the
//! system's credential store. No I/O, and nothing kept: what is opened goes back to the caller,
//! the private key in memory that is wiped when it is let go of.
//!
//! The file is believed only when its MAC checks with the passphrase (RFC 7292 §5.1, the
//! password-integrity mode, HMAC with SHA-1 or SHA-2 keyed by the PKCS #12 key derivation of
//! Appendix B): a file without one, or with a public-key integrity mode, is refused, and a MAC
//! that doesn't check is a wrong passphrase. The bags may be encrypted with PBES2 (RFC 8018:
//! PBKDF2 with HMAC-SHA-1 or SHA-2, and AES-128/192/256-CBC or 3DES-CBC) or with the old
//! pbeWithSHAAnd3-KeyTripleDES-CBC (RFC 7292 Appendix C); RC2, RC4, two-key 3DES, DES and the
//! 40-bit schemes are no longer safe and are refused.
//!
//! The file is hostile: its size, the bags, their nesting and every iteration count (and their
//! sum) are capped, lengths are checked against what is there, and nothing here panics on any
//! bytes. The cryptography is RustCrypto's (`pkcs12`'s key derivation, `pbkdf2`, `hmac`, `sha1`,
//! `sha2`, `aes`, `des`, `cbc`); only the layout is read here.

use aes::cipher::block_padding::Pkcs7;
use aes::cipher::{BlockModeDecrypt, KeyIvInit};
use const_oid::ObjectIdentifier;
use der::Decode;
use hmac::{KeyInit, Mac};
use mailcraft_ber::{self as ber, Tlv};
use zeroize::Zeroizing;

/// The largest file opened (the window reads at most as much).
pub const MAX_P12_LEN: usize = 1 << 20;
/// The most bags read, in all.
pub const MAX_BAGS: usize = 64;
/// The most iterations one key derivation may ask for: past it the file is refused as
/// unreasonable rather than spent time on.
pub const MAX_ITERATIONS: u64 = 1_000_000;
/// The most iterations every key derivation of one file may ask for together.
const MAX_TOTAL_ITERATIONS: u64 = 8_000_000;
/// How deep safe-contents bags may nest in each other.
const MAX_NESTING: usize = 4;
/// The most certificates kept from one file.
const MAX_CERTIFICATES: usize = 32;

const DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.1");
const ENCRYPTED_DATA: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.7.6");
const KEY_BAG: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.12.10.1.1");
const SHROUDED_KEY_BAG: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.12.10.1.2");
const CERT_BAG: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.12.10.1.3");
const SAFE_CONTENTS_BAG: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.12.10.1.6");
const X509_CERTIFICATE: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.22.1");
const LOCAL_KEY_ID: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.21");

const SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.14.3.2.26");
const SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.2.3");

const PBES2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.5.13");
const PBKDF2: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.5.12");
const HMAC_SHA1: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.7");
const HMAC_SHA256: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.9");
const HMAC_SHA384: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.10");
const HMAC_SHA512: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.2.11");
const AES128_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.2");
const AES192_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.22");
const AES256_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.16.840.1.101.3.4.1.42");
const DES_EDE3_CBC: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.3.7");
const SHA_3DES: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.12.1.3");
/// The PKCS #12 schemes that are no longer safe (RC4, two-key 3DES, RC2), and PKCS #5 v1.5's
/// (DES and RC2 with MD2, MD5 or SHA-1).
const WEAK: [&str; 11] = [
    "1.2.840.113549.1.12.1.1",
    "1.2.840.113549.1.12.1.2",
    "1.2.840.113549.1.12.1.4",
    "1.2.840.113549.1.12.1.5",
    "1.2.840.113549.1.12.1.6",
    "1.2.840.113549.1.5.1",
    "1.2.840.113549.1.5.3",
    "1.2.840.113549.1.5.4",
    "1.2.840.113549.1.5.6",
    "1.2.840.113549.1.5.10",
    "1.2.840.113549.1.5.11",
];

/// Why a PKCS #12 file couldn't be opened.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Larger than any such file is.
    #[error("the file is larger than a certificate file can be")]
    TooLarge,
    /// Not a PKCS #12 file, or a damaged one.
    #[error("the file isn't a certificate file (.p12 or .pfx) that can be read")]
    Unreadable,
    /// It has no MAC with a passphrase: nothing vouches that it is as it was made.
    #[error("the file isn't protected with a passphrase, so nothing vouches for it")]
    NoMac,
    /// The MAC doesn't check: the passphrase is wrong (or the file was changed).
    #[error("the passphrase is wrong")]
    WrongPassphrase,
    /// It is encrypted with a scheme that is no longer safe (RC2, RC4, DES, 40-bit keys).
    #[error("the file is encrypted in a way that is no longer safe (RC2, RC4, DES or 40-bit keys); export it again with AES")]
    WeakEncryption,
    /// It uses an algorithm this program doesn't know.
    #[error("the file uses an algorithm MailCraft doesn't know")]
    UnknownAlgorithm,
    /// It asks for more iterations than any file needs, or more bags or nesting than there is room for.
    #[error("the file asks for more work than a certificate file needs")]
    Unreasonable,
    /// It holds no private key, or more than one.
    #[error("the file holds no private key, or more than one")]
    NoPrivateKey,
    /// It holds no certificate for its private key.
    #[error("the file holds no certificate for its private key")]
    NoCertificate,
}

/// What a PKCS #12 file holds, opened.
pub struct Opened {
    /// The certificate of the private key, in DER: the one whose public key is the key's, else
    /// the one with the key's local key id.
    pub certificate: Vec<u8>,
    /// The other certificates (the authorities on the way to a trusted one), in DER.
    pub others: Vec<Vec<u8>>,
    /// The private key as PKCS #8 `PrivateKeyInfo` in DER, wiped when let go of.
    pub private_key: Zeroizing<Vec<u8>>,
    /// What kind of key the private key is, read from its own PKCS #8 algorithm identifier (and
    /// curve): whether it may be used here. The certificate is of the same kind.
    pub key_kind: KeyKind,
    /// Whether the private key was encrypted in the file (a shrouded key bag, or a bag in an
    /// encrypted safe), and not merely vouched for by the MAC: a key in a plain key bag of a
    /// plain safe is in the file in clear.
    pub key_encrypted: bool,
}

/// What kind of key a certificate holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum KeyKind {
    /// RSA, with a modulus of so many bits.
    Rsa { bits: usize },
    /// ECDSA on NIST P-256.
    P256,
    /// ECDSA on NIST P-384.
    P384,
    /// Any other.
    Other,
}

impl std::fmt::Debug for Opened {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Opened")
            .field("certificate", &self.certificate.len())
            .field("others", &self.others.len())
            .field("private_key", &"[redacted]")
            .field("key_kind", &self.key_kind)
            .field("key_encrypted", &self.key_encrypted)
            .finish()
    }
}

/// Whether `bytes` look like a PKCS #12 file (a version 3 PFX), without opening it: so that a
/// private key is never taken for a Public key to import.
pub fn looks_like(bytes: &[u8]) -> bool {
    let Some(der) = ber::to_der(bytes.get(..bytes.len().min(MAX_P12_LEN)).unwrap_or_default()) else { return false };
    let Some((pfx, _)) = ber::read(&der) else { return false };
    let Some(fields) = ber::children(pfx.value) else { return false };
    matches!(fields.as_slice(), [version, info, ..] if version.tag == 0x02 && version.value == [3] && info.tag == 0x30)
}

/// Open the PKCS #12 file `p12` with `passphrase`.
pub fn open(p12: &[u8], passphrase: &str) -> Result<Opened, Error> {
    if p12.len() > MAX_P12_LEN {
        return Err(Error::TooLarge);
    }
    let der = ber::to_der(p12).ok_or(Error::Unreadable)?;
    let (pfx, rest) = ber::read(&der).ok_or(Error::Unreadable)?;
    if pfx.tag != 0x30 || !rest.is_empty() {
        return Err(Error::Unreadable);
    }
    let fields = ber::children(pfx.value).ok_or(Error::Unreadable)?;
    let (version, auth_safe, mac) = match fields.as_slice() {
        [v, a, m] => (v, a, Some(m)),
        [v, a] => (v, a, None),
        _ => return Err(Error::Unreadable),
    };
    if version.tag != 0x02 || version.value != [3] {
        return Err(Error::Unreadable);
    }
    let mut work = Work { iterations: 0, bags: 0 };
    // Only the password-integrity mode: the authenticated safe is data, and the MAC is over it.
    let (content_type, content) = content_info(auth_safe)?;
    if content_type != DATA {
        return Err(Error::NoMac);
    }
    let safe = octets(content)?;
    let mac = mac.ok_or(Error::NoMac)?;
    check_mac(mac, &safe, passphrase, &mut work)?;
    let safe = ber::to_der(&safe).ok_or(Error::Unreadable)?;
    let mut found = Found::default();
    let (list, rest) = ber::read(&safe).ok_or(Error::Unreadable)?;
    if list.tag != 0x30 || !rest.is_empty() {
        return Err(Error::Unreadable);
    }
    for info in ber::children(list.value).ok_or(Error::Unreadable)? {
        let (kind, content) = content_info(&info)?;
        let (contents, encrypted) = if kind == DATA {
            (octets(content)?, false)
        } else if kind == ENCRYPTED_DATA {
            (Zeroizing::new(encrypted_data(content, passphrase, &mut work)?).to_vec(), true)
        } else {
            // (Enveloped data, which needs a key of the owner's to open: not a file to import.)
            return Err(Error::UnknownAlgorithm);
        };
        let contents = Zeroizing::new(contents);
        safe_contents(&contents, Safe { passphrase, encrypted, depth: 0 }, &mut work, &mut found)?;
    }
    found.finish()
}

/// What opening costs so far.
struct Work {
    iterations: u64,
    bags: usize,
}

impl Work {
    /// Count a key derivation of `iterations`: refused past the caps.
    fn derive(&mut self, iterations: u64) -> Result<(), Error> {
        if iterations == 0 || iterations > MAX_ITERATIONS {
            return Err(Error::Unreasonable);
        }
        self.iterations = self.iterations.saturating_add(iterations);
        if self.iterations > MAX_TOTAL_ITERATIONS { Err(Error::Unreasonable) } else { Ok(()) }
    }
}

/// The bags found.
#[derive(Default)]
struct Found {
    /// Each private key, its local key id, and whether it was encrypted in the file.
    keys: Vec<(Zeroizing<Vec<u8>>, Option<Vec<u8>>, bool)>,
    certificates: Vec<(Vec<u8>, Option<Vec<u8>>)>,
}

impl Found {
    /// The one private key and its certificate: the one whose public key is the key's, else
    /// (a key that doesn't carry its public half) the one with the key's local key id, when it
    /// is of the very kind the key is (algorithm and curve). The kind said is the private key's.
    fn finish(mut self) -> Result<Opened, Error> {
        let (key, key_id, key_encrypted) = match self.keys.len() {
            1 => self.keys.pop().ok_or(Error::NoPrivateKey)?,
            _ => return Err(Error::NoPrivateKey),
        };
        let key_kind = private_key_kind(&key);
        let kind_of = |c: &[u8]| crate::cert::Cert::from_der(c).map_or(KeyKind::Other, |c| crate::algorithms::key_kind(c.public_key()));
        let public = public_of(&key);
        let by_key = public.as_ref().and_then(|p| self.certificates.iter().position(|(c, _)| certificate_key(c).as_ref() == Some(p)));
        let by_id = key_id.as_ref().and_then(|id| self.certificates.iter().position(|(c, cid)| cid.as_ref() == Some(id) && kind_of(c) == key_kind));
        let at = by_key.or(by_id).ok_or(Error::NoCertificate)?;
        let (certificate, _) = self.certificates.remove(at);
        if kind_of(&certificate) != key_kind {
            return Err(Error::NoCertificate);
        }
        Ok(Opened { certificate, others: self.certificates.into_iter().map(|(c, _)| c).collect(), private_key: key, key_kind, key_encrypted })
    }
}

/// What kind of key a PKCS #8 `PrivateKeyInfo` is, by its own algorithm identifier: RSA by its
/// modulus, EC by its named curve (the identifier's parameters, else the `ECPrivateKey`'s own).
fn private_key_kind(pkcs8: &[u8]) -> KeyKind {
    let read = || -> Option<KeyKind> {
        let (info, _) = ber::read(pkcs8)?;
        let fields = ber::children(info.value)?;
        let [_, alg, key, ..] = fields.as_slice() else { return None };
        if key.tag != 0x04 {
            return None;
        }
        let (oid, params) = algorithm(alg).ok()?;
        let (inner, _) = ber::read(key.value)?;
        let parts = ber::children(inner.value)?;
        let curve_of = |t: &Tlv<'_>| oid_of(t).ok();
        let curve = params.as_ref().and_then(curve_of).or_else(|| {
            // ECPrivateKey (RFC 5915): [0] parameters, a named curve.
            let explicit = parts.iter().find(|p| p.tag == 0xa0)?;
            let (named, _) = ber::read(explicit.value)?;
            curve_of(&named)
        });
        // RSAPrivateKey (RFC 8017 A.1.2): version, modulus, …
        let modulus = parts.get(1).filter(|n| n.tag == 0x02).map(|n| n.value);
        Some(crate::algorithms::private_key_kind(oid, curve, modulus))
    };
    read().unwrap_or(KeyKind::Other)
}

/// A ContentInfo's type and content (inside its `[0] EXPLICIT`).
fn content_info<'a>(info: &Tlv<'a>) -> Result<(ObjectIdentifier, Tlv<'a>), Error> {
    if info.tag != 0x30 {
        return Err(Error::Unreadable);
    }
    let fields = ber::children(info.value).ok_or(Error::Unreadable)?;
    let [oid, explicit] = fields.as_slice() else { return Err(Error::Unreadable) };
    if explicit.tag != 0xa0 {
        return Err(Error::Unreadable);
    }
    let (content, rest) = ber::read(explicit.value).ok_or(Error::Unreadable)?;
    if !rest.is_empty() {
        return Err(Error::Unreadable);
    }
    Ok((oid_of(oid)?, content))
}

fn oid_of(tlv: &Tlv<'_>) -> Result<ObjectIdentifier, Error> {
    if tlv.tag != 0x06 {
        return Err(Error::Unreadable);
    }
    ObjectIdentifier::from_der(tlv.whole).map_err(|_| Error::Unreadable)
}

/// An OCTET STRING's octets: in one piece, or (BER) in pieces under a constructed tag.
fn octets(tlv: Tlv<'_>) -> Result<Vec<u8>, Error> {
    match tlv.tag {
        0x04 | 0x80 => Ok(tlv.value.to_vec()),
        0x24 | 0xa0 => {
            let mut out = Vec::new();
            for piece in ber::children(tlv.value).ok_or(Error::Unreadable)? {
                if piece.tag != 0x04 {
                    return Err(Error::Unreadable);
                }
                out.extend_from_slice(piece.value);
            }
            Ok(out)
        }
        _ => Err(Error::Unreadable),
    }
}

/// An INTEGER that fits, not negative.
fn uint(tlv: &Tlv<'_>) -> Result<u64, Error> {
    if tlv.tag != 0x02 || tlv.value.is_empty() || tlv.value.len() > 9 || tlv.value.first().is_some_and(|b| b & 0x80 != 0) {
        return Err(Error::Unreasonable);
    }
    let digits = tlv.value.strip_prefix(&[0]).unwrap_or(tlv.value);
    if digits.len() > 8 {
        return Err(Error::Unreasonable);
    }
    Ok(digits.iter().fold(0u64, |acc, d| (acc << 8) | u64::from(*d)))
}

/// An AlgorithmIdentifier: its OID and its parameters, if any.
fn algorithm<'a>(tlv: &Tlv<'a>) -> Result<(ObjectIdentifier, Option<Tlv<'a>>), Error> {
    if tlv.tag != 0x30 {
        return Err(Error::Unreadable);
    }
    let fields = ber::children(tlv.value).ok_or(Error::Unreadable)?;
    match fields.as_slice() {
        [oid] => Ok((oid_of(oid)?, None)),
        [oid, params] => Ok((oid_of(oid)?, (params.tag != 0x05).then_some(*params))),
        _ => Err(Error::Unreadable),
    }
}

/// The hashes a MAC or a key derivation is made with.
#[derive(Clone, Copy)]
enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    fn digest(oid: ObjectIdentifier) -> Result<Hash, Error> {
        Ok(match oid {
            SHA1 => Hash::Sha1,
            SHA256 => Hash::Sha256,
            SHA384 => Hash::Sha384,
            SHA512 => Hash::Sha512,
            _ => return Err(Error::UnknownAlgorithm),
        })
    }

    fn hmac(oid: ObjectIdentifier) -> Result<Hash, Error> {
        Ok(match oid {
            HMAC_SHA1 => Hash::Sha1,
            HMAC_SHA256 => Hash::Sha256,
            HMAC_SHA384 => Hash::Sha384,
            HMAC_SHA512 => Hash::Sha512,
            _ => return Err(Error::UnknownAlgorithm),
        })
    }

    fn len(self) -> usize {
        match self {
            Hash::Sha1 => 20,
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
            Hash::Sha512 => 64,
        }
    }

    /// The PKCS #12 key derivation (RFC 7292 Appendix B.2) of `len` bytes for `purpose`.
    fn pkcs12_kdf(self, password: &[u8], salt: &[u8], purpose: Purpose, iterations: u64, len: usize) -> Result<Zeroizing<Vec<u8>>, Error> {
        use pkcs12::kdf::{Pkcs12KeyType, derive_key};
        let id = match purpose {
            Purpose::Key => Pkcs12KeyType::EncryptionKey,
            Purpose::Iv => Pkcs12KeyType::Iv,
            Purpose::Mac => Pkcs12KeyType::Mac,
        };
        let rounds = i32::try_from(iterations).map_err(|_| Error::Unreasonable)?;
        let out = match self {
            Hash::Sha1 => derive_key::<sha1::Sha1>(password, salt, id, rounds, len),
            Hash::Sha256 => derive_key::<sha2::Sha256>(password, salt, id, rounds, len),
            Hash::Sha384 => derive_key::<sha2::Sha384>(password, salt, id, rounds, len),
            Hash::Sha512 => derive_key::<sha2::Sha512>(password, salt, id, rounds, len),
        };
        out.map(Zeroizing::new).map_err(|_| Error::Unreadable)
    }

    /// Whether the HMAC of `data` under `key` is `tag`, in constant time.
    fn hmac_checks(self, key: &[u8], data: &[u8], tag: &[u8]) -> bool {
        fn check<M: Mac + KeyInit>(key: &[u8], data: &[u8], tag: &[u8]) -> bool {
            match <M as KeyInit>::new_from_slice(key) {
                Ok(mut mac) => {
                    mac.update(data);
                    mac.verify_slice(tag).is_ok()
                }
                Err(_) => false,
            }
        }
        match self {
            Hash::Sha1 => check::<hmac::Hmac<sha1::Sha1>>(key, data, tag),
            Hash::Sha256 => check::<hmac::Hmac<sha2::Sha256>>(key, data, tag),
            Hash::Sha384 => check::<hmac::Hmac<sha2::Sha384>>(key, data, tag),
            Hash::Sha512 => check::<hmac::Hmac<sha2::Sha512>>(key, data, tag),
        }
    }

    fn pbkdf2(self, password: &[u8], salt: &[u8], iterations: u32, out: &mut [u8]) {
        match self {
            Hash::Sha1 => pbkdf2::pbkdf2_hmac::<sha1::Sha1>(password, salt, iterations, out),
            Hash::Sha256 => pbkdf2::pbkdf2_hmac::<sha2::Sha256>(password, salt, iterations, out),
            Hash::Sha384 => pbkdf2::pbkdf2_hmac::<sha2::Sha384>(password, salt, iterations, out),
            Hash::Sha512 => pbkdf2::pbkdf2_hmac::<sha2::Sha512>(password, salt, iterations, out),
        }
    }
}

#[derive(Clone, Copy)]
enum Purpose {
    Key,
    Iv,
    Mac,
}

/// The passphrase as the PKCS #12 key derivation takes it (RFC 7292 Appendix B.1): a BMPString,
/// two bytes to a character, ended by two zero bytes. `None` for a character outside the BMP.
fn bmp(passphrase: &str) -> Option<Zeroizing<Vec<u8>>> {
    let mut out = Zeroizing::new(Vec::with_capacity(passphrase.len().saturating_mul(2).saturating_add(2)));
    for c in passphrase.chars() {
        let unit = u16::try_from(u32::from(c)).ok()?;
        out.extend_from_slice(&unit.to_be_bytes());
    }
    out.extend_from_slice(&[0, 0]);
    Some(out)
}

/// The forms of the passphrase a file may have been made with: the BMPString, and for an empty
/// one also nothing at all (which some writers use).
fn passwords(passphrase: &str) -> Vec<Zeroizing<Vec<u8>>> {
    let mut out: Vec<Zeroizing<Vec<u8>>> = bmp(passphrase).into_iter().collect();
    if passphrase.is_empty() {
        out.push(Zeroizing::new(Vec::new()));
    }
    out
}

/// Check the file's MAC (RFC 7292 §5.1, Appendix B.4) over `safe`.
fn check_mac(mac: &Tlv<'_>, safe: &[u8], passphrase: &str, work: &mut Work) -> Result<(), Error> {
    if mac.tag != 0x30 {
        return Err(Error::Unreadable);
    }
    let fields = ber::children(mac.value).ok_or(Error::Unreadable)?;
    let (digest_info, salt, iterations) = match fields.as_slice() {
        [d, s] => (d, s, 1),
        [d, s, i] => (d, s, uint(i)?),
        _ => return Err(Error::Unreadable),
    };
    if digest_info.tag != 0x30 || salt.tag != 0x04 {
        return Err(Error::Unreadable);
    }
    let parts = ber::children(digest_info.value).ok_or(Error::Unreadable)?;
    let [alg, digest] = parts.as_slice() else { return Err(Error::Unreadable) };
    if digest.tag != 0x04 {
        return Err(Error::Unreadable);
    }
    let (oid, _) = algorithm(alg)?;
    let hash = Hash::digest(oid)?;
    if digest.value.len() != hash.len() {
        return Err(Error::Unreadable);
    }
    for password in passwords(passphrase) {
        work.derive(iterations)?;
        let key = hash.pkcs12_kdf(&password, salt.value, Purpose::Mac, iterations, hash.len())?;
        if hash.hmac_checks(&key, safe, digest.value) {
            return Ok(());
        }
    }
    Err(Error::WrongPassphrase)
}

/// The contents of an EncryptedData (RFC 5652 §8), decrypted.
fn encrypted_data(content: Tlv<'_>, passphrase: &str, work: &mut Work) -> Result<Vec<u8>, Error> {
    if content.tag != 0x30 {
        return Err(Error::Unreadable);
    }
    let fields = ber::children(content.value).ok_or(Error::Unreadable)?;
    let info = fields.get(1).ok_or(Error::Unreadable)?;
    if info.tag != 0x30 {
        return Err(Error::Unreadable);
    }
    let parts = ber::children(info.value).ok_or(Error::Unreadable)?;
    let [kind, alg, encrypted] = parts.as_slice() else { return Err(Error::Unreadable) };
    if oid_of(kind)? != DATA {
        return Err(Error::Unreadable);
    }
    let ciphertext = octets(*encrypted)?;
    decrypt(alg, &ciphertext, passphrase, work).map(|z| z.to_vec())
}

/// `ciphertext` decrypted with the passphrase as the algorithm identifier `alg` says.
fn decrypt(alg: &Tlv<'_>, ciphertext: &[u8], passphrase: &str, work: &mut Work) -> Result<Zeroizing<Vec<u8>>, Error> {
    let (oid, params) = algorithm(alg)?;
    if WEAK.iter().any(|w| oid.to_string() == *w) {
        return Err(Error::WeakEncryption);
    }
    let params = params.ok_or(Error::Unreadable)?;
    if oid == SHA_3DES {
        // RFC 7292 Appendix C: pkcs-12PbeParams, the key and IV from the PKCS #12 derivation.
        let fields = ber::children(params.value).ok_or(Error::Unreadable)?;
        let [salt, iterations] = fields.as_slice() else { return Err(Error::Unreadable) };
        if salt.tag != 0x04 {
            return Err(Error::Unreadable);
        }
        let iterations = uint(iterations)?;
        let password = bmp(passphrase).ok_or(Error::WrongPassphrase)?;
        work.derive(iterations)?;
        let key = Hash::Sha1.pkcs12_kdf(&password, salt.value, Purpose::Key, iterations, 24)?;
        work.derive(iterations)?;
        let iv = Hash::Sha1.pkcs12_kdf(&password, salt.value, Purpose::Iv, iterations, 8)?;
        return cbc_decrypt::<des::TdesEde3>(&key, &iv, ciphertext);
    }
    if oid != PBES2 {
        return Err(Error::UnknownAlgorithm);
    }
    let fields = ber::children(params.value).ok_or(Error::Unreadable)?;
    let [kdf, scheme] = fields.as_slice() else { return Err(Error::Unreadable) };
    let (kdf_oid, kdf_params) = algorithm(kdf)?;
    if kdf_oid != PBKDF2 {
        return Err(Error::UnknownAlgorithm);
    }
    let kdf_fields = ber::children(kdf_params.ok_or(Error::Unreadable)?.value).ok_or(Error::Unreadable)?;
    let mut kdf_fields = kdf_fields.iter();
    let salt = kdf_fields.next().filter(|s| s.tag == 0x04).ok_or(Error::UnknownAlgorithm)?;
    let iterations = uint(kdf_fields.next().ok_or(Error::Unreadable)?)?;
    let mut prf = Hash::Sha1;
    let mut key_length: Option<u64> = None;
    for field in kdf_fields {
        match field.tag {
            0x02 if key_length.is_none() => key_length = Some(uint(field)?),
            0x30 => prf = Hash::hmac(algorithm(field)?.0)?,
            _ => return Err(Error::Unreadable),
        }
    }
    let (cipher, iv) = algorithm(scheme)?;
    let iv = iv.filter(|v| v.tag == 0x04).ok_or(Error::Unreadable)?;
    let size = match cipher {
        AES128_CBC => 16,
        AES192_CBC => 24,
        AES256_CBC => 32,
        DES_EDE3_CBC => 24,
        _ => return Err(Error::UnknownAlgorithm),
    };
    if key_length.is_some_and(|l| l != size as u64) {
        return Err(Error::Unreadable);
    }
    work.derive(iterations)?;
    let rounds = u32::try_from(iterations).map_err(|_| Error::Unreasonable)?;
    let mut key = Zeroizing::new(vec![0u8; size]);
    // (RFC 8018 takes the passphrase as octets: its UTF-8, as the writers do.)
    prf.pbkdf2(passphrase.as_bytes(), salt.value, rounds, &mut key);
    match cipher {
        AES128_CBC => cbc_decrypt::<aes::Aes128>(&key, iv.value, ciphertext),
        AES192_CBC => cbc_decrypt::<aes::Aes192>(&key, iv.value, ciphertext),
        AES256_CBC => cbc_decrypt::<aes::Aes256>(&key, iv.value, ciphertext),
        _ => cbc_decrypt::<des::TdesEde3>(&key, iv.value, ciphertext),
    }
}

/// `ciphertext` decrypted in CBC mode with PKCS #7 padding. A wrong key shows as bad padding,
/// which is said as a wrong passphrase (the MAC has checked by then, so it seldom is).
fn cbc_decrypt<C>(key: &[u8], iv: &[u8], ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>, Error>
where
    C: aes::cipher::BlockCipherDecrypt + aes::cipher::KeyInit,
    cbc::Decryptor<C>: KeyIvInit + BlockModeDecrypt,
{
    let decryptor = cbc::Decryptor::<C>::new_from_slices(key, iv).map_err(|_| Error::Unreadable)?;
    let mut buffer = Zeroizing::new(ciphertext.to_vec());
    let len = decryptor.decrypt_padded::<Pkcs7>(&mut buffer).map_err(|_| Error::WrongPassphrase)?.len();
    buffer.truncate(len);
    Ok(buffer)
}

/// Where a SafeContents is: the passphrase, whether it came out of an encrypted safe, and how
/// deep in other bags.
#[derive(Clone, Copy)]
struct Safe<'a> {
    passphrase: &'a str,
    encrypted: bool,
    depth: usize,
}

/// The bags of a SafeContents, into `found`.
fn safe_contents(bytes: &[u8], safe: Safe<'_>, work: &mut Work, found: &mut Found) -> Result<(), Error> {
    let Safe { passphrase, encrypted, depth } = safe;
    if depth > MAX_NESTING {
        return Err(Error::Unreasonable);
    }
    let der = Zeroizing::new(ber::to_der(bytes).ok_or(Error::Unreadable)?);
    let (list, rest) = ber::read(&der).ok_or(Error::Unreadable)?;
    if list.tag != 0x30 || !rest.is_empty() {
        return Err(Error::Unreadable);
    }
    for bag in ber::children(list.value).ok_or(Error::Unreadable)? {
        work.bags = work.bags.saturating_add(1);
        if work.bags > MAX_BAGS {
            return Err(Error::Unreasonable);
        }
        if bag.tag != 0x30 {
            return Err(Error::Unreadable);
        }
        let fields = ber::children(bag.value).ok_or(Error::Unreadable)?;
        let (kind, value, attributes) = match fields.as_slice() {
            [k, v] => (k, v, None),
            [k, v, a] => (k, v, Some(a)),
            _ => return Err(Error::Unreadable),
        };
        if value.tag != 0xa0 {
            return Err(Error::Unreadable);
        }
        let (value, rest) = ber::read(value.value).ok_or(Error::Unreadable)?;
        if !rest.is_empty() {
            return Err(Error::Unreadable);
        }
        let key_id = attributes.and_then(local_key_id);
        match oid_of(kind)? {
            KEY_BAG => found.keys.push((Zeroizing::new(value.whole.to_vec()), key_id, encrypted)),
            SHROUDED_KEY_BAG => {
                // EncryptedPrivateKeyInfo (RFC 5958 §3).
                if value.tag != 0x30 {
                    return Err(Error::Unreadable);
                }
                let parts = ber::children(value.value).ok_or(Error::Unreadable)?;
                let [alg, data] = parts.as_slice() else { return Err(Error::Unreadable) };
                let ciphertext = octets(*data)?;
                let key = decrypt(alg, &ciphertext, passphrase, work)?;
                if ber::read(&key).is_none_or(|(k, rest)| k.tag != 0x30 || !rest.is_empty()) {
                    return Err(Error::WrongPassphrase);
                }
                found.keys.push((key, key_id, true));
            }
            CERT_BAG => {
                if value.tag != 0x30 {
                    return Err(Error::Unreadable);
                }
                let parts = ber::children(value.value).ok_or(Error::Unreadable)?;
                let [cert_kind, cert] = parts.as_slice() else { return Err(Error::Unreadable) };
                // (SDSI certificates are of no use for mail: left out.)
                if oid_of(cert_kind)? != X509_CERTIFICATE || cert.tag != 0xa0 {
                    continue;
                }
                let (octet, _) = ber::read(cert.value).ok_or(Error::Unreadable)?;
                let der = ber::to_der(&octets(octet)?).ok_or(Error::Unreadable)?;
                if found.certificates.len() < MAX_CERTIFICATES && crate::cert::Cert::from_der(&der).is_some() {
                    found.certificates.push((der, key_id));
                }
            }
            SAFE_CONTENTS_BAG => safe_contents(value.whole, Safe { depth: depth.saturating_add(1), ..safe }, work, found)?,
            // CRLs and secrets: of no use here.
            _ => {}
        }
    }
    Ok(())
}

/// A bag's localKeyId attribute (PKCS #9), when it has one.
fn local_key_id(attributes: &Tlv<'_>) -> Option<Vec<u8>> {
    for attribute in ber::children(attributes.value)? {
        let parts = ber::children(attribute.value)?;
        let [oid, values] = parts.as_slice() else { continue };
        if oid_of(oid).ok()? == LOCAL_KEY_ID {
            let (value, _) = ber::read(values.value)?;
            return (value.tag == 0x04).then(|| value.value.to_vec());
        }
    }
    None
}

/// What of a private key (PKCS #8) its certificate's public key must match: an RSA key's modulus
/// and exponent as `RSAPublicKey`, an EC key's public point as written in it.
fn public_of(pkcs8: &[u8]) -> Option<Vec<u8>> {
    let (info, _) = ber::read(pkcs8)?;
    let fields = ber::children(info.value)?;
    let [_, alg, key, ..] = fields.as_slice() else { return None };
    if key.tag != 0x04 {
        return None;
    }
    let (oid, _) = algorithm(alg).ok()?;
    let (inner, _) = ber::read(key.value)?;
    let parts = ber::children(inner.value)?;
    if oid.to_string() == "1.2.840.113549.1.1.1" {
        // RSAPrivateKey (RFC 8017 A.1.2): version, modulus, publicExponent, …
        let [_, n, e, ..] = parts.as_slice() else { return None };
        let mut body = n.whole.to_vec();
        body.extend_from_slice(e.whole);
        return Some(rsa_public(&body));
    }
    // ECPrivateKey (RFC 5915): version, privateKey, [0] parameters, [1] publicKey BIT STRING.
    let public = parts.iter().find(|p| p.tag == 0xa1)?;
    let (bits, _) = ber::read(public.value)?;
    (bits.tag == 0x03).then(|| bits.value.to_vec())
}

/// `RSAPublicKey ::= SEQUENCE { modulus, publicExponent }` around `body`.
fn rsa_public(body: &[u8]) -> Vec<u8> {
    let mut out = vec![0x30];
    let len = body.len();
    if len < 0x80 {
        out.push(len as u8);
    } else {
        let bytes: Vec<u8> = len.to_be_bytes().into_iter().skip_while(|b| *b == 0).collect();
        out.push(0x80 | bytes.len() as u8);
        out.extend(bytes);
    }
    out.extend_from_slice(body);
    out
}

/// A certificate's public key as [`public_of`] gives it: the subjectPublicKey's bits.
fn certificate_key(der: &[u8]) -> Option<Vec<u8>> {
    let cert = crate::cert::Cert::from_der(der)?;
    let spki = cert.public_key();
    let bits = spki.subject_public_key.as_bytes()?;
    if spki.algorithm.oid.to_string() == "1.2.840.113549.1.1.1" {
        return Some(bits.to_vec());
    }
    // (An EC point: the BIT STRING's octets, after its unused-bits byte, as `public_of` gives.)
    let mut with_unused = vec![0u8];
    with_unused.extend_from_slice(bits);
    Some(with_unused)
}
