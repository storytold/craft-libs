//! Hyphenation points for text layout. English uses the bundled dictionary and patterns below.
//! Other languages use the Liang patterns compiled into [`hypher`], one cargo feature per
//! language (`spanish`, `german`, …, or `all-languages`); their licenses are listed in NOTICE.
//!
//! English lookup order for each word:
//! 1. the **dictionary** ([`Dictionary::en_us`]) — the public-domain Moby Hyphenator II word list
//!    by Grady Ward (~160k words), stored compactly in `assets/hyphen/en-us.dic`; simple
//!    inflections (`'s`, `-s`, `-es`, `-ed`, `-d`) inherit the breaks of their stem;
//! 2. **Liang patterns** (`assets/hyphen/en-us.pat`) for words not in the dictionary — our own
//!    pattern set, trained from the same list by the patgen-style trainer behind the `generator`
//!    feature (`examples/hyphgen.rs` regenerates both files);
//! 3. a small syllable **heuristic** for words outside the pattern alphabet.
//!
//! Then the paragraph's limits ([`Limits`]) are applied.

#[cfg(feature = "generator")]
pub mod dict;
#[cfg(not(feature = "generator"))]
mod dict;
#[cfg(feature = "generator")]
pub mod patgen;
mod patterns;

use std::sync::OnceLock;

pub use dict::Dictionary;
#[cfg(feature = "generator")]
pub use patterns::Patterns;
#[cfg(not(feature = "generator"))]
use patterns::Patterns;

static EN_US_DIC: &[u8] = include_bytes!("../../assets/hyphen/en-us.dic");
static EN_US_PAT: &str = include_str!("../../assets/hyphen/en-us.pat");

impl Dictionary {
    /// The bundled US English dictionary (decoded on first use; empty if the data is corrupt).
    pub fn en_us() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| Dictionary::decode(EN_US_DIC).unwrap_or_default())
    }
}

impl Patterns {
    /// The bundled US English patterns (parsed on first use).
    pub fn en_us() -> &'static Patterns {
        static P: OnceLock<Patterns> = OnceLock::new();
        P.get_or_init(|| Patterns::parse(EN_US_PAT))
    }
}

/// Hyphenation limits from the paragraph's Hyphenation settings.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub min_word: usize,
    pub after_first: usize,
    pub before_last: usize,
    pub capitalized: bool,
}

impl Default for Limits {
    fn default() -> Self {
        Limits { min_word: 5, after_first: 2, before_last: 2, capitalized: true }
    }
}

/// The hyphenator for a run of text.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Lang {
    /// The bundled dictionary, patterns and heuristic.
    English,
    /// Patterns compiled into [`hypher`] (one cargo feature per language).
    Patterns(PatternLang),
}

/// A language whose patterns are compiled into [`hypher`]; obtained from [`Lang::for_language`].
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct PatternLang(hypher::Lang);

/// English names of every language hypher can hyphenate, with their ISO 639-1 codes. Languages
/// whose cargo feature is off are not compiled into hypher, so `from_iso` returns None for them.
const LANGUAGES: &[(&str, [u8; 2])] = &[
    ("afrikaans", *b"af"),
    ("albanian", *b"sq"),
    ("assamese", *b"as"),
    ("belarusian", *b"be"),
    ("bengali", *b"bn"),
    ("bangla", *b"bn"),
    ("bulgarian", *b"bg"),
    ("catalan", *b"ca"),
    ("croatian", *b"hr"),
    ("czech", *b"cs"),
    ("danish", *b"da"),
    ("dutch", *b"nl"),
    ("estonian", *b"et"),
    ("finnish", *b"fi"),
    ("french", *b"fr"),
    ("galician", *b"gl"),
    ("georgian", *b"ka"),
    ("german", *b"de"),
    ("greek", *b"el"),
    ("gujarati", *b"gu"),
    ("hindi", *b"hi"),
    ("hungarian", *b"hu"),
    ("icelandic", *b"is"),
    ("italian", *b"it"),
    ("kannada", *b"kn"),
    ("kurmanji", *b"ku"),
    ("latin", *b"la"),
    ("lithuanian", *b"lt"),
    ("malayalam", *b"ml"),
    ("marathi", *b"mr"),
    ("mongolian", *b"mn"),
    ("norwegian", *b"no"),
    ("bokmål", *b"nb"),
    ("bokmal", *b"nb"),
    ("nynorsk", *b"nn"),
    ("oriya", *b"or"),
    ("odia", *b"or"),
    ("panjabi", *b"pa"),
    ("punjabi", *b"pa"),
    ("polish", *b"pl"),
    ("portuguese", *b"pt"),
    ("russian", *b"ru"),
    ("sanskrit", *b"sa"),
    ("serbian", *b"sr"),
    ("slovak", *b"sk"),
    ("slovenian", *b"sl"),
    ("slovene", *b"sl"),
    ("spanish", *b"es"),
    ("castilian", *b"es"),
    ("swedish", *b"sv"),
    ("tamil", *b"ta"),
    ("telugu", *b"te"),
    ("turkish", *b"tr"),
    ("turkmen", *b"tk"),
    ("ukrainian", *b"uk"),
];

impl Lang {
    /// The hyphenator for a language name or locale code: an English name as InDesign and IDML
    /// write it ("Spanish", "German: 2006 Reform", "$ID/Norwegian: Bokmal"), or a locale code
    /// (`es`, `es_ES`, `pt-BR`, `de_DE_2006`). Case and punctuation don't matter. None when the
    /// language is unknown or its cargo feature is off.
    pub fn for_language(language: &str) -> Option<Lang> {
        if language.starts_with("English") {
            return Some(Lang::English);
        }
        let language = language.trim();
        let language = language.strip_prefix("$ID/").unwrap_or(language);
        let l = language.to_lowercase();
        let words: Vec<&str> = l.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty()).collect();
        let first = *words.first()?;
        let code = match locale_language(&words) {
            Some("en") => return Some(Lang::English),
            Some(code) => <[u8; 2]>::try_from(code.as_bytes()).ok()?,
            None if first == "english" => return Some(Lang::English),
            None => LANGUAGES.iter().find(|(name, _)| *name == first)?.1,
        };
        hypher::Lang::from_iso(code).map(|l| Lang::Patterns(PatternLang(l)))
    }
}

/// The language of a locale code split into lowercase `words`: a language (2 or 3 letters), then
/// optionally a region (2 letters or 3 digits) and a reform year (4 digits).
fn locale_language<'a>(words: &[&'a str]) -> Option<&'a str> {
    let letters = |w: &str, n: std::ops::RangeInclusive<usize>| n.contains(&w.len()) && w.bytes().all(|b| b.is_ascii_lowercase());
    let digits = |w: &str, n: usize| w.len() == n && w.bytes().all(|b| b.is_ascii_digit());
    match *words {
        [lang] if letters(lang, 2..=3) => Some(lang),
        [lang, region] if letters(lang, 2..=3) && (letters(region, 2..=2) || digits(region, 3)) => Some(lang),
        [lang, region, year] if letters(lang, 2..=3) && (letters(region, 2..=2) || digits(region, 3)) && digits(year, 4) => Some(lang),
        _ => None,
    }
}

/// Where a word's break points came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Source {
    Dictionary,
    Patterns,
    Heuristic,
    None,
}

fn is_apostrophe(c: char) -> bool {
    matches!(c, '\'' | '’' | 'ʼ')
}

/// Letters, apostrophes, and the combining marks and joiners inside a word: a virama or a
/// decomposed accent doesn't split it.
fn is_word_char(c: char) -> bool {
    c.is_alphabetic() || is_apostrophe(c) || is_mark(c) || is_joiner(c)
}

/// A combining mark (general category M): it belongs to the letter before it.
fn is_mark(c: char) -> bool {
    use unicode_general_category::{GeneralCategory, get_general_category};
    matches!(get_general_category(c), GeneralCategory::NonspacingMark | GeneralCategory::SpacingMark | GeneralCategory::EnclosingMark)
}

/// Zero-width joiner and non-joiner, which shape the letters on both sides.
fn is_joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// Unrestricted break points of one lowercase core (letters and `'` only, no leading/trailing `'`).
fn core_breaks(core: &[char], lang: Lang) -> (Vec<usize>, Source) {
    let n = core.len();
    if n < 2 {
        return (vec![], Source::None);
    }
    if let Lang::Patterns(PatternLang(l)) = lang {
        // The whole core, apostrophes included: French, Italian and Catalan patterns encode
        // elisions (`l'or-ga-ni-sa-tion`, `del-l'am-mi-ni-stra-zio-ne`).
        let s: String = core.iter().collect();
        let mut out = Vec::new();
        let mut at = 0;
        let syl: Vec<&str> = hypher::hyphenate_bounded(&s, l, 1, 1).collect();
        if let Some((_, init)) = syl.split_last() {
            for part in init {
                at += part.chars().count();
                out.push(at);
            }
        }
        return (out, Source::Patterns);
    }
    let dict = Dictionary::en_us();
    let s: String = core.iter().collect();
    let from_mask = |m: u64, len: usize| (1..len.min(64)).filter(|&i| m & (1 << i) != 0).collect::<Vec<_>>();
    if let Some(m) = dict.get(&s) {
        return (from_mask(m, n), Source::Dictionary);
    }
    // Inflections inherit the stem's breaks (the suffix adds no break of its own).
    for suf in ["'s", "s", "es", "ed", "d"] {
        if let Some(stem) = s.strip_suffix(suf)
            && stem.chars().count() >= 2
            && let Some(m) = dict.get(stem)
        {
            return (from_mask(m, stem.chars().count()), Source::Dictionary);
        }
    }
    let pats = Patterns::en_us();
    if !pats.is_empty()
        && let Some(p) = pats.points(core)
    {
        return (p, Source::Patterns);
    }
    (heuristic_points(core), Source::Heuristic)
}

/// Unrestricted break points of `word` as byte offsets (a hyphen goes before the char that starts
/// there), handling case, apostrophes (`don't`, `’s`), surrounding punctuation and compounds (each
/// letter run is hyphenated separately). No break is ever placed next to an apostrophe or
/// non-letter, before a combining mark or next to a joiner; marks and joiners don't end a run.
pub fn word_breaks(word: &str) -> (Vec<usize>, Source) {
    word_breaks_in(word, Lang::English)
}

/// [`word_breaks`] for a language.
pub fn word_breaks_in(word: &str, lang: Lang) -> (Vec<usize>, Source) {
    let (pts, src) = char_breaks(word, lang);
    (byte_offsets(word, pts), src)
}

/// Char indices into `word` as byte offsets.
fn byte_offsets(word: &str, pts: Vec<usize>) -> Vec<usize> {
    let starts: Vec<usize> = word.char_indices().map(|(b, _)| b).collect();
    pts.into_iter().filter_map(|p| starts.get(p).copied()).collect()
}

/// [`word_breaks_in`] as char indices.
fn char_breaks(word: &str, lang: Lang) -> (Vec<usize>, Source) {
    let chars: Vec<char> = word.chars().collect();
    let mut out = Vec::new();
    let mut src = Source::None;
    let mut i = 0;
    while i < chars.len() {
        if !is_word_char(chars[i]) {
            i += 1;
            continue;
        }
        let mut j = i;
        while j < chars.len() && is_word_char(chars[j]) {
            j += 1;
        }
        let (mut a, mut b) = (i, j);
        while a < b && is_apostrophe(chars[a]) {
            a += 1;
        }
        while b > a && is_apostrophe(chars[b - 1]) {
            b -= 1;
        }
        let core: Vec<char> = chars[a..b].iter().map(|&c| if is_apostrophe(c) { '\'' } else { c.to_lowercase().next().unwrap_or(c) }).collect();
        let (pts, s) = core_breaks(&core, lang);
        if src == Source::None {
            src = s;
        }
        // Never before a mark or next to a joiner: that would split a shaping cluster.
        let allowed = |p: usize| match (core.get(p.wrapping_sub(1)), core.get(p)) {
            (Some(&x), Some(&y)) => x != '\'' && y != '\'' && !is_mark(y) && !is_joiner(x) && !is_joiner(y),
            _ => false,
        };
        out.extend(pts.into_iter().filter(|&p| p > 0 && allowed(p)).map(|p| a + p));
        i = j;
    }
    (out, src)
}

/// Allowed hyphenation points in `word` as byte offsets (a hyphen goes before the char that
/// starts there), with the paragraph limits applied to each letter run.
pub fn hyphen_points(word: &str, lim: &Limits) -> Vec<usize> {
    hyphen_points_in(word, lim, Lang::English)
}

/// [`hyphen_points`] for a language.
pub fn hyphen_points_in(word: &str, lim: &Limits, lang: Lang) -> Vec<usize> {
    byte_offsets(word, char_points(word, lim, lang))
}

/// [`hyphen_points_in`] as char indices.
fn char_points(word: &str, lim: &Limits, lang: Lang) -> Vec<usize> {
    let chars: Vec<char> = word.chars().collect();
    let (pts, _) = char_breaks(word, lang);
    if pts.is_empty() {
        return pts;
    }
    let min_before = lim.after_first.max(1);
    let min_after = lim.before_last.max(1);
    // Counted once, so each point's checks take constant time however long the word: letters and
    // uppercase letters before each char, and the letter run (trimmed of apostrophes) of each char.
    let (mut letters, mut upper) = (Vec::with_capacity(chars.len() + 1), Vec::with_capacity(chars.len() + 1));
    let (mut l, mut u) = (0usize, 0usize);
    for c in &chars {
        letters.push(l);
        upper.push(u);
        if c.is_alphabetic() {
            l += 1;
            u += usize::from(c.is_uppercase());
        }
    }
    letters.push(l);
    upper.push(u);
    let mut run_of: Vec<(usize, usize)> = vec![(0, 0); chars.len()];
    let mut i = 0;
    while i < chars.len() {
        let mut j = i;
        while chars.get(j).is_some_and(|c| is_word_char(*c)) {
            j += 1;
        }
        let (mut a, mut b) = (i, j);
        while a < b && chars.get(a).is_some_and(|c| is_apostrophe(*c)) {
            a += 1;
        }
        while b > a && chars.get(b - 1).is_some_and(|c| is_apostrophe(*c)) {
            b -= 1;
        }
        for r in run_of.get_mut(i..j).into_iter().flatten() {
            *r = (a, b);
        }
        i = j.max(i + 1);
    }
    let count = |v: &[usize], a: usize, b: usize| v.get(b).zip(v.get(a)).map_or(0, |(y, x)| y.saturating_sub(*x));
    pts.into_iter()
        .filter(|&p| {
            // A point lies between two letters of one run; the char after it gives the run.
            let Some(&(a, b)) = run_of.get(p) else { return false };
            let n = count(&letters, a, b);
            if n < lim.min_word.max(2) {
                return false;
            }
            if n > 1 && count(&upper, a, b) == n {
                return false;
            }
            if !lim.capitalized && chars.get(a).is_some_and(|c| c.is_uppercase()) {
                return false;
            }
            count(&letters, a, p) >= min_before && count(&letters, p, b) >= min_after
        })
        .collect()
}

/// `word` with `-` at every hyphenation point (tests and diagnostics).
pub fn hyphenate_word(word: &str, lim: &Limits) -> String {
    let pts = hyphen_points(word, lim);
    let mut s = String::with_capacity(word.len() + pts.len());
    for (i, ch) in word.char_indices() {
        if pts.contains(&i) {
            s.push('-');
        }
        s.push(ch);
    }
    s
}

// ---------- heuristic fallback (words outside the pattern alphabet) ----------

fn is_vowel(c: char) -> bool {
    matches!(c.to_ascii_lowercase(), 'a' | 'e' | 'i' | 'o' | 'u' | 'y') || (c.is_alphabetic() && !c.is_ascii())
}

fn onset_pair(a: char, b: char) -> bool {
    let (a, b) = (a.to_ascii_lowercase(), b.to_ascii_lowercase());
    matches!(
        (a, b),
        ('c' | 's' | 't' | 'p' | 'w' | 'g', 'h')
            | ('b' | 'c' | 'd' | 'f' | 'g' | 'k' | 'p' | 't', 'r')
            | ('b' | 'c' | 'f' | 'g' | 'k' | 'p' | 's', 'l')
            | ('q', 'u')
            | ('s', 't' | 'p' | 'c' | 'k')
    )
}

/// Syllable heuristic: V-CV ("ty-po"), VC-CV ("hap-pen"), keeping onset clusters ("gra-phy").
fn heuristic_points(c: &[char]) -> Vec<usize> {
    let n = c.len();
    if n < 4 || !c.iter().all(|c| c.is_alphabetic()) {
        return vec![];
    }
    let v: Vec<bool> = c.iter().map(|&x| is_vowel(x)).collect();
    // Both parts need a sounded vowel: a trailing silent `e`, `ed` or `es` doesn't count. The part
    // after `i` is `c[i..end]`, `end` before that ending; computed once, so each point is O(1).
    let end = |i: usize| {
        if n - i >= 2 && (c.ends_with(&['e', 'd']) || c.ends_with(&['e', 's'])) {
            n - 2
        } else if c.ends_with(&['e']) {
            n - 1
        } else {
            n
        }
    };
    // UTF-8 bytes before each char, and the first vowel at or after each char.
    let mut bytes = Vec::with_capacity(n + 1);
    let mut acc = 0usize;
    for ch in c {
        bytes.push(acc);
        acc += ch.len_utf8();
    }
    bytes.push(acc);
    let mut next_vowel = vec![n; n + 1];
    for i in (0..n).rev() {
        next_vowel[i] = if v[i] { i } else { next_vowel[i + 1] };
    }
    let mut out = vec![];
    let mut vowel_before = false;
    for i in 1..n {
        vowel_before |= v[i - 1];
        let ok = if v[i - 1] && !v[i] {
            (i + 1 < n && v[i + 1]) || (i + 2 < n && onset_pair(c[i], c[i + 1]) && v[i + 2])
        } else if !v[i - 1] && !v[i] {
            i >= 2 && v[i - 2] && i + 1 < n && v[i + 1] && !onset_pair(c[i - 1], c[i]) && c[i - 1] != c[i] || (c[i - 1] == c[i] && i >= 2 && v[i - 2])
        } else {
            false
        };
        let e = end(i);
        if ok && next_vowel[i] < e && bytes[e] - bytes[i] > 1 && vowel_before {
            out.push(i);
        }
    }
    out
}

#[cfg(test)]
mod tests;
