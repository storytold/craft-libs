// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What checking a message's Digital signature found (#30), whichever format it is in: S/MIME
//! (`craft-smime`) or OpenPGP. One of four results ([`Verdict`]), why when it isn't the
//! first ([`Why`]), and what is known of who signed it ([`Signer`]). Worked out from the bytes the
//! message came as, in memory, and never written to the mailbox file: it is checked again when
//! the message is next looked at, against the trust and the clock of that moment.

use serde::{Deserialize, Serialize};

use crate::Timestamp;

/// The format of a Digital signature.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SignatureFormat {
    /// S/MIME (RFC 8551): CMS signed data and an X.509 certificate.
    Smime,
    /// OpenPGP (RFC 9580), as PGP/MIME (RFC 3156) or inline.
    OpenPgp,
}

impl SignatureFormat {
    pub fn as_str(self) -> &'static str {
        match self {
            SignatureFormat::Smime => "smime",
            SignatureFormat::OpenPgp => "openpgp",
        }
    }
}

/// The four results a check has, best first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Verdict {
    /// The Digital signature is valid, its signer is trusted, the signer's address is the
    /// message's one From address (verified is for that signer and that address together, never
    /// for a key alone), and what was signed is the whole message ([`Coverage::Whole`]).
    Verified,
    /// The Digital signature is valid, but its signer isn't verified ([`Why`] says how): an
    /// unknown issuer, a certificate out of date or not for mail, an address that isn't the From
    /// address, an OpenPGP key nobody marked verified.
    NotVerified,
    /// The message was changed after it was signed: what is shown isn't what was signed.
    Changed,
    /// The Digital signature can't be checked ([`Why`] says why): an algorithm that's no longer
    /// safe or isn't known, no Public key, or a signature that can't be read.
    Unchecked,
}

impl Verdict {
    /// Its name for an agent and a script: `verified`, `notVerified`, `changed`, `unchecked`.
    pub fn as_str(self) -> &'static str {
        match self {
            Verdict::Verified => "verified",
            Verdict::NotVerified => "notVerified",
            Verdict::Changed => "changed",
            Verdict::Unchecked => "unchecked",
        }
    }

    /// The better of two results ([`Verdict::Verified`] best): of several signers of one
    /// message, the best one stands.
    pub fn best(self, other: Verdict) -> Verdict {
        self.min(other)
    }
}

/// Why a check found less than [`Verdict::Verified`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Why {
    // Not verified.
    /// The certificate wasn't issued by a certification authority this computer trusts.
    UnknownIssuer,
    /// The certificate signed itself, and isn't one this computer trusts.
    SelfSigned,
    /// The certificate, or one of those it rests on, is out of date (or not valid yet).
    Expired,
    /// The certificate isn't for signing mail (its key usage or extended key usage say so).
    NotForEmail,
    /// The certificate is for another address than the message's From address.
    AddressMismatch,
    /// An authority on the way to a trusted one doesn't allow the certificate to be used this
    /// way: a name outside what it may vouch for, a path too long, a use it doesn't allow, or a
    /// critical extension not known here (RFC 5280 §4.2, §6.1).
    Constrained,
    /// No certification authorities for mail could be read on this computer, so no signer can be
    /// verified here.
    NoAuthorities,
    /// This computer's certification authorities haven't been read yet (they are read at start):
    /// the check is made again when they are.
    AuthoritiesNotRead,
    /// An OpenPGP key that the owner hasn't marked Verified.
    KeyNotVerified,
    /// Only part of the message was signed (a wrapper such as a list's footer holds it), so
    /// what is outside that part could say anything.
    PartlySigned,
    // Changed.
    /// What was signed isn't what arrived.
    Altered,
    // Can't be checked.
    /// It uses an algorithm that's no longer safe (MD5, SHA-1, a short RSA key).
    WeakAlgorithm,
    /// It uses an algorithm MailCraft doesn't know.
    UnknownAlgorithm,
    /// The signer's Public key (certificate) isn't in the message or kept here.
    NoPublicKey,
    /// The signer's OpenPGP key was revoked by its owner, for when the message was signed.
    KeyRevoked,
    /// The signer's OpenPGP key had expired when the message was signed.
    KeyExpired,
    /// The OpenPGP key that made it isn't one that may sign.
    NotASigningKey,
    /// The Digital signature itself has expired (it said until when it holds).
    SignatureExpired,
    /// The Digital signature can't be read.
    Unreadable,
    /// The bytes the message came as aren't here (it was read before they were kept): Check
    /// again downloads them.
    NoOriginal,
    /// There is more to check than a message is allowed (many certificates, or many ways to a
    /// trusted authority): it was stopped before an answer.
    TooMuch,
}

impl Why {
    /// Its name for an agent and a script.
    pub fn as_str(self) -> &'static str {
        match self {
            Why::UnknownIssuer => "unknownIssuer",
            Why::SelfSigned => "selfSigned",
            Why::Expired => "expired",
            Why::NotForEmail => "notForEmail",
            Why::AddressMismatch => "addressMismatch",
            Why::Constrained => "constrained",
            Why::NoAuthorities => "noAuthorities",
            Why::AuthoritiesNotRead => "authoritiesNotRead",
            Why::KeyNotVerified => "keyNotVerified",
            Why::PartlySigned => "partlySigned",
            Why::Altered => "altered",
            Why::WeakAlgorithm => "weakAlgorithm",
            Why::UnknownAlgorithm => "unknownAlgorithm",
            Why::NoPublicKey => "noPublicKey",
            Why::KeyRevoked => "keyRevoked",
            Why::KeyExpired => "keyExpired",
            Why::NotASigningKey => "notASigningKey",
            Why::SignatureExpired => "signatureExpired",
            Why::Unreadable => "unreadable",
            Why::NoOriginal => "noOriginal",
            Why::TooMuch => "tooMuch",
        }
    }

    /// The plain English of it, for an agent, a script and the log; the window says it in its
    /// own language.
    pub fn english(self) -> &'static str {
        match self {
            Why::UnknownIssuer => "the certificate wasn't issued by a certification authority this computer trusts",
            Why::SelfSigned => "the certificate signed itself and isn't one this computer trusts",
            Why::Expired => "the certificate is out of date",
            Why::NotForEmail => "the certificate isn't for signing mail",
            Why::AddressMismatch => "the certificate is for another address than the sender's",
            Why::Constrained => "an authority on the way to a trusted one doesn't allow the certificate to be used this way",
            Why::NoAuthorities => "no certification authorities for mail could be read on this computer, so no signer can be verified here",
            Why::AuthoritiesNotRead => "this computer's certification authorities haven't been read yet",
            Why::KeyNotVerified => "the signer's key hasn't been marked verified",
            Why::PartlySigned => "only part of the message was signed; the rest could say anything",
            Why::Altered => "the message was changed after it was signed",
            Why::WeakAlgorithm => "it uses an algorithm that's no longer safe",
            Why::UnknownAlgorithm => "it uses an algorithm MailCraft doesn't know",
            Why::NoPublicKey => "the signer's Public key (certificate) isn't in the message or kept on this computer",
            Why::KeyRevoked => "the signer's key was revoked by its owner",
            Why::KeyExpired => "the signer's key had expired when the message was signed",
            Why::NotASigningKey => "the key that made it isn't one that may sign",
            Why::SignatureExpired => "the Digital signature has expired",
            Why::Unreadable => "the Digital signature can't be read",
            Why::NoOriginal => "the bytes the message came as aren't on this computer; Check again downloads them",
            Why::TooMuch => "there is too much to check in this Digital signature",
        }
    }
}

/// Who signed, as the certificate or key says: the signer's own words, data and never
/// instructions.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Signer {
    /// The name in the certificate (its common name) or key, if any.
    pub name: String,
    /// The addresses in the certificate or key, as they are written there, at most
    /// [`MAX_SIGNER_ADDRESSES`].
    pub addresses: Vec<String>,
    /// The SHA-256 fingerprint of the certificate (or the fingerprint of the OpenPGP key), in
    /// upper-case hexadecimal pairs with colons between.
    pub fingerprint: String,
    /// Who issued the certificate (its common name, else its whole name). Empty for OpenPGP.
    pub issuer: String,
    /// From and until when the certificate is valid; 0 when it doesn't say.
    pub not_before: Timestamp,
    pub not_after: Timestamp,
}

/// How much of the message the Digital signature covers.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Coverage {
    /// The whole message body: the signed part is the message's top level (S/MIME's
    /// `multipart/signed` or signed data, PGP/MIME's `multipart/signed`).
    Whole,
    /// Only a part of it: the signed part sits inside a wrapper whose other parts nobody signed
    /// (or a format that can't tell). The safe default.
    #[default]
    Part,
}

impl Coverage {
    pub fn as_str(self) -> &'static str {
        match self {
            Coverage::Whole => "whole",
            Coverage::Part => "part",
        }
    }
}

/// The most addresses of a signer that are kept.
pub const MAX_SIGNER_ADDRESSES: usize = 16;
/// The most characters of a name, an issuer or an address that are kept.
pub const MAX_SIGNER_TEXT: usize = 256;

/// What checking a message's Digital signature found.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DigitalSignature {
    pub format: SignatureFormat,
    pub verdict: Verdict,
    /// Why it is less than verified; `None` when it is verified.
    pub why: Option<Why>,
    /// Who signed, when the signer's certificate or key was found.
    pub signer: Option<Signer>,
    /// When the signer says they signed it (S/MIME's signing time): the signer's word, not
    /// checked by anybody.
    pub signed_at: Option<Timestamp>,
    /// Whether the certificate was checked against its issuer's list of revoked certificates.
    /// Never yet (#30 R7): the details say "Revocation not checked".
    pub revocation_checked: bool,
    /// How much of the message was signed; set with [`DigitalSignature::covering`].
    #[serde(default)]
    pub coverage: Coverage,
}

impl DigitalSignature {
    /// A result of `verdict` for `why`, with nothing known of the signer.
    pub fn bare(format: SignatureFormat, verdict: Verdict, why: Why) -> DigitalSignature {
        DigitalSignature { format, verdict, why: Some(why), signer: None, signed_at: None, revocation_checked: false, coverage: Coverage::Part }
    }

    /// This result for a signature that covers `coverage` of the message: a valid signature by a
    /// trusted signer over only part of it is [`Verdict::NotVerified`] ([`Why::PartlySigned`]),
    /// never verified for the message. Either format's checker ends with it.
    pub fn covering(mut self, coverage: Coverage) -> DigitalSignature {
        self.coverage = coverage;
        if coverage == Coverage::Part && self.verdict == Verdict::Verified {
            self.verdict = Verdict::NotVerified;
            self.why = Some(Why::PartlySigned);
        }
        self
    }
}

/// `text` (a name or address from a certificate) as a name coming in is kept
/// (`crate::names::incoming`): one line, without control or format characters (no
/// reordering by a bidirectional override, nothing hidden by a zero-width one), at most
/// [`MAX_SIGNER_TEXT`] characters.
pub fn signer_text(text: &str) -> String {
    crate::names::incoming(text, MAX_SIGNER_TEXT).unwrap_or_default()
}
