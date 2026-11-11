//! `EntryRows`: the entries of a picker as the rows of its option list.

use super::dress::RowDress;
use crate::contract::{Entry, Props as _, TooltipSpec};
use crate::list::ListRow;
use crate::runtime::env::Label;
use craft_behavior::{DisplayOrder, ItemSource};
use std::borrow::Cow;

/// The entries as a collection source, keyed by entry position: the entries are fixed while the list is open, and the
/// list's filter leaves out the ones that do not match.
pub(super) struct EntryRows<'a, T> {
    entries: &'a [Entry<T>],
    labels: &'a [Label],
}

impl<'a, T> EntryRows<'a, T> {
    pub(super) fn new(entries: &'a [Entry<T>], labels: &'a [Label]) -> Self {
        EntryRows { entries, labels }
    }

    /// The row of the entry at `at`: its label and icon as an option, a spacer for a separator, the host's dress over
    /// the rest.
    pub(super) fn row<'r>(&self, at: usize, dress: Option<&RowDress<'r, T>>) -> ListRow<'r> {
        let entry = &self.entries[at];
        let Some(value) = entry.value() else {
            return ListRow::spacer();
        };
        let row = ListRow::new(self.labels[at].clone()).option().enabled(entry.is_enabled());
        let row = match entry.icon_spec() {
            Some(icon) => row.icon(icon.clone()),
            None => row,
        };
        let row = match entry.why_disabled() {
            Some(reason) => row.description(reason.as_str()).tooltip(TooltipSpec::new(reason.clone()).disabled_text(reason.clone())),
            None => row,
        };
        match dress {
            Some(dress) => dress.apply(value, row),
            None => row,
        }
    }
}

impl<T> DisplayOrder<usize> for EntryRows<'_, T> {
    fn len(&self) -> usize {
        self.entries.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.entries.len()).then_some(*key)
    }

    fn is_section(&self, at: usize) -> bool {
        self.entries[at].is_separator()
    }

    fn enabled(&self, at: usize) -> bool {
        self.entries[at].is_enabled()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        (!self.entries[at].is_separator()).then(|| Cow::Borrowed(self.labels[at].as_str()))
    }
}

impl<T> ItemSource<usize> for EntryRows<'_, T> {}
