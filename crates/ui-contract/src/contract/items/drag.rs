//! Drags in a collection (17.3 step 9, 17.4): toggle-column sweeps, row drags over the drag session (8.4) and the
//! item move of a `FreeLayout`.
//!
//! A toggle sweep flips each switch live: every cell the pointer crosses applies an `Update` at once, as its own
//! open gesture. The release commits the gestures; Escape or a lost press capture cancels them, each with the value
//! from before, which restores every crossed switch.
//!
//! [`SweepApply`] is the one host-side reader of those events: it turns each into the step the host applies.

use super::marquee::MarqueeDrag;
use super::{ItemsHooks, Placement};
use crate::contract::collection::{
    CollectionEvent, CollectionProps, ColumnKind, ContextMenuRequest, DropAt, DropPayload, DropPlace, ListDrag, PendingDrop, ReorderMove, RowPreview,
    ToggleColumn, ToggleEdit,
};
use crate::contract::reorder;
use crate::contract::{Collection, Cx, DragPreview, DropMark, DropTarget, Pointer as _};
use crate::runtime::drag;
use crate::runtime::egui::areas;
use crate::runtime::env::Label;
use crate::runtime::toggle_sweep::{self, Segment, Travel};
use craft_behavior::{
    Axis, CancelReason, ColumnKey, CopyKeys, DropOperation, EditEvent, Flow, GestureId, ItemLayout, Mixed, NavShape, Outline, PointerButton,
    PressCancel, PressPhase, ScrollPos,
};
use egui::{Pos2, Rect, Vec2};
use std::any::Any;
use std::borrow::Cow;

/// Maps a point to layout coordinates once for every collection type.
fn logical_point(layout: &dyn ItemLayout, rect: Rect, rtl: bool, at: Pos2) -> Pos2 {
    let at = at - rect.min.to_vec2();
    if layout.mirrors() && rtl {
        Flow::new(Rect::from_min_size(Pos2::ZERO, rect.size()), true).to_physical(Rect::from_min_size(at, Vec2::ZERO)).min
    } else {
        at
    }
}

/// Resolves a row's cells and sub-row indentation independently of the collection type.
fn row_cells(
    layout: &dyn ItemLayout,
    at: usize,
    offset: ScrollPos,
    sub_row: &dyn Fn() -> Option<usize>,
    header: &super::header::Header,
    indent_step: f32,
) -> Vec<(ColumnKey, Rect)> {
    let mut cells = Vec::new();
    layout.cells(at, offset, &mut cells);
    if let Some(steps) = sub_row() {
        let toggles: Vec<ColumnKey> = header.toggle_columns().map(|(key, _)| key).collect();
        craft_behavior::sub_row_cells(&mut cells, |key| toggles.contains(key), steps as f32 * indent_step, false);
    }
    cells
}

/// Finds the toggle and its geometry under a point in the row.
fn toggle_cell_at(props: &CollectionProps, cells: Vec<(ColumnKey, Rect)>, point: Pos2) -> Option<(ColumnKey, ToggleColumn, Rect)> {
    let (key, cell) = cells.into_iter().find(|(_, cell)| cell.contains(point))?;
    props.columns.iter().find(|spec| spec.key == key).and_then(|spec| match spec.kind {
        ColumnKind::Toggle(toggle) => Some((key.clone(), toggle, cell)),
        _ => None,
    })
}

/// The screen rect of a row in the layout's view.
fn screen_row(layout: &dyn ItemLayout, view: Rect, at: usize, offset: ScrollPos, rtl: bool) -> Rect {
    let local = layout.rect(at, offset);
    let local = if rtl && layout.mirrors() { Flow::new(Rect::from_min_size(Pos2::ZERO, view.size()), true).to_physical(local) } else { local };
    local.translate(view.min.to_vec2())
}

/// Resolves a row drop's geometry independently of the collection and its key type.
fn drop_spot(
    layout: &dyn ItemLayout,
    outline: &dyn craft_behavior::NavOutline,
    section_stop: &dyn Fn(usize) -> bool,
    xf: Option<crate::runtime::egui::areas::LayerMap>,
    (rect, offset, rtl): (Rect, ScrollPos, bool),
    groups: Option<f32>,
    at: Pos2,
) -> Option<Spot> {
    let to_screen = |point: Pos2| xf.map_or(point, |map| map.to_screen(point));
    let nav = layout.nav();
    let hit = layout.drop_hit(logical_point(layout, rect, rtl, at), offset);
    let across =
        !matches!(nav, NavShape::Flat | NavShape::Tree | NavShape::WrappedGrid { columns: 1, .. } | NavShape::BoundedGrid { columns: 1, .. });
    // With no item under the pointer, a point after the last item in reading order is over the last item. Across a
    // row-wise layout that is beyond the last item's end edge on its row, or below its row.
    let past = hit.is_none() && {
        let last = screen_row(layout, rect, outline.len() - 1, offset, rtl);
        let beyond = if rtl { at.x < last.min.x } else { at.x > last.max.x };
        !across || at.y > last.max.y || (at.y >= last.min.y && beyond)
    };
    let over = hit.or_else(|| past.then(|| outline.len() - 1))?;
    let row = screen_row(layout, rect, over, offset, rtl);
    if matches!(nav, NavShape::Spatial) {
        let (low, high) = (to_screen(row.min), to_screen(row.max));
        return Some(Spot { over, insert: over, parent: None, mark: DropMark::Into(Rect::from_min_max(low, high)) });
    }
    if let Some(step) = groups {
        let gap = reorder::tree_gap_at(outline, over, row, rtl, step, at);
        let parent = gap.parent;
        return Some(Spot { over, insert: gap.insert, parent, mark: gap.mark.mapped(to_screen) });
    }
    // A section row across a row-wise layout (a grid's group header) starts its group: a drop on it lands first in
    // the group.
    if across && section_stop(over) {
        return Some(Spot { over, insert: over + 1, parent: None, mark: DropMark::Into(Rect::from_min_max(to_screen(row.min), to_screen(row.max))) });
    }
    // The gap around the one row the layout hit. `gap_at` works in physical coordinates, so across a row under
    // right-to-left the physical end is the earlier place in reading order.
    let at = if past && across { Pos2::new(if rtl { row.min.x } else { row.max.x }, row.center().y) } else { at };
    let gap = reorder::gap_at(&[row], if across { Axis::X } else { Axis::Y }, at)?;
    let after = (gap.insert == 1) != (across && rtl);
    Some(Spot { over, insert: over + usize::from(after), parent: None, mark: gap.mark.mapped(to_screen) })
}

/// A toggle sweep: the value it sets and the rows it has crossed.
#[derive(Clone)]
struct Sweep<K> {
    column: ColumnKey,
    on: bool,
    /// The modifiers held at the first press, which every flip of the sweep carries.
    mods: craft_keymap::Mods,
    crossed: Vec<usize>,
    travel: Travel,
    /// Where the cell the sweep began in lies across the rows (logical): it flips the crossed rows whose cell of the
    /// column lies there too, so a sweep down a toggle column passes sub-rows by and one down sub-rows passes rows by.
    span: egui::Rangef,
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
    /// The first press held exactly one of the [`SweepApply::claims`] modifiers: it belongs to the host. It flips
    /// nothing, and the cells its drag crosses give nothing. Reported once; its Cancel gives nothing.
    Claimed(ToggleEdit<K>),
}

/// The state a sweep needs between events: which gesture opened it and whether Alt was held.
#[derive(Clone, Copy, Debug)]
struct Run {
    first: GestureId,
    alt: bool,
    /// The first press held one of the claimed modifiers: the sweep gives the host nothing but `Claimed`.
    claimed: bool,
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
    claims: Vec<craft_keymap::Mods>,
}

/// A move of a collection's own items as they stand before the drop, and as the drop leaves them.
type MovePair<K> = (ReorderMove<K>, ReorderMove<K>);

impl SweepApply {
    /// A plain column: Alt is ignored.
    pub fn new() -> Self {
        Self::default()
    }

    /// A column where Alt plus the first press shows only that cell.
    pub fn show_only() -> Self {
        SweepApply { show_only: true, ..Self::default() }
    }

    /// A first press with exactly `mods` held belongs to the host: it flips nothing, and the cells its drag crosses
    /// give nothing (`SweepStep::Claimed`). Claims are checked before `show_only`'s Alt, and the method can be called
    /// more than once.
    pub fn claims(mut self, mods: craft_keymap::Mods) -> Self {
        self.claims.push(mods);
        self
    }

    /// Whether this is a [`SweepApply::show_only`] column, where Alt plus the first press shows only that cell.
    pub fn shows_only(&self) -> bool {
        self.show_only
    }

    /// The first gesture of the running sweep, which a host that groups a sweep's edits as one undo step uses as the
    /// id of all of them. `None` between sweeps.
    pub fn gesture(&self) -> Option<GestureId> {
        self.run.filter(|run| !run.ended).map(|run| run.first)
    }

    /// The step for `edit`, if it asks the host for one. `Commit` repeats what its `Update` already applied.
    pub fn feed<K: Clone>(&mut self, edit: &EditEvent<ToggleEdit<K>>) -> Option<SweepStep<K>> {
        match edit {
            EditEvent::Set { after, .. } => Some(if self.claims.contains(&after.mods) {
                SweepStep::Claimed(after.clone())
            } else if self.show_only && after.mods.alt {
                SweepStep::ShowOnly(after.clone())
            } else {
                SweepStep::Flip(after.clone())
            }),
            EditEvent::Begin { id, before: Mixed::Same(before), .. } => {
                self.open(*id, before);
                None
            }
            EditEvent::Update { id, value, .. } => {
                let run = self.open(*id, value);
                match (run.claimed, run.alt, run.first == *id) {
                    (true, _, true) => Some(SweepStep::Claimed(value.clone())),
                    (true, _, false) | (false, true, false) => None,
                    (false, false, _) => Some(SweepStep::Flip(value.clone())),
                    (false, true, true) => Some(SweepStep::ShowOnly(value.clone())),
                }
            }
            EditEvent::Commit { .. } => {
                self.end();
                None
            }
            EditEvent::Cancel { id, before: Some(Mixed::Same(before)), .. } => match self.end() {
                Some(Run { claimed: true, .. }) => None,
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
                let claimed = self.claims.contains(&edit.mods);
                let run = Run { first: id, alt: !claimed && self.show_only && edit.mods.alt, claimed, ended: false };
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
    /// The drop a release would make this pass.
    pub(super) pending: Option<PendingDrop<K>>,
    /// The selected row a plain press landed on: a click on it narrows the selection to it, a drag keeps it.
    pub(super) collapse: Option<K>,
    /// The unselected row a plain primary press landed on: a click on it picks it, a drag carries it alone and leaves
    /// the selection as it was.
    pub(super) pick: Option<K>,
    /// The row whose rename column got a double press: the editor opens when the press ends as a click, not when it
    /// becomes a drag.
    pub(super) rename: Option<K>,
    /// The row a primary press of an `ActivateOn::Click` collection landed on: it activates when the press ends as a
    /// click, not when it becomes a drag.
    pub(super) activate: Option<K>,
    /// The row a Cmd/Ctrl press of a `command_click` collection landed on: the release as a click reports
    /// `CollectionEvent::CommandClick` with the modifiers held.
    pub(super) command: Option<K>,
    /// The marquee a press on empty space started.
    pub(super) marquee: Option<MarqueeDrag<K>>,
    /// A plain press on the only selected item of a `slow_click_rename` collection, waiting to become a rename.
    pub(super) slow: Option<SlowClick<K>>,
    /// The row a press an item part won landed on, and whether the row held the lead when the press began.
    pub(super) part_press: Option<(K, bool)>,
    /// The request the collection last made for a row's menu: the row shows as the menu's row, and the response
    /// reports the request (`CollectionResponse::menu`), while a menu hung from the collection is open.
    pub(super) menu: Option<ContextMenuRequest<K>>,
}

/// A press that opens a rename when it ends as a click on the item's label and no second press follows within the
/// double-click interval.
#[derive(Clone)]
pub(super) struct SlowClick<K> {
    pub(super) key: K,
    /// Where the press landed, in the rows' layer space.
    pub(super) at: Pos2,
    /// The pass's item showed its label under the press.
    pub(super) on_label: bool,
    /// When the press ended as a click.
    pub(super) released: Option<f64>,
}

impl<K> Default for Drags<K> {
    fn default() -> Self {
        Drags {
            grabbed: None,
            sweep: None,
            moving: None,
            pending: None,
            collapse: None,
            pick: None,
            rename: None,
            activate: None,
            command: None,
            marquee: None,
            slow: None,
            part_press: None,
            menu: None,
        }
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
    /// The held key that turns the drop into a copy; `None`: the drop only moves.
    copy: Option<CopyKeys>,
}

impl<K> DropTarget<ListDrag<K>> for RowDrop {
    type Drop = Landed;

    fn accept(&self, _: &ListDrag<K>, _: Pos2, op: DropOperation) -> Option<Landed> {
        if matches!(op, DropOperation::Copy) && self.copy.is_none() {
            return None;
        }
        self.spot.map(|spot| Landed { spot, op })
    }

    fn mark(&self, drop: &Landed) -> DropMark {
        drop.spot.mark
    }

    fn copy_keys(&self) -> CopyKeys {
        self.copy.unwrap_or_default()
    }
}

/// A drop kind's target: the place under the pointer was tested before it registers.
struct KindDrop {
    mark: DropMark,
}

impl DropTarget<dyn Any + Send + Sync> for KindDrop {
    type Drop = DropOperation;

    fn accept(&self, _: &(dyn Any + Send + Sync), _: Pos2, op: DropOperation) -> Option<DropOperation> {
        Some(op)
    }

    fn mark(&self, _: &DropOperation) -> DropMark {
        self.mark
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
    /// Scrolls the rows while the pointer, `y` below the rows' top edge, is within a row of an edge of the `view`,
    /// faster the further past it, up to `EDGE_ROWS_PER_SECOND` a full band in or beyond the edge: the rate of a row
    /// drag or a marquee near the edge of a scroll region. It asks for the next frame only while the pointer is in or
    /// past a band, so a drag held over the middle rests.
    ///
    /// The step is the frame's time, taken once: a pass that repeats a frame carries no input and scrolls nothing, as
    /// the body's scroll strips do (`body/scroll.rs`, `scroll_input`).
    pub(super) fn edge_scroll(&mut self, cx: &Cx<'_, C>, layout: &C::ItemLayout, view: f32, y: f32) {
        if !self.scrolls || self.source.is_empty() {
            return;
        }
        let row = layout.rect(0, ScrollPos::default()).height();
        // The step is linear in the frame's time, so the rate for one second says whether the pointer is in a band.
        let rate = reorder::edge_step(view, row, y, 1.0);
        if rate == 0.0 {
            return;
        }
        // Taken even by a pass that steps by no time, so the body around does not step in a rerun pass instead.
        reorder::mark_edge_step(&cx.core.ctx);
        cx.core.ctx.request_repaint();
        let dt = crate::runtime::frame::facts(&cx.core.ctx).elapsed.clamp(0.0, 0.1) as f32;
        self.offset = (self.offset + f64::from(rate * dt)).clamp(0.0, self.max_offset(view));
    }

    /// `at` in the layout's logical coordinates, relative to the rows' rect.
    pub(super) fn logical(layout: &C::ItemLayout, rect: Rect, rtl: bool, at: Pos2) -> Pos2 {
        logical_point(layout, rect, rtl, at)
    }

    /// The cells of row `at` (logical): the layout's, with a sub-row's toggle cells at its indent in its content
    /// (`sub_row_cells`), where the row draws them. A press acts on the cell it lands in.
    pub(super) fn row_cells(&self, layout: &C::ItemLayout, at: usize, offset: ScrollPos) -> Vec<(ColumnKey, Rect)> {
        row_cells(layout, at, offset, &|| self.source.sub_row(at), &self.header, self.indent_step)
    }

    /// The toggle cell under `point` (logical) in row `at`: its column, how it toggles and its rect.
    pub(super) fn toggle_cell(
        &self,
        props: &CollectionProps,
        layout: &C::ItemLayout,
        at: usize,
        offset: ScrollPos,
        point: Pos2,
    ) -> Option<(ColumnKey, ToggleColumn, Rect)> {
        toggle_cell_at(props, self.row_cells(layout, at, offset), point)
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
    pub(super) fn begin_toggle(&mut self, cx: &mut Cx<'_, C>, at: usize, (column, toggle, cell): (ColumnKey, ToggleColumn, Rect)) {
        if toggle.sweepable
            && let Some(start) = cx.core.press.phase.start()
        {
            let mods = crate::runtime::pointer::frame_raw(&cx.core.ctx).mods;
            self.drags.sweep = Some(Sweep {
                column: column.clone(),
                on: false,
                mods,
                crossed: vec![at],
                travel: Travel::new(start.origin),
                span: cell.x_range(),
                open: Vec::new(),
            });
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
        let mods = crate::runtime::pointer::frame_raw(&cx.core.ctx).mods;
        self.emit_toggle(cx, at, &column, on, mods);
        if toggle.exclusive && on {
            // Collected first: emitting needs `self` mutably while the source is read through it.
            let others: Vec<usize> = (0..self.source.len()).filter(|&other| other != at && self.source.toggled(other, &column)).collect();
            for other in others {
                self.emit_toggle(cx, other, &column, false, mods);
            }
        }
        Some(on)
    }

    fn sweep_to(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, place: Placement, phase: PressPhase) {
        let Some(sweep) = &mut self.drags.sweep else { return };
        let input = crate::runtime::pointer::pass_pointer(&cx.ctx);
        let events = match phase {
            PressPhase::Down { start, .. } => toggle_sweep::after_press(&input.events, start),
            _ => &input.events,
        };
        let mut path = Vec::new();
        sweep.travel.advance(events, input.frame.as_ref().map_or(0.0, |frame| frame.slop), &mut path);
        for segment in path {
            self.sweep_segment(cx, layout, place, segment);
        }
    }

    fn sweep_segment(&mut self, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, place: Placement, segment: Segment) {
        let Placement { rect, offset, rtl } = place;
        let Some(sweep) = &self.drags.sweep else { return };
        let (column, on, mods, span) = (sweep.column.clone(), sweep.on, sweep.mods, sweep.span);
        let parent = cx.response.layer_id;
        let layer = self.xf.map_or(parent, |_| areas::body_layer(parent, cx.id));
        let clip = rect.intersect(cx.visible);
        let clip = self.xf.map_or(clip, |map| Rect::from_min_max(map.to_layer(clip.min), map.to_layer(clip.max)));
        let mut rows = Vec::new();
        layout.visible(offset, rect.size(), &mut rows);
        if segment.1.y < segment.0.y {
            rows.reverse();
        }
        for at in rows {
            if self.drags.sweep.as_ref().is_some_and(|sweep| sweep.crossed.contains(&at))
                || !self.source.enabled(at)
                || !self.toggle_open(at, &column)
                || self.source.toggled(at, &column) == on
            {
                continue;
            }
            let below_frozen = Rect::from_min_max(Pos2::new(rect.min.x, rect.min.y + layout.frozen_band(at)), rect.max);
            let crossed = self
                .row_cells(layout, at, offset)
                .into_iter()
                .filter(|(key, cell)| *key == column && cell.min.x < span.max && cell.max.x > span.min)
                .map(|(_, cell)| self.to_screen(layout, rect, cell, rtl).intersect(clip).intersect(below_frozen))
                .any(|cell| toggle_sweep::crosses_visible(&cx.ctx, layer, cell, &[segment]));
            if crossed {
                if let Some(sweep) = &mut self.drags.sweep {
                    sweep.crossed.push(at);
                }
                self.emit_toggle(cx, at, &column, on, mods);
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
        let keys = self.keys_at(&self.acted_on(at));
        let count = keys.len() as u64;
        let text = || match props.drag_count {
            Some(word) => Some(cx.env.counted(word, count)),
            None if count > 1 => Some(count.to_string()),
            None => self.source.text(at).map(Cow::into_owned),
        };
        let preview = match props.drag_preview {
            RowPreview::Badge => text().map_or(DragPreview::None, |text| DragPreview::Badge(Label::from(text))),
            RowPreview::Named => text().map_or(DragPreview::None, |text| DragPreview::Named(Label::from(text))),
            _ => DragPreview::None,
        };
        let drag = ListDrag::new(id, keys, key.clone());
        let also: Vec<_> = props.drag_payloads.iter().filter_map(|map| map.of(&drag)).collect();
        if !also.is_empty() {
            cx.start_drag_also(drag, also, preview);
        } else if props.reorder.is_some() {
            cx.start_drag(drag, preview);
        } else {
            return;
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
        let outline = Outline::new(&*self.source);
        drop_spot(layout, &outline, &|at| self.source.section_stop(at), self.xf, (rect, offset, rtl), groups, at)
    }

    /// Whether a drop inside the group row `parent` would put a dragged item inside itself.
    fn inside_dragged(&self, payload: &ListDrag<C::Key>, parent: Option<usize>) -> bool {
        let outline = Outline::new(&*self.source);
        let mut held = parent;
        while let Some(row) = held {
            if payload.keys.contains(&self.source.key(row)) {
                return true;
            }
            held = reorder::parent_of(&outline, row);
        }
        false
    }

    /// The move a drop of the collection's own `payload` at `landed` makes, before and after; `None` when it would
    /// leave every item where it is.
    fn own_move(&self, payload: &ListDrag<C::Key>, landed: Landed, groups: bool) -> Option<MovePair<C::Key>> {
        // The moved items in display order, as they stand in the source now.
        let at: Vec<usize> =
            payload.keys.iter().filter_map(|key| self.index_of(key)).collect::<std::collections::BTreeSet<_>>().into_iter().collect();
        let &from = at.first()?;
        let to = reorder::move_all_to(&at, landed.spot.insert);
        let copy = matches!(landed.op, DropOperation::Copy);
        let together = at.iter().enumerate().all(|(n, &place)| place == from + n);
        let parent = landed.spot.parent.map(|row| self.source.key(row));
        let held = if groups { reorder::parent_of(&Outline::new(&*self.source), from).map(|row| self.source.key(row)) } else { None };
        if to == from && together && !copy && parent == held {
            return None;
        }
        let keys: Vec<C::Key> = at.iter().map(|&place| self.source.key(place)).collect();
        let key = payload.grabbed.clone();
        Some((ReorderMove::new(key.clone(), keys.clone(), from, false).in_group(held), ReorderMove::new(key, keys, to, copy).in_group(parent)))
    }

    fn land(&mut self, cx: &mut Cx<'_, C>, payload: &ListDrag<C::Key>, landed: Landed, groups: bool) {
        if let Some((before, after)) = self.own_move(payload, landed, groups) {
            self.events.push(CollectionEvent::Reorder(EditEvent::set(Mixed::Same(before), after, cx.core.label_arc(), None)));
        }
    }

    /// Where a drop kind at `place` would take a drop at `at` (the collection's layer space), with its mark.
    fn kind_spot(
        &self,
        layout: &C::ItemLayout,
        (rect, offset, rtl): (Rect, ScrollPos, bool),
        place: DropPlace,
        at: Pos2,
    ) -> Option<(DropAt<C::Key>, DropMark)> {
        match place {
            DropPlace::Between if self.source.is_empty() => {
                self.empty_spot(rect, at).map(|spot| (DropAt::Gap { key: None, insert: spot.insert }, spot.mark))
            }
            DropPlace::Between => self
                .spot(layout, rect, offset, rtl, None, at)
                .map(|spot| (DropAt::Gap { key: Some(self.source.key(spot.over)), insert: spot.insert }, spot.mark)),
            DropPlace::Onto => {
                let over = (at.y >= rect.min.y).then(|| layout.hit(Self::logical(layout, rect, rtl, at), offset)).flatten()?;
                if !self.source.enabled(over) {
                    return None;
                }
                let row = self.screen(layout, rect, over, offset, rtl);
                Some((DropAt::Onto(self.source.key(over)), DropMark::Into(self.on_screen_rect(row))))
            }
        }
    }

    /// Whether `at` lies over the middle of a row, between the top and bottom shares where a drop lands before or
    /// after it.
    fn over_middle(&self, layout: &C::ItemLayout, (rect, offset, rtl): (Rect, ScrollPos, bool), at: Pos2) -> bool {
        let Some(over) = (at.y >= rect.min.y).then(|| layout.hit(Self::logical(layout, rect, rtl, at), offset)).flatten() else { return false };
        let row = self.screen(layout, rect, over, offset, rtl);
        let from_top = if row.height() > 0.0 { (at.y - row.min.y) / row.height() } else { 0.0 };
        (reorder::GROUP_EDGE..1.0 - reorder::GROUP_EDGE).contains(&from_top)
    }

    /// A drag that did not start in the collection: the first drop kind that takes its payload at the place under the
    /// pointer registers the target and scrolls the rows at their edges; its release reports `Dropped`, and while held
    /// it is the pending drop. A drag no kind takes here scrolls nothing: it is meant for another target. `own` is the
    /// collection's own drag, offered here where its reorder refuses the landing: only `Onto` kinds take it, and the
    /// rows scrolled already. Where an `Onto` kind takes the same payload, a `Between` kind leaves it the middle of the
    /// row, as a tree's reorder leaves a group row's middle to a drop into it: there the drop is onto the row or
    /// nowhere.
    fn kind_drop(&mut self, comp: &C, cx: &mut Cx<'_, C>, layout: &C::ItemLayout, place: (Rect, ScrollPos, bool), own: bool) {
        let xf = self.xf;
        let (rect, _, rtl) = place;
        let drops = comp.drops();
        for kind in drops {
            if own && !matches!(kind.place(), DropPlace::Onto) {
                continue;
            }
            let Some((payload, at)) = cx.core.dragging_of(kind.payload()) else { continue };
            let point = xf.map_or(at, |map| map.to_layer(at));
            let onto_too = || drops.iter().any(|other| matches!(other.place(), DropPlace::Onto) && other.payload() == kind.payload());
            if matches!(kind.place(), DropPlace::Between) && onto_too() && self.over_middle(layout, place, point) {
                continue;
            }
            let Some((at, mark)) = self.kind_spot(layout, place, kind.place(), point).filter(|(at, _)| kind.accepts(&*payload, at)) else {
                continue;
            };
            if !own {
                self.edge_scroll(cx, layout, rect.height(), Self::logical(layout, rect, rtl, point).y);
            }
            let ordinal = drag::next_ordinal(&cx.core.ctx, cx.core.id);
            if let Some(op) = cx.core.drop_target_of(kind.payload(), &KindDrop { mark }) {
                self.events.push(CollectionEvent::Dropped { at, op, payload: DropPayload::new(payload) });
            } else if drag::winning(&cx.core.ctx, cx.core.id, ordinal) {
                self.drags.pending = Some(PendingDrop::Drop(at));
            }
            return;
        }
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
        self.drags.pending = None;
        let free = matches!(layout.nav(), NavShape::Spatial);
        let phase = cx.core.press.phase;
        if phase.is_press() && !matches!(phase, PressPhase::Cancel { .. }) {
            self.sweep_to(cx, layout, Placement { rect, offset, rtl }, phase);
        }
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
                if self.drags.sweep.is_none()
                    && let Some(at) = hit
                {
                    if free {
                        self.free_move(cx, origin, layer(pos));
                    } else if (props.reorder.is_some() || !props.drag_payloads.is_empty()) && self.drags.grabbed.is_none() {
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
        let own = cx.dragging::<ListDrag<C::Key>>().filter(|(payload, _)| payload.list == cx.core.id);
        if let Some((payload, at)) = own {
            let mut spot = None;
            if let Some(rule) = props.reorder {
                // Above or below the rows too, while the pointer stays over their columns: `edge_scroll` scrolls at
                // its full rate past an edge.
                let point = layer(at);
                if rect.x_range().contains(point.x) {
                    self.edge_scroll(cx, layout, rect.height(), Self::logical(layout, rect, rtl, point).y);
                }
                let groups = rule.into_groups && matches!(layout.nav(), NavShape::Tree);
                let step = groups.then(|| cx.env.control(cx.core.resolved.size).icon);
                spot = if self.source.is_empty() {
                    self.empty_spot(rect, layer(at)).filter(|spot| comp.accepts(&payload, None, spot.insert))
                } else {
                    self.spot(layout, rect, offset, rtl, step, layer(at))
                        .filter(|spot| comp.accepts(&payload, Some(&self.source.key(spot.over)), spot.insert))
                        .filter(|spot| !groups || !self.inside_dragged(&payload, spot.parent))
                };
                let copy = rule.copy;
                let ordinal = drag::next_ordinal(&cx.core.ctx, cx.core.id);
                if let Some(landed) = cx.drop_target::<ListDrag<C::Key>, _>(&RowDrop { spot, copy }) {
                    self.land(cx, &payload, landed, groups);
                } else if let Some(spot) = spot
                    && drag::winning(&cx.core.ctx, cx.core.id, ordinal)
                {
                    let op = drag::current_operation(&cx.core.ctx, copy.unwrap_or_default());
                    if copy.is_some() || !matches!(op, DropOperation::Copy) {
                        self.drags.pending = self.own_move(&payload, Landed { spot, op }, groups).map(|(_, after)| PendingDrop::Reorder(after));
                    }
                }
            }
            if spot.is_none() {
                self.kind_drop(comp, cx, layout, (rect, offset, rtl), true);
            }
        } else {
            self.drags.grabbed = None;
            self.kind_drop(comp, cx, layout, (rect, offset, rtl), false);
        }
    }

    /// A stacking key on the lead row `at`: the rows `acted_on` names move, as a row drag takes them
    /// (`reorder::key_move`).
    pub(super) fn key_reorder(&mut self, comp: &C, cx: &mut Cx<'_, C>, props: &CollectionProps, at: usize, mv: reorder::ItemMove) {
        let groups = props.reorder.is_some_and(|rule| rule.into_groups);
        let outline = Outline::new(&*self.source);
        let tree = groups.then_some(&outline as &dyn craft_behavior::NavOutline);
        let mut places = self.acted_on(at);
        places.sort_unstable();
        // A selected row inside another selected row moves with it.
        if groups {
            let all = places.clone();
            places.retain(|&row| {
                let mut held = reorder::parent_of(&outline, row);
                while let Some(up) = held {
                    if all.contains(&up) {
                        return false;
                    }
                    held = reorder::parent_of(&outline, up);
                }
                true
            });
        }
        let toward_end = mv.toward_end(self.overflow);
        let Some(gap) = reorder::key_move(self.source.len(), tree, &places, toward_end, mv.all_the_way) else { return };
        let Some(&from) = places.first() else { return };
        let to = reorder::move_all_to(&places, gap.insert);
        let parent = gap.parent.map(|row| self.source.key(row));
        let held = if groups { reorder::parent_of(&outline, from).map(|row| self.source.key(row)) } else { None };
        let together = places.iter().enumerate().all(|(n, &place)| place == from + n);
        if to == from && together && parent == held {
            return;
        }
        let keys = self.keys_at(&places);
        let key = self.source.key(at);
        // The collection's own reorder rules govern a stacking key as they govern a drag.
        if !keys.iter().all(|moved| comp.can_drag(moved)) || !comp.accepts(&ListDrag::new(cx.core.id, keys.clone(), key.clone()), None, gap.insert) {
            return;
        }
        let before = ReorderMove::new(key.clone(), keys.clone(), from, false).in_group(held);
        let after = ReorderMove::new(key, keys, to, false).in_group(parent);
        self.events.push(CollectionEvent::Reorder(EditEvent::set(Mixed::Same(before), after, cx.core.label_arc(), None)));
    }

    fn on_screen_rect(&self, rect: Rect) -> Rect {
        Rect::from_min_max(self.on_screen(rect.min), self.on_screen(rect.max))
    }
}
