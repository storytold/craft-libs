//! Conservative dictionary repair of a model token with a joined short suffix.
//! Never removes model boundaries, recursively splits, or changes source data.
use std::{collections::HashMap, ops::Range};

// A paragraph may contain arbitrarily many distinct unknown words. Share one
// dictionary-comparison budget across every span and its adjacent contexts.
const NEIGHBOR_COMPARISONS: usize = 160_000;

struct Neighbors<'a> {
    found: HashMap<(&'a str, &'a str), bool>,
    remaining: usize,
}

impl<'a> Neighbors<'a> {
    fn new() -> Self {
        Self { found: HashMap::new(), remaining: NEIGHBOR_COMPARISONS }
    }

    /// Conservatively report a possible correction when the paragraph budget runs out.
    /// This retains the original model span instead of introducing an unchecked split.
    fn nearby(&mut self, text: &'a str, except: &'a str, words: impl Iterator<Item = &'static str>) -> bool {
        if let Some(&found) = self.found.get(&(text, except)) {
            return found;
        }
        let mut found = false;
        for word in words {
            if self.remaining == 0 {
                return true;
            }
            self.remaining -= 1;
            if word != except && word.len().abs_diff(text.len()) <= 4 && one_edit(text, word) {
                found = true;
                break;
            }
        }
        if self.found.len() < 64 {
            self.found.insert((text, except), found);
        }
        found
    }
}

use super::{frequency, is_correct, kcc_edges, safe, spellings, standard_form};

// One Unicode insertion, deletion or substitution, without allocating edit paths.
fn one_edit(a: &str, b: &str) -> bool {
    let (mut a, mut b) = (a.chars(), b.chars());
    loop {
        let (before_a, before_b) = (a.as_str(), b.as_str());
        match (a.next(), b.next()) {
            (Some(x), Some(y)) if x == y => {}
            (Some(_), Some(_)) => return a.as_str() == b.as_str() || before_a == b.as_str() || a.as_str() == before_b,
            (Some(_), None) => return a.as_str().is_empty(),
            (None, Some(_)) => return b.as_str().is_empty(),
            (None, None) => return false,
        }
    }
}

fn nearby_spelling<'a>(text: &'a str, except: &'a str, cache: &mut Neighbors<'a>) -> bool {
    cache.nearby(text, except, spellings().iter().map(|(word, _)| word))
}

fn known(word: &str) -> bool {
    frequency(word) > 0 || spellings().contains_key(word)
}

fn split<'a>(text: &'a str, spans: &[Range<usize>], index: usize, cache: &mut Neighbors<'a>) -> Option<usize> {
    let span = spans.get(index)?;
    let word = text.get(span.clone())?;
    // Bound work on malformed input and retain even rare frequency-only entries:
    // they can be misspellings which the model deliberately keeps correctable.
    if word.len() > 96 || frequency(word) > 0 {
        return None;
    }
    let edges = kcc_edges(word);
    if edges.len() < 5 {
        return None;
    }
    let at = *edges.get(edges.len().checked_sub(2)?)?;
    let (left, right) = (word.get(..at)?, word.get(at..)?);
    // At least three clusters followed by one cluster, with both components
    // independently attested in the frequency and spelling dictionaries.
    if frequency(left) <= 1
        || frequency(right) <= 1
        || !spellings().contains_key(left)
        || !spellings().contains_key(right)
        || is_correct(word)
        || standard_form(word).is_some()
        || !safe(left.chars().next_back()?, right.chars().next()?)
    {
        return None;
    }
    // Abstain on competing two-word analyses, regardless of their lengths.
    for &other in edges.iter().skip(1).take(edges.len().saturating_sub(2)) {
        if other != at && known(word.get(..other)?) && known(word.get(other..)?) {
            return None;
        }
    }
    // Deleting the proposed suffix yields the prefix by definition. Other
    // nearby spellings remain correction candidates, rather than being hidden
    // by a dictionary split. Also check adjacent original tokens so an inserted
    // letter in a longer dictionary word cannot evade this guard.
    if nearby_spelling(word, left, cache) {
        return None;
    }
    let previous = index.checked_sub(1).and_then(|i| spans.get(i)).filter(|s| s.end == span.start).map(|s| s.start..span.end);
    let next = spans.get(index + 1).filter(|s| s.start == span.end).map(|s| span.start..s.end);
    for range in [previous, next].into_iter().flatten() {
        if let Some(context) = text.get(range)
            && context.len() <= 96
            && (known(context) || is_correct(context) || standard_form(context).is_some() || nearby_spelling(context, "", cache))
        {
            return None;
        }
    }
    Some(span.start + at)
}

pub(super) fn words(text: &str, spans: Vec<Range<usize>>) -> Vec<Range<usize>> {
    if !cfg!(craft_khmer_data) {
        return spans;
    }
    // Allocate a replacement only when a repair is found. Decisions always
    // inspect the original model spans, never the results of an earlier repair.
    let mut refined: Option<Vec<Range<usize>>> = None;
    let mut cache = Neighbors::new();
    for (index, span) in spans.iter().enumerate() {
        if let Some(at) = split(text, &spans, index, &mut cache) {
            let out = refined.get_or_insert_with(|| spans.get(..index).unwrap_or_default().to_vec());
            out.extend([span.start..at, at..span.end]);
        } else if let Some(out) = &mut refined {
            out.push(span.clone());
        }
    }
    refined.unwrap_or(spans)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn distinct_queries_and_adjacent_contexts_share_one_work_budget() {
        let mut cache = Neighbors { found: HashMap::new(), remaining: 3 };
        assert!(!cache.nearby("abc", "", ["xyz", "uvw"].into_iter()));
        assert_eq!(cache.remaining, 1);
        // A cached negative answer is safe and consumes no more dictionary work.
        assert!(!cache.nearby("abc", "", ["xyz", "uvw"].into_iter()));
        assert_eq!(cache.remaining, 1);
        assert!(cache.nearby("def", "", ["xyz", "uvw"].into_iter()), "abstain on an incomplete scan");
        assert_eq!(cache.remaining, 0);
        for query in ["ghi", "jkl", "mno", "pqr"] {
            assert!(cache.nearby(query, "", ["xyz"].into_iter()));
        }
        assert_eq!(cache.remaining, 0);
        assert_eq!(cache.found.len(), 1, "incomplete scans are never cached as negatives");
    }

    #[test]
    fn exclusions_and_unicode_edits_keep_their_correction_guard() {
        let mut cache = Neighbors::new();
        assert!(!cache.nearby("ខ្មែរX", "ខ្មែរ", ["ខ្មែរ"].into_iter()));
        assert!(cache.nearby("ខ្មែរY", "", ["ខ្មែរ"].into_iter()));
        assert!(cache.nearby("ខ្មែ", "", ["ខ្មែរ"].into_iter()));
        assert!(cache.nearby("ខ្មែរ", "", ["ខ្មែX"].into_iter()));
    }
}
