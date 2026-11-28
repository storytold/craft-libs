//! The lead and the selection: picks, ranges, and where the lead starts and follows.

use super::{ItemsHooks, Placement, members};
use crate::contract::collection::{CollectionEvent, HoverLead, LeadOnOpen, LeadSource, ListDrag, SelectMode};
use crate::contract::{Collection, Cx};
use crate::runtime::memory::sub;
use crate::runtime::{drag, pointer};
use craft_behavior::{Flow, ItemLayout, PointerEvent, SelectionChange, SelectionModel, Sense};
use craft_keymap::Mods;
use egui::{Context, Id, Pos2, Rect, Vec2};
use std::any::TypeId;

/// What made a selection change, reported with it (`CollectionEvent::Selection`): a key, or a pointer pick with the
/// modifiers it was made with.
#[derive(Clone, Copy, Debug)]
pub(super) enum PickBy {
    Keys,
    Pointer(Mods),
}

/// What the rest timer of `drag_select` last saw: the drag's pointer and the row it is timing.
#[derive(Clone, Copy, Default)]
struct DragRest {
    pointer: Option<Pos2>,
    row: Option<usize>,
}

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    pub(super) fn select_mode(comp: &C) -> SelectMode {
        comp.items().select
    }

    /// [`change_by`](Self::change_by) for a pointer change with no modifiers.
    // PLAN: rows.rs (`select_rows`, `cell_move`, `cell_press`), marquee.rs and access.rs report through this without their source; their owner passes a `PickBy` (cell_move's is `PickBy::Keys`).
    pub(super) fn change(&mut self, change: SelectionChange<C::Key>) {
        self.change_by(change, PickBy::Pointer(Mods::NONE));
    }

    /// Applies a selection change `by` a key or a pick and reports it when it changes the selection: a press on the
    /// row a hover already selected reports nothing. A key's change carries no modifiers.
    pub(super) fn change_by(&mut self, change: SelectionChange<C::Key>, by: PickBy) {
        let before = self.selection.clone();
        self.selection.apply(change.clone());
        if self.selection != before {
            let (by, mods) = match by {
                PickBy::Keys => (LeadSource::Keys, Mods::NONE),
                PickBy::Pointer(mods) => (LeadSource::Pointer, mods),
            };
            self.events.push(CollectionEvent::Selection { change, by, mods });
        }
    }

    pub(super) fn range(&self, from: usize, to: usize) -> Vec<C::Key> {
        let keys = (from.min(to)..=from.max(to))
            .filter(|&at| self.source.enabled(at) && (!self.source.is_section(at) || self.source.selectable_section(at)))
            .map(|at| self.source.key(at));
        if from <= to { keys.collect() } else { keys.rev().collect() }
    }

    /// Moves the lead to row `to`. A lead a key moved has rested at once: no timer runs for it.
    pub(super) fn move_lead(&mut self, to: usize, by: LeadSource) {
        let key = self.place_lead(to, by);
        if matches!(by, LeadSource::Keys) {
            self.report_rest(key);
        }
    }

    /// Places the lead on row `to` and reports the move, with no rest: the lead's key.
    fn place_lead(&mut self, to: usize, by: LeadSource) -> C::Key {
        let key = self.source.key(to);
        if self.lead.key.as_ref() != Some(&key) {
            self.lead.rested = None;
        }
        self.lead.key = Some(key.clone());
        self.lead.at = to;
        self.events.push(CollectionEvent::LeadMoved { key: key.clone(), by });
        key
    }

    /// Reports `CollectionEvent::Rested` for the lead `key`, once per lead change.
    fn report_rest(&mut self, key: C::Key) {
        if self.lead.rested.as_ref() != Some(&key) {
            self.lead.rested = Some(key.clone());
            self.events.push(CollectionEvent::Rested(key));
        }
    }

    /// The position of `key`: the lead's last position is tried first, so the common lookups of a pass do not scan.
    pub(super) fn index_of(&self, key: &C::Key) -> Option<usize> {
        let hint = self.lead.at;
        if hint < self.source.len() && self.source.key(hint) == *key { Some(hint) } else { self.source.index_of(key) }
    }

    pub(super) fn anchor_at(&self, lead: usize) -> usize {
        self.selection.anchor().and_then(|key| self.index_of(key)).unwrap_or(lead)
    }

    /// Whether a later click of a multi-click (`clicks` above one) on the row at `at` leaves the selection as the earlier
    /// click left it: it does when the selection holds the row. What the multi-click activates then acts on that
    /// selection, and no selection change follows the activation.
    pub(super) fn keeps_pick(&self, at: usize, clicks: u32) -> bool {
        clicks > 1 && self.holds(at)
    }

    /// The position of the collection's current item (`CollectionProps::current`) when nothing is picked.
    fn current_when_unpicked(&self, comp: &C) -> Option<usize> {
        let current = comp.items().current.as_deref().and_then(|key| key.downcast_ref::<C::Key>())?;
        let at = self.index_of(current)?;
        self.placed().rows().is_empty().then_some(at)
    }

    /// [`pick_by`](Self::pick_by) for a pointer pick with no modifiers.
    pub(super) fn pick(&mut self, comp: &C, at: usize, extend: bool, toggle: bool) {
        self.pick_by(comp, at, extend, toggle, PickBy::Pointer(Mods::NONE));
    }

    /// A primary click with `mods` held that selects row `at` as `pick_by` does. A plain pick that leaves the selection
    /// as it was reports `CollectionEvent::Picked`.
    pub(super) fn click_pick(&mut self, comp: &C, at: usize, extend: bool, toggle: bool, mods: Mods) {
        let before = self.selection.clone();
        self.pick_by(comp, at, extend, toggle, PickBy::Pointer(mods));
        if !(extend || toggle) && self.selection == before && self.holds(at) {
            self.events.push(CollectionEvent::Picked { key: self.source.key(at), mods });
        }
    }

    /// A press or key `by` that selects row `at` the way the modifiers say.
    pub(super) fn pick_by(&mut self, comp: &C, at: usize, extend: bool, toggle: bool, by: PickBy) {
        if self.source.section_stop(at) && !self.source.selectable_section(at) {
            return;
        }
        let key = self.source.key(at);
        match Self::select_mode(comp) {
            SelectMode::None => {}
            SelectMode::Single => self.change_by(SelectionChange::Set(vec![key]), by),
            SelectMode::Multi | SelectMode::Cells if (extend || !toggle) && self.selection.cells().is_some() => {
                let anchor = if extend { self.anchor_at(self.lead.at) } else { at };
                self.select_rows(comp, anchor, at);
            }
            SelectMode::Multi | SelectMode::Cells => {
                // With nothing picked, the current item counts as picked: a modified click adds to it, as a file
                // manager's or a page grid's does.
                let seed = if extend || toggle { self.current_when_unpicked(comp) } else { None };
                if extend && toggle {
                    self.add_range(at);
                } else if extend {
                    let anchor = self.selection.anchor().and_then(|key| self.index_of(key)).or(seed).unwrap_or(self.lead.at);
                    self.change_by(SelectionChange::Set(self.range(anchor, at)), by);
                } else if toggle {
                    match seed.filter(|&current| current != at) {
                        Some(current) => self.change_by(SelectionChange::Set(vec![self.source.key(current), key]), by),
                        None => self.change_by(SelectionChange::Toggle(key), by),
                    }
                } else if comp.items().picks_beside_current {
                    self.change_by(SelectionChange::Clear, by);
                } else {
                    self.change_by(SelectionChange::Set(vec![key]), by);
                }
            }
        }
    }

    /// Shift+Cmd: the rows from the anchor to `at` join the selection. `Set` takes its first key as the anchor and its
    /// last as the lead, so the anchor's key goes first and the key of `at` last.
    fn add_range(&mut self, at: usize) {
        let anchor = self.anchor_at(self.lead.at);
        let anchor_key = self.source.key(anchor);
        let mut keys = self.selected_keys(&self.selection);
        if let Some(held) = keys.iter().position(|key| *key == anchor_key) {
            keys[..=held].rotate_right(1);
        }
        let added: Vec<C::Key> = self.range(anchor, at).into_iter().filter(|key| !keys.contains(key)).collect();
        keys.extend(added);
        let end = self.source.key(at);
        if let Some(held) = keys.iter().position(|key| *key == end) {
            keys[held..].rotate_left(1);
        }
        self.change(SelectionChange::Set(keys));
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The lead of a collection that shows rows for the first time, or in a layer that opens: the first row that can
    /// be picked when the props ask for it.
    pub(super) fn open_lead(&mut self, comp: &C) {
        if self.lead.key.is_none()
            && comp.items().lead_on_open == LeadOnOpen::First
            && let Some(at) = (0..self.source.len())
                .find(|&at| self.source.enabled(at) && !self.source.is_section(at) && self.layout.as_ref().is_none_or(|layout| layout.shown(at)))
        {
            self.lead.key = Some(self.source.key(at));
            self.lead.at = at;
        }
    }

    /// The row under the pointer while it is over the collection and the row can be picked and shows: the row whose
    /// `ROW` part contains it.
    fn hovered_row(&self, cx: &Cx<'_, C>, layout: &C::ItemLayout) -> Option<usize> {
        if !cx.core.response.hovered() {
            return None;
        }
        self.row_under_pointer(&cx.core.ctx, cx.core.id).filter(|&row| self.source.enabled(row) && !self.source.is_section(row) && layout.shown(row))
    }

    /// The row under `pos` (screen space) in the rows' placement `at`: the drag-select step maps the drag session's
    /// pointer with it.
    pub(super) fn row_under(&self, layout: &C::ItemLayout, at: Placement, pos: Pos2) -> Option<usize> {
        let Placement { rect, offset, rtl } = at;
        let pos = self.xf.map_or(pos, |map| map.to_layer(pos));
        if !rect.contains(pos) {
            return None;
        }
        let local = pos - rect.min.to_vec2();
        let local = if layout.mirrors() && rtl {
            Flow::new(Rect::from_min_size(Pos2::ZERO, rect.size()), rtl).to_physical(Rect::from_min_size(local, Vec2::ZERO)).min
        } else {
            local
        };
        layout.hit(local, offset).filter(|&row| self.source.enabled(row) && !self.source.is_section(row) && layout.shown(row))
    }

    /// The lead follows a moving pointer over the rows; with `HoverLead::LeadAndSelection` the selection follows the
    /// lead the way it does for the arrow keys. A pass moves the pointer when it holds a move event: the pointer entering over a row moves the lead
    /// with no movement to measure, and a modifier change under a resting pointer leaves the lead where the keys put it.
    pub(super) fn hover_lead(&mut self, comp: &C, cx: &Cx<'_, C>, layout: &C::ItemLayout, hover: HoverLead) {
        let moved =
            crate::runtime::pointer::with_pass_raw(&cx.core.ctx, |pass| pass.events.iter().any(|event| matches!(event, PointerEvent::Moved { .. })));
        if !moved {
            return;
        }
        let Some(row) = self.hovered_row(cx, layout) else { return };
        if self.lead.key.as_ref() != Some(&self.source.key(row)) {
            self.move_lead(row, LeadSource::Pointer);
            if hover == HoverLead::LeadAndSelection {
                self.pick(comp, row, false, false);
            }
        }
    }

    /// The pointer resting on the lead row for the tooltip delay reports `CollectionEvent::Rested`, once per lead
    /// change. Motion inside the row does not restart the timer, as for a tooltip.
    pub(super) fn rest_lead(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout) {
        let lead = self.lead.key.clone();
        let over = self.hovered_row(cx, layout);
        let held = lead.is_some() && self.lead.rested != lead && over.is_some_and(|row| lead.as_ref() == Some(&self.source.key(row)));
        let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
        if crate::contract::overlay::dwell(&ctx, &mut *cx.core.memory, id, "rest", held, crate::contract::tooltip::DELAY)
            && let Some(key) = lead
        {
            self.report_rest(key);
        }
    }

    /// While a drag session another source started runs over the rows (its payload is neither this collection's row
    /// drag nor a kind it takes), the unselected row under the pointer becomes the lead and the selection once the
    /// pointer has rested on it for the tooltip delay. The drag holds the pointer, so the row comes from the session's
    /// pointer, not the collection's hover. Leaving the row restarts the wait; so does a part of the row that presses
    /// (a tab's close button) under the pointer. A row that comes under a still pointer because the rows scrolled
    /// starts no wait until the pointer moves.
    pub(super) fn drag_select(&mut self, comp: &C, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, at: Placement) {
        let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
        let foreign = drag::running(&ctx)
            && drag::active_of(&ctx, TypeId::of::<ListDrag<C::Key>>()).is_none()
            && !comp.drops().iter().any(|kind| drag::active_of(&ctx, kind.payload()).is_some());
        let pos = drag::session_pointer(&ctx).filter(|_| foreign);
        let slot = sub(id, "drag rest row");
        let before = *cx.core.memory.shown::<DragRest>(slot);
        let target = pos
            .and_then(|pos| self.row_under(layout, at, pos).map(|row| (pos, row)))
            .filter(|&(pos, row)| !self.holds(row) && !self.over_pressing_part(&ctx, id, row, pos))
            .map(|(_, row)| row)
            .filter(|row| before.pointer != pos || before.row == Some(*row));
        *cx.core.memory.shown::<DragRest>(slot) = DragRest { pointer: pos, row: target };
        // Another row restarts the wait.
        if target != before.row {
            crate::contract::overlay::dwell(&ctx, &mut *cx.core.memory, id, "drag rest", false, crate::contract::tooltip::DELAY);
        }
        if crate::contract::overlay::dwell(&ctx, &mut *cx.core.memory, id, "drag rest", target.is_some(), crate::contract::tooltip::DELAY)
            && let Some(row) = target
        {
            self.move_lead(row, LeadSource::Pointer);
            self.pick(comp, row, false, false);
        }
    }

    /// Whether a part of the row `row` that presses (a tab's close button) lies under `pos`, in the collection `id`.
    fn over_pressing_part(&self, ctx: &Context, id: Id, row: usize, pos: Pos2) -> bool {
        let item = members::item_id(id, &self.source.key(row));
        pointer::shapes_at(ctx, pos)
            .into_iter()
            .any(|(owner, part)| owner == item && part.is_some_and(|part| pointer::part_senses(ctx, owner, part, Sense::CLICK)))
    }

    /// A lead whose key left the source moves to the host's current item when the source holds it, with no report: the
    /// host is there already. Otherwise it moves to the key now in its place, else the one before it. The selection is
    /// the host's: a row that left (deleted, or hidden by a filter) is not replaced by its neighbour. The user did not
    /// move the lead, so the new lead does not rest at once.
    pub(super) fn follow_lead_source(&mut self, comp: &C) {
        let len = self.source.len();
        if let Some(key) = self.lead.key.clone()
            && self.index_of(&key).is_none()
        {
            self.lead.key = None;
            let current = comp.items().current.as_deref().and_then(|key| key.downcast_ref::<C::Key>());
            if let Some((at, key)) = current.and_then(|key| self.index_of(key).map(|at| (at, key.clone()))) {
                self.lead.key = Some(key);
                self.lead.at = at;
            } else if len > 0 {
                self.place_lead(self.lead.at.min(len - 1), LeadSource::Keys);
            }
        } else if let Some(at) = self.lead.key.as_ref().and_then(|key| self.index_of(key)) {
            self.lead.at = at;
        }
    }
}
