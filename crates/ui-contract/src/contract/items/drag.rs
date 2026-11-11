//! Drags in a collection (17.3 step 9, 17.4): toggle-column sweeps, row drags over the drag session (8.4) and the
//! item move of a `FreeLayout`.
//!
//! A toggle sweep flips each switch live: every cell the pointer crosses applies an `Update` at once, as its own
//! open gesture. The release commits the gestures; Escape or a lost press capture cancels them, each with the value
//! from before, which restores every crossed switch.
//!
//! [`SweepApply`] is the one host-side reader of those events: it turns each into the step the host applies.

use super::ItemsHooks;
use super::marquee::MarqueeDrag;
use crate::contract::collection::{CollectionEvent, CollectionProps, ColumnKind, ListDrag, ReorderMove, ToggleColumn, ToggleEdit};
use crate::contract::reorder;
use crate::contract::{Collection, Cx, DragPreview, DropMark, DropTarget, OsFiles, Pointer as _};
use crate::runtime::env::Label;
use craft_behavior::{
    Axis, CancelReason, ColumnKey, DropOperation, EditEvent, Flow, GestureId, ItemLayout, Mixed, NavShape, Outline, PointerButton, PressCancel,
    PressPhase, ScrollPos,
};
use egui::{Pos2, Rect, Vec2};

/// A toggle sweep: the value it sets and the rows it has crossed.
#[derive(Clone)]
struct Sweep<K> {
    column: ColumnKey,
    on: bool,
    /// The modifiers held at the first press, which every flip of the sweep carries.
    mods: craft_keymap::Mods,
    crossed: Vec<usize>,
    /// The flips made so far, each a gesture that stays open until the release commits it or Escape cancels it:
    /// its id and the edit that undoes it.
    open: Vec<(GestureId, ToggleEdit<K>, ToggleEdit<K>)>,
}

/// What a host does for one toggle event, from [`SweepApply::feed`]. Each carries the cell's key, column and value.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum SweepStep<K> {
    /// Set the cell to `on` now. A sweep emits one as the pointer crosses each cell; a plain press emits one.
    Flip(ToggleEdit<K>),
    /// Alt was held at the first press of a [`SweepApply::show_only`] column: show only this cell, turning the column's
    /// other cells off. A sweep emits it once, for its first cell; the cells it crosses afterwards emit nothing. A plain
    /// column never emits it.
    ShowOnly(ToggleEdit<K>),
    /// Put the cell back to the value it had before the sweep (Escape or a lost press capture). Every flipped cell
    /// emits one.
    Restore(ToggleEdit<K>),
    /// Undo the `ShowOnly` of a cancelled Alt sweep: the cell has the value from before it.
    UndoShowOnly(ToggleEdit<K>),
}

/// The state a sweep needs between events: which gesture opened it and whether Alt was held.
#[derive(Clone, Copy, Debug)]
struct Run {
    first: GestureId,
    alt: bool,
    /// A `Commit` or `Cancel` came: the next `Begin` or `Update` opens a new sweep.
    ended: bool,
}

/// The one consumer of a toggle column's events (`CollectionEvent::Toggle`), for a host that applies a sweep live.
///
/// The host keeps one per toggle column and passes it every `Toggle` edit of the pass, in order, applying the step
/// that comes back. Each crossed cell applies as the pointer reaches it (`Flip`); Escape or a lost press capture
/// restores every crossed cell (`Restore`); and a sweep that began with Alt shows only its first cell once
/// (`ShowOnly`), where the cells it crosses afterwards apply nothing and restore nothing. The host owns the toggle
/// state, so the steps carry no state to read: a `Restore` carries the value from before.
///
/// Alt show-only is the column's choice. [`SweepApply::new`] (also `Default`) is plain: Alt is ignored, every crossed
/// cell gives `Flip` and a cancel gives `Restore` per flipped cell. [`SweepApply::show_only`] makes Alt plus the first
/// press show only that cell (`ShowOnly`, undone by `UndoShowOnly`); it never locks the other cells.
#[derive(Clone, Debug, Default)]
pub struct SweepApply {
    run: Option<Run>,
    show_only: bool,
}

impl SweepApply {
    /// A plain column: Alt is ignored.
    pub fn new() -> Self {
        Self::default()
    }

    /// A column where Alt plus the first press shows only that cell.
    pub fn show_only() -> Self {
        SweepApply { run: None, show_only: true }
    }

    /// The first gesture of the running sweep, which a host that groups a sweep's edits as one undo step uses as the
    /// id of all of them. `None` between sweeps.
    pub fn gesture(&self) -> Option<GestureId> {
        self.run.filter(|run| !run.ended).map(|run| run.first)
    }

    /// The step for `edit`, if it asks the host for one. `Commit` repeats what its `Update` already applied.
    pub fn feed<K: Clone>(&mut self, edit: &EditEvent<ToggleEdit<K>>) -> Option<SweepStep<K>> {
        match edit {
            EditEvent::Set { after, .. } => {
                Some(if self.show_only && after.mods.alt { SweepStep::ShowOnly(after.clone()) } else { SweepStep::Flip(after.clone()) })
            }
            EditEvent::Begin { id, before: Mixed::Same(before), .. } => {
                self.open(*id, before);
                None
            }
            EditEvent::Update { id, value, .. } => {
                let run = self.open(*id, value);
                match (run.alt, run.first == *id) {
                    (false, _) => Some(SweepStep::Flip(value.clone())),
                    (true, true) => Some(SweepStep::ShowOnly(value.clone())),
                    (true, false) => None,
                }
            }
            EditEvent::Commit { .. } => {
                self.end();
                None
            }
            EditEvent::Cancel { id, before: Some(Mixed::Same(before)), .. } => match self.end() {
                Some(Run { alt: true, first, .. }) => (first == *id).then(|| SweepStep::UndoShowOnly(before.clone())),
                _ => Some(SweepStep::Restore(before.clone())),
            },
            _ => None,
        }
    }

    /// The running sweep; gesture `id` opens one when none runs or the last one ended.
    fn open<K>(&mut self, id: GestureId, edit: &ToggleEdit<K>) -> Run {
        match self.run {
            Some(run) if !run.ended => run,
            _ => {
                let run = Run { first: id, alt: self.show_only && edit.mods.alt, ended: false };
                self.run = Some(run);
                run
            }
        }
    }

    /// Marks the sweep ended and returns it; the cells of one release or cancel arrive one after the other.
    fn end(&mut self) -> Option<Run> {
        let run = self.run.as_mut()?;
        run.ended = true;
        Some(*run)
    }
}

/// What a collection keeps of the drags it runs, in session memory.
#[derive(Clone)]
pub(super) struct Drags<K> {
    /// The row a row drag started on: it stays built while scrolled out of view.
    pub(super) grabbed: Option<K>,
    sweep: Option<Sweep<K>>,
    moving: Option<GestureId>,
    /// The row a drag from another collection would drop on this pass.
    pub(super) over: Option<K>,
    /// The selected row a plain press landed on: a click on it narrows the selection to it, a drag keeps it.
    pub(super) collapse: Option<K>,
    /// The row whose rename column got a double press: the editor opens when the press ends as a click, not when it
    /// becomes a drag.
    pub(super) rename: Option<K>,
    /// The row a primary press of an `ActivateOn::Click` collection landed on: it activates when the press ends as a
    /// click, not when it becomes a drag.
    pub(super) activate: Option<K>,
    /// The marquee a press on empty space started.
    pub(super) marquee: Option<MarqueeDrag<K>>,
}

impl<K> Default for Drags<K> {
    fn default() -> Self {
        Drags { grabbed: None, sweep: None, moving: None, over: None, collapse: None, rename: None, activate: None, marquee: None }
    }
}

/// Where a drop would land.
#[derive(Clone, Copy)]
struct Spot {
    over: usize,
    insert: usize,
    /// The group row the items land in, in a tree whose reorder rule has `into_groups`; `None` at the top level.
    parent: Option<usize>,
    mark: DropMark,
}

/// A drop the collection took.
#[derive(Clone, Copy)]
struct Landed {
    spot: Spot,
    op: DropOperation,
}

struct RowDrop {
    spot: Option<Spot>,
    /// Whether the held Alt may turn the drop into a copy.
    copy_ok: bool,
}

impl<K> DropTarget<ListDrag<K>> for RowDrop {
    type Drop = Landed;

    fn accept(&self, _: &ListDrag<K>, _: Pos2, op: DropOperation) -> Option<Landed> {
        if matches!(op, DropOperation::Copy) && !self.copy_ok {
            return None;
        }
        self.spot.map(|spot| Landed { spot, op })
    }

    fn mark(&self, drop: &Landed) -> DropMark {
        drop.spot.mark
    }
}

/// Files from the OS dropped where the rows say an internal drag would.
struct FileDrop {
    spot: Option<Spot>,
}

impl DropTarget<OsFiles> for FileDrop {
    type Drop = Spot;

    fn accept(&self, _: &OsFiles, _: Pos2, _: DropOperation) -> Option<Spot> {
        self.spot
    }

    fn mark(&self, drop: &Spot) -> DropMark {
        drop.mark
    }
}

fn cancel_reason(cancel: PressCancel) -> CancelReason {
    match cancel {
        PressCancel::Escape => CancelReason::Escape,
        PressCancel::Removed => CancelReason::Removed,
        _ => CancelReason::Blur,
    }
}

impl<C: Collection> ItemsHooks<'_, '_, C> {
    /// `at` in the layout's logical coordinates, relative to the rows' rect.
    pub(super) fn logical(layout: &C::ItemLayout, rect: Rect, rtl: bool, at: Pos2) -> Pos2 {
        let at = at - rect.min.to_vec2();
        if layout.mirrors() && rtl {
            Flow::new(Rect::from_min_size(Pos2::ZERO, rect.size()), true).to_physical(Rect::from_min_size(at, Vec2::ZERO)).min
        } else {
            at
        }
    }

    /// The toggle column under `point` (logical) in row `at`, and how it toggles.
    pub(super) fn toggle_cell(
        &self,
        props: &CollectionProps,
        layout: &C::ItemLayout,
        at: usize,
        offset: ScrollPos,
        point: Pos2,
    ) -> Option<(ColumnKey, ToggleColumn)> {
        let mut cells = Vec::new();
        layout.cells(at, offset, &mut cells);
        let (key, _) = cells.into_iter().find(|(_, cell)| cell.contains(point))?;
        props.columns.iter().find(|spec| spec.key == key).and_then(|spec| match spec.kind {
            ColumnKind::Toggle(toggle) => Some((key.clone(), toggle)),
            _ => None,
        })
    }

    fn emit_toggle(&mut self, cx: &mut Cx<'_, C>, at: usize, column: &ColumnKey, on: bool, mods: craft_keymap::Mods) {
        let key = self.source.key(at);
        let gesture = crate::runtime::edit::next_gesture_id(&cx.core.ctx);
        let before = ToggleEdit { key: key.clone(), column: column.clone(), on: !on, mods };
        let after = ToggleEdit { key, column: column.clone(), on, mods };
        if let Some(sweep) = &mut self.drags.sweep {
            // A flip of a sweep stays open until the release, so Escape can restore it.
            self.events.push(CollectionEvent::Toggle(EditEvent::begin(gesture, Mixed::Same(before.clone()), cx.core.label_arc(), None)));
            self.events.push(CollectionEvent::Toggle(EditEvent::update(gesture, after.clone())));
            sweep.open.push((gesture, before, after));
            return;
        }
        self.events.push(CollectionEvent::Toggle(EditEvent::set(Mixed::Same(before), after, cx.core.label_arc(), None)));
    }

    /// Whether the toggle cell of row `at` in `column` takes a press or a sweep: a section row has no toggles, and the
    /// source can lock a cell.
    pub(super) fn toggle_open(&self, at: usize, column: &ColumnKey) -> bool {
        !self.source.is_section(at) && self.source.toggle_enabled(at, column)
    }

    /// A press on a toggle cell flips it. On a sweepable column it also starts a sweep of the new value; on an
    /// exclusive column, turning a cell on turns the column's other cells off. A cell that is not open to toggling
    /// does nothing and starts no sweep.
    pub(super) fn begin_toggle(&mut self, cx: &mut Cx<'_, C>, at: usize, column: ColumnKey, toggle: ToggleColumn) {
        if toggle.sweepable {
            let mods = crate::runtime::pointer::frame(&cx.core.ctx).mods;
            self.drags.sweep = Some(Sweep { column: column.clone(), on: false, mods, crossed: vec![at], open: Vec::new() });
        }
        match self.flip_toggle(cx, at, column, toggle) {
            Some(on) => {
                if let Some(sweep) = &mut self.drags.sweep {
                    sweep.on = on;
                }
            }
            None => self.drags.sweep = None,
        }
    }

    /// Flips the toggle cell of row `at` in `column` and returns its new value. On an exclusive column, turning a
    /// cell on turns the column's other cells off. `None` when the cell is not open to toggling.
    pub(super) fn flip_toggle(&mut self, cx: &mut Cx<'_, C>, at: usize, column: ColumnKey, toggle: ToggleColumn) -> Option<bool> {
        if !self.toggle_open(at, &column) {
            return None;
        }
        let on = !self.source.toggled(at, &column);
        let mods = crate::runtime::pointer::frame(&cx.core.ctx).mods;
        self.emit_toggle(cx, at, &column, on, mods);
        if toggle.exclusive && on {
            for other in (0..self.source.len()).filter(|&other| other != at && self.source.toggled(other, &column)) {
                self.emit_toggle(cx, other, &column, false, mods);
            }
        }
        Some(on)
    }

    fn sweep_to(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, rect: Rect, offset: ScrollPos, rtl: bool, pos: Pos2) {
        let Some(sweep) = self.drags.sweep.clone() else { return };
        if pos.y < rect.min.y {
            return;
        }
        let Some(now) = layout.hit(Self::logical(layout, rect, rtl, pos), offset) else { return };
        let Some(&last) = sweep.crossed.last() else { return };
        let rows: Vec<usize> = if now >= last { (last + 1..=now).collect() } else { (now..last).rev().collect() };
        for at in rows {
            if let Some(held) = &mut self.drags.sweep {
                held.crossed.push(at);
            }
            if self.source.enabled(at) && self.toggle_open(at, &sweep.column) && self.source.toggled(at, &sweep.column) != sweep.on {
                self.emit_toggle(cx, at, &sweep.column, sweep.on, sweep.mods);
            }
        }
    }

    fn free_move(&mut self, cx: &mut Cx<'_, C>, origin: Pos2, pos: Pos2) {
        let gesture = match self.drags.moving {
            Some(gesture) => gesture,
            None => {
                let gesture = crate::runtime::edit::next_gesture_id(&cx.core.ctx);
                self.drags.moving = Some(gesture);
                self.events.push(CollectionEvent::Move(EditEvent::begin(gesture, Mixed::Same(Vec2::ZERO), cx.core.label_arc(), None)));
                gesture
            }
        };
        self.events.push(CollectionEvent::Move(EditEvent::update(gesture, pos - origin)));
    }

    fn begin_row_drag(&mut self, cx: &mut Cx<'_, C>, props: &CollectionProps, at: usize) {
        let id = cx.core.id;
        let key = self.source.key(at);
        let keys: Vec<C::Key> = if self.holds(at) { self.selected_keys(&self.selection) } else { vec![key.clone()] };
        let preview = if keys.len() > 1 {
            DragPreview::Badge(Label::from(keys.len().to_string()))
        } else {
            self.source.text(at).map_or(DragPreview::None, |text| DragPreview::Badge(Label::from(text.into_owned())))
        };
        let drag = ListDrag::new(id, keys, key.clone());
        match props.drag_payload.as_ref().and_then(|map| map.of(&drag)) {
            Some(also) => cx.start_drag_also(drag, also, preview),
            None if props.reorder.is_some() => cx.start_drag(drag, preview),
            None => return,
        }
        self.drags.grabbed = Some(key);
    }

    /// The one drop spot of an empty collection: its first position, marked over the whole collection.
    fn empty_spot(&self, rect: Rect, at: Pos2) -> Option<Spot> {
        rect.contains(at).then_some(Spot { over: 0, insert: 0, parent: None, mark: DropMark::Into(self.on_screen_rect(rect)) })
    }

    /// Where a drop at `at` (the collection's layer space) lands, `None` outside the rows. `groups` is the indent step
    /// of a tree whose reorder rule lets items land inside groups.
    fn spot(&self, layout: &C::ItemLayout, rect: Rect, offset: ScrollPos, rtl: bool, groups: Option<f32>, at: Pos2) -> Option<Spot> {
        if at.y < rect.min.y || self.source.is_empty() {
            return None;
        }
        let nav = layout.nav();
        let hit = layout.hit(Self::logical(layout, rect, rtl, at), offset);
        let across = !matches!(nav, NavShape::Flat | NavShape::Tree);
        let past = hit.is_none() && !across;
        let over = hit.or_else(|| past.then(|| self.source.len() - 1))?;
        let row = self.screen(layout, rect, over, offset, rtl);
        if matches!(nav, NavShape::Spatial) {
            let (low, high) = (self.on_screen(row.min), self.on_screen(row.max));
            return Some(Spot { over, insert: over, parent: None, mark: DropMark::Into(Rect::from_min_max(low, high)) });
        }
        if let Some(step) = groups {
            let gap = reorder::tree_gap_at(&Outline::new(self.source), over, row, rtl, step, at);
            let parent = gap.parent;
            return Some(Spot { over, insert: gap.insert, parent, mark: gap.mark.mapped(|point| self.on_screen(point)) });
        }
        // The gap around the one row the layout hit. `gap_at` works in physical coordinates, so across a row under
        // right-to-left the physical end is the earlier place in reading order.
        let gap = reorder::gap_at(&[row], if across { Axis::X } else { Axis::Y }, at)?;
        let after = (gap.insert == 1) != (across && rtl);
        Some(Spot { over, insert: over + usize::from(after), parent: None, mark: gap.mark.mapped(|point| self.on_screen(point)) })
    }

    /// Whether a drop inside the group row `parent` would put a dragged item inside itself.
    fn inside_dragged(&self, payload: &ListDrag<C::Key>, parent: Option<usize>) -> bool {
        let outline = Outline::new(self.source);
        let mut held = parent;
        while let Some(row) = held {
            if payload.keys.contains(&self.source.key(row)) {
                return true;
            }
            held = reorder::parent_of(&outline, row);
        }
        false
    }

    fn land(&mut self, cx: &mut Cx<'_, C>, payload: &ListDrag<C::Key>, landed: Landed, groups: bool) {
        if payload.list != cx.core.id {
            let key = (!self.source.is_empty()).then(|| self.source.key(landed.spot.over));
            self.events.push(CollectionEvent::Dropped { key, insert: landed.spot.insert, op: landed.op, keys: payload.keys.clone() });
            return;
        }
        // The moved items in display order, as they stand in the source now.
        let at: Vec<usize> =
            payload.keys.iter().filter_map(|key| self.index_of(key)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let Some(&from) = at.first() else { return };
        let insert = landed.spot.insert;
        let to = reorder::move_all_to(&at, insert);
        let copy = matches!(landed.op, DropOperation::Copy);
        let together = at.iter().enumerate().all(|(n, &place)| place == from + n);
        let parent = landed.spot.parent.map(|row| self.source.key(row));
        let held = if groups { reorder::parent_of(&Outline::new(self.source), from).map(|row| self.source.key(row)) } else { None };
        if to == from && together && !copy && parent == held {
            return;
        }
        let keys: Vec<C::Key> = at.iter().map(|&place| self.source.key(place)).collect();
        let key = payload.grabbed.clone();
        let before = ReorderMove::new(key.clone(), keys.clone(), from, false).in_group(held);
        self.events.push(CollectionEvent::Reorder(EditEvent::set(
            Mixed::Same(before),
            ReorderMove::new(key, keys, to, copy).in_group(parent),
            cx.core.label_arc(),
            None,
        )));
    }

    /// Step 9: sweeps, row drags, drops and the free-layout move, from the collection's own press and the drag session.
    pub(super) fn drag_step(
        &mut self,
        comp: &C,
        cx: &mut Cx<'_, C>,
        layout: &C::ItemLayout,
        (rect, offset, rtl): (Rect, ScrollPos, bool),
        props: &CollectionProps,
    ) {
        self.drags.over = None;
        let free = matches!(layout.nav(), NavShape::Spatial);
        let phase = cx.core.press.phase;
        // The pointer comes in screen space; the rows and a move's offset are in the rows' own space.
        let xf = self.xf;
        let layer = |point: Pos2| xf.map_or(point, |map| map.to_layer(point));
        match phase {
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary => {
                let origin = layer(start.origin);
                let hit = (origin.y >= rect.min.y)
                    .then(|| layout.hit(Self::logical(layout, rect, rtl, origin), offset))
                    .flatten()
                    .filter(|&at| self.source.enabled(at) && !self.source.section_stop(at) && comp.can_drag(&self.source.key(at)));
                if self.drags.sweep.is_some() {
                    self.sweep_to(cx, layout, rect, offset, rtl, layer(pos));
                } else if let Some(at) = hit {
                    if free {
                        self.free_move(cx, origin, layer(pos));
                    } else if (props.reorder.is_some() || props.drag_payload.is_some()) && self.drags.grabbed.is_none() {
                        self.begin_row_drag(cx, props, at);
                    }
                }
            }
            PressPhase::Release { start, pos, .. } => {
                if let Some(gesture) = self.drags.moving.take() {
                    self.events.push(CollectionEvent::Move(EditEvent::commit(gesture, Mixed::Same(Vec2::ZERO), layer(pos) - layer(start.origin))));
                }
                for (gesture, before, after) in self.drags.sweep.take().map(|sweep| sweep.open).unwrap_or_default() {
                    self.events.push(CollectionEvent::Toggle(EditEvent::commit(gesture, Mixed::Same(before), after)));
                }
            }
            PressPhase::Cancel { reason, .. } => {
                if let Some(gesture) = self.drags.moving.take() {
                    self.events.push(CollectionEvent::Move(EditEvent::cancel(gesture, cancel_reason(reason), Some(Mixed::Same(Vec2::ZERO)))));
                }
                for (gesture, before, _) in self.drags.sweep.take().map(|sweep| sweep.open).unwrap_or_default() {
                    self.events.push(CollectionEvent::Toggle(EditEvent::cancel(gesture, cancel_reason(reason), Some(Mixed::Same(before)))));
                }
            }
            _ => {}
        }
        if let Some((payload, at)) = cx.dragging::<ListDrag<C::Key>>() {
            let own = payload.list == cx.core.id;
            if (own && props.reorder.is_some()) || (!own && comp.accept_from(payload.list)) {
                let groups = props.reorder.is_some_and(|rule| rule.into_groups) && matches!(layout.nav(), NavShape::Tree);
                let step = groups.then(|| cx.env.control(cx.core.resolved.size).icon);
                let spot = if self.source.is_empty() {
                    self.empty_spot(rect, layer(at)).filter(|spot| comp.accepts(&payload, None, spot.insert))
                } else {
                    self.spot(layout, rect, offset, rtl, step, layer(at))
                        .filter(|spot| comp.accepts(&payload, Some(&self.source.key(spot.over)), spot.insert))
                        .filter(|spot| !(own && groups) || !self.inside_dragged(&payload, spot.parent))
                };
                if let Some(spot) = spot
                    && !own
                    && !self.source.is_empty()
                {
                    self.drags.over = Some(self.source.key(spot.over));
                }
                let copy_ok = !own || props.reorder.is_some_and(|rule| rule.copy_with_alt);
                if let Some(landed) = cx.drop_target::<ListDrag<C::Key>, _>(&RowDrop { spot, copy_ok }) {
                    self.land(cx, &payload, landed, groups);
                }
            }
        } else {
            self.drags.grabbed = None;
        }
        if comp.takes_files()
            && let Some((files, at)) = cx.dragging::<OsFiles>()
        {
            let at = layer(at);
            let spot = if self.source.is_empty() { self.empty_spot(rect, at) } else { self.spot(layout, rect, offset, rtl, None, at) };
            if let Some(spot) = cx.drop_target::<OsFiles, _>(&FileDrop { spot }) {
                self.events.push(CollectionEvent::FilesDropped { insert: spot.insert, files: files.files.clone() });
            }
        }
    }

    /// Alt+arrow on row `at`: the row moves one place, and with `into_groups` in a tree it enters or leaves a group.
    pub(super) fn nudge_reorder(&mut self, cx: &mut Cx<'_, C>, props: &CollectionProps, at: usize, forward: bool) {
        let groups = props.reorder.is_some_and(|rule| rule.into_groups);
        let outline = Outline::new(self.source);
        let (to, parent) = if groups {
            let Some(nudge) = reorder::nudge_in_tree(&outline, at, forward) else { return };
            (nudge.to, nudge.parent.map(|row| self.source.key(row)))
        } else {
            (reorder::nudge_to(at, self.source.len(), forward), None)
        };
        let held = if groups { reorder::parent_of(&outline, at).map(|row| self.source.key(row)) } else { None };
        if to == at && parent == held {
            return;
        }
        let key = self.source.key(at);
        let before = ReorderMove::new(key.clone(), vec![key.clone()], at, false).in_group(held);
        let after = ReorderMove::new(key.clone(), vec![key], to, false).in_group(parent);
        self.events.push(CollectionEvent::Reorder(EditEvent::set(Mixed::Same(before), after, cx.core.label_arc(), None)));
    }

    fn on_screen_rect(&self, rect: Rect) -> Rect {
        Rect::from_min_max(self.on_screen(rect.min), self.on_screen(rect.max))
    }
}
