//! Compact immutable spelling index: UTF-8 offsets into one decoded text buffer.
//!
//! The contiguous-storage idea follows touchde's WordCraft #3110 dictionary.
//! This index retains Aksara's counts and last-record-wins semantics; it does not
//! substitute longest-match segmentation for the trained boundary model.

#[derive(Clone, Copy)]
struct Entry {
    start: u32,
    end: u32,
    count: u64,
}

pub(super) struct Lexicon {
    text: &'static str,
    entries: Vec<Entry>,
}

impl Lexicon {
    pub(super) fn new(text: &'static str) -> Self {
        let mut offset = 0usize;
        let mut entries = Vec::new();
        for record in text.split_inclusive('\n') {
            let line = record.trim_end_matches(['\r', '\n']);
            if let Some((word, number)) = line.rsplit_once('\t')
                && !word.is_empty()
                && let Ok(count) = number.parse()
                && let Ok(start) = u32::try_from(offset)
                && let Some(end) = offset.checked_add(word.len()).and_then(|n| u32::try_from(n).ok())
            {
                entries.push(Entry { start, end, count });
            }
            offset = offset.saturating_add(record.len());
        }
        let word = |entry: &Entry| text.get(entry.start as usize..entry.end as usize).unwrap_or_default();
        // Stable sorting retains source order for duplicate records. HashMap's
        // previous last-record-wins behavior must survive the representation change.
        entries.sort_by(|a, b| word(a).cmp(word(b)));
        entries.dedup_by(|later, earlier| {
            if word(later) == word(earlier) {
                *earlier = *later;
                true
            } else {
                false
            }
        });
        entries.shrink_to_fit();
        Self { text, entries }
    }

    fn word(&self, entry: &Entry) -> &'static str {
        self.text.get(entry.start as usize..entry.end as usize).unwrap_or_default()
    }

    pub(super) fn contains_key(&self, word: &str) -> bool {
        self.get(word).is_some()
    }

    pub(super) fn get(&self, word: &str) -> Option<&u64> {
        let at = self.entries.binary_search_by(|entry| self.word(entry).cmp(word)).ok()?;
        self.entries.get(at).map(|entry| &entry.count)
    }

    pub(super) fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub(super) fn iter(&self) -> impl Iterator<Item = (&'static str, u64)> + '_ {
        self.entries.iter().map(|entry| (self.word(entry), entry.count))
    }

    #[cfg(test)]
    pub(super) fn len(&self) -> usize {
        self.entries.len()
    }

    #[cfg(test)]
    pub(super) fn keys(&self) -> impl Iterator<Item = &'static str> + '_ {
        self.entries.iter().map(|entry| self.word(entry))
    }
}

#[cfg(test)]
mod tests {
    use super::Lexicon;

    #[test]
    fn unsorted_unicode_records_and_duplicate_counts_are_preserved() {
        let index = Lexicon::new("ខ្មែរ\t9\nabc\t1\nខ្មែរ\t17\r\nក\t0\n");
        assert_eq!(index.len(), 3);
        assert_eq!(index.get("ខ្មែរ"), Some(&17));
        assert_eq!(index.get("ក"), Some(&0));
        assert!(index.contains_key("abc"));
        assert!(!index.contains_key("ខ្មែ"));
        assert!(!index.contains_key(""));
        assert_eq!(index.keys().collect::<Vec<_>>(), vec!["abc", "ក", "ខ្មែរ"]);
    }

    #[test]
    fn malformed_records_do_not_create_false_entries() {
        let index = Lexicon::new("bad\n\t7\nnegative\t-1\noverflow\t18446744073709551616\nvalid\t2");
        assert_eq!(index.iter().collect::<Vec<_>>(), vec![("valid", 2)]);
        assert!(Lexicon::new("").is_empty());
    }
}
