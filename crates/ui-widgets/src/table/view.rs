//! `Table`: a `List` with columns. The contract's header row draws the headers and reports sort presses, widths,
//! resizes and moves; the item step lays each row's cells out along the columns; the rows are `ListRow`s that draw a
//! value in each cell. The table keeps no sort order and no hidden columns: the app owns both and passes the columns
//! it shows.

use crate::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, ColumnSpec, Common, Component, ContainerProps, Cx, EditResponse, ListDrag,
    Props as _, Role, SelectMode, Sizing,
};
use crate::list::{DropTest, ListRow, RowEvent, RowTest, SourceTest};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{ItemSource, Metrics, Selection, SpanLayout};
use egui::{Id, Rect, Ui};
use std::hash::Hash;

/// The style of the table: it has no look of its own beyond its container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TableStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A table: one Tab stop, a header row over the columns, the selection rules of the collection contract, type-ahead
/// on the rows' text, rename in place, drag to reorder, and the requests for the app's row and header menus.
///
/// The app sorts its rows and tells the table the order it used (`sort`); a press on a sortable header reports
/// `ColumnEvent::Sort` with the next order. A table of groups and their children is a tree: the label column is
/// indented by depth and groups open and close as in a `List`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, probe(Table<'static, usize>))]
pub struct Table<'a, K: Clone + Eq + Hash + Send + Sync + 'static> {
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
    /// Which collections a drag may come from: the test gets the id of the table the drag started in.
    #[prop(func = "Fn(Id) -> bool")]
    accept_from: Option<SourceTest<'a>>,
    /// Where a drag may land: the test gets the drag, the row under the pointer and the position before which the
    /// dragged rows would go. A refused spot shows no drop mark.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
    /// Files dragged in from the OS may drop on the table; a drop reports `CollectionEvent::FilesDropped`.
    #[prop]
    takes_files: bool,
}

impl<'a, K: Clone + Eq + Hash + Send + Sync + 'static> Table<'a, K> {
    /// A table named `label` over `columns`, with multiple selection.
    pub fn new(label: impl Into<Label>, columns: Vec<ColumnSpec>) -> Self {
        Table {
            common: Common::new(),
            props: ContainerProps::new().sizing(Sizing::Fill),
            items: CollectionProps::new().select(SelectMode::Multi).columns(columns),
            selection: None,
            reveal: None,
            tree: false,
            can_rename: None,
            can_drag: None,
            accept_from: None,
            accepts: None,
            takes_files: false,
        }
        .label(label)
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Component for Table<'_, K> {
    const ROLE: Role = Role::TABLE;
    const PARTS: bool = false;
    type Style = TableStyle;
    type Output = CollectionResponse<K, RowEvent>;

    fn style(_: &StyleCx<'_>, _: TableStyleKey<K>) -> TableStyle {
        TableStyle::default()
    }

    fn finish(self, mut cx: Cx<'_, Self>) -> Self::Output {
        cx.collection_response()
    }
}

impl<K: Clone + Eq + Hash + Send + Sync + 'static> Collection for Table<'_, K> {
    type Key = K;
    type ItemLayout = SpanLayout;
    type Selection = Selection<K>;
    type Item<'r> = ListRow<'r>;
    type ItemEvent = RowEvent;

    fn items(&self) -> &CollectionProps {
        &self.items
    }

    fn selection(&mut self) -> Option<BindMut<'_, Selection<K>>> {
        self.selection.as_mut().map(Bind::reborrow)
    }

    fn reveal(&self) -> Option<&K> {
        self.reveal.as_ref()
    }

    /// Column widths come from the specs and the header's memory; a row is a control high.
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
