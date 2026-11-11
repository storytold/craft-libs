//! The formats of digital signatures: ASN.1 DER, X.509 certificates, CMS SignedData (PKCS #7),
//! PKCS #12 identity files, RFC 3161 timestamps, and CRL / OCSP revocation evidence.
//!
//! - [`der`]: a small BER/DER reader and writer; a [`der::Tlv`] keeps its exact encoding, so
//!   signed structures are verified over the bytes that were signed.
//! - [`x509`]: certificates (RFC 5280), the checks along a chain of issuers, certificate files
//!   (DER, PEM, PKCS #7 bundles), and a self-signed certificate builder.
//! - [`cms`]: CMS SignedData (RFC 5652), parsed, verified and written (detached, or with the
//!   content encapsulated), with RFC 3161 tokens attached as unsigned attributes.
//! - [`pkcs12`]: `.p12` / `.pfx` files (RFC 7292), read and written, with the key-derivation
//!   work a file may ask for capped.
//! - [`timestamp`]: RFC 3161 requests, responses and tokens. No transport.
//! - [`revocation`]: CRLs and OCSP responses, parsed, verified and looked up. Nothing is fetched.
//! - [`keys`]: digests, public keys (verification) and private keys (signing).
//!
//! The cryptography is RustCrypto's, and aws-lc-rs's for RSA private-key operations on native
//! targets. Two things are done here because no audited crate offers them, both verification
//! only and on public data: the RSA signature paddings for any digest, MGF1 hash and salt
//! length (over the `rsa` crate's raw public-key operation), and ECDSA on brainpoolP512r1.
//!
//! What a signature means for a document or a message (which bytes it covers, what changed
//! after it, whom the reader trusts) is the caller's.
//!
//! Input is hostile: nesting is bounded, lengths are checked against what is there, and
//! nothing here panics on any bytes.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod cms;
pub mod der;
mod ec512;
pub mod keys;
pub mod pkcs12;
pub mod revocation;
mod rsa_pad;
pub mod timestamp;
pub mod x509;

pub use der::Time;
pub use keys::{DigestAlg, PrivateKey, PublicKey};
pub use pkcs12::DigitalId;
pub use timestamp::{TimestampAuthority, TimestampQuery, TimestampToken};
pub use x509::{Certificate, Name};

/// What went wrong reading, checking or writing one of these formats.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("malformed data: {0}")]
    Malformed(String),
    #[error("not supported: {0}")]
    Unsupported(String),
    #[error("the password is incorrect")]
    WrongPassword,
    #[error("{0}")]
    Crypto(String),
}
