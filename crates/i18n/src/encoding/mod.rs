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

pub use detect::{detect_vietnamese_encoding, is_already_unicode_vietnamese, is_vietnamese_font};
pub use tcvn3::{transcode_tcvn3, transcode_tcvn3_h};
pub use vni::transcode_vni;
pub use windows1258::{decode_windows1258, transcode_windows1258};

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
pub fn decode(input: &[u8], encoding: LegacyEncoding) -> Result<Cow<'_, str>, DecodeError> {
    match encoding {
        LegacyEncoding::Windows1258 => decode_windows1258(input),
        LegacyEncoding::Tcvn3 | LegacyEncoding::Tcvn3H | LegacyEncoding::Vni => {
            if input.is_ascii() {
                if let Ok(s) = std::str::from_utf8(input) {
                    return Ok(Cow::Borrowed(s));
                }
                return Err(DecodeError::InvalidUtf8 { offset: 0 });
            }
            if let Ok(s) = std::str::from_utf8(input) {
                let transcoded = match encoding {
                    LegacyEncoding::Tcvn3 => transcode_tcvn3(s),
                    LegacyEncoding::Tcvn3H => transcode_tcvn3_h(s),
                    LegacyEncoding::Vni => transcode_vni(s),
                    LegacyEncoding::Windows1258 => Cow::Borrowed(s),
                };
                return Ok(transcoded);
            }
            let s: String = input.iter().map(|&b| b as char).collect();
            let transcoded = match encoding {
                LegacyEncoding::Tcvn3 => transcode_tcvn3(&s),
                LegacyEncoding::Tcvn3H => transcode_tcvn3_h(&s),
                LegacyEncoding::Vni => transcode_vni(&s),
                LegacyEncoding::Windows1258 => Cow::Borrowed(""),
            };
            Ok(Cow::Owned(transcoded.into_owned()))
        }
    }
}

/// Transcode using an explicitly known encoding, skipping heuristic detection.
///
/// Use this in `pdfcraft` or strict pipelines when metadata (such as a known PDF font name)
/// guarantees the encoding, eliminating heuristic detection false positives.
pub fn transcode_explicit<'a>(text: &'a str, encoding: LegacyEncoding) -> Cow<'a, str> {
    match encoding {
        LegacyEncoding::Tcvn3 => transcode_tcvn3(text),
        LegacyEncoding::Tcvn3H => transcode_tcvn3_h(text),
        LegacyEncoding::Vni => transcode_vni(text),
        LegacyEncoding::Windows1258 => transcode_windows1258(text),
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
/// if the text contains unidentifiable legacy patterns, preventing silent data corruption.
///
/// Use this in `pdfcraft` or strict pipelines when you want to guarantee the output is verified Unicode.
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
