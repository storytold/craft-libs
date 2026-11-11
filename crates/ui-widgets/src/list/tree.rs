//! The open state of a tree: which groups show their children.

use std::collections::HashSet;
use std::hash::Hash;

/// The groups of a tree that are open, for an app whose source reports `ItemSource::open` from it. Every group is
/// closed to begin with.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenSet<K: Hash + Eq> {
    open: HashSet<K>,
}

impl<K: Hash + Eq> Default for OpenSet<K> {
    fn default() -> Self {
        OpenSet { open: HashSet::new() }
    }
}

impl<K: Hash + Eq + Clone> OpenSet<K> {
    /// A set with every group closed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Whether the group `key` is open.
    pub fn is_open(&self, key: &K) -> bool {
        self.open.contains(key)
    }

    /// Opens or closes the group `key`.
    pub fn set(&mut self, key: &K, open: bool) {
        if open {
            self.open.insert(key.clone());
        } else {
            self.open.remove(key);
        }
    }

    /// Applies what a collection reported: the group `key` takes the state, or with `all` (Alt held) every group
    /// in `groups` does.
    pub fn apply<'k>(&mut self, key: &K, open: bool, all: bool, groups: impl IntoIterator<Item = &'k K>)
    where
        K: 'k,
    {
        if all {
            for group in groups {
                self.set(group, open);
            }
        } else {
            self.set(key, open);
        }
    }
}
