//! `UndoList`: the actions that can be undone, newest first, as a listbox for a popup the app opens from its own
//! drop-down arrow. Marking a row marks it and every newer one; picking a row undoes through it.

use super::view_words::UndoWords;
use super::{ListRow, RowEvent};
use crate::contract::{
    ActivateOn, Bind, BindMut, Collection, CollectionEvent, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, Props as _,
    Role, SelectMode, Sizing,
};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{DisplayOrder, ItemSource, Metrics, RowAt, SelectionChange, SelectionModel, SpanLayout};
use craft_i18n::Format;
use std::borrow::Cow;

/// The marked actions of an undo list: the row at `marked` and every newer one (a smaller position).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct UndoMark {
    marked: Option<usize>,
}

impl UndoMark {
    /// A mark on the row at `position`, 0 being the newest action.
    pub const fn at(position: usize) -> Self {
        UndoMark { marked: Some(position) }
    }

    /// How many actions the mark covers: the marked row and every newer one.
    pub const fn actions(&self) -> usize {
        match self.marked {
            Some(position) => position + 1,
            None => 0,
        }
    }
}

impl SelectionModel<usize> for UndoMark {
    fn contains(&self, row: RowAt<'_, usize>) -> bool {
        self.marked.is_some_and(|marked| *row.key <= marked)
    }

    fn lead(&self) -> Option<&usize> {
        self.marked.as_ref()
    }

    fn anchor(&self) -> Option<&usize> {
        self.marked.as_ref()
    }

    fn apply(&mut self, change: SelectionChange<usize>) {
        self.marked = match change {
            SelectionChange::Set(keys) => keys.last().copied(),
            SelectionChange::Toggle(key) => Some(key),
            SelectionChange::Clear => None,
            SelectionChange::Cells(range) => Some(range.last),
            _ => self.marked,
        };
    }
}

/// The footer text of an undo list: "Undo 3 Actions" while rows are marked, `None` while none is (the footer is the
/// Cancel row then).
pub fn footer_text(words: &UndoWords, format: &Format, mark: &UndoMark) -> Option<String> {
    let actions = mark.actions() as u64;
    (actions > 0).then(|| words.actions.text(actions, &format.integer(actions)))
}

/// The actions of the undo list as an item source: the label of each, newest first, keyed by position.
#[derive(Clone, Copy, Debug)]
pub struct UndoRows<'a>(&'a [String]);

impl<'a> UndoRows<'a> {
    /// The actions labelled `labels`, the newest first.
    pub const fn new(labels: &'a [String]) -> Self {
        UndoRows(labels)
    }
}

impl DisplayOrder<usize> for UndoRows<'_> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0.len()).then_some(*key)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.0.get(at).map(|label| Cow::Borrowed(label.as_str()))
    }
}

impl ItemSource<usize> for UndoRows<'_> {}

impl UndoRows<'_> {
    /// The row of the action at `at`: an option of the listbox, marked by the list's selection.
    pub fn row(&self, at: usize) -> ListRow<'static> {
        ListRow::new(self.0[at].clone()).option()
    }
}

/// How many actions an undo list asks the app to undo: through the row picked by a click or Enter, `None` while
/// no row was picked. The app undoes that many and closes the popup.
pub fn undo_count(events: &[CollectionEvent<usize, RowEvent>]) -> Option<usize> {
    events.iter().find_map(|event| match event {
        CollectionEvent::Activate(position) => Some(position + 1),
        _ => None,
    })
}

/// The style of the undo list: it has no look of its own.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct UndoListStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// The undo list: one Tab stop, arrows mark rows through the row, Enter or a click picks it.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection)]
pub struct UndoList<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    /// The mark the caller owns.
    #[prop]
    selection: Option<Bind<'a, UndoMark>>,
}

impl<'a> UndoList<'a> {
    /// An undo list named `label`.
    pub fn new(label: impl Into<Label>) -> Self {
        UndoList {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Single).activate(ActivateOn::Click).lead_follows_hover(true),
            selection: None,
        }
        .label(label)
    }
}

impl Component for UndoList<'_> {
    const ROLE: Role = Role::LIST_BOX;
    const PARTS: bool = false;
    type Style = UndoListStyle;
    type Output = CollectionResponse<usize, RowEvent>;

    fn style(_: &StyleCx<'_>, _: UndoListStyleKey) -> UndoListStyle {
        UndoListStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for UndoList<'_> {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = UndoMark;
    type Item<'r> = ListRow<'r>;
    type ItemEvent = RowEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, UndoMark>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        SpanLayout::new(source, width, cx.control().height)
    }
}
