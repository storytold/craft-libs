//! `Table`: a `List` with columns. The contract's header row draws the headers and reports sort presses, widths,
//! resizes, moves and hidden columns; the item step lays each row's cells out along the columns; the rows are
//! `ListRow`s that draw a value in each cell. The table shows the header menu. The app owns the sort order.

use super::words::TableWords;
use crate::contract::overlay::Anchor;
use crate::contract::{
    Bind, BindMut, Collection, CollectionProps, CollectionResponse, ColumnCommands, ColumnRequest, ColumnSpec, Common, Component, ContainerProps, Cx,
    DropKind, EditResponse, ListDrag, OverlayUi, Overlays as _, Props as _, Role, RowState, SelectMode, Show as _, ShowContainer as _, Sizing,
    WordSetCx as _,
};
use crate::display::Divider;
use crate::list::{DropTest, ListRow, RowEvent, RowTest, RowText};
use crate::menu::{Menu, MenuItem};
use crate::runtime::env::Label;
use crate::style::{ComponentStyle, StyleCx};
use craft_behavior::{ItemSource, Metrics, Selection, SpanLayout};
use egui::{Rect, Ui};
use std::hash::Hash;

/// The header menu's popup.
const HEADER_MENU: &str = "header_menu";

/// The style of the table: it has no look of its own beyond its container's surface.
#[derive(Clone, Debug, Default, PartialEq, ComponentStyle)]
#[non_exhaustive]
pub struct TableStyle {
    #[style(metrics)]
    metrics: Metrics,
}

/// A table: one Tab stop, a header row over the columns, the selection rules of the collection contract, type-ahead on
/// the rows' text (with `type_ahead`), rename in place, drag to reorder, and the requests for the app's row and header
/// menus.
///
/// The app sorts its rows and tells the table the order it used (`sort`); a press on a sortable header reports
/// `ColumnEvent::Sort` with the next order. A secondary click on a header, or on the empty header after the last
/// one, or the Menu key on a focused header, opens the header menu: "Columns" over a check per hideable column, which
/// shows or hides it and leaves the menu open, then Reset Columns (the app's order, every column shown, the app's
/// widths; the sort stays), and for a header the column commands (Resize Column…, Move Column Left and Right). Ctrl or
/// Cmd with Left or Right on a focused header moves its column. What the user changes is reported as `ColumnEvent`s
/// for the app to save. A table of groups and their children is a tree: the label column is
/// indented by depth and groups open and close as in a `List`.
#[derive(crate::contract::Component, crate::contract::Props)]
#[component(collection, overlay, probe(Table<'static, usize>))]
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
    /// The text a row's rename field opens with, `None` for the row's text: an empty draft when several rows are picked
    /// takes a name for all of them.
    #[prop(func = "Fn(&K) -> Option<String>")]
    rename_draft: Option<RowText<'a, K>>,
    /// Which rows may be dragged.
    #[prop(func = "Fn(&K) -> bool")]
    can_drag: Option<RowTest<'a, K>>,
    /// What the table takes from drags that did not start in it, in order.
    drops: Vec<DropKind<'a, K>>,
    /// Where the table's own drag may land: the test gets the drag, the row under the pointer and the position before which the
    /// dragged rows would go. A refused spot shows no drop mark.
    #[prop(func = "Fn(&ListDrag<K>, Option<&K>, usize) -> bool")]
    accepts: Option<DropTest<'a, K>>,
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
            rename_draft: None,
            can_drag: None,
            drops: Vec::new(),
            accepts: None,
        }
        .label(label)
    }

    /// Takes the drops of `kind` from drags that did not start in the table; kinds given earlier come first.
    #[must_use]
    pub fn takes_drop(mut self, kind: DropKind<'a, K>) -> Self {
        self.drops.push(kind);
        self
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

    fn update(&mut self, cx: &mut Cx<'_, Self>) {
        cx.popup(HEADER_MENU);
    }

    /// The header menu: the item step's header asks for it after `update`, so it opens here.
    fn overlay(&mut self, ui: &mut OverlayUi<'_>, cx: &mut Cx<'_, Self>) {
        let popup = cx.popup(HEADER_MENU);
        let Some((_, asked)) = cx.header_menu() else { return };
        if asked {
            popup.open(cx);
        }
        let Some((menu, _)) = cx.header_menu() else { return };
        let (at, for_column) = (menu.at, menu.column.is_some());
        let hidden = cx.hidden_columns().to_vec();
        let hideable: Vec<_> = self.items.columns.iter().filter(|spec| spec.hideable).map(|spec| (spec.key.clone(), spec.header.clone())).collect();
        let commands: Vec<(ColumnCommands, bool)> = if for_column {
            ColumnCommands::ALL.into_iter().map(|command| (command, cx.column_command_enabled(command))).collect()
        } else {
            Vec::new()
        };
        let ctx = cx.core.ctx.clone();
        let words = cx.words::<TableWords>();
        let mut picks = Vec::new();
        let layer = crate::menu::layer(false).anchor(Anchor::Point(at));
        ui.show(popup, layer, |ui, _| {
            Menu::inline(words.columns.clone()).show(ui, |ui, _| {
                if !hideable.is_empty() {
                    MenuItem::new(words.columns.clone()).enabled(false).show(ui);
                    for (key, header) in &hideable {
                        let shown = !hidden.contains(key);
                        if MenuItem::new(header.clone()).check(shown).keep_open(true).show(ui).ran {
                            picks.push(ColumnRequest::Shown(key.clone(), !shown));
                        }
                    }
                    Divider::horizontal().show(ui);
                }
                if MenuItem::new(words.reset_columns.clone()).show(ui).ran {
                    picks.push(ColumnRequest::Reset);
                }
                if !commands.is_empty() {
                    Divider::horizontal().show(ui);
                }
                for (command, enabled) in &commands {
                    let label = crate::runtime::commands::resolve(&ctx, *command).label;
                    if MenuItem::new(label.to_string()).enabled(*enabled).show(ui).ran {
                        picks.push(ColumnRequest::Command(*command));
                    }
                }
            });
        });
        for pick in picks {
            cx.column_request(pick);
        }
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

    /// Column widths come from the specs and the header's memory; a row is as tall as `CollectionProps::row_height`.
    fn item_layout(&self, source: &dyn ItemSource<K>, width: f32, cx: &StyleCx<'_>) -> SpanLayout {
        let layout = SpanLayout::sized(source, width, self.items.row_height.of(cx));
        if self.tree { layout.tree() } else { layout }
    }

    fn dress<'r>(&self, row: Self::Item<'r>, _: &RowState) -> Self::Item<'r> {
        row.rows(self.items.row_height)
    }

    fn rename_draft(&self, key: &K) -> Option<String> {
        self.rename_draft.as_ref().and_then(|draft| draft.text(key))
    }

    fn can_rename(&self, key: &K) -> bool {
        self.can_rename.as_ref().is_none_or(|test| test.passes(key))
    }

    fn can_drag(&self, key: &K) -> bool {
        self.can_drag.as_ref().is_none_or(|test| test.passes(key))
    }

    fn drops(&self) -> &[DropKind<'_, K>] {
        &self.drops
    }

    fn accepts(&self, drag: &ListDrag<K>, over: Option<&K>, insert: usize) -> bool {
        self.accepts.as_ref().is_none_or(|test| test.passes(drag, over, insert))
    }

    fn rename_editor(&mut self, ui: &mut Ui, draft: Bind<'_, String>, label: Rect, name: &str) -> Option<EditResponse<String>> {
        Some(crate::text::rename_field(ui, draft, label, name))
    }
}
