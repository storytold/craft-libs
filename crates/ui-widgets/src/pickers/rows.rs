//! `EntryRows`: the entries of a picker as the rows of its option list.

use super::dress::{RowDress, SelectPicture};
use crate::contract::{Entry, Props as _, TooltipSpec};
use crate::list::{ListRow, ThumbSize};
use crate::runtime::env::Label;
use craft_behavior::{DisplayOrder, ItemSource};
use std::borrow::Cow;

/// The entries as a collection source, keyed by entry position: the entries are fixed while the list is open, and the
/// list's filter leaves out the ones that do not match.
pub(super) struct EntryRows<'e, 'l, T> {
    entries: &'e [Entry<T>],
    labels: &'l [Label],
}

impl<'e, 'l, T> EntryRows<'e, 'l, T> {
    pub(super) fn new(entries: &'e [Entry<T>], labels: &'l [Label]) -> Self {
        EntryRows { entries, labels }
    }

    /// The row of the entry at `at`: its label and icon as an option, checked when it is the `chosen` entry, a caption
    /// for a heading, a spacer for a separator, the host's picture as the thumbnail and its dress over the rest.
    pub(super) fn row<'r>(
        &self,
        at: usize,
        chosen: Option<usize>,
        dress: Option<&RowDress<'r, T>>,
        picture: Option<&SelectPicture<'r, T>>,
    ) -> ListRow<'r>
    where
        'e: 'r,
    {
        let entries: &'e [Entry<T>] = self.entries;
        let entry = &entries[at];
        let Some(value) = entry.value() else {
            return if entry.is_heading() { ListRow::new(self.labels[at].clone()).heading() } else { ListRow::spacer() };
        };
        let row = ListRow::new(self.labels[at].clone()).option().check(chosen == Some(at)).enabled(entry.is_enabled());
        let row = match entry.icon_spec() {
            Some(icon) => row.icon(icon.clone()),
            None => row,
        };
        let row = match picture {
            Some(picture) => {
                let picture = picture.clone();
                row.thumb(ThumbSize::Small, move |cx, rect| picture.paint(value, cx, rect))
            }
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

impl<T> DisplayOrder<usize> for EntryRows<'_, '_, T> {
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
        !self.entries[at].is_item()
    }

    fn enabled(&self, at: usize) -> bool {
        self.entries[at].is_enabled()
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.entries[at].is_item().then(|| Cow::Borrowed(self.labels[at].as_str()))
    }
}

impl<T> ItemSource<usize> for EntryRows<'_, '_, T> {}
