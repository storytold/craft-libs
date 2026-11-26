//! The command search's word index and its ranking.
//!
//! Every text is folded with the environment's folder (case and accents) before it is indexed or compared, so a query
//! matches however the label is written.

use super::{HelpEntry, Synonyms};
use crate::runtime::env::{EnvView, Label};
use crate::runtime::words::common::CommonWords;
use craft_host::Host;
use craft_i18n::Folder;
use std::cmp::Reverse;
use std::collections::HashMap;

/// The most characters of a query the index reads. The substring rule and the exact tiers read the whole query.
const INDEX_CHARS: usize = 200;

/// The field of an entry a word comes from, best first, and the fields of the English stem forms ([`stems`]) of the
/// words of the label, the path and the description words.
const LABEL: u8 = 0;
const PATH: u8 = 1;
const ID: u8 = 2;
const LABEL_STEMMED: u8 = 3;
const PATH_STEMMED: u8 = 4;
const DESCRIPTION: u8 = 5;
const DESCRIPTION_STEMMED: u8 = 6;

/// A word's grade in an entry, best first: a whole word of the label, a word of the label it shares an English stem
/// form with, a word of the label it starts, a word of the label with the same meaning, a word of the path it is,
/// shares a stem form with or starts, a word of the path with the same meaning, a word of an id it is or starts, text
/// inside a word of any field, a word of the description words, a word of the label a typo away from it, or a word of
/// the path a typo away. A description comes after the substring rule, so every entry the substring rule finds ranks
/// above one found only by its description.
const LABEL_WORD: u8 = 2;
const LABEL_STEM: u8 = 3;
const LABEL_PREFIX: u8 = 4;
const LABEL_SYNONYM: u8 = 5;
const PATH_WORD: u8 = 6;
const PATH_SYNONYM: u8 = 7;
const ID_WORD: u8 = 8;
const SUBSTRING: u8 = 10;
const DESCRIPTION_WORD: u8 = 11;
const LABEL_TYPO: u8 = 12;
const PATH_TYPO: u8 = 14;
const NONE: u8 = u8::MAX;

/// The words of the description words the index leaves out, besides words of 2 characters or fewer and words with no
/// letter.
const STOP_WORDS: &[&str] = &["the", "and", "ids", "true", "false", "null"];

/// The most words a query word with a typo meets ([`SearchIndex::typos`]), nearest first.
const TYPO_TERMS: usize = 32;

/// How a whole query matches an entry, best first: it is the entry's id, it is a label, a label starts with it, or it
/// is the initials of a label.
const EXACT_ID: u8 = 0;
const EXACT_LABEL: u8 = 1;
const PREFIX: u8 = 2;
const INITIALS: u8 = 3;
const NO_PHRASE: u8 = 4;

/// The most readings of a query [`best_reading`] compares for one entry.
const MAX_READINGS: usize = 16;

/// One entry in the list of a term: the entry's place and the field the term is a word or a word's stem form of.
#[derive(Clone, Copy)]
struct Posting {
    entry: usize,
    field: u8,
}

/// The folded texts of one entry that the whole-query rules read.
struct Names {
    /// The `(label, name with its path)` pairs: the shown label, then the English label when there is one.
    names: Vec<(String, String)>,
    /// The ids the entry is found by.
    ids: Vec<String>,
    /// Every name and id, one per line: the text the substring rule searches.
    haystack: String,
    /// The initials of each label, of two or more letters.
    initials: Vec<String>,
    /// The entry's [`HelpEntry::with_tie_order`].
    tie_order: u8,
    /// The characters of the shown label.
    label_len: usize,
}

/// The word index of the help search: the words of each entry's label, English label, path, ids and words, English
/// stems, a typo bound and synonym expansion.
pub struct SearchIndex {
    folder: Folder,
    /// The path separator, folded: a query that holds it reads the path as part of a name.
    separator: String,
    entries: Vec<Names>,
    /// Every distinct word with its postings, sorted by the word.
    terms: Vec<(String, Vec<Posting>)>,
    /// Each word of a group of words with the same meaning, and the English words' stem forms, with the group's place.
    synonyms: HashMap<Box<str>, usize>,
}

/// Splits `text` into words at whitespace and at `.`, `-`, `/`, `(`, `)`, `…`, `›`, `,` and `:`.
fn words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| c.is_whitespace() || matches!(c, '.' | '-' | '/' | '(' | ')' | '…' | '›' | ',' | ':')).filter(|w| !w.is_empty())
}

/// Is `w` one word as the index splits text?
pub(super) fn is_one_word(w: &str) -> bool {
    let mut it = words(w);
    it.next() == Some(w) && it.next().is_none()
}

/// The first letter of each word of `label`, words split at whitespace and at `-`, `/`, `(` and `›`.
fn initials_of(label: &str) -> String {
    label.split(|c: char| c.is_whitespace() || matches!(c, '-' | '/' | '(' | '›')).filter_map(|w| w.chars().find(|c| c.is_alphanumeric())).collect()
}

/// The English stem forms of the folded `word`, at most two, and none for a word that is not plain English letters.
/// `'s` is dropped; `sses` becomes `ss`; `es` after `s`, `x`, `z`, `ch` or `sh` is dropped, and with only its `s`
/// dropped the word is a second form; in a word of more than 5 letters, `ies` becomes `y` or `ie`; `ing` (more than 5
/// letters) and `ed` (more than 4) are dropped, and when a doubled `b`, `d`, `g`, `m`, `n`, `p`, `r` or `t` is left
/// before them, the rest with that letter made single and the rest as it is are the forms, the single one only when it
/// keeps at least 3 letters; otherwise the rest and the rest followed by `e` are the forms; a final `s` is dropped
/// from a word of more than 3 letters that does not end in `ss`, `us` or `is`. `series` and `species` have none.
fn stems(word: &str) -> Vec<String> {
    let one = |rest: &str| if rest.is_empty() { vec![] } else { vec![rest.to_string()] };
    if !word.bytes().all(|b| b.is_ascii_lowercase() || b == b'\'') || matches!(word, "series" | "species") {
        return vec![];
    }
    let n = word.len();
    if let Some(rest) = word.strip_suffix("'s") {
        return one(rest);
    }
    if let Some(rest) = word.strip_suffix("sses") {
        return vec![format!("{rest}ss")];
    }
    if let Some(rest) = word.strip_suffix("es")
        && ["s", "x", "z", "ch", "sh"].iter().any(|end| rest.ends_with(end))
    {
        return vec![rest.to_string(), format!("{rest}e")];
    }
    if n > 5
        && let Some(rest) = word.strip_suffix("ies")
    {
        return vec![format!("{rest}y"), format!("{rest}ie")];
    }
    let verb = if n > 5 { word.strip_suffix("ing") } else { None }.or_else(|| if n > 4 { word.strip_suffix("ed") } else { None });
    if let Some(rest) = verb {
        if let [.., a, b] = rest.as_bytes()
            && a == b
            && b"bdgmnprt".contains(a)
        {
            let single = rest.get(..rest.len() - 1).unwrap_or_default();
            if single.len() < 3 {
                return vec![rest.to_string()];
            }
            return vec![single.to_string(), rest.to_string()];
        }
        return vec![rest.to_string(), format!("{rest}e")];
    }
    if n > 3 && word.ends_with('s') && !["ss", "us", "is"].iter().any(|end| word.ends_with(end)) {
        return one(word.get(..n - 1).unwrap_or_default());
    }
    vec![]
}

/// Is `c` a letter of a script written without spaces between words (Han, kana or Hangul)?
fn spaceless(c: char) -> bool {
    matches!(
        u32::from(c),
        0x1100..=0x11FF
            | 0x3040..=0x30FF
            | 0x3130..=0x318F
            | 0x31F0..=0x31FF
            | 0x3400..=0x4DBF
            | 0x4E00..=0x9FFF
            | 0xA960..=0xA97F
            | 0xAC00..=0xD7FF
            | 0xF900..=0xFAFF
            | 0xFF66..=0xFFDC
            | 0x20000..=0x323AF
    )
}

/// The most edits a query word may be away from a word it meets as a typo: 1 for a word of 4 to 7 characters, 2 for a
/// longer one, and none for a shorter word or one with letters of a script written without spaces.
fn typo_bound(word: &str) -> Option<usize> {
    if word.chars().any(spaceless) {
        return None;
    }
    match word.chars().count() {
        0..=3 => None,
        4..=7 => Some(1),
        _ => Some(2),
    }
}

/// The number of edits between `a` and `b` when it is at most `bound`: a character inserted, removed or changed, or
/// two adjacent characters swapped, each count as one, and each character inserted between two swapped characters or
/// removed from between them counts as one more. The count stops as soon as every way of lining the two up needs more
/// than `bound`.
fn edits_within(a: &[char], b: &[char], bound: usize) -> Option<usize> {
    if a.len().abs_diff(b.len()) > bound {
        return None;
    }
    let width = b.len() + 1;
    // The edits between the first i characters of `a` and the first j of `b`, at (i % kept) * width + j, for the last
    // `kept` rows: a swap within the bound reaches back at most `bound` + 1 rows.
    let kept = bound.saturating_add(2);
    let mut table: Vec<usize> = (0..width).collect();
    table.resize(kept.min(a.len() + 1) * width, 0);
    let place = |i: usize, j: usize| (i % kept) * width + j;
    let at = |table: &[usize], i: usize, j: usize| table.get(place(i, j)).copied().unwrap_or(usize::MAX);
    for (i, &ca) in (1usize..).zip(a) {
        if let Some(first) = table.get_mut(place(i, 0)) {
            *first = i;
        }
        // The last column so far in this row whose character of `b` is `ca`.
        let mut column = 0;
        for (j, &cb) in (1usize..).zip(b) {
            let changed = at(&table, i - 1, j - 1).saturating_add(usize::from(ca != cb));
            let removed = at(&table, i - 1, j).saturating_add(1);
            let inserted = at(&table, i, j - 1).saturating_add(1);
            let mut edits = changed.min(removed).min(inserted);
            // `cb` is in row k of `a` and `ca` in column `column` of `b`: swap the two, remove the characters of `a`
            // between them and insert the characters of `b` between them. Removing more than `bound` characters goes
            // past the bound, so only the `bound` rows before this one are searched, newest first.
            let k = (i.saturating_sub(bound).max(1)..i).rev().find(|&k| a.get(k - 1) == Some(&cb)).unwrap_or(0);
            if k > 0 && column > 0 {
                edits = edits.min(at(&table, k - 1, column - 1).saturating_add(i - k + (j - column - 1)));
            }
            if ca == cb {
                column = j;
            }
            if let Some(cell) = table.get_mut(place(i, j)) {
                *cell = edits;
            }
        }
        if (0..width).all(|j| at(&table, i, j) > bound) {
            return None;
        }
    }
    Some(at(&table, a.len(), b.len())).filter(|&e| e <= bound)
}

/// Adds each word of `text` to `found` for entry `entry` in `field`, and with `stemmed`, each word's English stem forms
/// in that field.
fn add_words(found: &mut Vec<(String, usize, u8)>, text: &str, entry: usize, field: u8, stemmed: Option<u8>) {
    for w in words(text) {
        found.push((w.to_string(), entry, field));
        if let Some(stemmed) = stemmed {
            found.extend(stems(w).into_iter().map(|s| (s, entry, stemmed)));
        }
    }
}

/// The words of the folded description `text` the index holds: split at every character that is not a letter, a digit
/// or an apostrophe, without apostrophes at either end, and leaving out words of 2 characters or fewer, words with no
/// letter and [`STOP_WORDS`].
fn description_words(text: &str) -> impl Iterator<Item = &str> {
    text.split(|c: char| !(c.is_alphanumeric() || c == '\''))
        .map(|w| w.trim_matches('\''))
        .filter(|w| w.chars().count() > 2 && w.chars().any(char::is_alphabetic) && !STOP_WORDS.contains(w))
}

/// `text` folded, with each run of whitespace made one space.
fn normalize(folder: &Folder, text: &str) -> String {
    folder.fold(&text.split_whitespace().collect::<Vec<_>>().join(" ")).as_str().to_owned()
}

impl SearchIndex {
    /// The index of `entries`, with `synonyms` expanding each word. Labels resolve with `env.resolve` and fold with
    /// `env.folder()`; the path joins with the common words' path separator. The words of a label that has an English
    /// label are not stemmed (they may be in another language); a label with none is the English one. The path's words
    /// and the description words are stemmed.
    pub fn new(env: &EnvView<'_>, entries: &[HelpEntry], synonyms: &Synonyms) -> Self {
        let separator = &env.words::<CommonWords>().path_separator;
        Self::build(&env.folder(), separator.as_str(), &|label| env.resolve(label), entries, synonyms)
    }

    /// [`SearchIndex::new`] without a `Ui`, in the interface language of `host` (a command, which runs outside a
    /// frame): the words, folding and path separator a scope under `host` reads.
    pub fn for_host(host: &Host, entries: &[HelpEntry], synonyms: &Synonyms) -> Self {
        let words = host.words();
        let separator = &words.set::<CommonWords>().path_separator;
        Self::build(&Folder::new(host.language_code()), separator.as_str(), &|label| label.resolved(&words), entries, synonyms)
    }

    /// The index of `entries` with labels shown through `resolve`, folded with `folder`, the path joined with
    /// `separator`.
    fn build(folder: &Folder, separator: &str, resolve: &dyn Fn(&Label) -> Label, entries: &[HelpEntry], synonyms: &Synonyms) -> Self {
        let fold = |text: &str| normalize(folder, text);
        let separator = fold(separator);
        let joiner = format!(" {separator} ");
        let mut found: Vec<(String, usize, u8)> = Vec::new();
        let mut names = Vec::with_capacity(entries.len());
        for (at, entry) in entries.iter().enumerate() {
            let shown = fold(resolve(entry.label()).as_str());
            let english = entry.english().map(|label| fold(resolve(label).as_str()));
            let path = entry.path().iter().map(|label| fold(resolve(label).as_str())).collect::<Vec<_>>().join(&joiner);
            add_words(&mut found, &path, at, PATH, Some(PATH_STEMMED));
            let labels: Vec<(String, bool)> = std::iter::once((shown, english.is_none())).chain(english.map(|label| (label, true))).collect();
            let mut pairs = Vec::with_capacity(labels.len());
            let mut initials: Vec<String> = Vec::new();
            for (label, is_english) in &labels {
                add_words(&mut found, label, at, LABEL, is_english.then_some(LABEL_STEMMED));
                let full = if path.is_empty() { label.clone() } else { format!("{path}{joiner}{label}") };
                let ini = initials_of(label);
                if ini.chars().count() >= 2 && !initials.contains(&ini) {
                    initials.push(ini);
                }
                pairs.push((label.clone(), full));
            }
            let ids: Vec<String> =
                if entry.ids().is_empty() { vec![fold(&entry.command().id)] } else { entry.ids().iter().map(|id| fold(id)).collect() };
            for id in &ids {
                found.extend(words(id).map(|w| (w.to_string(), at, ID)));
            }
            if let Some(text) = entry.words() {
                for w in description_words(&fold(text)) {
                    found.push((w.to_string(), at, DESCRIPTION));
                    found.extend(stems(w).into_iter().map(|s| (s, at, DESCRIPTION_STEMMED)));
                }
            }
            let haystack = pairs.iter().map(|(_, full)| full.as_str()).chain(ids.iter().map(String::as_str)).collect::<Vec<_>>().join("\n");
            let label_len = pairs.first().map_or(0, |(label, _)| label.chars().count());
            names.push(Names { names: pairs, ids, haystack, initials, tie_order: entry.tie_order, label_len });
        }
        found.sort_unstable();
        found.dedup();
        let mut terms: Vec<(String, Vec<Posting>)> = Vec::new();
        for (word, entry, field) in found {
            let posting = Posting { entry, field };
            if let Some((_, postings)) = terms.last_mut().filter(|(last, _)| *last == word) {
                postings.push(posting);
            } else {
                terms.push((word, vec![posting]));
            }
        }
        let groups: Vec<Vec<String>> = synonyms.groups().iter().map(|group| group.iter().map(|word| fold(word)).collect()).collect();
        let mut map: HashMap<Box<str>, usize> = HashMap::new();
        for (g, group) in groups.iter().enumerate() {
            for w in group {
                map.insert(w.as_str().into(), g);
            }
        }
        for (g, group) in groups.iter().enumerate() {
            for form in group.iter().flat_map(|w| stems(w)) {
                map.entry(form.into()).or_insert(g);
            }
        }
        SearchIndex { folder: *folder, separator, entries: names, terms, synonyms: map }
    }

    /// The text of the word at place `t` in `terms`.
    fn term(&self, t: usize) -> &str {
        self.terms.get(t).map_or("", |(text, _)| text.as_str())
    }

    /// The postings of the word at place `t` in `terms`.
    fn postings_of(&self, t: usize) -> &[Posting] {
        self.terms.get(t).map_or(&[], |(_, postings)| postings.as_slice())
    }

    /// The place of the word `text` in `terms`, found by a binary search.
    fn find(&self, text: &str) -> Option<usize> {
        self.terms.binary_search_by(|(term, _)| term.as_str().cmp(text)).ok()
    }

    /// The best grade ([`LABEL_WORD`] to [`DESCRIPTION_WORD`], without [`SUBSTRING`]) `word` gets in each entry,
    /// [`NONE`] where it is no word, shares no English stem form with a word of the label, path or description words,
    /// means the same as none of them, and starts no word of the entry. The words it starts are found by a binary
    /// search of the sorted words; a stem form matches only a word or stem form equal to it.
    fn grades(&self, word: &str) -> Vec<u8> {
        let mut out = vec![NONE; self.entries.len()];
        let mut mark = |postings: &[Posting], grade: &dyn Fn(u8) -> Option<u8>| {
            for p in postings {
                if let Some(grade) = grade(p.field)
                    && let Some(g) = out.get_mut(p.entry)
                {
                    *g = (*g).min(grade);
                }
            }
        };
        let start = self.terms.partition_point(|(term, _)| term.as_str() < word);
        for (term, postings) in self.terms.iter().skip(start) {
            if !term.starts_with(word) {
                break;
            }
            let whole = term == word;
            // A stem form is not a word of the entry, so a query word that only starts it does not match.
            mark(postings, &|field| match field {
                LABEL if whole => Some(LABEL_WORD),
                LABEL => Some(LABEL_PREFIX),
                PATH => Some(PATH_WORD),
                DESCRIPTION => Some(DESCRIPTION_WORD),
                ID => Some(ID_WORD),
                _ => None,
            });
        }
        // The word meets a stem form of a word of the entry ("scale" and Scaling) when it is itself a word of some
        // entry's label, path or id, so a partly typed word ("scal") that only equals a stem form does not lift
        // Scaling above Scale. A word that only descriptions hold meets the stem forms of descriptions only.
        if let Some(t) = self.find(word) {
            let postings = self.postings_of(t);
            let named = postings.iter().any(|p| matches!(p.field, LABEL | PATH | ID));
            let described = named || postings.iter().any(|p| p.field == DESCRIPTION);
            mark(postings, &|field| match field {
                LABEL_STEMMED if named => Some(LABEL_STEM),
                PATH_STEMMED if named => Some(PATH_WORD),
                DESCRIPTION_STEMMED if described => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        // A stem form of the word meets a word of the entry or a stem form of one ("outlines" and Outline). Ids are
        // not stemmed.
        for form in stems(word) {
            let Some(t) = self.find(&form) else { continue };
            mark(self.postings_of(t), &|field| match field {
                LABEL | LABEL_STEMMED => Some(LABEL_STEM),
                PATH | PATH_STEMMED => Some(PATH_WORD),
                DESCRIPTION | DESCRIPTION_STEMMED => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        // Each other word of the word's group of words with the same meaning meets a word of the label, path or
        // description, or a stem form of one, equal to it. Ids are not matched this way.
        for t in self.synonyms_of(word) {
            mark(self.postings_of(t), &|field| match field {
                LABEL | LABEL_STEMMED => Some(LABEL_SYNONYM),
                PATH | PATH_STEMMED => Some(PATH_SYNONYM),
                DESCRIPTION | DESCRIPTION_STEMMED => Some(DESCRIPTION_WORD),
                _ => None,
            });
        }
        out
    }

    /// The other words of the group of words with the same meaning that `word`, or a stem form of it, is in, with the
    /// stem forms of the group's words, as places in `terms`; the words of the group no entry holds are left out.
    fn synonyms_of(&self, word: &str) -> Vec<usize> {
        let group = std::iter::once(word.to_string()).chain(stems(word)).find_map(|w| self.synonyms.get(w.as_str()).copied());
        let Some(group) = group else { return Vec::new() };
        let mut found: Vec<usize> = self.synonyms.iter().filter(|&(w, &g)| g == group && &**w != word).filter_map(|(w, _)| self.find(w)).collect();
        // The map's order changes from one build to the next; the words' order in `terms` does not.
        found.sort_unstable();
        found
    }

    /// The words of labels and paths that `word` is a typo of: within [`typo_bound`] edits of it ([`edits_within`]), at
    /// most [`TYPO_TERMS`], nearest first, as places in `terms`. Ids and words with letters of a script written
    /// without spaces are not compared.
    fn typos(&self, word: &str) -> Vec<usize> {
        let Some(bound) = typo_bound(word) else { return Vec::new() };
        let word: Vec<char> = word.chars().collect();
        let mut term: Vec<char> = Vec::new();
        let mut near: Vec<(usize, usize)> = Vec::new();
        for (t, (text, postings)) in self.terms.iter().enumerate() {
            // A word of no label or path is skipped before its edits are counted.
            if text.chars().count().abs_diff(word.len()) > bound
                || text.chars().any(spaceless)
                || !postings.iter().any(|p| matches!(p.field, LABEL | PATH))
            {
                continue;
            }
            term.clear();
            term.extend(text.chars());
            if let Some(edits) = edits_within(&word, &term, bound) {
                near.push((edits, t));
            }
        }
        near.sort_unstable();
        near.truncate(TYPO_TERMS);
        near.into_iter().map(|(_, t)| t).collect()
    }

    /// Gives each entry a word of `near` ([`SearchIndex::typos`]) is a word of the label or path of [`LABEL_TYPO`] or
    /// [`PATH_TYPO`] in `grades`, where its grade is not better. A typo replaces [`DESCRIPTION_WORD`], since an entry
    /// found only by its description ranks below one found through a typo.
    fn mark_typos(&self, near: &[usize], grades: &mut [u8]) {
        for &t in near {
            for p in self.postings_of(t) {
                let grade = match p.field {
                    LABEL => LABEL_TYPO,
                    PATH => PATH_TYPO,
                    _ => continue,
                };
                if let Some(g) = grades.get_mut(p.entry) {
                    *g = if *g == DESCRIPTION_WORD { grade } else { (*g).min(grade) };
                }
            }
        }
    }

    /// Is the word at place `t` in `terms` a word of entry `i` in one of `fields`? A word's postings are sorted by entry
    /// ([`SearchIndex::build`]), so a binary search finds entry `i`'s first one.
    fn holds(&self, t: usize, i: usize, fields: &[u8]) -> bool {
        let postings = self.postings_of(t);
        let from = postings.partition_point(|p| p.entry < i);
        postings.get(from..).unwrap_or_default().iter().take_while(|p| p.entry == i).any(|p| fields.contains(&p.field))
    }

    /// The first word of `near` that is a word of entry `i` in one of `fields`.
    fn nearest_of(&self, near: &[usize], i: usize, fields: &[u8]) -> Option<&str> {
        near.iter().find(|&&t| self.holds(t, i, fields)).map(|&t| self.term(t))
    }

    /// The words of `near` that are words of entry `i` in one of `fields`, in the order of `near`.
    fn words_of(&self, near: &[usize], i: usize, fields: &[u8]) -> Vec<&str> {
        near.iter().filter(|&&t| self.holds(t, i, fields)).map(|&t| self.term(t)).collect()
    }

    /// Is `query` (no spaces) the initials of one of entry `i`'s labels?
    fn initials_are(&self, i: usize, query: &str) -> bool {
        self.entries.get(i).is_some_and(|e| e.initials.iter().any(|ini| ini == query))
    }

    /// How the whole folded `query` matches entry `e` ([`EXACT_ID`] to [`NO_PHRASE`]), where `initials` says whether
    /// the query is the initials of one of its labels. A query starts a name when it starts the label, or, for a query
    /// that holds the path separator, the name with its path.
    fn phrase(&self, e: &Names, query: &str, initials: bool) -> u8 {
        if e.ids.iter().any(|id| id == query) {
            return EXACT_ID;
        }
        let trimmed = query.trim_end_matches(['…', '.']);
        if e.names.iter().flat_map(|(label, full)| [label, full]).any(|l| l == query || l.trim_end_matches(['…', '.']) == trimmed) {
            return EXACT_LABEL;
        }
        // A menu's title alone does not put the items of that menu in this tier.
        let with_path = !self.separator.is_empty() && query.contains(&self.separator);
        if e.names.iter().any(|(label, full)| label.starts_with(query) || (with_path && full.starts_with(query))) {
            return PREFIX;
        }
        if initials { INITIALS } else { NO_PHRASE }
    }
}

/// What entries are ordered by, compared field by field: whether a word matched only through a description, whether a
/// word matched only through a typo, how the whole query matches, the worst grade a word gets, the sum of the grades,
/// how the whole query matches when read with the words of the same meaning it matched through, how often the entry
/// was picked (higher grade first), the entry's place among the recent picks, its tie order and the label's length.
type RankKey = (bool, bool, u8, u8, u32, u8, Reverse<u32>, usize, u8, usize);

/// The entries of `index` matching `query`, best first, as indexes into the entries it was built from: tiers, grades
/// from `uses` (`ilog2(min(count, 999) + 1)`), `recent` places and a substring fallback for long queries. It does not
/// cap the list, and an empty query matches nothing.
///
/// An entry matches when every word of the query is text in one of its names or ids, when every word is or starts a
/// word of its label, path, description words or id, shares an English stem form with a word of its label, path or
/// description words, or means the same as one, or when the query is the initials of a label. A query word that is no
/// word, stem form or word start of any entry also matches the words of labels and paths it is a typo of. Entries where
/// a word matched only through a description come after every other entry, and before them those where a word matched
/// only through a typo. The rest are ordered by how the whole query matches, the worst grade a word gets, the sum of
/// the grades, how the whole query matches when read through the words of the same meaning, the use grade of the
/// entry (highest first), its place in `recent`, its [`HelpEntry::with_tie_order`], the label's length and the entry's
/// place. So an entry picked often, or
/// recently, ranks above the entries that match as well as it does and below every entry that matches better.
pub fn rank(index: &SearchIndex, query: &str, recent: &[usize], uses: &dyn Fn(usize) -> u32) -> Vec<usize> {
    let query = normalize(&index.folder, query);
    if query.is_empty() {
        return Vec::new();
    }
    let spaced: Vec<&str> = query.split(' ').collect();
    let head: String = query.chars().take(INDEX_CHARS).collect();
    let tokens: Vec<&str> = words(&head).collect();
    // Where each word starts in the query (`head` is its start), for reading the query with a typo corrected.
    let starts: Vec<usize> = tokens.iter().map(|t| (t.as_ptr() as usize).saturating_sub(head.as_ptr() as usize)).collect();
    let mut indexed: Vec<Vec<u8>> = Vec::with_capacity(tokens.len());
    let mut typos: Vec<Vec<usize>> = Vec::with_capacity(tokens.len());
    let mut meanings: Vec<Vec<usize>> = Vec::with_capacity(tokens.len());
    // After a word that is nothing in any entry, text in no entry's names or ids and a typo of nothing, an entry is
    // found only by its initials, where a typo grade changes nothing, so the later words are not compared.
    let mut compare = true;
    for (k, t) in tokens.iter().enumerate() {
        // A repeated word is graded once.
        if let Some(j) = tokens.iter().take(k).position(|u| u == t)
            && let (Some(grades), Some(near), Some(same)) = (indexed.get(j).cloned(), typos.get(j).cloned(), meanings.get(j).cloned())
        {
            indexed.push(grades);
            typos.push(near);
            meanings.push(same);
            continue;
        }
        meanings.push(index.synonyms_of(t));
        let mut grades = index.grades(t);
        // A word that matches only words of descriptions, or words with the same meaning, is still compared with the
        // words of labels and paths it may be a typo of.
        let near = if compare && grades.iter().all(|&g| matches!(g, NONE | DESCRIPTION_WORD | LABEL_SYNONYM | PATH_SYNONYM)) {
            index.typos(t)
        } else {
            Vec::new()
        };
        if near.is_empty() && grades.iter().all(|&g| g == NONE) && !index.entries.iter().any(|e| e.haystack.contains(t)) {
            compare = false;
        }
        index.mark_typos(&near, &mut grades);
        indexed.push(grades);
        typos.push(near);
    }
    // The index reads only the start of a longer query, so such a query matches by the substring rule, which reads all
    // of it.
    let fits = query.chars().count() <= INDEX_CHARS;
    let packed: String = query.chars().filter(|c| !c.is_whitespace()).collect();
    let packed = Some(packed).filter(|p| (2..=6).contains(&p.chars().count()) && p.chars().all(char::is_alphabetic));
    let mut ranked: Vec<(RankKey, usize)> = Vec::new();
    for (i, e) in index.entries.iter().enumerate() {
        let substring = spaced.iter().all(|w| e.haystack.contains(w));
        let initials = packed.as_deref().is_some_and(|p| index.initials_are(i, p));
        // Each word's grade, the worst and their sum; with initials, every word counts as a word the label starts, as
        // the initials stand for every word of the label.
        let (mut all_indexed, mut worst, mut sum) = (fits && !tokens.is_empty(), 0u8, 0u32);
        // Whether each word is in this entry's index or is text in its names or ids, for an entry a word matches only
        // through a typo.
        let mut held = all_indexed;
        // The query as read for the whole-query match, with each word matched only through a typo replaced by the word
        // of this entry it is a typo of; and as read with each word matched through a word with the same meaning
        // replaced by that word too.
        let (mut typo, mut corrected, mut copied) = (false, String::new(), 0);
        let mut described = false;
        // The words each such word may be read as, by its place in the query.
        let (mut synonym, mut meant): (bool, Vec<(usize, Vec<&str>)>) = (false, vec![]);
        for (k, (grades, token)) in indexed.iter().zip(&tokens).enumerate() {
            let mut g = grades.get(i).copied().unwrap_or(NONE);
            if g == NONE {
                held &= e.haystack.contains(token);
            }
            if g > SUBSTRING && e.haystack.contains(token) {
                g = SUBSTRING;
            }
            // A word that is text inside a word of the entry counts as matched, so it combines with words matched
            // through a stem form, a word of the same meaning, a typo or the description.
            if g == NONE {
                all_indexed = false;
            }
            let g = if initials {
                g.min(LABEL_PREFIX)
            } else if g == NONE {
                SUBSTRING
            } else {
                g
            };
            described |= g == DESCRIPTION_WORD;
            let field = match g {
                LABEL_TYPO => Some(LABEL),
                PATH_TYPO => Some(PATH),
                _ => None,
            };
            if let Some(field) = field {
                typo = true;
                if let Some(near) = typos.get(k).and_then(|near| index.nearest_of(near, i, &[field])) {
                    if let Some(&start) = starts.get(k)
                        && let Some(between) = query.get(copied..start)
                    {
                        corrected.push_str(between);
                        corrected.push_str(near);
                        copied = start + token.len();
                    }
                    meant.push((k, vec![near]));
                }
            }
            // The word may be read as each word of its group the label or path holds, and then as each stem form of
            // one; the reading that matches the whole query best counts.
            let fields: &[u8] = match g {
                LABEL_SYNONYM => &[LABEL, LABEL_STEMMED],
                PATH_SYNONYM => &[PATH, PATH_STEMMED],
                _ => &[],
            };
            if !fields.is_empty() {
                synonym = true;
                let mut same: Vec<&str> = vec![];
                for &f in fields {
                    for w in meanings.get(k).map(|near| index.words_of(near, i, &[f])).unwrap_or_default() {
                        if !same.contains(&w) {
                            same.push(w);
                        }
                    }
                }
                if !same.is_empty() {
                    meant.push((k, same));
                }
            }
            worst = worst.max(g);
            sum += u32::from(g);
        }
        if !(substring || all_indexed || initials || (typo && held)) {
            continue;
        }
        let tier = if typo {
            corrected.push_str(query.get(copied..).unwrap_or_default());
            index.phrase(e, &corrected, false)
        } else {
            index.phrase(e, &query, initials)
        };
        let meant_tier = if synonym { best_reading(&query, &starts, &tokens, &meant, |read| index.phrase(e, read, false)) } else { tier };
        let picked = recent.iter().position(|&r| r == i).unwrap_or(recent.len());
        let used = Reverse((uses(i).min(999) + 1).ilog2());
        ranked.push(((described, typo, tier, worst, sum, meant_tier, used, picked, e.tie_order, e.label_len), i));
    }
    ranked.sort_by_key(|&(key, place)| (key, place));
    ranked.into_iter().map(|(_, place)| place).collect()
}

/// The best whole-query match `tier` gives `query` read with each word `k` of `swaps` (the word at `starts[k]`, as long
/// as `tokens[k]`) replaced by one of its words. At most [`MAX_READINGS`] readings are compared, the first one with
/// each word's first choice.
fn best_reading(query: &str, starts: &[usize], tokens: &[&str], swaps: &[(usize, Vec<&str>)], tier: impl Fn(&str) -> u8) -> u8 {
    let readings = swaps.iter().fold(1usize, |n, (_, words)| n.saturating_mul(words.len().max(1))).min(MAX_READINGS);
    let (mut best, mut read) = (NO_PHRASE, String::new());
    for r in 0..readings {
        read.clear();
        let (mut copied, mut rest) = (0, r);
        for (k, words) in swaps {
            let len = words.len().max(1);
            let pick = words.get(rest % len);
            rest /= len;
            if let (Some(word), Some(&start), Some(token)) = (pick, starts.get(*k), tokens.get(*k))
                && let Some(between) = query.get(copied..start)
            {
                read.push_str(between);
                read.push_str(word);
                copied = start + token.len();
            }
        }
        read.push_str(query.get(copied..).unwrap_or_default());
        best = best.min(tier(&read));
        if best == EXACT_ID {
            break;
        }
    }
    best
}
