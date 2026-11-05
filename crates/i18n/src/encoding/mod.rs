//! Legacy Vietnamese 8-bit encoding detection and transcoding to Unicode NFC.
//!
//! # Source Attribution & Standard References
//! The character mapping tables and heuristic classification logic in this module
//! are based on published public domain conventions, national standards, and
//! cross-verified open-source reference implementations:
//!
//! 1. **TCVN3 (ABC) Encoding:**
//!    - **TCVN 5712:1993**: *Information technology – 8-bit coded character set for Vietnamese*.
//!      Standardized by the Directorate for Standards, Metrology and Quality (STAMEQ),
//!      Ministry of Science, Technology and Environment of Vietnam (1993).
//!    - Variant implementation: TCVN 5712-3 (commonly known as TCVN3 / ABC lower & upper font pairs).
//!    - Cross-verified with GNU C Library (`glibc`) official charmap specification:
//!      `localedata/charmaps/TCVN5712-1`.
//!
//! 2. **VNI-Windows Encoding (2-byte composite & single-byte):**
//!    - **De facto Industry Convention**: Established by VNI Software Company
//!      (founded by Hồ Thành Việt, c. 1991–1995) for Vietnamese typography on Microsoft
//!      Windows 3.x/95 and Autodesk AutoCAD in Vietnam.
//!    - Encoding characteristics: Pairing base ASCII Latin vowels with high-byte (0x80..=0xFF)
//!      Latin-1 accent codes, with specific single-byte assignments for complex composite vowels.
//!    - Cross-verified with established open-source reference implementations:
//!      - UniKey core engine (`vnconv` / `vni.tab`) by Phạm Kim Long.
//!      - CPAN standard encoding tables (`Encode::Vietnamese` / `Encode::VN::VNI`).
//!
//! # Target Representation
//! All transcoding pipelines in this module deterministically emit standard
//! precomposed Unicode (NFC), adhering to Vietnamese National Standard TCVN 6909:2001
//! (*Information technology – 16-bit Vietnamese coded character set*).

use std::borrow::Cow;

use icu_normalizer::ComposingNormalizerBorrowed;

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
    InvalidUtf8 { offset: usize },
    /// Unmapped byte in legacy encoding table encountered at byte offset.
    UnmappedByte { byte: u8, offset: usize, encoding: LegacyEncoding },
    /// Malformed composite byte sequence encountered at byte offset.
    MalformedSequence { offset: usize, encoding: LegacyEncoding },
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

/// Transcode legacy 8-bit Vietnamese text to Unicode NFC using an optional font hint or detection heuristics.
///
/// If `font_hint` matches a recognized legacy font family (e.g. `.VnTime`, `VNI-Times`),
/// that encoding is applied directly. Otherwise, conservative heuristic detection is attempted.
/// Returns `Cow::Borrowed(s)` with zero allocations if the text is pure ASCII, already Unicode,
/// or not identified as legacy Vietnamese.
pub fn transcode_vietnamese<'a>(s: &'a str, font_hint: Option<&str>) -> Cow<'a, str> {
    let enc = detect_legacy_encoding(s, font_hint);

    let Some(enc) = enc else {
        return Cow::Borrowed(s);
    };

    if (s.is_ascii() && enc != LegacyEncoding::Tcvn3H) || is_already_unicode_vietnamese(s) {
        return Cow::Borrowed(s);
    }

    match enc {
        LegacyEncoding::Tcvn3 => transcode_tcvn3(s),
        LegacyEncoding::Tcvn3H => transcode_tcvn3_h(s),
        LegacyEncoding::Vni => transcode_vni(s),
        LegacyEncoding::Windows1258 => transcode_windows1258(s),
    }
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
            return Ok(apply_nfc_cow(decoded));
        }
        return Ok(decoded);
    }

    // 2. Fast path: check valid UTF-8
    // Genuine UTF-8: never apply heuristic or metadata legacy decoding to valid UTF-8
    // when force_encoding is None. This preserves modern UTF-8 text from accidental corruption.
    if let Ok(s) = std::str::from_utf8(input) {
        if options.normalize_nfc {
            return Ok(normalize_nfc(s));
        }
        return Ok(Cow::Borrowed(s));
    }

    // 3. Non-UTF8 bytes: detect encoding via content heuristics or metadata
    let detection = detect_encoding(input, options.metadata);
    if let Some(enc) = detection.encoding() {
        let decoded = decode_with_mode(input, enc, options.mode)?;
        if options.normalize_nfc {
            return Ok(apply_nfc_cow(decoded));
        }
        return Ok(decoded);
    }

    // 4. Neither valid UTF-8 nor a recognized Vietnamese legacy encoding
    let valid_up_to = match std::str::from_utf8(input) {
        Ok(_) => 0,
        Err(e) => e.valid_up_to(),
    };
    Err(DecodeError::InvalidUtf8 { offset: valid_up_to })
}

/// Canonical Unicode Normalization Form C (NFC) helper preserving borrowed Cow where possible.
#[inline]
fn apply_nfc_cow<'a>(cow: Cow<'a, str>) -> Cow<'a, str> {
    match cow {
        Cow::Borrowed(b) => normalize_nfc(b),
        Cow::Owned(o) => {
            let nfc = ComposingNormalizerBorrowed::new_nfc();
            if nfc.is_normalized_utf8(o.as_bytes()) { Cow::Owned(o) } else { Cow::Owned(nfc.normalize(&o).into_owned()) }
        }
    }
}

/// Transcode using an explicitly known encoding, skipping heuristic detection.
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
            let nfc = normalize_nfc(&decoded);
            if nfc.as_ref() == text { Cow::Borrowed(text) } else { Cow::Owned(nfc.into_owned()) }
        }
        Err(_) => Cow::Borrowed(text),
    }
}

/// Transcode legacy text to Unicode NFC and return detailed status.
pub fn transcode_to_unicode<'a>(s: &'a str, font_hint: Option<&str>) -> TranscodeResult<'a> {
    let enc = detect_legacy_encoding(s, font_hint);

    let Some(encoding) = enc else {
        return TranscodeResult { text: Cow::Borrowed(s), encoding: None, was_transcoded: false };
    };

    if (s.is_ascii() && encoding != LegacyEncoding::Tcvn3H) || is_already_unicode_vietnamese(s) {
        return TranscodeResult { text: Cow::Borrowed(s), encoding: Some(encoding), was_transcoded: false };
    }

    let transcoded = match encoding {
        LegacyEncoding::Tcvn3 => transcode_tcvn3(s),
        LegacyEncoding::Tcvn3H => transcode_tcvn3_h(s),
        LegacyEncoding::Vni => transcode_vni(s),
        LegacyEncoding::Windows1258 => transcode_windows1258(s),
    };

    let was_transcoded = matches!(&transcoded, Cow::Owned(_)) || (transcoded.as_ref() != s);
    TranscodeResult { text: transcoded, encoding: Some(encoding), was_transcoded }
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

/// Check whether a legacy font name suggests a Serif typeface (vs Sans-Serif).
#[inline]
pub fn is_serif_legacy_font(font: &str) -> bool {
    let lower = font.trim().to_ascii_lowercase();
    lower.contains("time")
        || lower.contains("century")
        || lower.contains("roman")
        || lower.contains("book")
        || lower.contains("cambria")
        || lower.contains("palatin")
}

/// Normalize an arbitrary string to Unicode Normalization Form C (NFC).
///
/// Returns `Cow::Borrowed` when the string is already in NFC format.
pub fn normalize_nfc<'a>(s: &'a str) -> Cow<'a, str> {
    let nfc = ComposingNormalizerBorrowed::new_nfc();
    if nfc.is_normalized_utf8(s.as_bytes()) { Cow::Borrowed(s) } else { Cow::Owned(nfc.normalize(s).into_owned()) }
}
