//! Which PDF/X level a file claims (ISO 15930), read from where each part puts the claim. A
//! claim only: nothing here checks conformance (`pdfx1a::check_pdfx1a`, feature `flatten`,
//! checks PDF/X-1a:2003).
//!
//! Carriers: the information dictionary's `GTS_PDFXVersion` (and, for the 2001 edition only,
//! `GTS_PDFXConformance`) through PDF/X-3:2003; the XMP property `GTS_PDFXVersion` in the
//! namespace `http://www.npes.org/pdfx/ns/id/` from PDF/X-4 on (ISO 15930-1 §5, 15930-4 §5,
//! 15930-7 §6.10 and §6.11, 15930-9 §6.11.3). The header and the catalog's `/Version` are never
//! read (15930-3 and 15930-4 §5). A `GTS_PDFX` output intent alone is not a claim: every part
//! requires one, but none identifies a file by it.
//!
//! Adapted from pdfcer (MIT, same author), `crates/pdfcer-core/src/prepress/pdfx.rs` at
//! f5525637, rewritten over craft-pdf's reader.

use craft_pdf::syntax::Pdf;
use craft_pdf::syntax::object::{Array, Dict, Name, Stream};

/// The PDF/X identification namespace (`pdfxid`).
const PDFXID_NS: &[u8] = b"http://www.npes.org/pdfx/ns/id/";
/// Longest claim value read; registered values are at most 15 characters.
const MAX_VALUE: usize = 64;
/// Most bytes of the XMP packet decoded and searched.
pub const MAX_XMP_SCAN: usize = 512 * 1024;
/// Most `/OutputIntents` entries inspected.
const MAX_INTENTS: usize = 64;
/// Longest output condition identifier kept, in characters.
const MAX_IDENTIFIER: usize = 256;
/// Most elements open at once; a packet nested deeper states nothing.
const MAX_DEPTH: usize = 256;
/// Longest character or entity reference read, `&` through `;`.
const MAX_REFERENCE: usize = 12;

/// Every registered `GTS_PDFXVersion` value (the CGATS GTS_ registry), plus the PDF/X-6 levels of
/// ISO 15930-9.
const KNOWN_LEVELS: &[&str] = &[
    "CGATS.12/1-1999",
    "PDF/X-1:2001",
    "PDF/X-1a:2001",
    "PDF/X-3:2002",
    "PDF/X-1a:2003",
    "PDF/X-2:2003",
    "PDF/X-3:2003",
    "PDF/X-4",
    "PDF/X-4p",
    "PDF/X-5g",
    "PDF/X-5n",
    "PDF/X-5pg",
    "PDF/X-6",
    "PDF/X-6p",
    "PDF/X-6n",
];

/// The PDF/X claim a file makes, from [`pdfx_claim`].
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PdfxClaim {
    /// The level claimed, verbatim: the XMP value when there is one, else the information
    /// dictionary's, with `PDF/X-1:2001` read as `PDF/X-1a:2001` when `GTS_PDFXConformance` says
    /// so. An unregistered value is passed through, never mapped to the nearest known level.
    pub level: String,
    /// XMP `pdfxid:GTS_PDFXVersion`.
    pub xmp_version: Option<String>,
    /// Information dictionary `/GTS_PDFXVersion`.
    pub info_version: Option<String>,
    /// Information dictionary `/GTS_PDFXConformance` (the 2001 edition's X-1 and X-1a split).
    pub info_conformance: Option<String>,
    /// [`pdfx_output_intents`] of the file.
    pub output_intents: Vec<String>,
}

impl PdfxClaim {
    /// Whether XMP and the information dictionary don't both state a version, or state the same
    /// one. From PDF/X-4 on they must match when both are present.
    #[must_use]
    pub fn carriers_agree(&self) -> bool {
        match (&self.xmp_version, &self.info_version) {
            (Some(x), Some(i)) => x == i,
            _ => true,
        }
    }

    /// Whether [`Self::level`] is a registered PDF/X level.
    #[must_use]
    pub fn is_recognised(&self) -> bool {
        KNOWN_LEVELS.contains(&self.level.as_str())
    }

    /// Whether the file has the `GTS_PDFX` output intent every PDF/X part requires.
    #[must_use]
    pub fn has_output_intent(&self) -> bool {
        !self.output_intents.is_empty()
    }
}

/// The PDF/X level `file` claims, or `None` when neither its XMP packet nor its information
/// dictionary states one. An XMP packet that doesn't decode, isn't UTF-8 or states the version
/// only past its first [`MAX_XMP_SCAN`] bytes is read as stating nothing; a value that isn't
/// short printable text is ignored.
#[must_use]
pub fn pdfx_claim(file: &Pdf) -> Option<PdfxClaim> {
    let catalog = catalog(file);
    let xmp_version = catalog.as_ref().and_then(xmp_version);
    let info = file.info();
    let info_text = |key: &[u8]| info.as_ref().and_then(|d| text_entry(d, key));
    let info_version = info_text(b"GTS_PDFXVersion");
    let info_conformance = info_text(b"GTS_PDFXConformance");
    let version = xmp_version.clone().or_else(|| info_version.clone())?;
    let level = match info_conformance.as_deref() {
        Some(c @ "PDF/X-1a:2001") if version == "PDF/X-1:2001" => c.to_owned(),
        _ => version,
    };
    Some(PdfxClaim { level, xmp_version, info_version, info_conformance, output_intents: catalog.as_ref().map(output_intents).unwrap_or_default() })
}

/// The `/OutputConditionIdentifier` of each `/OutputIntents` entry with `/S /GTS_PDFX` (empty
/// text where an entry has none) among the first 64 entries, whether or not the file claims a
/// PDF/X level. A caller that counts an intent alone as PDF/X asks this beside [`pdfx_claim`].
#[must_use]
pub fn pdfx_output_intents(file: &Pdf) -> Vec<String> {
    catalog(file).as_ref().map(output_intents).unwrap_or_default()
}

fn catalog(file: &Pdf) -> Option<Dict<'_>> {
    let xref = file.xref();
    xref.get::<Dict<'_>>(xref.root_id())
}

fn output_intents(catalog: &Dict<'_>) -> Vec<String> {
    let Some(intents) = catalog.get::<Array<'_>>(b"OutputIntents") else { return Vec::new() };
    intents
        .iter::<Dict<'_>>()
        .take(MAX_INTENTS)
        .filter(|oi| oi.get::<Name<'_>>(b"S").is_some_and(|s| s.as_ref() == b"GTS_PDFX"))
        .map(|oi| identifier(&oi))
        .collect()
}

/// The `/OutputConditionIdentifier` text, trimmed, at most [`MAX_IDENTIFIER`] characters. Not a
/// claim value: any text is kept.
fn identifier(intent: &Dict<'_>) -> String {
    intent
        .get::<craft_pdf::syntax::object::String<'_>>(b"OutputConditionIdentifier")
        .map(|s| text_string(s.as_bytes()).trim().chars().take(MAX_IDENTIFIER).collect())
        .unwrap_or_default()
}

fn text_entry(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    let s = dict.get::<craft_pdf::syntax::object::String<'_>>(key)?;
    claim_value(&text_string(s.as_bytes()))
}

/// A text string's characters (ISO 32000-2 §7.9.2.2): UTF-16BE or UTF-8 after a byte order
/// mark, else the bytes read as UTF-8. That matches PDFDocEncoding only for ASCII; other bytes
/// become replacement characters.
fn text_string(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => {
            let units = rest.as_chunks::<2>().0.iter().map(|c| u16::from_be_bytes(*c));
            char::decode_utf16(units).map(|c| c.unwrap_or(char::REPLACEMENT_CHARACTER)).collect()
        }
        [0xEF, 0xBB, 0xBF, rest @ ..] => String::from_utf8_lossy(rest).into_owned(),
        _ => String::from_utf8_lossy(bytes).into_owned(),
    }
}

/// `value` trimmed, when it is short printable ASCII.
fn claim_value(value: &str) -> Option<String> {
    let v = value.trim();
    let printable = v.bytes().all(|b| b.is_ascii_graphic() || b == b' ');
    (!v.is_empty() && v.len() <= MAX_VALUE && printable).then(|| v.to_owned())
}

fn xmp_version(catalog: &Dict<'_>) -> Option<String> {
    let bytes = catalog.get::<Stream<'_>>(b"Metadata")?.decoded_within(MAX_XMP_SCAN).ok()?;
    version_in_packet(bytes.get(..bytes.len().min(MAX_XMP_SCAN)).unwrap_or_default())
}

/// The prefixes one open element declares (empty for the default namespace), each with whether
/// it is bound to [`PDFXID_NS`].
type Scope<'p> = Vec<(&'p [u8], bool)>;

/// A start tag: its name, its attributes with their raw values, and whether it is an empty
/// element.
struct Tag<'p> {
    name: &'p [u8],
    attributes: Vec<(&'p [u8], &'p [u8])>,
    empty: bool,
}

enum Markup<'p> {
    Start(Tag<'p>),
    End,
    Other,
}

/// The first `GTS_PDFXVersion` value in `packet` whose prefix is bound to [`PDFXID_NS`] where it
/// occurs, in attribute form (`p:GTS_PDFXVersion="v"`) or element form
/// (`<p:GTS_PDFXVersion>v<`). One pass over the markup: a namespace declaration applies to its
/// element and that element's content only, and comments, CDATA sections, processing
/// instructions and declarations are skipped. A default namespace doesn't apply to attributes.
fn version_in_packet(packet: &[u8]) -> Option<String> {
    let mut scopes: Vec<Scope<'_>> = Vec::new();
    let mut at = 0usize;
    loop {
        let rest = packet.get(at..)?;
        let open = rest.iter().position(|b| *b == b'<')?;
        let (len, markup) = markup(rest.get(open.saturating_add(1)..)?)?;
        at = at.saturating_add(open).saturating_add(1).saturating_add(len);
        let tag = match markup {
            Markup::Start(tag) => tag,
            Markup::End => {
                scopes.pop();
                continue;
            }
            Markup::Other => continue,
        };
        scopes.push(tag.attributes.iter().filter_map(|(n, v)| declared_prefix(n).map(|p| (p, *v == PDFXID_NS))).collect());
        let attribute = tag.attributes.iter().filter(|(n, _)| is_version(&scopes, n, false)).find_map(|(_, v)| xml_value(v));
        if attribute.is_some() {
            return attribute;
        }
        if !tag.empty && is_version(&scopes, tag.name, true) {
            let text = packet.get(at..)?;
            if let Some(v) = xml_value(text.get(..text.iter().position(|b| *b == b'<')?)?) {
                return Some(v);
            }
        }
        if tag.empty {
            scopes.pop();
        }
        if scopes.len() > MAX_DEPTH {
            return None;
        }
    }
}

/// Whether `name` is `GTS_PDFXVersion` under a prefix that `scopes`, the innermost last, bind to
/// [`PDFXID_NS`]. Only an element name may use the default namespace.
fn is_version(scopes: &[Scope<'_>], name: &[u8], element: bool) -> bool {
    let (prefix, local) = match name.iter().rposition(|b| *b == b':') {
        Some(colon) => (name.get(..colon).unwrap_or_default(), name.get(colon.saturating_add(1)..).unwrap_or_default()),
        None => (&[][..], name),
    };
    if local != b"GTS_PDFXVersion" || (prefix.is_empty() && !element) {
        return false;
    }
    scopes.iter().rev().flat_map(|s| s.iter().rev()).find(|(p, _)| *p == prefix).is_some_and(|(_, bound)| *bound)
}

/// The prefix an `xmlns` or `xmlns:p` attribute name declares.
fn declared_prefix(name: &[u8]) -> Option<&[u8]> {
    match name.strip_prefix(b"xmlns")? {
        [] => Some(&[]),
        [b':', prefix @ ..] => Some(prefix),
        _ => None,
    }
}

/// The markup after a `<`: its length through its closing `>`, and what it is. `None` when it
/// doesn't close.
fn markup(rest: &[u8]) -> Option<(usize, Markup<'_>)> {
    let skip = |open: &[u8], close: &[u8]| {
        let body = rest.get(open.len()..)?;
        Some((open.len().saturating_add(find(body, close)?).saturating_add(close.len()), Markup::Other))
    };
    if rest.starts_with(b"!--") {
        return skip(b"!--", b"-->");
    }
    if rest.starts_with(b"![CDATA[") {
        return skip(b"![CDATA[", b"]]>");
    }
    if rest.starts_with(b"?") {
        return skip(b"?", b"?>");
    }
    if rest.starts_with(b"!") {
        return skip(b"!", b">");
    }
    let close = tag_end(rest)?;
    let len = close.saturating_add(1);
    let body = rest.get(..close)?;
    if body.starts_with(b"/") {
        return Some((len, Markup::End));
    }
    Some((len, start_tag(body).map_or(Markup::Other, Markup::Start)))
}

/// Where a tag's `>` is, outside quoted attribute values.
fn tag_end(rest: &[u8]) -> Option<usize> {
    let mut quote = None;
    rest.iter().position(|&b| match quote {
        Some(q) => {
            if b == q {
                quote = None;
            }
            false
        }
        None if b == b'"' || b == b'\'' => {
            quote = Some(b);
            false
        }
        None => b == b'>',
    })
}

/// A start tag's name and attributes, from the text between `<` and `>`. Attributes stop at the
/// first one that isn't `name="value"` or `name='value'`.
fn start_tag(body: &[u8]) -> Option<Tag<'_>> {
    let name_len = body.iter().take_while(|b| is_name_byte(**b)).count();
    let name = body.get(..name_len).filter(|n| !n.is_empty())?;
    let empty = body.trim_ascii_end().ends_with(b"/");
    let mut attributes = Vec::new();
    let mut rest = body.get(name_len..).unwrap_or_default();
    loop {
        let trimmed = rest.trim_ascii_start();
        let len = trimmed.iter().take_while(|b| is_name_byte(**b)).count();
        let (Some(attr), Some(after)) = (trimmed.get(..len).filter(|n| !n.is_empty()), trimmed.get(len..)) else { break };
        let Some(after) = after.trim_ascii_start().strip_prefix(b"=").map(<[u8]>::trim_ascii_start) else { break };
        let Some((&quote, tail)) = after.split_first().filter(|(q, _)| **q == b'"' || **q == b'\'') else { break };
        let Some(end) = tail.iter().position(|b| *b == quote) else { break };
        attributes.push((attr, tail.get(..end).unwrap_or_default()));
        rest = tail.get(end.saturating_add(1)..).unwrap_or_default();
    }
    Some(Tag { name, attributes, empty })
}

/// A claim value from raw XML text: UTF-8, with character references and the five predefined
/// entity references decoded.
fn xml_value(raw: &[u8]) -> Option<String> {
    claim_value(&decode_references(std::str::from_utf8(raw).ok()?))
}

/// `text` with `&#N;`, `&#xH;`, `&amp;`, `&lt;`, `&gt;`, `&quot;` and `&apos;` decoded. Anything
/// else after `&` is kept as written.
fn decode_references(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(amp) = rest.find('&') {
        out.push_str(rest.get(..amp).unwrap_or_default());
        let tail = rest.get(amp..).unwrap_or_default();
        let semi = tail.as_bytes().iter().take(MAX_REFERENCE).position(|b| *b == b';');
        let decoded = semi.and_then(|semi| Some((semi, reference(tail.get(1..semi)?)?)));
        let (keep, skip) = match decoded {
            Some((semi, c)) => (c, semi.saturating_add(1)),
            None => ('&', 1),
        };
        out.push(keep);
        rest = tail.get(skip..).unwrap_or_default();
    }
    out.push_str(rest);
    out
}

fn reference(name: &str) -> Option<char> {
    let code = match name {
        "amp" => return Some('&'),
        "lt" => return Some('<'),
        "gt" => return Some('>'),
        "quot" => return Some('"'),
        "apos" => return Some('\''),
        _ => name.strip_prefix('#')?,
    };
    let value = match code.strip_prefix('x') {
        Some(hex) if !hex.is_empty() && hex.bytes().all(|b| b.is_ascii_hexdigit()) => u32::from_str_radix(hex, 16).ok()?,
        None if !code.is_empty() && code.bytes().all(|b| b.is_ascii_digit()) => code.parse().ok()?,
        _ => return None,
    };
    char::from_u32(value)
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.' | b':') || b >= 0x80
}

fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    if needle.is_empty() {
        return None;
    }
    haystack.windows(needle.len()).position(|w| w == needle)
}

#[cfg(test)]
#[path = "pdfx_tests.rs"]
mod tests;
