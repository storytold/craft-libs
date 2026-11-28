//! Spelling: Edit › Check Spelling, and the suggestions in the type tool's context menu
//! over a misspelled word.
//!
//! [`Dictionary::english`] is SCOWL size 50 (American English, about 72,000 words), deflated into
//! the binary and decoded on first use. Words are grouped by SCOWL level (10 = most common … 50),
//! which ranks suggestions. Other languages load their own list with [`Dictionary::from_list`].
//!
//! Case rules follow common spell checkers: a lowercase dictionary word may be written
//! capitalised or in all caps; a capitalised entry (a proper noun) must keep its capital. A
//! trailing possessive `'s` is ignored, typographic apostrophes count as `'`, and tokens with
//! digits are skipped. The user's own words ([`UserWords`], the dialog's Add button) are accepted
//! in any case.
//!
//! ```ignore
//! let user = UserWords::from_words(settings.user_words.clone());
//! for m in Dictionary::english().misspellings("Teh cat", &user) {
//!     let suggestions = Dictionary::english().suggest(&m.word, 8);
//! }
//! ```

use std::collections::HashMap;
use std::ops::Range;
use std::sync::OnceLock;

static WORDS: &[u8] = include_bytes!("../assets/spell/en-us-scowl-50.txt.deflate");

/// A word list with frequency levels.
pub struct Dictionary {
    /// Word (as listed) → SCOWL level.
    words: HashMap<String, u8>,
    /// Lowercase form → listed forms, for case-insensitive lookups and suggestions.
    by_lower: HashMap<String, Vec<String>>,
    /// Lowercase words grouped by char length, with their chars and level, for suggestions.
    by_len: Vec<Vec<(String, Vec<char>, u8)>>,
}

/// A misspelled word in a checked string.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Misspelling {
    /// Byte range in the checked text.
    pub range: Range<usize>,
    /// The word as written.
    pub word: String,
}

/// The user's own dictionary: words added with the Check Spelling dialog's Add button, accepted
/// in any case. Kept in the order the dialog lists them (case-insensitive alphabetical); the app
/// stores [`words`](Self::words) in its settings (`ui::spelling::SpellingSettings`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct UserWords {
    /// As added, sorted by their lowercase form, no two equal ignoring case.
    words: Vec<String>,
}

impl UserWords {
    /// From a stored list; words are trimmed with typographic apostrophes made straight, as
    /// [`Dictionary::check`] reads them, empty ones and duplicates (ignoring case) are dropped and
    /// the rest sorted.
    // pc: crates/engine/src/cmd/type_spell_cmds.rs:75 (the lowercase set built from
    // `prefs.user_dictionary`).
    pub fn from_words(words: impl IntoIterator<Item = String>) -> Self {
        let mut words: Vec<String> = words.into_iter().map(|w| stored(&w)).filter(|w| !w.is_empty()).collect();
        words.sort_by_cached_key(|w| w.to_lowercase());
        words.dedup_by_key(|w| w.to_lowercase());
        UserWords { words }
    }

    /// Add `word` (trimmed, apostrophes straight); `false` when it is empty or already there in
    /// any case.
    // pc: crates/engine/src/cmd/type_spell_cmds.rs:185 (case-insensitive check, then sorted by
    // lowercase).
    pub fn add(&mut self, word: &str) -> bool {
        let word = stored(word);
        if word.is_empty() {
            return false;
        }
        let lower = word.to_lowercase();
        match self.words.binary_search_by_key(&lower, |w| w.to_lowercase()) {
            Ok(_) => false,
            Err(at) => {
                self.words.insert(at, word);
                true
            }
        }
    }

    /// Remove `word` in any case; `false` when it wasn't there.
    // new: PhotoCraft has no removal; Photoshop's dictionary editing is outside the app.
    pub fn remove(&mut self, word: &str) -> bool {
        let lower = stored(word).to_lowercase();
        match self.words.binary_search_by_key(&lower, |w| w.to_lowercase()) {
            Ok(at) => {
                self.words.remove(at);
                true
            }
            Err(_) => false,
        }
    }

    /// Whether `word` (possessive and apostrophes normalised as in [`Dictionary::check`]) is one
    /// of the user's words, ignoring case.
    // pc: crates/text/src/spell.rs:89 `check` (the `user` set lookup).
    pub fn contains(&self, word: &str) -> bool {
        let w = normalize(word);
        self.contains_lower(&strip_word(&w).to_lowercase())
    }

    fn contains_lower(&self, lower: &str) -> bool {
        self.words.binary_search_by(|w| w.to_lowercase().as_str().cmp(lower)).is_ok()
    }

    /// The words, for the app's settings.
    pub fn words(&self) -> &[String] {
        &self.words
    }
}

impl Dictionary {
    /// The bundled US English dictionary (SCOWL 50), decoded once.
    // pc: crates/text/src/spell.rs:39 `english`. The asset is
    // `assets/spell/en-us-scowl-50.txt.deflate` (raw deflate, as `hyphen::Dictionary`'s data;
    // PhotoCraft ships gzip), decoded with `miniz_oxide::inflate::decompress_to_vec`.
    pub fn english() -> &'static Dictionary {
        static D: OnceLock<Dictionary> = OnceLock::new();
        D.get_or_init(|| {
            let raw = miniz_oxide::inflate::decompress_to_vec(WORDS).unwrap_or_default();
            Dictionary::from_list(&String::from_utf8_lossy(&raw))
        })
    }

    /// A dictionary from one word per line; a `#NN` line sets the SCOWL level of the words after
    /// it (50 before any).
    // pc: crates/text/src/spell.rs:49 `from_list`.
    pub fn from_list(list: &str) -> Dictionary {
        let mut words = HashMap::new();
        let mut by_lower: HashMap<String, Vec<String>> = HashMap::new();
        let mut level = 50u8;
        for line in list.lines() {
            let w = line.trim();
            if let Some(l) = w.strip_prefix('#') {
                level = l.parse().unwrap_or(level);
                continue;
            }
            if w.is_empty() || words.contains_key(w) {
                continue;
            }
            words.insert(w.to_string(), level);
            by_lower.entry(w.to_lowercase()).or_default().push(w.to_string());
        }
        let mut by_len: Vec<Vec<(String, Vec<char>, u8)>> = Vec::new();
        for (lw, forms) in &by_lower {
            let n = lw.chars().count();
            if by_len.len() <= n {
                by_len.resize(n + 1, Vec::new());
            }
            let lvl = forms.iter().filter_map(|f| words.get(f)).min().copied().unwrap_or(50);
            by_len[n].push((lw.clone(), lw.chars().collect(), lvl));
        }
        for v in &mut by_len {
            v.sort();
        }
        Dictionary { words, by_lower, by_len }
    }

    /// The number of words.
    pub fn len(&self) -> usize {
        self.words.len()
    }

    /// Whether the dictionary has no words.
    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }

    /// Whether `word` is spelled correctly: in the dictionary under the case rules above, one of
    /// `user`'s words, or a token the checker skips (one with a digit, or with no letter).
    // pc: crates/text/src/spell.rs:89 `check`.
    pub fn check(&self, word: &str, user: &UserWords) -> bool {
        let w = normalize(word);
        let w = strip_word(&w);
        if w.is_empty() || w.chars().any(|c| c.is_numeric()) || !w.chars().any(char::is_alphabetic) {
            return true;
        }
        if self.words.contains_key(w) || user.contains_lower(&w.to_lowercase()) {
            return true;
        }
        let Some(forms) = self.by_lower.get(&w.to_lowercase()) else {
            return false;
        };
        let all_caps = w.chars().filter(|c| c.is_alphabetic()).all(char::is_uppercase);
        let first_cap = w.chars().next().is_some_and(char::is_uppercase) && w.chars().skip(1).all(|c| !c.is_uppercase());
        forms.iter().any(|f| {
            let f_lower = f.chars().all(|c| !c.is_uppercase());
            // "the" → "The"/"THE"; "Paris" → "PARIS"; "NASA" stays as listed (exact match above).
            all_caps || (f_lower && first_cap) || (first_cap && capitalize(f) == w)
        })
    }

    /// Up to `max` suggestions for `word`: dictionary words within edit distance 1 (words of up to
    /// four letters) or 2 (Damerau: a transposition counts 1), nearest first; among equals, the
    /// same letters in another order first, then the most common (lowest SCOWL level). Written in
    /// `word`'s case (capitalised or all caps).
    // pc: crates/text/src/spell.rs:114 `suggest` and :220 `distance`.
    pub fn suggest(&self, word: &str, max: usize) -> Vec<String> {
        let w = normalize(word);
        let lower = w.to_lowercase();
        let lc: Vec<char> = lower.chars().collect();
        let n = lc.len();
        let max_d = if n <= 4 { 1 } else { 2 };
        // Same letters in another order (transposed typing) ranks first among equal distances.
        let mut sorted = lc.clone();
        sorted.sort_unstable();
        let anagram = |c: &str| {
            let mut v: Vec<char> = c.chars().collect();
            v.sort_unstable();
            v == sorted
        };
        let mut found: Vec<(usize, bool, u8, &str)> = Vec::new();
        let mut rows = Rows::default();
        for len in n.saturating_sub(max_d)..=(n + max_d) {
            let Some(bucket) = self.by_len.get(len) else {
                continue;
            };
            for (cand, cc, lvl) in bucket {
                if let Some(d) = distance(&lc, cc, max_d, &mut rows) {
                    found.push((d, !anagram(cand), *lvl, cand));
                }
            }
        }
        found.sort();
        let letters = w.chars().filter(|c| c.is_alphabetic());
        let all_caps = letters.clone().all(char::is_uppercase) && letters.count() > 1;
        let first_cap = w.chars().next().is_some_and(char::is_uppercase);
        let mut out: Vec<String> = Vec::new();
        for (_, _, _, cand) in found {
            // Common (lowercase) forms before proper nouns: "brwon" → "brown", then "Brown".
            let mut forms: Vec<&String> = self.by_lower.get(cand).into_iter().flatten().collect();
            forms.sort_by_key(|f| f.chars().any(char::is_uppercase));
            for form in forms {
                let s = if all_caps {
                    form.to_uppercase()
                } else if first_cap {
                    capitalize(form)
                } else {
                    form.clone()
                };
                if !out.contains(&s) && s != w {
                    out.push(s);
                }
            }
            if out.len() >= max {
                break;
            }
        }
        out.truncate(max);
        out
    }

    /// Every misspelled word in `text`, in order.
    // pc: crates/text/src/spell.rs:169 `misspellings`.
    pub fn misspellings(&self, text: &str, user: &UserWords) -> Vec<Misspelling> {
        words(text)
            .into_iter()
            .filter(|r| !self.check(&text[r.clone()], user))
            .map(|r| Misspelling { word: text[r.clone()].to_string(), range: r })
            .collect()
    }
}

/// The words of `text` as byte ranges: runs of letters and digits with inner apostrophes
/// (straight, typographic or U+02BC), so "don’t" is one word; hyphens split ("well-known" is two).
// pc: crates/text/src/spell.rs:192 `words`.
pub fn words(text: &str) -> Vec<Range<usize>> {
    let is_apos = |c: char| matches!(c, '\'' | '\u{2019}' | '\u{02BC}');
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    let chars: Vec<(usize, char)> = text.char_indices().collect();
    for (i, &(b, c)) in chars.iter().enumerate() {
        let inner_apos = is_apos(c) && start.is_some() && chars.get(i + 1).is_some_and(|(_, n)| n.is_alphabetic());
        if c.is_alphanumeric() || inner_apos {
            start.get_or_insert(b);
        } else if let Some(s) = start.take() {
            out.push(s..b);
        }
    }
    if let Some(s) = start {
        out.push(s..text.len());
    }
    out
}

/// A word as the user dictionary keeps it.
fn stored(w: &str) -> String {
    normalize(w).trim().to_owned()
}

/// Typographic apostrophes → `'`.
fn normalize(w: &str) -> String {
    w.replace(['\u{2019}', '\u{2018}', '\u{02BC}'], "'")
}

/// A normalised token without its trailing possessive `'s` and surrounding apostrophes.
fn strip_word(w: &str) -> &str {
    let w = w.strip_suffix("'s").or_else(|| w.strip_suffix("'S")).filter(|b| !b.is_empty()).unwrap_or(w);
    w.trim_matches('\'')
}

fn capitalize(w: &str) -> String {
    let mut c = w.chars();
    match c.next() {
        Some(f) => f.to_uppercase().chain(c).collect(),
        None => String::new(),
    }
}

/// Scratch rows for [`distance`].
#[derive(Default)]
struct Rows {
    prev2: Vec<usize>,
    prev: Vec<usize>,
    cur: Vec<usize>,
}

/// Damerau–Levenshtein (optimal string alignment) distance, or `None` above `max`.
fn distance(a: &[char], b: &[char], max: usize, rows: &mut Rows) -> Option<usize> {
    let (n, m) = (a.len(), b.len());
    if n.abs_diff(m) > max {
        return None;
    }
    let Rows { prev2, prev, cur } = rows;
    prev2.clear();
    prev2.resize(m + 1, 0);
    prev.clear();
    prev.extend(0..=m);
    cur.clear();
    cur.resize(m + 1, 0);
    for i in 1..=n {
        cur[0] = i;
        let mut row_min = cur[0];
        for j in 1..=m {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            let mut v = (prev[j] + 1).min(cur[j - 1] + 1).min(prev[j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                v = v.min(prev2[j - 2] + 1);
            }
            cur[j] = v;
            row_min = row_min.min(v);
        }
        if row_min > max {
            return None;
        }
        std::mem::swap(prev2, prev);
        std::mem::swap(prev, cur);
    }
    (prev[m] <= max).then_some(prev[m])
}
