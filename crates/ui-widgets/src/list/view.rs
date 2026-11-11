//! `List`: a list of `ListRow`s over a display order, with the selection, reorder, rename, reveal and tree settings of
//! the collection contract. The item step does the work: virtualisation, lead and selection rules, type-ahead,
//! drag with drop marks, rename lifecycle and the context-menu request. The list adds the item layout and the editor
//! the rename lifecycle shows.

use super::predicate::{DropTest, RowTest, SourceTest};
use super::{ListRow, RowEvent};
use crate::contract::{
    Access, ActivateOn, Bind, BindMut, Collection, CollectionProps, CollectionResponse, Common, Component, ContainerProps, Cx, EditResponse,
    LeadOnOpen, ListDrag, Props as _, Role, SelectMode, Sizing,
};
use crate::runtime::env::{Env, Label};
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{ColumnKey, DisplayOrder, ItemSource, Metrics, Selection, SizeHint, SpanLayout};
use craft_i18n::matcher::Matcher;
use egui::{Id, Rect, Ui};
use std::borrow::Cow;
use std::hash::{DefaultHasher, Hash, Hasher};

/// The style of the list: it has no look of its own beyond its container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct ListStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A list of rows: one Tab stop, the selection rules of the collection contract, type-ahead on the rows' text,
/// rename in place, drag to reorder, and the request for the app's context menu.
///
/// Rows have the height their `ListRow` measures, or the height the item source's `size_hint` gives. A list of
/// groups and their children is a tree: rows are indented by their depth in the display order, groups open and
/// close with the arrow keys, and the list reports each change as `CollectionEvent::Open`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, probe(List<'static, usize>))]
pub struct List<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
    #[common]
    common: Common,
    #[container]
    props: ContainerProps,
    #[collection]
    items: CollectionProps,
    /// The selection the caller owns.
    #[prop]
    selection: Option<Bind<'a, Selection<K>>>,
    /// Scrolls the row of `key` into view once.
    #[prop]
    reveal: Option<K>,
    /// A tree: rows are indented by depth and groups open and close.
    #[prop]
    tree: bool,
    /// Which rows may be renamed; a rename request on another row does nothing.
    #[prop(func = "Fn(&K) -> bool")]
    can_rename: Option<RowTest<'a, K>>,
    /// Which rows may be dragged.
    #[prop(func = "Fn(&K) -> bool")]
    can_drag: Option<RowTest<'a, K>>,
    /// Which collections a drag may come from: the test gets the id of the list the drag started in.
    #[prop(func = "Fn(Id) -> bool")]
    accept_from: Option<SourceTest<'a>>,
    /// Where a drag may land: the test gets the drag, the row under the pointer and the position before which the
    /// dragged rows would go. A refused spot shows no drop mark.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    /// Files dragged in from the OS may drop on the list; a drop reports `CollectionEvent::FilesDropped`.
    #[prop]
    takes_files: bool,
    /// The list is a listbox of options: the popup of a value picker.
    #[prop(flag)]
    listbox: bool,
    /// The rows the query keeps, set by `filter`.
    filter: Option<Filter<'a, K>>,
    /// The kept rows are in order of how well they match the query.
    #[prop]
    ranked: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> List<'a, K> {
    /// A list named `label`, with multiple selection.
    pub fn new(label: impl Into<Label>) -> Self {
        List {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi),
            selection: None,
            reveal: None,
            tree: false,
            can_rename: None,
            can_drag: None,
            accept_from: None,
            accepts: None,
            takes_files: false,
            listbox: false,
            filter: None,
            ranked: false,
        }
        .label(label)
    }

    /// The options list of a value picker's popup (completions, number presets, `PopupList`): one selection, a click
    /// activates, the lead starts on the first row and follows the pointer, and a press leaves the key focus on the
    /// opener, whose list keys the host forwards while the popup is open. Its rows are `ListRow::option()`. Menus are
    /// `Menu`s, not option lists.
    pub fn options(label: impl Into<Label>) -> Self {
        Self::new(label)
            .select(SelectMode::Single)
            .activate(ActivateOn::Click)
            .lead_on_open(LeadOnOpen::First)
            .lead_follows_hover(true)
            .focus_on_press(false)
            .sizing(Sizing::Content)
            .listbox()
    }

    /// Only the rows whose `label` matches `query` reach the item step, folded for the scope's language (case and
    /// accents ignored); an empty query keeps every row. The rows stamp folds in the query, so the lead starts over
    /// on the first shown row and the layout rebuilds when the query changes. Sets the rows stamp.
    #[must_use]
    pub fn filter(mut self, query: &str, label: impl Fn(&K) -> Label + 'a) -> Self {
        let query = query.trim();
        let mut hasher = DefaultHasher::new();
        query.hash(&mut hasher);
        self.items = self.items.rows_stamp(hasher.finish());
        self.filter = Some(Filter { query: query.to_owned(), label: Box::new(label) });
        self
    }

    /// A row drag carries `map` of it, for the drop targets of type `P`; a row the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.items = self.items.drag_payload(map);
        self
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Component for List<'_, K> {
    const ROLE: Role = Role::LIST;
    const PARTS: bool = false;
    const ALT_ROLES: &'static [Role] = &[Role::LIST_BOX];
    type Style = ListStyle;
    type Output = CollectionResponse<K, RowEvent>;

    fn style(_: &StyleCx<'_>, _: ListStyleKey<K>) -> ListStyle {
        ListStyle::default()
    }

    fn access(&self, access: &mut Access, _: &Cx<'_, Self>) {
        if self.listbox {
            access.role(Role::LIST_BOX);
        }
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Collection for List<'_, K> {
    type Key = K;
    type ItemLayout = SpanLayout;
    type Selection = Selection<K>;
    type Item<'r> = ListRow<'r>;
    type ItemEvent = RowEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn filtered<'s>(&self, source: &'s dyn ItemSource<K>, env: &Env) -> Option<Box<dyn ItemSource<K> + 's>> {
        let filter = self.filter.as_ref()?;
        let matcher = Matcher::new(&env.folder(), &filter.query);
        if matcher.is_empty() {
            return None;
        }
        let mut kept: Vec<(craft_i18n::matcher::Rank, usize)> = (0..source.len())
            .filter_map(|at| {
                let label = (filter.label)(&source.key(at)).resolved(&env.words);
                matcher.score(label.as_str()).map(|found| (found.rank, at))
            })
            .collect();
        if self.ranked {
            kept.sort_by_key(|(rank, _)| *rank);
        }
        Some(Box::new(Filtered { source, at: kept.into_iter().map(|(_, at)| at).collect() }))
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<K>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn reveal(&self) -> Option<&K> {
        self.reveal.as_ref()
    }

    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        let layout = SpanLayout::sized(source, width, cx.control().height);
        if self.tree { layout.tree() } else { layout }
    }

    fn can_rename(&self, key: &K) -> bool {
        self.can_rename.as_ref().is_none_or(|test| test.passes(key))
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn accept_from(&self, from: Id) -> bool {
        self.accept_from.as_ref().is_none_or(|test| test.passes(from))
    }

    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }

    fn takes_files(&self) -> bool {
        self.takes_files
    }

    fn rename_editor(&mut self, ui: &mut Ui, draft: Bind<'_, String>, label: Rect, name: &str) -> Option<EditResponse<String>> {
        Some(crate::text::rename_field(ui, draft, label, name))
    }
}

/// What `List::filter` keeps: the query and how a row's key reads as text.
struct Filter<'a, K> {
    query: String,
    label: Box<dyn Fn(&K) -> Label + 'a>,
}

/// The rows of `source` at the positions `at`, in that order.
struct Filtered<'s, K> {
    source: &'s dyn ItemSource<K>,
    at: Vec<usize>,
}

impl<K> DisplayOrder<K> for Filtered<'_, K> {
    fn len(&self) -> usize {
        self.at.len()
    }

    fn key(&self, at: usize) -> K {
        self.source.key(self.at[at])
    }

    fn index_of(&self, key: &K) -> Option<usize>
    where
        K: PartialEq,
    {
        let held = self.source.index_of(key)?;
        self.at.iter().position(|&at| at == held)
    }

    fn depth(&self, at: usize) -> usize {
        self.source.depth(self.at[at])
    }

    fn has_children(&self, at: usize) -> bool {
        self.source.has_children(self.at[at])
    }

    fn open(&self, at: usize) -> bool {
        self.source.open(self.at[at])
    }

    fn is_section(&self, at: usize) -> bool {
        self.source.is_section(self.at[at])
    }

    fn enabled(&self, at: usize) -> bool {
        self.source.enabled(self.at[at])
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.source.text(self.at[at])
    }
}

impl<K> ItemSource<K> for Filtered<'_, K> {
    fn size_hint(&self, at: usize) -> SizeHint {
        self.source.size_hint(self.at[at])
    }

    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        self.source.toggled(self.at[at], column)
    }
}
