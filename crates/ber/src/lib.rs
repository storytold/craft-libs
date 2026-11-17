// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Reading the TLVs of ASN.1 BER and DER (X.690), and turning BER into DER where they differ in
//! what CMS writers use: indefinite lengths (which streaming signers write) and OCTET STRINGs in
//! pieces (constructed). What is DER already is kept byte for byte, an element with a definite
//! length inside an indefinite one too, so that the bytes a signature covers (signed attributes,
//! a certificate's body) are those the signer wrote. For S/MIME: `craft-smime` checks
//! signatures with it, and a message's reader takes out what an opaque signed one holds
//! ([`signed_content`]).
//!
//! The input is hostile: lengths are checked against what is there, nesting is bounded
//! ([`MAX_DEPTH`]), and the work is linear in the input (times the bounded depth).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

/// CMS's id-signedData and id-data (RFC 5652 §5.1, §4), as OBJECT IDENTIFIER values.
const SIGNED_DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x02];
const DATA: &[u8] = &[0x2a, 0x86, 0x48, 0x86, 0xf7, 0x0d, 0x01, 0x07, 0x01];

/// What the CMS `cms` holds when it is SignedData with its content inside, of type id-data: the
/// MIME entity an opaque signed message (`application/pkcs7-mime`, RFC 8551 §3.5) signed, for
/// reading as the message. Nothing is checked here (`craft-smime` checks it); `None` for any
/// other CMS (an Encrypted message's enveloped data) or bytes that aren't one.
pub fn signed_content(cms: &[u8]) -> Option<Vec<u8>> {
    let der = to_der(cms)?;
    let (info, rest) = read(&der)?;
    if info.tag != 0x30 || !rest.is_empty() {
        return None;
    }
    let info = children(info.value)?;
    let [oid, explicit] = info.as_slice() else { return None };
    if oid.tag != 0x06 || oid.value != SIGNED_DATA || explicit.tag != 0xa0 {
        return None;
    }
    let (signed, rest) = read(explicit.value)?;
    if signed.tag != 0x30 || !rest.is_empty() {
        return None;
    }
    let signed = children(signed.value)?;
    let [version, digests, encap, tail @ ..] = signed.as_slice() else { return None };
    if version.tag != 0x02 || !matches!(version.value, [1 | 3 | 4 | 5]) || digests.tag != 0x31 || encap.tag != 0x30 {
        return None;
    }
    if children(digests.value)?.iter().any(|algorithm| algorithm.tag != 0x30) {
        return None;
    }
    // Certificates and revocation lists are optional, but ordered and present at most once.
    let tail = if tail.first().is_some_and(|field| field.tag == 0xa0) { tail.get(1..)? } else { tail };
    let tail = if tail.first().is_some_and(|field| field.tag == 0xa1) { tail.get(1..)? } else { tail };
    let [signers] = tail else { return None };
    if signers.tag != 0x31 || children(signers.value)?.iter().any(|signer| signer.tag != 0x30) {
        return None;
    }
    let encap = children(encap.value)?;
    let [kind, content] = encap.as_slice() else { return None };
    if kind.tag != 0x06 || kind.value != DATA || content.tag != 0xa0 {
        return None;
    }
    let (octets, rest) = read(content.value)?;
    (octets.tag == 0x04 && rest.is_empty()).then(|| octets.value.to_vec())
}

/// How deep elements may nest.
pub const MAX_DEPTH: usize = 48;

/// Maximum elements materialized or indexed in one parsing operation. This bounds the
/// allocation amplification from hostile, shallow input containing many empty TLVs.
pub const MAX_ELEMENTS: usize = 65_536;

/// The universal tag of an OCTET STRING, and of one in pieces.
const OCTET_STRING: u8 = 0x04;
const OCTET_STRING_PIECES: u8 = 0x24;

/// One element: its tag bytes, and its value (the content octets).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Tlv<'a> {
    /// The tag's first byte (class, constructed bit, number below 31).
    pub tag: u8,
    /// The whole element, tag and length included.
    pub whole: &'a [u8],
    /// Its content octets.
    pub value: &'a [u8],
}

/// The tag bytes at the start of `bytes`: one, or a high tag number in up to four more.
fn tag_len(bytes: &[u8]) -> Option<usize> {
    let first = *bytes.first()?;
    if first & 0x1f != 0x1f {
        return Some(1);
    }
    // High tag number: base-128 bytes, the last without its top bit.
    for (i, b) in bytes.iter().enumerate().skip(1).take(4) {
        if b & 0x80 == 0 {
            return i.checked_add(1);
        }
    }
    None
}

/// The length at the start of `bytes`: `(length, bytes it took)`, `None` for an indefinite one
/// (`Some((None, 1))`) is told apart by the caller.
fn length(bytes: &[u8]) -> Option<(Option<usize>, usize)> {
    let first = *bytes.first()?;
    if first < 0x80 {
        return Some((Some(usize::from(first)), 1));
    }
    if first == 0x80 {
        return Some((None, 1));
    }
    let n = usize::from(first & 0x7f);
    if n > 4 {
        return None;
    }
    let digits = bytes.get(1..n.checked_add(1)?)?;
    let len = digits.iter().fold(0usize, |acc, d| (acc << 8) | usize::from(*d));
    Some((Some(len), n.checked_add(1)?))
}

/// The element at the start of `bytes` with a definite length, and what follows it. `None` for
/// an indefinite length, or one longer than what is there.
pub fn read(bytes: &[u8]) -> Option<(Tlv<'_>, &[u8])> {
    let t = tag_len(bytes)?;
    let tag = *bytes.first()?;
    let (len, l) = length(bytes.get(t..)?)?;
    let len = len?;
    let start = t.checked_add(l)?;
    let end = start.checked_add(len)?;
    let value = bytes.get(start..end)?;
    let whole = bytes.get(..end)?;
    Some((Tlv { tag, whole, value }, bytes.get(end..)?))
}

/// The elements of `value`, each with a definite length; `None` when one can't be read.
pub fn children(mut value: &[u8]) -> Option<Vec<Tlv<'_>>> {
    let mut out = Vec::new();
    while !value.is_empty() {
        if out.len() >= MAX_ELEMENTS {
            return None;
        }
        let (tlv, rest) = read(value)?;
        out.push(tlv);
        value = rest;
    }
    Some(out)
}

/// `bytes` as DER as far as CMS needs it: indefinite lengths made definite and OCTET STRINGs in
/// pieces joined into one, everything else as it was. One element is read, and must be all of
/// `bytes` (bar trailing zero bytes, which some writers pad with). `None` when it isn't BER.
pub fn to_der(bytes: &[u8]) -> Option<Vec<u8>> {
    if let Some((_, rest)) = read(bytes)
        && rest.iter().all(|b| *b == 0)
        && der_shaped(bytes.get(..bytes.len().saturating_sub(rest.len()))?, 0)
    {
        return bytes.get(..bytes.len().saturating_sub(rest.len())).map(<[u8]>::to_vec);
    }
    let mut out = Vec::with_capacity(bytes.len());
    let used = element(bytes, 0, &mut out)?;
    let rest = bytes.get(used..)?;
    rest.iter().all(|b| *b == 0).then_some(out)
}

/// Whether the elements of `bytes` all have definite lengths and no OCTET STRING is in pieces,
/// as deep as they go: nothing to change.
fn der_shaped(mut bytes: &[u8], depth: usize) -> bool {
    if depth > MAX_DEPTH {
        return false;
    }
    while !bytes.is_empty() {
        let Some((tlv, rest)) = read(bytes) else { return false };
        if tlv.tag == OCTET_STRING_PIECES || (tlv.tag & 0x20 != 0 && !der_shaped(tlv.value, depth.saturating_add(1))) {
            return false;
        }
        bytes = rest;
    }
    true
}

/// What [`measure`] found of one element: how many bytes of the input it was, and how long its
/// content is as DER.
#[derive(Clone, Copy)]
struct Size {
    used: usize,
    content: usize,
    /// It is DER-shaped already (a definite length, and nothing in it to change): copied as it
    /// is, its lengths as the writer wrote them, since a signature may cover them.
    verbatim: bool,
}

/// The sizes of the constructed elements of the input, by where each starts in it: worked out
/// once, so that writing them is one pass with no copy of a level for each level above it.
type Sizes = std::collections::HashMap<usize, Size>;

/// The element at `at` in `input`: how many bytes it was, and its length as DER, header and all.
fn measure(input: &[u8], at: usize, depth: usize, sizes: &mut Sizes) -> Option<(usize, usize)> {
    if depth > MAX_DEPTH || sizes.len() >= MAX_ELEMENTS {
        return None;
    }
    let bytes = input.get(at..)?;
    if let Some((tlv, _)) = read(bytes)
        && der_shaped(tlv.whole, depth)
    {
        sizes.insert(at, Size { used: tlv.whole.len(), content: tlv.value.len(), verbatim: true });
        return Some((tlv.whole.len(), tlv.whole.len()));
    }
    let t = tag_len(bytes)?;
    let tag = *bytes.first()?;
    let (len, l) = length(bytes.get(t..)?)?;
    let start = t.checked_add(l)?;
    if tag & 0x20 == 0 {
        let used = start.checked_add(len?)?;
        bytes.get(..used)?;
        return Some((used, used_der(t, used.checked_sub(start)?)?));
    }
    // The children, one after another, to the end of a definite length or to the end mark.
    let mut content = 0usize;
    let mut pos = start;
    let end = match len {
        Some(len) => Some(start.checked_add(len)?),
        None => None,
    };
    loop {
        match end {
            Some(end) if pos >= end => break,
            None if bytes.get(pos..)?.starts_with(&[0, 0]) => {
                pos = pos.checked_add(2)?;
                break;
            }
            _ => {}
        }
        // (The pieces of an OCTET STRING are OCTET STRINGs, in pieces themselves or not.)
        if tag == OCTET_STRING_PIECES && !matches!(bytes.get(pos), Some(&OCTET_STRING | &OCTET_STRING_PIECES)) {
            return None;
        }
        let (n, der) = measure(input, at.checked_add(pos)?, depth.checked_add(1)?, sizes)?;
        if n == 0 {
            return None;
        }
        // An OCTET STRING in pieces is one of their contents together.
        content = content.checked_add(if tag == OCTET_STRING_PIECES { der_content(input, at.checked_add(pos)?, sizes)? } else { der })?;
        pos = pos.checked_add(n)?;
    }
    if let Some(end) = end
        && pos != end
    {
        return None;
    }
    if sizes.len() >= MAX_ELEMENTS {
        return None;
    }
    sizes.insert(at, Size { used: pos, content, verbatim: false });
    let header_tag = if tag == OCTET_STRING_PIECES { 1 } else { t };
    Some((pos, used_der(header_tag, content)?))
}

/// The length of a DER element with a tag of `tag` bytes and `content` bytes of content.
fn used_der(tag: usize, content: usize) -> Option<usize> {
    let mut l = Vec::new();
    put_length(&mut l, content);
    tag.checked_add(l.len())?.checked_add(content)
}

/// The DER content length of the (measured) element at `at`: a primitive's own.
fn der_content(input: &[u8], at: usize, sizes: &Sizes) -> Option<usize> {
    if let Some(s) = sizes.get(&at) {
        return Some(s.content);
    }
    let bytes = input.get(at..)?;
    let t = tag_len(bytes)?;
    let (len, _) = length(bytes.get(t..)?)?;
    len
}

/// Write the (measured) element at `at` in `input` to `out` as DER; how many bytes it was.
fn write(input: &[u8], at: usize, sizes: &Sizes, pieces: bool, out: &mut Vec<u8>) -> Option<usize> {
    if let Some(s) = sizes.get(&at).filter(|s| s.verbatim && !pieces) {
        out.extend_from_slice(input.get(at..at.checked_add(s.used)?)?);
        return Some(s.used);
    }
    let bytes = input.get(at..)?;
    let t = tag_len(bytes)?;
    let tag_bytes = bytes.get(..t)?;
    let tag = *tag_bytes.first()?;
    let (len, l) = length(bytes.get(t..)?)?;
    let start = t.checked_add(l)?;
    if tag & 0x20 == 0 {
        let used = start.checked_add(len?)?;
        let value = bytes.get(start..used)?;
        // (A piece of an OCTET STRING in pieces gives its bytes alone.)
        if !pieces {
            out.extend_from_slice(tag_bytes);
            put_length(out, value.len());
        }
        out.extend_from_slice(value);
        return Some(used);
    }
    let size = *sizes.get(&at)?;
    if tag == OCTET_STRING_PIECES {
        if !pieces {
            out.push(OCTET_STRING);
            put_length(out, size.content);
        }
    } else {
        out.extend_from_slice(tag_bytes);
        put_length(out, size.content);
    }
    let inner_pieces = tag == OCTET_STRING_PIECES;
    let mut pos = start;
    while pos < size.used {
        if bytes.get(pos..)?.starts_with(&[0, 0]) && len.is_none() {
            break;
        }
        let n = write(input, at.checked_add(pos)?, sizes, inner_pieces, out)?;
        pos = pos.checked_add(n)?;
    }
    Some(size.used)
}

/// Write the element at the start of `bytes` to `out` as DER; how many bytes of `bytes` it was.
fn element(bytes: &[u8], depth: usize, out: &mut Vec<u8>) -> Option<usize> {
    let mut sizes = Sizes::new();
    let (used, der) = measure(bytes, 0, depth, &mut sizes)?;
    out.reserve(der);
    write(bytes, 0, &sizes, false, out)?;
    Some(used)
}

/// A definite length in DER's shortest form.
fn put_length(out: &mut Vec<u8>, len: usize) {
    if len < 0x80 {
        out.push(len as u8);
        return;
    }
    let bytes = len.to_be_bytes();
    let skip = bytes.iter().take_while(|b| **b == 0).count();
    let digits = bytes.get(skip..).unwrap_or_default();
    out.push(0x80 | digits.len() as u8);
    out.extend_from_slice(digits);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tlv(tag: u8, value: &[u8]) -> Vec<u8> {
        let mut out = vec![tag];
        put_length(&mut out, value.len());
        out.extend_from_slice(value);
        out
    }

    fn cms(outer_tag: u8, signed_tag: u8, oid_tag: u8, long_oid: bool, trailing: bool) -> Vec<u8> {
        let oid = |value: &[u8]| {
            if long_oid {
                let mut out = vec![oid_tag, 0x81, 9];
                out.extend_from_slice(value);
                out
            } else {
                tlv(oid_tag, value)
            }
        };
        let mut encap = oid(DATA);
        encap.extend(tlv(0xa0, &tlv(0x04, b"hello")));
        let mut signed = tlv(0x02, &[1]);
        signed.extend(tlv(0x31, &[]));
        signed.extend(tlv(0x30, &encap));
        signed.extend(tlv(0x31, &[]));
        let mut signed = tlv(signed_tag, &signed);
        if trailing {
            signed.extend(tlv(0x05, &[]));
        }
        let mut info = oid(SIGNED_DATA);
        info.extend(tlv(0xa0, &signed));
        tlv(outer_tag, &info)
    }

    #[test]
    fn extracts_content_with_der_and_ber_oid_lengths() {
        for long_oid in [false, true] {
            assert_eq!(signed_content(&cms(0x30, 0x30, 0x06, long_oid, false)), Some(b"hello".to_vec()));
        }
    }

    #[test]
    fn rejects_wrong_cms_container_and_oid_tags() {
        for (outer, signed, oid) in [(0x31, 0x30, 0x06), (0x30, 0x31, 0x06), (0x30, 0x30, 0x04)] {
            assert_eq!(signed_content(&cms(outer, signed, oid, false, false)), None);
        }
    }

    #[test]
    fn rejects_missing_mandatory_signed_data_fields() {
        let mut encap = tlv(0x06, DATA);
        encap.extend(tlv(0xa0, &tlv(0x04, b"hello")));
        for prefix in [vec![0x05, 0, 0x05, 0], vec![0x02, 1, 1, 0x31, 0]] {
            let mut malformed = prefix;
            malformed.extend(tlv(0x30, &encap));
            let mut info = tlv(0x06, SIGNED_DATA);
            info.extend(tlv(0xa0, &tlv(0x30, &malformed)));
            assert_eq!(signed_content(&tlv(0x30, &info)), None);
        }
    }

    #[test]
    fn rejects_extra_signed_data_inside_explicit_container() {
        assert_eq!(signed_content(&cms(0x30, 0x30, 0x06, false, true)), None);
    }

    #[test]
    fn rejects_wide_inputs_before_unbounded_metadata_allocation() {
        let children_bytes = [0x05, 0].repeat(MAX_ELEMENTS + 1);
        assert!(children(&children_bytes).is_none());
        let mut indefinite = vec![0x30, 0x80];
        indefinite.extend_from_slice(&children_bytes);
        indefinite.extend_from_slice(&[0, 0]);
        assert!(to_der(&indefinite).is_none());
    }

    #[test]
    fn joins_indefinite_octet_string_pieces() {
        assert_eq!(to_der(&[0x24, 0x80, 0x04, 1, b'a', 0x04, 1, b'b', 0, 0]), Some(vec![0x04, 2, b'a', b'b']));
    }

    #[test]
    fn rejects_truncated_and_over_nested_input() {
        assert_eq!(to_der(&[0x04, 2, b'a']), None);
        let mut nested = tlv(0x04, &[]);
        for _ in 0..=MAX_DEPTH {
            nested = tlv(0x30, &nested);
        }
        assert_eq!(to_der(&nested), None);
    }
}
