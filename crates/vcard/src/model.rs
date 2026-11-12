//! What a card holds: a contact, as the vCard properties this crate reads and writes.

use chrono::NaiveDate;

/// A value with a label (`work`, `home`, `mobile`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Labeled {
    pub label: String,
    pub value: String,
}

impl Labeled {
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Labeled {
        Labeled { label: label.into(), value: value.into() }
    }
}

/// A postal address (`ADR`). The street holds the post office box and the extended address too.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PostalAddress {
    pub label: String,
    pub street: String,
    pub city: String,
    pub region: String,
    pub postal_code: String,
    pub country: String,
}

impl PostalAddress {
    /// The address on one line, skipping empty parts.
    pub fn one_line(&self) -> String {
        let city = [self.city.as_str(), self.region.as_str(), self.postal_code.as_str()]
            .into_iter()
            .filter(|p| !p.trim().is_empty())
            .collect::<Vec<_>>()
            .join(" ");
        [self.street.as_str(), city.as_str(), self.country.as_str()].into_iter().filter(|p| !p.trim().is_empty()).collect::<Vec<_>>().join(", ")
    }

    pub fn is_empty(&self) -> bool {
        self.one_line().is_empty()
    }
}

/// What a key on a card is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum KeyKind {
    /// An S/MIME certificate (X.509, DER).
    #[default]
    X509,
    /// An OpenPGP public key, binary or ASCII-armoured, as the card had it.
    OpenPgp,
    /// Where the card says a key is ([`ContactKey::uri`]); never fetched.
    Uri,
}

/// A public key or certificate a card carries (`KEY`). The reader keeps only what has the shape
/// of a certificate or of an OpenPGP public key, never secret key material.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ContactKey {
    pub kind: KeyKind,
    /// The key's bytes; empty for a [`KeyKind::Uri`].
    pub data: Vec<u8>,
    /// For a [`KeyKind::Uri`]: the address the card gave (`https:`, `ldap:`, `openpgp4fpr:`).
    pub uri: String,
    /// For a [`KeyKind::Uri`]: what the card says is there, [`ContactKey::X509_MEDIA_TYPE`] or
    /// [`ContactKey::OPENPGP_MEDIA_TYPE`]; empty when it doesn't say.
    pub media_type: String,
}

impl ContactKey {
    /// Most bytes of one key kept.
    pub const MAX_BYTES: usize = 64 * 1024;
    /// Longest address of a key kept, in bytes.
    pub const MAX_URI: usize = 2048;
    /// The media type of an S/MIME certificate.
    pub const X509_MEDIA_TYPE: &str = "application/pkix-cert";
    /// The media type of OpenPGP keys.
    pub const OPENPGP_MEDIA_TYPE: &str = "application/pgp-keys";
}

/// One card.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Contact {
    /// `FN`; when empty, [`Contact::name`] builds one.
    pub display_name: String,
    pub first: String,
    pub middle: String,
    pub last: String,
    pub nickname: String,
    pub company: String,
    pub title: String,
    pub department: String,
    pub emails: Vec<Labeled>,
    pub phones: Vec<Labeled>,
    pub addresses: Vec<PostalAddress>,
    pub websites: Vec<String>,
    pub birthday: Option<NaiveDate>,
    pub notes: String,
    pub categories: Vec<String>,
    /// `X-FAVORITE` (and the forms other address books write it in).
    pub favorite: bool,
    /// `KEY`: certificates and OpenPGP public keys, at most [`Contact::MAX_KEYS`].
    pub keys: Vec<ContactKey>,
    /// `UID`, kept so a written card updates the same contact elsewhere.
    pub uid: String,
}

impl Contact {
    /// Most keys kept for one contact.
    pub const MAX_KEYS: usize = 32;

    /// The name to show: the display name, else first and last, the company, or the first address.
    pub fn name(&self) -> String {
        if !self.display_name.trim().is_empty() {
            return self.display_name.trim().to_string();
        }
        let full = [self.first.trim(), self.middle.trim(), self.last.trim()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        if !full.is_empty() {
            return full;
        }
        if !self.company.trim().is_empty() {
            return self.company.trim().to_string();
        }
        self.emails.first().map(|e| e.value.clone()).unwrap_or_default()
    }
}
