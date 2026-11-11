//! Keyboard moves and actions at the lead: navigation, pages, sections, Space, Delete, the menu key and type-ahead.

use super::rows;
use super::{ItemsHooks, Placement, Typing};
use crate::contract::collection::{CollectionEvent, CollectionProps, ContextMenuRequest, LeadSource, OpenChange, SelectMode, SpaceAction};
use crate::contract::{Collection, Cx};
use crate::runtime::frame;
use crate::runtime::memory::sub;
use craft_behavior::{Granularity, Intent, ItemLayout, MoveMods, NavMove, NavState, NavStep, Outline, SelectionChange, TypeAhead, nav};

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// The pass's moves: lead, selection, expand and collapse.
    pub(super) fn navigate(&mut self, comp: &C, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, view: f32, header_held: bool) {
        let len = self.source.len();
        let mode = Self::select_mode(comp);
        let outline = Outline::new(self.source).columns(layout.column_count()).layout(layout);
        let cells = layout.column_count() > 0 && Self::selects_cells(comp);
        let page = self.page_rows(layout, view);
        while !header_held && let Some((mv, mods)) = cx.core.intents.take_move() {
            if len == 0 {
                break;
            }
            if self.row_child_move(cx, layout, mv, mods) {
                continue;
            }
            let lead = self.lead.key.as_ref().and_then(|key| self.index_of(key));
            if let Some(at) = lead.filter(|&at| self.source.section_stop(at))
                && self.section_move(at, mv, mods)
            {
                continue;
            }
            // A range of cells that extends moves from its head, which is not the lead (the active cell).
            let start = lead
                .map(|at| (at, self.lead.column))
                .map(|lead| if cells && mods.extend { self.cell_head(comp, layout).unwrap_or(lead) } else { lead });
            let mut state = NavState::new().page(page).column(start.map_or(self.lead.column, |(_, column)| column));
            state.lead = start.map(|(at, _)| at);
            state.anchor = lead.map(|at| self.anchor_at(at)).filter(|_| mods.extend);
            let step = match nav::step(layout.nav(), &outline, &mut state, mv, mods) {
                // The ends of the outline, and the first move into it, land on an item, not on a section stop.
                NavStep::Moved(to) if self.source.section_stop(to) && (lead.is_none() || matches!(mv, NavMove::Start | NavMove::End)) => {
                    let forward = !matches!(mv, NavMove::End | NavMove::Back | NavMove::Up | NavMove::PageUp);
                    NavStep::Moved(self.off_section(layout, to, forward))
                }
                step => step,
            };
            match step {
                NavStep::Moved(to) if cells => {
                    let reveal = self.cell_move(comp, layout, mv, to, state.column, mods);
                    self.reveal_with(layout, reveal, view);
                }
                NavStep::Column(column) if cells => {
                    if let Some((at, _)) = start {
                        let reveal = self.cell_move(comp, layout, mv, at, column, mods);
                        self.reveal_with(layout, reveal, view);
                    }
                }
                // An arrow at the edge of the grid still runs the rules: it collapses the selection to the active cell.
                NavStep::Stay if cells && !mods.lead && rows::cell_direction(mv, mods).is_some() => {
                    if let Some((at, column)) = start {
                        self.cell_move(comp, layout, mv, at, column, mods);
                    }
                }
                NavStep::Column(column) => self.lead.column = column,
                NavStep::Moved(to) => {
                    self.move_lead(to, LeadSource::Keys);
                    if mods.extend && matches!(mode, SelectMode::Multi | SelectMode::Cells) {
                        let anchor = state.anchor.unwrap_or(to);
                        self.change(SelectionChange::Set(self.range(anchor, to)));
                    } else if !mods.lead && !mods.extend && comp.items().arrows_select {
                        self.pick(comp, to, false, false);
                    }
                    self.reveal_with(layout, to, view);
                }
                NavStep::Expand(at) => self.events.push(CollectionEvent::Open(OpenChange::new(self.source.key(at), true))),
                NavStep::Collapse(at) => self.events.push(CollectionEvent::Open(OpenChange::new(self.source.key(at), false))),
                _ => {}
            }
        }
    }

    /// Ctrl+Tab and its platform equivalents at a collection of pages: the lead and the selection move to the next
    /// (or previous) item that can be taken, wrapping at the ends.
    pub(super) fn switch_page(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, layout: &C::ItemLayout, view: f32) {
        if !props.pages {
            return;
        }
        let forward = if cx.core.intents.take_exact(&Intent::switch_page(true)) {
            true
        } else if cx.core.intents.take_exact(&Intent::switch_page(false)) {
            false
        } else {
            return;
        };
        let len = self.source.len();
        let Some(from) = self.lead.key.as_ref().and_then(|key| self.index_of(key)) else { return };
        let takes = |row: &usize| self.source.enabled(*row) && !self.source.is_section(*row) && layout.shown(*row);
        let to = (1..len).map(|step| if forward { (from + step) % len } else { (from + len - step) % len }).find(takes);
        if let Some(to) = to {
            self.move_lead(to, LeadSource::Keys);
            self.pick(comp, to, false, false);
            self.reveal_with(layout, to, view);
        }
    }

    /// The nearest row from `at` in the direction that is an item the lead can take, `at` when there is none.
    fn off_section(&self, layout: &C::ItemLayout, at: usize, forward: bool) -> usize {
        let item = |row: &usize| self.source.enabled(*row) && !self.source.is_section(*row) && layout.shown(*row);
        let found = if forward { (at..self.source.len()).find(item) } else { (0..=at).rev().find(item) };
        found.unwrap_or(at)
    }

    /// Forward and Back on the lead section `at` open and close it when it has children, the tree rule; whether or not
    /// they do, a plain Forward or Back is the section's and moves nothing else. `true` when the move was taken.
    fn section_move(&mut self, at: usize, mv: NavMove, mods: MoveMods) -> bool {
        let wanted = match mv {
            NavMove::Forward => true,
            NavMove::Back => false,
            _ => return false,
        };
        if mods != MoveMods::PLAIN {
            return false;
        }
        if self.source.has_children(at) && self.source.open(at) != wanted {
            self.events.push(CollectionEvent::Open(OpenChange::new(self.source.key(at), wanted)));
        }
        true
    }

    /// Space and activation at the lead.
    pub(super) fn intent_actions(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, lead_at: Option<usize>, header_held: bool) {
        if !header_held
            && cx.core.intents.take(Intent::Space)
            && let Some(row) = lead_at
        {
            self.space_key(comp, cx, props, row);
        }
        // The pass's `update_inputs` turned an `Activate` intent into the keyed activation.
        let activated = cx.core.intents.take(Intent::Activate) || cx.core.activation.is_some_and(|activation| activation.by_key);
        if activated
            && !header_held
            && let Some(row) = lead_at
        {
            self.events.push(CollectionEvent::Activate(self.source.key(row)));
        }
    }

    /// The context-menu key at the lead: the event's request is at the row's bottom start corner, and the context
    /// request a `Trigger::Context` opener reads (9.2 rule 3) is the row's rect. With no lead the intent stays
    /// untaken, so the collection's own context request covers it, as a secondary release on its empty area does.
    pub(super) fn context_key(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, at: Placement, lead_at: Option<usize>) {
        if let Some(row) = lead_at
            && cx.core.intents.take(Intent::ContextMenu)
        {
            let rect = self.screen(layout, at.rect, row, at.offset, at.rtl);
            crate::runtime::keys::request_context(&cx.core.ctx, cx.core.id, rect);
            let place = self.on_screen(rect.left_bottom());
            self.events.push(CollectionEvent::ContextMenu(ContextMenuRequest { key: self.source.key(row), at: place, column: None }));
        }
    }

    /// Delete and Backspace at the lead of a `deletable` collection: the request names the selected items when the lead
    /// is one of them, else the lead alone. The keys stay untaken (shortcut candidates) in a collection that is not
    /// `deletable`.
    pub(super) fn delete_key(&mut self, cx: &mut Cx<'_, C>, props: &CollectionProps, lead_at: Option<usize>) {
        if !props.deletable {
            return;
        }
        let forward = cx.core.intents.take_exact(&Intent::delete(true, Granularity::Unit));
        let back = cx.core.intents.take_exact(&Intent::delete(false, Granularity::Unit));
        let Some(row) = lead_at.filter(|_| forward || back) else { return };
        let lead = self.source.key(row);
        let keys = if self.holds(row) { self.selected_keys(&self.selection) } else { vec![lead] };
        self.events.push(CollectionEvent::Delete(keys));
    }

    /// Space on the lead row `row`, as `CollectionProps.space` says.
    fn space_key(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, row: usize) {
        match props.space {
            SpaceAction::Activate => match Self::select_mode(comp) {
                SelectMode::None => self.events.push(CollectionEvent::Activate(self.source.key(row))),
                mode => self.pick(comp, row, false, mode != SelectMode::Single),
            },
            SpaceAction::ToggleCell => self.toggle_by_key(cx, row),
            SpaceAction::None => {}
        }
    }

    /// Flips the first enabled toggle cell of row `row`; when the row is selected, the same cell of every selected row
    /// follows to the value the row took, as a sweep would.
    fn toggle_by_key(&mut self, cx: &mut Cx<'_, C>, row: usize) {
        let Some((column, toggle)) = self.header.toggle_columns().find(|(column, _)| self.source.toggle_enabled(row, column)) else { return };
        let selected = self.holds(row);
        let others: Vec<usize> =
            if selected && !toggle.exclusive { self.placed().rows().into_iter().filter(|&at| at != row).collect() } else { Vec::new() };
        let Some(on) = self.flip_toggle(cx, row, column.clone(), toggle) else { return };
        for at in others {
            if self.source.enabled(at)
                && !self.source.is_section(at)
                && self.source.toggle_enabled(at, &column)
                && self.source.toggled(at, &column) != on
            {
                self.flip_toggle(cx, at, column.clone(), toggle);
            }
        }
    }

    /// Alt+arrow moves the lead's row; the rename command starts an edit of its name.
    pub(super) fn reorder_and_rename(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, lead_at: Option<usize>) {
        if let Some(forward) = crate::contract::reorder::take_move_item(&mut cx.core.intents)
            && props.reorder.is_some()
            && let Some(at) = lead_at
            && !self.source.section_stop(at)
            && comp.can_drag(&self.source.key(at))
        {
            self.nudge_reorder(cx, props, at, forward);
        }
        if props.rename
            && cx.core.intents.take(Intent::Rename)
            && let Some(at) = lead_at
            && !self.source.section_stop(at)
            && comp.can_rename(&self.source.key(at))
        {
            self.begin_rename(cx, at);
        }
    }

    /// Typed text moves the lead to the next row whose label starts with it.
    pub(super) fn type_ahead(&mut self, comp: &C, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, view: f32, lead_at: Option<usize>) {
        let (ctx, id, len) = (cx.core.ctx.clone(), cx.core.id, self.source.len());
        while let Some(text) = cx.core.intents.take_text() {
            let now = frame::clock(&ctx);
            let mut typing = cx.core.memory.session::<Typing>(sub(id, "typing")).0.clone();
            let mut prefix = String::new();
            for letter in text.chars() {
                prefix = typing.push(letter, now).to_owned();
            }
            cx.core.memory.session::<Typing>(sub(id, "typing")).0 = typing;
            let source = self.source;
            if let Some(to) = TypeAhead::pick(&prefix, lead_at, len, |at| source.text(at).unwrap_or_default()) {
                self.move_lead(to, LeadSource::Keys);
                self.pick(comp, to, false, false);
                self.reveal_with(layout, to, view);
            }
        }
    }
}
