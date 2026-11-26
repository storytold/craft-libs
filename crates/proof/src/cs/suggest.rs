//! Czech suggestions: the shared ranking ([`crate::affix_suggest`]) with the installed
//! dictionary's own MAP/REP/TRY/KEY tables and nothing Czech hard-coded.

use crate::affix::AffixDict;
use crate::affix_suggest::Tuning;

/// Spelling suggestions for a Czech word, best first, at most `max`, in the word's letter case.
pub fn suggest(word: &str, max: usize) -> Vec<String> {
    suggest_in(&super::dictionary(), word, max)
}

/// [`suggest`] against a given dictionary.
pub fn suggest_in(d: &AffixDict, word: &str, max: usize) -> Vec<String> {
    crate::affix_suggest::suggest_in(d, word, max, &Tuning::DEFAULT)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_letter_that_lowercases_to_two_chars_is_still_suggested_for() {
        // wordcraft#4683: "İx" lowercases to three chars ("i", U+0307, "x") but has two typed
        // letters; the one-letter edits stay in range and dropping the dot still finds "ix".
        let d = AffixDict::from_hunspell("", "1\nix\n", &[]).unwrap();
        let suggestions = suggest_in(&d, "İx", 4);
        assert!(suggestions.contains(&"Ix".into()), "{suggestions:?}");
    }
}
