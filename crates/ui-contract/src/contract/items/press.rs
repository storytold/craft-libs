//! Presses on rows: select, activate, ask for a row's menu, and the parts an item opens menus from.

use super::drag::SlowClick;
use super::lead::PickBy;
use super::{ItemsHooks, Placement, RowPress, SECTION, section_part};
use crate::contract::collection::{ActivateOn, CollectionEvent, CollectionProps, ContextMenuRequest, LeadSource, OpenChange, RowPressed, SelectMode};
use crate::contract::{Collection, Cx, ItemOpener, ItemOutput};
use crate::runtime::egui::response;
use crate::runtime::env::ArenaId;
use crate::runtime::focus;
use crate::runtime::frame;
use crate::runtime::memory::sub;
use crate::runtime::motion::Animate as _;
use craft_behavior::{Flow, ItemLayout, MiddleButton, PointerButton, PressPhase, SelectionChange, Sense};
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
    #[expect(
        clippy::cognitive_complexity,
        reason = "one match over the press kinds (select, activate, menu), each arm short; splitting it only moves the match"
    )]
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
        let mods_now = crate::runtime::pointer::frame_raw(&ctx).mods;
        // A click on a row the collection does not take (a caption, a disabled row) is no pick: the layers around the
        // collection (a dropdown's list) stay open.
        if cx.core.activation.as_ref().is_some_and(|activation| !activation.by_key)
            && let Some(pos) = crate::runtime::pointer::frame_raw(&ctx).pos
            && local(pos).y >= 0.0
            && layout
                .hit(local(pos), offset)
                .is_some_and(|at| !self.source.enabled(at) || (self.source.is_section(at) && !self.source.section_stop(at)))
        {
            cx.keep_open();
        }
        self.pressed = match cx.core.press.phase {
            PressPhase::Down { start, .. } | PressPhase::Held { start, .. }
                if start.button == craft_behavior::PointerButton::Primary && local(start.origin).y >= 0.0 =>
            {
                let origin = local(start.origin);
                layout.hit(origin, offset).filter(|&at| self.source.enabled(at) && layout.shown(at)).and_then(|at| {
                    match self.toggle_cell(props, layout, at, offset, origin) {
                        Some((column, ..)) => self.toggle_open(at, &column).then_some((at, RowPressed::Cell(column))),
                        None => Some((at, RowPressed::Row)),
                    }
                })
            }
            _ => None,
        };
        // Under `AutoScroll` a middle press on the rows starts or ends the latch (the items step's `middle_scroll`) and
        // picks no row. In a collection that senses `MIDDLE_CLICK` the middle click is the row's own action
        // (`CollectionEvent::MiddleClick`: a tab strip closes the tab), so the press picks no row there either.
        let middle_clicks = cx.core.root_sense.contains(craft_behavior::Sense::MIDDLE_CLICK);
        if let PressPhase::Down { start, .. } = cx.core.press.phase
            && local(start.origin).y >= 0.0
            && !(start.button == PointerButton::Middle && (props.middle_button == MiddleButton::AutoScroll || middle_clicks))
        {
            // A second press, slow or not, ends the wait of the first.
            self.drags.slow = None;
            let primary = start.button == craft_behavior::PointerButton::Primary;
            if let Some(at) = layout.hit(local(start.origin), offset).filter(|&at| self.source.enabled(at) && layout.shown(at)) {
                let on_toggle = self.toggle_cell(props, layout, at, offset, local(start.origin));
                // A secondary press on a toggle cell asks for the cell's menu and neither toggles nor picks.
                let secondary_on_toggle = !primary && on_toggle.is_some();
                let toggle = on_toggle.filter(|_| primary);
                if let Some(cell) = toggle {
                    self.begin_toggle(cx, at, cell);
                } else if primary && props.command_click && mods_now.cmd {
                    // A command on the row, reported when the press ends as a click; the selection stays.
                    self.drags.command = Some(self.source.key(at));
                    self.move_lead(at, LeadSource::Pointer);
                } else if !secondary_on_toggle && (primary || !props.primary_picks) {
                    let mods = crate::runtime::pointer::frame_raw(&ctx).mods;
                    let key = self.source.key(at);
                    let column = if Self::selects_cells(comp) { layout.column_at(local(start.origin), offset) } else { None };
                    // A plain press on a selected row of a multiple selection leaves the selection whole, so a drag
                    // carries all of it; the release as a click narrows it to the row. In a single selection that
                    // allows an empty one, the release as a click empties it. A plain press on a row outside the
                    // selection picks it when it ends as a click, so a drag carries the row and leaves the selection
                    // as it was. A later click of a multi-click on a held row keeps the selection the earlier click
                    // left (`keeps_pick`).
                    let holds = match Self::select_mode(comp) {
                        SelectMode::Multi => true,
                        SelectMode::Single => props.allow_empty,
                        _ => false,
                    };
                    let plain = primary && !mods.shift && !mods.cmd;
                    let hold = plain && holds && self.holds(at);
                    let again = plain && self.keeps_pick(at, start.clicks);
                    let slow = primary
                        && start.clicks == 1
                        && mods == craft_keymap::Mods::NONE
                        && props.slow_click_rename
                        && props.rename
                        && !self.source.section_stop(at)
                        && comp.can_rename(&key)
                        && self.selected_keys(&self.selection) == std::slice::from_ref(&key);
                    if slow {
                        let point = xf.map_or(start.origin, |map| map.to_layer(start.origin));
                        self.drags.slow = Some(SlowClick { key: key.clone(), at: point, on_label: false, released: None });
                    }
                    // A secondary press on a row of the selection leaves the selection whole: its menu acts on all of it.
                    let keep = !primary && matches!(Self::select_mode(comp), SelectMode::Multi) && self.holds(at);
                    match column {
                        Some(column) => self.cell_press(comp, layout, at, column, mods.shift, mods.cmd),
                        None if again => {}
                        None if hold => self.drags.collapse = Some(key.clone()),
                        None if keep => {}
                        None if plain && !self.holds(at) => self.drags.pick = Some(key.clone()),
                        None if primary => self.click_pick(comp, at, mods.shift, mods.cmd, mods),
                        None => self.pick_by(comp, at, mods.shift, mods.cmd, PickBy::Pointer(mods)),
                    }
                    // A modified click on a pick kept beside the current item changes the pick only: the lead, which the
                    // host follows to the current item, stays.
                    if !(props.picks_beside_current && primary && (mods.shift || mods.cmd)) {
                        self.move_lead(at, LeadSource::Pointer);
                    }
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
            } else {
                if primary && start.clicks == 2 {
                    self.events.push(CollectionEvent::EmptyActivated);
                }
                if primary
                    && props.allow_empty
                    && !(props.marquee && (mods_now.shift || mods_now.cmd || mods_now.alt))
                    && !matches!(Self::select_mode(comp), SelectMode::None)
                    && self.selection != C::Selection::default()
                {
                    self.change_by(SelectionChange::Clear, PickBy::Pointer(mods_now));
                }
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
            let key = self.source.key(at);
            self.drags.part_press = Some((key.clone(), self.lead.key.as_ref() == Some(&key)));
            self.move_lead(at, LeadSource::Pointer);
            if props.focus_on_press && !target.sense.intersects(Sense::TAKE_FOCUS | Sense::KEEP_FOCUS) {
                focus::request_now(&ctx, focus::FocusTarget::Id(cx.core.id));
            }
        }
        // A selectable section owns its pointer surface. Pick on its click, without taking clicks on disclosure.
        if let (Some(PressPhase::Release { start, click: true, .. }), Some(target)) = (outcome.phase, outcome.target)
            && start.button == PointerButton::Primary
            && target.owner == cx.core.id
            && target.part.is_some_and(|part| part.key == SECTION)
            && let Some((key, _)) = &self.drags.part_press
            && target.part == Some(section_part(key))
            && let Some(at) = self.index_of(key)
            && self.source.selectable_section(at)
        {
            let mods = crate::runtime::pointer::frame(&ctx).pick_mods;
            self.click_pick(comp, at, mods.shift, mods.cmd, mods);
        }
        // While a part of an item holds the press, and in the pass it ends, the row reads it with whether it held the lead
        // when the press began.
        match outcome.phase {
            Some(PressPhase::Down { .. } | PressPhase::Held { .. } | PressPhase::Release { .. })
                if outcome.target.is_some_and(|target| target.part.is_some()) =>
            {
                if let Some((key, was_lead)) = self.drags.part_press.clone()
                    && let Some(at) = self.index_of(&key)
                {
                    self.pressed = Some((at, RowPressed::Part { was_lead }));
                }
            }
            _ => self.drags.part_press = None,
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
            let mods = crate::runtime::pointer::frame_raw(&ctx).mods;
            if !(mods.shift || mods.cmd || mods.alt) {
                if !self.keeps_pick(at, start.clicks) {
                    self.click_pick(comp, at, false, false, mods);
                }
            } else if props.command_click && mods.cmd {
                self.events.push(CollectionEvent::CommandClick { key: self.source.key(at), mods });
            }
        }
        match cx.core.press.phase {
            PressPhase::Release { click, start, pos, .. } => {
                if let Some(key) = self.drags.pick.take()
                    && click
                    && let Some(at) = self.index_of(&key)
                {
                    self.click_pick(comp, at, false, false, mods_now);
                }
                if click
                    && props.menu_on_click
                    && start.button == craft_behavior::PointerButton::Primary
                    && crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id))
                        .hover
                        .find(|hit| hit.part.is_some() && hit.sense.takes_press())
                        .is_none()
                    && let Some(row) = layout.hit(local(pos), offset).filter(|&row| self.source.enabled(row) && layout.shown(row))
                {
                    let request = self.menu_request(layout, Placement { rect, offset, rtl }, row, local(pos));
                    self.ask_menu(request);
                }
                match self.drags.slow.as_mut() {
                    Some(slow) if click && slow.on_label => slow.released = Some(frame::clock(&ctx)),
                    _ => self.drags.slow = None,
                }
                if let Some(key) = self.drags.rename.take()
                    && click
                    && let Some(at) = self.index_of(&key)
                {
                    self.begin_rename(comp, cx, at);
                }
                if let Some(key) = self.drags.command.take()
                    && click
                    && self.index_of(&key).is_some()
                {
                    self.events.push(CollectionEvent::CommandClick { key, mods: mods_now });
                }
                // The press machine makes a middle release a click only for a collection that senses `MIDDLE_CLICK`.
                if click
                    && start.button == PointerButton::Middle
                    && props.middle_button != MiddleButton::AutoScroll
                    && local(start.origin).y >= 0.0
                    && let Some(at) = layout.hit(local(start.origin), offset).filter(|&at| self.source.enabled(at) && layout.shown(at))
                {
                    self.events.push(CollectionEvent::MiddleClick(self.source.key(at)));
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
                        self.change_by(SelectionChange::Clear, PickBy::Pointer(mods_now));
                    } else {
                        self.click_pick(comp, at, false, false, mods_now);
                    }
                }
            }
            PressPhase::DragStart { .. } => self.drags.slow = None,
            PressPhase::Cancel { .. } => {
                self.drags.slow = None;
                self.drags.collapse = None;
                self.drags.pick = None;
                self.drags.rename = None;
                self.drags.activate = None;
                self.drags.command = None;
            }
            _ => {}
        }
        // A secondary release on a row asks for its menu (9.2 rule 3); the row joins the selection unless it is in it.
        // Something over the collection, or a part of a row or of the collection that takes the secondary click itself
        // (its own menu, L-198) and is under the pointer, keeps the request away; a part that picks its item (a thumbnail) is the row's, and the request names
        // it.
        let released = crate::runtime::pointer::frame_raw(&ctx);
        let hover = crate::primitives::sensing::read_outcome(&ctx, ArenaId::new(cx.core.id)).hover;
        let menu_at = if released.release == Some(craft_behavior::PointerButton::Secondary)
            && cx.core.response.hovered()
            && hover.find(|hit| hit.part.is_some() && hit.sense.contains(Sense::CONTEXT_CLICK) && !hit.sense.contains(Sense::PICKS_ITEM)).is_none()
        {
            released.pos.filter(|pos| rect.contains(xf.map_or(*pos, |map| map.to_layer(*pos))))
        } else {
            None
        };
        let menu_row = menu_at.map(|pos| (pos, layout.hit(local(pos), offset)));
        // A secondary release on the empty space asks for the collection's own menu, which no item's menu is.
        if let Some((pos, None)) = menu_row {
            self.drags.menu = None;
            self.events.push(CollectionEvent::EmptyContextMenu { at: pos });
        }
        if let Some((pos, Some(at))) = menu_row
            && self.source.enabled(at)
        {
            if !props.primary_picks && !self.holds(at) && self.toggle_cell(props, layout, at, offset, local(pos)).is_none() {
                let column = if Self::selects_cells(comp) { layout.column_at(local(pos), offset) } else { None };
                match column {
                    Some(column) => self.cell_press(comp, layout, at, column, false, false),
                    None => self.pick_by(comp, at, false, false, PickBy::Pointer(released.mods)),
                }
            }
            self.move_lead(at, LeadSource::Pointer);
            let mut request = self.menu_request(layout, Placement { rect, offset, rtl }, at, local(pos));
            request.part = hover.find(|hit| hit.owner != cx.core.id).and_then(|hit| hit.part);
            self.ask_menu(request);
        }
        self.slow_click_due(comp, cx);
    }

    /// The rename a slow click waits for opens once the double-click interval has passed with no second press and the
    /// item still the only one selected; until then the pass asks to run again at that time.
    fn slow_click_due(&mut self, comp: &C, cx: &mut Cx<'_, C>) {
        let Some((key, released)) = self.drags.slow.as_ref().and_then(|slow| Some((slow.key.clone(), slow.released?))) else { return };
        let at = self.index_of(&key).filter(|_| self.selected_keys(&self.selection) == std::slice::from_ref(&key));
        let Some(at) = at else {
            self.drags.slow = None;
            return;
        };
        let due = released + craft_behavior::press::MULTI_CLICK_TIME;
        if cx.now() >= due {
            self.drags.slow = None;
            self.begin_rename(comp, cx, at);
        } else {
            cx.wake_at(due);
        }
    }

    /// Where the item `key` shows its label this pass: a slow click waiting on it counts only when it landed there.
    pub(super) fn slow_click_label(&mut self, key: &C::Key, label: Rect) {
        if let Some(slow) = self.drags.slow.as_mut().filter(|slow| slow.key == *key && slow.released.is_none()) {
            slow.on_label = label.contains(slow.at);
        }
    }

    /// The request for the menu of the row `at`, opened by a pointer at `point` (the layout's coordinates): at the
    /// row's bottom start corner, with the column the point lies in.
    fn menu_request(&self, layout: &C::ItemLayout, at: Placement, row: usize, point: Pos2) -> ContextMenuRequest<C::Key> {
        let Placement { rect, offset, rtl } = at;
        let place = self.on_screen(self.screen(layout, rect, row, offset, rtl).left_bottom());
        let column = self.row_cells(layout, row, offset).into_iter().find(|(_, cell)| cell.contains(point)).map(|(column, _)| column);
        ContextMenuRequest { key: self.source.key(row), at: place, column, part: None }
    }

    /// Reports a request for the menu of a row; the row keeps its menu button shown while a menu hung from the
    /// collection is open (`RowState::menu_open`).
    pub(super) fn ask_menu(&mut self, request: ContextMenuRequest<C::Key>) {
        self.drags.menu = Some(request.clone());
        self.events.push(CollectionEvent::ContextMenu(request));
    }

    /// Reports what the shown item `key` asked for: opening or closing its children, its menu, and its event.
    pub(super) fn report_item_output(&mut self, ctx: &Context, id: Id, key: &C::Key, output: impl ItemOutput<C::ItemEvent>) {
        if let Some(ask) = output.open_request() {
            self.events.push(CollectionEvent::Open(OpenChange { key: key.clone(), open: ask.open, all: ask.all }));
        }
        if let Some(rect) = output.menu_request() {
            crate::runtime::keys::request_context(ctx, id, rect, None);
            self.ask_menu(ContextMenuRequest { key: key.clone(), at: self.on_screen(rect.left_bottom()), column: None, part: None });
        }
        if let Some(event) = output.into_event() {
            self.events.push(CollectionEvent::Item(key.clone(), event));
        }
    }

    /// The response of each part an item asked to open a menu from (`ItemCx::part_opener`).
    pub(super) fn item_openers(&self, ui: &Ui, ctx: &Context, id: Id, enabled: bool, openers: Vec<ItemOpener>) -> Vec<(ItemOpener, egui::Response)> {
        item_openers(ui, ctx, id, enabled, openers)
    }
}

/// Resolves the item opener responses without duplicating the sensing and response code per collection type.
fn item_openers(ui: &Ui, ctx: &Context, id: Id, enabled: bool, openers: Vec<ItemOpener>) -> Vec<(ItemOpener, egui::Response)> {
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
                hovered: outcome.hover.find(|hit| on_part(Some(*hit))).is_some(),
                press: response::PressView {
                    pressed: on_part(outcome.target) && outcome.phase.is_some_and(|phase| phase.button_down()),
                    clicked: on_part(outcome.activated),
                    ..response::PressView::default()
                },
                pointer: crate::runtime::pointer::frame_raw(ctx).pos,
            };
            let response = response::member(ui, opener_id(opener.item, opener.part), rect, enabled, view);
            Some((opener, response))
        })
        .collect()
}

/// The id of the response `CollectionResponse::opener` gives for the part `part` of the item `item`.
fn opener_id(item: Id, part: craft_behavior::PartRef) -> Id {
    sub(item, ("opener", part))
}

/// Whether a menu or popover hung from the part `part` of the item `item` (`ItemCx::part_opener`) is open: an item
/// keeps such a part shown while its menu is.
pub fn item_part_open(ctx: &Context, item: Id, part: impl Into<craft_behavior::PartRef>) -> bool {
    crate::contract::overlay::container_open::opened_from(ctx, opener_id(item, part.into()))
}
