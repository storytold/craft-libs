//! Password-protected Word documents and Restrict Editing passwords, through `craft-offcrypto`
//! ([MS-OFFCRYPTO]): this module maps its results onto [`DocxError`] and the
//! `w:documentProtection` attributes.

use craft_offcrypto::protection;

use crate::DocxError;

#[cfg(test)]
use craft_offcrypto::protection::legacy_key;
pub use craft_offcrypto::{DEFAULT_SPIN_COUNT, MAX_PASSWORD_CHARS, is_encrypted};

fn map(e: craft_offcrypto::Error) -> DocxError {
    match e {
        craft_offcrypto::Error::PasswordRequired => DocxError::PasswordRequired,
        craft_offcrypto::Error::WrongPassword => DocxError::WrongPassword,
        craft_offcrypto::Error::TooLarge(m) => DocxError::Limit(m),
        other => DocxError::Encryption(other.to_string()),
    }
}

/// The zip package inside an encrypted Office file. Without a password (`None`) this says
/// [`DocxError::PasswordRequired`] once it knows the file is one it can decrypt; a password that
/// doesn't fit gives [`DocxError::WrongPassword`].
pub fn decrypt(bytes: &[u8], password: Option<&str>) -> Result<Vec<u8>, DocxError> {
    craft_offcrypto::decrypt(bytes, password).map_err(map)
}

/// Encrypt a zip package with `password` (agile encryption: AES-256-CBC, SHA-512, Word's spin
/// count), as Word's "Encrypt with Password" does.
pub fn encrypt(package: &[u8], password: &str) -> Result<Vec<u8>, DocxError> {
    craft_offcrypto::encrypt(package, password).map_err(map)
}

/// [`encrypt`] with a chosen spin count (tests use a small one; files should use the default).
pub fn encrypt_with_spin_count(package: &[u8], password: &str, spin_count: u32) -> Result<Vec<u8>, DocxError> {
    craft_offcrypto::encrypt_with_spin_count(package, password, spin_count).map_err(map)
}

/// A package encrypted with the *Standard* method (Word 2007), for tests: saves use the agile
/// method ([`encrypt`]). `key_bits` is 128, 192 or 256.
#[doc(hidden)]
pub fn encrypt_standard_for_tests(package: &[u8], password: &str, key_bits: u32) -> Result<Vec<u8>, DocxError> {
    craft_offcrypto::encrypt_standard(package, password, key_bits).map_err(map)
}

/// Refuse a password Word wouldn't accept: empty, or longer than [`MAX_PASSWORD_CHARS`].
pub fn check_password(password: &str) -> Result<(), DocxError> {
    craft_offcrypto::check_password(password).map_err(map)
}

/// Restrict Editing's password (`w:documentProtection`, ECMA-376 Part 1 §17.15.1.29): the
/// attributes that store its hash. Protection isn't encryption: the document stays readable,
/// the password only guards turning protection off.
pub fn protection_hash(password: &str, spin_count: u32) -> Result<Vec<(String, String)>, DocxError> {
    let h = protection::hash_password(password, spin_count).map_err(map)?;
    Ok(vec![
        ("w:algorithmName".into(), h.algorithm_name.into()),
        ("w:hashValue".into(), h.hash_value),
        ("w:saltValue".into(), h.salt_value),
        ("w:spinCount".into(), h.spin_count.to_string()),
    ])
}

/// What checking a Restrict Editing password found ([`check_protection_password`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PasswordCheck {
    /// The protection has no password: anyone may stop it.
    NoPassword,
    /// The password matches the stored hash.
    Correct,
    /// The password doesn't match.
    Incorrect,
    /// The hash uses an algorithm or form that can't be checked, or is damaged. The protection
    /// must stay on: guessing would let any password through.
    Unsupported,
}

impl From<protection::PasswordCheck> for PasswordCheck {
    fn from(c: protection::PasswordCheck) -> PasswordCheck {
        match c {
            protection::PasswordCheck::Correct => PasswordCheck::Correct,
            protection::PasswordCheck::Incorrect => PasswordCheck::Incorrect,
            protection::PasswordCheck::Unsupported => PasswordCheck::Unsupported,
        }
    }
}

/// Does `password` match a Restrict Editing hash (attributes as [`protection_hash`] makes them, or
/// as read from a file)? Two forms are checked:
///
/// - `w:algorithmName` / `w:hashValue` / `w:saltValue` / `w:spinCount`.
/// - `w:cryptAlgorithmSid` / `w:hash` / `w:salt` / `w:cryptSpinCount` (ECMA-376 Part 4 §14.8.1),
///   with `w:cryptAlgorithmClass` `hash`, `w:cryptAlgorithmType` `typeAny` and no `w:algIdExt`.
///
/// Anything else is [`PasswordCheck::Unsupported`], never a match.
pub fn check_protection_password(attrs: &[(String, String)], password: &str) -> PasswordCheck {
    let get = |k: &str| attrs.iter().find(|(n, _)| n == k).map(|(_, v)| v.trim());
    if let Some(hash) = get("w:hashValue") {
        let Some(alg) = get("w:algorithmName") else { return PasswordCheck::Unsupported };
        return protection::check_password_hash(alg, hash, get("w:saltValue"), get("w:spinCount"), password).into();
    }
    let Some(hash) = get("w:hash") else {
        // §17.15.1.29: without a hash no password is associated with the protection.
        return PasswordCheck::NoPassword;
    };
    let class_ok = get("w:cryptAlgorithmClass").is_none_or(|c| c == "hash");
    let type_ok = get("w:cryptAlgorithmType").is_none_or(|t| t == "typeAny");
    if !class_ok || !type_ok || get("w:algIdExt").is_some() {
        return PasswordCheck::Unsupported;
    }
    let Some(sid) = get("w:cryptAlgorithmSid").and_then(|v| v.parse::<u32>().ok()) else { return PasswordCheck::Unsupported };
    protection::check_legacy_password_hash(sid, hash, get("w:salt"), get("w:cryptSpinCount"), password).into()
}

#[cfg(test)]
mod tests;
