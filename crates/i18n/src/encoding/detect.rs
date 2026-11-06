//! Detection of Vietnamese legacy 8-bit encodings (TCVN3, TCVN3H, VNI-Windows)
//! and recognition of modern Unicode text to prevent corruption.

use super::LegacyEncoding;
use super::tcvn3::{match_tcvn3_composite, match_tcvn3_single};
use super::vni::{match_vni_composite, match_vni_single};

/// Known font family name prefixes indicating VNI-Windows encoding.
const VNI_PREFIXES: &[&str] = &[
    "vni", "vhelv", "vtime", "vavon", "vapto", "vcomm", "vkorin", "vpres", "vrevu", "vsouv", "vtekn", "vuniv", "vbaham", "vbook", "vcent", "vcoop",
    "vcour", "vfree", "vgoud", "vpark", "vscri",
];

/// Known font family name prefixes indicating dotless TCVN3 encoding.
const TCVN3_DOTLESS_PREFIXES: &[&str] = &[
    "vntime",
    "vnarial",
    "vnuniverse",
    "vnavant",
    "vncourier",
    "vncentury",
    "vngothic",
    "vnhelvet",
    "vnsouthern",
    "vnteknical",
    "vnvogue",
    "vnpresent",
    "vnrevue",
    "vnfree",
    "vncooper",
    "vnpark",
    "vnlincoln",
    "vnlinus",
    "vnkoala",
    "vnmemorandum",
    "vnmystical",
    "vnstamp",
    "vnsim",
    "vnroman",
    "vntxt",
    "vnmono",
    "vn-",
    "vn_",
    "tcvn",
];

/// Detect whether a font name indicates a legacy Vietnamese encoding.
pub fn is_vietnamese_font(font: &str) -> Option<LegacyEncoding> {
    let lower = font.trim().to_ascii_lowercase();
    let stem = lower.strip_suffix(".ttf").or_else(|| lower.strip_suffix(".shx")).or_else(|| lower.strip_suffix(".otf")).unwrap_or(&lower).trim();

    if stem.is_empty() {
        return None;
    }

    if VNI_PREFIXES.iter().any(|p| stem.starts_with(p)) {
        return Some(LegacyEncoding::Vni);
    }

    let is_tcvn = (stem.starts_with(".vn") && stem.len() > 3) || stem == "vn" || TCVN3_DOTLESS_PREFIXES.iter().any(|p| stem.starts_with(p));

    if is_tcvn {
        let is_header = stem.ends_with('h')
            && !stem.ends_with("tech")
            && !stem.ends_with("math")
            && !stem.ends_with("graph")
            && !stem.ends_with("flash")
            && !stem.ends_with("brush");
        if is_header { Some(LegacyEncoding::Tcvn3H) } else { Some(LegacyEncoding::Tcvn3) }
    } else {
        None
    }
}

/// Checks whether a character is a precomposed Vietnamese vowel in the Latin-1 Supplement block (0xC0..=0xFF).
#[inline]
fn is_latin1_vietnamese_vowel(c: char) -> bool {
    matches!(
        c,
        'À' | 'Á'
            | 'Â'
            | 'Ã'
            | 'È'
            | 'É'
            | 'Ê'
            | 'Ì'
            | 'Í'
            | 'Ò'
            | 'Ó'
            | 'Ô'
            | 'Õ'
            | 'Ù'
            | 'Ú'
            | 'Ý'
            | 'à'
            | 'á'
            | 'â'
            | 'ã'
            | 'è'
            | 'é'
            | 'ê'
            | 'ì'
            | 'í'
            | 'ò'
            | 'ó'
            | 'ô'
            | 'õ'
            | 'ù'
            | 'ú'
            | 'ý'
    )
}

/// Check if a string already contains genuine Unicode Vietnamese diacritic characters
/// that exist outside the single-byte 0x00..=0xFF Windows-1252 range (U+0100..=U+024F, U+1E00..=U+1EFF),
/// or precomposed Latin-1 Vietnamese vowels in positions that cannot be VNI or TCVN3 encodings.
/// If true, the text MUST NOT be transcoded, preserving existing modern Unicode text.
pub fn is_already_unicode_vietnamese(s: &str) -> bool {
    let mut prev: Option<char> = None;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        let code = c as u32;
        if (0x0100..=0x024F).contains(&code) || (0x1E00..=0x1EFF).contains(&code) || matches!(c, 'Đ' | 'đ' | 'Ă' | 'ă' | 'Ơ' | 'ơ' | 'Ư' | 'ư')
        {
            return true;
        }

        if is_latin1_vietnamese_vowel(c) {
            let next_c = chars.peek().copied();

            // Case A: Unicode "TÒA", "TÒA ÁN" ('Ò' before 'A')
            if matches!(c, 'Ò' | 'ò') && matches!(next_c, Some('A' | 'a')) {
                return true;
            }

            // Case B: Unicode diphthong OÁ, OÀ, OÃ followed by a final ASCII consonant (TOÁN, HOÀN, KHOÁC, HOÃN)
            if matches!(prev, Some('O' | 'o'))
                && matches!(c, 'Á' | 'á' | 'À' | 'à' | 'Ã' | 'ã')
                && next_c.is_some_and(|post| post.is_ascii_alphabetic() && !matches!(post.to_ascii_uppercase(), 'A' | 'E' | 'I' | 'O' | 'U' | 'Y'))
            {
                return true;
            }

            // Case C: Standalone Latin-1 vowel not forming a VNI composite with `prev` or `next_c`
            let is_vni_prev = prev.is_some_and(|p| match_vni_composite(p, c).is_some());
            let is_vni_next = next_c.is_some_and(|n| match_vni_composite(c, n).is_some());

            if !is_vni_prev && !is_vni_next {
                if matches!(c, 'Ô' | 'ô' | 'Ò' | 'ò' | 'Ó' | 'ó') {
                    // Protect Unicode words like "TÔI", "CÔNG", "CÔ" when preceded by uppercase or followed by vowel/NG
                    if next_c.is_some_and(|post| matches!(post, 'A' | 'a' | 'I' | 'i' | 'N' | 'n'))
                        || (c.is_uppercase() && prev.is_some_and(|p| p.is_ascii_uppercase()) && next_c.is_none_or(|n| n.is_whitespace()))
                    {
                        return true;
                    }
                } else if matches!(c, 'À' | 'Á' | 'Ã' | 'Ì' | 'Í' | 'Ù' | 'Ú' | 'Ý' | 'à')
                    && prev.is_none_or(|p| !p.is_ascii_lowercase() || c.is_lowercase())
                {
                    return true;
                }
            }
        }

        prev = Some(c);
    }
    false
}

/// Source of evidence used to detect or enforce a legacy Vietnamese text encoding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum EncodingEvidence {
    /// Explicitly forced by caller or application configuration (highest priority).
    Explicit,
    /// Container metadata (e.g. DXF `$DWGCODEPAGE`, PDF font dict).
    Metadata,
    /// Advisory font name pattern (e.g. `.VnTime` -> TCVN3, `VNI-Times` -> VNI).
    FontHint,
    /// Statistical byte-distribution heuristic score on text content.
    Heuristic,
}

/// Categorized outcome of Vietnamese text encoding detection.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DetectionResult {
    /// High-confidence detection backed by explicit evidence or verified signatures.
    HighConfidence {
        /// Detected legacy encoding.
        encoding: LegacyEncoding,
        /// Source of evidence backing the detection result.
        evidence: EncodingEvidence,
    },
    /// Multiple candidate encodings match valid byte distributions equally, or metadata conflicts.
    Ambiguous(&'static [LegacyEncoding]),
    /// Byte patterns match no known Vietnamese encoding profile.
    Unknown,
}

impl DetectionResult {
    /// Returns the detected encoding if confidence is high, or `None` if ambiguous or unknown.
    pub const fn encoding(&self) -> Option<LegacyEncoding> {
        match self {
            Self::HighConfidence { encoding, .. } => Some(*encoding),
            _ => None,
        }
    }

    /// Returns the evidence source if confidence is high, or `None`.
    pub const fn evidence(&self) -> Option<EncodingEvidence> {
        match self {
            Self::HighConfidence { evidence, .. } => Some(*evidence),
            _ => None,
        }
    }
}

/// External metadata hints for encoding detection.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct EncodingMetadata<'a> {
    /// Font name hint (e.g. ".VnTime", "VNI-Helve").
    pub font_name: Option<&'a str>,
    /// Codepage hint (e.g. 1258).
    pub codepage_hint: Option<u32>,
}

/// Score text content against VNI and TCVN3 characteristic byte signatures.
///
/// Returns `(vni_valid, tcvn_valid, vni_score, tcvn_score)`.
fn score_vietnamese_encodings(s: &str) -> (bool, bool, usize, usize) {
    let mut vni_score = 0;
    let mut tcvn_score = 0;
    let mut vni_composite_found = false;
    let mut tcvn_composite_found = false;
    let mut tcvn_inversion_found = false;

    let mut prev: Option<char> = None;
    let mut chars = s.chars().peekable();
    while let Some(c) = chars.next() {
        if let Some(&next_c) = chars.peek() {
            let mut matched_composite = false;
            let is_diameter = c.eq_ignore_ascii_case(&'O') && next_c == 'Ø';
            let is_ambiguous_western_pair = matches!(
                (c, next_c),
                ('A', 'É')
                    | ('a', 'é')
                    | ('O', 'Ù')
                    | ('o', 'ù')
                    | ('A', 'Ï')
                    | ('a', 'ï')
                    | ('O', 'Ï')
                    | ('o', 'ï')
                    | ('U', 'ñ')
                    | ('u', 'ñ')
                    | ('E', 'Ñ')
                    | ('e', 'ñ')
            );
            if !is_diameter && !is_ambiguous_western_pair && match_vni_composite(c, next_c).is_some() {
                vni_score += 2;
                vni_composite_found = true;
                matched_composite = true;
            }
            if !is_ambiguous_western_pair && match_tcvn3_composite(c, next_c).is_some() {
                tcvn_score += 2;
                tcvn_composite_found = true;
                matched_composite = true;
            }
            if matched_composite {
                prev = Some(next_c);
                chars.next();
                continue;
            }
        }

        // Unhinted VNI single-byte characters:
        // Standalone Latin-1 letters (like 'Ñ' in NIÑO or 'ö' in Größe) must never trigger VNI
        // transcoding unless at least one genuine VNI composite has been identified in the string.
        if vni_composite_found && match_vni_single(c).is_some() && !matches!(c, 'Ô' | 'ô' | 'Ò' | 'ò' | 'Ó' | 'ó') {
            vni_score += 1;
        }

        // Unhinted TCVN3 single-byte characters:
        // Exclude symbols, punctuation, and currency marks:
        // ©, ®, §, ¶, ·, µ, ¼, ½, ¾, °, ±, ², ³, ¿, ¡, ¢, £, ¤, ¥
        let is_excluded_symbol =
            matches!(c, '©' | '®' | '§' | '¶' | '·' | 'µ' | '¼' | '½' | '¾' | '°' | '±' | '²' | '³' | '¿' | '¡' | '¢' | '£' | '¤' | '¥');

        if !is_excluded_symbol
            && match_tcvn3_single(c).is_some()
            && !matches!(c, 'á' | 'â' | 'ã' | 'è' | 'é' | 'ê' | 'ì' | 'í' | 'ò' | 'ó' | 'ô' | 'õ' | 'ù' | 'ú' | 'ý')
        {
            let code = c as u32;
            let is_midword_inversion = prev.is_some_and(|p| p.is_ascii_lowercase()) && (0x00C0..=0x00DE).contains(&code);
            if is_midword_inversion {
                tcvn_inversion_found = true;
                tcvn_score += 2;
            } else if tcvn_composite_found {
                tcvn_score += 1;
            }
        }

        prev = Some(c);
    }

    let vni_threshold = if vni_composite_found { 2 } else { usize::MAX };
    let tcvn_threshold = if tcvn_composite_found {
        2
    } else if tcvn_inversion_found {
        4
    } else {
        usize::MAX
    };

    let vni_valid = vni_score >= vni_threshold;
    let tcvn_valid = tcvn_score >= tcvn_threshold;

    (vni_valid, tcvn_valid, vni_score, tcvn_score)
}

/// Detect the legacy Vietnamese encoding of a string slice with evidence categorization.
pub fn detect_encoding_from_str(s: &str) -> DetectionResult {
    // Short-String Invariant: strings shorter than 16 bytes disable statistical heuristics
    if s.len() < 16 || s.is_ascii() || is_already_unicode_vietnamese(s) {
        return DetectionResult::Unknown;
    }

    let (vni_valid, tcvn_valid, vni_score, tcvn_score) = score_vietnamese_encodings(s);

    if vni_valid && tcvn_valid && vni_score == tcvn_score {
        return DetectionResult::Ambiguous(&[LegacyEncoding::Vni, LegacyEncoding::Tcvn3]);
    }

    if vni_valid && vni_score > tcvn_score {
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Vni, evidence: EncodingEvidence::Heuristic }
    } else if tcvn_valid && tcvn_score > vni_score {
        DetectionResult::HighConfidence { encoding: LegacyEncoding::Tcvn3, evidence: EncodingEvidence::Heuristic }
    } else {
        DetectionResult::Unknown
    }
}

/// Detect the legacy Vietnamese encoding of raw input bytes using metadata hints and content heuristics.
pub fn detect_encoding(input: &[u8], metadata: Option<&EncodingMetadata<'_>>) -> DetectionResult {
    if let Some(meta) = metadata {
        // 1. Container metadata (codepage hint) takes precedence over font name
        if meta.codepage_hint == Some(1258) {
            return DetectionResult::HighConfidence { encoding: LegacyEncoding::Windows1258, evidence: EncodingEvidence::Metadata };
        }
        // 2. Font name hint (advisory)
        if let Some(font) = meta.font_name
            && let Some(enc) = is_vietnamese_font(font)
        {
            return DetectionResult::HighConfidence { encoding: enc, evidence: EncodingEvidence::FontHint };
        }
    }

    // Short-String Invariant (< 16 bytes): strings shorter than 16 bytes automatically disable
    // statistical byte-frequency heuristics to prevent false-positive guessing. Short strings rely
    // strictly on the Evidence Hierarchy (Valid UTF-8 or FontHint / Metadata).
    if input.len() < 16 {
        return DetectionResult::Unknown;
    }

    if let Ok(s) = std::str::from_utf8(input) {
        detect_encoding_from_str(s)
    } else {
        let latin1_s: String = input.iter().map(|&b| b as char).collect();
        detect_encoding_from_str(&latin1_s)
    }
}

/// Detect if a text string has unambiguous characteristic byte signatures of VNI or TCVN3.
pub fn detect_vietnamese_encoding(s: &str) -> Option<LegacyEncoding> {
    detect_encoding_from_str(s).encoding()
}
