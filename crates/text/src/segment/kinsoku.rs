//! Kinsoku character sets (JLREQ 3.1.7–3.1.8, CLREQ 3.1.7). The sets are VectorCraft's, with the
//! additions listed in the crate README.

use super::Kinsoku;

/// Whether kinsoku lets a line break between `before` and `after` (`None`: the end of the
/// paragraph). `Hard` keeps every character of the set off the line start and end; `Soft` lets
/// 々, ー and small kana start a line; `None` allows every break. Breaks with no East Asian
/// character on either side are always allowed, so Latin text keeps its own rules.
pub fn kinsoku_allows(before: char, after: Option<char>, mode: Kinsoku) -> bool {
    if mode == Kinsoku::None || (!is_east_asian(before) && !after.is_some_and(is_east_asian)) {
        return true;
    }
    let start_ok = |c: char| !no_line_start(c) || (mode == Kinsoku::Soft && soft_line_start(c));
    !no_line_end(before) && after.is_none_or(start_ok)
}

/// East Asian text for kinsoku: Han, kana, bopomofo, Hangul syllables and compatibility jamo, CJK
/// punctuation and symbols, full-width and half-width forms. Not the ideographic space, conjoining
/// jamo or the (combining and spacing) kana sound marks, which never take a break before them.
pub(crate) fn is_east_asian(c: char) -> bool {
    matches!(c as u32,
        0x2E80..=0x2FFF          // radicals, ideographic description
        | 0x3001..=0x3096        // CJK symbols and punctuation, hiragana
        | 0x309D..=0x30FF        // hiragana iteration marks, katakana
        | 0x3100..=0x312F        // bopomofo
        | 0x3131..=0x318F        // Hangul compatibility jamo
        | 0x3190..=0x33FF        // kanbun, strokes, small katakana, enclosed and compatibility forms
        | 0x3400..=0x4DBF        // CJK extension A
        | 0x4E00..=0x9FFF        // CJK unified ideographs
        | 0xAC00..=0xD7A3        // Hangul syllables
        | 0xF900..=0xFAFF        // compatibility ideographs
        | 0xFE30..=0xFE4F        // CJK compatibility forms
        | 0xFF01..=0xFF9D        // full-width forms, half-width katakana
        | 0xFFA0..=0xFFEF        // half-width Hangul, full-width signs
        | 0x20000..=0x3FFFF) // supplementary ideographs
}

/// A character that can't start a line: closing brackets and quotes, hyphens, dividing
/// punctuation, middle dots, colons, full stops and commas, iteration marks, the prolonged sound
/// mark, small kana, kana sound marks and postfixed abbreviations (JLREQ cl-02 to cl-07, cl-09 to
/// cl-11, cl-13).
pub(crate) fn no_line_start(c: char) -> bool {
    matches!(c,
        ')' | ']' | '}' | ',' | '.' | ':' | ';' | '!' | '?' | '»' | '’' | '”' | '‐' | '–' | '‼' | '⁇' | '⁈' | '⁉'
        | '、' | '。' | '〉' | '》' | '」' | '』' | '】' | '〕' | '〗' | '〙' | '〛' | '〞' | '〟' | '〜' | '゠' | '・' | '｠'
        | 'ー' | 'ゝ' | 'ゞ' | 'ヽ' | 'ヾ' | '々' | '〻' | '゛' | '゜'
        | 'ぁ' | 'ぃ' | 'ぅ' | 'ぇ' | 'ぉ' | 'っ' | 'ゃ' | 'ゅ' | 'ょ' | 'ゎ' | 'ゕ' | 'ゖ'
        | 'ァ' | 'ィ' | 'ゥ' | 'ェ' | 'ォ' | 'ッ' | 'ャ' | 'ュ' | 'ョ' | 'ヮ' | 'ヵ' | 'ヶ' | 'ㇰ'..='ㇿ'
        | '！' | '％' | '）' | '，' | '．' | '：' | '；' | '？' | '］' | '｝' | '～' | '｡' | '｣' | '､' | '･' | 'ｰ' | 'ｧ'..='ｯ'
        | 'ﾞ' | 'ﾟ' | '￠')
}

/// A character of [`no_line_start`] that may start a line under [`Kinsoku::Soft`]: 々, the
/// prolonged sound mark ー (JLREQ cl-10) and small kana (cl-11), as JLREQ's level 3 rules allow
/// (Appendix C.3, <https://www.w3.org/TR/jlreq/#addendum_a3>). Half-width forms aren't in those
/// classes.
pub(crate) fn soft_line_start(c: char) -> bool {
    matches!(
        c,
        '々' | 'ー'
            | 'ぁ'
            | 'ぃ'
            | 'ぅ'
            | 'ぇ'
            | 'ぉ'
            | 'っ'
            | 'ゃ'
            | 'ゅ'
            | 'ょ'
            | 'ゎ'
            | 'ゕ'
            | 'ゖ'
            | 'ァ'
            | 'ィ'
            | 'ゥ'
            | 'ェ'
            | 'ォ'
            | 'ッ'
            | 'ャ'
            | 'ュ'
            | 'ョ'
            | 'ヮ'
            | 'ヵ'
            | 'ヶ'
            | 'ㇰ'..='ㇿ'
    )
}

/// A character that can't end a line: opening brackets and quotes, and prefixed abbreviations
/// (JLREQ cl-01, cl-12).
pub(crate) fn no_line_end(c: char) -> bool {
    matches!(
        c,
        '(' | '['
            | '{'
            | '«'
            | '‘'
            | '“'
            | '〈'
            | '《'
            | '「'
            | '『'
            | '【'
            | '〔'
            | '〖'
            | '〘'
            | '〚'
            | '〝'
            | '（'
            | '［'
            | '｛'
            | '｟'
            | '｢'
            | '＄'
            | '￡'
            | '￥'
    )
}
