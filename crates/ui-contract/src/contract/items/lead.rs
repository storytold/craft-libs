//! The lead and the selection: picks, ranges, and where the lead starts and follows.

use super::{ItemsHooks, Placement};
use crate::contract::collection::{CollectionEvent, LeadOnOpen, LeadSource, ListDrag, SelectMode};
use crate::contract::{Collection, Cx};
use craft_behavior::{Flow, ItemLayout, PointerEvent, SelectionChange, SelectionModel};
use egui::{Pos2, Rect, Vec2};
use std::any::TypeId;

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    pub(super) fn select_mode(comp: &C) -> SelectMode {
        comp.items().select
    }

    /// Applies a selection change and reports it when it changes the selection: a press on the row a hover already
    /// selected reports nothing.
    pub(super) fn change(&mut self, change: SelectionChange<C::Key>) {
        let before = self.selection.clone();
        self.selection.apply(change.clone());
        if self.selection != before {
            self.events.push(CollectionEvent::Selection(change));
        }
    }

    pub(super) fn range(&self, from: usize, to: usize) -> Vec<C::Key> {
        let keys = (from.min(to)..=from.max(to)).filter(|&at| self.source.enabled(at) && !self.source.is_section(at)).map(|at| self.source.key(at));
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

    /// A press or key that selects row `at` the way the modifiers say.
    pub(super) fn pick(&mut self, comp: &C, at: usize, extend: bool, toggle: bool) {
        if self.source.section_stop(at) {
            return;
        }
        let key = self.source.key(at);
        match Self::select_mode(comp) {
            SelectMode::None => {}
            SelectMode::Single => self.change(SelectionChange::Set(vec![key])),
            SelectMode::Multi | SelectMode::Cells if (extend || !toggle) && self.selection.cells().is_some() => {
                let anchor = if extend { self.anchor_at(self.lead.at) } else { at };
                self.select_rows(comp, anchor, at);
            }
            SelectMode::Multi | SelectMode::Cells => {
                if extend {
                    let anchor = self.anchor_at(self.lead.at);
                    self.change(SelectionChange::Set(self.range(anchor, at)));
                } else if toggle {
                    self.change(SelectionChange::Toggle(key));
                } else {
                    self.change(SelectionChange::Set(vec![key]));
                }
            }
        }
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

    /// The pointer's position, in screen space, while it is over the collection.
    fn hovered_pos(cx: &Cx<'_, C>) -> Option<Pos2> {
        crate::runtime::pointer::frame(&cx.core.ctx).pos.filter(|_| cx.core.response.hovered())
    }

    /// The row under `pos` (screen space) in the rows' placement `at`: the pointer-to-row mapping of `hover_lead`,
    /// which `hover_lead`, the rest check and the drag-select step all call.
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

    /// The lead follows a moving pointer over the rows, and the selection follows the lead the way it does for the
    /// arrow keys. A pass moves the pointer when it holds a move event: the pointer entering over a row moves the lead
    /// with no movement to measure, and a modifier change under a resting pointer leaves the lead where the keys put it.
    pub(super) fn hover_lead(&mut self, comp: &C, cx: &Cx<'_, C>, layout: &C::ItemLayout, at: Placement) {
        let moved = crate::runtime::pointer::with_pass_pointer(&cx.core.ctx, |pass| {
            pass.events.iter().any(|event| matches!(event, PointerEvent::Moved { .. }))
        });
        if !moved {
            return;
        }
        let Some(row) = Self::hovered_pos(cx).and_then(|pos| self.row_under(layout, at, pos)) else { return };
        if self.lead.key.as_ref() != Some(&self.source.key(row)) {
            self.move_lead(row, LeadSource::Pointer);
            self.pick(comp, row, false, false);
        }
    }

    /// The pointer resting on the lead row for the tooltip delay reports `CollectionEvent::Rested`, once per lead
    /// change. Motion inside the row does not restart the timer, as for a tooltip.
    pub(super) fn rest_lead(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, at: Placement) {
        let lead = self.lead.key.clone();
        let over = Self::hovered_pos(cx).and_then(|pos| self.row_under(layout, at, pos));
        let held = lead.is_some() && self.lead.rested != lead && over.is_some_and(|row| lead.as_ref() == Some(&self.source.key(row)));
        let (ctx, id) = (cx.core.ctx.clone(), cx.core.id);
        if crate::contract::overlay::dwell(&ctx, &mut *cx.core.memory, id, "rest", held, crate::contract::tooltip::DELAY)
            && let Some(key) = lead
        {
            self.report_rest(key);
        }
    }

    /// While a drag session another source started runs over the rows (its payload is neither this collection's row
    /// drag nor a kind it takes), the unselected row under the pointer becomes the lead and the selection. The drag
    /// holds the pointer, so the row comes from the pointer's position, not the collection's hover.
    pub(super) fn drag_select(&mut self, comp: &C, cx: &Cx<'_, C>, layout: &C::ItemLayout, at: Placement) {
        let ctx = &cx.core.ctx;
        if !crate::runtime::drag::running(ctx)
            || crate::runtime::drag::active_of(ctx, TypeId::of::<ListDrag<C::Key>>()).is_some()
            || comp.drops().iter().any(|kind| crate::runtime::drag::active_of(ctx, kind.payload()).is_some())
        {
            return;
        }
        let Some(row) = crate::runtime::pointer::frame(ctx).pos.and_then(|pos| self.row_under(layout, at, pos)) else { return };
        if self.holds(row) {
            return;
        }
        self.move_lead(row, LeadSource::Pointer);
        self.pick(comp, row, false, false);
    }

    /// A lead whose key left the source moves to the key now in its place, else the one before it. The selection is
    /// the host's: a row that left (deleted, or hidden by a filter) is not replaced by its neighbour. The user did not
    /// move the lead, so the new lead does not rest at once.
    pub(super) fn follow_lead_source(&mut self) {
        let len = self.source.len();
        if let Some(key) = self.lead.key.clone()
            && self.index_of(&key).is_none()
        {
            self.lead.key = None;
            if len > 0 {
                self.place_lead(self.lead.at.min(len - 1), LeadSource::Keys);
            }
        } else if let Some(at) = self.lead.key.as_ref().and_then(|key| self.index_of(key)) {
            self.lead.at = at;
        }
    }
}
