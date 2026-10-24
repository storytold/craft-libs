//! Case- and accent-insensitive matching for search fields, with a map back to the original text
//! for highlighting.
//!
//! [`Folder::fold`] turns text into a search key: compatibility forms decomposed (`ﬁ` → `fi`,
//! full-width `Ａ` → `a`, `㎏` → `kg`), lower case, `ß` → `ss`, final `ς` → `σ`, and with accents
//! ignored (the default) combining accents, Hebrew points and Arabic vowel marks dropped and the
//! few letters with built-in strokes mapped to their base (`ø`, `ł`, `đ`, `ı`, `æ` → `ae`), and
//! Persian letter forms mapped to the Arabic ones (`ک` → `ك`, `ی` → `ي`, `ة` → `ه`). Decimal
//! digits of every script fold to ASCII (`١٢` → `12`). Invisible format characters (soft
//! hyphens, zero-width joiners, bidi isolates) are dropped too.
//!
//! Dotted and dotless i follow the locale when accents count. In Turkish and Azerbaijani, `I`
//! folds to `ı` and `İ` to `i`; elsewhere both fold to `i` and `ı` stays a letter of its own.
//! Ignoring accents, all four fold to `i`, so `istanbul` finds `İSTANBUL` and `irmak` finds
//! `IRMAK` and `ırmak`. Plain `str::to_lowercase` follows no locale and turns `İ` into `i` plus a
//! combining dot, which nothing typed would match.
//!
//! The Turkish i handling and sigma folding come from PdfCraft's `fold_case` (snipeTR,
//! nevicia2010-hub).

use std::ops::Range;

use icu_normalizer::DecomposingNormalizerBorrowed;
use icu_properties::props::{GraphemeExtend, HangulSyllableType};
use icu_properties::{CodePointMapData, CodePointSetData};

/// Folds text for one locale. Cheap to build and copy.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Folder {
    turkic: bool,
    accents: bool,
}

impl Folder {
    /// Case- and accent-insensitive folding for `locale`, a language code such as `tr`, `pt-br`
    /// or `zh-hant` (any case, `-` or `_`).
    pub fn new(locale: &str) -> Folder {
        let language = locale.split(['-', '_', '.', '@']).next().unwrap_or_default();
        Folder { turkic: language.eq_ignore_ascii_case("tr") || language.eq_ignore_ascii_case("az"), accents: false }
    }

    /// Accents count: `é` no longer matches `e`, and a match can't end on a letter whose accent it
    /// leaves out.
    pub fn keep_accents(self) -> Folder {
        Folder { accents: true, ..self }
    }

    /// `s` folded, with the map back to `s`.
    pub fn fold(&self, s: &str) -> Folded {
        let nfkd = DecomposingNormalizerBorrowed::new_nfkd();
        let extend = CodePointSetData::new::<GraphemeExtend>();
        let hangul = CodePointMapData::<HangulSyllableType>::new();
        let mut text = String::with_capacity(s.len());
        let mut starts = Vec::new();
        // A cluster is a character and the combining marks after it, folded and mapped as one;
        // conjoining jamo that spell one syllable are one cluster too.
        let mut cluster_start = 0;
        let mut chars = s.char_indices();
        let mut prev = chars.next().map(|(_, c)| hangul.get(c));
        for (i, c) in chars {
            let kind = hangul.get(c);
            if !extend.contains(c) && !prev.is_some_and(|p| continues_syllable(p, kind)) {
                starts.push((text.len(), cluster_start));
                self.fold_cluster(&nfkd.normalize(&s[cluster_start..i]), &mut text);
                cluster_start = i;
            }
            prev = Some(kind);
        }
        if !s.is_empty() {
            starts.push((text.len(), cluster_start));
            self.fold_cluster(&nfkd.normalize(&s[cluster_start..]), &mut text);
        }
        starts.push((text.len(), s.len()));
        Folded { text, starts }
    }

    /// `s` folded, without the map: a key to compare or store.
    pub fn key(&self, s: &str) -> String {
        self.fold(s).text
    }

    /// Whether `haystack` contains `needle`, both folded.
    pub fn contains(&self, haystack: &str, needle: &str) -> bool {
        self.fold(haystack).find(&self.fold(needle)).is_some()
    }

    /// One cluster, already in NFKD.
    fn fold_cluster(&self, cluster: &str, out: &mut String) {
        let mut chars = cluster.chars().peekable();
        while let Some(c) = chars.next() {
            match c {
                // `İ` decomposes to `I` and a combining dot above: the dot of a lower-case `i`.
                'I' if chars.peek() == Some(&'\u{307}') => {
                    chars.next();
                    out.push('i');
                }
                'I' if self.turkic && self.accents => out.push('ı'),
                // Persian heh with hamza above (ۀ) decomposes to ae (ە) and hamza.
                '\u{6D5}' if !self.accents && chars.peek() == Some(&'\u{654}') => {
                    chars.next();
                    out.push('ه');
                }
                'ß' | 'ẞ' => out.push_str("ss"),
                c if let Some(d) = ascii_digit(c) => out.push(d),
                'Σ' | 'σ' | 'ς' => out.push('σ'),
                c if ignorable(c) || (!self.accents && accent(c)) => {}
                c => {
                    for l in c.to_lowercase() {
                        match base_letter(l) {
                            Some(base) if !self.accents => out.push_str(base),
                            _ => out.push(l),
                        }
                    }
                }
            }
        }
    }
}

/// The base of a lower-case letter whose diacritic is built in rather than combining, and the
/// Arabic form of a letter that Persian and Urdu write differently: keheh (ک) is kaf (ك), Farsi
/// yeh (ی) and alef maksura (ى) are yeh (ي), teh marbuta (ة) is heh (ه), alef wasla (ٱ) is alef.
fn base_letter(c: char) -> Option<&'static str> {
    Some(match c {
        'ک' => "ك",
        'ی' | 'ى' => "ي",
        'ة' => "ه",
        'ٱ' => "ا",
        'ı' => "i",
        'ø' => "o",
        'ł' => "l",
        'đ' => "d",
        'ħ' => "h",
        'ŧ' => "t",
        'æ' => "ae",
        'œ' => "oe",
        _ => return None,
    })
}

/// Invisible characters that only steer layout: soft hyphen, Arabic letter mark, the kashida
/// that stretches Arabic words, Mongolian variation selectors, zero-width space and joiners, direction marks, embeddings and isolates,
/// word joiner, variation selectors, byte order mark and tags.
fn ignorable(c: char) -> bool {
    matches!(c,
        '\u{AD}' | '\u{61C}' | '\u{640}' | '\u{180B}'..='\u{180F}' | '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}'
        | '\u{2060}'..='\u{206F}' | '\u{FE00}'..='\u{FE0F}' | '\u{FEFF}' | '\u{E0000}'..='\u{E0FFF}')
}

/// Marks that searching without accents ignores: Latin, Greek and Cyrillic combining diacritics,
/// Hebrew points and cantillation, Arabic vowel marks.
/// Kana voicing marks, Indic and Thai vowel signs are letters' parts, not accents, and stay.
fn accent(c: char) -> bool {
    matches!(c,
        '\u{300}'..='\u{36F}' | '\u{1AB0}'..='\u{1AFF}' | '\u{1DC0}'..='\u{1DFF}' | '\u{20D0}'..='\u{20FF}'
        | '\u{FE20}'..='\u{FE2F}'
        | '\u{591}'..='\u{5BD}' | '\u{5BF}' | '\u{5C1}' | '\u{5C2}' | '\u{5C4}' | '\u{5C5}' | '\u{5C7}'
        | '\u{610}'..='\u{61A}' | '\u{64B}'..='\u{65F}' | '\u{670}' | '\u{6D6}'..='\u{6DC}'
        | '\u{6DF}'..='\u{6E4}' | '\u{6E7}' | '\u{6E8}' | '\u{6EA}'..='\u{6ED}')
}

/// The first code point of each run of ten Unicode decimal digits (general category Nd), Unicode
/// 16.0.
const ZEROS: [u32; 76] = [
    0x30, 0x660, 0x6F0, 0x7C0, 0x966, 0x9E6, 0xA66, 0xAE6, 0xB66, 0xBE6, 0xC66, 0xCE6, 0xD66, 0xDE6, 0xE50, 0xED0, 0xF20, 0x1040, 0x1090, 0x17E0,
    0x1810, 0x1946, 0x19D0, 0x1A80, 0x1A90, 0x1B50, 0x1BB0, 0x1C40, 0x1C50, 0xA620, 0xA8D0, 0xA900, 0xA9D0, 0xA9F0, 0xAA50, 0xABF0, 0xFF10, 0x104A0,
    0x10D30, 0x10D40, 0x11066, 0x110F0, 0x11136, 0x111D0, 0x112F0, 0x11450, 0x114D0, 0x11650, 0x116C0, 0x116D0, 0x116DA, 0x11730, 0x118E0, 0x11950,
    0x11BF0, 0x11C50, 0x11D50, 0x11DA0, 0x11F50, 0x16130, 0x16A60, 0x16AC0, 0x16B50, 0x16D70, 0x1CCF0, 0x1D7CE, 0x1D7D8, 0x1D7E2, 0x1D7EC, 0x1D7F6,
    0x1E140, 0x1E2F0, 0x1E4F0, 0x1E5F1, 0x1E950, 0x1FBF0,
];

/// A decimal digit of any script as its ASCII digit: `١٢` and `१२` search as `12`.
fn ascii_digit(c: char) -> Option<char> {
    let c = u32::from(c);
    let zero = match ZEROS.binary_search(&c) {
        Ok(i) => ZEROS.get(i),
        Err(i) => ZEROS.get(i.checked_sub(1)?),
    }?;
    let value = c - zero;
    if value < 10 { char::from_digit(value, 10) } else { None }
}

/// Whether a conjoining jamo of kind `next` continues the syllable a character of kind `prev`
/// ends: a vowel after a leading consonant, a final consonant after a vowel or an open syllable
/// (`가` and `ᆨ` spell `각`).
fn continues_syllable(prev: HangulSyllableType, next: HangulSyllableType) -> bool {
    matches!(
        (prev, next),
        (HangulSyllableType::LeadingJamo, HangulSyllableType::VowelJamo)
            | (HangulSyllableType::VowelJamo | HangulSyllableType::LVSyllable, HangulSyllableType::TrailingJamo)
    )
}

/// Folded text and where each part came from.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Folded {
    text: String,
    /// One (folded offset, source offset) per source cluster, in order, then both lengths. A
    /// cluster folds to the text up to the next entry's folded offset, possibly none.
    starts: Vec<(usize, usize)>,
}

impl Folded {
    /// The folded text.
    pub fn as_str(&self) -> &str {
        &self.text
    }

    /// The source byte range that folded bytes `range` came from, widened to whole source
    /// characters with their accents: `ss` in the folding of `Straße` maps back to `ß`, `e` in
    /// that of `é` to the whole `é`. An empty range maps to an empty one.
    pub fn source_range(&self, range: Range<usize>) -> Range<usize> {
        // The last cluster starting at or before `at` is the one holding it: clusters that fold to
        // nothing share their offset with the next one.
        let cluster = |at: usize| self.starts.partition_point(|&(f, _)| f <= at).saturating_sub(1);
        if range.is_empty() {
            let s = self.starts[cluster(range.start)].1;
            return s..s;
        }
        let first = cluster(range.start);
        let last = cluster(range.end - 1);
        self.starts[first].1..self.starts[(last + 1).min(self.starts.len() - 1)].1
    }

    /// The first match of `needle` (folded the same way), as a byte range of the source text.
    /// An empty needle matches at the start.
    pub fn find(&self, needle: &Folded) -> Option<Range<usize>> {
        self.matches(needle).next()
    }

    /// Every non-overlapping match of `needle` (folded the same way), as byte ranges of the
    /// source text. A match may start or end inside an expansion (`ss` of `ß`, `fi` of `ﬁ`) but
    /// not leave out a mark that follows a letter, so `re` doesn't match `ré` when accents count.
    pub fn matches<'a>(&'a self, needle: &'a Folded) -> impl Iterator<Item = Range<usize>> + 'a {
        let extend = CodePointSetData::new::<GraphemeExtend>();
        let hangul = CodePointMapData::<HangulSyllableType>::new();
        // The vowel and final consonant of a decomposed Hangul syllable continue it, like a mark,
        // but only inside one source cluster: standalone jamo (`ㅋㅠ`, `ㅠㅠ`) are letters.
        let starts_mark = move |at: usize| {
            let Some(c) = self.text[at..].chars().next() else { return false };
            if extend.contains(c) {
                return true;
            }
            let inside = self.starts.binary_search_by_key(&at, |&(f, _)| f).is_err();
            inside && self.text[..at].chars().next_back().is_some_and(|p| continues_syllable(hangul.get(p), hangul.get(c)))
        };
        let n = needle.text.as_str();
        let mut from = 0;
        let mut done = false;
        std::iter::from_fn(move || {
            if done {
                return None;
            }
            if n.is_empty() {
                done = true;
                return Some(0..0);
            }
            while let Some(i) = self.text.get(from..).and_then(|t| t.find(n)) {
                let start = from + i;
                let end = start + n.len();
                if !starts_mark(start) && !starts_mark(end) {
                    from = end;
                    return Some(self.source_range(start..end));
                }
                from = start + self.text[start..].chars().next().map_or(1, char::len_utf8);
            }
            done = true;
            None
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(locale: &str, s: &str) -> String {
        Folder::new(locale).key(s)
    }

    fn key_accents(locale: &str, s: &str) -> String {
        Folder::new(locale).keep_accents().key(s)
    }

    #[test]
    fn folding_table() {
        let nfd = |s: &str| DecomposingNormalizerBorrowed::new_nfd().normalize(s).into_owned();
        // (locale, input, ignoring accents, keeping accents)
        let table = [
            ("en", "Hello WORLD", "hello world".to_owned(), "hello world".to_owned()),
            ("en", "İSTANBUL", "istanbul".into(), "istanbul".into()),
            ("tr", "İSTANBUL", "istanbul".into(), "istanbul".into()),
            ("en", "IRMAK ırmak", "irmak irmak".into(), "irmak ırmak".into()),
            ("tr", "IRMAK ırmak", "irmak irmak".into(), "ırmak ırmak".into()),
            ("az", "İIıi", "iiii".into(), "iııi".into()),
            ("tr-TR", "Şişli", "sisli".into(), nfd("şişli")),
            ("de", "Straße STRAẞE", "strasse strasse".into(), "strasse strasse".into()),
            ("el", "ΟΔΥΣΣΕΥΣ οδυσσευς", "οδυσσευσ οδυσσευσ".into(), "οδυσσευσ οδυσσευσ".into()),
            ("el", "Ἀθῆναι", "αθηναι".into(), nfd("ἀθῆναι")),
            ("fr", "Œuvre ÉTÉ", "oeuvre ete".into(), nfd("œuvre été")),
            ("da", "Ærø", "aero".into(), nfd("ærø")),
            ("pl", "Łódź", "lodz".into(), nfd("łódź")),
            ("vi", "Đà Nẵng", "da nang".into(), nfd("đà nẵng")),
            ("en", "ﬁnal ﬀ", "final ff".into(), "final ff".into()),
            ("ja", "ＡＢＣ１２３　ｶﾞ", nfd("abc123 ガ"), nfd("abc123 ガ")),
            ("ar", "مُحَمَّد أحمـــد", nfd("محمد احمد"), nfd("مُحَمَّد أحمد")),
            ("he", "שָׁלוֹם", "שלום".into(), nfd("שָׁלוֹם")),
            ("en", "soft\u{AD}ware \u{2068}name\u{2069}", "software name".into(), "software name".into()),
            ("ru", "Ёлка", "елка".into(), nfd("ёлка")),
            ("ar", "Layer ١٢ ۳۴ १२", "layer 12 34 12".into(), "layer 12 34 12".into()),
            ("fa", "کتاب یک", "كتاب يك".into(), "کتاب یک".into()),
            ("fa", "خانۀ", "خانه".into(), nfd("خانۀ")),
            ("ar", "مدرسة على إسلام آمن", "مدرسه علي اسلام امن".into(), nfd("مدرسة على إسلام آمن")),
        ];
        for (locale, input, ignoring, keeping) in table {
            assert_eq!(key(locale, input), ignoring, "{locale} {input}");
            assert_eq!(key_accents(locale, input), keeping, "{locale} {input}, accents kept");
        }
    }

    #[test]
    fn turkish_i_depends_on_locale_when_accents_count() {
        let tr = Folder::new("tr").keep_accents();
        let en = Folder::new("en").keep_accents();
        assert!(tr.contains("IRMAK", "ırmak") && !tr.contains("IRMAK", "irmak"));
        assert!(en.contains("IRMAK", "irmak") && !en.contains("IRMAK", "ırmak"));
        assert!(tr.contains("İSTANBUL", "istanbul") && en.contains("İSTANBUL", "istanbul"));
        for locale in ["tr", "en"] {
            let f = Folder::new(locale);
            assert!(f.contains("IRMAK", "ırmak") && f.contains("ırmak", "IRMAK") && f.contains("İzmir", "izmir"));
        }
    }

    #[test]
    fn matches_map_back_to_the_source() {
        let f = Folder::new("en");
        fn find<'a>(f: Folder, haystack: &'a str, needle: &str) -> Option<&'a str> {
            f.fold(haystack).find(&f.fold(needle)).map(|r| &haystack[r])
        }
        let find = |haystack, needle| find(f, haystack, needle);
        assert_eq!(find("Große Straße", "strasse"), Some("Straße"));
        assert_eq!(find("Große Straße", "SS"), Some("ß"));
        assert_eq!(find("Große Straße", "sse"), Some("ße"));
        assert_eq!(find("Café crème", "creme"), Some("crème"));
        assert_eq!(find("Cafe\u{301} noir", "cafe"), Some("Cafe\u{301}"));
        assert_eq!(find("Ｆｉｌｅ ２", "file 2"), Some("Ｆｉｌｅ ２"));
        assert_eq!(find("the ﬁnal cut", "fin"), Some("ﬁn"));
        assert_eq!(find("Layer \u{2068}İSTANBUL\u{2069}", "istanbul"), Some("İSTANBUL"));
        assert_eq!(find("abc", "x"), None);
        assert_eq!(find("abc", ""), Some(""));

        let all: Vec<_> = f.fold("Ab ab AB").matches(&f.fold("ab")).collect();
        assert_eq!(all, [0..2, 3..5, 6..8]);
    }

    #[test]
    fn persian_and_arabic_letters_match() {
        let f = Folder::new("fa");
        // Persian keheh and Farsi yeh against Arabic kaf and yeh, and hamza forms of alef.
        assert!(f.contains("كتاب عربي", "کتاب عربی"));
        assert!(f.contains("کتاب عربی", "كتاب عربي"));
        assert!(f.contains("أحمد إبراهيم", "احمد ابراهیم"));
        assert!(f.contains("مدرسة", "مدرسه"));
        assert_eq!(f.fold("Layer ١٢").find(&f.fold("layer 12")), Some(0.."Layer ١٢".len()));
        assert!(Folder::new("en").keep_accents().contains("Layer ١٢", "layer 12"));
        let digits = ZEROS.iter().map(|&zero| char::from_u32(zero + 7).unwrap());
        assert!(digits.clone().all(|d| d.is_numeric() && ascii_digit(d) == Some('7')));
        assert_eq!(ascii_digit('\u{1D800}'), None);
        assert_eq!(ascii_digit('/'), None);
    }

    #[test]
    fn accents_count_only_whole_letters() {
        let f = Folder::new("fr").keep_accents();
        assert!(!f.contains("ré", "re"));
        assert!(f.contains("ré", "RÉ") && f.contains("re\u{301}", "ré"));
        assert!(f.contains("ré", "é"));
        // Kana voicing marks stay even with accents ignored: か doesn't find が.
        assert!(!Folder::new("ja").contains("が", "か"));
        // Nor does a Hangul syllable without its final consonant: 하 doesn't find 한.
        let ko = Folder::new("ko");
        assert!(!ko.contains("한국", "하") && !ko.keep_accents().contains("한국", "하"));
        assert!(ko.contains("한국", "한") && ko.contains("하늘", "하"));
        // Conjoining jamo spell the same syllable; compatibility jamo stand alone.
        assert!(!ko.contains("\u{1112}\u{1161}\u{11AB}", "하") && ko.contains("\u{1112}\u{1161}\u{11AB}", "한"));
        assert!(!ko.contains("하\u{11AB}", "하"));
        for (haystack, needle) in [("ㅠㅠ", "ㅠ"), ("ㅠㅠ", "ㅠㅠ"), ("ㅋㅠ", "ㅋ"), ("ㅋㅠ", "ㅠ"), ("ㅋㅋㅋ", "ㅋㅋ"), ("ㄳ", "ㄳ"), ("값ㄳ", "ㄳ")]
        {
            assert!(ko.contains(haystack, needle), "{haystack} {needle}");
        }
        assert_eq!(ko.fold("ㅋㅠㅠ").find(&ko.fold("ㅠ")).map(|r| r.start), Some("ㅋ".len()));
    }

    #[test]
    fn source_ranges_round_trip() {
        let f = Folder::new("de");
        for s in ["Straße", "ﬃ Ａ", "e\u{301}\u{AD}x", "", "İı", "\u{301}a"] {
            let folded = f.fold(s);
            let text = folded.as_str();
            // A leading mark folds to nothing and has no part in any match.
            let whole = if s.starts_with('\u{301}') { 2..s.len() } else { 0..s.len() };
            assert_eq!(folded.source_range(0..text.len()), whole, "{s:?}");
            // Every folded character maps to a non-empty source range that folds back to text
            // containing it.
            for (i, c) in text.char_indices() {
                let r = folded.source_range(i..i + c.len_utf8());
                assert!(!r.is_empty() && f.key(&s[r.clone()]).contains(c), "{s:?} {i} {r:?}");
            }
        }
    }

    #[test]
    fn folding_is_idempotent() {
        // A folded query folds to itself, so a stored key can be searched with a fresh fold.
        for locale in ["en", "tr"] {
            for f in [Folder::new(locale), Folder::new(locale).keep_accents()] {
                for c in ('\0'..='\u{FFFF}').chain('\u{1D400}'..='\u{1D7FF}') {
                    let once = f.key(c.encode_utf8(&mut [0; 4]));
                    assert_eq!(f.key(&once), once, "{locale} {:?} U+{:04X}", f, c as u32);
                }
            }
        }
    }
}
