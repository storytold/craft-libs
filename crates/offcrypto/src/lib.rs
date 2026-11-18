#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
//! Clean-room Office document cryptography, written from Microsoft's public Open Specification
//! **[MS-OFFCRYPTO]: Office Document Cryptography Structure** and ECMA-376.
//!
//! A password-encrypted `.docx`, `.xlsx` or `.pptx` is not a zip but a compound file
//! ([MS-CFB], read and written with `craft-cfb`) holding `EncryptionInfo` (how the key comes
//! from the password) and `EncryptedPackage` (the zip, AES encrypted). This crate goes between
//! the two: file bytes in, package bytes out, and back. It knows nothing about what the package
//! holds.
//!
//! - [`is_encrypted`] tells such a file from any other compound file; [`check`] says, without
//!   a password, whether a password could open it.
//! - [`decrypt`] reads *agile* encryption (Office 2010 and later: AES-CBC in 4096-byte
//!   segments, SHA-1 to SHA-512, a password verifier and an HMAC over the encrypted stream,
//!   which is required and checked) and *standard* encryption (Office 2007: AES-ECB, SHA-1).
//!   RC4, extensible, certificate-only and rights-managed files are refused by name
//!   ([`Error::Unsupported`]).
//! - [`encrypt`] writes agile encryption as Office does: AES-256, SHA-512, 100,000 spins.
//! - [`protection`] has the password hashes of document, sheet and workbook protection.
//!
//! Files are hostile: every stream read is capped, every length is checked before slicing, and
//! the spin count is bounded by the specification's maximum (10,000,000). Passwords and keys
//! are wiped from memory when dropped and never logged.
//!
//! ```
//! # fn main() -> Result<(), craft_offcrypto::Error> {
//! let package = b"PK\x03\x04 a zip package";
//! let file = craft_offcrypto::encrypt_with_spin_count(package, "secret", 1000)?;
//! assert!(craft_offcrypto::is_encrypted(&file));
//! assert_eq!(craft_offcrypto::decrypt(&file, None), Err(craft_offcrypto::Error::PasswordRequired));
//! assert_eq!(craft_offcrypto::decrypt(&file, Some("guess")), Err(craft_offcrypto::Error::WrongPassword));
//! assert_eq!(craft_offcrypto::decrypt(&file, Some("secret"))?, package);
//! # Ok(())
//! # }
//! ```

mod package;
pub mod protection;

pub use package::{
    DEFAULT_SPIN_COUNT, MAX_PASSWORD_CHARS, check, check_password, decrypt, decrypt_streams, encrypt, encrypt_standard, encrypt_with_spin_count,
    is_encrypted,
};

/// Why a file could not be decrypted or encrypted.
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[non_exhaustive]
pub enum Error {
    /// The file can be decrypted, and no password was given.
    #[error("a password is required")]
    PasswordRequired,
    #[error("the password is incorrect")]
    WrongPassword,
    /// A scheme that is not read, named to complete "uses {0} encryption": `RC4`, `Extensible`,
    /// `external`, `certificate`, `rights-management`, a cipher or hash name, `version 1.1`.
    #[error("{0} encryption isn't supported")]
    Unsupported(String),
    /// The file is damaged, or fails its integrity check.
    #[error("the encrypted file is damaged ({0})")]
    Malformed(String),
    /// A stream is larger than its cap.
    #[error("the encrypted file is too large ({0})")]
    TooLarge(String),
    /// A password that can't be used to encrypt: empty, or longer than [`MAX_PASSWORD_CHARS`].
    #[error("{0}")]
    BadPassword(String),
}
