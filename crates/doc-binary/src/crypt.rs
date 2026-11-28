//! Password-protected Word 97-2003 files ([MS-DOC] §2.2.6, [MS-OFFCRYPTO] §2.3.5-2.3.7).
//!
//! Three schemes exist for `.doc`:
//!
//! * **XOR obfuscation** (`fObfuscated`): a 32-bit password verifier sits in `FibBase.lKey` and
//!   the streams are XORed with a 16-byte array derived from the password (method 2).
//! * **RC4** (header version 1.1): `EncryptionHeader` at the start of the Table stream; the key
//!   per 512-byte block comes from MD5.
//! * **RC4 CryptoAPI** (header version 2/3/4 . 2): like RC4 but with a SHA-1 key derivation, a
//!   key size of 40-128 bits and a variable-size header.
//!
//! In all three the WordDocument stream (past its first 68 bytes), the Table stream (past its
//! header, for RC4) and the whole Data stream are transformed. The password is checked against
//! the stored verifier before anything is decrypted, so a wrong password is reported and never
//! yields a garbled document. Everything is written from the public specifications. The two RC4
//! schemes are `craft-offcrypto`'s `rc4` module, shared with the other binary formats; XOR
//! obfuscation, which only Word uses in this form, is here.

use craft_offcrypto::rc4;

use crate::DocbinError;

/// Bytes at the start of the WordDocument stream that are stored untransformed.
pub(crate) const FIB_PLAIN: usize = 68;
/// XOR obfuscation uses at most this many single-byte password characters.
const MAX_XOR_PASSWORD: usize = 15;
/// `FibBase.flags`: the document is encrypted (`fEncrypted`) / obfuscated (`fObfuscated`).
pub(crate) const F_ENCRYPTED: u16 = 0x0100;
pub(crate) const F_OBFUSCATED: u16 = 0x8000;

/// The Word error for an RC4 header `craft-offcrypto` didn't open.
fn rc4_error(e: craft_offcrypto::Error) -> DocbinError {
    match e {
        craft_offcrypto::Error::WrongPassword => DocbinError::WrongPassword,
        // [MS-OFFCRYPTO] defines AES only for the encrypted packages of Word 2007 and later
        // (which open through the .docx reader whatever their extension); a Word 97-2003 file
        // may only use XOR, RC4 or RC4 CryptoAPI, so there is no public scheme to follow here.
        craft_offcrypto::Error::Unsupported(what) if what == "AES" => DocbinError::Encryption(
            "this Word 97-2003 file claims AES encryption, which is only defined for the newer encrypted-package format; it can't be opened".into(),
        ),
        craft_offcrypto::Error::Malformed(m) => DocbinError::Malformed(m),
        other => DocbinError::Encryption(other.to_string()),
    }
}

// ---------------------------------------------------------------------------------------------
// XOR obfuscation (method 1 key/verifier pieces, method 2 array and transform)

const PAD_ARRAY: [u8; 15] = [0xBB, 0xFF, 0xFF, 0xBA, 0xFF, 0xFF, 0xB9, 0x80, 0x00, 0xBE, 0x0F, 0x00, 0xBF, 0x0F, 0x00];
const INITIAL_CODE: [u16; 15] =
    [0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F, 0x84F9, 0x280C, 0xA96A, 0x4EC3];
const XOR_MATRIX: [u16; 105] = [
    0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09, 0x7B61, 0xF6C2, 0xFDA5, 0xEB6B, 0xC6F7, 0x9DCF, 0x2BBF, 0x4563, 0x8AC6, 0x05AD, 0x0B5A,
    0x16B4, 0x2D68, 0x5AD0, 0x0375, 0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40, 0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D, 0xAA7A, 0x44D5, 0x6F45,
    0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A, 0xEB23, 0xC667, 0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9, 0x47D3, 0x8FA6, 0x0F6D, 0x1EDA, 0x3DB4,
    0x7B68, 0xF6D0, 0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC, 0x45A0, 0x8B40, 0x06A1, 0x0D42, 0x1A84, 0x3508, 0x6A10, 0xAA51, 0x4483,
    0x8906, 0x022D, 0x045A, 0x08B4, 0x1168, 0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C, 0x3730, 0x6E60, 0xDCC0, 0xA9A1, 0x4363, 0x86C6,
    0x1DAD, 0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC, 0x1021, 0x2042, 0x4084, 0x8108, 0x1231, 0x2462, 0x48C4,
];

/// `CreateXorKey_Method1`. `pw` is 1 to 15 bytes.
fn xor_key(pw: &[u8]) -> u16 {
    let mut key = INITIAL_CODE.get(pw.len().wrapping_sub(1)).copied().unwrap_or(0);
    let mut current = 0x68i32;
    for &ch in pw.iter().rev() {
        let mut ch = ch;
        for _ in 0..7 {
            if ch & 0x40 != 0
                && let Some(m) = usize::try_from(current).ok().and_then(|c| XOR_MATRIX.get(c))
            {
                key ^= m;
            }
            ch <<= 1;
            current -= 1;
        }
    }
    key
}

/// `CreatePasswordVerifier_Method1`.
fn verifier_method1(pw: &[u8]) -> u16 {
    let mut v = 0u16;
    for &b in pw.iter().rev().chain(std::iter::once(&(pw.len() as u8))) {
        let i1 = u16::from(v & 0x4000 != 0);
        let i2 = v.wrapping_mul(2) & 0x7FFF;
        v = (i1 | i2) ^ u16::from(b);
    }
    v ^ 0xCE4B
}

/// `CreatePasswordVerifier_Method2`: the 32-bit value stored in `FibBase.lKey`.
pub(crate) fn xor_verifier(pw: &[u8]) -> u32 {
    (u32::from(xor_key(pw)) << 16) | u32::from(verifier_method1(pw))
}

/// `CreateXorArray_Method2`.
pub(crate) fn xor_array(pw: &[u8]) -> [u8; 16] {
    let key = (xor_verifier(pw) >> 16) as u16;
    let (high, low) = ((key >> 8) as u8, key as u8);
    let mut arr = [0u8; 16];
    for (i, slot) in arr.iter_mut().enumerate() {
        *slot = pw.get(i).copied().unwrap_or_else(|| PAD_ARRAY.get(i.saturating_sub(pw.len())).copied().unwrap_or(0));
    }
    let (pairs, _) = arr.as_chunks_mut::<2>();
    for pair in pairs {
        pair[0] = (pair[0] ^ low).rotate_right(1);
        pair[1] = (pair[1] ^ high).rotate_right(1);
    }
    arr
}

/// The single-byte forms of a password the XOR scheme tries: the system-codepage form (Latin-1
/// here) and the "low byte, or high byte when the low one is zero" form. Each is cut to 15.
fn xor_candidates(password: &str) -> Vec<Vec<u8>> {
    let mut out: Vec<Vec<u8>> = Vec::new();
    let latin: Option<Vec<u8>> = password.chars().take(MAX_XOR_PASSWORD).map(|c| u8::try_from(u32::from(c)).ok()).collect();
    if let Some(l) = latin {
        out.push(l);
    }
    let folded: Vec<u8> = password.encode_utf16().take(MAX_XOR_PASSWORD).map(|u| if u & 0xFF == 0 { (u >> 8) as u8 } else { u as u8 }).collect();
    if !out.contains(&folded) {
        out.push(folded);
    }
    out.retain(|p| !p.is_empty());
    out
}

// ---------------------------------------------------------------------------------------------
// The cipher

/// How the streams of one file are transformed, once the password has been verified.
pub(crate) enum Cipher {
    Xor { array: [u8; 16] },
    Rc4 { key: rc4::Key, table_skip: usize },
}

impl Cipher {
    /// Check `password` against the file's verifier and set up the cipher. `flags` and `lkey` come
    /// from the FIB base, `table` is the (still encrypted) Table stream.
    pub(crate) fn open(flags: u16, lkey: u32, table: &[u8], password: &str) -> Result<Cipher, DocbinError> {
        if flags & F_OBFUSCATED != 0 {
            for pw in xor_candidates(password) {
                if xor_verifier(&pw) == lkey {
                    return Ok(Cipher::Xor { array: xor_array(&pw) });
                }
            }
            return Err(DocbinError::WrongPassword);
        }
        let (key, table_skip) = rc4::Key::open(table, password).map_err(rc4_error)?;
        Ok(Cipher::Rc4 { key, table_skip })
    }

    /// Decrypt `data` in place; its first `skip` bytes are stored untransformed.
    fn decrypt(&self, data: &mut [u8], skip: usize) {
        match self {
            Cipher::Xor { array } => {
                for (i, b) in data.iter_mut().enumerate().skip(skip) {
                    let a = array[i % 16];
                    // Zero bytes, and bytes equal to the array element, were left as they were.
                    if *b != 0 && *b != a {
                        *b ^= a;
                    }
                }
            }
            Cipher::Rc4 { key, .. } => key.decrypt_blocks(data, skip),
        }
    }

    pub(crate) fn decrypt_word(&self, data: &mut [u8]) {
        self.decrypt(data, FIB_PLAIN);
    }

    pub(crate) fn decrypt_table(&self, data: &mut [u8]) {
        let skip = match self {
            Cipher::Xor { .. } => 0,
            Cipher::Rc4 { table_skip, .. } => *table_skip,
        };
        self.decrypt(data, skip);
    }

    pub(crate) fn decrypt_data(&self, data: &mut [u8]) {
        self.decrypt(data, 0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xor_candidates_cut_to_fifteen_and_skip_empty() {
        assert!(xor_candidates("").is_empty());
        assert_eq!(xor_candidates("abcdefghijklmnopqrst")[0].len(), 15);
        // Not representable as single bytes: only the folded form remains.
        assert_eq!(xor_candidates("\u{4e2d}").len(), 1);
    }
}
