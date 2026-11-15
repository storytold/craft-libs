// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Certificates read from a file the owner imports as Public keys (#30): DER or PEM, what each
//! one is for, and whether the file holds a private key instead (which is never a Public key
//! to import, and goes through File › Options › Security in the window). No I/O; the file is
//! hostile: sizes and counts are capped, and nothing here panics on any bytes.

use mailcraft_ber as ber;
use mailcraft_time::Timestamp;

use crate::cert::{Cert, MAX_CERT_BYTES, printable};

/// The most certificates read from one file.
pub const MAX_IMPORT_CERTIFICATES: usize = 16;
/// The most addresses a certificate is kept for.
const MAX_ADDRESSES: usize = 16;

/// What a certificate to import says of itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateInfo {
    /// The certificate, in DER.
    pub der: Vec<u8>,
    /// Its SHA-256 fingerprint, as a Digital signature's details have it.
    pub fingerprint: String,
    /// The mail addresses it is for (subject alternative names, and an `emailAddress` in its
    /// subject), in lower case: plain ASCII only, as a signer's address is compared.
    pub addresses: Vec<String>,
    /// Until when it is valid.
    pub not_after: Timestamp,
    /// Whether it is for mail ([`Cert::for_mail`]).
    pub for_mail: bool,
    /// Whether it is a certification authority's.
    pub authority: bool,
    /// Who it is for (its subject's common name, else its whole subject), cleaned.
    pub name: String,
    /// Who issued it (the issuer's common name, else its whole name), cleaned.
    pub issuer: String,
}

/// What the certificate `der` says of itself; `None` when it isn't one.
pub fn certificate_info(der: &[u8]) -> Option<CertificateInfo> {
    let cert = Cert::from_der(der)?;
    let mut addresses: Vec<String> = Vec::new();
    for a in cert.raw_addresses() {
        let a = a.to_ascii_lowercase();
        if printable(&a) && a.contains('@') && !addresses.contains(&a) && addresses.len() < MAX_ADDRESSES {
            addresses.push(a);
        }
    }
    Some(CertificateInfo {
        der: cert.der().to_vec(),
        fingerprint: cert.fingerprint(),
        addresses,
        not_after: cert.validity().1,
        for_mail: cert.for_mail(),
        authority: cert.is_authority(),
        name: cert.name(),
        issuer: cert.issuer_name(),
    })
}

/// The certificates in `bytes`: PEM `CERTIFICATE` blocks (as many as there are, at most
/// [`MAX_IMPORT_CERTIFICATES`]), else one certificate in DER (or BER). Empty when there is none.
pub fn certificates_in(bytes: &[u8]) -> Vec<Vec<u8>> {
    let blocks = pem_blocks(bytes, "CERTIFICATE");
    if !blocks.is_empty() {
        return blocks.into_iter().filter(|b| Cert::from_der(b).is_some()).take(MAX_IMPORT_CERTIFICATES).collect();
    }
    if bytes.len() > MAX_CERT_BYTES {
        return Vec::new();
    }
    match ber::to_der(bytes) {
        Some(der) if Cert::from_der(&der).is_some() => vec![der],
        _ => Vec::new(),
    }
}

/// The decoded bodies of the PEM blocks labelled `label` in `bytes` (RFC 7468), each at most a
/// certificate's size, at most [`MAX_IMPORT_CERTIFICATES`].
fn pem_blocks(bytes: &[u8], label: &str) -> Vec<Vec<u8>> {
    let text = String::from_utf8_lossy(bytes);
    let begin = format!("-----BEGIN {label}-----");
    let end = format!("-----END {label}-----");
    let mut out = Vec::new();
    let mut body: Option<String> = None;
    for line in text.lines() {
        let line = line.trim();
        match body.as_mut() {
            None if line == begin => body = Some(String::new()),
            None => {}
            Some(b) if line == end => {
                if let Ok(der) = mailcraft_charset::base64::STANDARD.decode(b.as_bytes())
                    && der.len() <= MAX_CERT_BYTES
                {
                    out.push(der);
                }
                body = None;
                if out.len() >= MAX_IMPORT_CERTIFICATES {
                    break;
                }
            }
            Some(b) => {
                // (A header line of RFC 1421's kind is no base64: such a block won't decode.)
                if b.len() > MAX_CERT_BYTES.saturating_mul(2) {
                    body = None;
                } else {
                    b.push_str(line);
                }
            }
        }
    }
    out
}

/// Whether `bytes` hold a private key in a form S/MIME's tools write: a PEM block of any kind of
/// `PRIVATE KEY` (PKCS #8, encrypted or not, PKCS #1, SEC 1, OpenSSH, OpenPGP's armour too), a
/// PKCS #12 file, or a PKCS #8, PKCS #1 or SEC 1 key in DER.
pub fn holds_private_key(bytes: &[u8]) -> bool {
    let text = String::from_utf8_lossy(bytes.get(..bytes.len().min(crate::pkcs12::MAX_P12_LEN)).unwrap_or_default());
    if text.lines().any(|l| {
        let l = l.trim();
        l.starts_with("-----BEGIN ") && l.contains("PRIVATE KEY")
    }) {
        return true;
    }
    if crate::pkcs12::looks_like(bytes) {
        return true;
    }
    let Some(der) = ber::to_der(bytes.get(..bytes.len().min(crate::pkcs12::MAX_P12_LEN)).unwrap_or_default()) else { return false };
    let Some((outer, _)) = ber::read(&der) else { return false };
    if outer.tag != 0x30 {
        return false;
    }
    let Some(fields) = ber::children(outer.value) else { return false };
    let tags: Vec<u8> = fields.iter().map(|f| f.tag).collect();
    let first_int = |v: &[u8]| fields.first().is_some_and(|f| f.tag == 0x02 && f.value == v);
    // PKCS #8 PrivateKeyInfo / OneAsymmetricKey: version, algorithm, the key in an OCTET STRING.
    let pkcs8 = matches!(tags.as_slice(), [0x02, 0x30, 0x04, ..]) && (first_int(&[0]) || first_int(&[1]));
    // EncryptedPrivateKeyInfo: algorithm, the encrypted key.
    let encrypted = matches!(tags.as_slice(), [0x30, 0x04]);
    // PKCS #1 RSAPrivateKey: version 0 and eight INTEGERs.
    let rsa = tags.len() >= 9 && tags.iter().take(9).all(|t| *t == 0x02) && first_int(&[0]);
    // SEC 1 ECPrivateKey: version 1, the key in an OCTET STRING.
    let ec = matches!(tags.as_slice(), [0x02, 0x04, ..]) && first_int(&[1]);
    pkcs8 || encrypted || rsa || ec
}
