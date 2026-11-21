//! The command search's word index and its ranking.

use super::{HelpEntry, Synonyms};
use craft_i18n::Folder;

/// The word index of the help search: the words of each entry's label, English label, path, id and words, English
/// stems, a typo bound and synonym expansion.
pub struct SearchIndex {}

impl SearchIndex {
    /// The index of `entries`, folded by `folder`, with `synonyms` expanding each word.
    pub fn new(folder: &Folder, entries: &[HelpEntry], synonyms: &Synonyms) -> Self {
        let _ = (folder, entries, synonyms);
        todo!("mn.M19")
    }
}

/// The entries of `index` matching `query`, best first, as indexes into the entries it was built from: tiers, grades
/// from `uses` (`ilog2(min(count, 999) + 1)`), `recent` places and a substring fallback for long queries. It does not
/// cap the list.
pub fn rank(index: &SearchIndex, query: &str, recent: &[usize], uses: &dyn Fn(usize) -> u32) -> Vec<usize> {
    let _ = (index, query, recent, uses);
    todo!("mn.M20")
}
