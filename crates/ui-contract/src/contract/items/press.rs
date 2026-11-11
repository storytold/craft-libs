//! Presses on rows: select, activate, ask for a row's menu, and the parts an item opens menus from.

use super::{ItemsHooks, Placement, RowPress, SECTION};
use crate::contract::collection::{ActivateOn, CollectionEvent, CollectionProps, ContextMenuRequest, LeadSource, SelectMode};
use crate::contract::{Collection, Cx, ItemOpener};
use crate::runtime::egui::response;
use crate::runtime::env::ArenaId;
use crate::runtime::focus;
use crate::runtime::memory::sub;
use craft_behavior::{Flow, ItemLayout, PressPhase, SelectionChange, Sense};
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// What a press on an enabled row does besides selecting it: it activates the row at once, or on release, or opens
    /// its name for editing.
    fn press_activation(&mut self, comp: &C, props: &CollectionProps, layout: &C::ItemLayout, key: C::Key, at: usize, press: RowPress) {
        let RowPress { primary, .. } = press;
        // A section stop activates on the first primary press whatever `activate` says: the press is its toggle.
        let activates = match props.activate {
            _ if self.source.section_stop(at) => primary && press.clicks == 1,
            ActivateOn::DoubleClick => press.clicks == 2,
            ActivateOn::Click => false,
        };
        let on_release = primary && !self.source.section_stop(at) && matches!(props.activate, ActivateOn::Click);
        let names = primary
            && press.clicks == 2
            && props.rename
            && props.rename_column.is_some()
            && layout.column_at(press.origin, press.offset).and_then(|column| self.header.key_at(column)) == props.rename_column.as_ref()
            && comp.can_rename(&key);
        if names && !self.source.section_stop(at) {
            self.drags.rename = Some(key);
        } else if activates {
            self.activation_clicks = press.clicks.min(u8::MAX as u32) as u8;
            self.events.push(CollectionEvent::Activate(key));
        } else if on_release {
            self.drags.activate = Some(key);
        }
    }

    /// A press on a row selects it; a double press activates it; a right press asks for its menu.
    pub(super) fn press_step(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, layout: &C::ItemLayout, at: Placement) {
        let Placement { rect, offset, rtl } = at;
        let ctx = cx.core.ctx.clone();
        let flow = Flow::new(Rect::from_min_size(Pos2::ZERO, rect.size()), rtl);
        let xf = self.xf;
        // A screen point in the layout's logical coordinates, relative to the rows' rect.
        let local = |at: Pos2| {
            let at = xf.map_or(at, |map| map.to_layer(at)) - rect.min.to_vec2();
            if layout.mirrors() && rtl { flow.to_physical(Rect::from_min_size(at, Vec2::ZERO)).min } else { at }
        };
        let mods_now = crate::runtime::pointer::frame(&ctx).mods;
        if let PressPhase::Down { start, .. } = cx.core.press.phase
            && local(start.origin).y >= 0.0
        {
            let primary = start.button == craft_behavior::PointerButton::Primary;
            if let Some(at) = layout.hit(local(start.origin), offset).filter(|&at| self.source.enabled(at) && layout.shown(at)) {
                let on_toggle = self.toggle_cell(props, layout, at, offset, local(start.origin));
                // A secondary press on a toggle cell asks for the cell's menu and neither toggles nor picks.
                let secondary_on_toggle = !primary && on_toggle.is_some();
                let toggle = on_toggle.filter(|_| primary);
                if let Some((column, toggle)) = toggle {
                    self.begin_toggle(cx, at, column, toggle);
                } else if !secondary_on_toggle {
                    let mods = crate::runtime::pointer::frame(&ctx).mods;
                    let key = self.source.key(at);
                    let column = if Self::selects_cells(comp) { layout.column_at(local(start.origin), offset) } else { None };
                    // A plain press on a selected row of a multiple selection leaves the selection whole, so a drag
                    // carries all of it; the release as a click narrows it to the row. In a single selection that
                    // allows an empty one, the release as a click empties it.
                    let holds = match Self::select_mode(comp) {
                        SelectMode::Multi => true,
                        SelectMode::Single => props.allow_empty,
                        _ => false,
                    };
                    let hold = primary && !mods.shift && !mods.cmd && holds && self.holds(at);
                    // A secondary press on a row of the selection leaves the selection whole: its menu acts on all of it.
                    let keep = !primary && matches!(Self::select_mode(comp), SelectMode::Multi) && self.holds(at);
                    match column {
                        Some(column) => self.cell_press(comp, layout, at, column, mods.shift, mods.cmd),
                        None if hold => self.drags.collapse = Some(key.clone()),
                        None if keep => {}
                        None => self.pick(comp, at, mods.shift, mods.cmd),
                    }
                    self.move_lead(at, LeadSource::Pointer);
                    // A secondary press takes focus the way a primary press does: egui and the press classification only move it for the primary button.
                    if !primary && props.focus_on_press {
                        focus::request_now(&ctx, focus::FocusTarget::Id(cx.core.id));
                    }
                    self.press_activation(
                        comp,
                        props,
                        layout,
                        key,
                        at,
                        RowPress { primary, clicks: start.clicks, origin: local(start.origin), offset },
                    );
                }
            } else if primary
                && props.allow_empty
                && !(props.marquee && (mods_now.shift || mods_now.cmd || mods_now.alt))
                && !matches!(Self::select_mode(comp), SelectMode::None)
                && self.selection != C::Selection::default()
            {
                self.change(SelectionChange::Clear);
            }
        }
        // A press that a part of an item won leaves the selection alone and moves the lead to its row. A press on the
        // part of a section row (`section_part`) is the section's own, so the lead goes to it as well. Key focus goes
        // to the collection's stop with the lead, as a press on the row's own area takes it, unless the part takes
        // focus itself or keeps it where it is.
        let outcome = crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id));
        if let (Some(PressPhase::Down { start, .. }), Some(target)) = (outcome.phase, outcome.target)
            && target.part.is_some()
            && local(start.origin).y >= 0.0
            && let Some(at) = layout.hit(local(start.origin), offset).filter(|&at| self.source.enabled(at) && layout.shown(at))
            && (target.owner != cx.core.id || target.part.is_some_and(|part| part.key == SECTION))
        {
            self.move_lead(at, LeadSource::Pointer);
            if props.focus_on_press && !target.sense.intersects(Sense::TAKE_FOCUS | Sense::KEEP_FOCUS) {
                focus::request_now(&ctx, focus::FocusTarget::Id(cx.core.id));
            }
        }
        // A click on a row's thumbnail is also a click on the row: the item reports the thumbnail's click and an unmodified click picks
        // in the same pass.
        if let (Some(PressPhase::Release { click: true, start, .. }), Some(target)) = (outcome.phase, outcome.target)
            && start.button == craft_behavior::PointerButton::Primary
            && target.part.is_some()
            && target.sense.contains(Sense::PICKS_ITEM)
            && local(start.origin).y >= 0.0
            && let Some(at) = layout.hit(local(start.origin), offset).filter(|&at| self.source.enabled(at) && layout.shown(at))
        {
            // A modified click is a command on the thumbnail (load as selection, and so on), not a pick.
            let mods = crate::runtime::pointer::frame(&ctx).mods;
            if !(mods.shift || mods.cmd || mods.alt) {
                self.pick(comp, at, false, false);
            }
        }
        match cx.core.press.phase {
            PressPhase::Release { click, start, pos, .. } => {
                if click
                    && props.menu_on_click
                    && start.button == craft_behavior::PointerButton::Primary
                    && crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id))
                        .hover
                        .is_none_or(|hit| hit.part.is_none() || !hit.sense.takes_press())
                    && let Some(row) = layout.hit(local(pos), offset).filter(|&row| self.source.enabled(row) && layout.shown(row))
                {
                    let request = self.menu_request(layout, Placement { rect, offset, rtl }, row, local(pos));
                    self.events.push(CollectionEvent::ContextMenu(request));
                }
                if let Some(key) = self.drags.rename.take()
                    && click
                    && let Some(at) = self.index_of(&key)
                {
                    self.begin_rename(cx, at);
                }
                if let Some(key) = self.drags.activate.take()
                    && click
                    && self.index_of(&key).is_some()
                {
                    self.activation_clicks = start.clicks.min(u8::MAX as u32) as u8;
                    self.events.push(CollectionEvent::Activate(key));
                }
                if let Some(key) = self.drags.collapse.take()
                    && click
                    && let Some(at) = self.index_of(&key)
                {
                    if Self::select_mode(comp) == SelectMode::Single {
                        self.change(SelectionChange::Clear);
                    } else {
                        self.pick(comp, at, false, false);
                    }
                }
            }
            PressPhase::Cancel { .. } => {
                self.drags.collapse = None;
                self.drags.rename = None;
                self.drags.activate = None;
            }
            _ => {}
        }
        // A secondary release on a row asks for its menu (9.2 rule 3); the row joins the selection unless it is in it.
        // Something over the collection, or a part of a row or of the collection that takes presses and is under the
        // pointer, keeps the request away.
        let released = crate::runtime::pointer::frame(&ctx);
        if released.release == Some(craft_behavior::PointerButton::Secondary)
            && cx.core.response.hovered()
            && crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id))
                .hover
                .is_none_or(|hit| hit.part.is_none() || !hit.sense.takes_press())
            && let Some(pos) = released.pos.filter(|pos| rect.contains(xf.map_or(*pos, |map| map.to_layer(*pos))))
            && let Some(at) = layout.hit(local(pos), offset)
            && self.source.enabled(at)
        {
            if !self.holds(at) && self.toggle_cell(props, layout, at, offset, local(pos)).is_none() {
                let column = if Self::selects_cells(comp) { layout.column_at(local(pos), offset) } else { None };
                match column {
                    Some(column) => self.cell_press(comp, layout, at, column, false, false),
                    None => self.pick(comp, at, false, false),
                }
            }
            self.move_lead(at, LeadSource::Pointer);
            let request = self.menu_request(layout, Placement { rect, offset, rtl }, at, local(pos));
            self.events.push(CollectionEvent::ContextMenu(request));
        }
    }

    /// The request for the menu of the row `at`, opened by a pointer at `point` (the layout's coordinates): at the
    /// row's bottom start corner, with the column the point lies in.
    fn menu_request(&self, layout: &C::ItemLayout, at: Placement, row: usize, point: Pos2) -> ContextMenuRequest<C::Key> {
        let Placement { rect, offset, rtl } = at;
        let place = self.on_screen(self.screen(layout, rect, row, offset, rtl).left_bottom());
        let mut cells = Vec::new();
        layout.cells(row, offset, &mut cells);
        let column = cells.into_iter().find(|(_, cell)| cell.contains(point)).map(|(column, _)| column);
        ContextMenuRequest { key: self.source.key(row), at: place, column }
    }

    /// The response of each part an item asked to open a menu from (`ItemCx::part_opener`): the part's click this pass
    /// and its rect, the rect the press was hit-tested against.
    pub(super) fn item_openers(&self, ui: &Ui, ctx: &Context, id: Id, enabled: bool, openers: Vec<ItemOpener>) -> Vec<(ItemOpener, egui::Response)> {
        if openers.is_empty() {
            return Vec::new();
        }
        let outcome = crate::primitives::sensing::read_outcome(ctx, ArenaId::new(id));
        openers
            .into_iter()
            .filter_map(|opener| {
                let rect = crate::runtime::pointer::part_bounds(ctx, opener.item, opener.part)?;
                let on_part =
                    |hit: Option<crate::primitives::sensing::Hit>| hit.is_some_and(|hit| hit.owner == opener.item && hit.part == Some(opener.part));
                let view = response::MemberView {
                    hovered: on_part(outcome.hover),
                    pressed: on_part(outcome.target) && outcome.phase.is_some(),
                    clicked: on_part(outcome.activated),
                    pointer: ctx.pointer_latest_pos(),
                    ..response::MemberView::default()
                };
                let response = response::member(ui, sub(opener.item, ("opener", opener.part)), rect, enabled, view);
                Some((opener, response))
            })
            .collect()
    }
}
