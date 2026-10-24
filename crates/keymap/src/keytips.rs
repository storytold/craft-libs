//! Deterministic single-letter keytips for contextual menus and tabs.
//!
//! This shares allocation policy, not an application's Alt-key state machine or command catalog.
//! Allocate over the full ordered set of possible contextual tabs, then display its active subset;
//! allocating only visible tabs can change their letters when another tab appears.

const MAX_CHOICES: usize = 4096;
const MAX_LABEL_CHARS: usize = 256;

/// One optional ASCII A-Z letter per considered label, in input order. `None` means all letters
/// are reserved or already assigned; the host should keep the control available without a badge.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Allocation {
    pub letters: Vec<Option<char>>,
    /// Labels after the first 4096 are not considered. Report this if exposing imported catalogs.
    pub omitted: usize,
}

/// Prefer the first unused ASCII letter in each label, then the first free A-Z letter.
/// Reserved sequences protect their first letter, case-insensitively, including prefixes of
/// multi-key tips. Empty/non-letter prefixes reserve no A-Z key. Assignments never duplicate
/// another result or shadow a reserved prefix. Exhaustion is explicit, not a fabricated key.
///
/// The caller supplies its reserved fixed tabs, search and legacy-menu prefixes. Labels may be
/// localized or stable source labels according to host policy; non-Latin names fall back to A-Z
/// rather than attempting transliteration. At most 4096 choices and 256 characters per label are
/// scanned. All reserved prefixes are inspected so truncation cannot introduce a collision.
pub fn allocate(labels: &[&str], reserved: &[&str]) -> Allocation {
    let mut taken = [false; 26];
    let index = |c: char| {
        if c.is_ascii_alphabetic() { Some((c.to_ascii_uppercase() as u8 - b'A') as usize) } else { None }
    };
    for text in reserved {
        if let Some(i) = text.chars().next().and_then(index) {
            taken[i] = true;
        }
    }
    let mut letters = Vec::with_capacity(labels.len().min(MAX_CHOICES));
    for label in labels.iter().take(MAX_CHOICES) {
        let preferred = label.chars().take(MAX_LABEL_CHARS).filter_map(index).find(|&i| !taken[i]);
        let free = preferred.or_else(|| taken.iter().position(|used| !used));
        letters.push(free.map(|i| {
            taken[i] = true;
            char::from(b'A' + i as u8)
        }));
    }
    Allocation { letters, omitted: labels.len().saturating_sub(MAX_CHOICES) }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn contextual_letters_avoid_fixed_and_legacy_prefixes_even_when_labels_repeat() {
        let allocation = allocate(&["Picture Format", "Shape Format", "Shape Format", "图表"], &["P", "S", "jT", "Q", "fO"]);
        let letters = allocation.letters.iter().flatten().copied().collect::<Vec<_>>();
        assert_eq!(letters.len(), 4);
        let unique = letters.iter().copied().collect::<std::collections::HashSet<_>>();
        assert_eq!(unique.len(), 4);
        assert!(letters.iter().all(|c| !"PSJQF".contains(*c)));
        assert_eq!(allocate(&["Picture Format", "Shape Format", "Shape Format", "图表"], &["P", "S", "jT", "Q", "fO"]), allocation);
    }
    #[test]
    fn exhausted_alphabet_is_explicit_and_large_catalogs_are_reported() {
        let labels = vec!["A"; 5000];
        let allocation = allocate(&labels, &[]);
        assert_eq!(allocation.letters.len(), MAX_CHOICES);
        assert_eq!(allocation.omitted, 5000 - MAX_CHOICES);
        assert_eq!(allocation.letters.iter().filter(|c| c.is_some()).count(), 26);
        let reserved = allocate(
            &["Anything"],
            &["A", "B", "C", "D", "E", "F", "G", "H", "I", "J", "K", "L", "M", "N", "O", "P", "Q", "R", "S", "T", "U", "V", "W", "X", "Y", "Z"],
        );
        assert_eq!(reserved.letters, vec![None]);
    }
}
