// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft's S/MIME (RFC 8551): Digital signatures checked (#30), written from the public
//! specifications (RFC 5652 for CMS, RFC 5280 for X.509, RFC 5754, RFC 4055 and RFC 5480 for
//! the algorithms). No I/O: the host hands in the certificates it trusts ([`Anchors`]), the
//! message's halves come from `mailcraft-mime`, and the result is
//! [`mailcraft_model::digital_signature::DigitalSignature`], which OpenPGP shares.
//!
//! [`check`] reads the CMS SignedData of a detached or an opaque Digital signature, finds the
//! signer's certificate, compares the signer's digest with the message's, checks the signature
//! (RSA PKCS #1 v1.5 or PSS of 2048 bits or more, ECDSA on P-256 or P-384, over SHA-2), and then
//! whether the certificate rests on a trust anchor, is for signing mail and is for the From
//! address. MD5 and SHA-1 are no longer safe and are said to be. Revocation isn't checked.
//!
//! Input is hostile: BER is turned into DER with nesting bounded, every length is checked
//! against what is there, the certificates, signers and the path to an anchor are capped, and
//! nothing here panics on any bytes.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]
#![cfg_attr(not(test), deny(clippy::indexing_slicing))]

mod algorithms;
mod cert;
mod chain;
pub mod import;
pub mod pkcs12;
mod verify;

#[cfg(test)]
mod import_tests;
#[cfg(test)]
mod pkcs12_tests;
#[cfg(test)]
mod tests;

pub use chain::{Anchors, MAX_ANCHORS};
pub use verify::{Checked, Context, MAX_CERTS, MAX_SIGNERS, MAX_VERIFICATIONS, Signed, check};

/// The fingerprint and addresses of a certificate in DER, as a Digital signature's details show
/// them: `None` when it isn't one.
pub fn certificate_summary(der: &[u8]) -> Option<(String, Vec<String>, mailcraft_time::Timestamp)> {
    let cert = cert::Cert::from_der(der)?;
    Some((cert.fingerprint(), cert.addresses(), cert.validity().1))
}
