//! [`ModelReport`]: how much of a document the model understood, per class: objects turned into typed
//! values (recognised), objects kept only as [`super::RawEntry`] (raw), and the rest (details the model
//! does not mirror, such as stroke points or projection grids).

use std::collections::BTreeMap;

/// Counts for one class.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ClassTally {
    /// Objects of this class in the file.
    pub total: usize,
    pub recognised: usize,
    pub raw: usize,
}

impl ClassTally {
    /// Objects neither recognised nor kept raw (details below what the model mirrors).
    pub fn not_modelled(&self) -> usize {
        self.total.saturating_sub(self.recognised + self.raw)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ModelReport {
    pub classes: BTreeMap<String, ClassTally>,
    /// Value tags the walker could not size (their container's rest was skipped).
    pub unknown_tags: usize,
    /// Places where nesting exceeded the model's limit and the rest was kept raw.
    pub depth_limited: usize,
    /// Texture sets with more than one material stack (only the first is modelled; never seen so far).
    pub extra_stacks: usize,
    /// Fields the reader's field table doesn't list (`Class.field` → objects), see [`super::fields`]. Empty for every
    /// project seen so far; a newer version's fields show up here.
    pub unknown_fields: BTreeMap<String, usize>,
}

impl ModelReport {
    /// Starts from the class counts of the walk.
    pub fn with_totals(totals: &BTreeMap<String, usize>) -> ModelReport {
        let classes = totals.iter().map(|(c, n)| (c.clone(), ClassTally { total: *n, ..ClassTally::default() })).collect();
        ModelReport { classes, ..ModelReport::default() }
    }

    pub fn recognise(&mut self, class: &str) {
        self.classes.entry(class.to_string()).or_default().recognised += 1;
    }

    /// `n` objects of `class` turned into typed values at once (the rows of a packed list).
    pub fn recognise_many(&mut self, class: &str, n: usize) {
        self.classes.entry(class.to_string()).or_default().recognised += n;
    }

    pub fn keep_raw(&mut self, class: &str) {
        self.classes.entry(class.to_string()).or_default().raw += 1;
    }

    pub fn total(&self) -> usize {
        self.classes.values().map(|t| t.total).sum()
    }

    pub fn recognised(&self) -> usize {
        self.classes.values().map(|t| t.recognised).sum()
    }

    pub fn raw(&self) -> usize {
        self.classes.values().map(|t| t.raw).sum()
    }

    pub fn not_modelled(&self) -> usize {
        self.classes.values().map(|t| t.not_modelled()).sum()
    }

    /// Classes with raw entries, most frequent first.
    pub fn raw_classes(&self) -> Vec<(&str, usize)> {
        let mut v: Vec<_> = self.classes.iter().filter(|(_, t)| t.raw > 0).map(|(c, t)| (c.as_str(), t.raw)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v
    }

    /// Classes with objects the model does not mirror, most frequent first.
    pub fn not_modelled_classes(&self) -> Vec<(&str, usize)> {
        let mut v: Vec<_> = self.classes.iter().filter(|(_, t)| t.not_modelled() > 0).map(|(c, t)| (c.as_str(), t.not_modelled())).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v
    }
}
