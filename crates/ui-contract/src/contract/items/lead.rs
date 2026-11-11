//! The lead and the selection: picks, ranges, and where the lead starts and follows.

use super::{ItemsHooks, Placement};
use crate::contract::collection::{CollectionEvent, LeadOnOpen, LeadSource, SelectMode};
use crate::contract::{Collection, Cx};
use crate::runtime::env::ArenaId;
use craft_behavior::{Flow, ItemLayout, SelectionChange, SelectionModel};
use egui::{Pos2, Rect, Vec2};

impl<'s, 'r, C: Collection> ItemsHooks<'s, 'r, C> {
    pub(super) fn select_mode(comp: &C) -> SelectMode {
        comp.items().select
    }

    /// Reports and applies a selection change.
    pub(super) fn change(&mut self, change: SelectionChange<C::Key>) {
        self.selection.apply(change.clone());
        self.events.push(CollectionEvent::Selection(change));
    }

    pub(super) fn range(&self, from: usize, to: usize) -> Vec<C::Key> {
        let keys = (from.min(to)..=from.max(to)).filter(|&at| self.source.enabled(at) && !self.source.is_section(at)).map(|at| self.source.key(at));
        if from <= to { keys.collect() } else { keys.rev().collect() }
    }

    pub(super) fn move_lead(&mut self, to: usize, by: LeadSource) {
        let key = self.source.key(to);
        self.lead.key = Some(key.clone());
        self.lead.at = to;
        self.events.push(CollectionEvent::LeadMoved { key, by });
    }

    /// The position of `key`: the lead's last position is tried first, so the common lookups of a pass do not scan.
    pub(super) fn index_of(&self, key: &C::Key) -> Option<usize> {
        let hint = self.lead.at;
        if hint < self.source.len() && self.source.key(hint) == *key { Some(hint) } else { self.source.index_of(key) }
    }

    pub(super) fn anchor_at(&self, lead: usize) -> usize {
        self.selection.anchor().and_then(|key| self.index_of(key)).unwrap_or(lead)
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

    /// The lead follows a moving pointer over the rows, and the selection follows the lead the way it does for the
    /// arrow keys.
    pub(super) fn hover_lead(&mut self, comp: &C, cx: &Cx<'_, C>, layout: &C::ItemLayout, at: Placement) {
        let Placement { rect, offset, rtl } = at;
        let frame = crate::runtime::pointer::frame(&cx.core.ctx);
        let hovered = cx.core.response.hovered() || crate::primitives::sensing::read_outcome(&cx.core.ctx, ArenaId::new(cx.core.id)).hover.is_some();
        let Some(pos) = frame.pos.filter(|_| hovered && frame.delta != Vec2::ZERO) else { return };
        let pos = self.xf.map_or(pos, |map| map.to_layer(pos));
        if !rect.contains(pos) {
            return;
        }
        let local = pos - rect.min.to_vec2();
        let local = if layout.mirrors() && rtl {
            Flow::new(Rect::from_min_size(Pos2::ZERO, rect.size()), rtl).to_physical(Rect::from_min_size(local, Vec2::ZERO)).min
        } else {
            local
        };
        if let Some(row) = layout.hit(local, offset).filter(|&row| self.source.enabled(row) && !self.source.is_section(row) && layout.shown(row))
            && self.lead.key.as_ref() != Some(&self.source.key(row))
        {
            self.move_lead(row, LeadSource::Pointer);
            self.pick(comp, row, false, false);
        }
    }

    /// A lead whose key left the source moves to the key now in its place, else the one before it.
    pub(super) fn follow_lead_source(&mut self, comp: &C) {
        let len = self.source.len();
        if let Some(key) = self.lead.key.clone()
            && self.index_of(&key).is_none()
        {
            self.lead.key = None;
            if len > 0 {
                let to = self.lead.at.min(len - 1);
                self.move_lead(to, LeadSource::Keys);
                if self.holds_key(&key, to) {
                    self.pick(comp, to, false, false);
                }
            }
        } else if let Some(at) = self.lead.key.as_ref().and_then(|key| self.index_of(key)) {
            self.lead.at = at;
        }
    }
}
