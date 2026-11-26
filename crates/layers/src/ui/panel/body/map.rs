//! Maps the list's events to the panel's events.

use craft_behavior::{ColumnKey, DropOperation, EditEvent, Mixed, SelectionChange};
use craft_keymap::Mods;
use craft_ui::contract::{CollectionEvent, OpenChange, request_rename};
use craft_ui::list::{RowEvent, ThumbTarget};
use craft_ui::runtime::env::Label;
use egui::Ui;
use std::sync::Arc;

use super::super::Session;
use super::super::context::PanelContext;
use super::super::drag::{LayersDrag, drop_event, host_drop_place, part_drop_event};
use super::super::events::{ActivateOn, LayerColumn, LayerSelect, LayerToggle, LayersEvent, MenuTarget};
use super::super::menu::{OpenMenu, shared_label};
use super::super::row::{EYE, FX_PART, LOCK_PART, NAME, OPTIONS_PART};
use super::rows::{ToggleSweeps, thumb_click_replaces_select, thumb_part};
use crate::ui::sweep::{SweepOut, sweep_events};
use crate::ui::tree::{LayerRowKey, LayerTree};

#[allow(clippy::too_many_arguments)]
pub(super) fn map_events<T: LayerTree>(
    cx: &PanelContext<'_, T>,
    order: &[LayerRowKey<T::Id>],
    parent_of: &dyn Fn(&T::Id) -> Option<T::Id>,
    filtered: bool,
    lock_name: &Label,
    session: &mut Session<T::Id>,
    list: Vec<CollectionEvent<LayerRowKey<T::Id>, RowEvent>>,
    events: &mut Vec<LayersEvent<T::Id>>,
) {
    let tree = cx.tree;
    let clicks: Vec<(LayerRowKey<T::Id>, ThumbTarget, Mods, u32)> = list
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Item(key, RowEvent::ThumbClicked { target, mods, clicks, .. }) => Some((key.clone(), *target, *mods, *clicks)),
            _ => None,
        })
        .collect();
    let mut used = vec![false; clicks.len()];
    let activated: Vec<LayerRowKey<T::Id>> = list
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Activate(key) => Some(key.clone()),
            _ => None,
        })
        .collect();

    let mut toggle_label: Arc<str> = Arc::from("");
    for event in list {
        match event {
            CollectionEvent::Selection { change, mods, .. } => {
                // A set made on an effects or Smart Filter row names that row; a toggle acts on its layer only.
                let (row, set, mapped) = match change {
                    SelectionChange::Set(keys) => (keys.last().cloned(), true, SelectionChange::Set(layers_of(&keys))),
                    SelectionChange::Toggle(key) => (Some(key.clone()), false, SelectionChange::Toggle(key.layer().clone())),
                    SelectionChange::Clear => (None, false, SelectionChange::Clear),
                    _ => continue,
                };
                // The selection names the thumbnail part its click hit, a double click's included: a double click still
                // reports its activation below, and only a single click acts in place of the select.
                let mut part = None;
                if let Some(row) = &row
                    && let Some(at) = clicks.iter().enumerate().position(|(i, click)| click.0 == *row && !used[i])
                {
                    used[at] = true;
                    let info = tree.info(row.layer());
                    let hit = thumb_part(row, &info, clicks[at].1);
                    if clicks[at].3 < 2 && thumb_click_replaces_select(hit, clicks[at].2, &info) {
                        events.push(LayersEvent::ThumbClick { layer: row.layer().clone(), part: hit, mods: clicks[at].2 });
                        continue;
                    }
                    part = Some(hit);
                }
                let mut select = LayerSelect::new(mapped, part);
                select.mods = mods;
                if set && let Some(row) = &row {
                    select = select.on_row(row);
                }
                events.push(LayersEvent::Select(select));
            }
            // A double click on a thumbnail is reported once, by its own click.
            CollectionEvent::Activate(row) if clicks.iter().any(|click| click.0 == row && click.3 == 2) => {}
            CollectionEvent::Activate(row) => {
                let on = clicks
                    .iter()
                    .find(|click| click.0 == row)
                    .map_or(ActivateOn::Row, |click| ActivateOn::Thumbnail(thumb_part(&row, &tree.info(row.layer()), click.1)));
                events.push(LayersEvent::Activate { row, on });
            }
            CollectionEvent::Open(change) => {
                if let LayerRowKey::Layer(id) = &change.key {
                    let mut open = OpenChange::new(id.clone(), change.open);
                    if change.all {
                        open = open.all();
                    }
                    events.push(LayersEvent::Open(open));
                }
            }
            CollectionEvent::ContextMenu(request) => {
                let layer = request.key.layer().clone();
                let eye = request.column.as_ref() == Some(&EYE) && matches!(request.key, LayerRowKey::Layer(_));
                let plain = request.column.as_ref().is_none_or(|column| *column == NAME)
                    || matches!(request.key, LayerRowKey::Effects(..) | LayerRowKey::Filters(..));
                let selected = cx.selection.layers.contains(&layer);
                if eye {
                    let rows = if selected { cx.selection.layers.clone() } else { vec![layer] };
                    let current = shared_label(tree, &rows);
                    session.menu = Some(OpenMenu::Eye { rows, current });
                } else if plain {
                    if !selected {
                        events.push(LayersEvent::Select(LayerSelect::new(SelectionChange::Set(vec![layer.clone()]), None).on_row(&request.key)));
                        session.produced = vec![layer.clone()];
                        session.anchor = Some(LayerRowKey::Layer(layer));
                    }
                    let on_selection = selected && cx.selection.layers.len() > 1;
                    // A right-click on a thumbnail opens the menu for what it shows (a mask).
                    let part =
                        request.part.and_then(ThumbTarget::of_part).map(|which| thumb_part(&request.key, &tree.info(request.key.layer()), which));
                    session.menu = Some(OpenMenu::Row(MenuTarget { row: Some(request.key.clone()), part, on_selection }));
                }
            }
            CollectionEvent::EdgePressed { above: LayerRowKey::Layer(upper), .. } => {
                let release = tree.info(&upper).clipped;
                events.push(LayersEvent::ClipLine { upper, release });
            }
            CollectionEvent::EmptyContextMenu { .. } => {
                session.menu = Some(OpenMenu::Row(MenuTarget { row: None, part: None, on_selection: false }));
            }
            CollectionEvent::Reorder(edit) => events.extend(drop_event(edit, order, parent_of, filtered)),
            CollectionEvent::Dropped { at, op, payload, .. } => match payload.get::<LayersDrag<T::Id>>() {
                Some(drag) => events.extend(part_drop_event(cx.list, drag, &at, op, Arc::from(cx.words.list.as_str()))),
                // Only the host's payload type is taken besides the panel's own drag.
                None => {
                    if let Some((target, position)) = host_drop_place(order, parent_of, &at) {
                        events.push(LayersEvent::host_drop(payload, target, position, op == DropOperation::Copy));
                    }
                }
            },
            CollectionEvent::Delete(keys) => events.push(LayersEvent::Delete(keys)),
            CollectionEvent::Toggle(edit) => toggle_events(edit, session.sweeps_for(cx.look), &mut toggle_label, events),
            CollectionEvent::Rename(edit) => {
                if let EditEvent::Commit { after, .. } | EditEvent::Set { after, .. } = edit
                    && let LayerRowKey::Layer(id) = &after.key
                {
                    let text = after.name.trim();
                    if !text.is_empty() && text != tree.info(id).name {
                        events.push(LayersEvent::Rename { layer: id.clone(), text: text.to_owned() });
                    }
                }
            }
            CollectionEvent::Item(row, RowEvent::LinkToggled { thumb, linked, .. }) => {
                let info = tree.info(row.layer());
                if let Some(part) = info.thumb_shows(ThumbTarget::Extra(thumb)).filter(|part| info.masks.iter().any(|mask| mask.part == *part)) {
                    events.push(LayersEvent::MaskLink { layer: row.layer().clone(), part, linked });
                }
            }
            CollectionEvent::Item(row, RowEvent::Trailing { part, mods, clicks, source, .. }) => {
                let layer = row.layer().clone();
                match part {
                    part if part == craft_ui::list::TrailingPart::new(LOCK_PART) => {
                        let unlock = EditEvent::set(Mixed::Same(true), false, Arc::from(lock_name.as_str()), None);
                        events.push(LayersEvent::Toggle(LayerToggle::new(vec![row], LayerColumn::Lock, unlock)));
                    }
                    part if part == craft_ui::list::TrailingPart::new(FX_PART) => {
                        events.push(LayersEvent::FxOpen { open: !tree.effects_open(&layer), layer, all: mods.alt });
                    }
                    // The Blending Options open on a double click, as Photoshop's do, or on any non-pointer activation.
                    part if part == craft_ui::list::TrailingPart::new(OPTIONS_PART)
                        && (clicks == 2 || source != craft_ui::runtime::commands::FireSource::Pointer) =>
                    {
                        events.push(LayersEvent::Activate { row, on: ActivateOn::Options });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // A thumbnail's double click activates it; any other click that selected nothing new still acts.
    for (i, (row, target, mods, count)) in clicks.iter().enumerate() {
        if (used[i] && *count != 2) || (*count < 2 && activated.contains(row)) {
            continue;
        }
        let info = tree.info(row.layer());
        let part = thumb_part(row, &info, *target);
        if *count == 2 {
            events.push(LayersEvent::Activate { row: row.clone(), on: ActivateOn::Thumbnail(part) });
            continue;
        }
        events.push(if thumb_click_replaces_select(part, *mods, &info) {
            LayersEvent::ThumbClick { layer: row.layer().clone(), part, mods: *mods }
        } else {
            let mut select = LayerSelect::new(SelectionChange::Set(vec![row.layer().clone()]), Some(part)).on_row(row);
            select.mods = *mods;
            LayersEvent::Select(select)
        });
    }
}

fn layers_of<Id: Clone + PartialEq>(keys: &[LayerRowKey<Id>]) -> Vec<Id> {
    let mut layers: Vec<Id> = Vec::new();
    for key in keys {
        if !layers.contains(key.layer()) {
            layers.push(key.layer().clone());
        }
    }
    layers
}

fn column_of(column: &ColumnKey) -> LayerColumn {
    if *column == EYE { LayerColumn::Visibility } else { LayerColumn::Lock }
}

/// The layers panel's toggle events, as [`sweep_events`] groups them: the eye and lock columns are sweeps of one
/// column each.
impl<Id: Clone + PartialEq> SweepOut<LayerRowKey<Id>, LayerColumn> for Vec<LayersEvent<Id>> {
    fn last_toggle(&mut self) -> Option<(LayerColumn, &EditEvent<bool>, &mut Vec<LayerRowKey<Id>>)> {
        match self.last_mut() {
            Some(LayersEvent::Toggle(toggle)) => Some((toggle.column, &toggle.edit, &mut toggle.rows)),
            _ => None,
        }
    }

    fn push_toggle(&mut self, rows: Vec<LayerRowKey<Id>>, column: LayerColumn, edit: EditEvent<bool>) {
        self.push(LayersEvent::Toggle(LayerToggle::new(rows, column, edit)));
    }

    fn show_only(&mut self, key: LayerRowKey<Id>) {
        self.push(LayersEvent::ShowOnly(key));
    }

    fn undo_show_only(&mut self, key: LayerRowKey<Id>) {
        self.push(LayersEvent::UndoShowOnly(key));
    }

    fn claimed(&mut self, key: LayerRowKey<Id>, column: LayerColumn, mods: Mods) {
        self.push(LayersEvent::cell_click(key, column, mods));
    }
}

/// The cells of an eye or lock sweep as events: the column the edit names, then [`sweep_events`] for the rules.
fn toggle_events<Id: Clone + PartialEq>(
    edit: EditEvent<craft_ui::contract::ToggleEdit<LayerRowKey<Id>>>,
    sweeps: &mut ToggleSweeps,
    label: &mut Arc<str>,
    events: &mut Vec<LayersEvent<Id>>,
) {
    let column = match &edit {
        EditEvent::Update { value: cell, .. } | EditEvent::Set { after: cell, .. } | EditEvent::Commit { after: cell, .. } => column_of(&cell.column),
        EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => column_of(&cell.column),
        _ => return,
    };
    sweep_events(sweeps.of(column), column, edit, label, events);
}

/// Opens the rename field of `layer`'s row in the list `list`.
pub(in crate::ui::panel) fn start_rename<K: Clone + Eq + std::hash::Hash + Send + Sync + 'static>(ui: &Ui, list: egui::Id, layer: K) {
    request_rename(ui.ctx(), list, &LayerRowKey::Layer(layer));
}
