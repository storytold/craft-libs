//! Detection of Vietnamese legacy 8-bit encodings (TCVN3, TCVN3H, VNI-Windows)
//! and recognition of modern Unicode text to prevent corruption.

use super::LegacyEncoding;
use super::tcvn3::{match_tcvn3_composite, match_tcvn3_single};
use super::vni::{match_vni_composite, match_vni_single};

const VNI_PREFIXES: &[&str] = &[
    "vni", "vhelv", "vtime", "vavon", "vapto", "vcomm", "vkorin", "vpres", "vrevu", "vsouv", "vtekn", "vuniv", "vbaham", "vbook", "vcent", "vcoop",
    "vcour", "vfree", "vgoud", "vpark", "vscri",
];

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

/// Detect if a text string has unambiguous characteristic byte signatures of VNI or TCVN3.
pub fn detect_vietnamese_encoding(s: &str) -> Option<LegacyEncoding> {
    if s.is_ascii() || is_already_unicode_vietnamese(s) {
        return None;
    }

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

    if vni_valid && vni_score > tcvn_score {
        Some(LegacyEncoding::Vni)
    } else if tcvn_valid && tcvn_score > vni_score {
        Some(LegacyEncoding::Tcvn3)
    } else {
        None
    }
}
