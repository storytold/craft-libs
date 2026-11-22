//! Legacy Vietnamese 8-bit encoding detection and transcoding to Unicode NFC.
//!
//! # Source Attribution & Standard References
//! The character mapping tables and heuristic classification logic in this module
//! follow published national standards and de facto conventions, described from the
//! public specifications below. No third-party implementation source is copied.
//!
//! 1. **TCVN3 (ABC) Encoding:**
//!    - **TCVN 5712:1993**: *Information technology – 8-bit coded character set for Vietnamese*.
//!      Standardized by the Directorate for Standards, Metrology and Quality (STAMEQ),
//!      Ministry of Science, Technology and Environment of Vietnam (1993).
//!    - Variant implementation: TCVN 5712-3 (commonly known as TCVN3 / ABC lower & upper font pairs).
//!
//! 2. **VNI-Windows Encoding (2-byte composite & single-byte):**
//!    - **De facto Industry Convention**: Established by VNI Software Company
//!      (founded by Hồ Thành Việt, c. 1991–1995) for Vietnamese typography on Microsoft
//!      Windows 3.x/95 and Autodesk AutoCAD in Vietnam.
//!    - Encoding characteristics: Pairing base ASCII Latin vowels with high-byte (0x80..=0xFF)
//!      Latin-1 accent codes, with specific single-byte assignments for complex composite vowels.
//!
//! 3. **Windows-1258 and Unicode:** Microsoft code page 1258 (`CP1258.TXT`, Unicode Consortium
//!    mapping) and Unicode canonical composition (UAX #15) for the NFC composer.
//!
//! # Target Representation
//! All transcoding pipelines in this module deterministically emit standard
//! precomposed Unicode (NFC), adhering to Vietnamese National Standard TCVN 6909:2001
//! (*Information technology – 16-bit Vietnamese coded character set*).

use std::borrow::Cow;

pub mod detect;
pub mod tcvn3;
pub mod vni;
pub mod windows1258;

#[cfg(test)]
mod tests;

pub use detect::{
    DetectionResult, EncodingEvidence, EncodingMetadata, detect_encoding, detect_encoding_from_str, detect_vietnamese_encoding,
    is_already_unicode_vietnamese, is_vietnamese_font,
};
pub use tcvn3::{WINDOWS_1252_PUNCTUATION, decode_tcvn3, transcode_tcvn3, transcode_tcvn3_h};
pub use vni::{VniDecoder, decode_vni, transcode_vni};
pub use windows1258::{decode_windows1258, transcode_windows1258};

/// Decoding mode: Strict specification adherence vs Practical real-world legacy document ingestion.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DecodeMode {
    /// Strict specification mode: only canonical standard bytes are mapped.
    /// Unofficial byte positions (such as CP1252 punctuation in TCVN3/VNI) return `DecodeError::UnmappedByte`.
    Strict,
    /// Practical mode (default): enables CP1252 punctuation fallback (0x80..=0x9F), VNI CapsLock handling,
    /// and standalone/orphan diacritic fallback to preserve real-world legacy documents (Word, Excel, AutoCAD).
    #[default]
    Practical,
}

/// Detected or specified legacy Vietnamese encoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LegacyEncoding {
    /// TCVN3 (ABC) single-byte lowercase / composite uppercase encoding.
    Tcvn3,
    /// TCVN3H all-uppercase font encoding (where lowercase byte slots display capital letters).
    Tcvn3H,
    /// VNI Windows composite and extended byte encoding.
    Vni,
    /// Windows-1258 (CP1258) Vietnamese code page with combining diacritics.
    Windows1258,
}

/// Backwards-compatible alias matching CADCraft's `VietnameseEncoding`.
pub type VietnameseEncoding = LegacyEncoding;

impl LegacyEncoding {
    /// Returns a human-readable display name for logs, status bars, and audit summaries.
    pub const fn display_name(&self) -> &'static str {
        match self {
            Self::Tcvn3 => "TCVN3 (ABC)",
            Self::Tcvn3H => "TCVN3 All-Caps",
            Self::Vni => "VNI-Windows",
            Self::Windows1258 => "Windows-1258",
        }
    }
}

/// Errors that can occur during 8-bit legacy decoding.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DecodeError {
    /// Invalid UTF-8 sequence encountered at byte offset.
    InvalidUtf8 {
        /// Byte offset in the input where invalid UTF-8 sequence begins.
        offset: usize,
    },
    /// Unmapped byte in legacy encoding table encountered at byte offset.
    UnmappedByte {
        /// Raw byte value that could not be mapped.
        byte: u8,
        /// Byte offset in the input where unmapped byte occurred.
        offset: usize,
        /// Legacy encoding table being used for decoding.
        encoding: LegacyEncoding,
    },
    /// Malformed composite byte sequence encountered at byte offset.
    MalformedSequence {
        /// Byte offset where malformed composite sequence began.
        offset: usize,
        /// Legacy encoding table being used for decoding.
        encoding: LegacyEncoding,
    },
}

impl std::fmt::Display for DecodeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidUtf8 { offset } => {
                write!(f, "invalid UTF-8 byte sequence encountered at byte offset {offset}")
            }
            Self::UnmappedByte { byte, offset, encoding } => {
                write!(f, "unmapped byte 0x{byte:02X} encountered at byte offset {offset} for encoding {:?}", encoding)
            }
            Self::MalformedSequence { offset, encoding } => {
                write!(f, "malformed composite byte sequence encountered at byte offset {offset} for encoding {:?}", encoding)
            }
        }
    }
}

impl std::error::Error for DecodeError {}

/// Result of a transcoding operation, preserving whether the text was modified and which encoding was used.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TranscodeResult<'a> {
    /// Transcoded or original text.
    pub text: Cow<'a, str>,
    /// Matched or detected legacy encoding, if any.
    pub encoding: Option<LegacyEncoding>,
    /// Whether transcoding actually modified the input text.
    pub was_transcoded: bool,
}

/// Return the first candidate in `candidates` (e.g. `&[&style.font, &style.name]`)
/// that matches a recognized legacy font naming pattern.
pub fn font_hint<'a>(candidates: &[&'a str]) -> Option<&'a str> {
    for &c in candidates {
        let trimmed = c.trim();
        if !trimmed.is_empty() && is_vietnamese_font(trimmed).is_some() {
            return Some(c);
        }
    }
    None
}

/// Heuristic detection or font hint inspection for legacy Vietnamese text.
#[inline]
pub fn detect_legacy_encoding(text: &str, font_hint: Option<&str>) -> Option<LegacyEncoding> {
    font_hint.and_then(is_vietnamese_font).or_else(|| detect_vietnamese_encoding(text))
}

/// Return true if `text` contains legacy-encoded characters that would be transcoded under `font_hint`.
#[inline]
pub fn is_legacy_text(text: &str, font_hint: Option<&str>) -> bool {
    transcode_vietnamese(text, font_hint) != text
}

/// Return a human-readable summary of the matched legacy encoding family.
pub fn transcode_summary(hint: Option<&str>) -> &'static str {
    match hint.and_then(is_vietnamese_font) {
        Some(LegacyEncoding::Vni) => "VNI Windows",
        Some(LegacyEncoding::Tcvn3 | LegacyEncoding::Tcvn3H) => "TCVN3",
        Some(LegacyEncoding::Windows1258) => "Windows-1258",
        None => "legacy 8-bit",
    }
}

#[inline]
fn transcode_str_inner<'a>(s: &'a str, font_hint: Option<&str>) -> TranscodeResult<'a> {
    let Some(encoding) = detect_legacy_encoding(s, font_hint) else {
        return TranscodeResult { text: Cow::Borrowed(s), encoding: None, was_transcoded: false };
    };

    let bypass_unicode_guard = match encoding {
        LegacyEncoding::Tcvn3 => tcvn3::should_bypass_unicode_guard_for_tcvn3(s, false),
        LegacyEncoding::Tcvn3H => tcvn3::should_bypass_unicode_guard_for_tcvn3(s, true),
        LegacyEncoding::Vni | LegacyEncoding::Windows1258 => false,
    };
    if (s.is_ascii() && encoding != LegacyEncoding::Tcvn3H) || (!bypass_unicode_guard && is_already_unicode_vietnamese(s)) {
        return TranscodeResult { text: Cow::Borrowed(s), encoding: Some(encoding), was_transcoded: false };
    }

    let text = match encoding {
        LegacyEncoding::Tcvn3 => tcvn3::transcode_tcvn3_unchecked(s),
        LegacyEncoding::Tcvn3H => tcvn3::transcode_tcvn3_h_unchecked(s),
        LegacyEncoding::Vni => vni::transcode_vni_unchecked(s),
        LegacyEncoding::Windows1258 => transcode_windows1258(s),
    };

    let was_transcoded = matches!(&text, Cow::Owned(_)) || (text.as_ref() != s);
    TranscodeResult { text, encoding: Some(encoding), was_transcoded }
}

/// Transcode legacy 8-bit Vietnamese text to Unicode NFC using an optional font hint or detection heuristics.
///
/// `font_hint` is advisory evidence: if `s` is already recognized as Unicode Vietnamese (without
/// unambiguous legacy markers), `s` is preserved unchanged; callers that know the exact source
/// encoding should call [`transcode_explicit`] instead.
///
/// Returns `Cow::Borrowed(s)` with zero allocations when `s` is unchanged (including pure ASCII
/// for non-`Tcvn3H` hints and non-Latin-1 inputs; a `Tcvn3H` hint allocates `Cow::Owned` when
/// uppercasing lowercase ASCII).
pub fn transcode_vietnamese<'a>(s: &'a str, font_hint: Option<&str>) -> Cow<'a, str> {
    transcode_str_inner(s, font_hint).text
}

/// Drop-in alias for [`transcode_vietnamese`] matching downstream caller conventions.
pub use transcode_vietnamese as transcode;

/// Pure decoding primitive: maps bytes strictly to Unicode codepoints without hidden normalization.
///
/// Uses [`DecodeMode::Practical`] by default to preserve typography and diacritics in legacy documents.
pub fn decode(input: &[u8], encoding: LegacyEncoding) -> Result<Cow<'_, str>, DecodeError> {
    decode_with_mode(input, encoding, DecodeMode::Practical)
}

/// Pure decoding primitive with explicit [`DecodeMode`] selection.
pub fn decode_with_mode(input: &[u8], encoding: LegacyEncoding, mode: DecodeMode) -> Result<Cow<'_, str>, DecodeError> {
    match encoding {
        LegacyEncoding::Windows1258 => decode_windows1258(input),
        LegacyEncoding::Tcvn3 => decode_tcvn3(input, false, mode),
        LegacyEncoding::Tcvn3H => decode_tcvn3(input, true, mode),
        LegacyEncoding::Vni => decode_vni(input, mode),
    }
}

/// Configuration options for the [`decode_text`] composite facade.
#[derive(Clone, Copy, Debug)]
pub struct DecodeOptions<'a> {
    /// Explicitly forced encoding, bypassing heuristic detection.
    pub force_encoding: Option<LegacyEncoding>,
    /// Metadata hints (e.g. font name, codepage) from document containers.
    pub metadata: Option<&'a EncodingMetadata<'a>>,
    /// Whether to canonically compose the output into Unicode NFC (default: true).
    pub normalize_nfc: bool,
    /// Decoding mode: Strict specification vs Practical legacy tolerance (default: Practical).
    pub mode: DecodeMode,
}

impl Default for DecodeOptions<'_> {
    fn default() -> Self {
        Self { force_encoding: None, metadata: None, normalize_nfc: true, mode: DecodeMode::Practical }
    }
}

/// Unified decoding facade combining detection, decoding, and canonical NFC normalization.
///
/// Invariants:
/// - If `force_encoding` is `None` and `input` is valid UTF-8, heuristic transcoding is never executed.
/// - If `normalize_nfc` is true, output is guaranteed canonical Unicode Normalization Form C.
/// - Returns `Cow::Borrowed` with zero allocations on pure ASCII or already-canonical UTF-8 inputs.
pub fn decode_text<'a>(input: &'a [u8], options: &DecodeOptions<'a>) -> Result<Cow<'a, str>, DecodeError> {
    // 1. Explicit forced encoding
    if let Some(enc) = options.force_encoding {
        let decoded = decode_with_mode(input, enc, options.mode)?;
        if options.normalize_nfc {
            return Ok(apply_nfc_cow_for(decoded, enc));
        }
        return Ok(decoded);
    }

    // 2. Fast path: check valid UTF-8
    // Genuine UTF-8: never apply heuristic or metadata legacy decoding to valid UTF-8
    // when force_encoding is None. This preserves modern UTF-8 text from accidental corruption.
    let utf8_err = match std::str::from_utf8(input) {
        Ok(s) => {
            if options.normalize_nfc {
                return Ok(normalize_nfc(s));
            }
            return Ok(Cow::Borrowed(s));
        }
        Err(e) => e,
    };

    // 3. Non-UTF8 bytes: detect encoding via content heuristics or metadata
    let detection = detect_encoding(input, options.metadata);
    if let Some(enc) = detection.encoding() {
        let decoded = decode_with_mode(input, enc, options.mode)?;
        if options.normalize_nfc {
            return Ok(apply_nfc_cow_for(decoded, enc));
        }
        return Ok(decoded);
    }

    // 4. Neither valid UTF-8 nor a recognized Vietnamese legacy encoding
    Err(DecodeError::InvalidUtf8 { offset: utf8_err.valid_up_to() })
}

/// Canonical Unicode Normalization Form C (NFC) helper preserving borrowed Cow where possible.
#[inline]
fn apply_nfc_cow_with<'a>(cow: Cow<'a, str>, normalizer: fn(&str) -> Cow<'_, str>) -> Cow<'a, str> {
    match cow {
        Cow::Borrowed(b) => match normalizer(b) {
            Cow::Borrowed(_) => Cow::Borrowed(b),
            Cow::Owned(composed) => Cow::Owned(composed),
        },
        Cow::Owned(o) => match normalizer(&o) {
            Cow::Borrowed(_) => Cow::Owned(o),
            Cow::Owned(composed) => Cow::Owned(composed),
        },
    }
}

#[inline]
pub(crate) fn apply_vietnamese_nfc_cow<'a>(cow: Cow<'a, str>) -> Cow<'a, str> {
    apply_nfc_cow_with(cow, normalize_nfc)
}

#[inline]
pub(crate) fn apply_cp1258_nfc_cow<'a>(cow: Cow<'a, str>) -> Cow<'a, str> {
    apply_nfc_cow_with(cow, normalize_cp1258_nfc)
}

#[inline]
fn apply_nfc_cow_for<'a>(cow: Cow<'a, str>, enc: LegacyEncoding) -> Cow<'a, str> {
    match enc {
        LegacyEncoding::Windows1258 => apply_cp1258_nfc_cow(cow),
        _ => apply_nfc_cow_with(cow, normalize_nfc),
    }
}

/// Transcode using an explicitly known encoding, skipping heuristic detection.
///
/// The output is normalized to Unicode NFC with the same policy as [`decode_text`]:
/// `Windows1258` uses the full UAX #15 pipeline in [`normalize_cp1258_nfc`], other
/// encodings use [`normalize_nfc`].
///
/// Use this in `pdfcraft` or strict pipelines when metadata (such as a known PDF font name)
/// guarantees the encoding, eliminating heuristic detection false positives.
pub fn transcode_explicit<'a>(text: &'a str, encoding: LegacyEncoding) -> Cow<'a, str> {
    let mut bytes = Vec::with_capacity(text.len());
    for ch in text.chars() {
        let code = ch as u32;
        if code > 0xFF {
            return Cow::Borrowed(text);
        }
        bytes.push(code as u8);
    }

    match decode_with_mode(&bytes, encoding, DecodeMode::Practical) {
        Ok(decoded) => {
            // Why: normalizing must depend on the encoding, not on the entry point.
            // Windows-1258 needs the full UAX #15 pipeline (decompose + CCC order +
            // starter-retaining recomposition), exactly as `decode_text` and
            // `transcode_windows1258` do, so `É` + `U+0323` here also yields `Ẹ\u{0301}`.
            let nfc = apply_nfc_cow_for(decoded, encoding);
            if nfc.as_ref() == text { Cow::Borrowed(text) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(text),
    }
}

/// Transcode legacy text to Unicode NFC and return detailed status.
pub fn transcode_to_unicode<'a>(s: &'a str, font_hint: Option<&str>) -> TranscodeResult<'a> {
    transcode_str_inner(s, font_hint)
}

/// One-shot transcode facade returning a tuple for lightweight callers.
///
/// Returns `(normalized_text, detected_encoding_if_any)`.
#[inline]
pub fn transcode_with_summary<'a>(text: &'a str, font_hint: Option<&str>) -> (Cow<'a, str>, Option<LegacyEncoding>) {
    let result = transcode_to_unicode(text, font_hint);
    (result.text, result.encoding)
}

/// Strict normalization and validation. Returns `Err` with the detected encoding display name
/// if a legacy Vietnamese encoding is detected in the text, preventing silent data corruption.
/// Returns `Ok` with the NFC-normalized text when no legacy encoding is detected. This does not
/// confirm that the text is Unicode. Legacy text that detection does not match, such as `B¶n`, is returned as `Ok`.
///
/// Use this in `pdfcraft` or strict pipelines to reject text that contains a detected legacy encoding.
pub fn validate_and_normalize(text: &str) -> Result<Cow<'_, str>, &'static str> {
    if let Some(enc) = detect_legacy_encoding(text, None) {
        return Err(enc.display_name());
    }
    Ok(normalize_nfc(text))
}

#[inline]
fn contains_ignore_ascii_case(haystack: &str, needle: &str) -> bool {
    !needle.is_empty() && haystack.as_bytes().windows(needle.len()).any(|w| w.eq_ignore_ascii_case(needle.as_bytes()))
}

/// Check whether a legacy font name suggests a Serif typeface (vs Sans-Serif).
#[inline]
pub fn is_serif_legacy_font(font: &str) -> bool {
    let trimmed = font.trim();
    contains_ignore_ascii_case(trimmed, "time")
        || contains_ignore_ascii_case(trimmed, "century")
        || contains_ignore_ascii_case(trimmed, "roman")
        || contains_ignore_ascii_case(trimmed, "book")
        || contains_ignore_ascii_case(trimmed, "cambria")
        || contains_ignore_ascii_case(trimmed, "palatin")
}

/// Map a combining mark in the Vietnamese / Windows-1258 repertoire to its
/// `(bucket_index, canonical_combining_class)` pair (`[202, 216, 220, 230]`).
#[inline]
const fn ccc_bucket(c: char) -> Option<(usize, u8)> {
    match c {
        '\u{0327}' => Some((0, 202)), // cedilla
        '\u{031B}' => Some((1, 216)), // horn
        '\u{0323}' => Some((2, 220)), // dot below
        '\u{0300}' | '\u{0301}' | '\u{0302}' | '\u{0303}' | '\u{0306}' | '\u{0308}' | '\u{0309}' | '\u{030A}' | '\u{030C}' => Some((3, 230)),
        _ => None,
    }
}

/// Canonical Combining Class (UAX #15) of every combining mark in the Vietnamese and
/// Windows-1258 repertoires. Starters (and characters outside this repertoire) return `0`.
#[cfg(test)]
#[inline]
const fn combining_class(c: char) -> u8 {
    match ccc_bucket(c) {
        Some((_, cls)) => cls,
        None => 0,
    }
}

/// Zero-dependency canonical pair composition for Vietnamese vowels and combining diacritics.
///
/// Covers all 132 precomposed Vietnamese vowel+diacritic combinations (structural marks
/// `\u{0302}`, `\u{0306}`, `\u{031B}` and tone marks `\u{0300}`, `\u{0301}`, `\u{0309}`,
/// `\u{0303}`, `\u{0323}`), including both CP1258 sequences and NFD canonical ordering
/// (where `U+0323` dot-below precedes circumflex/breve).
#[inline]
pub const fn compose_vietnamese_pair(base: char, mark: char) -> Option<char> {
    Some(match (base, mark) {
        // Structural marks (circumflex, breve, horn)
        ('a', '\u{0302}') => 'â',
        ('A', '\u{0302}') => 'Â',
        ('a', '\u{0306}') => 'ă',
        ('A', '\u{0306}') => 'Ă',
        ('e', '\u{0302}') => 'ê',
        ('E', '\u{0302}') => 'Ê',
        ('o', '\u{0302}') => 'ô',
        ('O', '\u{0302}') => 'Ô',
        ('o', '\u{031B}') => 'ơ',
        ('O', '\u{031B}') => 'Ơ',
        ('u', '\u{031B}') => 'ư',
        ('U', '\u{031B}') => 'Ư',
        // NFD canonical order: dot-below (ccc=220) before circumflex/breve/horn (ccc=230)
        ('ạ', '\u{0302}') => 'ậ',
        ('Ạ', '\u{0302}') => 'Ậ',
        ('ạ', '\u{0306}') => 'ặ',
        ('Ạ', '\u{0306}') => 'Ặ',
        ('ẹ', '\u{0302}') => 'ệ',
        ('Ẹ', '\u{0302}') => 'Ệ',
        ('ọ', '\u{0302}') => 'ộ',
        ('Ọ', '\u{0302}') => 'Ộ',
        ('ọ', '\u{031B}') => 'ợ',
        ('Ọ', '\u{031B}') => 'Ợ',
        ('ụ', '\u{031B}') => 'ự',
        ('Ụ', '\u{031B}') => 'Ự',
        // a / A
        ('a', '\u{0300}') => 'à',
        ('a', '\u{0301}') => 'á',
        ('a', '\u{0309}') => 'ả',
        ('a', '\u{0303}') => 'ã',
        ('a', '\u{0323}') => 'ạ',
        ('A', '\u{0300}') => 'À',
        ('A', '\u{0301}') => 'Á',
        ('A', '\u{0309}') => 'Ả',
        ('A', '\u{0303}') => 'Ã',
        ('A', '\u{0323}') => 'Ạ',
        // ă / Ă
        ('ă', '\u{0300}') => 'ằ',
        ('ă', '\u{0301}') => 'ắ',
        ('ă', '\u{0309}') => 'ẳ',
        ('ă', '\u{0303}') => 'ẵ',
        ('ă', '\u{0323}') => 'ặ',
        ('Ă', '\u{0300}') => 'Ằ',
        ('Ă', '\u{0301}') => 'Ắ',
        ('Ă', '\u{0309}') => 'Ẳ',
        ('Ă', '\u{0303}') => 'Ẵ',
        ('Ă', '\u{0323}') => 'Ặ',
        // â / Â
        ('â', '\u{0300}') => 'ầ',
        ('â', '\u{0301}') => 'ấ',
        ('â', '\u{0309}') => 'ẩ',
        ('â', '\u{0303}') => 'ẫ',
        ('â', '\u{0323}') => 'ậ',
        ('Â', '\u{0300}') => 'Ầ',
        ('Â', '\u{0301}') => 'Ấ',
        ('Â', '\u{0309}') => 'Ẩ',
        ('Â', '\u{0303}') => 'Ẫ',
        ('Â', '\u{0323}') => 'Ậ',
        // e / E
        ('e', '\u{0300}') => 'è',
        ('e', '\u{0301}') => 'é',
        ('e', '\u{0309}') => 'ẻ',
        ('e', '\u{0303}') => 'ẽ',
        ('e', '\u{0323}') => 'ẹ',
        ('E', '\u{0300}') => 'È',
        ('E', '\u{0301}') => 'É',
        ('E', '\u{0309}') => 'Ẻ',
        ('E', '\u{0303}') => 'Ẽ',
        ('E', '\u{0323}') => 'Ẹ',
        // ê / Ê
        ('ê', '\u{0300}') => 'ề',
        ('ê', '\u{0301}') => 'ế',
        ('ê', '\u{0309}') => 'ể',
        ('ê', '\u{0303}') => 'ễ',
        ('ê', '\u{0323}') => 'ệ',
        ('Ê', '\u{0300}') => 'Ề',
        ('Ê', '\u{0301}') => 'Ế',
        ('Ê', '\u{0309}') => 'Ể',
        ('Ê', '\u{0303}') => 'Ễ',
        ('Ê', '\u{0323}') => 'Ệ',
        // i / I
        ('i', '\u{0300}') => 'ì',
        ('i', '\u{0301}') => 'í',
        ('i', '\u{0309}') => 'ỉ',
        ('i', '\u{0303}') => 'ĩ',
        ('i', '\u{0323}') => 'ị',
        ('I', '\u{0300}') => 'Ì',
        ('I', '\u{0301}') => 'Í',
        ('I', '\u{0309}') => 'Ỉ',
        ('I', '\u{0303}') => 'Ĩ',
        ('I', '\u{0323}') => 'Ị',
        // o / O
        ('o', '\u{0300}') => 'ò',
        ('o', '\u{0301}') => 'ó',
        ('o', '\u{0309}') => 'ỏ',
        ('o', '\u{0303}') => 'õ',
        ('o', '\u{0323}') => 'ọ',
        ('O', '\u{0300}') => 'Ò',
        ('O', '\u{0301}') => 'Ó',
        ('O', '\u{0309}') => 'Ỏ',
        ('O', '\u{0303}') => 'Õ',
        ('O', '\u{0323}') => 'Ọ',
        // ô / Ô
        ('ô', '\u{0300}') => 'ồ',
        ('ô', '\u{0301}') => 'ố',
        ('ô', '\u{0309}') => 'ổ',
        ('ô', '\u{0303}') => 'ỗ',
        ('ô', '\u{0323}') => 'ộ',
        ('Ô', '\u{0300}') => 'Ồ',
        ('Ô', '\u{0301}') => 'Ố',
        ('Ô', '\u{0309}') => 'Ổ',
        ('Ô', '\u{0303}') => 'Ỗ',
        ('Ô', '\u{0323}') => 'Ộ',
        // ơ / Ơ
        ('ơ', '\u{0300}') => 'ờ',
        ('ơ', '\u{0301}') => 'ớ',
        ('ơ', '\u{0309}') => 'ở',
        ('ơ', '\u{0303}') => 'ỡ',
        ('ơ', '\u{0323}') => 'ợ',
        ('Ơ', '\u{0300}') => 'Ờ',
        ('Ơ', '\u{0301}') => 'Ớ',
        ('Ơ', '\u{0309}') => 'Ở',
        ('Ơ', '\u{0303}') => 'Ỡ',
        ('Ơ', '\u{0323}') => 'Ợ',
        // u / U
        ('u', '\u{0300}') => 'ù',
        ('u', '\u{0301}') => 'ú',
        ('u', '\u{0309}') => 'ủ',
        ('u', '\u{0303}') => 'ũ',
        ('u', '\u{0323}') => 'ụ',
        ('U', '\u{0300}') => 'Ù',
        ('U', '\u{0301}') => 'Ú',
        ('U', '\u{0309}') => 'Ủ',
        ('U', '\u{0303}') => 'Ũ',
        ('U', '\u{0323}') => 'Ụ',
        // ư / Ư
        ('ư', '\u{0300}') => 'ừ',
        ('ư', '\u{0301}') => 'ứ',
        ('ư', '\u{0309}') => 'ử',
        ('ư', '\u{0303}') => 'ữ',
        ('ư', '\u{0323}') => 'ự',
        ('Ư', '\u{0300}') => 'Ừ',
        ('Ư', '\u{0301}') => 'Ứ',
        ('Ư', '\u{0309}') => 'Ử',
        ('Ư', '\u{0303}') => 'Ữ',
        ('Ư', '\u{0323}') => 'Ự',
        // y / Y
        ('y', '\u{0300}') => 'ỳ',
        ('y', '\u{0301}') => 'ý',
        ('y', '\u{0309}') => 'ỷ',
        ('y', '\u{0303}') => 'ỹ',
        ('y', '\u{0323}') => 'ỵ',
        ('Y', '\u{0300}') => 'Ỳ',
        ('Y', '\u{0301}') => 'Ý',
        ('Y', '\u{0309}') => 'Ỷ',
        ('Y', '\u{0303}') => 'Ỹ',
        ('Y', '\u{0323}') => 'Ỵ',
        _ => return None,
    })
}

/// Zero-dependency canonical pair composition for the full Windows-1258 character repertoire.
///
/// Extends [`compose_vietnamese_pair`] with the 50 non-Vietnamese Latin base + CP1258 combining
/// mark (`U+0300`, `U+0301`, `U+0303`, `U+0323`) pairs that have precomposed Unicode NFC forms
/// (such as `n + \u{0303} -> ñ` and `ç + \u{0301} -> ḉ`).
#[inline]
pub const fn compose_cp1258_pair(base: char, mark: char) -> Option<char> {
    if let Some(ch) = compose_vietnamese_pair(base, mark) {
        return Some(ch);
    }
    Some(match (base, mark) {
        // U+0300 COMBINING GRAVE ACCENT
        ('N', '\u{0300}') => '\u{01F8}',        // Ǹ
        ('n', '\u{0300}') => '\u{01F9}',        // ǹ
        ('W', '\u{0300}') => '\u{1E80}',        // Ẁ
        ('w', '\u{0300}') => '\u{1E81}',        // ẁ
        ('\u{00A8}', '\u{0300}') => '\u{1FED}', // ῭ (Greek dialytika and varia)
        ('\u{00DC}', '\u{0300}') => '\u{01DB}', // Ǜ
        ('\u{00FC}', '\u{0300}') => '\u{01DC}', // ǜ
        // U+0301 COMBINING ACUTE ACCENT
        ('C', '\u{0301}') => '\u{0106}',        // Ć
        ('c', '\u{0301}') => '\u{0107}',        // ć
        ('\u{00C7}', '\u{0301}') => '\u{1E08}', // Ḉ
        ('\u{00E7}', '\u{0301}') => '\u{1E09}', // ḉ
        ('\u{00DC}', '\u{0301}') => '\u{01D7}', // Ǘ (Ü + U+0301)
        ('\u{00FC}', '\u{0301}') => '\u{01D8}', // ǘ (ü + U+0301, e.g. 0xFC 0xEC)
        ('\u{00A8}', '\u{0301}') => '\u{0385}', // ΅ (Greek dialytika tonos)
        ('\u{00C5}', '\u{0301}') => '\u{01FA}', // Ǻ
        ('\u{00E5}', '\u{0301}') => '\u{01FB}', // ǻ
        ('\u{00C6}', '\u{0301}') => '\u{01FC}', // Ǽ
        ('\u{00E6}', '\u{0301}') => '\u{01FD}', // ǽ
        ('\u{00CF}', '\u{0301}') => '\u{1E2E}', // Ḯ
        ('\u{00EF}', '\u{0301}') => '\u{1E2F}', // ḯ
        ('\u{00D5}', '\u{0301}') => '\u{1E4C}', // Ṍ
        ('\u{00F5}', '\u{0301}') => '\u{1E4D}', // ṍ
        ('\u{00D8}', '\u{0301}') => '\u{01FE}', // Ǿ
        ('\u{00F8}', '\u{0301}') => '\u{01FF}', // ǿ
        ('\u{0168}', '\u{0301}') => '\u{1E78}', // Ṹ
        ('\u{0169}', '\u{0301}') => '\u{1E79}', // ṹ
        ('G', '\u{0301}') => '\u{01F4}',        // Ǵ
        ('g', '\u{0301}') => '\u{01F5}',        // ǵ
        ('K', '\u{0301}') => '\u{1E30}',        // Ḱ
        ('k', '\u{0301}') => '\u{1E31}',        // ḱ
        ('L', '\u{0301}') => '\u{0139}',        // Ĺ
        ('l', '\u{0301}') => '\u{013A}',        // ĺ
        ('M', '\u{0301}') => '\u{1E3E}',        // Ḿ
        ('m', '\u{0301}') => '\u{1E3F}',        // ḿ
        ('N', '\u{0301}') => '\u{0143}',        // Ń
        ('n', '\u{0301}') => '\u{0144}',        // ń
        ('P', '\u{0301}') => '\u{1E54}',        // Ṕ
        ('p', '\u{0301}') => '\u{1E55}',        // ṕ
        ('R', '\u{0301}') => '\u{0154}',        // Ŕ
        ('r', '\u{0301}') => '\u{0155}',        // ŕ
        ('S', '\u{0301}') => '\u{015A}',        // Ś
        ('s', '\u{0301}') => '\u{015B}',        // ś
        ('W', '\u{0301}') => '\u{1E82}',        // Ẃ
        ('w', '\u{0301}') => '\u{1E83}',        // ẃ
        ('Z', '\u{0301}') => '\u{0179}',        // Ź
        ('z', '\u{0301}') => '\u{017A}',        // ź
        // U+0303 COMBINING TILDE
        ('N', '\u{0303}') => '\u{00D1}', // Ñ
        ('n', '\u{0303}') => '\u{00F1}', // ñ
        ('V', '\u{0303}') => '\u{1E7C}', // Ṽ
        ('v', '\u{0303}') => '\u{1E7D}', // ṽ
        // U+0323 COMBINING DOT BELOW
        ('B', '\u{0323}') => '\u{1E04}', // Ḅ
        ('b', '\u{0323}') => '\u{1E05}', // ḅ
        ('D', '\u{0323}') => '\u{1E0C}', // Ḍ
        ('d', '\u{0323}') => '\u{1E0D}', // ḍ
        ('H', '\u{0323}') => '\u{1E24}', // Ḥ
        ('h', '\u{0323}') => '\u{1E25}', // ḥ
        ('K', '\u{0323}') => '\u{1E32}', // Ḳ
        ('k', '\u{0323}') => '\u{1E33}', // ḳ
        ('L', '\u{0323}') => '\u{1E36}', // Ḷ
        ('l', '\u{0323}') => '\u{1E37}', // ḷ
        ('M', '\u{0323}') => '\u{1E42}', // Ṃ
        ('m', '\u{0323}') => '\u{1E43}', // ṃ
        ('N', '\u{0323}') => '\u{1E46}', // Ṇ
        ('n', '\u{0323}') => '\u{1E47}', // ṇ
        ('R', '\u{0323}') => '\u{1E5A}', // Ṛ
        ('r', '\u{0323}') => '\u{1E5B}', // ṛ
        ('S', '\u{0323}') => '\u{1E62}', // Ṣ
        ('s', '\u{0323}') => '\u{1E63}', // ṣ
        ('T', '\u{0323}') => '\u{1E6C}', // Ṭ
        ('t', '\u{0323}') => '\u{1E6D}', // ṭ
        ('V', '\u{0323}') => '\u{1E7E}', // Ṿ
        ('v', '\u{0323}') => '\u{1E7F}', // ṿ
        ('W', '\u{0323}') => '\u{1E88}', // Ẉ
        ('w', '\u{0323}') => '\u{1E89}', // ẉ
        ('Z', '\u{0323}') => '\u{1E92}', // Ẓ
        ('z', '\u{0323}') => '\u{1E93}', // ẓ
        // U+0302 COMBINING CIRCUMFLEX ACCENT (Latin-1 Î/Û; Vietnamese only uses â/ê/ô)
        ('I', '\u{0302}') => '\u{00CE}', // Î
        ('i', '\u{0302}') => '\u{00EE}', // î
        ('U', '\u{0302}') => '\u{00DB}', // Û
        ('u', '\u{0302}') => '\u{00FB}', // û
        // U+0308 COMBINING DIAERESIS
        ('A', '\u{0308}') => '\u{00C4}', // Ä
        ('a', '\u{0308}') => '\u{00E4}', // ä
        ('E', '\u{0308}') => '\u{00CB}', // Ë
        ('e', '\u{0308}') => '\u{00EB}', // ë
        ('I', '\u{0308}') => '\u{00CF}', // Ï
        ('i', '\u{0308}') => '\u{00EF}', // ï
        ('O', '\u{0308}') => '\u{00D6}', // Ö
        ('o', '\u{0308}') => '\u{00F6}', // ö
        ('U', '\u{0308}') => '\u{00DC}', // Ü
        ('u', '\u{0308}') => '\u{00FC}', // ü
        ('Y', '\u{0308}') => '\u{0178}', // Ÿ
        ('y', '\u{0308}') => '\u{00FF}', // ÿ
        // U+030A COMBINING RING ABOVE
        ('A', '\u{030A}') => '\u{00C5}', // Å
        ('a', '\u{030A}') => '\u{00E5}', // å
        // U+030C COMBINING CARON
        ('S', '\u{030C}') => '\u{0160}', // Š
        ('s', '\u{030C}') => '\u{0161}', // š
        ('Z', '\u{030C}') => '\u{017D}', // Ž
        ('z', '\u{030C}') => '\u{017E}', // ž
        // U+0327 COMBINING CEDILLA
        ('C', '\u{0327}') => '\u{00C7}', // Ç
        ('c', '\u{0327}') => '\u{00E7}', // ç
        _ => return None,
    })
}

/// Primary canonical decomposition (UAX #15) of one Windows-1258 repertoire character.
///
/// Every entry is a single-step decomposition into a starter and one combining mark, so
/// the normalizer never needs to recurse. Characters without a canonical decomposition
/// (`Đ`, `đ`, `Ø`, `Æ`, `ß`, ...) return `None`. Decomposing first is what makes
/// `É` + `U+0323` normalize like its canonically equivalent `E` + `U+0323` + `U+0301`.
const fn decompose_cp1258_char(c: char) -> Option<(char, char)> {
    Some(match c {
        'À' => ('A', '\u{0300}'),
        'Á' => ('A', '\u{0301}'),
        'Â' => ('A', '\u{0302}'),
        'Ã' => ('A', '\u{0303}'),
        'Ä' => ('A', '\u{0308}'),
        'Å' => ('A', '\u{030A}'),
        'Ă' => ('A', '\u{0306}'),
        'Ç' => ('C', '\u{0327}'),
        'È' => ('E', '\u{0300}'),
        'É' => ('E', '\u{0301}'),
        'Ê' => ('E', '\u{0302}'),
        'Ë' => ('E', '\u{0308}'),
        'Ì' => ('I', '\u{0300}'),
        'Í' => ('I', '\u{0301}'),
        'Î' => ('I', '\u{0302}'),
        'Ï' => ('I', '\u{0308}'),
        'Ñ' => ('N', '\u{0303}'),
        'Ò' => ('O', '\u{0300}'),
        'Ó' => ('O', '\u{0301}'),
        'Ô' => ('O', '\u{0302}'),
        'Õ' => ('O', '\u{0303}'),
        'Ö' => ('O', '\u{0308}'),
        'Ơ' => ('O', '\u{031B}'),
        'Ù' => ('U', '\u{0300}'),
        'Ú' => ('U', '\u{0301}'),
        'Û' => ('U', '\u{0302}'),
        'Ü' => ('U', '\u{0308}'),
        'Ư' => ('U', '\u{031B}'),
        'Ý' => ('Y', '\u{0301}'),
        'à' => ('a', '\u{0300}'),
        'á' => ('a', '\u{0301}'),
        'â' => ('a', '\u{0302}'),
        'ã' => ('a', '\u{0303}'),
        'ä' => ('a', '\u{0308}'),
        'å' => ('a', '\u{030A}'),
        'ă' => ('a', '\u{0306}'),
        'ç' => ('c', '\u{0327}'),
        'è' => ('e', '\u{0300}'),
        'é' => ('e', '\u{0301}'),
        'ê' => ('e', '\u{0302}'),
        'ë' => ('e', '\u{0308}'),
        'ì' => ('i', '\u{0300}'),
        'í' => ('i', '\u{0301}'),
        'î' => ('i', '\u{0302}'),
        'ï' => ('i', '\u{0308}'),
        'ñ' => ('n', '\u{0303}'),
        'ò' => ('o', '\u{0300}'),
        'ó' => ('o', '\u{0301}'),
        'ô' => ('o', '\u{0302}'),
        'õ' => ('o', '\u{0303}'),
        'ö' => ('o', '\u{0308}'),
        'ơ' => ('o', '\u{031B}'),
        'ù' => ('u', '\u{0300}'),
        'ú' => ('u', '\u{0301}'),
        'û' => ('u', '\u{0302}'),
        'ü' => ('u', '\u{0308}'),
        'ư' => ('u', '\u{031B}'),
        'ý' => ('y', '\u{0301}'),
        'ÿ' => ('y', '\u{0308}'),
        'Š' => ('S', '\u{030C}'),
        'š' => ('s', '\u{030C}'),
        'Ž' => ('Z', '\u{030C}'),
        'ž' => ('z', '\u{030C}'),
        'Ÿ' => ('Y', '\u{0308}'),
        _ => return None,
    })
}

/// Identity decomposition for strings already in precomposed Vietnamese form.
#[inline]
const fn keep_precomposed(_: char) -> Option<(char, char)> {
    None
}

/// Flush one combining-mark cluster (4 fixed CCC buckets `[202, 216, 220, 230]`) in strictly O(k) time.
fn flush_cluster(
    starter: &mut Option<char>,
    buckets: &mut [Vec<char>; 4],
    remaining: &mut Vec<char>,
    composer: fn(char, char) -> Option<char>,
    out: &mut String,
) {
    const CLASSES: [u8; 4] = [202, 216, 220, 230];
    let mut last_class = 0_u8;
    remaining.clear();

    for (bucket, class) in buckets.iter_mut().zip(CLASSES) {
        for mark in bucket.drain(..) {
            if let Some(st) = *starter
                && last_class < class
                && let Some(composed) = composer(st, mark)
            {
                *starter = Some(composed);
            } else {
                last_class = class;
                remaining.push(mark);
            }
        }
    }

    if let Some(st) = starter.take() {
        out.push(st);
    }
    for mark in remaining.drain(..) {
        out.push(mark);
    }
}

/// Canonical UAX #15 normalization (decompose, order by Canonical Combining Class,
/// recompose around the last starter) restricted to a closed character repertoire.
///
/// Runs in strictly O(n) time and O(n) worst-case space by streaming cluster-by-cluster
/// into 4 reusable CCC buckets (`[202, 216, 220, 230]`), preserving stability for marks
/// of equal class and lazily decomposing a starter only when followed by combining marks.
fn normalize_with<'a>(s: &'a str, decompose: fn(char) -> Option<(char, char)>, composer: fn(char, char) -> Option<char>) -> Cow<'a, str> {
    // Why: every precomposed character in the repertoire already has NFC_QC=Yes, so only an
    // explicit combining mark can force reordering or recomposition. `É` alone normalizes to
    // itself, while `É` + `U+0323` still takes the full pipeline below.
    if s.is_ascii() || !s.chars().any(|c| ccc_bucket(c).is_some()) {
        return Cow::Borrowed(s);
    }

    let mut out = String::with_capacity(s.len());
    let mut buckets: [Vec<char>; 4] = [Vec::new(), Vec::new(), Vec::new(), Vec::new()];
    let mut remaining: Vec<char> = Vec::new();
    let mut starter: Option<char> = None;
    let mut has_marks = false;

    for c in s.chars() {
        if let Some((bucket_idx, _)) = ccc_bucket(c) {
            if !has_marks {
                if let Some(st) = starter
                    && let Some((base, mark0)) = decompose(st)
                {
                    starter = Some(base);
                    if let Some((b0, _)) = ccc_bucket(mark0) {
                        buckets[b0].push(mark0);
                    }
                }
                has_marks = true;
            }
            buckets[bucket_idx].push(c);
        } else {
            if has_marks {
                flush_cluster(&mut starter, &mut buckets, &mut remaining, composer, &mut out);
                has_marks = false;
            } else if let Some(st) = starter.take() {
                out.push(st);
            }
            starter = Some(c);
        }
    }

    if has_marks {
        flush_cluster(&mut starter, &mut buckets, &mut remaining, composer, &mut out);
    } else if let Some(st) = starter {
        out.push(st);
    }

    if out == s { Cow::Borrowed(s) } else { Cow::Owned(out) }
}

/// Normalize a Vietnamese string to precomposed Unicode Normalization Form C (NFC)
/// using zero runtime dependencies.
///
/// Input is expected to be either precomposed or a decomposition over Vietnamese
/// base letters, so it is not canonically decomposed first; ordering and recomposition
/// still follow UAX #15.
///
/// Returns `Cow::Borrowed` with zero allocations when the string is ASCII or already composed.
pub fn normalize_nfc<'a>(s: &'a str) -> Cow<'a, str> {
    normalize_with(s, keep_precomposed, compose_vietnamese_pair)
}

/// Normalize a Windows-1258 decoded string to Unicode Normalization Form C (NFC),
/// composing both Vietnamese vowels and the non-Vietnamese CP1258 component+mark pairs.
///
/// Implements the three UAX #15 steps: every precomposed CP1258 character is
/// canonically decomposed (`decompose_cp1258_char`), marks are ordered by Canonical
/// Combining Class (so ccc 220 `U+0323` precedes ccc 230 `U+0300`/`U+0301`/`U+0303`/`U+0309`),
/// and the result is recomposed around the last starter.
///
/// Returns `Cow::Borrowed` with zero allocations when the string is ASCII or already composed.
pub fn normalize_cp1258_nfc<'a>(s: &'a str) -> Cow<'a, str> {
    normalize_with(s, decompose_cp1258_char, compose_cp1258_pair)
}
