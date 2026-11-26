//! The sweep helper the toggle columns of the Layers and Layer Comps panels share: the cells of a sweep as events,
//! grouped the same way in both panels.

use std::sync::Arc;

use craft_behavior::{EditEvent, GestureId, Mixed};
use craft_keymap::Mods;
use craft_ui::contract::{SweepApply, SweepStep, ToggleEdit};

/// The cells of a sweep over `column` as events in `out`. The cells of one sweep are `Update`s of the gesture of its
/// first, opened by a `Begin` and closed by one `Commit` on release; the cells of one pass share one event, in the order
/// crossed. An Escape reports the cells as one `Cancel` of that gesture with the state each had before; a `Cancel` joins
/// the last event only when its `before` equals that event's. A plain press is a `Set`. Alt on the first press of a
/// show-only column shows only that row and drops the rest of the sweep.
pub(crate) fn sweep_events<K: Clone, C: Copy + PartialEq, O: SweepOut<K, C>>(
    sweep: &mut SweepApply,
    column: C,
    edit: EditEvent<ToggleEdit<K>>,
    label: &mut Arc<str>,
    out: &mut O,
) {
    if let EditEvent::Begin { label: named, .. } = &edit {
        *label = named.clone();
    }
    // The sweep ends with its first Commit or Cancel of a pass; the cells after it still belong to its gesture.
    let running = sweep.gesture();
    let ending = matches!(edit, EditEvent::Commit { .. } | EditEvent::Cancel { .. });
    // An Alt sweep of a show-only column only shows one row: it has no gesture to report.
    let shows_only = sweep.shows_only();
    let Some(step) = sweep.feed(&edit) else {
        // The first cell of a sweep opens its gesture; the cells after it join that one.
        if let EditEvent::Begin { before: Mixed::Same(before), .. } = &edit
            && !(shows_only && before.mods.alt)
            && running.is_none()
            && let Some(gesture) = sweep.gesture()
        {
            let begin = EditEvent::begin(gesture, Mixed::Same(before.on), label.clone(), None);
            push_cell(out, Some(gesture), before.key.clone(), column, begin);
        }
        if let EditEvent::Commit { before: Mixed::Same(before), after, .. } = &edit
            && !(shows_only && after.mods.alt)
            && let Some(gesture) = running
        {
            let commit = EditEvent::commit(gesture, Mixed::Same(before.on), after.on);
            push_cell(out, Some(gesture), after.key.clone(), column, commit);
        }
        return;
    };
    let gesture = if ending { running.or_else(|| closing_gesture::<K, C, O>(out, column)) } else { sweep.gesture() };
    match step {
        // Only a show-only column gives `ShowOnly` and `UndoShowOnly`; a plain column never does.
        SweepStep::ShowOnly(cell) => out.show_only(cell.key),
        SweepStep::Flip(cell) => {
            let edit = match gesture {
                Some(id) => EditEvent::update(id, cell.on),
                None => EditEvent::set(Mixed::Same(!cell.on), cell.on, label.clone(), None),
            };
            push_cell(out, gesture, cell.key, column, edit);
        }
        SweepStep::Restore(cell) => {
            let EditEvent::Cancel { reason, .. } = edit else { return };
            let Some(gesture) = gesture else { return };
            let edit = EditEvent::cancel(gesture, reason, Some(Mixed::Same(cell.on)));
            push_cell(out, Some(gesture), cell.key, column, edit);
        }
        SweepStep::UndoShowOnly(cell) => out.undo_show_only(cell.key),
        SweepStep::Claimed(cell) => out.claimed(cell.key, column, cell.mods),
        _ => {}
    }
}

/// Where [`sweep_events`] writes a panel's toggle events.
pub(crate) trait SweepOut<K, C> {
    /// The last event when it is a toggle: its column, its edit and its rows.
    fn last_toggle(&mut self) -> Option<(C, &EditEvent<bool>, &mut Vec<K>)>;

    /// A toggle of `rows` in `column`.
    fn push_toggle(&mut self, rows: Vec<K>, column: C, edit: EditEvent<bool>);

    /// Alt on the first press of a show-only column: show only `key`. A plain column never gives it.
    fn show_only(&mut self, key: K) {
        let _ = key;
    }

    /// The cancel of a show-only press on `key`. A plain column never gives it.
    fn undo_show_only(&mut self, key: K) {
        let _ = key;
    }

    /// A first press on `key` in `column` with the claimed `mods` held: it belongs to the host and flips nothing. A
    /// column that claims no modifiers never gives it.
    fn claimed(&mut self, key: K, column: C, mods: Mods) {
        let _ = (key, column, mods);
    }
}

/// The gesture of the sweep of `column` that this pass already closed, for the cells of its release or cancel that come
/// after the first: they arrive one after the other, so the last event holds it. It reads only the last event, which is
/// enough because a `ShowOnly` never closes a gesture.
fn closing_gesture<K, C: Copy + PartialEq, O: SweepOut<K, C>>(out: &mut O, column: C) -> Option<GestureId> {
    let (last, edit, _) = out.last_toggle()?;
    if last == column { edit.id() } else { None }
}

/// Adds `key` to the last toggle event when it is the same kind of the same gesture in the same column, and starts a
/// new event otherwise. Events with no gesture (a plain press) never merge.
fn push_cell<K, C: Copy + PartialEq, O: SweepOut<K, C>>(out: &mut O, gesture: Option<GestureId>, key: K, column: C, edit: EditEvent<bool>) {
    if let Some((last, joined, rows)) = out.last_toggle()
        && gesture.is_some()
        && last == column
        && joined.id() == gesture
        && joins(joined, &edit)
    {
        rows.push(key);
        return;
    }
    out.push_toggle(vec![key], column, edit);
}

/// Whether `edit` can share one event with `last`: the same kind, and for a `Cancel` the same state to go back to,
/// since one event carries one `before` for all its rows.
fn joins(last: &EditEvent<bool>, edit: &EditEvent<bool>) -> bool {
    match (last, edit) {
        (EditEvent::Cancel { before: was, .. }, EditEvent::Cancel { before, .. }) => was == before,
        _ => std::mem::discriminant(last) == std::mem::discriminant(edit),
    }
}
