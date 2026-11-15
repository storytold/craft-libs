//! OpenPGP for mail: checking a message's Digital signature and reading the Public keys it
//! carries. No I/O, no clock and no keys of its own: the caller hands in the message's
//! original bytes (what `Message::original` keeps), the Public keys it keeps
//! ([`KeyRing`]), the time, and whether the owner has Verified a key; so it builds for the
//! browser too, which checks but never signs.
//!
//! - [`verify`] checks a PGP/MIME message (`multipart/signed;
//!   protocol="application/pgp-signature"`, RFC 3156 §5) over the signed part's canonical
//!   bytes, or a cleartext-signed ("inline") text (RFC 9580 §7), and says which of the four
//!   results it is ([`Outcome`]). OpenPGP has no certificate authorities and this crate no web of
//!   trust: a key is Verified only when the owner marked it after comparing its fingerprint
//!   or their GnuPG keyring trusts it fully, both of which the caller knows and this doesn't.
//! - [`keys_in_message`] reads the keys a message carries: attached `application/pgp-keys`
//!   parts and the `Autocrypt:` header (Autocrypt Level 1), for the caller to keep by
//!   address. Nothing is fetched or sent.
//!
//! The cryptography is rPGP's (`pgp`, MIT OR Apache-2.0); the MIME around it, the armour and the
//! Cleartext Signature Framework are read here, from RFC 2045/2046, RFC 3156 and RFC 9580, so
//! that every size is capped by this crate. Everything from outside is hostile: nothing here
//! panics on any bytes (a panic inside rPGP is caught and is an unreadable signature), work is
//! linear in the input, and sizes, counts and nesting are capped.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

mod armour;
mod autocrypt;
mod check;
mod keys;
mod mime;
pub mod secret;

#[cfg(test)]
mod armour_tests;
#[cfg(test)]
mod autocrypt_tests;
#[cfg(test)]
mod coverage_tests;
#[cfg(test)]
#[path = "fixtures_tests.rs"]
mod fixtures;
#[cfg(test)]
mod keys_tests;
#[cfg(test)]
mod mime_tests;
#[cfg(test)]
mod prop_tests;
#[cfg(test)]
mod secret_tests;
#[cfg(test)]
mod tests;
#[cfg(test)]
mod vector_tests;

pub use autocrypt::{Autocrypt, parse_autocrypt};
pub use check::{CantCheck, Checked, Coverage, Issuer, NotVerified, Outcome, Signer};
pub use keys::{Fingerprint, KeyId, KeyRing, PublicKey};

/// The largest message looked at, as large as the largest any server is fetched for.
pub const MAX_MESSAGE: usize = 150 << 20;
/// How deep into wrapping `multipart/mixed` parts the signed part is looked for, and how deep
/// into multiparts an attached key.
const MAX_DEPTH: usize = 8;
/// How many parts of a message are looked at for attached keys.
const MAX_PARTS: usize = 256;

/// Why key material couldn't be read.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
pub enum Error {
    /// Not OpenPGP key material, or none of it checks.
    #[error("no Public key that can be read")]
    Unreadable,
    /// Larger than any real key or header.
    #[error("larger than a Public key can be")]
    TooLarge,
    /// More than one key where only one is taken (a secret key to import).
    #[error("more than one key")]
    SeveralKeys,
}

/// The address of a From field's value, lower case: as [`verify`] takes it. Empty when the
/// field names no one address (none, or several).
pub fn from_address(value: &str) -> String {
    mime::address(value).trim().to_lowercase()
}

/// Runs `f`, giving `None` instead of letting a panic out of it: rPGP is a large library
/// reading hostile bytes, and a panic in it must not take the program down. Where panics abort (the
/// browser build) this is only a function call.
pub(crate) fn guard<T>(f: impl FnOnce() -> T) -> Option<T> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)).ok()
}

/// Checks the OpenPGP Digital signature of the message `original` (its bytes as they came),
/// sent from `from` (the From field's value or its address), against `keys`, at `now` (seconds
/// since 1970). `verified(fingerprint, address)` says whether the owner has Verified that key
/// for that address: marked it after comparing its fingerprint, or their GnuPG keyring
/// trusts it fully with that User ID.
///
/// `None` when the message has no OpenPGP Digital signature (an S/MIME one isn't this crate's,
/// and one inside `multipart/alternative` is none: the other alternative would be shown
/// unsigned). Never fails: what can't be read is [`CantCheck::Unreadable`]. A message is
/// [`Outcome::Verified`] only when the signature covers all of it ([`Coverage::whole`]); one
/// that covers part of it is at best [`Outcome::Valid`] with [`NotVerified::partly_signed`].
pub fn verify(original: &[u8], from: &str, keys: &KeyRing, now: u64, verified: impl Fn(&Fingerprint, &str) -> bool) -> Option<Checked> {
    if original.len() > MAX_MESSAGE {
        return looks_signed(original).then(|| Checked { outcome: Outcome::cant(CantCheck::TooLarge), coverage: Coverage::default() });
    }
    let cx = check::Context {
        from: mime::address(from),
        keys,
        now,
        verified: &verified,
        verifications: std::cell::Cell::new(0),
        data_passes: std::cell::Cell::new(0),
    };
    let (headers, body) = mime::split(original);
    let mut checked = find(&headers, body, &mut Vec::new(), &cx)?;
    if !checked.coverage.whole {
        checked.outcome = match checked.outcome {
            Outcome::Verified(signer) => Outcome::Valid { signer, why: NotVerified { partly_signed: true, ..NotVerified::default() } },
            Outcome::Valid { signer, why } => Outcome::Valid { signer, why: NotVerified { partly_signed: true, ..why } },
            other => other,
        };
    }
    Some(checked)
}

/// Whether a message too large to check is a signed one, told cheaply: a PGP/MIME type at the
/// top, or a cleartext-signed block anywhere in it.
fn looks_signed(original: &[u8]) -> bool {
    let (headers, _) = mime::split(original.get(..1 << 20).unwrap_or(original));
    let content_type = headers.content_type();
    let pgp_mime = content_type.value == "multipart/signed"
        && content_type.get("protocol").is_some_and(|p| p.trim().eq_ignore_ascii_case("application/pgp-signature"));
    pgp_mime || original.windows(34).any(|w| w == b"-----BEGIN PGP SIGNED MESSAGE-----")
}

/// The signed part of a message at `path`, checked.
fn find(headers: &mime::Headers, body: &[u8], path: &mut Vec<usize>, cx: &check::Context<'_>) -> Option<Checked> {
    let content_type = headers.content_type();
    match content_type.value.as_str() {
        "multipart/signed" => {
            let protocol = content_type.get("protocol").unwrap_or_default().trim().to_ascii_lowercase();
            if protocol != "application/pgp-signature" {
                return None;
            }
            // Only a PGP/MIME message at the top is all of the message.
            let coverage = Coverage { whole: path.is_empty(), path: path.clone(), span: None };
            let Some(boundary) = content_type.get("boundary").filter(|b| !b.is_empty()) else {
                return Some(Checked { outcome: Outcome::cant(CantCheck::Unreadable), coverage });
            };
            let parts = mime::parts(body, boundary, 3);
            // RFC 1847 §2.1: exactly two parts, what was signed and the signature.
            let [signed, signature] = parts.as_slice() else { return Some(Checked { outcome: Outcome::cant(CantCheck::Unreadable), coverage }) };
            let (sig_headers, sig_body) = mime::split(signature);
            let sig_bytes = mime::decode(&sig_headers.encoding(), sig_body);
            let packets = signature_packets(&sig_bytes);
            // RFC 3156 §5: the signed part with its header, in canonical form (CR LF).
            Some(Checked { outcome: check::check(&packets, &mime::crlf(signed), cx), coverage })
        }
        "text/plain" => {
            let text = mime::decode(&headers.encoding(), body);
            let clear = armour::cleartext(&text, check::MAX_SIGNATURE_BYTES)?;
            let coverage = Coverage { whole: path.is_empty() && clear.alone, path: path.clone(), span: Some(clear.span) };
            // Words shown in the block that aren't signed: nothing in it can be vouched for.
            let outcome = if clear.well_formed { check::check(&clear.signature, &clear.signed, cx) } else { Outcome::cant(CantCheck::Unreadable) };
            Some(Checked { outcome, coverage })
        }
        // A mailing list's wrapper: its first part, which is then only part of the message.
        // Never into `multipart/alternative`, whose other alternatives would be shown unsigned.
        "multipart/mixed" if path.len() < MAX_DEPTH => {
            let boundary = content_type.get("boundary").filter(|b| !b.is_empty())?;
            let first = *mime::parts(body, boundary, 1).first()?;
            let (part_headers, part_body) = mime::split(first);
            path.push(0);
            find(&part_headers, part_body, path, cx)
        }
        _ => None,
    }
}

/// The signature packets of a PGP/MIME signature part: armoured, as RFC 3156 §5 says, or
/// binary from a writer that didn't armour them. Empty when it holds neither, which checks as
/// an unreadable signature: the message is still a signed one.
fn signature_packets(bytes: &[u8]) -> Vec<u8> {
    let blocks = armour::blocks(bytes, armour::Kind::Signature, 1, check::MAX_SIGNATURE_BYTES);
    match blocks.into_iter().next() {
        Some(block) => block,
        None if bytes.first().is_some_and(|b| b & 0x80 != 0) && bytes.len() <= check::MAX_SIGNATURE_BYTES => bytes.to_vec(),
        None => Vec::new(),
    }
}

/// The Public keys a message carries, for the caller to keep by address.
#[derive(Clone, Debug, Default)]
pub struct Found {
    /// Keys in attached `application/pgp-keys` parts, their self-signatures checked.
    pub attached: Vec<PublicKey>,
    /// The sender's `Autocrypt:` header, when there is exactly one that reads and it is for
    /// the From address (Autocrypt Level 1 §2.3).
    pub autocrypt: Option<Autocrypt>,
}

/// How many keys are taken from one message's attachments.
const MAX_ATTACHED: usize = 16;
/// How many `Autocrypt:` headers of one message are looked at.
const MAX_AUTOCRYPT: usize = 8;

/// Reads the Public keys in the message `original`: attached key parts and the `Autocrypt:`
/// header. Nothing is fetched or sent; what can't be read is left out. All of it together costs
/// at most a fixed number of signature checks.
pub fn keys_in_message(original: &[u8]) -> Found {
    let mut found = Found::default();
    if original.len() > MAX_MESSAGE {
        return found;
    }
    let mut total = keys::Budget::new(keys::MAX_CHECKS_PER_CALL);
    let (headers, body) = mime::split(original);
    // One From mailbox, or none counts.
    let from = headers.get("from").map(mime::address).unwrap_or_default();
    // Level 1 §2.3: not in a report (a bounce quotes the message it is about), and more than
    // one header for the address is as good as none. The attributes are read first, and the
    // key, which costs, only of the one header that counts.
    if headers.content_type().value != "multipart/report" && !from.is_empty() {
        let mut headers_for_from =
            headers.all("autocrypt").take(MAX_AUTOCRYPT).filter_map(|v| autocrypt::attributes(v).ok()).filter(|a| a.address == from);
        if let (Some(one), None) = (headers_for_from.next(), headers_for_from.next()) {
            found.autocrypt = one.read(&mut total).ok();
        }
    }
    let mut parts = MAX_PARTS;
    attached(&headers, body, 0, &mut parts, &mut total, &mut found.attached);
    found
}

fn attached(headers: &mime::Headers, body: &[u8], depth: usize, parts: &mut usize, total: &mut keys::Budget, out: &mut Vec<PublicKey>) {
    if *parts == 0 || out.len() >= MAX_ATTACHED {
        return;
    }
    *parts = parts.saturating_sub(1);
    let content_type = headers.content_type();
    let media = content_type.value.as_str();
    if media == "application/pgp-keys" {
        let bytes = mime::decode(&headers.encoding(), body);
        if let Ok(keys) = PublicKey::parse_with(&bytes, total) {
            let room = MAX_ATTACHED.saturating_sub(out.len());
            out.extend(keys.into_iter().take(room));
        }
        return;
    }
    if media.starts_with("multipart/") && depth < MAX_DEPTH {
        let Some(boundary) = content_type.get("boundary").filter(|b| !b.is_empty()) else { return };
        for part in mime::parts(body, boundary, *parts) {
            let (part_headers, part_body) = mime::split(part);
            attached(&part_headers, part_body, depth.saturating_add(1), parts, total, out);
        }
    }
}
