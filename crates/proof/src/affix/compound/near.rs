//! Near-miss compound suggestions: a word of three or more parts that one slip turns into a word.

use super::*;

/// Most `MAP` variants, `TRY` letters and `KEY` row letters the three-part check tries.
const MAX_MAP_VARIANTS: usize = 256;

const MAX_TRY: usize = 128;

const MAX_KEY_ROW: usize = 128;

/// The keyboard rows the CLI assumes when a dictionary has no `KEY`.
const DEFAULT_KEY: &str = "qwertyuiop|asdfghjkl|zxcvbnm";

impl AffixDict {
    /// The check on a compound of three or more parts: is the word one slip away from a word
    /// (not a compound)? Then it is taken for a misspelling of that word. The slips are the
    /// CLI's (black-box, see the tests): a letter too many, a pair typed twice, two letters
    /// swapped or one moved (up to 4 apart), a keyboard neighbour (`KEY`), a `TRY` letter, a
    /// `REP` or `MAP` change, the word capitalized, and (without `NOSPLITSUGS`) two words. It
    /// depends on the word only, so it is answered once per check.
    pub(super) fn near_word(&self, s: &mut Search) -> bool {
        if let Some(near) = s.near {
            return near;
        }
        let excluded = s.excluded;
        let mut work = s.work;
        // A word listed as a stem that can't stand alone (ONLYINCOMPOUND, NEEDAFFIX) counts here
        // only with a suffix, not with a prefix (`ver` + `kehr` beside `verkehr/zo`:
        // `Fahrradverkehr` is right in the CLI).
        let mut is_word = |cand: &[char]| {
            work += 1;
            let cand: String = cand.iter().collect();
            // Past its budget a candidate counts as a word: the compound is then rejected, as
            // it is past MAX_WORK, so a suggestion never offers one the check would refuse.
            work > MAX_WORK || self.simple_word_budgeted(&cand, excluded, self.stem_set(&cand).is_none()).unwrap_or(true)
        };
        let near = self.near_miss(&s.chars, s.c.two_words, &mut is_word);
        s.work = work;
        s.near = Some(near);
        near
    }

    fn near_miss(&self, w: &[char], two_words: bool, is_word: &mut impl FnMut(&[char]) -> bool) -> bool {
        let n = w.len();
        let mut cand: Vec<char> = Vec::with_capacity(n + 8);
        let mut first = w.iter().take(1).flat_map(|c| c.to_uppercase()).collect::<Vec<char>>();
        first.extend(w.iter().skip(1));
        if is_word(&first) || is_word(&w.iter().flat_map(|c| c.to_uppercase()).collect::<Vec<char>>()) {
            return true;
        }
        for i in 0..n {
            // A letter too many, and a pair typed twice (`abab` for `ab`).
            cand.clear();
            cand.extend(w.iter().take(i).chain(w.iter().skip(i + 1)));
            if is_word(&cand) {
                return true;
            }
            if i + 3 < n && w.get(i) == w.get(i + 2) && w.get(i + 1) == w.get(i + 3) {
                cand.clear();
                cand.extend(w.iter().take(i).chain(w.iter().skip(i + 2)));
                if is_word(&cand) {
                    return true;
                }
            }
            for d in 1..=4 {
                if i + d >= n {
                    break;
                }
                // Swapped with a letter up to 4 further on.
                cand.clear();
                cand.extend_from_slice(w);
                cand.swap(i, i + d);
                if cand != w && is_word(&cand) {
                    return true;
                }
                // Moved 2 to 4 places on or back (one place is a swap).
                if d >= 2 {
                    cand.clear();
                    cand.extend_from_slice(w);
                    for turn in [1, 2] {
                        if let Some(slice) = cand.get_mut(i..=i + d) {
                            // Left by one: `w[i]` moves on; then right by two: `w[i + d]` back.
                            if turn == 1 { slice.rotate_left(1) } else { slice.rotate_right(2) }
                        }
                        if is_word(&cand) {
                            return true;
                        }
                    }
                }
            }
        }
        // One letter for another: a keyboard neighbour, a TRY letter.
        let key = if self.key.is_empty() { DEFAULT_KEY } else { self.key.as_str() };
        let neighbours = |c: char| {
            key.split('|').flat_map(move |row| {
                let row: Vec<char> = row.chars().take(MAX_KEY_ROW).collect();
                let at: Vec<usize> = row.iter().enumerate().filter(|(_, k)| **k == c).map(|(i, _)| i).collect();
                at.into_iter()
                    .flat_map(move |i| [i.checked_sub(1).and_then(|j| row.get(j)).copied(), row.get(i + 1).copied()])
                    .flatten()
                    .collect::<Vec<char>>()
            })
        };
        let tries: Vec<char> = self.try_chars.chars().take(MAX_TRY).collect();
        for i in 0..n {
            let Some(&c) = w.get(i) else { continue };
            for r in neighbours(c).chain(tries.iter().copied()) {
                if r == c {
                    continue;
                }
                cand.clear();
                cand.extend_from_slice(w);
                if let Some(x) = cand.get_mut(i) {
                    *x = r;
                }
                if is_word(&cand) {
                    return true;
                }
            }
        }
        // `REP` (here `^` and `$` tie to the word's start and end) and `MAP` changes.
        let text: String = w.iter().collect();
        for (from, to) in &self.rep {
            let (start, from) = from.strip_prefix('^').map_or((false, from.as_str()), |f| (true, f));
            let (end, from) = from.strip_suffix('$').map_or((false, from), |f| (true, f));
            if from.is_empty() || to.contains(' ') {
                continue;
            }
            for (i, _) in text.match_indices(from) {
                if (start && i != 0) || (end && i + from.len() != text.len()) {
                    continue;
                }
                let changed: Vec<char> = text
                    .get(..i)
                    .unwrap_or_default()
                    .chars()
                    .chain(to.chars())
                    .chain(text.get(i + from.len()..).unwrap_or_default().chars())
                    .collect();
                if is_word(&changed) {
                    return true;
                }
            }
        }
        let mut budget = MAX_MAP_VARIANTS;
        if !self.map.is_empty() && self.map_variant(w, 0, &mut w.to_vec(), &mut budget, is_word) {
            return true;
        }
        two_words
            && (1..n).any(|i| {
                let (head, tail) = w.split_at(i);
                is_word(head) && is_word(tail)
            })
    }

    /// Does changing letters of `cand` from `i` on to others of their `MAP` group make a word?
    fn map_variant(&self, w: &[char], i: usize, cand: &mut Vec<char>, budget: &mut usize, is_word: &mut impl FnMut(&[char]) -> bool) -> bool {
        let Some(&c) = w.get(i) else { return cand.as_slice() != w && is_word(cand) };
        if self.map_variant(w, i + 1, cand, budget, is_word) {
            return true;
        }
        for group in self.map.iter().filter(|g| g.contains(&c)) {
            for &r in group.iter().filter(|&&r| r != c) {
                if *budget == 0 {
                    return false;
                }
                *budget -= 1;
                if let Some(x) = cand.get_mut(i) {
                    *x = r;
                }
                let hit = self.map_variant(w, i + 1, cand, budget, is_word);
                if let Some(x) = cand.get_mut(i) {
                    *x = c;
                }
                if hit {
                    return true;
                }
            }
        }
        false
    }

    /// Does a `REP` replacement turn `text` into a word (not a compound)?
    pub(super) fn rep_fault(&self, text: &str, s: &mut Search) -> bool {
        let mut cand = String::with_capacity(text.len() + 8);
        for (from, to) in &self.rep {
            // Replacements tied to a word's start or end (`^`, `$`) are for suggestions: the CLI
            // never applies them here.
            if from.is_empty() || from.starts_with('^') || from.ends_with('$') {
                continue;
            }
            for (i, _) in text.match_indices(from.as_str()) {
                s.work += 1;
                if s.work > MAX_WORK {
                    return true;
                }
                cand.clear();
                cand.push_str(text.get(..i).unwrap_or_default());
                cand.push_str(to);
                cand.push_str(text.get(i + from.len()..).unwrap_or_default());
                if self.simple_word(&cand, s.excluded) {
                    return true;
                }
            }
        }
        false
    }
}
