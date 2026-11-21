//! One pass of the ruler's input: the gestures on the items, the strip and the column borders, keys and the
//! screen reader's requests.

use super::items::{COLUMN, GAP, RulerItem, STRIP, gap_of_part, gap_part, moved, moved_all, place, without};
use super::{
    ColumnBorder, ColumnDrag, GapDrag, GapEdge, RulerDrag, RulerValues, SAME_POSITION, TabLeader, TabStop, TextRuler, dropped_off, snap_stop,
};
use crate::ui::length::{DocUnit, shown};
use craft_behavior::{CancelReason, GesturePhase, Granularity, Intent, NavMove, PointerButton, PressPhase, RulerUnits, Ticks};
use craft_ui::canvas::{RulerText, Rulers, TickRule, readout};
use craft_ui::contract::{Cx, Edit as _, Focus as _, Pointer as _, Remember as _, WordSetCx as _};
use craft_ui::runtime::env::{Label, Units};
use egui::Pos2;

/// How many snap steps a Shift nudge moves.
const LARGE_NUDGE: f32 = 10.0;

/// What a drag on the ruler is doing, kept for its length.
#[derive(Clone, Debug, Default)]
pub(super) struct Held {
    /// The values when the gesture began.
    anchor: Option<RulerValues>,
    /// The item grabbed (a stop by its index when the gesture began).
    from: Option<RulerItem>,
    /// A new stop is being placed, not an existing one moved.
    placing: bool,
    /// Cmd was held when a stop was grabbed: every stop moves with it.
    all: bool,
    /// The value last proposed on the ruler; a stop off the ruler proposes it again so the drag note goes.
    shown: Option<RulerValues>,
    /// Where the item is now (a stop's index follows it through the sorted stops).
    pub(super) now: Option<RulerItem>,
    /// The pointer is far enough off the ruler to clear the stop on release.
    pub(super) off: bool,
    /// The borders of the table when a column border drag began.
    columns: Option<Vec<f32>>,
    /// Alt is held in a column border drag: the columns' widths show between the borders.
    pub(super) widths: bool,
    /// The gap edge being dragged: its part index and the position it had when the drag began.
    gap: Option<(u64, f32)>,
}

/// The ruler's units: ruler units per point, written as the unit's kind.
pub fn ruler_text(unit: DocUnit, units: &Units) -> RulerText {
    RulerText::new(RulerUnits::new(shown(units, unit, 1.0), Ticks::decimal(false)), unit.kind)
}

/// The ruler's zoom for the snap grid, screen points per point: the grid is the finest tick shown at this zoom
/// (1/16 in for inches), so it does not change with the zoom the ruler is shown at.
const SNAP_ZOOM: f64 = 2.0;

/// The distance a snapping drag moves by: the ruler's smallest tick at `SNAP_ZOOM`, in points.
pub fn snap_step(text: &RulerText, unit: DocUnit) -> f32 {
    let ticks = Rulers::ticks(&text.units, TickRule::of_unit(unit.kind), SNAP_ZOOM, 0.0..=0.0);
    let gap = ticks.windows(2).find_map(|pair| match pair {
        [a, b] => Some(b.value - a.value),
        _ => None,
    });
    match gap.map(|minor| minor / text.units.per_doc) {
        Some(points) if points.is_finite() && points > 0.0 => points as f32,
        _ => SAME_POSITION,
    }
}

/// `position` snapped to `step` (unless `free`), kept within `bounds`.
fn snap_within(position: f32, step: f32, bounds: (f32, f32), free: bool) -> f32 {
    let snapped = if free || !step.is_finite() || step <= 0.0 { position } else { (position / step).round() * step };
    snapped.clamp(bounds.0, bounds.1)
}

/// How far one key step moves a part, points: `SAME_POSITION` when `fine`, ten snap `step`s when `large`, else one. The
/// one key rule of every ruler.
pub fn nudge_size(step: f32, fine: bool, large: bool) -> f32 {
    if fine {
        SAME_POSITION
    } else if large {
        LARGE_NUDGE * step
    } else {
        step
    }
}

/// The moves and nudges taken from the focused part this pass: how far they move it, in points (positive: further
/// from the start edge).
fn take_steps(cx: &mut Cx<'_, TextRuler<'_>>, g: &super::Geometry, step: f32) -> f32 {
    let mut by = 0.0;
    while let Some((mv, mods)) = cx.intents().take_move() {
        let size = nudge_size(step, mods.fine, mods.extend);
        match mv {
            NavMove::Forward | NavMove::Up => by += size * g.sign(),
            NavMove::Back | NavMove::Down => by -= size * g.sign(),
            _ => {}
        }
    }
    if let Some((direction, mods)) = cx.intents().take_nudge() {
        let size = nudge_size(step, mods.alt, mods.shift);
        by += direction.x * size * g.sign();
    }
    by
}

impl TextRuler<'_> {
    pub(super) fn run(&mut self, cx: &mut Cx<'_, Self>) {
        let values = cx.value().cloned().unwrap_or_default();
        let layout = self.layout(cx.rect, cx.style, cx.flow.is_rtl());
        let numbers = cx.env.units();
        let text = ruler_text(self.unit, &numbers);
        let step = snap_step(&text, self.unit);
        let mut held = cx.memory::<Held>().clone();
        if !cx.gesture().is_some() && held.columns.is_none() {
            held.off = false;
            held.placing = false;
            held.all = false;
            held.shown = None;
            held.anchor = None;
        }

        for item in RulerItem::all(&values) {
            self.run_item(cx, &layout.g, step, &mut held, &text, item);
        }
        self.run_strip(cx, &layout.g, step, &mut held, &text, &values);
        self.run_columns(cx, &layout.g, step, &mut held);
        self.run_gaps(cx, &layout.g, step, &mut held);
        if self.kind.is_some() {
            let press = cx.part(super::items::SELECTOR).press;
            if let PressPhase::Release { start, click: true, .. } = press.phase {
                let kind = self.kind.unwrap_or_default();
                self.report.kind = Some(kind.next(start.button == PointerButton::Secondary));
            }
        }
        self.run_keys(cx, &layout.g, step, &mut held);
        *cx.memory::<Held>() = held;
    }

    /// The pointer on one marker or stop.
    fn run_item(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, step: f32, held: &mut Held, text: &RulerText, item: RulerItem) {
        let part = item.part();
        // A stop set in this pass has no part until the next one.
        let Some(press) = cx.part_if_declared(part).map(|declared| declared.press) else { return };
        match press.phase {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary => {
                self.report.picked = Some(item);
                if matches!(item, RulerItem::Stop(_)) && start.clicks >= 2 {
                    self.report.open_tabs_dialog = true;
                }
                cx.focus_part(part);
            }
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary => {
                let anchor = cx.value().cloned().unwrap_or_default();
                let all = press.mods.cmd && matches!(item, RulerItem::Stop(_));
                *held = Held { anchor: Some(anchor), from: Some(item), now: Some(item), all, ..Held::default() };
                cx.focus_part(part);
                cx.begin();
                self.drag_item(cx, g, step, held, text, start.origin, pos, press.mods.alt);
            }
            PressPhase::Drag { start, pos, .. } if start.button == PointerButton::Primary && held.from.is_some() && !held.placing => {
                // Only the press that began the drag moves the item: its part keeps the index it had.
                if held.from == Some(item) {
                    self.drag_item(cx, g, step, held, text, start.origin, pos, press.mods.alt);
                }
            }
            PressPhase::Release { start, click: false, pos, .. } if start.button == PointerButton::Primary && held.from == Some(item) => {
                self.drag_item(cx, g, step, held, text, start.origin, pos, press.mods.alt);
                if held.off
                    && let (Some(anchor), Some(RulerItem::Stop(grab))) = (held.anchor.clone(), held.from)
                {
                    cx.propose_with(without(&anchor, grab), None::<RulerDrag>);
                }
                cx.commit();
                if let Some(now) = held.now {
                    cx.focus_part(now.part());
                }
                *held = Held::default();
            }
            PressPhase::Cancel { .. } if held.from == Some(item) => {
                cx.cancel(CancelReason::Escape);
                *held = Held::default();
            }
            _ => {}
        }
    }

    /// Follows the pointer with the grabbed item: a stop off the ruler stays where it was and fades.
    #[allow(clippy::too_many_arguments)]
    fn drag_item(
        &mut self,
        cx: &mut Cx<'_, Self>,
        g: &super::Geometry,
        step: f32,
        held: &mut Held,
        text: &RulerText,
        origin: Pos2,
        pos: Pos2,
        free: bool,
    ) {
        let (Some(anchor), Some(from)) = (held.anchor.clone(), held.from) else { return };
        let start = from.position(&anchor, g.width);
        let target = snap_stop(start + (pos.x - origin.x) / g.scale * g.sign(), step, g.width, free);
        let moved = match from {
            RulerItem::Stop(grab) if held.all => (moved_all(&anchor, grab, target, g.width), from),
            _ => moved(&anchor, from, target, g.width),
        };
        held.off = matches!(from, RulerItem::Stop(_)) && dropped_off(pos.y, g.strip);
        self.follow(cx, g, held, text, pos, moved);
    }

    /// Proposes the dragged value with its note. Off the ruler the stop keeps its last shown value and the note goes, so
    /// [`TextRuler::dragging`] never reports a position the pointer left.
    fn follow(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, held: &mut Held, text: &RulerText, pos: Pos2, moved: (RulerValues, RulerItem)) {
        let (value, now) = moved;
        if held.off {
            let kept = held.shown.clone().or_else(|| held.anchor.clone()).unwrap_or_default();
            cx.propose_with(kept, None::<RulerDrag>);
            return;
        }
        held.now = Some(now);
        held.shown = Some(value.clone());
        cx.propose_with(value.clone(), Some(RulerDrag { item: now, position: now.position(&value, g.width) }));
        self.show_position(cx, text, pos, now, &value, g.width);
    }

    /// The position of the dragged item at the pointer.
    fn show_position(&self, cx: &Cx<'_, Self>, text: &RulerText, at: Pos2, item: RulerItem, values: &RulerValues, width: f32) {
        let words = cx.words::<super::TextRulerWords>();
        let name = Self::item_name(&words, item, values).to_owned();
        let value = text.format_value(&cx.env.units(), f64::from(item.position(values, width)), 0.0);
        readout(cx, at, &[(Label::from(name), value)]);
    }

    /// A press on the empty column places a stop of the selector's kind; a click does too.
    fn run_strip(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, step: f32, held: &mut Held, text: &RulerText, values: &RulerValues) {
        let press = cx.part(STRIP).press;
        let kind = self.kind.unwrap_or_default();
        let stop_at = |x: f32, free: bool| TabStop { position: snap_stop(g.at(x), step, g.width, free), align: kind, leader: TabLeader::None };
        match press.phase {
            PressPhase::DragStart { start, pos, .. } if start.button == PointerButton::Primary && g.column().contains(start.origin) => {
                *held = Held { anchor: Some(values.clone()), placing: true, ..Held::default() };
                cx.begin();
                self.drag_placing(cx, g, held, text, pos, &stop_at(pos.x, press.mods.alt));
            }
            PressPhase::Drag { pos, .. } if held.placing => self.drag_placing(cx, g, held, text, pos, &stop_at(pos.x, press.mods.alt)),
            PressPhase::Release { click: false, pos, .. } if held.placing => {
                self.drag_placing(cx, g, held, text, pos, &stop_at(pos.x, press.mods.alt));
                if held.off
                    && let Some(anchor) = held.anchor.clone()
                {
                    cx.propose_with(anchor, None::<RulerDrag>);
                }
                cx.commit();
                *held = Held::default();
            }
            PressPhase::Cancel { .. } if held.placing => {
                cx.cancel(CancelReason::Escape);
                *held = Held::default();
            }
            PressPhase::Release { start, click: true, pos, .. } if start.button == PointerButton::Primary && g.column().contains(pos) => {
                let (value, at) = place(values, stop_at(pos.x, press.mods.alt));
                cx.set(value);
                self.report.picked = Some(RulerItem::Stop(at));
            }
            _ => {}
        }
    }

    fn drag_placing(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, held: &mut Held, text: &RulerText, pos: Pos2, stop: &TabStop) {
        let Some(anchor) = held.anchor.clone() else { return };
        held.off = dropped_off(pos.y, g.strip);
        let (value, at) = place(&anchor, *stop);
        self.follow(cx, g, held, text, pos, (value, RulerItem::Stop(at)));
    }

    /// The table column borders: one gesture per border drag, reported with its phase. The columns right of a dragged
    /// border move with it, which the app applies from the report.
    fn run_columns(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, step: f32, held: &mut Held) {
        let (lo, hi) = g.span();
        for (index, position) in self.columns.iter().copied().enumerate() {
            let press = cx.part(COLUMN.at(index as u64)).press;
            let report = |phase: GesturePhase, position: f32| Some(ColumnDrag { phase, border: ColumnBorder { index, position }, mods: press.mods });
            match press.phase {
                PressPhase::DragStart { start, .. } if start.button == PointerButton::Primary => {
                    held.columns = Some(self.columns.to_vec());
                    self.report.column = report(GesturePhase::Begin, position);
                    cx.focus_own();
                }
                PressPhase::Drag { start, pos, .. } if held.columns.is_some() => {
                    let from = held.columns.as_ref().and_then(|columns| columns.get(index)).copied().unwrap_or(position);
                    let to = from + (pos.x - start.origin.x) / g.scale * g.sign();
                    self.report.column = report(GesturePhase::Update, snap_within(to, step, (lo, hi), press.mods.alt));
                }
                PressPhase::Release { start, pos, click: false, .. } if held.columns.is_some() => {
                    let from = held.columns.as_ref().and_then(|columns| columns.get(index)).copied().unwrap_or(position);
                    let to = from + (pos.x - start.origin.x) / g.scale * g.sign();
                    self.report.column = report(GesturePhase::Commit, snap_within(to, step, (lo, hi), press.mods.alt));
                    held.columns = None;
                }
                PressPhase::Cancel { .. } if held.columns.is_some() => {
                    self.report.column = report(GesturePhase::Cancel, position);
                    held.columns = None;
                }
                _ => {}
            }
        }
        held.widths = held.columns.is_some()
            && (0..self.columns.len()).any(|index| {
                let press = cx.part(COLUMN.at(index as u64)).press;
                press.captured && press.mods.alt
            });
    }

    /// The column gap edges: one gesture per drag (Begin, Update, Commit, or Cancel on Escape), snapped to the ruler's
    /// step unless Alt is held. An edge stays on its side of the other edge of the gap and within the strip, and a
    /// press released without a drag reports nothing.
    fn run_gaps(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, step: f32, held: &mut Held) {
        // Fixed gaps declared no edge parts.
        if !self.gaps_draggable {
            return;
        }
        let (lo, hi) = g.span();
        for (index, gap) in self.gaps.iter().copied().enumerate() {
            for edge in [GapEdge::End, GapEdge::Start] {
                let id = gap_part(index, edge);
                let press = cx.part(GAP.at(id)).press;
                let (here, bounds) = match edge {
                    GapEdge::End => (gap.0, (lo, gap.1.max(lo))),
                    GapEdge::Start => (gap.1, (gap.0.min(hi), hi)),
                };
                let report =
                    |phase: GesturePhase, position: f32| Some(GapDrag { phase, index: gap_of_part(id).0, edge: gap_of_part(id).1, position });
                match press.phase {
                    PressPhase::DragStart { start, .. } if start.button == PointerButton::Primary => {
                        held.gap = Some((id, here));
                        self.report.gap = report(GesturePhase::Begin, here);
                    }
                    PressPhase::Drag { start, pos, .. } if matches!(held.gap, Some((held_id, _)) if held_id == id) => {
                        let from = held.gap.map_or(here, |(_, from)| from);
                        let to = from + (pos.x - start.origin.x) / g.scale * g.sign();
                        let at = if to == from { from } else { snap_within(to, step, bounds, press.mods.alt) };
                        self.report.gap = report(GesturePhase::Update, at);
                    }
                    PressPhase::Release { start, pos, click: false, .. } if matches!(held.gap, Some((held_id, _)) if held_id == id) => {
                        let from = held.gap.map_or(here, |(_, from)| from);
                        let to = from + (pos.x - start.origin.x) / g.scale * g.sign();
                        let at = if to == from { from } else { snap_within(to, step, bounds, press.mods.alt) };
                        self.report.gap = report(GesturePhase::Commit, at);
                        held.gap = None;
                    }
                    PressPhase::Cancel { .. } if matches!(held.gap, Some((held_id, _)) if held_id == id) => {
                        self.report.gap = report(GesturePhase::Cancel, here);
                        held.gap = None;
                    }
                    _ => {}
                }
            }
        }
    }

    /// A key or the screen reader sets the table column border `index` to `to` points: one gesture of one commit.
    fn step_column(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, index: usize, to: f32) {
        let (lo, hi) = g.span();
        let border = ColumnBorder { index, position: snap_within(to, 0.0, (lo, hi), true) };
        self.report.column = Some(ColumnDrag { phase: GesturePhase::Commit, border, mods: craft_keymap::Mods::NONE });
        cx.focus_part(COLUMN.at(index as u64));
    }

    /// Keys on the focused item: the arrows nudge it, Delete and Backspace clear a stop; a screen reader's SetValue sets an item's position.
    fn run_keys(&mut self, cx: &mut Cx<'_, Self>, g: &super::Geometry, step: f32, held: &mut Held) {
        if cx.gesture().is_some() {
            return;
        }
        let values = cx.value().cloned().unwrap_or_default();
        for item in RulerItem::all(&values) {
            if let Some(to) = cx.intents().take_set_value_at(item.part()) {
                let (value, now) = moved(&values, item, to as f32, g.width);
                cx.set(value);
                cx.focus_part(now.part());
                return;
            }
        }
        for index in 0..self.columns.len() {
            if let Some(to) = cx.intents().take_set_value_at(COLUMN.at(index as u64)) {
                self.step_column(cx, g, index, to as f32);
                return;
            }
        }
        let Some(part) = cx.active_part() else { return };
        if part.key == COLUMN {
            let by = take_steps(cx, g, step);
            if let (Some(index), true) = (part.index, by != 0.0)
                && let Some(position) = self.columns.get(index as usize).copied()
            {
                self.step_column(cx, g, index as usize, position + by);
            }
            return;
        }
        let Some(item) = RulerItem::of_part(part) else { return };
        let by = take_steps(cx, g, step);
        if by != 0.0 {
            let (value, now) = moved(&values, item, item.position(&values, g.width) + by, g.width);
            cx.set(value);
            cx.focus_part(now.part());
        }
        let forward = cx.intents().take_exact(&Intent::delete(true, Granularity::Unit));
        let backward = cx.intents().take_exact(&Intent::delete(false, Granularity::Unit));
        if (forward || backward)
            && let RulerItem::Stop(at) = item
        {
            cx.set(without(&values, at));
            let left = values.stops.len().saturating_sub(1);
            let next = left.checked_sub(1).map(|last| RulerItem::Stop(at.saturating_sub(1).min(last))).unwrap_or(RulerItem::LeftBox);
            cx.focus_part(next.part());
        }
        let _ = held;
    }
}
