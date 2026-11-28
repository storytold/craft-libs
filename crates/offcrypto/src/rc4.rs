//! RC4 encryption of the Office 97-2003 binary formats ([MS-OFFCRYPTO] §2.3.5, §2.3.6).
//!
//! Two schemes share the cipher:
//!
//! * **RC4** (header version 1.1, §2.3.6): the password hash and each block's key come from
//!   MD5, and the key is 128 bits.
//! * **RC4 CryptoAPI** (header version 2.2, 3.2 or 4.2, §2.3.5): the same with SHA-1, a key of
//!   40 to 128 bits and a variable-size header naming the provider.
//!
//! [`Key::open`] reads the encryption header at the start of a stream, checks the password
//! against the stored verifier and returns the key, so a wrong password is reported and never
//! yields garbled data. What is encrypted, and with which block number, is the format's
//! business: [`Key::decrypt_blocks`] is Word's 512-byte blocks, and [`Key::cipher`] gives the
//! cipher for any one block number (Excel's 1024-byte blocks, PowerPoint's persist objects).
//! Keys and the password hash are wiped when dropped.

use sha1::{Digest, Sha1};
use zeroize::{Zeroize, Zeroizing};

use crate::Error;

/// RC4 re-keys every this many bytes of a Word stream.
pub const BLOCK: usize = 512;
/// Longest password considered (the specification's limit for the RC4 schemes).
const MAX_PASSWORD_CHARS: usize = 255;
/// `EncryptionHeaderFlags`: `fCryptoAPI`, `fExternal`, `fAES`.
const H_CRYPTO_API: u32 = 0x04;
const H_EXTERNAL: u32 = 0x10;
const H_AES: u32 = 0x20;

// ---------------------------------------------------------------------------------------------
// Primitives

/// MD5 ([RFC 1321]); the RC4 scheme is defined on it.
pub fn md5(data: &[u8]) -> [u8; 16] {
    const S: [u32; 64] = [
        7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 7, 12, 17, 22, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 5, 9, 14, 20, 4, 11, 16, 23, 4, 11, 16,
        23, 4, 11, 16, 23, 4, 11, 16, 23, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21, 6, 10, 15, 21,
    ];
    // K[i] = floor(2^32 * abs(sin(i + 1))), as the RFC defines it.
    let k: Vec<u32> = (0..64u32).map(|i| (f64::from(i + 1).sin().abs() * 4_294_967_296.0) as u32).collect();
    let (mut a0, mut b0, mut c0, mut d0) = (0x6745_2301u32, 0xefcd_ab89u32, 0x98ba_dcfeu32, 0x1032_5476u32);
    let mut msg = Zeroizing::new(Vec::with_capacity(data.len() + 72));
    msg.extend_from_slice(data);
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&(data.len() as u64).wrapping_mul(8).to_le_bytes());
    for chunk in msg.as_chunks::<64>().0 {
        let mut m = [0u32; 16];
        let words = chunk.as_chunks::<4>().0;
        for (w, b) in m.iter_mut().zip(words) {
            *w = u32::from_le_bytes(*b);
        }
        let (mut a, mut b, mut c, mut d) = (a0, b0, c0, d0);
        for i in 0..64usize {
            let (f, g) = match i / 16 {
                0 => ((b & c) | (!b & d), i),
                1 => ((d & b) | (!d & c), (5 * i + 1) % 16),
                2 => (b ^ c ^ d, (3 * i + 5) % 16),
                _ => (c ^ (b | !d), (7 * i) % 16),
            };
            let f = f.wrapping_add(a).wrapping_add(k[i]).wrapping_add(m[g]);
            a = d;
            d = c;
            c = b;
            b = b.wrapping_add(f.rotate_left(S[i]));
        }
        a0 = a0.wrapping_add(a);
        b0 = b0.wrapping_add(b);
        c0 = c0.wrapping_add(c);
        d0 = d0.wrapping_add(d);
        m.zeroize();
    }
    let mut out = [0u8; 16];
    let (cells, _) = out.as_chunks_mut::<4>();
    for (o, v) in cells.iter_mut().zip([a0, b0, c0, d0]) {
        *o = v.to_le_bytes();
    }
    out
}

/// SHA-1, as [MS-OFFCRYPTO] §2.3.5 specifies for the RC4 CryptoAPI key derivation.
/// The digest is part of the file format, not a choice made here: a stronger hash would
/// make password-protected files written by Office unreadable.
fn sha1(parts: &[&[u8]]) -> Zeroizing<Vec<u8>> {
    let mut h = Sha1::new(); // NOSONAR: required by the MS-OFFCRYPTO RC4 CryptoAPI format
    for p in parts {
        h.update(p);
    }
    Zeroizing::new(h.finalize().to_vec())
}

/// The RC4 stream cipher. Its state is wiped when dropped.
pub struct Rc4 {
    s: [u8; 256],
    i: u8,
    j: u8,
}

impl Rc4 {
    pub fn new(key: &[u8]) -> Rc4 {
        let mut s = [0u8; 256];
        for (n, v) in s.iter_mut().enumerate() {
            *v = n as u8;
        }
        let kl = key.len().max(1);
        let mut j = 0u8;
        for i in 0..256usize {
            let k = key.get(i % kl).copied().unwrap_or(0);
            j = j.wrapping_add(s[i]).wrapping_add(k);
            s.swap(i, usize::from(j));
        }
        Rc4 { s, i: 0, j: 0 }
    }

    fn next(&mut self) -> u8 {
        self.i = self.i.wrapping_add(1);
        self.j = self.j.wrapping_add(self.s[usize::from(self.i)]);
        self.s.swap(usize::from(self.i), usize::from(self.j));
        self.s[usize::from(self.s[usize::from(self.i)].wrapping_add(self.s[usize::from(self.j)]))]
    }

    /// Encrypt or decrypt (the same thing) `data` in place, continuing the keystream.
    pub fn apply(&mut self, data: &mut [u8]) {
        for b in data {
            *b ^= self.next();
        }
    }
}

impl Drop for Rc4 {
    fn drop(&mut self) {
        self.s.zeroize();
        self.i.zeroize();
        self.j.zeroize();
    }
}

fn utf16le(password: &str) -> Zeroizing<Vec<u8>> {
    Zeroizing::new(password.encode_utf16().take(MAX_PASSWORD_CHARS).flat_map(u16::to_le_bytes).collect())
}

fn u16_at(d: &[u8], at: usize) -> Option<u16> {
    Some(u16::from_le_bytes(d.get(at..at.checked_add(2)?)?.try_into().ok()?))
}

fn u32_at(d: &[u8], at: usize) -> Option<u32> {
    Some(u32::from_le_bytes(d.get(at..at.checked_add(4)?)?.try_into().ok()?))
}

fn truncated() -> Error {
    Error::Malformed("the encryption header is truncated".into())
}

// ---------------------------------------------------------------------------------------------
// The key

/// The key of one RC4 or RC4 CryptoAPI file, once its password has been checked.
pub struct Key {
    /// The password hash: the truncated MD5 hash for RC4, the SHA-1 hash for RC4 CryptoAPI.
    base: Zeroizing<Vec<u8>>,
    crypto_api: bool,
    /// Bytes of the per-block key (CryptoAPI only; RC4 keys are 16 bytes).
    key_len: usize,
}

impl Key {
    /// Read the encryption header at the start of `info` (`EncryptionVersionInfo` and what
    /// follows it), check `password` against its verifier, and return the key and the length
    /// of the header, the bytes before any encrypted data.
    ///
    /// [`Error::WrongPassword`] when the verifier doesn't match; [`Error::Unsupported`] for a
    /// header this module doesn't read (`AES`, `external`, a `version`, `cipher`, `hash` or
    /// key size); [`Error::Malformed`] for a truncated or inconsistent header.
    pub fn open(info: &[u8], password: &str) -> Result<(Key, usize), Error> {
        let major = u16_at(info, 0).ok_or_else(truncated)?;
        let minor = u16_at(info, 2).ok_or_else(truncated)?;
        match (major, minor) {
            (1, 1) => Self::open_rc4(info, password),
            (2..=4, 2) => Self::open_crypto_api(info, password),
            _ => Err(Error::Unsupported(format!("version {major}.{minor}"))),
        }
    }

    /// Whether this is an RC4 CryptoAPI key (header version x.2) rather than plain RC4 (1.1).
    pub fn is_crypto_api(&self) -> bool {
        self.crypto_api
    }

    fn open_rc4(info: &[u8], password: &str) -> Result<(Key, usize), Error> {
        let salt = info.get(4..20).ok_or_else(truncated)?;
        let enc_verifier = info.get(20..36).ok_or_else(truncated)?;
        let enc_hash = info.get(36..52).ok_or_else(truncated)?;
        let mut h0 = md5(&utf16le(password));
        let mut buf = Zeroizing::new(Vec::with_capacity(336));
        for _ in 0..16 {
            buf.extend_from_slice(&h0[..5]);
            buf.extend_from_slice(salt);
        }
        h0.zeroize();
        let mut h1 = md5(&buf);
        let key = Key { base: Zeroizing::new(h1[..5].to_vec()), crypto_api: false, key_len: 16 };
        h1.zeroize();
        let mut check = Zeroizing::new([enc_verifier, enc_hash].concat());
        key.cipher(0).apply(&mut check);
        let (verifier, hash) = check.split_at(16);
        if !same(&md5(verifier), hash) {
            return Err(Error::WrongPassword);
        }
        Ok((key, 52))
    }

    fn open_crypto_api(info: &[u8], password: &str) -> Result<(Key, usize), Error> {
        let hflags = u32_at(info, 4).ok_or_else(truncated)?;
        if hflags & H_EXTERNAL != 0 {
            return Err(Error::Unsupported("external".into()));
        }
        // [MS-OFFCRYPTO] defines AES only for the encrypted packages of Office 2007 and later;
        // a 97-2003 binary file may only use RC4 CryptoAPI here, so there is no scheme to follow.
        if hflags & H_AES != 0 {
            return Err(Error::Unsupported("AES".into()));
        }
        if hflags & H_CRYPTO_API == 0 {
            return Err(Error::Unsupported("version x.2 without CryptoAPI".into()));
        }
        let hsize = usize::try_from(u32_at(info, 8).ok_or_else(truncated)?).unwrap_or(usize::MAX);
        // EncryptionHeader: flags, sizeExtra, algId, algIdHash, keySize, providerType, 2 reserved
        // words and the CSP name; it starts at byte 12.
        if hsize < 32 || hsize > info.len() {
            return Err(Error::Malformed("the encryption header has a bad size".into()));
        }
        let alg_id = u32_at(info, 20).ok_or_else(truncated)?;
        let alg_hash = u32_at(info, 24).ok_or_else(truncated)?;
        if !matches!(alg_id, 0 | 0x6801) {
            return Err(Error::Unsupported(format!("cipher 0x{alg_id:X}")));
        }
        if !matches!(alg_hash, 0 | 0x8004) {
            return Err(Error::Unsupported(format!("hash 0x{alg_hash:X}")));
        }
        let mut bits = u32_at(info, 28).ok_or_else(truncated)?;
        if bits == 0 {
            bits = 40;
        }
        if !(40..=128).contains(&bits) || bits % 8 != 0 {
            return Err(Error::Unsupported(format!("{bits}-bit RC4")));
        }
        let key_len = (bits / 8) as usize;
        // EncryptionVerifier: saltSize, salt[16], encryptedVerifier[16], verifierHashSize,
        // encryptedVerifierHash[20].
        let v = 12 + hsize;
        if u32_at(info, v) != Some(16) {
            return Err(Error::Malformed("the encryption salt has a bad size".into()));
        }
        let salt = info.get(v + 4..v + 20).ok_or_else(truncated)?;
        let enc_verifier = info.get(v + 20..v + 36).ok_or_else(truncated)?;
        if u32_at(info, v + 36) != Some(20) {
            return Err(Error::Unsupported("verifier hash other than SHA-1".into()));
        }
        let enc_hash = info.get(v + 40..v + 60).ok_or_else(truncated)?;
        let key = Key { base: sha1(&[salt, &utf16le(password)]), crypto_api: true, key_len };
        let mut check = Zeroizing::new([enc_verifier, enc_hash].concat());
        key.cipher(0).apply(&mut check);
        let (verifier, hash) = check.split_at(16);
        if !same(&sha1(&[verifier]), hash) {
            return Err(Error::WrongPassword);
        }
        Ok((key, v + 60))
    }

    /// The RC4 key of block number `block`.
    pub fn block_key(&self, block: u32) -> Zeroizing<Vec<u8>> {
        if !self.crypto_api {
            let mut input = Zeroizing::new([self.base.as_slice(), &block.to_le_bytes()].concat());
            let k = Zeroizing::new(md5(&input).to_vec());
            input.zeroize();
            return k;
        }
        let mut k = sha1(&[&self.base, &block.to_le_bytes()]);
        k.truncate(self.key_len);
        // A 40-bit key is zero-padded to 128 bits.
        if self.key_len == 5 {
            k.resize(16, 0);
        }
        k
    }

    /// A fresh cipher for block number `block`.
    pub fn cipher(&self, block: u32) -> Rc4 {
        Rc4::new(&self.block_key(block))
    }

    /// Decrypt (or encrypt) `data` in place as a stream re-keyed every [`BLOCK`] bytes, block
    /// numbers counting from 0 at its start. Its first `skip` bytes are stored untransformed,
    /// though the keystream runs over them too.
    pub fn decrypt_blocks(&self, data: &mut [u8], skip: usize) {
        for (n, chunk) in data.chunks_mut(BLOCK).enumerate() {
            let Ok(block) = u32::try_from(n) else { return };
            let mut rc4 = self.cipher(block);
            let start = n * BLOCK;
            let plain = skip.saturating_sub(start).min(chunk.len());
            let head = Zeroizing::new(chunk.get(..plain).map(<[u8]>::to_vec).unwrap_or_default());
            rc4.apply(chunk);
            if let Some(dst) = chunk.get_mut(..plain) {
                dst.copy_from_slice(&head);
            }
        }
    }
}

/// Compare without stopping at the first difference.
fn same(a: &[u8], b: &[u8]) -> bool {
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(b: &[u8]) -> String {
        b.iter().map(|x| format!("{x:02x}")).collect()
    }

    #[test]
    fn md5_matches_the_rfc_1321_vectors() {
        assert_eq!(hex(&md5(b"")), "d41d8cd98f00b204e9800998ecf8427e");
        assert_eq!(hex(&md5(b"a")), "0cc175b9c0f1b6a831c399e269772661");
        assert_eq!(hex(&md5(b"abc")), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(&md5(b"message digest")), "f96b697d7cb7938d525a2f31aaf161d0");
        assert_eq!(
            hex(&md5(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890")),
            "57edf4a22be3c955ac49da2e2107b67a"
        );
    }

    #[test]
    fn rc4_matches_a_published_vector() {
        let mut d = b"Plaintext".to_vec();
        Rc4::new(b"Key").apply(&mut d);
        assert_eq!(hex(&d), "bbf316e8d940af0ad3");
    }
}
