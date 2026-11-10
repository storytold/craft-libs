//! Small counting helpers shared by the report sections.

use std::collections::BTreeMap;

/// Counts how often each label occurs, plus a byte total per label.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Tally {
    counts: BTreeMap<String, (usize, u64)>,
}

impl Tally {
    pub fn add(&mut self, label: impl Into<String>, bytes: u64) {
        let slot = self.counts.entry(label.into()).or_default();
        slot.0 += 1;
        slot.1 += bytes;
    }

    /// (label, count, bytes), most frequent first.
    pub fn rows(&self) -> Vec<(&str, usize, u64)> {
        let mut v: Vec<_> = self.counts.iter().map(|(k, (n, b))| (k.as_str(), *n, *b)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v
    }

    pub fn total(&self) -> usize {
        self.counts.values().map(|(n, _)| n).sum()
    }
}

/// Result of decompressing pixel blocks to check they are readable.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Verification {
    pub passed: usize,
    /// (item, reason) for each failure.
    pub failed: Vec<(String, String)>,
}

impl Verification {
    pub fn record(&mut self, item: &str, outcome: Result<(), String>) {
        match outcome {
            Ok(()) => self.passed += 1,
            Err(reason) => self.failed.push((item.to_string(), reason)),
        }
    }

    pub fn checked(&self) -> usize {
        self.passed + self.failed.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tally_sorts_by_count() {
        let mut t = Tally::default();
        t.add("b", 1);
        t.add("a", 2);
        t.add("b", 3);
        assert_eq!(t.rows(), vec![("b", 2, 4), ("a", 1, 2)]);
        assert_eq!(t.total(), 3);
    }
}
