//! The list of groups and datasets in a project file, with simple queries over it.

use crate::container::hdf5::{Entry, EntryKind};

/// One dataset: its absolute path and length in bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DatasetRef<'a> {
    pub path: &'a str,
    pub len: u64,
}

impl<'a> DatasetRef<'a> {
    /// Last path segment, e.g. `document.bin`.
    pub fn name(&self) -> &'a str {
        self.path.rsplit('/').next().unwrap_or(self.path)
    }
}

#[derive(Debug, Clone, Default)]
pub struct Layout {
    entries: Vec<Entry>,
}

impl Layout {
    pub fn new(entries: Vec<Entry>) -> Self {
        Layout { entries }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    pub fn datasets(&self) -> impl Iterator<Item = DatasetRef<'_>> {
        self.entries.iter().filter_map(|e| match e.kind {
            EntryKind::Dataset { len } => Some(DatasetRef { path: &e.path, len }),
            EntryKind::Group => None,
        })
    }

    pub fn dataset(&self, path: &str) -> Option<DatasetRef<'_>> {
        self.datasets().find(|d| d.path == path)
    }

    /// Datasets whose path is inside `group` (at any depth).
    pub fn datasets_under<'a>(&'a self, group: &'a str) -> impl Iterator<Item = DatasetRef<'a>> + 'a {
        self.datasets().filter(move |d| is_inside(d.path, group))
    }

    pub fn total_dataset_bytes(&self) -> u64 {
        self.datasets().map(|d| d.len).sum()
    }
}

fn is_inside(path: &str, group: &str) -> bool {
    path.strip_prefix(group).is_some_and(|rest| rest.starts_with('/'))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layout() -> Layout {
        let ds = |p: &str, len| Entry { path: p.into(), kind: EntryKind::Dataset { len } };
        Layout::new(vec![
            Entry { path: "/paint".into(), kind: EntryKind::Group },
            ds("/paint/document.bin", 10),
            ds("/paintings", 1),
            ds("/resources/a.image", 5),
        ])
    }

    #[test]
    fn finds_datasets_inside_a_group_only() {
        let l = layout();
        let inside: Vec<_> = l.datasets_under("/paint").map(|d| d.name()).collect();
        assert_eq!(inside, ["document.bin"]);
        assert_eq!(l.total_dataset_bytes(), 16);
        assert_eq!(l.dataset("/resources/a.image").map(|d| d.len), Some(5));
    }
}
