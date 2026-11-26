//! Smart quotes: straight quotes typed into text become the language's quotation marks
//! (Photoshop's Preferences › Type › Use Smart Quotes, on by default). The marks follow the
//! language of the text at the caret, as Photoshop's follow the Character panel's language.
//! Only typed text is converted; pasted text keeps its quotes (Photoshop).
//!
//! ```ignore
//! let style = QuoteStyle::for_language("de");
//! assert_eq!(smart_quotes("\"Ja\"", None, style), "„Ja“");
//! ```

/// A language's quotation marks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct QuoteStyle {
    /// The opening and closing marks a typed `"` becomes.
    pub double: (char, char),
    /// The opening and closing marks of a quotation inside a quotation, which a typed `'` becomes
    /// where it opens one.
    pub single: (char, char),
    /// What a typed `'` after a letter becomes: the apostrophe (’ in every language here), which
    /// also closes an English single quotation.
    pub apostrophe: char,
}

impl QuoteStyle {
    /// “English” and ‘English’.
    pub const ENGLISH: QuoteStyle = QuoteStyle { double: ('\u{201C}', '\u{201D}'), single: ('\u{2018}', '\u{2019}'), apostrophe: '\u{2019}' };
    /// „Deutsch“ and ‚Deutsch‘.
    pub const GERMAN: QuoteStyle = QuoteStyle { double: ('\u{201E}', '\u{201C}'), single: ('\u{201A}', '\u{2018}'), apostrophe: '\u{2019}' };
    /// «français» and «français» (CLDR's French marks; no spaces are inserted).
    pub const FRENCH: QuoteStyle = QuoteStyle { double: ('\u{00AB}', '\u{00BB}'), single: ('\u{00AB}', '\u{00BB}'), apostrophe: '\u{2019}' };
    /// 「日本語」 and 『日本語』.
    pub const JAPANESE: QuoteStyle = QuoteStyle { double: ('\u{300C}', '\u{300D}'), single: ('\u{300E}', '\u{300F}'), apostrophe: '\u{2019}' };

    /// The marks for a BCP 47 language tag (`"de"`, `"de-CH"`, `"fr-CA"`), by its primary subtag,
    /// region where CLDR differs (`de-CH` uses «» and ‹›). Unknown or empty tags give
    /// [`QuoteStyle::ENGLISH`].
    // new: PhotoCraft always uses English marks (pc: crates/ui-egui/src/tools/type_tool/editing.rs:186).
    // CLDR 49.0.0 (cldr-json `cldr-misc-full/main/<locale>/delimiters.json`, resolved with
    // inheritance). The table is CLDR's `delimiters` (quotationStart/End, alternateQuotationStart/End) for the
    // languages craft-text hyphenates, Japanese, Chinese and Korean.
    pub fn for_language(tag: &str) -> QuoteStyle {
        let lower = tag.to_ascii_lowercase();
        let mut parts = lower.split(['-', '_']);
        let lang = parts.next().unwrap_or_default();
        let rest: Vec<&str> = parts.collect();
        let region = |r: &[&str]| rest.iter().any(|p| r.contains(p));
        let q = |d: (char, char), s: (char, char)| QuoteStyle { double: d, single: s, apostrophe: '\u{2019}' };
        match lang {
            "de" => QuoteStyle::GERMAN,
            "fr" if region(&["ch"]) => q(('\u{00AB}', '\u{00BB}'), ('\u{2039}', '\u{203A}')),
            "fr" if region(&["ca"]) => q(('\u{00AB}', '\u{00BB}'), ('\u{201D}', '\u{201C}')),
            "fr" => QuoteStyle::FRENCH,
            "it" | "el" => q(('\u{00AB}', '\u{00BB}'), ('\u{201C}', '\u{201D}')),
            "pt" if region(&["pt"]) => q(('\u{00AB}', '\u{00BB}'), ('\u{201C}', '\u{201D}')),
            "ja" => QuoteStyle::JAPANESE,
            "zh" if region(&["hant", "tw", "hk", "mo"]) => QuoteStyle::JAPANESE,
            "cs" | "sk" | "sl" | "hr" | "et" | "is" => QuoteStyle::GERMAN,
            "bg" | "lt" => q(('\u{201E}', '\u{201C}'), ('\u{201E}', '\u{201C}')),
            "sr" => q(('\u{201E}', '\u{201D}'), ('\u{2019}', '\u{2019}')),
            "ka" => q(('\u{201E}', '\u{201C}'), ('\u{00AB}', '\u{00BB}')),
            "pl" | "ro" => q(('\u{201E}', '\u{201D}'), ('\u{00AB}', '\u{00BB}')),
            "hu" => q(('\u{201E}', '\u{201D}'), ('\u{00BB}', '\u{00AB}')),
            "ru" | "uk" | "be" => q(('\u{00AB}', '\u{00BB}'), ('\u{201E}', '\u{201C}')),
            "kk" => q(('\u{00AB}', '\u{00BB}'), ('\u{201C}', '\u{201D}')),
            "hy" => q(('\u{00AB}', '\u{00BB}'), ('\u{00AB}', '\u{00BB}')),
            "sv" | "fi" => q(('\u{201D}', '\u{201D}'), ('\u{2019}', '\u{2019}')),
            "nb" | "no" | "nn" => q(('\u{00AB}', '\u{00BB}'), ('\u{2018}', '\u{2019}')),
            "nl" => q(('\u{2018}', '\u{2019}'), ('\u{2018}', '\u{2019}')),
            "he" => q(('\u{201D}', '\u{201D}'), ('\u{2019}', '\u{2019}')),
            "ar" => q(('\u{201D}', '\u{201C}'), ('\u{2019}', '\u{2018}')),
            "fa" => q(('\u{00AB}', '\u{00BB}'), ('\u{2039}', '\u{203A}')),
            // en, es, pt, da, lv, tr, ko, zh-Hans and unknown tags.
            _ => QuoteStyle::ENGLISH,
        }
    }
}

/// `s` as typed after the character `prev` (the one before the caret, `None` at the start of the
/// text), with straight quotes replaced: a quote opens after whitespace, an opening bracket or a
/// dash (or at the start), and closes elsewhere; `'` after a letter is the apostrophe. Each
/// converted character becomes `prev` for the next.
// pc: crates/ui-egui/src/tools/type_tool/editing.rs:186 `smart_quotes` and :212 `opens_quote`.
pub fn smart_quotes(s: &str, mut prev: Option<char>, style: QuoteStyle) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        let c = match c {
            '"' if opens_quote(prev) => style.double.0,
            '"' => style.double.1,
            '\'' if opens_quote(prev) => style.single.0,
            '\'' if prev.is_some_and(char::is_alphanumeric) => style.apostrophe,
            '\'' => style.single.1,
            _ => c,
        };
        out.push(c);
        prev = Some(c);
    }
    out
}

fn opens_quote(prev: Option<char>) -> bool {
    prev.is_none_or(|p| p.is_whitespace() || matches!(p, '(' | '[' | '{' | '-' | '\u{2013}' | '\u{2014}'))
}
