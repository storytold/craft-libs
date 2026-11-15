// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! An X.509 certificate (RFC 5280) as a Digital signature is checked with: its bytes, what it
//! says, and the few questions asked of it (who it names, for what, until when, who signed it).

use const_oid::ObjectIdentifier;
use der::{Decode, Encode};
use mailcraft_model::digital_signature::{MAX_SIGNER_ADDRESSES, signer_text};
use mailcraft_time::Timestamp;
use x509_cert::Certificate;
use x509_cert::ext::pkix::constraints::name::GeneralSubtrees;
use x509_cert::ext::pkix::name::GeneralName;
use x509_cert::ext::pkix::{BasicConstraints, ExtendedKeyUsage, KeyUsage, NameConstraints, SubjectAltName, SubjectKeyIdentifier};

use crate::algorithms::{self, Fault};
use mailcraft_ber as ber;

/// The largest certificate read.
pub const MAX_CERT_BYTES: usize = 64 * 1024;

const EMAIL_PROTECTION: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.3.6.1.5.5.7.3.4");
const ANY_EXTENDED_KEY_USAGE: ObjectIdentifier = ObjectIdentifier::new_unwrap("2.5.29.37.0");
const EMAIL_ADDRESS: ObjectIdentifier = ObjectIdentifier::new_unwrap("1.2.840.113549.1.9.1");

/// The extensions whose meaning is known here (RFC 5280 §4.2): a certificate with any other
/// marked critical is refused (§6.1.4 (o)). Certificate policies are taken as any policy (no
/// policy is asked for, §6.1.1 (c)); the identifiers say nothing to check.
const KNOWN: [ObjectIdentifier; 8] = [
    ObjectIdentifier::new_unwrap("2.5.29.14"), // subject key identifier
    ObjectIdentifier::new_unwrap("2.5.29.15"), // key usage
    ObjectIdentifier::new_unwrap("2.5.29.17"), // subject alternative name
    ObjectIdentifier::new_unwrap("2.5.29.19"), // basic constraints
    ObjectIdentifier::new_unwrap("2.5.29.30"), // name constraints
    ObjectIdentifier::new_unwrap("2.5.29.32"), // certificate policies
    ObjectIdentifier::new_unwrap("2.5.29.35"), // authority key identifier
    ObjectIdentifier::new_unwrap("2.5.29.37"), // extended key usage
];

/// The name constraints a certification authority puts on what it vouches for (RFC 5280
/// §4.2.1.10), for the two kinds of name a mail certificate has: mail addresses and directory
/// names. The other kinds name nothing that is checked here.
#[derive(Clone, Debug, Default)]
pub struct Constraints {
    pub permitted_mail: Option<Vec<String>>,
    pub excluded_mail: Vec<String>,
    pub permitted_dirs: Option<Vec<Vec<Vec<u8>>>>,
    pub excluded_dirs: Vec<Vec<Vec<u8>>>,
}

/// A certificate: its DER, the body its issuer signed, and what it says.
#[derive(Clone, Debug)]
pub struct Cert {
    der: Vec<u8>,
    /// Where the to-be-signed body is in `der`.
    tbs: std::ops::Range<usize>,
    parsed: Certificate,
    subject: Vec<u8>,
    issuer: Vec<u8>,
}

impl PartialEq for Cert {
    fn eq(&self, other: &Self) -> bool {
        self.der == other.der
    }
}

impl Cert {
    /// The certificate `der` is, when it is one.
    pub fn from_der(der: &[u8]) -> Option<Cert> {
        if der.len() > MAX_CERT_BYTES {
            return None;
        }
        let (outer, rest) = ber::read(der)?;
        if !rest.is_empty() {
            return None;
        }
        let (tbs, _) = ber::read(outer.value)?;
        let start = outer.whole.len().checked_sub(outer.value.len())?;
        let end = start.checked_add(tbs.whole.len())?;
        let parsed = Certificate::from_der(der).ok()?;
        let subject = parsed.tbs_certificate().subject().to_der().ok()?;
        let issuer = parsed.tbs_certificate().issuer().to_der().ok()?;
        Some(Cert { der: der.to_vec(), tbs: start..end, parsed, subject, issuer })
    }

    pub fn der(&self) -> &[u8] {
        &self.der
    }

    fn tbs(&self) -> &[u8] {
        self.der.get(self.tbs.clone()).unwrap_or_default()
    }

    pub fn subject(&self) -> &[u8] {
        &self.subject
    }

    pub fn issuer(&self) -> &[u8] {
        &self.issuer
    }

    /// The serial number's bytes.
    pub fn serial(&self) -> &[u8] {
        self.parsed.tbs_certificate().serial_number().as_bytes()
    }

    /// The subject key identifier, when it has one.
    pub fn key_id(&self) -> Option<Vec<u8>> {
        let (_, ski) = self.parsed.tbs_certificate().get_extension::<SubjectKeyIdentifier>().ok()??;
        Some(ski.0.as_bytes().to_vec())
    }

    pub fn public_key(&self) -> &spki::SubjectPublicKeyInfoOwned {
        self.parsed.tbs_certificate().subject_public_key_info()
    }

    /// Its SHA-256 fingerprint: upper-case hexadecimal pairs with colons between.
    pub fn fingerprint(&self) -> String {
        algorithms::Hash::Sha256.digest(&self.der).iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join(":")
    }

    /// From and until when it is valid, in seconds since 1970.
    pub fn validity(&self) -> (Timestamp, Timestamp) {
        let v = self.parsed.tbs_certificate().validity();
        let secs = |t: &x509_cert::time::Time| Timestamp::try_from(t.to_unix_duration().as_secs()).unwrap_or(Timestamp::MAX);
        (secs(&v.not_before), secs(&v.not_after))
    }

    /// Whether it is valid at `now`.
    pub fn valid_at(&self, now: Timestamp) -> bool {
        let (from, until) = self.validity();
        from <= now && now <= until
    }

    /// The name it is for: its common name, else its whole name.
    pub fn name(&self) -> String {
        name_of(self.parsed.tbs_certificate().subject())
    }

    /// The name of who issued it.
    pub fn issuer_name(&self) -> String {
        name_of(self.parsed.tbs_certificate().issuer())
    }

    /// The mail addresses it is for: its subject alternative names of that kind, and an
    /// `emailAddress` in its subject (RFC 8551 §3.1), at most [`MAX_SIGNER_ADDRESSES`].
    pub fn addresses(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(Some((_, san))) = self.parsed.tbs_certificate().get_extension::<SubjectAltName>() {
            for name in san.0.iter() {
                if let GeneralName::Rfc822Name(a) = name {
                    out.push(signer_text(a.as_str()));
                }
            }
        }
        for atv in self.parsed.tbs_certificate().subject().iter() {
            if atv.oid == EMAIL_ADDRESS
                && let Ok(a) = atv.value.decode_as::<der::asn1::Ia5String>()
            {
                out.push(signer_text(a.as_str()));
            }
        }
        out.retain(|a| !a.is_empty());
        out.dedup_by(|a, b| a.eq_ignore_ascii_case(b));
        out.truncate(MAX_SIGNER_ADDRESSES);
        out
    }

    /// The mail addresses it names, exactly as written (its alternative names of that kind and
    /// the `emailAddress` in its subject), at most [`MAX_SIGNER_ADDRESSES`] of each: for comparing
    /// and for name constraints, never for showing.
    pub fn raw_addresses(&self) -> Vec<String> {
        let mut out = Vec::new();
        if let Ok(Some((_, san))) = self.parsed.tbs_certificate().get_extension::<SubjectAltName>() {
            out.extend(
                san.0
                    .iter()
                    .filter_map(|n| match n {
                        GeneralName::Rfc822Name(a) => Some(a.as_str().to_string()),
                        _ => None,
                    })
                    .take(MAX_SIGNER_ADDRESSES),
            );
        }
        out.extend(
            self.parsed
                .tbs_certificate()
                .subject()
                .iter()
                .filter(|atv| atv.oid == EMAIL_ADDRESS)
                .filter_map(|atv| atv.value.decode_as::<der::asn1::Ia5String>().ok())
                .map(|a| a.as_str().to_string())
                .take(MAX_SIGNER_ADDRESSES),
        );
        out
    }

    /// Whether it is for `address`: one of its addresses as written is that address, letter case
    /// aside in plain ASCII (RFC 5280 §4.2.1.6, §7.5). An address with a space, a control or any
    /// character past ASCII on either side never is.
    pub fn is_for(&self, address: &str) -> bool {
        printable(address) && self.raw_addresses().iter().any(|a| printable(a) && a.eq_ignore_ascii_case(address))
    }

    /// The address of its it is for `address` by ([`Cert::is_for`]), as it shows it.
    pub fn address_for(&self, address: &str) -> Option<String> {
        printable(address).then(|| self.raw_addresses().into_iter().find(|a| printable(a) && a.eq_ignore_ascii_case(address))).flatten()
    }

    /// Whether it carries an extension marked critical that isn't known here.
    pub fn unknown_critical(&self) -> bool {
        self.parsed.tbs_certificate().extensions().is_some_and(|exts| exts.iter().any(|e| e.critical && !KNOWN.contains(&e.extn_id)))
    }

    /// Its path length constraint, when its basic constraints give one (unreadable: none
    /// allowed below it).
    pub fn path_len(&self) -> Option<u8> {
        match self.parsed.tbs_certificate().get_extension::<BasicConstraints>() {
            Ok(Some((_, bc))) => bc.path_len_constraint,
            Ok(None) => None,
            Err(_) => Some(0),
        }
    }

    /// Whether, as a certification authority on a path, its extended key usage (when it has
    /// one) lets it vouch for mail: email protection, or any use.
    pub fn may_vouch_for_mail(&self) -> bool {
        match self.parsed.tbs_certificate().get_extension::<ExtendedKeyUsage>() {
            Ok(Some((_, eku))) => eku.0.iter().any(|o| *o == EMAIL_PROTECTION || *o == ANY_EXTENDED_KEY_USAGE),
            Ok(None) => true,
            Err(_) => false,
        }
    }

    /// Whether it is a certification authority's by any sign (basic constraints say so, or its
    /// key usage lets it sign certificates): never a mail signer's.
    pub fn is_authority(&self) -> bool {
        let tbs = self.parsed.tbs_certificate();
        let ca = matches!(tbs.get_extension::<BasicConstraints>(), Ok(Some((_, bc))) if bc.ca);
        let signs = matches!(tbs.get_extension::<KeyUsage>(), Ok(Some((_, ku))) if ku.key_cert_sign());
        ca || signs
    }

    /// The name constraints it puts on what it vouches for; `Err` when they can't be read or use
    /// what RFC 5280 says mustn't be (a minimum or a maximum).
    pub fn constraints(&self) -> Result<Option<Constraints>, ()> {
        let Some((_, nc)) = self.parsed.tbs_certificate().get_extension::<NameConstraints>().map_err(|_| ())? else { return Ok(None) };
        let mut c = Constraints::default();
        if let Some(permitted) = &nc.permitted_subtrees {
            let (mail, dirs) = subtrees(permitted)?;
            c.permitted_mail = mail;
            c.permitted_dirs = dirs;
        }
        if let Some(excluded) = &nc.excluded_subtrees {
            let (mail, dirs) = subtrees(excluded)?;
            c.excluded_mail = mail.unwrap_or_default();
            c.excluded_dirs = dirs.unwrap_or_default();
        }
        Ok(Some(c))
    }

    /// Its subject's relative names, each in DER, in order: for directory name constraints.
    pub fn subject_rdns(&self) -> Vec<Vec<u8>> {
        self.parsed.tbs_certificate().subject().iter_rdn().filter_map(|r| r.to_der().ok()).collect()
    }

    /// Whether it may be a certification authority's: basic constraints say so, and its key
    /// usage, when it has one, lets it sign certificates.
    pub fn is_ca(&self) -> bool {
        let tbs = self.parsed.tbs_certificate();
        let ca = matches!(tbs.get_extension::<BasicConstraints>(), Ok(Some((_, bc))) if bc.ca);
        let signs = match tbs.get_extension::<KeyUsage>() {
            Ok(Some((_, ku))) => ku.key_cert_sign(),
            Ok(None) => true,
            Err(_) => false,
        };
        ca && signs
    }

    /// Whether it is for signing mail (RFC 8551 §4.4): its key usage, when it has one, allows a
    /// Digital signature or non-repudiation, and its extended key usage, when it has one, names
    /// email protection (or any use).
    pub fn for_email(&self) -> bool {
        let tbs = self.parsed.tbs_certificate();
        let usage = match tbs.get_extension::<KeyUsage>() {
            Ok(Some((_, ku))) => ku.digital_signature() || ku.non_repudiation(),
            Ok(None) => true,
            Err(_) => false,
        };
        let extended = match tbs.get_extension::<ExtendedKeyUsage>() {
            Ok(Some((_, eku))) => eku.0.iter().any(|o| *o == EMAIL_PROTECTION || *o == ANY_EXTENDED_KEY_USAGE),
            Ok(None) => true,
            Err(_) => false,
        };
        usage && extended
    }

    /// Whether it is for mail at all, to sign it or to encrypt it (RFC 8551 §4.4): its key usage,
    /// when it has one, allows a Digital signature, non-repudiation, key encipherment or key
    /// agreement, and its extended key usage, when it has one, names email protection (or any
    /// use). What a Public key imported for an address must be.
    pub fn for_mail(&self) -> bool {
        let tbs = self.parsed.tbs_certificate();
        let usage = match tbs.get_extension::<KeyUsage>() {
            Ok(Some((_, ku))) => ku.digital_signature() || ku.non_repudiation() || ku.key_encipherment() || ku.key_agreement(),
            Ok(None) => true,
            Err(_) => false,
        };
        usage && self.may_vouch_for_mail()
    }

    /// Whether it names itself as its issuer.
    pub fn self_issued(&self) -> bool {
        self.subject == self.issuer
    }

    /// Whether `issuer`'s key made its signature.
    pub fn signed_by(&self, issuer: &Cert) -> Result<(), Fault> {
        let signature = self.parsed.signature().as_bytes().ok_or(Fault::Invalid)?;
        algorithms::verify(issuer.public_key(), self.parsed.signature_algorithm(), None, self.tbs(), signature)
    }
}

/// Whether `address` has only the printable ASCII an address is compared in (and something).
pub(crate) fn printable(address: &str) -> bool {
    !address.is_empty() && address.bytes().all(|b| (0x21..=0x7e).contains(&b))
}

/// The mail and directory names of `subtrees` (`None` of a kind: none of that kind given).
type Subtrees = (Option<Vec<String>>, Option<Vec<Vec<Vec<u8>>>>);

fn subtrees(subtrees: &GeneralSubtrees) -> Result<Subtrees, ()> {
    let (mut mail, mut dirs): Subtrees = (None, None);
    for s in subtrees.iter().take(64) {
        if s.minimum != 0 || s.maximum.is_some() {
            return Err(());
        }
        match &s.base {
            GeneralName::Rfc822Name(a) => mail.get_or_insert_with(Vec::new).push(a.as_str().to_string()),
            GeneralName::DirectoryName(n) => dirs.get_or_insert_with(Vec::new).push(n.iter_rdn().filter_map(|r| r.to_der().ok()).collect()),
            _ => {}
        }
    }
    if subtrees.len() > 64 {
        return Err(());
    }
    Ok((mail, dirs))
}

/// Whether the mail address `address` is within the constraint `base` (RFC 5280 §4.2.1.10): a
/// whole address, every address at one host, or (with a leading dot) every address at any host
/// under a domain. Letter case aside.
pub fn mail_within(address: &str, base: &str) -> bool {
    let address = address.to_ascii_lowercase();
    let base = base.to_ascii_lowercase();
    if base.contains('@') {
        return address == base;
    }
    let Some((_, host)) = address.rsplit_once('@') else { return false };
    if let Some(domain) = base.strip_prefix('.') {
        return host.len() > domain.len() && host.ends_with(domain) && host.as_bytes().get(host.len() - domain.len() - 1) == Some(&b'.');
    }
    host == base
}

/// Whether the name `rdns` is within the directory name `base`: it begins with all of it, each
/// relative name compared as RFC 5280 §7.1 compares them (a name written in another case, with
/// other spaces or in the other of the two string types is the same name).
pub fn dir_within(rdns: &[Vec<u8>], base: &[Vec<u8>]) -> bool {
    rdns.len() >= base.len() && rdns.iter().zip(base).all(|(a, b)| a == b || matches!((rdn_key(a), rdn_key(b)), (Some(x), Some(y)) if x == y))
}

/// A relative name's attributes as they compare: each type with its value, a PrintableString or
/// UTF8String's with its letters in one case and its runs of spaces made one (and none at either
/// end), any other's as it is written; sorted, since the attributes of one are a set.
fn rdn_key(der: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    use der::Tag;
    let rdn = x509_cert::name::RelativeDistinguishedName::from_der(der).ok()?;
    let mut key: Vec<(String, Vec<u8>)> = rdn
        .iter()
        .map(|atv| {
            let value = match der::Tagged::tag(&atv.value) {
                Tag::PrintableString | Tag::Utf8String => {
                    let text = std::str::from_utf8(atv.value.value()).ok()?;
                    let folded: Vec<String> = text.split_whitespace().map(str::to_lowercase).collect();
                    let mut v = b"s".to_vec();
                    v.extend_from_slice(folded.join(" ").as_bytes());
                    v
                }
                _ => {
                    let mut v = b"b".to_vec();
                    v.extend(atv.value.to_der().ok()?);
                    v
                }
            };
            Some((atv.oid.to_string(), value))
        })
        .collect::<Option<_>>()?;
    key.sort();
    Some(key)
}

/// A name for people: its common name, else the whole of it as RFC 4514 writes it.
fn name_of(name: &x509_cert::name::Name) -> String {
    if let Ok(Some(cn)) = name.common_name() {
        let cn = signer_text(&cn.value());
        if !cn.is_empty() {
            return cn;
        }
    }
    signer_text(&name.to_string())
}
