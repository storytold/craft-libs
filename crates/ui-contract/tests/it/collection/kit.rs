//! Collections for the trials, written with the derives and nothing else of the library.

use craft_behavior::{DisplayOrder, ItemSource, Selection, SpanLayout};
use craft_ui::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, ItemOutput, Outcome, Role, SelectMode,
};
use craft_ui::style::StyleCx;
use craft_ui_test_kit::contract::PlainStyle;
use egui::{Rect, Response, Ui, Vec2};
use std::borrow::Cow;
use std::cell::{Cell, RefCell};

/// Row height of every list of the trials.
pub const ROW_H: f32 = 20.0;

#[non_exhaustive]
/// Words in display order; rows with children and their open state come from the tree fields.
pub struct Words {
    pub words: Vec<&'static str>,
    /// `(depth, has children)` per row; empty for a flat list.
    pub tree: Vec<(usize, bool)>,
    pub open: RefCell<Vec<bool>>,
    /// Rows that are section stops: they hold the lead and report presses, and are never selected.
    pub sections: Vec<usize>,
}

impl Words {
    pub fn flat(words: &[&'static str]) -> Self {
        Words { words: words.to_vec(), tree: Vec::new(), open: RefCell::new(Vec::new()), sections: Vec::new() }
    }
}

impl DisplayOrder<usize> for Words {
    fn len(&self) -> usize {
        self.words.len()
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.words.len()).then_some(*key)
    }

    fn depth(&self, at: usize) -> usize {
        self.tree.get(at).map_or(0, |row| row.0)
    }

    fn has_children(&self, at: usize) -> bool {
        self.tree.get(at).is_some_and(|row| row.1)
    }

    fn open(&self, at: usize) -> bool {
        self.open.borrow().get(at).copied().unwrap_or(false)
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        Some(Cow::Borrowed(self.words[at]))
    }

    fn is_section(&self, at: usize) -> bool {
        self.sections.contains(&at)
    }

    fn section_stop(&self, at: usize) -> bool {
        self.sections.contains(&at)
    }

    fn section_stops_before(&self, at: usize) -> usize {
        self.sections.iter().filter(|&&section| section < at).count()
    }
}

impl ItemSource<usize> for Words {}

/// `n` rows with no text.
#[non_exhaustive]
pub struct Count(pub usize);

impl DisplayOrder<usize> for Count {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Count {}

/// What an item reports.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Dismissed {}

/// The output of a row: its response and the item event it raised.
#[non_exhaustive]
pub struct RowOut {
    pub response: Response,
    pub event: Option<Dismissed>,
}

impl Outcome for RowOut {
    type Value = craft_ui::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }
}

impl ItemOutput<Dismissed> for RowOut {
    fn into_event(self) -> Option<Dismissed> {
        self.event
    }
}

/// A row that records its rect.
#[derive(craft_ui::contract::Component)]
pub struct Row {
    #[common]
    common: Common,
    seen: Option<&'static Cell<Rect>>,
}

impl Row {
    pub fn new() -> Self {
        Row { common: Common::new(), seen: None }
    }
}

impl Component for Row {
    const ROLE: Role = Role::LIST_ITEM;
    type Style = PlainStyle;
    type Output = RowOut;

    fn style(_: &StyleCx<'_>, _: RowStyleKey) -> PlainStyle {
        PlainStyle::min(Vec2::new(10.0, 10.0))
    }

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        if let Some(seen) = self.seen {
            seen.set(cx.rect);
        }
    }

    fn finish(self, cx: Cx<'_, Self>) -> RowOut {
        RowOut { response: cx.response.clone(), event: None }
    }
}

/// A list of rows at the control height, with the collection props and selection the trial gives.
#[derive(craft_ui::contract::Component)]
#[component(collection)]
pub struct List<'a> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    items: CollectionProps,
    selection: Option<Bind<'a, Selection<usize>>>,
    tree: bool,
    on_empty: Option<&'a Cell<Rect>>,
}

impl<'a> List<'a> {
    pub fn new(select: SelectMode) -> Self {
        List {
            common: Common::new(),
            props: ContainerProps::new().sizing(craft_ui::contract::Sizing::Fill),
            items: CollectionProps::new().select(select),
            selection: None,
            tree: false,
            on_empty: None,
        }
    }

    pub fn reorder(mut self) -> Self {
        self.items = self.items.reorder(craft_ui::contract::ReorderRule::new());
        self
    }

    /// Reordering may land inside the groups of a tree.
    pub fn reorder_into_groups(mut self) -> Self {
        self.items = self.items.reorder(craft_ui::contract::ReorderRule::new().into_groups());
        self
    }

    pub fn selection(mut self, selection: Bind<'a, Selection<usize>>) -> Self {
        self.selection = Some(selection);
        self
    }

    pub fn tree(mut self) -> Self {
        self.tree = true;
        self
    }
}

impl Component for List<'_> {
    const ROLE: Role = Role::LIST;
    type Style = PlainStyle;
    type Output = CollectionResponse<usize, Dismissed>;

    fn style(_: &StyleCx<'_>, _: ListStyleKey) -> PlainStyle {
        PlainStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl Collection for List<'_> {
    type Key = usize;
    type ItemLayout = SpanLayout;
    type Selection = Selection<usize>;
    type Item<'r> = Row;
    type ItemEvent = Dismissed;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<usize>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn item_layout(&self, source: &dyn ItemSource<usize>, width: f32, _: &StyleCx<'_>) -> SpanLayout {
        let layout = SpanLayout::new(source, width, ROW_H);
        if self.tree { layout.tree() } else { layout }
    }

    fn empty(&mut self, _ui: &mut Ui, cx: &mut craft_ui::contract::BodyCx<'_>) {
        if let Some(seen) = self.on_empty {
            seen.set(cx.rect);
        }
    }

    fn empty_height(&self, _: f32, _: &Ui, _: &StyleCx<'_>) -> f32 {
        ROW_H
    }
}
