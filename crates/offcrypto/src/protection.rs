//! Protection passwords: the hashes that guard turning document, sheet or workbook protection
//! off ([MS-OFFCRYPTO] §2.4.2.4, the ISO write-protection method; ECMA-376 Part 1 §17.15.1.29
//! and §18.2.29; Part 4 §14.8.1 for the legacy form). Protection isn't encryption: the file
//! stays readable, the password only guards the setting.
//!
//! The values are the attribute values as files hold them (base64 text, decimal spin count);
//! which attributes carry them (`w:hashValue`, `hashValue`, `w:hash`, ...) is the format's.

use base64::Engine as _;
use zeroize::Zeroizing;

use crate::Error;
use crate::package::{Hash, MAX_SPIN_COUNT, SALT, check_password, random, same};

/// A new protection password hash: SHA-512 of a random 16-byte salt and the password as
/// UTF-16LE, hashed again `spin_count` times with the iteration number appended.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PasswordHash {
    /// `algorithmName`: always `SHA-512`.
    pub algorithm_name: &'static str,
    /// `hashValue`, base64.
    pub hash_value: String,
    /// `saltValue`, base64.
    pub salt_value: String,
    /// `spinCount`.
    pub spin_count: u32,
}

/// Hashes a new protection password. The spin count is capped at 10,000,000; an empty or
/// over-long password is refused ([`check_password`]).
pub fn hash_password(password: &str, spin_count: u32) -> Result<PasswordHash, Error> {
    check_password(password)?;
    let spin_count = spin_count.min(MAX_SPIN_COUNT);
    let salt = random(SALT)?;
    let hash = iterate_appended(Hash::Sha512, &salt, password, spin_count);
    let b64 = |b: &[u8]| base64::engine::general_purpose::STANDARD.encode(b);
    Ok(PasswordHash { algorithm_name: "SHA-512", hash_value: b64(&hash), salt_value: b64(&salt), spin_count })
}

/// What checking a protection password found.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    /// The password matches the stored hash.
    Correct,
    /// The password doesn't match.
    Incorrect,
    /// The hash uses an algorithm or form that can't be checked, or is damaged. The protection
    /// must stay on: guessing would let any password through.
    Unsupported,
}

/// Does `password` match a stored hash of the current form (`algorithmName`, `hashValue`,
/// `saltValue`, `spinCount`)? The algorithm is SHA-1, SHA-256, SHA-384 or SHA-512 (with or
/// without the dash); a missing salt is empty and a missing spin count is 0. Anything else, a
/// spin count above 10,000,000 or damaged values are [`PasswordCheck::Unsupported`], never a
/// match.
pub fn check_password_hash(
    algorithm_name: &str,
    hash_value: &str,
    salt_value: Option<&str>,
    spin_count: Option<&str>,
    password: &str,
) -> PasswordCheck {
    let Some(hash) = Hash::parse(algorithm_name.trim()) else { return PasswordCheck::Unsupported };
    verify(hash, Some(hash_value.trim()), salt_value.map(str::trim), spin_count.map(str::trim), password)
}

/// Does `password` match a stored hash of the legacy form (`cryptAlgorithmSid`, `hash`, `salt`,
/// `cryptSpinCount`; ECMA-376 Part 4 §14.8.1)? The password first becomes Word's legacy 32-bit
/// key ([`legacy_key`]), whose bytes, reversed and written as an upper-case hex string, are
/// hashed like a password. `algorithm_sid` is 4 (SHA-1), 12 (SHA-256), 13 (SHA-384) or 14
/// (SHA-512); the older MD2, MD4 and MD5 are [`PasswordCheck::Unsupported`].
pub fn check_legacy_password_hash(algorithm_sid: u32, hash: &str, salt: Option<&str>, spin_count: Option<&str>, password: &str) -> PasswordCheck {
    // Part 4 §14.8.1 lists 1 MD2, 2 MD4, 3 MD5, 4 SHA-1, 12 SHA-256, 13 SHA-384, 14 SHA-512.
    let algorithm = match algorithm_sid {
        4 => Hash::Sha1,
        12 => Hash::Sha256,
        13 => Hash::Sha384,
        14 => Hash::Sha512,
        _ => return PasswordCheck::Unsupported,
    };
    verify(algorithm, Some(hash.trim()), salt.map(str::trim), spin_count.map(str::trim), &legacy_key_hex(password))
}

/// Hash `input` with the stored salt and spin count and compare with the stored hash. Damaged or
/// out-of-range values are [`PasswordCheck::Unsupported`].
fn verify(hash: Hash, want: Option<&str>, salt: Option<&str>, spin_count: Option<&str>, input: &str) -> PasswordCheck {
    let dec = |v: &str| base64::engine::general_purpose::STANDARD.decode(v).ok();
    let Some(want) = want.and_then(dec) else { return PasswordCheck::Unsupported };
    // §17.15.1.29: without a salt nothing is prepended.
    let salt = match salt {
        Some(v) => match dec(v) {
            Some(s) => s,
            None => return PasswordCheck::Unsupported,
        },
        None => Vec::new(),
    };
    // Word accepts spin counts from 0 to 10,000,000 ([MS-OI29500] 2.1.402 d); a missing one is 0.
    let spin_count = match spin_count.map(str::parse::<u32>) {
        None => 0,
        Some(Ok(n)) if n <= MAX_SPIN_COUNT => n,
        Some(_) => return PasswordCheck::Unsupported,
    };
    if want.len() != hash.size() || salt.len() > 1024 {
        return PasswordCheck::Unsupported;
    }
    if same(&iterate_appended(hash, &salt, input, spin_count), &want) { PasswordCheck::Correct } else { PasswordCheck::Incorrect }
}

/// The initial value of the legacy key's high-order word, by password length 1–15 (ECMA-376
/// Part 4 §14.8.1, "Initial code array").
const LEGACY_INITIAL_CODE: [u16; 15] =
    [0xE1F0, 0x1D0F, 0xCC9C, 0x84C0, 0x110C, 0x0E10, 0xF1CE, 0x313E, 0x1872, 0xE139, 0xD40F, 0x84F9, 0x280C, 0xA96A, 0x4EC3];

/// The legacy key's encryption matrix (Part 4 §14.8.1): one row per character position, rows
/// "Last-14" to "Last", one column per bit 0–6. The password's last character uses the last row.
const LEGACY_MATRIX: [[u16; 7]; 15] = [
    [0xAEFC, 0x4DD9, 0x9BB2, 0x2745, 0x4E8A, 0x9D14, 0x2A09],
    [0x7B61, 0xF6C2, 0xFDA5, 0xEB6B, 0xC6F7, 0x9DCF, 0x2BBF],
    [0x4563, 0x8AC6, 0x05AD, 0x0B5A, 0x16B4, 0x2D68, 0x5AD0],
    [0x0375, 0x06EA, 0x0DD4, 0x1BA8, 0x3750, 0x6EA0, 0xDD40],
    [0xD849, 0xA0B3, 0x5147, 0xA28E, 0x553D, 0xAA7A, 0x44D5],
    [0x6F45, 0xDE8A, 0xAD35, 0x4A4B, 0x9496, 0x390D, 0x721A],
    [0xEB23, 0xC667, 0x9CEF, 0x29FF, 0x53FE, 0xA7FC, 0x5FD9],
    [0x47D3, 0x8FA6, 0x0F6D, 0x1EDA, 0x3DB4, 0x7B68, 0xF6D0],
    [0xB861, 0x60E3, 0xC1C6, 0x93AD, 0x377B, 0x6EF6, 0xDDEC],
    [0x45A0, 0x8B40, 0x06A1, 0x0D42, 0x1A84, 0x3508, 0x6A10],
    [0xAA51, 0x4483, 0x8906, 0x022D, 0x045A, 0x08B4, 0x1168],
    [0x76B4, 0xED68, 0xCAF1, 0x85C3, 0x1BA7, 0x374E, 0x6E9C],
    [0x3730, 0x6E60, 0xDCC0, 0xA9A1, 0x4363, 0x86C6, 0x1DAD],
    [0x3331, 0x6662, 0xCCC4, 0x89A9, 0x0373, 0x06E6, 0x0DCC],
    [0x1021, 0x2042, 0x4084, 0x8108, 0x1231, 0x2462, 0x48C4],
];

/// Word's legacy 32-bit password key (ECMA-376 Part 4 §14.8.1): the password as UTF-16 without a
/// leading BOM, cut to 15 characters, each character reduced to one byte (its low byte, or its
/// high byte when the low byte is 0); the high-order word comes from the initial code array and
/// the encryption matrix, the low-order word from rotating and XOR-ing the bytes backwards.
pub fn legacy_key(password: &str) -> u32 {
    let mut units = password.encode_utf16().peekable();
    units.next_if_eq(&0xFEFF);
    let bytes: Zeroizing<Vec<u8>> =
        Zeroizing::new(units.take(LEGACY_INITIAL_CODE.len()).map(|u| if u & 0xFF != 0 { (u & 0xFF) as u8 } else { (u >> 8) as u8 }).collect());
    let Some(&initial) = bytes.len().checked_sub(1).and_then(|i| LEGACY_INITIAL_CODE.get(i)) else { return 0 };
    let mut high = initial;
    let first_row = LEGACY_MATRIX.len() - bytes.len();
    for (row, &c) in LEGACY_MATRIX.iter().skip(first_row).zip(bytes.iter()) {
        for (bit, &code) in row.iter().enumerate() {
            if c & (1 << bit) != 0 {
                high ^= code;
            }
        }
    }
    let rotate = |w: u16| ((w >> 14) & 0x0001) | ((w << 1) & 0x7FFF);
    let mut low = 0u16;
    for &c in bytes.iter().rev() {
        low = rotate(low) ^ u16::from(c);
    }
    // `bytes` holds at most 15 values, so its length fits a word.
    low = rotate(low) ^ (bytes.len() as u16) ^ 0xCE4B;
    (u32::from(high) << 16) | u32::from(low)
}

/// What the legacy form hashes: the key's bytes in reversed order (Part 4 §14.8.1: 0x64CEED7E
/// becomes 7EEDCE64), as an upper-case hex string — hashed as UTF-16LE like a password, as Word
/// does ([MS-OE376] 2.1.410 a).
fn legacy_key_hex(password: &str) -> Zeroizing<String> {
    let key = legacy_key(password);
    Zeroizing::new(key.to_le_bytes().iter().map(|b| format!("{b:02X}")).collect())
}

/// The ISO write-protection hash ([MS-OFFCRYPTO] §2.4.2.4): H0 = H(salt + password as UTF-16LE),
/// Hn = H(Hn-1 + iterator) — the iterator after the hash, unlike encryption's [`iterate`].
fn iterate_appended(hash: Hash, salt: &[u8], password: &str, spin_count: u32) -> Zeroizing<Vec<u8>> {
    let pw: Zeroizing<Vec<u8>> = Zeroizing::new(password.encode_utf16().flat_map(u16::to_le_bytes).collect());
    let mut h = Zeroizing::new(hash.hash(&[salt, &pw]));
    for i in 0..spin_count.min(MAX_SPIN_COUNT) {
        *h = hash.hash(&[&h, &i.to_le_bytes()]);
    }
    h
}

#[cfg(test)]
mod tests {
    use super::PasswordCheck::{Correct, Incorrect, Unsupported};
    use super::*;

    #[test]
    fn password_hash_round_trip() {
        let h = hash_password("s3cret", 1000).unwrap();
        assert_eq!((h.algorithm_name, h.spin_count), ("SHA-512", 1000));
        let spin = h.spin_count.to_string();
        let check = |algorithm: &str, hash: &str, password: &str| check_password_hash(algorithm, hash, Some(&h.salt_value), Some(&spin), password);
        assert_eq!(check(h.algorithm_name, &h.hash_value, "s3cret"), Correct);
        assert_eq!(check("SHA512", &format!(" {} ", h.hash_value), "s3cret"), Correct, "as SpreadsheetML spells it, with stray spaces");
        assert_eq!(check(h.algorithm_name, &h.hash_value, "S3cret"), Incorrect);
        // Damaged values and unknown algorithms are refused, never accepted.
        assert_eq!(check(h.algorithm_name, "not base64!", "s3cret"), Unsupported);
        assert_eq!(check("WHIRLPOOL", &h.hash_value, "s3cret"), Unsupported);
        assert_eq!(check("SHA-256", &h.hash_value, "s3cret"), Unsupported, "a hash of the wrong length");
        assert_eq!(check_password_hash("SHA-512", &h.hash_value, Some("!"), Some(&spin), "s3cret"), Unsupported);
        assert_eq!(check_password_hash("SHA-512", &h.hash_value, Some(&h.salt_value), Some("10000001"), "s3cret"), Unsupported);
        assert_eq!(check_password_hash("SHA-512", &h.hash_value, Some(&h.salt_value), Some("-1"), "s3cret"), Unsupported);
        // A missing salt is empty and a missing spin count is 0: a different hash, not a match.
        assert_eq!(check_password_hash("SHA-512", &h.hash_value, None, None, "s3cret"), Incorrect);
        // The spin count is capped, and unusable passwords are refused.
        assert!(matches!(hash_password("", 1), Err(Error::BadPassword(_))));
    }

    /// The worked example of ECMA-376 Part 4 §14.8.1: "Example" gives the key 0x64CEED7E, hashed
    /// as "7EEDCE64".
    #[test]
    fn legacy_key_matches_the_specification_example() {
        assert_eq!(legacy_key("Example"), 0x64CE_ED7E);
        assert_eq!(legacy_key_hex("Example").as_str(), "7EEDCE64");
        assert_eq!(legacy_key(""), 0);
        // A leading BOM is dropped; only the first 15 characters count.
        assert_eq!(legacy_key("\u{FEFF}Example"), 0x64CE_ED7E);
        assert_eq!(legacy_key("abcdefghijklmnopq"), legacy_key("abcdefghijklmno"));
        assert_ne!(legacy_key("abcdefghijklmno"), legacy_key("abcdefghijklmn"));
    }

    /// SHA-1 of the salt 00 01 … 0F and "7EEDCE64" as UTF-16LE, iterated 100,000 times,
    /// computed independently from the specification's stages.
    const KNOWN_LEGACY_SHA1: &str = "pEWelKoH6SVpvMmyPAJ8bHA/g/s=";

    #[test]
    fn legacy_hash_is_verified() {
        let check = |sid: u32, spin: &str, password: &str| {
            check_legacy_password_hash(sid, KNOWN_LEGACY_SHA1, Some("AAECAwQFBgcICQoLDA0ODw=="), Some(spin), password)
        };
        assert_eq!(check(4, "100000", "Example"), Correct);
        assert_eq!(check(4, "100000", "example"), Incorrect);
        assert_eq!(check(4, "100000", ""), Incorrect);
        // MD5 and the other algorithms that are not implemented, and hostile spin counts: refused.
        assert_eq!(check(3, "100000", "Example"), Unsupported);
        assert_eq!(check(4, "4294967295", "Example"), Unsupported);
        assert_eq!(check(4, "-1", "Example"), Unsupported);
    }
}
