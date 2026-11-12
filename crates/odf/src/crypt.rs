//! Password-protected packages (ODF 1.3 Part 2 §3.4, §4.4–4.16): each encrypted file is
//! deflated, then encrypted with a key derived from the password, and described in the
//! manifest (`manifest:encryption-data`). Read: the package decrypted into a plain one, which
//! the reader opens as any other. Written as LibreOffice writes it: AES-256-CBC, a SHA-256
//! start key, PBKDF2 with HMAC-SHA-1 and a salt of its own for each file, a SHA-256 checksum of
//! the first 1024 bytes of each deflated file.
//!
//! Blowfish (the old default, §3.4.2) isn't supported: such files say so.
//!
//! [`decrypt`] gives a plain package, which a reader opens as any other; [`encrypt`] takes a
//! plain package. Neither looks at what the files hold.

use std::io::{Read, Write};

use aes::cipher::{BlockCipherDecrypt, BlockCipherEncrypt, KeyInit};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use hmac::Mac;
use sha2::Digest;
use zeroize::Zeroizing;

use crate::manifest::{Encryption as Params, MANIFEST_PATH, Manifest};
use crate::{Limits, zip_guard};

/// Most bytes one file of a package decrypts or inflates to.
const MAX_ENTRY: u64 = Limits::DEFAULT.max_entry;
/// Most entries a package may declare.
const MAX_ZIP_ENTRIES: u64 = Limits::DEFAULT.max_entries;

/// Why an encrypted package can't be opened, or a package can't be encrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CryptError {
    /// The password doesn't decrypt it (the checksums don't match).
    WrongPassword,
    /// An encryption that isn't supported, a broken package, or (saving) no random numbers.
    Unsupported(String),
}

impl std::error::Error for CryptError {}

impl std::fmt::Display for CryptError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CryptError::WrongPassword => write!(f, "the password is incorrect"),
            CryptError::Unsupported(why) => write!(f, "{why}"),
        }
    }
}

const AES256_CBC: &str = "http://www.w3.org/2001/04/xmlenc#aes256-cbc";
const SHA256: &str = "http://www.w3.org/2000/09/xmldsig#sha256";
const SHA256_1K: &str = "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0#sha256-1k";
/// PBKDF2 iterations written (LibreOffice's), and most read (a hostile count would hang).
const ITERATIONS: u32 = 100_000;
const MAX_ITERATIONS: u32 = 10_000_000;
/// Most files in a package decrypted.
const MAX_FILES: usize = 10_000;
/// Most PBKDF2 work (iterations times the key's 20-byte blocks) for a whole package: a thousand
/// files as LibreOffice writes them (100,000 iterations, a 32-byte key), so many
/// files at the per-file cap can't add up to a hang.
const MAX_WORK: u64 = 200_000_000;

/// `p`'s key derivation added to `spent`, an error once the package's work passes [`MAX_WORK`].
fn charge(spent: &mut u64, p: &Params) -> Result<(), CryptError> {
    let blocks = p.key_size.unwrap_or(16).clamp(16, 32).div_ceil(20) as u64;
    *spent = spent.saturating_add(u64::from(p.iterations.min(MAX_ITERATIONS)).saturating_mul(blocks));
    if *spent > MAX_WORK {
        return Err(CryptError::Unsupported("the package's key derivations would take too long".into()));
    }
    Ok(())
}

/// Each encrypted file's parameters and declared size, by path, from a manifest.
fn read_manifest(xml: &[u8]) -> Vec<(String, Params, Option<usize>)> {
    Manifest::parse(xml)
        .entries
        .into_iter()
        .filter_map(|e| Some((e.path, e.encryption?, e.size.and_then(|s| usize::try_from(s).ok()))))
        .take(MAX_FILES)
        .collect()
}

/// The package, its declared entry count checked before it is indexed (with
/// [`zip_guard`]'s wording).
fn open_zip(bytes: &[u8]) -> Result<craft_zip::Archive<'_>, CryptError> {
    zip_guard::check(bytes, MAX_ZIP_ENTRIES).map_err(CryptError::Unsupported)?;
    craft_zip::Archive::open(bytes, craft_zip::Limits { max_entries: MAX_ZIP_ENTRIES, max_total_bytes: u64::MAX })
        .map_err(|e| CryptError::Unsupported(format!("not a valid package ({e})")))
}

/// `name` read from `z`, at most [`MAX_ENTRY`] bytes of it. `None` when it is missing, damaged
/// or larger.
fn read_entry(z: &mut craft_zip::Archive<'_>, name: &str) -> Option<Vec<u8>> {
    z.read(name, MAX_ENTRY).ok().flatten()
}

/// The files of a package in its order, folders left out, each at most [`MAX_ENTRY`] bytes.
fn files_of(z: &craft_zip::Archive<'_>) -> Vec<String> {
    z.names().filter(|n| !n.ends_with('/')).map(|n| n.into_owned()).collect()
}

/// Does this package hold encrypted files?
pub fn is_encrypted(bytes: &[u8]) -> bool {
    // A package with too many entries isn't parsed here: opening it refuses it (`zip_guard`).
    let Ok(mut z) = open_zip(bytes) else { return false };
    read_entry(&mut z, MANIFEST_PATH).is_some_and(|m| !read_manifest(&m).is_empty())
}

/// The start key: the password's UTF-8 digest (§4.16.6), `size` bytes of it.
fn start_key(name: &str, size: Option<usize>, password: &str) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let mut k = match name {
        SHA256 | "SHA256" => sha2::Sha256::digest(password.as_bytes()).to_vec(),
        "SHA1" | "http://www.w3.org/2000/09/xmldsig#sha1" => sha1::Sha1::digest(password.as_bytes()).to_vec(),
        other => return Err(CryptError::Unsupported(format!("a start key made with {other} isn't supported"))),
    };
    if let Some(s) = size.filter(|s| *s > 0 && *s < k.len()) {
        k.truncate(s);
    }
    Ok(Zeroizing::new(k))
}

/// PBKDF2 with HMAC-SHA-1 (RFC 2898 §5.2): `len` bytes.
fn pbkdf2(password: &[u8], salt: &[u8], iterations: u32, len: usize) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let mac = hmac::Hmac::<sha1::Sha1>::new_from_slice(password).map_err(|_| CryptError::Unsupported("bad key".into()))?;
    let mut out = Zeroizing::new(Vec::with_capacity(len + 20));
    let mut block = 1u32;
    while out.len() < len {
        let mut m = mac.clone();
        m.update(salt);
        m.update(&block.to_be_bytes());
        let mut u = m.finalize().into_bytes();
        let mut t = u;
        for _ in 1..iterations {
            let mut m = mac.clone();
            m.update(&u);
            u = m.finalize().into_bytes();
            t.iter_mut().zip(u.iter()).for_each(|(a, b)| *a ^= b);
        }
        out.extend_from_slice(&t);
        block = block.saturating_add(1);
    }
    out.truncate(len);
    Ok(out)
}

/// The key for a file with parameters `p`.
fn file_key(p: &Params, password: &str) -> Result<Zeroizing<Vec<u8>>, CryptError> {
    let start = start_key(&p.start_key, p.start_key_size, password)?;
    if !(p.derivation == "PBKDF2" || p.derivation.ends_with("#pbkdf2")) {
        return Err(CryptError::Unsupported(format!("key derivation {:?} isn't supported", p.derivation)));
    }
    if p.iterations == 0 || p.iterations > MAX_ITERATIONS {
        return Err(CryptError::Unsupported("the key derivation's iteration count is out of range".into()));
    }
    pbkdf2(&start, &p.salt, p.iterations, p.key_size.unwrap_or(16).clamp(16, 32))
}

/// AES-256-CBC, `data` whole blocks.
fn cbc(key: &[u8], iv: &[u8], data: &[u8], encrypt: bool) -> Result<Vec<u8>, CryptError> {
    let aes = aes::Aes256::new_from_slice(key).map_err(|_| CryptError::Unsupported("the key isn't 256 bits".into()))?;
    let mut prev: [u8; 16] = iv.try_into().map_err(|_| CryptError::Unsupported("the initialisation vector isn't 16 bytes".into()))?;
    let (blocks, rest) = data.as_chunks::<16>();
    if !rest.is_empty() {
        return Err(CryptError::Unsupported("encrypted data isn't a whole number of blocks".into()));
    }
    let mut out = Vec::with_capacity(data.len());
    for b in blocks {
        if encrypt {
            let mut x = aes::Block::from(*b);
            x.iter_mut().zip(prev.iter()).for_each(|(a, p)| *a ^= p);
            aes.encrypt_block(&mut x);
            prev = x.into();
            out.extend_from_slice(&prev);
        } else {
            let mut x = aes::Block::from(*b);
            aes.decrypt_block(&mut x);
            x.iter_mut().zip(prev.iter()).for_each(|(a, p)| *a ^= p);
            out.extend_from_slice(&x);
            prev = *b;
        }
    }
    Ok(out)
}

/// The checksum of a deflated file (§4.16.3): of its first 1024 bytes.
fn checksum(kind: &str, deflated: &[u8]) -> Result<Vec<u8>, CryptError> {
    let head = deflated.get(..deflated.len().min(1024)).unwrap_or_default();
    match kind {
        SHA256_1K => Ok(sha2::Sha256::digest(head).to_vec()),
        "SHA1/1K" | "urn:oasis:names:tc:opendocument:xmlns:manifest:1.0#sha1-1k" => Ok(sha1::Sha1::digest(head).to_vec()),
        other => Err(CryptError::Unsupported(format!("checksum {other} isn't supported"))),
    }
}

/// One encrypted file decrypted and inflated.
fn decrypt_file(data: &[u8], p: &Params, size: Option<usize>, password: &str) -> Result<Vec<u8>, CryptError> {
    if p.algorithm != AES256_CBC {
        let what = if p.algorithm.to_ascii_lowercase().contains("blowfish") { "Blowfish".to_string() } else { format!("{:?}", p.algorithm) };
        return Err(CryptError::Unsupported(format!("this file is encrypted with {what}, which isn't supported")));
    }
    let key = file_key(p, password)?;
    let plain = Zeroizing::new(cbc(&key, &p.iv, data, false)?);
    // W3C XML Encryption padding: the last byte says how many bytes were added.
    let pad = usize::from(plain.last().copied().unwrap_or(0));
    let deflated = (1..=16).contains(&pad).then(|| plain.get(..plain.len().saturating_sub(pad))).flatten().ok_or(CryptError::WrongPassword)?;
    if !p.checksum.is_empty() && checksum(&p.checksum_type, deflated)? != p.checksum {
        return Err(CryptError::WrongPassword);
    }
    let limit = size.unwrap_or(MAX_ENTRY as usize).min(MAX_ENTRY as usize);
    inflate(deflated, limit).ok_or(CryptError::WrongPassword)
}

/// Raw deflate data inflated, `None` when it is damaged or longer than `limit`.
fn inflate(deflated: &[u8], limit: usize) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    flate2::read::DeflateDecoder::new(deflated).take(limit as u64 + 1).read_to_end(&mut out).ok()?;
    (out.len() <= limit).then_some(out)
}

fn deflate(data: &[u8]) -> Result<Vec<u8>, std::io::Error> {
    let mut e = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::new(6));
    e.write_all(data)?;
    e.finish()
}

/// The manifest without its encryption data (each entry keeps its `manifest:size`).
fn plain_manifest(xml: &str) -> String {
    let mut out = String::with_capacity(xml.len());
    let mut rest = xml;
    while let Some(i) = rest.find("<manifest:encryption-data") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        let end =
            after.find("</manifest:encryption-data>").map(|e| e + "</manifest:encryption-data>".len()).or_else(|| after.find("/>").map(|e| e + 2));
        rest = end.and_then(|e| after.get(e..)).unwrap_or("");
    }
    out.push_str(rest);
    out
}

/// A password-protected package decrypted into a plain one.
pub fn decrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, CryptError> {
    let broken = |e: craft_zip::Error| CryptError::Unsupported(format!("not a valid package ({e})"));
    let mut z = open_zip(bytes)?;
    let manifest = read_entry(&mut z, MANIFEST_PATH).unwrap_or_default();
    let encrypted = read_manifest(&manifest);
    let mut w = craft_zip::Writer::new();
    let mut checked = false;
    let mut work = 0u64;
    for name in files_of(&z).into_iter().take(MAX_FILES) {
        // A file past the limit is refused, not cut short.
        let Some(data) = z.read(&name, MAX_ENTRY).map_err(broken)? else { continue };
        let data = match encrypted.iter().find(|(p, ..)| *p == name) {
            Some((_, p, size)) => {
                charge(&mut work, p)?;
                let plain = decrypt_file(&data, p, *size, password)?;
                checked = true;
                plain
            }
            None if name == MANIFEST_PATH => plain_manifest(&String::from_utf8_lossy(&data)).into_bytes(),
            None => data,
        };
        let compression = if name == "mimetype" { craft_zip::Compression::Stored } else { craft_zip::Compression::Deflated };
        w.add(&name, &data, compression).map_err(broken)?;
    }
    if !checked {
        return Err(CryptError::Unsupported("no encrypted file could be read".into()));
    }
    w.finish().map_err(broken)
}

/// An attribute value's text (the manifest's own entity escapes).
fn unescape(s: &str) -> String {
    s.replace("&quot;", "\"").replace("&apos;", "'").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&")
}

fn random(n: usize) -> Result<Vec<u8>, CryptError> {
    let mut v = vec![0u8; n];
    getrandom::fill(&mut v).map_err(|e| CryptError::Unsupported(format!("no random numbers for encryption ({e})")))?;
    Ok(v)
}

/// A package encrypted with `password`: every file but the `mimetype` and the manifest, the
/// manifest saying how.
pub fn encrypt(bytes: &[u8], password: &str) -> Result<Vec<u8>, CryptError> {
    let err = |e: craft_zip::Error| CryptError::Unsupported(e.to_string());
    let io = |e: std::io::Error| CryptError::Unsupported(e.to_string());
    let mut z = open_zip(bytes)?;
    let start = start_key(SHA256, Some(32), password)?;
    let mut files: Vec<(String, Vec<u8>, bool)> = Vec::new();
    let mut manifest = String::new();
    // name → its encryption data and size, for the manifest.
    let mut data_of: Vec<(String, String)> = Vec::new();
    for name in files_of(&z) {
        // At most `MAX_ENTRY` a file: it was read without a limit.
        let Some(data) = z.read(&name, MAX_ENTRY).map_err(err)? else { continue };
        if name == MANIFEST_PATH {
            manifest = String::from_utf8_lossy(&data).into_owned();
            continue;
        }
        if name == "mimetype" {
            files.push((name, data, false));
            continue;
        }
        let size = data.len();
        let mut deflated = deflate(&data).map_err(io)?;
        let sum = checksum(SHA256_1K, &deflated)?;
        // W3C XML Encryption padding: random bytes, the last one their count.
        let pad = 16 - deflated.len() % 16;
        let mut tail = random(pad)?;
        if let Some(last) = tail.last_mut() {
            *last = pad as u8;
        }
        deflated.extend_from_slice(&tail);
        let (salt, iv) = (random(16)?, random(16)?);
        let key = pbkdf2(&start, &salt, ITERATIONS, 32)?;
        let enc = cbc(&key, &iv, &deflated, true)?;
        data_of.push((
            name.clone(),
            format!(
                " manifest:size=\"{size}\"><manifest:encryption-data manifest:checksum-type=\"{SHA256_1K}\" manifest:checksum=\"{}\"><manifest:algorithm manifest:algorithm-name=\"{AES256_CBC}\" manifest:initialisation-vector=\"{}\"/><manifest:start-key-generation manifest:start-key-generation-name=\"{SHA256}\" manifest:key-size=\"32\"/><manifest:key-derivation manifest:key-derivation-name=\"PBKDF2\" manifest:iteration-count=\"{ITERATIONS}\" manifest:salt=\"{}\" manifest:key-size=\"32\"/></manifest:encryption-data></manifest:file-entry>",
                STANDARD.encode(&sum),
                STANDARD.encode(&iv),
                STANDARD.encode(&salt)
            ),
        ));
        files.push((name, enc, true));
    }
    // Each encrypted file's entry gets its size and encryption data; a file the manifest
    // doesn't list gets an entry of its own.
    let mut listed = std::collections::HashSet::new();
    let mut out = String::with_capacity(manifest.len() + data_of.len() * 600);
    let mut rest = manifest.as_str();
    while let Some(i) = rest.find("<manifest:file-entry ") {
        out.push_str(rest.get(..i).unwrap_or(""));
        let after = rest.get(i..).unwrap_or("");
        let Some(end) = after.find("/>") else { break };
        let tag = after.get(..end).unwrap_or("");
        let path = tag.split("manifest:full-path=\"").nth(1).and_then(|r| r.split('"').next()).map(unescape).unwrap_or_default();
        match data_of.iter().find(|(n, _)| *n == path) {
            Some((n, enc)) => {
                listed.insert(n.clone());
                out.push_str(tag);
                out.push_str(enc);
            }
            None => out.push_str(after.get(..end + 2).unwrap_or("")),
        }
        rest = after.get(end + 2..).unwrap_or("");
    }
    let unlisted: String = data_of
        .iter()
        .filter(|(n, _)| !listed.contains(n))
        .map(|(n, enc)| format!("<manifest:file-entry manifest:full-path=\"{}\" manifest:media-type=\"\"{enc}\n", craft_xml::esc_attr(n)))
        .collect();
    match rest.rfind("</manifest:manifest>") {
        Some(e) => {
            out.push_str(rest.get(..e).unwrap_or(""));
            out.push_str(&unlisted);
            out.push_str(rest.get(e..).unwrap_or(""));
        }
        None => out.push_str(rest),
    }
    use craft_zip::Compression::{Deflated, Stored};
    let mut w = craft_zip::Writer::new();
    for (name, data, _) in files.iter().filter(|(n, _, _)| n == "mimetype") {
        w.add(name, data, Stored).map_err(err)?;
    }
    w.add(MANIFEST_PATH, out.as_bytes(), Deflated).map_err(err)?;
    // Encrypted files are stored: they're deflated already (§3.4.1).
    for (name, data, _) in files.iter().filter(|(n, _, _)| n != "mimetype") {
        w.add(name, data, Stored).map_err(err)?;
    }
    w.finish().map_err(err)
}

#[cfg(test)]
mod work_tests {
    use super::*;

    /// Key derivations are budgeted over the whole package, not only per file: a thousand of
    /// LibreOffice's fit, twenty-one at the per-file cap don't.
    #[test]
    fn a_packages_key_derivations_share_one_budget() {
        let ours = Params { iterations: ITERATIONS, key_size: Some(32), ..Default::default() };
        let mut spent = 0;
        for _ in 0..1000 {
            assert!(charge(&mut spent, &ours).is_ok());
        }
        assert!(charge(&mut spent, &ours).is_err());
        let hostile = Params { iterations: MAX_ITERATIONS, key_size: Some(16), ..Default::default() };
        let mut spent = 0;
        let ok = (0..30).take_while(|_| charge(&mut spent, &hostile).is_ok()).count();
        assert_eq!(ok, 20);
    }
}
