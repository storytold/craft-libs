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
/// Most namespace declarations bound to [`PDFXID_NS`] tried in one packet.
const MAX_PREFIXES: usize = 32;
/// Most output intents read.
const MAX_INTENTS: usize = 64;
/// How far past a property or namespace name the reader looks for its value, so a packet of
/// unterminated names costs linear work, not quadratic.
const MAX_LOOKAHEAD: usize = 1024;

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
/// text where an entry has none), whether or not the file claims a PDF/X level; at most 64. A
/// caller that counts an intent alone as PDF/X asks this beside [`pdfx_claim`].
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
        .filter(|oi| oi.get::<Name<'_>>(b"S").is_some_and(|s| s.as_ref() == b"GTS_PDFX"))
        .take(MAX_INTENTS)
        .map(|oi| text_entry(&oi, b"OutputConditionIdentifier").unwrap_or_default())
        .collect()
}

fn text_entry(dict: &Dict<'_>, key: &[u8]) -> Option<String> {
    let s = dict.get::<craft_pdf::syntax::object::String<'_>>(key)?;
    claim_value(&text_string(s.as_bytes()))
}

/// A text string's characters (ISO 32000-2 §7.9.2.2): UTF-16BE or UTF-8 after a byte order
/// mark, else the bytes, which for the printable ASCII a claim allows are the same in
/// PDFDocEncoding.
fn text_string(bytes: &[u8]) -> String {
    match bytes {
        [0xFE, 0xFF, rest @ ..] => {
            let units = rest.chunks_exact(2).map(|c| c.iter().fold(0u16, |u, b| (u << 8) | u16::from(*b)));
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
    let window = bytes.get(..bytes.len().min(MAX_XMP_SCAN)).unwrap_or_default();
    namespace_prefixes(window, PDFXID_NS).into_iter().find_map(|p| xml_property(window, p, b"GTS_PDFXVersion"))
}

/// The prefixes `packet` binds to `uri` (`xmlns:p="uri"`), and an empty prefix for a
/// default-namespace declaration (`xmlns="uri"`).
fn namespace_prefixes<'p>(packet: &'p [u8], uri: &[u8]) -> Vec<&'p [u8]> {
    let mut out = Vec::new();
    let mut at = 0usize;
    while let Some(found) = find(packet.get(at..).unwrap_or_default(), b"xmlns") {
        let start = at.saturating_add(found).saturating_add(b"xmlns".len());
        at = start;
        let rest = packet.get(start..).unwrap_or_default();
        let (prefix, after) = match rest.split_first() {
            Some((b':', tail)) => {
                let len = tail.iter().take_while(|b| is_name_byte(**b)).count();
                (tail.get(..len).unwrap_or_default(), tail.get(len..))
            }
            _ => (&[][..], Some(rest)),
        };
        if after.and_then(attribute_value) == Some(uri) {
            out.push(prefix);
            if out.len() >= MAX_PREFIXES {
                break;
            }
        }
    }
    out
}

/// The first value of the property `prefix:local`, in attribute form (`prefix:local="v"`) or
/// element form (`<prefix:local>v<`). An empty prefix matches the element form only: a default
/// namespace doesn't apply to attributes.
fn xml_property(packet: &[u8], prefix: &[u8], local: &[u8]) -> Option<String> {
    let qname = if prefix.is_empty() { local.to_vec() } else { [prefix, b":", local].concat() };
    let mut at = 0usize;
    while let Some(found) = find(packet.get(at..)?, &qname) {
        let start = at.saturating_add(found);
        at = start.saturating_add(qname.len());
        let before = start.checked_sub(1).and_then(|i| packet.get(i)).copied();
        let rest = packet.get(at..)?;
        let value = match before {
            Some(b'<') => element_text(rest),
            Some(b' ' | b'\t' | b'\r' | b'\n') if !prefix.is_empty() => attribute_value(rest),
            _ => None,
        };
        if let Some(v) = value.and_then(|v| claim_value(std::str::from_utf8(v).ok()?)) {
            return Some(v);
        }
    }
    None
}

/// The text after a start tag's name, past any attributes to `>`, up to the next `<`. `None` for
/// an empty-element tag, or when the name only begins `rest`'s real name.
fn element_text(rest: &[u8]) -> Option<&[u8]> {
    let rest = lookahead(rest);
    let close = rest.iter().position(|b| *b == b'>')?;
    let tag = rest.get(..close)?;
    let is_name_end = tag.first().is_none_or(u8::is_ascii_whitespace);
    if !is_name_end || tag.last() == Some(&b'/') {
        return None;
    }
    let text = rest.get(close.saturating_add(1)..)?;
    text.get(..text.iter().position(|b| *b == b'<')?)
}

/// The quoted value after `=` at the start of `rest`, in either quote.
fn attribute_value(rest: &[u8]) -> Option<&[u8]> {
    let rest = lookahead(rest).trim_ascii_start().strip_prefix(b"=")?.trim_ascii_start();
    let (&quote, tail) = rest.split_first()?;
    if quote != b'"' && quote != b'\'' {
        return None;
    }
    tail.get(..tail.iter().position(|b| *b == quote)?)
}

fn lookahead(rest: &[u8]) -> &[u8] {
    rest.get(..rest.len().min(MAX_LOOKAHEAD)).unwrap_or(rest)
}

fn is_name_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b'.') || b >= 0x80
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
