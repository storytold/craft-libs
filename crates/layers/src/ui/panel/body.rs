//! The rows: the display order over the tree, the `List` wiring, and the mapping of the list's
//! events to the panel's.

mod map;
mod rows;

use map::map_events;
pub(in crate::ui::panel) use map::start_rename;
pub(in crate::ui::panel) use rows::{LayerRows, ToggleSweeps, reveal_plan};
use rows::{RevealPlan, list_selection};

use craft_behavior::{ColumnWidth, IconId};
use craft_ui::contract::{Bind, CollectionEvent, ColumnKind, ColumnSpec, ListDrag, Props as _, ReorderRule, ShowCollection as _, ToggleColumn};
use craft_ui::list::List;
use craft_ui::runtime::env::Label;
use egui::Ui;

use super::Session;
use super::context::PanelContext;
use super::drag::{LayersDrag, accepts_drop};
use super::events::LayersEvent;
use super::row::{EYE, LOCK, NAME, build_row};
use crate::ui::tree::{LayerRowKey, LayerTree};

/// What the rows' pass returns.
pub(super) struct BodyOutput<Id> {
    pub(super) events: Vec<LayersEvent<Id>>,
    pub(super) response: egui::Response,
}

/// Shows the list over `rows` and maps what it reports.
pub(super) fn show_body<T: LayerTree>(
    ui: &mut Ui,
    cx: &PanelContext<'_, T>,
    rows: &LayerRows<'_, T>,
    session: &mut Session<T::Id>,
    reveal: &RevealPlan<T::Id>,
    filtered: bool,
    lock_name: &Label,
) -> BodyOutput<T::Id> {
    let tree = cx.tree;
    let order = rows.keys();
    let mut selection = list_selection(cx, session);
    let reveal_key =
        reveal.target.clone().filter(|id| order.iter().any(|key| matches!(key, LayerRowKey::Layer(k) if k == id))).map(LayerRowKey::Layer);

    let mut columns = vec![
        ColumnSpec::new(EYE, Label::Empty, ColumnWidth::Controls(1.0)).kind(ColumnKind::Toggle(ToggleColumn::sweepable())).icon(IconId::new("eye")),
    ];
    if cx.lock_column {
        columns.push(
            ColumnSpec::new(LOCK, Label::Empty, ColumnWidth::Controls(1.0))
                .kind(ColumnKind::Toggle(ToggleColumn::sweepable()))
                .icon(IconId::new("lock")),
        );
    }
    columns.push(ColumnSpec::new(NAME, Label::Empty, ColumnWidth::fill(1.0)));
    let parent_of = |id: &T::Id| tree.parent(id);
    let ctx = ui.ctx().clone();

    let mut list = List::new(cx.words.list.clone())
        .id(cx.list)
        .tree(true)
        .reorder(ReorderRule::new().into_groups().copy_with_alt())
        .allow_empty(true)
        .rename(true)
        .rename_column(NAME)
        .columns(columns)
        .selection(Bind::bound(&mut selection))
        .can_rename(|key| matches!(key, LayerRowKey::Layer(id) if tree.info(id).renamable))
        .accepts(|drag: &ListDrag<LayerRowKey<T::Id>>, _over, insert| {
            accepts_drop(tree, drag, &order, insert, craft_ui::runtime::pointer::frame(&ctx).mods)
        })
        .drag_payload(|drag: &ListDrag<LayerRowKey<T::Id>>| Some(LayersDrag::of_rows(drag)));
    if let Some(key) = reveal_key {
        list = list.reveal(key);
    }
    let out = list.show_items(ui, rows, &mut |key, item| {
        let row = build_row(cx, key).cells(item.cells()).state(item.row);
        match rows.rows.get(item.at) {
            Some(shown) if shown.has_children => row.depth(shown.paint_depth).group(shown.open),
            Some(shown) => row.depth(shown.paint_depth),
            None => row,
        }
    });

    let mut events: Vec<LayersEvent<T::Id>> = reveal.opens.iter().cloned().map(LayersEvent::Open).collect();
    let selection_changed = out.events.iter().any(|event| matches!(event, CollectionEvent::Selection(_)));
    let list_events = out.events;
    map_events(cx, &order, &parent_of, filtered, lock_name, session, list_events, &mut events);
    if selection_changed {
        let mut layers: Vec<T::Id> = Vec::new();
        for key in &selection.items {
            if !layers.contains(key.layer()) {
                layers.push(key.layer().clone());
            }
        }
        session.anchor = selection.anchor.as_ref().map(|key| LayerRowKey::Layer(key.layer().clone()));
        session.produced = layers;
    }
    BodyOutput { events, response: out.response }
}
