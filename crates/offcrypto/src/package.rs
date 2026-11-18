//! Encrypted packages: ECMA-376 document encryption, from [MS-OFFCRYPTO] §2.3.4.4 and
//! §2.3.4.10–§2.3.4.15 (agile), §2.3.4.5–§2.3.4.9 (standard), with the data-spaces streams of
//! §2.1 / §2.3.4.1–§2.3.4.3.

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use base64::Engine as _;
use hmac::Mac;
use sha2::Digest;
use zeroize::Zeroizing;

use craft_xml::El;

use crate::Error;

/// The spin count Word uses, and [`encrypt`] writes.
pub const DEFAULT_SPIN_COUNT: u32 = 100_000;
/// The largest spin count the specification allows (§2.3.4.10 `ST_SpinCount`): it bounds the
/// work a hostile file can ask for.
pub(crate) const MAX_SPIN_COUNT: u32 = 10_000_000;
/// Word's limit on password length, in characters.
pub const MAX_PASSWORD_CHARS: usize = 255;
/// `EncryptedPackage` is encrypted in segments of this many bytes (§2.3.4.15).
const SEGMENT: usize = 4096;
/// Largest `EncryptionInfo` stream read (the XML is a few kilobytes).
const MAX_INFO: u64 = 1 << 20;
/// AES's block size.
const BLOCK: usize = 16;
/// Salt length written.
pub(crate) const SALT: usize = 16;

/// Block keys (§2.3.4.13, §2.3.4.14).
const BK_VERIFIER_INPUT: [u8; 8] = [0xfe, 0xa7, 0xd2, 0x76, 0x3b, 0x4b, 0x9e, 0x79];
const BK_VERIFIER_VALUE: [u8; 8] = [0xd7, 0xaa, 0x0f, 0x6d, 0x30, 0x61, 0x34, 0x4e];
const BK_KEY_VALUE: [u8; 8] = [0x14, 0x6e, 0x0b, 0xe7, 0xab, 0xac, 0xd0, 0xd6];
const BK_HMAC_KEY: [u8; 8] = [0x5f, 0xb2, 0xad, 0x01, 0x0c, 0xb9, 0xe1, 0xf6];
const BK_HMAC_VALUE: [u8; 8] = [0xa0, 0x67, 0x7f, 0x02, 0xb2, 0x2c, 0x84, 0x33];

/// Standard encryption (§2.3.2, §2.3.4.5–§2.3.4.7): `EncryptionHeader.Flags` bits, the AES and
/// SHA-1 algorithm ids, the provider type, the fixed spin count and the SHA-1 size.
const F_CRYPTO_API: u32 = 0x04;
const F_EXTERNAL: u32 = 0x10;
const F_AES: u32 = 0x20;
const STANDARD_FLAGS: u32 = F_CRYPTO_API | F_AES;
const ALG_AES_128: u32 = 0x660E;
const ALG_AES_192: u32 = 0x660F;
const ALG_AES_256: u32 = 0x6610;
const ALG_SHA1: u32 = 0x8004;
const PROV_RSA_AES: u32 = 0x18;
const STANDARD_SPIN_COUNT: u32 = 50_000;
const SHA1_SIZE: usize = 20;

const NS_ENCRYPTION: &str = "http://schemas.microsoft.com/office/2006/encryption";
const NS_PASSWORD: &str = "http://schemas.microsoft.com/office/2006/keyEncryptor/password";

/// The root-level stream called `name`.
fn root_stream(cf: &craft_cfb::CompoundFile<'_>, name: &str) -> Option<usize> {
    cf.child(0, name).filter(|&id| cf.entry(id).is_some_and(|e| e.kind == craft_cfb::EntryKind::Stream))
}

/// Is this an encrypted Office package (a compound file with an `EncryptedPackage` stream)
/// rather than a zip? A rights-managed file counts: it has the stream but no `EncryptionInfo`,
/// and [`decrypt`] names it as unsupported.
pub fn is_encrypted(bytes: &[u8]) -> bool {
    craft_cfb::CompoundFile::open(bytes).is_ok_and(|cf| root_stream(&cf, "EncryptedPackage").is_some())
}

/// Whether the right password could decrypt this file, checked without one: an error names a
/// scheme no password opens here ([`Error::Unsupported`]) or damage, so a caller can say so
/// instead of asking for a password first.
pub fn check(bytes: &[u8]) -> Result<(), Error> {
    match decrypt(bytes, None) {
        Ok(_) | Err(Error::PasswordRequired) => Ok(()),
        Err(e) => Err(e),
    }
}

/// The zip package inside an encrypted Office file. Without a password (`None`) this says
/// [`Error::PasswordRequired`] once it knows the file is one it can decrypt; a password that
/// doesn't fit gives [`Error::WrongPassword`].
pub fn decrypt(bytes: &[u8], password: Option<&str>) -> Result<Vec<u8>, Error> {
    let cf = craft_cfb::CompoundFile::open(bytes).map_err(|e| bad(format!("not a readable compound file ({e})")))?;
    let package = root_stream(&cf, "EncryptedPackage").ok_or_else(|| bad("no EncryptedPackage stream"))?;
    // Rights management (IRM) keeps the package but describes its encryption elsewhere.
    let info = root_stream(&cf, "EncryptionInfo").ok_or_else(|| Error::Unsupported("rights-management".into()))?;
    let info = stream(&cf, info, "EncryptionInfo", MAX_INFO)?;
    let package = stream(&cf, package, "EncryptedPackage", bytes.len() as u64)?;
    decrypt_streams(&info, &package, password)
}

/// [`decrypt`] from the contents of the two streams.
pub fn decrypt_streams(info: &[u8], package: &[u8], password: Option<&str>) -> Result<Vec<u8>, Error> {
    if matches!(info.get(..4), Some([2..=4, 0, 2, 0])) {
        return decrypt_standard(info, package, password);
    }
    let info = parse_info(info)?;
    let Some(password) = password else { return Err(Error::PasswordRequired) };
    let pe = &info.password;
    let hn = iterate(pe.hash, &pe.salt, password, pe.spin_count);
    // The verifier (§2.3.4.13): a random value and its hash, both encrypted with keys derived
    // from the password. They match only for the right password.
    let key_input = derive_key(pe.hash, &hn, &BK_VERIFIER_INPUT, pe.key_bytes);
    let key_value = derive_key(pe.hash, &hn, &BK_VERIFIER_VALUE, pe.key_bytes);
    let iv = make_iv(pe.hash, &pe.salt, None, pe.block_size);
    let input = Zeroizing::new(cbc_decrypt(&key_input, &iv, &pe.verifier_input)?);
    let input = input.get(..pe.salt_size).ok_or_else(|| bad("the password verifier is too short"))?;
    let value = Zeroizing::new(cbc_decrypt(&key_value, &iv, &pe.verifier_hash)?);
    let value = value.get(..pe.hash.size()).ok_or_else(|| bad("the password verifier hash is too short"))?;
    if !same(&pe.hash.hash(&[input]), value) {
        return Err(Error::WrongPassword);
    }
    // The intermediate key that encrypts the package.
    let key_key = derive_key(pe.hash, &hn, &BK_KEY_VALUE, pe.key_bytes);
    let key = Zeroizing::new(cbc_decrypt(&key_key, &iv, &pe.key_value)?);
    let key = Zeroizing::new(key.get(..info.key_bytes).ok_or_else(|| bad("the encrypted key is too short"))?.to_vec());
    // Data integrity (§2.3.4.14): an HMAC of the whole EncryptedPackage stream. It is required:
    // a file that could drop it would let altered ciphertext, or an altered StreamSize, through
    // unauthenticated (`parse_info` refuses a file without it).
    let (enc_key, enc_value) = &info.integrity;
    let hmac_key = Zeroizing::new(cbc_decrypt(&key, &make_iv(info.hash, &info.salt, Some(&BK_HMAC_KEY), info.block_size), enc_key)?);
    // The key is the salt, zero-padded to the block size; HMAC pads short keys with zeros
    // anyway, so the padded key gives the same HMAC.
    let hmac_key = hmac_key.get(..hmac_key.len().min(info.hash.size())).unwrap_or(&[]);
    let expected = cbc_decrypt(&key, &make_iv(info.hash, &info.salt, Some(&BK_HMAC_VALUE), info.block_size), enc_value)?;
    let expected = expected.get(..info.hash.size()).ok_or_else(|| bad("the integrity check value is too short"))?;
    if !same(&info.hash.hmac(hmac_key, package)?, expected) {
        return Err(bad("the document is damaged (its integrity check failed)"));
    }
    // The package itself (§2.3.4.15): StreamSize, then 4096-byte segments, each with its own IV.
    let size = package
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| bad("the encrypted package is truncated"))?;
    let data = package.get(8..).unwrap_or(&[]);
    let size = usize::try_from(size).ok().filter(|s| *s <= data.len()).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let needed = size.div_ceil(BLOCK).saturating_mul(BLOCK);
    let data = data.get(..needed).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let mut out = Vec::with_capacity(needed);
    for (i, seg) in data.chunks(SEGMENT).enumerate() {
        let i = u32::try_from(i).map_err(|_| bad("the encrypted package is too large"))?;
        let iv = make_iv(info.hash, &info.salt, Some(&i.to_le_bytes()), info.block_size);
        out.extend_from_slice(&cbc_decrypt(&key, &iv, seg)?);
    }
    out.truncate(size);
    Ok(out)
}

/// [`decrypt`] for Standard encryption (§2.3.4.5–§2.3.4.9): one AES key from the password,
/// checked with the verifier, and the whole package AES-ECB encrypted after its 8-byte size.
fn decrypt_standard(info: &[u8], package: &[u8], password: Option<&str>) -> Result<Vec<u8>, Error> {
    let s = parse_standard(info)?;
    let Some(password) = password else { return Err(Error::PasswordRequired) };
    let key = standard_key(s.salt, password, s.key_bytes);
    // The verifier (§2.3.4.9): a random value and its SHA-1, both encrypted with the key.
    let verifier = Zeroizing::new(ecb(&key, s.verifier, false)?);
    let hash = Zeroizing::new(ecb(&key, s.verifier_hash, false)?);
    let hash = hash.get(..SHA1_SIZE).ok_or_else(|| bad("the password verifier hash is too short"))?;
    if !same(&Hash::Sha1.hash(&[&verifier]), hash) {
        return Err(Error::WrongPassword);
    }
    // The package (§2.3.4.4): StreamSize, then the data; bytes past the last block are ignored.
    let size = package
        .get(..8)
        .and_then(|b| <[u8; 8]>::try_from(b).ok())
        .map(u64::from_le_bytes)
        .ok_or_else(|| bad("the encrypted package is truncated"))?;
    let data = package.get(8..).unwrap_or(&[]);
    let size = usize::try_from(size).ok().filter(|s| *s <= data.len()).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let needed = size.div_ceil(BLOCK).saturating_mul(BLOCK);
    let data = data.get(..needed).ok_or_else(|| bad("the encrypted package is truncated"))?;
    let mut out = ecb(&key, data, false)?;
    out.truncate(size);
    Ok(out)
}

/// What a Standard `EncryptionInfo` stream says (§2.3.4.5): the key size, and the verifier's salt
/// and encrypted values.
struct Standard<'a> {
    key_bytes: usize,
    salt: &'a [u8],
    verifier: &'a [u8],
    verifier_hash: &'a [u8],
}

fn parse_standard(info: &[u8]) -> Result<Standard<'_>, Error> {
    let short = || bad("EncryptionInfo is truncated");
    let u32_at = |at: usize| -> Result<u32, Error> {
        let b = at.checked_add(4).and_then(|end| info.get(at..end)).ok_or_else(short)?;
        <[u8; 4]>::try_from(b).map(u32::from_le_bytes).map_err(|_| short())
    };
    let flags = u32_at(4)?;
    if flags & F_EXTERNAL != 0 {
        return Err(Error::Unsupported("external".into()));
    }
    if flags & STANDARD_FLAGS != STANDARD_FLAGS {
        return Err(Error::Unsupported("RC4".into()));
    }
    // EncryptionHeader (§2.3.2): Flags, SizeExtra, AlgID, AlgIDHash, KeySize, ProviderType, two
    // reserved words, then the CSP name, which isn't needed.
    let header_size = usize::try_from(u32_at(8)?).map_err(|_| short())?;
    if header_size < 32 {
        return Err(bad("the encryption header is too short"));
    }
    let verifier_at = header_size.checked_add(12).filter(|&v| v <= info.len()).ok_or_else(short)?;
    // AlgID 0 and AlgIDHash 0 mean "set by the flags": AES-128 and SHA-1.
    let bits = match u32_at(20)? {
        0 | ALG_AES_128 => 128,
        ALG_AES_192 => 192,
        ALG_AES_256 => 256,
        id => return Err(Error::Unsupported(format!("cipher 0x{id:04X}"))),
    };
    let hash_id = u32_at(24)?;
    if !matches!(hash_id, 0 | ALG_SHA1) {
        return Err(Error::Unsupported(format!("hash 0x{hash_id:04X}")));
    }
    if u32_at(28)? != bits {
        return Err(bad("the key size doesn't match the encryption algorithm"));
    }
    // EncryptionVerifier (§2.3.3): SaltSize (16), Salt, EncryptedVerifier, VerifierHashSize (20
    // for SHA-1), EncryptedVerifierHash (32 bytes for AES).
    if u32_at(verifier_at)? != SALT as u32 {
        return Err(bad("the salt isn't 16 bytes"));
    }
    let field = |at: usize, len: usize| verifier_at.checked_add(at).and_then(|a| info.get(a..a.checked_add(len)?)).ok_or_else(short);
    let salt = field(4, SALT)?;
    let verifier = field(4 + SALT, BLOCK)?;
    if u32_at(verifier_at.saturating_add(4 + SALT + BLOCK))? != SHA1_SIZE as u32 {
        return Err(bad("the password verifier hash isn't SHA-1"));
    }
    let verifier_hash = field(8 + SALT + BLOCK, 2 * BLOCK)?;
    Ok(Standard { key_bytes: bits as usize / 8, salt, verifier, verifier_hash })
}

/// Encrypt a zip package with `password` (agile encryption: AES-256-CBC, SHA-512, Word's spin
/// count), as Word's "Encrypt with Password" does.
pub fn encrypt(package: &[u8], password: &str) -> Result<Vec<u8>, Error> {
    encrypt_with_spin_count(package, password, DEFAULT_SPIN_COUNT)
}

/// [`encrypt`] with a chosen spin count (tests use a small one; files should use the default).
pub fn encrypt_with_spin_count(package: &[u8], password: &str, spin_count: u32) -> Result<Vec<u8>, Error> {
    check_password(password)?;
    let spin_count = spin_count.min(MAX_SPIN_COUNT);
    let hash = Hash::Sha512;
    let key_bytes = 32;
    let key_data_salt = random(SALT)?;
    let password_salt = random(SALT)?;
    let key = random(key_bytes)?;
    let verifier = random(SALT)?;
    // The HMAC key is a salt as long as the hash (MS-OFFCRYPTO §2.3.4.14).
    let hmac_salt = random(hash.size())?;

    // The package, segment by segment; the last segment is zero-padded to the block size.
    let mut stream = Vec::with_capacity(package.len().saturating_add(8 + BLOCK));
    stream.extend_from_slice(&(package.len() as u64).to_le_bytes());
    for (i, seg) in package.chunks(SEGMENT).enumerate() {
        let i = u32::try_from(i).map_err(|_| bad("the document is too large to encrypt"))?;
        let iv = make_iv(hash, &key_data_salt, Some(&i.to_le_bytes()), BLOCK);
        stream.extend_from_slice(&cbc_encrypt(&key, &iv, &padded(seg))?);
    }

    // The password key encryptor (§2.3.4.13).
    let hn = iterate(hash, &password_salt, password, spin_count);
    let iv = make_iv(hash, &password_salt, None, BLOCK);
    let enc_verifier_input = cbc_encrypt(&derive_key(hash, &hn, &BK_VERIFIER_INPUT, key_bytes), &iv, &padded(&verifier))?;
    let enc_verifier_hash = cbc_encrypt(&derive_key(hash, &hn, &BK_VERIFIER_VALUE, key_bytes), &iv, &padded(&hash.hash(&[&verifier])))?;
    let enc_key_value = cbc_encrypt(&derive_key(hash, &hn, &BK_KEY_VALUE, key_bytes), &iv, &padded(&key))?;

    // Data integrity (§2.3.4.14).
    let enc_hmac_key = cbc_encrypt(&key, &make_iv(hash, &key_data_salt, Some(&BK_HMAC_KEY), BLOCK), &padded(&hmac_salt))?;
    let hmac = hash.hmac(&hmac_salt, &stream)?;
    let enc_hmac_value = cbc_encrypt(&key, &make_iv(hash, &key_data_salt, Some(&BK_HMAC_VALUE), BLOCK), &padded(&hmac))?;

    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    let params = format!(
        r#"saltSize="{SALT}" blockSize="{BLOCK}" keyBits="{}" hashSize="{}" cipherAlgorithm="AES" cipherChaining="ChainingModeCBC" hashAlgorithm="{}""#,
        key_bytes * 8,
        hash.size(),
        hash.name(),
    );
    let xml = format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\" standalone=\"yes\"?>\r\n\
         <encryption xmlns=\"{NS_ENCRYPTION}\" xmlns:p=\"{NS_PASSWORD}\">\
         <keyData {params} saltValue=\"{}\"/>\
         <dataIntegrity encryptedHmacKey=\"{}\" encryptedHmacValue=\"{}\"/>\
         <keyEncryptors><keyEncryptor uri=\"{NS_PASSWORD}\">\
         <p:encryptedKey spinCount=\"{spin_count}\" {params} saltValue=\"{}\" encryptedVerifierHashInput=\"{}\" encryptedVerifierHashValue=\"{}\" encryptedKeyValue=\"{}\"/>\
         </keyEncryptor></keyEncryptors></encryption>",
        b64(&key_data_salt),
        b64(&enc_hmac_key),
        b64(&enc_hmac_value),
        b64(&password_salt),
        b64(&enc_verifier_input),
        b64(&enc_verifier_hash),
        b64(&enc_key_value),
    );
    // EncryptionInfo (§2.3.4.10): version 4.4, reserved 0x40, the XML.
    let mut info = vec![4, 0, 4, 0, 0x40, 0, 0, 0];
    info.extend_from_slice(xml.as_bytes());

    compound(&[
        ("\u{6}DataSpaces/Version", data_space_version()),
        ("\u{6}DataSpaces/DataSpaceMap", data_space_map()),
        ("\u{6}DataSpaces/DataSpaceInfo/StrongEncryptionDataSpace", data_space_definition()),
        ("\u{6}DataSpaces/TransformInfo/StrongEncryptionTransform/\u{6}Primary", primary_transform()),
        ("EncryptionInfo", info),
        ("EncryptedPackage", stream),
    ])
}

/// A version 3 compound file of streams at `/`-separated paths.
fn compound(streams: &[(&str, Vec<u8>)]) -> Result<Vec<u8>, Error> {
    let refs: Vec<(&str, &[u8])> = streams.iter().map(|(path, data)| (*path, data.as_slice())).collect();
    craft_cfb::write_streams(craft_cfb::Version::V3, &refs).map_err(|e| bad(format!("couldn't write the compound file ({e})")))
}

/// A package encrypted with the *Standard* method (§2.3.4.5–§2.3.4.9: AES-ECB, SHA-1, 50,000
/// spins), as Word 2007 wrote it, for tests and fixtures: files are saved with the agile method
/// ([`encrypt`]). `key_bits` is 128, 192 or 256. The compound file holds just the two streams
/// [`decrypt`] reads.
#[doc(hidden)]
pub fn encrypt_standard(package: &[u8], password: &str, key_bits: u32) -> Result<Vec<u8>, Error> {
    check_password(password)?;
    let alg_id = match key_bits {
        128 => ALG_AES_128,
        192 => ALG_AES_192,
        256 => ALG_AES_256,
        _ => return Err(bad(format!("unsupported AES key size {key_bits}"))),
    };
    let salt = random(SALT)?;
    let verifier = random(SALT)?;
    let key = standard_key(&salt, password, key_bits as usize / 8);
    let mut header = Vec::new();
    for v in [STANDARD_FLAGS, 0, alg_id, ALG_SHA1, key_bits, PROV_RSA_AES, 0, 0] {
        header.extend_from_slice(&v.to_le_bytes());
    }
    // CSPName: an empty null-terminated UTF-16 string.
    header.extend_from_slice(&[0, 0]);
    let mut info = vec![3, 0, 2, 0];
    info.extend_from_slice(&STANDARD_FLAGS.to_le_bytes());
    info.extend_from_slice(&(header.len() as u32).to_le_bytes());
    info.extend_from_slice(&header);
    info.extend_from_slice(&(SALT as u32).to_le_bytes());
    info.extend_from_slice(&salt);
    info.extend_from_slice(&ecb(&key, &verifier, true)?);
    info.extend_from_slice(&(SHA1_SIZE as u32).to_le_bytes());
    info.extend_from_slice(&ecb(&key, &padded(&Hash::Sha1.hash(&[&verifier])), true)?);
    let mut stream = (package.len() as u64).to_le_bytes().to_vec();
    stream.extend_from_slice(&ecb(&key, &padded(package), true)?);
    compound(&[("EncryptionInfo", info), ("EncryptedPackage", stream)])
}

/// Refuse a password Word wouldn't accept: empty, or longer than [`MAX_PASSWORD_CHARS`].
pub fn check_password(password: &str) -> Result<(), Error> {
    if password.is_empty() {
        return Err(Error::BadPassword("the password is empty".into()));
    }
    if password.chars().count() > MAX_PASSWORD_CHARS {
        return Err(Error::BadPassword(format!("passwords are at most {MAX_PASSWORD_CHARS} characters")));
    }
    Ok(())
}

pub(crate) fn bad(msg: impl Into<String>) -> Error {
    Error::Malformed(msg.into())
}

/// A stream's contents, at most `cap` bytes.
fn stream(cf: &craft_cfb::CompoundFile<'_>, id: usize, name: &str, cap: u64) -> Result<Vec<u8>, Error> {
    cf.read_capped(id, cap).map_err(|e| match e {
        craft_cfb::Error::LimitExceeded(_) => Error::TooLarge(format!("{name} is too large")),
        e => bad(format!("can't read {name} ({e})")),
    })
}

/// The hash algorithms agile encryption names that are supported.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Hash {
    Sha1,
    Sha256,
    Sha384,
    Sha512,
}

impl Hash {
    pub(crate) fn parse(s: &str) -> Option<Hash> {
        match s.to_ascii_uppercase().replace('-', "").as_str() {
            "SHA1" => Some(Hash::Sha1),
            "SHA256" => Some(Hash::Sha256),
            "SHA384" => Some(Hash::Sha384),
            "SHA512" => Some(Hash::Sha512),
            _ => None,
        }
    }

    fn name(self) -> &'static str {
        match self {
            Hash::Sha1 => "SHA-1",
            Hash::Sha256 => "SHA256",
            Hash::Sha384 => "SHA384",
            Hash::Sha512 => "SHA512",
        }
    }

    pub(crate) fn size(self) -> usize {
        match self {
            Hash::Sha1 => 20,
            Hash::Sha256 => 32,
            Hash::Sha384 => 48,
            Hash::Sha512 => 64,
        }
    }

    /// The hash of the concatenated `parts`.
    pub(crate) fn hash(self, parts: &[&[u8]]) -> Vec<u8> {
        fn go<D: Digest>(parts: &[&[u8]]) -> Vec<u8> {
            let mut d = D::new();
            for p in parts {
                d.update(p);
            }
            d.finalize().to_vec()
        }
        match self {
            Hash::Sha1 => go::<sha1::Sha1>(parts),
            Hash::Sha256 => go::<sha2::Sha256>(parts),
            Hash::Sha384 => go::<sha2::Sha384>(parts),
            Hash::Sha512 => go::<sha2::Sha512>(parts),
        }
    }

    fn hmac(self, key: &[u8], msg: &[u8]) -> Result<Vec<u8>, Error> {
        fn go<M: Mac + KeyInit>(key: &[u8], msg: &[u8]) -> Result<Vec<u8>, Error> {
            let mut m = <M as KeyInit>::new_from_slice(key).map_err(|_| bad("bad integrity key"))?;
            m.update(msg);
            Ok(m.finalize().into_bytes().to_vec())
        }
        match self {
            Hash::Sha1 => go::<hmac::Hmac<sha1::Sha1>>(key, msg),
            Hash::Sha256 => go::<hmac::Hmac<sha2::Sha256>>(key, msg),
            Hash::Sha384 => go::<hmac::Hmac<sha2::Sha384>>(key, msg),
            Hash::Sha512 => go::<hmac::Hmac<sha2::Sha512>>(key, msg),
        }
    }
}

/// The password hash after `spin_count` iterations (§2.3.4.11): H0 = H(salt + password as
/// UTF-16LE), Hn = H(iterator + Hn-1).
fn iterate(hash: Hash, salt: &[u8], password: &str, spin_count: u32) -> Zeroizing<Vec<u8>> {
    let pw: Zeroizing<Vec<u8>> = Zeroizing::new(password.encode_utf16().flat_map(u16::to_le_bytes).collect());
    let mut h = Zeroizing::new(hash.hash(&[salt, &pw]));
    for i in 0..spin_count.min(MAX_SPIN_COUNT) {
        *h = hash.hash(&[&i.to_le_bytes(), &h]);
    }
    h
}

/// A key from the iterated password hash and a block key (§2.3.4.11): H(Hn + blockKey),
/// truncated to `len`, or padded with 0x36.
fn derive_key(hash: Hash, hn: &[u8], block_key: &[u8], len: usize) -> Zeroizing<Vec<u8>> {
    let mut k = Zeroizing::new(hash.hash(&[hn, block_key]));
    k.resize(len, 0x36);
    k
}

/// Standard encryption's key (§2.3.4.7): the SHA-1 password hash after 50,000 iterations, hashed
/// with block number 0; then X1 = SHA-1 of that hash XOR-ed into 64 bytes of 0x36, X2 the same
/// with 0x5C, and the key is the first `len` bytes of X1 + X2.
fn standard_key(salt: &[u8], password: &str, len: usize) -> Zeroizing<Vec<u8>> {
    let hn = iterate(Hash::Sha1, salt, password, STANDARD_SPIN_COUNT);
    let hfinal = Zeroizing::new(Hash::Sha1.hash(&[&hn, &0u32.to_le_bytes()]));
    let x = |fill: u8| {
        let mut buf = Zeroizing::new([fill; 64]);
        for (b, h) in buf.iter_mut().zip(hfinal.iter()) {
            *b ^= h;
        }
        Zeroizing::new(Hash::Sha1.hash(&[&buf[..]]))
    };
    let mut key = Zeroizing::new(x(0x36).to_vec());
    key.extend_from_slice(&x(0x5C));
    key.truncate(len);
    key
}

/// AES-ECB without padding (Standard encryption, §2.3.4.7–§2.3.4.9): `data` must be whole blocks.
fn ecb(key: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, Error> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::from(*b);
        if encrypt {
            aes.encrypt(&mut x);
        } else {
            aes.decrypt(&mut x);
        }
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// An initialization vector (§2.3.4.12): H(salt + blockKey), or the salt itself, truncated to
/// the block size or padded with 0x36.
fn make_iv(hash: Hash, salt: &[u8], block_key: Option<&[u8]>, block_size: usize) -> Vec<u8> {
    let mut iv = match block_key {
        Some(b) => hash.hash(&[salt, b]),
        None => salt.to_vec(),
    };
    iv.resize(block_size, 0x36);
    iv
}

/// `data` zero-padded to a whole number of blocks.
fn padded(data: &[u8]) -> Zeroizing<Vec<u8>> {
    let mut v = Zeroizing::new(data.to_vec());
    v.resize(data.len().div_ceil(BLOCK).saturating_mul(BLOCK), 0);
    v
}

pub(crate) fn random(n: usize) -> Result<Zeroizing<Vec<u8>>, Error> {
    let mut v = Zeroizing::new(vec![0u8; n]);
    getrandom::fill(&mut v).map_err(|e| bad(format!("no secure random numbers available ({e})")))?;
    Ok(v)
}

/// Compare without stopping at the first difference.
pub(crate) fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// AES with a 128-, 192- or 256-bit key.
enum Aes {
    A128(aes::Aes128),
    A192(aes::Aes192),
    A256(aes::Aes256),
}

impl Aes {
    fn new(key: &[u8]) -> Result<Aes, Error> {
        let err = |_| bad("bad AES key");
        Ok(match key.len() {
            16 => Aes::A128(aes::Aes128::new_from_slice(key).map_err(err)?),
            24 => Aes::A192(aes::Aes192::new_from_slice(key).map_err(err)?),
            32 => Aes::A256(aes::Aes256::new_from_slice(key).map_err(err)?),
            n => return Err(bad(format!("unsupported AES key length {} bits", n * 8))),
        })
    }

    fn encrypt(&self, b: &mut aes::Block) {
        match self {
            Aes::A128(c) => c.encrypt_block(b),
            Aes::A192(c) => c.encrypt_block(b),
            Aes::A256(c) => c.encrypt_block(b),
        }
    }

    fn decrypt(&self, b: &mut aes::Block) {
        match self {
            Aes::A128(c) => c.decrypt_block(b),
            Aes::A192(c) => c.decrypt_block(b),
            Aes::A256(c) => c.decrypt_block(b),
        }
    }
}

/// AES-CBC without padding: `data` must be whole blocks.
fn cbc_encrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    let mut prev: [u8; BLOCK] = iv.try_into().map_err(|_| bad("bad initialization vector"))?;
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::default();
        for ((o, p), v) in x.iter_mut().zip(b).zip(&prev) {
            *o = p ^ v;
        }
        aes.encrypt(&mut x);
        prev.copy_from_slice(&x);
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// AES-CBC decryption without padding.
fn cbc_decrypt(key: &[u8], iv: &[u8], data: &[u8]) -> Result<Vec<u8>, Error> {
    let aes = Aes::new(key)?;
    let (blocks, rest) = data.as_chunks::<BLOCK>();
    if !rest.is_empty() {
        return Err(bad("encrypted data isn't a whole number of blocks"));
    }
    let mut prev: [u8; BLOCK] = iv.try_into().map_err(|_| bad("bad initialization vector"))?;
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        let mut x = aes::Block::default();
        x.copy_from_slice(b);
        aes.decrypt(&mut x);
        for (o, v) in x.iter_mut().zip(&prev) {
            *o ^= v;
        }
        prev = *b;
        out.extend_from_slice(&x);
    }
    Ok(out)
}

/// What `EncryptionInfo` says.
struct Info {
    hash: Hash,
    salt: Vec<u8>,
    block_size: usize,
    key_bytes: usize,
    /// encryptedHmacKey, encryptedHmacValue.
    integrity: (Vec<u8>, Vec<u8>),
    password: PasswordEncryptor,
}

struct PasswordEncryptor {
    hash: Hash,
    salt: Vec<u8>,
    salt_size: usize,
    block_size: usize,
    key_bytes: usize,
    spin_count: u32,
    verifier_input: Vec<u8>,
    verifier_hash: Vec<u8>,
    key_value: Vec<u8>,
}

/// Cipher parameters shared by `keyData` and `encryptedKey`.
struct Params {
    hash: Hash,
    salt: Vec<u8>,
    salt_size: usize,
    block_size: usize,
    key_bytes: usize,
}

fn parse_info(info: &[u8]) -> Result<Info, Error> {
    let (major, minor) = match info.get(..4) {
        Some([a, b, c, d]) => (u16::from_le_bytes([*a, *b]), u16::from_le_bytes([*c, *d])),
        _ => return Err(bad("EncryptionInfo is truncated")),
    };
    match (major, minor) {
        (4, 4) => {}
        (3 | 4, 3) => return Err(Error::Unsupported("Extensible".into())),
        _ => return Err(Error::Unsupported(format!("version {major}.{minor}"))),
    }
    let options = craft_xml::Options {
        limits: craft_xml::Limits { max_input: MAX_INFO as usize, ..craft_xml::Limits::default() },
        ..craft_xml::Options::default()
    };
    let root = options.parse(info.get(8..).unwrap_or(&[])).map_err(|e| bad(format!("EncryptionInfo isn't readable XML ({e})")))?;
    if root.local() != "encryption" {
        return Err(bad("EncryptionInfo has no <encryption> element"));
    }
    let key_data = root.elements().find(|e| e.local() == "keyData").ok_or_else(|| bad("EncryptionInfo has no keyData"))?;
    let kd = params(key_data)?;
    // The integrity check is mandatory (§2.3.4.14): a file is not trusted to opt out of it.
    let di = root
        .elements()
        .find(|e| e.local() == "dataIntegrity")
        .ok_or_else(|| bad("EncryptionInfo has no dataIntegrity, so the document cannot be checked for tampering"))?;
    let integrity = (b64_attr(di, "encryptedHmacKey")?, b64_attr(di, "encryptedHmacValue")?);
    // The password key encryptor: the one with the password URI, whose `encryptedKey` has a
    // spin count (a certificate key encryptor has neither).
    let encryptors = root.elements().find(|e| e.local() == "keyEncryptors").ok_or_else(|| bad("EncryptionInfo has no keyEncryptors"))?;
    let ek = encryptors
        .elements()
        .filter(|e| e.local() == "keyEncryptor" && e.attr("uri").is_none_or(|uri| uri == NS_PASSWORD))
        .flat_map(|e| e.elements())
        .find(|e| e.local() == "encryptedKey" && e.attr("spinCount").is_some())
        .ok_or_else(|| Error::Unsupported("certificate".into()))?;
    let pe = params(ek)?;
    let spin_count: u32 = ek.attr("spinCount").and_then(|s| s.trim().parse().ok()).ok_or_else(|| bad("bad spinCount"))?;
    if spin_count > MAX_SPIN_COUNT {
        return Err(bad(format!("spinCount {spin_count} is over the allowed maximum")));
    }
    Ok(Info {
        hash: kd.hash,
        salt: kd.salt,
        block_size: kd.block_size,
        key_bytes: kd.key_bytes,
        integrity,
        password: PasswordEncryptor {
            hash: pe.hash,
            salt: pe.salt,
            salt_size: pe.salt_size,
            block_size: pe.block_size,
            key_bytes: pe.key_bytes,
            spin_count,
            verifier_input: b64_attr(ek, "encryptedVerifierHashInput")?,
            verifier_hash: b64_attr(ek, "encryptedVerifierHashValue")?,
            key_value: b64_attr(ek, "encryptedKeyValue")?,
        },
    })
}

fn params(e: &El) -> Result<Params, Error> {
    let num = |k: &str| -> Result<usize, Error> { e.attr(k).and_then(|s| s.trim().parse::<usize>().ok()).ok_or_else(|| bad(format!("bad {k}"))) };
    let cipher = e.attr("cipherAlgorithm").unwrap_or("");
    if !cipher.eq_ignore_ascii_case("AES") {
        return Err(Error::Unsupported(cipher.to_string()));
    }
    let chaining = e.attr("cipherChaining").unwrap_or("");
    if chaining != "ChainingModeCBC" {
        return Err(Error::Unsupported(format!("{chaining} chaining")));
    }
    let name = e.attr("hashAlgorithm").unwrap_or("");
    let hash = Hash::parse(name).ok_or_else(|| Error::Unsupported(format!("{name} hash")))?;
    if num("hashSize")? != hash.size() {
        return Err(bad("hashSize doesn't match the hash algorithm"));
    }
    let block_size = num("blockSize")?;
    if block_size != BLOCK {
        return Err(bad(format!("AES needs a block size of {BLOCK}, not {block_size}")));
    }
    let key_bits = num("keyBits")?;
    if !matches!(key_bits, 128 | 192 | 256) {
        return Err(bad(format!("unsupported AES key size {key_bits}")));
    }
    let salt = b64_attr(e, "saltValue")?;
    let salt_size = num("saltSize")?;
    if salt.is_empty() || salt.len() != salt_size {
        return Err(bad("the salt doesn't match saltSize"));
    }
    Ok(Params { hash, salt, salt_size, block_size, key_bytes: key_bits / 8 })
}

fn b64_attr(e: &El, k: &str) -> Result<Vec<u8>, Error> {
    let s = e.attr(k).ok_or_else(|| bad(format!("{k} is missing")))?;
    let s: String = s.chars().filter(|c| !c.is_ascii_whitespace()).collect();
    base64::engine::general_purpose::STANDARD.decode(s).map_err(|_| bad(format!("{k} isn't valid base64")))
}

/// A UNICODE-LP-P4 string (§2.1.2): byte length, UTF-16LE, zero-padded to 4 bytes.
fn lp_p4(out: &mut Vec<u8>, s: &str) {
    let units: Vec<u8> = s.encode_utf16().flat_map(u16::to_le_bytes).collect();
    out.extend_from_slice(&(units.len() as u32).to_le_bytes());
    out.extend_from_slice(&units);
    while !out.len().is_multiple_of(4) {
        out.push(0);
    }
}

/// `\x06DataSpaces\Version` (§2.1.5).
fn data_space_version() -> Vec<u8> {
    let mut v = Vec::new();
    lp_p4(&mut v, "Microsoft.Container.DataSpaces");
    for _ in 0..3 {
        v.extend_from_slice(&[1, 0, 0, 0]);
    }
    v
}

/// `\x06DataSpaces\DataSpaceMap` (§2.1.6, §2.3.4.1): EncryptedPackage → StrongEncryptionDataSpace.
fn data_space_map() -> Vec<u8> {
    let mut entry = Vec::new();
    entry.extend_from_slice(&1u32.to_le_bytes());
    entry.extend_from_slice(&0u32.to_le_bytes());
    lp_p4(&mut entry, "EncryptedPackage");
    lp_p4(&mut entry, "StrongEncryptionDataSpace");
    let mut v = Vec::new();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&((entry.len() + 4) as u32).to_le_bytes());
    v.extend_from_slice(&entry);
    v
}

/// `\x06DataSpaces\DataSpaceInfo\StrongEncryptionDataSpace` (§2.1.7, §2.3.4.2).
fn data_space_definition() -> Vec<u8> {
    let mut v = Vec::new();
    v.extend_from_slice(&8u32.to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    lp_p4(&mut v, "StrongEncryptionTransform");
    v
}

/// `\x06DataSpaces\TransformInfo\StrongEncryptionTransform\x06Primary` (§2.1.8, §2.1.9,
/// §2.3.4.3): the encryption transform, with a null EncryptionName as agile encryption requires.
fn primary_transform() -> Vec<u8> {
    let mut id = Vec::new();
    lp_p4(&mut id, "{FF9A3F03-56EF-4613-BDD5-5A41C1D07246}");
    let mut v = Vec::new();
    // TransformLength: the bytes before TransformName.
    v.extend_from_slice(&((8 + id.len()) as u32).to_le_bytes());
    v.extend_from_slice(&1u32.to_le_bytes());
    v.extend_from_slice(&id);
    lp_p4(&mut v, "Microsoft.Container.EncryptionTransform");
    for _ in 0..3 {
        v.extend_from_slice(&[1, 0, 0, 0]);
    }
    // EncryptionTransformInfo: null name, block size, cipher mode 0, reserved 4.
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&(BLOCK as u32).to_le_bytes());
    v.extend_from_slice(&0u32.to_le_bytes());
    v.extend_from_slice(&4u32.to_le_bytes());
    v
}

#[cfg(test)]
mod tests;
