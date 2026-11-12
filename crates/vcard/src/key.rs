//! `KEY`: an S/MIME certificate or an OpenPGP public key on a card, kept as bytes.
//!
//! Read in every form the versions have: vCard 2.1 (`KEY;X509;ENCODING=BASE64:`), 3.0
//! (`KEY;ENCODING=b;TYPE=PGP:`, RFC 2426) and 4.0 (`KEY:data:application/pgp-keys;base64,…` or
//! `KEY;MEDIATYPE=application/pgp-keys:https://…`, RFC 6350), and a key written as text (an
//! ASCII-armoured OpenPGP key, a PEM certificate).
//!
//! What a key is follows from its bytes, whatever the card calls it: a certificate is kept as DER
//! with the outline of an X.509 certificate (a PEM one is unwrapped), an OpenPGP key as the
//! packets of a public key, binary or armoured as it came. A card whose label disagrees with the
//! bytes, anything else, and above all secret key material (an armour or PEM label that says
//! PRIVATE or SECRET, an OpenPGP secret-key packet, a PKCS #8 or PKCS #12 structure) is dropped.
//! Nothing is checked beyond that outline, nothing is trusted, and nothing is fetched: an address
//! is kept as an address. Written as vCard 3.0.

use crate::{ContactKey, KeyKind};

use crate::lines::ContentLine;
use crate::text::{base64_bytes, quoted_printable, unescape};

/// The schemes an address of a key may have; any other address is dropped.
const SCHEMES: [&str; 5] = ["http", "https", "ldap", "ldaps", "openpgp4fpr"];
const PEM_BEGIN: &str = "-----BEGIN CERTIFICATE-----";
const PEM_END: &str = "-----END CERTIFICATE-----";
const PGP_BEGIN: &str = "-----BEGIN PGP PUBLIC KEY BLOCK-----";
const PGP_END: &str = "-----END PGP PUBLIC KEY BLOCK-----";

/// Most bytes a value may decode to before what it holds is worked out: a PEM certificate or
/// an armoured key is larger than the bytes it is kept as.
const MAX_DECODED: usize = ContactKey::MAX_BYTES.saturating_mul(2);
/// Most characters of a value read for a key: the most it takes to spell [`MAX_DECODED`] bytes
/// (three characters a byte, quoted-printable or percent-encoded), with room for a header.
const MAX_VALUE: usize = MAX_DECODED.saturating_mul(3).saturating_add(1024);

/// The kind a type or media type names: `X509`, `PGP`, `application/pkix-cert`,
/// `application/pgp-keys` and the spellings writers use for them.
fn kind_named(name: &str) -> Option<KeyKind> {
    match name.trim().to_ascii_lowercase().as_str() {
        "x509" | "x.509" | "x-x509" | "application/pkix-cert" | "application/x-x509-ca-cert" | "application/x-x509-user-cert" => Some(KeyKind::X509),
        "pgp" | "x-pgp" | "gpg" | "openpgp" | "application/pgp-keys" | "application/pgp" | "application/x-pgp-keys" => Some(KeyKind::OpenPgp),
        _ => None,
    }
}

/// The key of one `KEY` line; `None` for one that holds nothing kept (junk, secret key material,
/// a label the bytes don't bear out, a key too large, an address of another scheme).
pub(crate) fn read(line: &ContentLine) -> Option<ContactKey> {
    if line.value.len() > MAX_VALUE {
        return None;
    }
    let mut stated = line.types().iter().chain(line.param("MEDIATYPE").map(str::to_string).iter()).find_map(|t| kind_named(t));
    let bytes = match line.param("ENCODING").unwrap_or("").to_ascii_uppercase().as_str() {
        "B" | "BASE64" => base64_bytes(&line.value),
        "QUOTED-PRINTABLE" | "Q" => quoted_printable(&line.value),
        _ => {
            let value = unescape(&line.value);
            let value = value.trim();
            if starts_with_ci(value, "data:") {
                let (named, bytes) = data_uri(value.get(5..).unwrap_or(""))?;
                stated = named.or(stated);
                bytes
            } else if value.starts_with("-----BEGIN") {
                value.as_bytes().to_vec()
            } else if line.param("VALUE").is_some_and(|v| v.eq_ignore_ascii_case("text")) {
                // Text that is neither armour nor PEM is no key (a fingerprint, a note); it is
                // never taken for the bytes of one.
                return None;
            } else {
                return address(value, stated);
            }
        }
    };
    if bytes.len() > MAX_DECODED {
        return None;
    }
    key_of(&bytes, stated)
}

fn starts_with_ci(s: &str, start: &str) -> bool {
    s.get(..start.len()).is_some_and(|head| head.eq_ignore_ascii_case(start))
}

/// An address of a key, with what the card says is there.
fn address(value: &str, stated: Option<KeyKind>) -> Option<ContactKey> {
    let uri = checked_uri(value)?;
    let media_type = match stated {
        Some(KeyKind::X509) => ContactKey::X509_MEDIA_TYPE,
        Some(KeyKind::OpenPgp) => ContactKey::OPENPGP_MEDIA_TYPE,
        _ => "",
    };
    Some(ContactKey { kind: KeyKind::Uri, uri, media_type: media_type.into(), ..Default::default() })
}

/// The key `bytes` hold, however they were spelled: a certificate as DER, an OpenPGP public key as
/// it came. `None` for anything else, for secret key material, for bytes that are not the kind
/// the card `stated`, and for a key larger than is kept.
pub(crate) fn key_of(bytes: &[u8], stated: Option<KeyKind>) -> Option<ContactKey> {
    let text = std::str::from_utf8(bytes).ok().map(str::trim_start).filter(|t| t.starts_with("-----BEGIN"));
    let (kind, data) = match text {
        Some(t) if says_secret(t) => return None,
        Some(t) if t.starts_with(PEM_BEGIN) => {
            let body = t.get(PEM_BEGIN.len()..)?;
            let (body, _) = body.split_once(PEM_END)?;
            (KeyKind::X509, base64_bytes(body))
        }
        Some(t) if t.starts_with(PGP_BEGIN) => {
            // Up to the end of the armour; what follows it is not kept.
            let end = t.find(PGP_END)?.checked_add(PGP_END.len())?;
            let armour = t.get(..end)?;
            openpgp_public(&armour_body(armour)?).then_some(())?;
            (KeyKind::OpenPgp, armour.as_bytes().to_vec())
        }
        Some(_) => return None,
        None if is_certificate(bytes) => (KeyKind::X509, bytes.to_vec()),
        None if openpgp_public(bytes) => (KeyKind::OpenPgp, bytes.to_vec()),
        None => return None,
    };
    let fits = !data.is_empty() && data.len() <= ContactKey::MAX_BYTES;
    let agrees = stated.is_none_or(|s| s == kind);
    let whole = kind != KeyKind::X509 || is_certificate(&data);
    (fits && agrees && whole).then(|| ContactKey { kind, data, ..Default::default() })
}

/// Whether an armour or PEM label in the text says it holds secret key material
/// (`-----BEGIN PGP PRIVATE KEY BLOCK-----`, `-----BEGIN RSA PRIVATE KEY-----`,
/// `-----BEGIN ENCRYPTED PRIVATE KEY-----`, …), anywhere in it.
fn says_secret(text: &str) -> bool {
    text.lines().map(str::trim).filter(|l| l.starts_with("-----")).any(|l| {
        let label = l.to_ascii_uppercase();
        label.contains("PRIVATE") || label.contains("SECRET")
    })
}

/// The bytes of an ASCII armour (RFC 9580 section 6): after the header lines and the blank line
/// that ends them, up to the checksum or the end line. `None` when that isn't base64.
fn armour_body(armour: &str) -> Option<Vec<u8>> {
    let mut lines = armour.lines().skip(1).map(str::trim);
    let mut body = String::new();
    // Header lines (`Version: …`, `Version:GnuPG`) end at a blank line; an armour without headers
    // begins at once. (A colon is never base64.)
    let mut first = lines.next()?;
    if first.contains(':') {
        first = lines.by_ref().find(|l| l.is_empty())?;
    }
    for line in std::iter::once(first).chain(lines) {
        if line.starts_with('=') || line.starts_with("-----") {
            break;
        }
        body.push_str(line);
    }
    let compact = body.trim_end_matches('=');
    craft_mime::base64::STANDARD_NO_PAD.decode(compact).ok().filter(|b| !b.is_empty())
}

/// One DER element: its tag, its contents and what follows it. Only the definite lengths DER
/// allows, of at most four bytes.
fn element(bytes: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let tag = *bytes.first()?;
    let first = usize::from(*bytes.get(1)?);
    let (length, start) = if first < 0x80 {
        (first, 2usize)
    } else {
        let count = first.checked_sub(0x80).filter(|n| (1..=4).contains(n))?;
        let digits = bytes.get(2..2usize.checked_add(count)?)?;
        let length = digits.iter().try_fold(0usize, |n, d| n.checked_mul(256)?.checked_add(usize::from(*d)))?;
        (length, 2usize.checked_add(count)?)
    };
    let end = start.checked_add(length)?;
    Some((tag, bytes.get(start..end)?, bytes.get(end..)?))
}

/// Whether the bytes have the outline of an X.509 certificate (RFC 5280 section 4.1): one
/// SEQUENCE of the to-be-signed SEQUENCE (beginning with its version or serial number), the
/// signature algorithm's SEQUENCE and the signature's BIT STRING, and nothing after it. A private
/// key (PKCS #1, PKCS #8, encrypted or not) or a PKCS #12 file does not have it.
fn is_certificate(bytes: &[u8]) -> bool {
    let shape = || -> Option<()> {
        let (0x30, certificate, []) = element(bytes)? else { return None };
        let (0x30, signed, rest) = element(certificate)? else { return None };
        matches!(signed.first(), Some(0xa0 | 0x02)).then_some(())?;
        let (0x30, _, rest) = element(rest)? else { return None };
        let (0x03, _, []) = element(rest)? else { return None };
        Some(())
    };
    shape().is_some()
}

/// Whether the bytes are OpenPGP packets (RFC 9580 section 4.2) that make a public key: a
/// public-key packet first, every packet whole, and no secret-key or secret-subkey packet
/// anywhere.
fn openpgp_public(bytes: &[u8]) -> bool {
    let mut at = 0usize;
    let mut first = true;
    while at < bytes.len() {
        let Some((tag, next)) = packet(bytes, at) else { return false };
        if matches!(tag, 5 | 7) || (first && tag != 6) {
            return false;
        }
        first = false;
        at = next;
    }
    !first
}

/// The tag of the packet at `at` and where the next one begins; `None` for a header that isn't
/// one or a packet that runs past the end. A partial length (only data packets have one) is not a
/// key's.
fn packet(bytes: &[u8], at: usize) -> Option<(u8, usize)> {
    let header = *bytes.get(at)?;
    if header & 0x80 == 0 {
        return None;
    }
    let byte = |i: usize| bytes.get(at.checked_add(i)?).map(|b| usize::from(*b));
    let big_endian = |from: usize, count: usize| (from..from.checked_add(count)?).try_fold(0usize, |n, i| n.checked_mul(256)?.checked_add(byte(i)?));
    let (tag, length, header_len) = if header & 0x40 == 0 {
        // The old format: the tag in bits 5 to 2, the length's size in bits 1 and 0.
        let tag = (header >> 2) & 0x0f;
        match header & 0x03 {
            0 => (tag, byte(1)?, 2usize),
            1 => (tag, big_endian(1, 2)?, 3),
            2 => (tag, big_endian(1, 4)?, 5),
            // An indeterminate length, which only data packets may have: not a key's.
            _ => return None,
        }
    } else {
        let tag = header & 0x3f;
        match byte(1)? {
            n @ 0..=191 => (tag, n, 2usize),
            n @ 192..=223 => (tag, n.checked_sub(192)?.checked_mul(256)?.checked_add(byte(2)?)?.checked_add(192)?, 3),
            255 => (tag, big_endian(2, 4)?, 6),
            _ => return None,
        }
    };
    let next = at.checked_add(header_len)?.checked_add(length)?;
    (next <= bytes.len()).then_some((tag, next))
}

/// The kind its media type names and the bytes of a `data:` URI (after the scheme):
/// `application/pgp-keys;base64,mQIN…` or percent-encoded text.
fn data_uri(rest: &str) -> Option<(Option<KeyKind>, Vec<u8>)> {
    let (head, data) = rest.split_once(',')?;
    let mut parts = head.split(';');
    let named = parts.next().and_then(kind_named);
    let base64 = parts.any(|p| p.trim().eq_ignore_ascii_case("base64"));
    Some((named, if base64 { base64_bytes(data) } else { percent_decoded(data) }))
}

/// `%41` as `A`; a `%` without two hex digits after it is kept.
fn percent_decoded(s: &str) -> Vec<u8> {
    let bytes = s.as_bytes();
    let hex = |at: usize| bytes.get(at).and_then(|b| char::from(*b).to_digit(16)).and_then(|d| u8::try_from(d).ok());
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0usize;
    while let Some(&b) = bytes.get(i) {
        if b == b'%'
            && let (Some(high), Some(low)) = (hex(i.saturating_add(1)), hex(i.saturating_add(2)))
        {
            out.push(high.saturating_mul(16).saturating_add(low));
            i = i.saturating_add(3);
        } else {
            out.push(b);
            i = i.saturating_add(1);
        }
    }
    out
}

/// The address when it is one a key may be at: a known scheme, no white space, control or
/// format characters (a right-to-left override, a zero-width space) or backslashes, not too long.
pub(crate) fn checked_uri(value: &str) -> Option<String> {
    let (scheme, rest) = value.split_once(':')?;
    let known = SCHEMES.iter().any(|s| s.eq_ignore_ascii_case(scheme));
    // (A backslash would be read back as an escape.)
    let clean = !rest.is_empty() && !value.chars().any(|c| c.is_whitespace() || c.is_control() || crate::text::is_format(c) || c == '\\');
    (known && clean && value.len() <= ContactKey::MAX_URI).then(|| value.to_string())
}

/// The `KEY` line of a key, as vCard 3.0 writes it; `None` for a key that would not read back as
/// itself (an unknown kind, bytes that aren't a certificate or a public key, too many of them).
///
/// An address of a key is not written: vCard 3.0 (RFC 2426 section 3.7.2) lets `KEY` be binary
/// or text only, and an address written as text would be read elsewhere as the key itself. It
/// stays on the contact.
pub(crate) fn line(key: &ContactKey) -> Option<String> {
    let kind = match key.kind {
        KeyKind::X509 => "X509",
        KeyKind::OpenPgp => "PGP",
        KeyKind::Uri => return None,
    };
    // Only what the reader would keep as this very key: never secret material a mailbox file
    // may hold.
    (key_of(&key.data, Some(key.kind)).as_ref() == Some(key))
        .then(|| format!("KEY;TYPE={kind};ENCODING=b:{}", craft_mime::base64::STANDARD.encode(&key.data)))
}
