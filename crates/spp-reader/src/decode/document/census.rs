//! Counts of `Data*` object classes in one serialised file, to measure which classes a reader still has to support.

use super::header::DocumentHeader;
use super::walker::{Visitor, walk};
use crate::decode::DecodeError;
use std::collections::BTreeMap;

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ClassCensus {
    counts: BTreeMap<String, usize>,
    unknown_tags: BTreeMap<u32, usize>,
}

impl ClassCensus {
    /// Checks the document signature, then counts class names by walking the object tree.
    pub fn scan(data: &[u8]) -> Result<Self, DecodeError> {
        DocumentHeader::parse(data)?;
        let mut census = ClassCensus::default();
        walk(data, &mut census)?;
        Ok(census)
    }

    pub fn count(&self, class: &str) -> usize {
        self.counts.get(class).copied().unwrap_or(0)
    }

    /// Total number of objects found.
    pub fn total(&self) -> usize {
        self.counts.values().sum()
    }

    /// Classes, most frequent first (ties in name order).
    pub fn by_frequency(&self) -> Vec<(&str, usize)> {
        let mut v: Vec<(&str, usize)> = self.counts.iter().map(|(k, n)| (k.as_str(), *n)).collect();
        v.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(b.0)));
        v
    }

    /// Value tags the walker could not size, with how often each cut a container short.
    /// Empty for every file seen so far; objects inside a skipped part are not counted.
    pub fn unknown_tags(&self) -> &BTreeMap<u32, usize> {
        &self.unknown_tags
    }

    /// True when every object was reached.
    pub fn is_complete(&self) -> bool {
        self.unknown_tags.is_empty()
    }
}

impl Visitor for ClassCensus {
    fn object(&mut self, class: &str) {
        *self.counts.entry(class.to_owned()).or_default() += 1;
    }

    fn unknown_tag(&mut self, tag: u32, _offset: usize) {
        *self.unknown_tags.entry(tag).or_default() += 1;
    }
}
