// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The algorithms a Digital signature or a certificate is checked with (RFC 8551 §2, RFC 5754,
//! RFC 4055, RFC 5480): SHA-2 digests, RSA (PKCS #1 v1.5 and PSS, 2048 bits or more) and ECDSA
//! on P-256 and P-384. MD5, SHA-1 and shorter RSA keys are no longer safe and are refused as
//! such; anything else is unknown. The primitives are RustCrypto's; this only names them.

use const_oid::ObjectIdentifier;
use der::{Decode, Encode};
use sha2::Digest;
use spki::{AlgorithmIdentifierOwned, SubjectPublicKeyInfoOwned};

/// Why a signature didn't check out.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fault {
    /// It doesn't match: what it covers was changed, or another key made it.
    Invalid,
    /// It uses an algorithm that's no longer safe (MD5, SHA-1, RSA under 2048 bits).
    Weak,
    /// It uses an algorithm that isn't known here.
    Unknown,
    /// The key or the signature can't be read, or they are of different kinds: nothing can be
    /// said of what it covers.
    Malformed,
}

/// The least RSA modulus, in bits, that is safe enough (RFC 8551 §6).
pub const MIN_RSA_BITS: usize = 2048;
/// The largest RSA modulus checked, in bits: a longer key takes too long to be worth it.
pub const MAX_RSA_BITS: usize = 8192;

const fn oid(s: &str) -> ObjectIdentifier {
    ObjectIdentifier::new_unwrap(s)
}

// Digests.
const MD5: ObjectIdentifier = oid("1.2.840.113549.2.5");
const SHA1: ObjectIdentifier = oid("1.3.14.3.2.26");
const SHA224: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.4");
const SHA256: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.1");
const SHA384: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.2");
const SHA512: ObjectIdentifier = oid("2.16.840.1.101.3.4.2.3");

// RSA.
const RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.1");
const MD2_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.2");
const MD5_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.4");
const SHA1_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.5");
const PSS: ObjectIdentifier = oid("1.2.840.113549.1.1.10");
const SHA256_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.11");
const SHA384_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.12");
const SHA512_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.13");
const SHA224_RSA: ObjectIdentifier = oid("1.2.840.113549.1.1.14");
const MGF1: ObjectIdentifier = oid("1.2.840.113549.1.1.8");

/// The bits of an RSA modulus written big-endian.
fn modulus_bits(modulus: &[u8]) -> usize {
    let mut significant = modulus.iter().skip_while(|b| **b == 0);
    let top = significant.next().copied().unwrap_or(0);
    significant.count().saturating_mul(8).saturating_add(8usize.saturating_sub(top.leading_zeros() as usize))
}

/// What kind of key `key` is: RSA of so many bits, ECDSA on P-256 or P-384, or another.
pub(crate) fn key_kind(key: &SubjectPublicKeyInfoOwned) -> crate::pkcs12::KeyKind {
    use crate::pkcs12::KeyKind;
    if key.algorithm.oid == RSA {
        return key
            .subject_public_key
            .as_bytes()
            .and_then(|b| rsa::pkcs1::RsaPublicKey::<der::asn1::UintRef<'_>>::from_der(b).ok())
            .map_or(KeyKind::Other, |k| KeyKind::Rsa { bits: modulus_bits(k.modulus.as_bytes()) });
    }
    if key.algorithm.oid == EC_KEY {
        return match key.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok()) {
            Some(P256) => KeyKind::P256,
            Some(P384) => KeyKind::P384,
            _ => KeyKind::Other,
        };
    }
    KeyKind::Other
}

/// What kind of private key a PKCS #8 `PrivateKeyInfo` holds, from its own algorithm identifier
/// (`algorithm`; for EC, its named `curve`, from the identifier's parameters or the key's own)
/// and, for RSA, its `modulus` (big-endian): never from a certificate that came with it.
pub(crate) fn private_key_kind(algorithm: ObjectIdentifier, curve: Option<ObjectIdentifier>, modulus: Option<&[u8]>) -> crate::pkcs12::KeyKind {
    use crate::pkcs12::KeyKind;
    match (algorithm, curve, modulus) {
        (RSA, _, Some(modulus)) => KeyKind::Rsa { bits: modulus_bits(modulus) },
        (EC_KEY, Some(P256), _) => KeyKind::P256,
        (EC_KEY, Some(P384), _) => KeyKind::P384,
        _ => KeyKind::Other,
    }
}

// ECDSA.
const EC_KEY: ObjectIdentifier = oid("1.2.840.10045.2.1");
const SHA1_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.1");
const SHA224_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.1");
const SHA256_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.2");
const SHA384_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.3");
const SHA512_ECDSA: ObjectIdentifier = oid("1.2.840.10045.4.3.4");
const P256: ObjectIdentifier = oid("1.2.840.10045.3.1.7");
const P384: ObjectIdentifier = oid("1.3.132.0.34");

/// A digest algorithm that is safe to rely on.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Hash {
    Sha224,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    /// The digest a CMS `digestAlgorithm` names.
    pub fn of(algorithm: &AlgorithmIdentifierOwned) -> Result<Hash, Fault> {
        Hash::by_oid(algorithm.oid)
    }

    fn by_oid(oid: ObjectIdentifier) -> Result<Hash, Fault> {
        match oid {
            SHA224 => Ok(Hash::Sha224),
            SHA256 => Ok(Hash::Sha256),
            SHA384 => Ok(Hash::Sha384),
            SHA512 => Ok(Hash::Sha512),
            MD5 | SHA1 => Err(Fault::Weak),
            _ => Err(Fault::Unknown),
        }
    }

    /// The digest of `data`.
    pub fn digest(self, data: &[u8]) -> Vec<u8> {
        match self {
            Hash::Sha224 => sha2::Sha224::digest(data).to_vec(),
            Hash::Sha256 => sha2::Sha256::digest(data).to_vec(),
            Hash::Sha384 => sha2::Sha384::digest(data).to_vec(),
            Hash::Sha512 => sha2::Sha512::digest(data).to_vec(),
        }
    }
}

/// How a signature is made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Scheme {
    Pkcs1(Hash),
    Pss(Hash, usize),
    Ecdsa(Hash),
}

/// The scheme `algorithm` names; `digest` is the CMS signer's digest algorithm, for the names
/// that leave the hash to it (`rsaEncryption`, `id-ecPublicKey`), `None` for a certificate.
fn scheme(algorithm: &AlgorithmIdentifierOwned, digest: Option<Hash>) -> Result<Scheme, Fault> {
    match algorithm.oid {
        RSA => digest.map(Scheme::Pkcs1).ok_or(Fault::Unknown),
        SHA224_RSA => Ok(Scheme::Pkcs1(Hash::Sha224)),
        SHA256_RSA => Ok(Scheme::Pkcs1(Hash::Sha256)),
        SHA384_RSA => Ok(Scheme::Pkcs1(Hash::Sha384)),
        SHA512_RSA => Ok(Scheme::Pkcs1(Hash::Sha512)),
        MD2_RSA | MD5_RSA | SHA1_RSA | SHA1_ECDSA => Err(Fault::Weak),
        PSS => pss(algorithm),
        EC_KEY => digest.map(Scheme::Ecdsa).ok_or(Fault::Unknown),
        SHA224_ECDSA => Ok(Scheme::Ecdsa(Hash::Sha224)),
        SHA256_ECDSA => Ok(Scheme::Ecdsa(Hash::Sha256)),
        SHA384_ECDSA => Ok(Scheme::Ecdsa(Hash::Sha384)),
        SHA512_ECDSA => Ok(Scheme::Ecdsa(Hash::Sha512)),
        _ => Err(Fault::Unknown),
    }
}

/// RSASSA-PSS with its parameters (RFC 4055 §3.1): the hash, MGF1 with the same hash, and the
/// salt's length. Their defaults are SHA-1, which is no longer safe.
fn pss(algorithm: &AlgorithmIdentifierOwned) -> Result<Scheme, Fault> {
    let params = algorithm.parameters.as_ref().ok_or(Fault::Weak)?;
    let der = params.to_der().map_err(|_| Fault::Unknown)?;
    let params = rsa::pkcs1::RsaPssParams::<der::Any>::from_der(&der).map_err(|_| Fault::Unknown)?;
    let hash = Hash::by_oid(params.hash.oid)?;
    if params.mask_gen.oid != MGF1 {
        return Err(Fault::Unknown);
    }
    let mgf_hash = params.mask_gen.parameters.as_ref().map(|p| p.oid).ok_or(Fault::Weak)?;
    if Hash::by_oid(mgf_hash)? != hash {
        return Err(Fault::Unknown);
    }
    Ok(Scheme::Pss(hash, usize::from(params.salt_len)))
}

/// Check `signature` over `data` with the key `key`, by `algorithm` (and `digest`, for a CMS
/// signer: see [`scheme`]).
pub fn verify(
    key: &SubjectPublicKeyInfoOwned,
    algorithm: &AlgorithmIdentifierOwned,
    digest: Option<Hash>,
    data: &[u8],
    signature: &[u8],
) -> Result<(), Fault> {
    verify_hashed(key, algorithm, digest, &mut |hash| hash.digest(data), signature)
}

/// [`verify`], with what was signed given by its digest: `hashed` gives it by each algorithm
/// (so that a long content is hashed once per algorithm however many keys are tried).
pub fn verify_hashed(
    key: &SubjectPublicKeyInfoOwned,
    algorithm: &AlgorithmIdentifierOwned,
    digest: Option<Hash>,
    hashed: &mut dyn FnMut(Hash) -> Vec<u8>,
    signature: &[u8],
) -> Result<(), Fault> {
    let scheme = scheme(algorithm, digest)?;
    match scheme {
        Scheme::Pkcs1(hash) | Scheme::Pss(hash, _) => {
            if key.algorithm.oid != RSA && key.algorithm.oid != PSS {
                return Err(Fault::Malformed);
            }
            let public = rsa_key(key)?;
            let hashed = hashed(hash);
            let checked = match scheme {
                Scheme::Pss(_, salt) => match hash {
                    Hash::Sha224 => public.verify(rsa::pss::Pss::<sha2::Sha224>::new_with_salt(salt), &hashed, signature),
                    Hash::Sha256 => public.verify(rsa::pss::Pss::<sha2::Sha256>::new_with_salt(salt), &hashed, signature),
                    Hash::Sha384 => public.verify(rsa::pss::Pss::<sha2::Sha384>::new_with_salt(salt), &hashed, signature),
                    Hash::Sha512 => public.verify(rsa::pss::Pss::<sha2::Sha512>::new_with_salt(salt), &hashed, signature),
                },
                _ => {
                    let padding = match hash {
                        Hash::Sha224 => rsa::Pkcs1v15Sign::new::<sha2::Sha224>(),
                        Hash::Sha256 => rsa::Pkcs1v15Sign::new::<sha2::Sha256>(),
                        Hash::Sha384 => rsa::Pkcs1v15Sign::new::<sha2::Sha384>(),
                        Hash::Sha512 => rsa::Pkcs1v15Sign::new::<sha2::Sha512>(),
                    };
                    public.verify(padding, &hashed, signature)
                }
            };
            checked.map_err(|_| Fault::Invalid)
        }
        Scheme::Ecdsa(hash) => {
            if key.algorithm.oid != EC_KEY {
                return Err(Fault::Malformed);
            }
            let curve = key.algorithm.parameters.as_ref().and_then(|p| p.decode_as::<ObjectIdentifier>().ok()).ok_or(Fault::Unknown)?;
            let point = key.subject_public_key.as_bytes().ok_or(Fault::Unknown)?;
            let hashed = hashed(hash);
            use p256::ecdsa::signature::hazmat::PrehashVerifier;
            match curve {
                P256 => {
                    let key = p256::ecdsa::VerifyingKey::from_sec1_bytes(point).map_err(|_| Fault::Malformed)?;
                    let sig = p256::ecdsa::Signature::from_der(signature).map_err(|_| Fault::Malformed)?;
                    key.verify_prehash(&hashed, &sig).map_err(|_| Fault::Invalid)
                }
                P384 => {
                    let key = p384::ecdsa::VerifyingKey::from_sec1_bytes(point).map_err(|_| Fault::Malformed)?;
                    let sig = p384::ecdsa::Signature::from_der(signature).map_err(|_| Fault::Malformed)?;
                    key.verify_prehash(&hashed, &sig).map_err(|_| Fault::Invalid)
                }
                _ => Err(Fault::Unknown),
            }
        }
    }
}

/// The RSA key of `key`: [`Fault::Weak`] under [`MIN_RSA_BITS`], [`Fault::Unknown`] past what
/// the library takes (8192 bits) or when it can't be read.
fn rsa_key(key: &SubjectPublicKeyInfoOwned) -> Result<rsa::PublicKey, Fault> {
    let bits = key.subject_public_key.as_bytes().ok_or(Fault::Unknown)?;
    let pkcs1 = rsa::pkcs1::RsaPublicKey::<der::asn1::UintRef<'_>>::from_der(bits).map_err(|_| Fault::Unknown)?;
    let bits = modulus_bits(pkcs1.modulus.as_bytes());
    if bits < MIN_RSA_BITS {
        return Err(Fault::Weak);
    }
    if bits > MAX_RSA_BITS {
        return Err(Fault::Unknown);
    }
    rsa::PublicKey::try_from(pkcs1).map_err(|_| Fault::Unknown)
}
