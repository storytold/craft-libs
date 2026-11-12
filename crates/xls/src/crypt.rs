//! Password-encrypted workbooks ([MS-XLS] 2.2.10, [MS-OFFCRYPTO] 2.3.5, 2.3.6). A
//! FilePass record in the workbook globals holds a salt and a password verifier; the record data
//! after it is encrypted with RC4, rekeyed every 1024 bytes of the stream. Record headers, and the
//! data of a few records, are left in the clear.
//!
//! RC4 (version 1.1), keyed from the password through MD5, is read. XOR obfuscation and RC4
//! CryptoAPI are reported as unsupported. Clean-room from Microsoft's public
//! specifications.

use md5::{Digest, Md5};

/// Why a Workbook stream could not be decrypted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecryptError {
    WrongPassword,
    /// A scheme that is not read, named to complete "uses {0} encryption".
    Unsupported(String),
    Malformed(String),
}

/// The password Excel encrypts with when a workbook has none of its own (a workbook protected
/// against changes, not against opening). Excel tries it before asking.
pub(crate) const DEFAULT_PASSWORD: &str = "VelvetSweatshop";

/// FilePass: the globals record that marks an encrypted workbook.
pub(crate) const FILEPASS: u16 = 0x002F;
const EOF: u16 = 0x000A;
const BOUNDSHEET: u16 = 0x0085;
/// Records whose data is never encrypted: BOF, FilePass, UsrExcl, FileLock, InterfaceHdr,
/// RRDInfo and RRDHead.
const CLEAR: [u16; 7] = [0x0809, FILEPASS, 0x0194, 0x0195, 0x00E1, 0x0196, 0x0138];
/// The RC4 key changes every this many bytes of the stream.
const REKEY: usize = 1024;
fn malformed(m: &str) -> DecryptError {
    DecryptError::Malformed(m.into())
}

/// The record header at `pos`: its type and data length.
fn header(stream: &[u8], pos: usize) -> Option<(u16, usize)> {
    let h = stream.get(pos..pos.checked_add(4)?)?;
    let id = u16::from_le_bytes([*h.first()?, *h.get(1)?]);
    Some((id, usize::from(u16::from_le_bytes([*h.get(2)?, *h.get(3)?]))))
}

/// The FilePass record's data, when the workbook globals hold one.
pub(crate) fn filepass(stream: &[u8]) -> Option<&[u8]> {
    let mut pos = 0usize;
    while let Some((id, len)) = header(stream, pos) {
        let start = pos + 4;
        match id {
            EOF => return None,
            FILEPASS => return stream.get(start..start.saturating_add(len)),
            _ => pos = start + len,
        }
    }
    None
}

/// Decrypts a Workbook stream in place with `password`. The FilePass record stays where it is
/// (removing it would move the substreams the BoundSheet records point at).
pub(crate) fn decrypt(stream: &mut [u8], password: &str) -> Result<(), DecryptError> {
    let key = password_key(filepass(stream).ok_or_else(|| malformed("no FilePass record"))?, password)?;
    let mut keystream = Keystream::new(key);
    let mut pos = 0usize;
    while let Some((id, len)) = header(stream, pos) {
        let start = pos + 4;
        let end = start.saturating_add(len).min(stream.len());
        // BoundSheet8's stream position (its first 4 bytes) stays in the clear.
        let clear = if CLEAR.contains(&id) {
            len
        } else if id == BOUNDSHEET {
            4
        } else {
            0
        };
        let from = start.saturating_add(clear).min(end);
        if let Some(data) = stream.get_mut(from..end) {
            keystream.apply(from, data);
        }
        pos = start + len;
    }
    Ok(())
}

/// What the block keys are made from.
enum Key {
    /// RC4 1.1: the 5 bytes every block key is the MD5 of, with the block number.
    Rc4([u8; 5]),
}

impl Key {
    /// The cipher for 1024-byte block `block`, at its start.
    fn block(&self, block: usize) -> Rc4 {
        let block = u32::try_from(block).unwrap_or(u32::MAX).to_le_bytes();
        match self {
            // MD5(key ‖ LE32(block)) ([MS-OFFCRYPTO] 2.3.6.2).
            Key::Rc4(key) => Rc4::new(&md5(&[key, &block])),
        }
    }
}

/// The key every block key is derived from, after checking `password` against the FilePass
/// verifier.
fn password_key(filepass: &[u8], password: &str) -> Result<Key, DecryptError> {
    let short = || malformed("truncated FilePass record");
    match u16_at(filepass, 0).ok_or_else(short)? {
        0 => return Err(DecryptError::Unsupported("XOR obfuscation".into())),
        1 => {}
        other => return Err(DecryptError::Malformed(format!("unknown encryption type {other}"))),
    }
    match (u16_at(filepass, 2).ok_or_else(short)?, u16_at(filepass, 4).ok_or_else(short)?) {
        (1, 1) => rc4_key(filepass, password),
        (2..=4, 2) => Err(DecryptError::Unsupported("RC4 CryptoAPI".into())),
        (major, minor) => Err(DecryptError::Unsupported(format!("version {major}.{minor}"))),
    }
}

fn u16_at(b: &[u8], at: usize) -> Option<u16> {
    let v = b.get(at..at.checked_add(2)?)?;
    Some(u16::from_le_bytes([*v.first()?, *v.get(1)?]))
}

/// RC4 1.1 ([MS-OFFCRYPTO] 2.3.6.1, 2.3.6.2): the key, after checking the verifier.
fn rc4_key(filepass: &[u8], password: &str) -> Result<Key, DecryptError> {
    let short = || malformed("truncated FilePass record");
    let salt = filepass.get(6..22).ok_or_else(short)?;
    let verifier = filepass.get(22..38).ok_or_else(short)?;
    let verifier_hash = filepass.get(38..54).ok_or_else(short)?;

    // H0 = MD5(UTF-16LE password); H1 = MD5(16 × (first 5 bytes of H0 ‖ salt)); the key is the
    // first 5 bytes of H1.
    let pw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let h0 = md5(&[&pw]);
    let truncated = h0.get(..5).unwrap_or_default();
    let mut m = Md5::new();
    for _ in 0..16 {
        m.update(truncated);
        m.update(salt);
    }
    let mut key = [0u8; 5];
    key.iter_mut().zip(m.finalize().iter()).for_each(|(k, h)| *k = *h);
    let key = Key::Rc4(key);

    // The verifier and its hash are encrypted one after the other with the block-0 key.
    let mut rc4 = key.block(0);
    let verifier: Vec<u8> = verifier.iter().map(|b| b ^ rc4.next_byte()).collect();
    let verifier_hash: Vec<u8> = verifier_hash.iter().map(|b| b ^ rc4.next_byte()).collect();
    if md5(&[&verifier]).as_slice() != verifier_hash.as_slice() {
        return Err(DecryptError::WrongPassword);
    }
    Ok(key)
}

fn md5(parts: &[&[u8]]) -> [u8; 16] {
    let mut m = Md5::new();
    for p in parts {
        m.update(p);
    }
    let mut out = [0u8; 16];
    out.iter_mut().zip(m.finalize().iter()).for_each(|(o, h)| *o = *h);
    out
}

/// The keystream by stream offset: byte `n` comes from block `n / 1024`'s key, at position
/// `n % 1024`, whether or not the bytes before it were encrypted.
struct Keystream {
    key: Key,
    block: usize,
    rc4: Rc4,
    /// The stream offset of the next byte `rc4` gives.
    at: usize,
}

impl Keystream {
    fn new(key: Key) -> Keystream {
        Keystream { rc4: key.block(0), key, block: 0, at: 0 }
    }

    /// XORs `data`, found at stream offset `offset`, with the keystream there. Offsets only
    /// move forward between calls in the normal case; going back rekeys.
    fn apply(&mut self, offset: usize, data: &mut [u8]) {
        for (n, b) in data.iter_mut().enumerate() {
            *b ^= self.byte_at(offset.saturating_add(n));
        }
    }

    fn byte_at(&mut self, offset: usize) -> u8 {
        let block = offset / REKEY;
        if block != self.block || offset < self.at {
            self.block = block;
            self.rc4 = self.key.block(block);
            self.at = block * REKEY;
        }
        while self.at < offset {
            self.rc4.next_byte();
            self.at += 1;
        }
        self.at += 1;
        self.rc4.next_byte()
    }
}

/// The RC4 stream cipher.
struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    /// RC4 keyed with `key` (5 to 16 bytes here; an empty key acts as one zero byte).
    fn new(key: &[u8]) -> Rc4 {
        let mut s = [0u8; 256];
        for (n, x) in (0u8..=255).zip(s.iter_mut()) {
            *x = n;
        }
        let mut j = 0u8;
        for n in 0..256 {
            let k = key.get(n % key.len().max(1)).copied().unwrap_or(0);
            j = j.wrapping_add(s[n]).wrapping_add(k);
            s.swap(n, usize::from(j));
        }
        Rc4 { s, i: 0, j: 0 }
    }

    fn next_byte(&mut self) -> u8 {
        self.i = self.i.wrapping_add(1);
        self.j = self.j.wrapping_add(self.s[usize::from(self.i)]);
        self.s.swap(usize::from(self.i), usize::from(self.j));
        let t = self.s[usize::from(self.i)].wrapping_add(self.s[usize::from(self.j)]);
        self.s[usize::from(t)]
    }
}

/// Encrypts a Workbook stream the way [`decrypt`] reads it, for tests: the FilePass record
/// (RC4 1.1 with a fixed salt and verifier) goes right after the globals' BOF. `None` when the
/// stream does not start with a record.
pub(crate) fn encrypt(stream: &[u8], password: &str) -> Option<Vec<u8>> {
    let salt: [u8; 16] = std::array::from_fn(|n| n as u8 + 0x40);
    let verifier: [u8; 16] = std::array::from_fn(|n| n as u8 + 0x60);
    let pw: Vec<u8> = password.encode_utf16().flat_map(u16::to_le_bytes).collect();
    let h0 = md5(&[&pw]);
    let mut m = Md5::new();
    for _ in 0..16 {
        m.update(h0.get(..5)?);
        m.update(salt);
    }
    let mut key = [0u8; 5];
    key.iter_mut().zip(m.finalize().iter()).for_each(|(k, h)| *k = *h);
    let mut rc4 = Key::Rc4(key).block(0);
    let mut filepass = vec![1, 0, 1, 0, 1, 0];
    filepass.extend(salt);
    filepass.extend(verifier.iter().map(|b| b ^ rc4.next_byte()));
    filepass.extend(md5(&[&verifier]).iter().map(|b| b ^ rc4.next_byte()));
    with_filepass(stream, &filepass, password)
}

/// `stream` with a FilePass record holding `filepass` right after its first BOF, encrypted with
/// `password`.
fn with_filepass(stream: &[u8], filepass: &[u8], password: &str) -> Option<Vec<u8>> {
    let (_, bof) = header(stream, 0)?;
    let bof_len = 4 + bof;
    let mut out = stream.get(..bof_len)?.to_vec();
    out.extend(FILEPASS.to_le_bytes());
    out.extend(u16::try_from(filepass.len()).ok()?.to_le_bytes());
    out.extend(filepass);
    let shift = u32::try_from(out.len() - bof_len).ok()?;
    let mut rest = stream.get(bof_len..)?.to_vec();
    // BoundSheet8 positions move by the FilePass record's size.
    let mut pos = 0;
    while let Some((id, len)) = header(&rest, pos) {
        if id == BOUNDSHEET {
            let at = rest.get_mut(pos + 4..pos + 8)?;
            let p = u32::from_le_bytes([*at.first()?, *at.get(1)?, *at.get(2)?, *at.get(3)?]).checked_add(shift)?;
            at.copy_from_slice(&p.to_le_bytes());
        }
        pos += 4 + len;
    }
    out.extend(rest);
    // The cipher is its own inverse.
    decrypt(&mut out, password).ok()?;
    Some(out)
}
