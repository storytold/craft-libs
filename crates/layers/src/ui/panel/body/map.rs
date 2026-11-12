//! Maps the list's events to the panel's events.

use craft_behavior::{ColumnKey, EditEvent, GestureId, Mixed, SelectionChange};
use craft_keymap::Mods;
use craft_ui::contract::{CollectionEvent, OpenChange, SweepStep, request_rename};
use craft_ui::list::{RowEvent, ThumbTarget};
use craft_ui::runtime::env::Label;
use egui::Ui;
use std::sync::Arc;

use super::super::Session;
use super::super::context::PanelContext;
use super::super::drag::{LayersDrag, drop_event, part_drop_event};
use super::super::events::{ActivateOn, LayerColumn, LayerSelect, LayerToggle, LayersEvent, MenuTarget};
use super::super::menu::{OpenMenu, shared_label};
use super::super::row::{EYE, FX_PART, LOCK_PART, NAME, OPTIONS_PART};
use super::rows::{ToggleSweeps, extra_parts, thumb_click_replaces_select, thumb_part};
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
    let clicks: Vec<(LayerRowKey<T::Id>, ThumbTarget, Mods)> = list
        .iter()
        .filter_map(|event| match event {
            CollectionEvent::Item(key, RowEvent::ThumbClicked { target, mods, .. }) => Some((key.clone(), *target, *mods)),
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
            CollectionEvent::Selection(change) => {
                // A set made on an effects or Smart Filter row names that row; a toggle acts on its layer only.
                let (row, set, mapped) = match change {
                    SelectionChange::Set(keys) => (keys.last().cloned(), true, SelectionChange::Set(layers_of(&keys))),
                    SelectionChange::Toggle(key) => (Some(key.clone()), false, SelectionChange::Toggle(key.layer().clone())),
                    SelectionChange::Clear => (None, false, SelectionChange::Clear),
                    _ => continue,
                };
                let mut part = None;
                if let Some(row) = &row
                    && let Some(at) = clicks.iter().enumerate().position(|(i, click)| click.0 == *row && !used[i])
                {
                    used[at] = true;
                    let info = tree.info(row.layer());
                    let hit = thumb_part(row, &info, clicks[at].1);
                    if thumb_click_replaces_select(hit, clicks[at].2, &info) {
                        events.push(LayersEvent::ThumbClick { layer: row.layer().clone(), part: hit, mods: clicks[at].2 });
                        continue;
                    }
                    part = Some(hit);
                }
                let mut select = LayerSelect::new(mapped, part);
                if set && let Some(row) = &row {
                    select = select.on_row(row);
                }
                events.push(LayersEvent::Select(select));
            }
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
            CollectionEvent::EmptyContextMenu { .. } => {
                session.menu = Some(OpenMenu::Row(MenuTarget { row: None, part: None, on_selection: false }));
            }
            CollectionEvent::Reorder(edit) => events.extend(drop_event(edit, order, parent_of, filtered)),
            CollectionEvent::Dropped { at, op, payload, .. } => {
                if let Some(drag) = payload.get::<LayersDrag<T::Id>>() {
                    events.extend(part_drop_event(cx.list, drag, &at, op, Arc::from(cx.words.list.as_str())));
                }
            }
            CollectionEvent::Toggle(edit) => toggle_events(edit, &mut session.sweeps, &mut toggle_label, events),
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
                if let Some(&part) = extra_parts(&info).get(thumb).filter(|part| info.masks.iter().any(|mask| mask.part == **part)) {
                    events.push(LayersEvent::MaskLink { layer: row.layer().clone(), part, linked });
                }
            }
            CollectionEvent::Item(row, RowEvent::Trailing { part, mods, clicks, .. }) => {
                let layer = row.layer().clone();
                match part {
                    part if part == craft_ui::list::TrailingPart::new(LOCK_PART) => {
                        let unlock = EditEvent::set(Mixed::Same(true), false, Arc::from(lock_name.as_str()), None);
                        events.push(LayersEvent::Toggle(LayerToggle::new(vec![row], LayerColumn::Lock, unlock)));
                    }
                    part if part == craft_ui::list::TrailingPart::new(FX_PART) => {
                        events.push(LayersEvent::FxOpen { open: !tree.effects_open(&layer), layer, all: mods.alt });
                    }
                    // The Blending Options open on a double click, as Photoshop's do.
                    part if part == craft_ui::list::TrailingPart::new(OPTIONS_PART) && clicks == 2 => {
                        events.push(LayersEvent::Activate { row, on: ActivateOn::Options });
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }
    // A thumbnail click that selected nothing new still acts.
    for (i, (row, target, mods)) in clicks.iter().enumerate() {
        if used[i] || activated.contains(row) {
            continue;
        }
        let info = tree.info(row.layer());
        let part = thumb_part(row, &info, *target);
        events.push(if thumb_click_replaces_select(part, *mods, &info) {
            LayersEvent::ThumbClick { layer: row.layer().clone(), part, mods: *mods }
        } else {
            LayersEvent::Select(LayerSelect::new(SelectionChange::Set(vec![row.layer().clone()]), Some(part)).on_row(row))
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

/// The cells of an eye or lock sweep as events. The cells of one sweep are `Update`s of the gesture of its first, opened by a
/// `Begin` and closed by one `Commit` on release; the cells of one pass share one event, in the order crossed. An Escape reports the cells
/// as one `Cancel` of that gesture with the state each had before. A plain press is a `Set`. Alt on the first press of
/// an eye sweep shows only that layer and drops the rest of the sweep.
fn toggle_events<Id: Clone + PartialEq>(
    edit: EditEvent<craft_ui::contract::ToggleEdit<LayerRowKey<Id>>>,
    sweeps: &mut ToggleSweeps,
    label: &mut Arc<str>,
    events: &mut Vec<LayersEvent<Id>>,
) {
    if let EditEvent::Begin { label: named, .. } = &edit {
        *label = named.clone();
    }
    let column = match &edit {
        EditEvent::Update { value: cell, .. } | EditEvent::Set { after: cell, .. } | EditEvent::Commit { after: cell, .. } => column_of(&cell.column),
        EditEvent::Begin { before: Mixed::Same(cell), .. } | EditEvent::Cancel { before: Some(Mixed::Same(cell)), .. } => column_of(&cell.column),
        _ => return,
    };
    // The sweep ends with its first Commit or Cancel of a pass; the cells after it still belong to its gesture.
    let running = sweeps.of(column).gesture();
    let ending = matches!(edit, EditEvent::Commit { .. } | EditEvent::Cancel { .. });
    let Some(step) = sweeps.of(column).feed(&edit) else {
        // The first cell of a sweep opens its gesture; the cells after it join that one.
        // An Alt eye sweep only shows one layer: it has no gesture to report.
        let shows_only = column == LayerColumn::Visibility;
        if let EditEvent::Begin { before: Mixed::Same(before), .. } = &edit
            && !(shows_only && before.mods.alt)
            && running.is_none()
            && let Some(gesture) = sweeps.of(column).gesture()
        {
            let begin = EditEvent::begin(gesture, Mixed::Same(before.on), label.clone(), None);
            push_cell(events, Some(gesture), before.key.clone(), column, begin);
        }
        if let EditEvent::Commit { before: Mixed::Same(before), after, .. } = &edit
            && !(shows_only && after.mods.alt)
            && let Some(gesture) = running
        {
            let commit = EditEvent::commit(gesture, Mixed::Same(before.on), after.on);
            push_cell(events, Some(gesture), after.key.clone(), column, commit);
        }
        return;
    };
    let gesture = if ending { running.or_else(|| closing_gesture(events, column)) } else { sweeps.of(column).gesture() };
    match step {
        // Only the eye column shows one layer; the lock column is plain and never gives `ShowOnly`.
        SweepStep::ShowOnly(cell) => events.push(LayersEvent::ShowOnly(cell.key)),
        SweepStep::Flip(cell) => {
            let edit = match gesture {
                Some(id) => EditEvent::update(id, cell.on),
                None => EditEvent::set(Mixed::Same(!cell.on), cell.on, label.clone(), None),
            };
            push_cell(events, gesture, cell.key, column, edit);
        }
        SweepStep::Restore(cell) => {
            let EditEvent::Cancel { reason, .. } = edit else { return };
            let Some(gesture) = gesture else { return };
            let edit = EditEvent::cancel(gesture, reason, Some(Mixed::Same(cell.on)));
            push_cell(events, Some(gesture), cell.key, column, edit);
        }
        SweepStep::UndoShowOnly(cell) => events.push(LayersEvent::UndoShowOnly(cell.key)),
        _ => {}
    }
}

/// The gesture of the sweep of `column` that this pass already closed, for the cells of its release or cancel that come
/// after the first.
fn closing_gesture<Id>(events: &[LayersEvent<Id>], column: LayerColumn) -> Option<GestureId> {
    events.iter().rev().find_map(|event| match event {
        LayersEvent::Toggle(toggle) if toggle.column == column => toggle.edit.id(),
        _ => None,
    })
}

/// Adds `key` to the last toggle event when it is the same kind of the same gesture, and starts a new event otherwise.
/// Events with no gesture (a plain press) never merge.
fn push_cell<Id: Clone + PartialEq>(
    events: &mut Vec<LayersEvent<Id>>,
    gesture: Option<GestureId>,
    key: LayerRowKey<Id>,
    column: LayerColumn,
    edit: EditEvent<bool>,
) {
    if let Some(LayersEvent::Toggle(last)) = events.last_mut()
        && gesture.is_some()
        && last.column == column
        && last.edit.id() == gesture
        && std::mem::discriminant(&last.edit) == std::mem::discriminant(&edit)
    {
        last.rows.push(key);
        return;
    }
    events.push(LayersEvent::Toggle(LayerToggle::new(vec![key], column, edit)));
}

/// Opens the rename field of `layer`'s row in the list `list`.
pub(in crate::ui::panel) fn start_rename<K: Clone + Eq + std::hash::Hash + Send + Sync + 'static>(ui: &Ui, list: egui::Id, layer: K) {
    request_rename(ui.ctx(), list, &LayerRowKey::Layer(layer));
}
