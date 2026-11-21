//! `SectionList`: the "sections beside the shown section" arrangement of the PDF export dialog and the preferences
//! body.

use crate::list::ListRow;
use crate::runtime::env::Label;
use craft_behavior::ItemSource;
use egui::Ui;
use std::marker::PhantomData;

/// A single-select, click-activated, content-sized list without a header and a vertical divider at the start edge,
/// and the body of the shown section in a filling, vertically scrolled stack beside them; the list stays put while the
/// body scrolls.
pub struct SectionList<'a, K> {
    _marker: PhantomData<(&'a (), K)>,
}

impl<'a, K> SectionList<'a, K> {
    /// The list labelled `label` over the sections of `source`, with `shown` picked.
    pub fn new(label: impl Into<Label>, source: &'a dyn ItemSource<K>, shown: K) -> Self {
        let _ = (label, source, shown);
        todo!("ty.X70")
    }

    /// Content under the list, in the list's column.
    #[must_use]
    pub fn foot(self, foot: impl FnOnce(&mut Ui) + 'a) -> Self {
        let _ = (self, foot);
        todo!("ex.E57")
    }

    /// Shows the list with a row per section from `row` and `body` beside it; the section picked this pass.
    pub fn show(self, ui: &mut Ui, row: impl FnMut(&K) -> ListRow, body: impl FnOnce(&mut Ui)) -> Option<K> {
        let _ = (self, ui, row, body);
        todo!("ty.X70")
    }
}
