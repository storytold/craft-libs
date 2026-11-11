//! The overlay's pointer: the press on a part, the drag of an end, the line, a stop or a midpoint, the click that adds a
//! stop, double-clicks and secondary clicks.

use craft_behavior::kurbo::Point;
use craft_behavior::{Axis, CanvasView, PointerButton, PressPhase};
use craft_keymap::Mods;
use craft_ui::canvas::{CanvasSpace, Snap, context_click};
use craft_ui::contract::{Cx, Edit as _, Focus as _, Pointer as _, Remember as _};
use egui::Pos2;

use super::GradientOverlay;
use super::parts::{keyboard_order, part_ref, selection_of};
use crate::ui::gradient::gradient_line::{MIN_MIDPOINT_GAP, project, snap45};
use crate::ui::gradient::stops::{self, round_location};
use crate::ui::gradient::{GradientLine, LineGradient, LinePart, Sampler, StopRef, StopRow, default_sampler};

/// The press a drag edits from: the part it took and the gradient at the press. Kept for the length of the press.
#[derive(Clone, Default)]
pub(super) struct Held {
    part: LinePart,
    base: LineGradient,
    /// An Alt-drag of a stop: the drag moves a copy of it.
    copy: bool,
    /// The selection before the press, restored when the press is cancelled.
    before: Option<StopRef>,
}

/// What a pointer position makes of the held press.
struct Dragged {
    value: LineGradient,
    /// The selection the drag makes: a dragged stop is selected at its new index, a stop torn off clears it.
    selected: Option<Option<StopRef>>,
    /// The dragged stop is torn off the line.
    torn: bool,
}

/// `p` moved to what the canvas snaps it to.
fn snapped(snap: Option<&Snap>, p: Point, mods: Mods) -> Point {
    match snap {
        Some(snap) => Point::new(snap.apply(Axis::Y, p.x, mods), snap.apply(Axis::X, p.y, mods)),
        None => p,
    }
}

impl GradientOverlay<'_> {
    /// The presses on the parts of `value`: select, drag, click, double click and the context request.
    pub(super) fn run_pointer(&mut self, cx: &mut Cx<'_, Self>, space: &CanvasSpace, snap: Option<&Snap>, value: &LineGradient) {
        let order = keyboard_order(value, self.midpoints);
        let hovered = |cx: &Cx<'_, Self>, part: &LinePart| cx.part_if_declared(part_ref(*part)).is_some_and(|result| result.hovered);
        // The part a press would take: the end, the start, the nearest stop, the nearest midpoint, then the line.
        let by_priority = |kind: fn(&LinePart) -> bool| order.iter().rev().filter(move |part| kind(part)).copied().collect::<Vec<_>>();
        let ranked = [
            vec![LinePart::End, LinePart::Start],
            by_priority(|p| matches!(p, LinePart::Stop(_))),
            by_priority(|p| matches!(p, LinePart::Midpoint(_))),
            vec![LinePart::Line],
        ];
        self.report.hovered = ranked.iter().flatten().copied().find(|part| hovered(cx, part));
        for part in order {
            let Some(result) = cx.part_if_declared(part_ref(part)) else { continue };
            let press = result.press;
            let (mods, view) = (press.mods, &space.view);
            match press.phase {
                PressPhase::Down { start, .. } if start.button == PointerButton::Primary && !mods.cmd => {
                    *cx.press_slot::<Held>() = Some(self.held(cx, part, value, mods));
                    if let Some(selected) = selection_of(part) {
                        self.select(cx, Some(selected));
                    }
                    if press.double_pressed() {
                        self.report.color_requested = self.color_request(value, part);
                    }
                }
                PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. }
                    if start.button == PointerButton::Primary && !mods.cmd =>
                {
                    if cx.press_slot::<Held>().is_none() {
                        *cx.press_slot::<Held>() = Some(self.held(cx, part, value, mods));
                    }
                    if matches!(press.phase, PressPhase::DragStart { .. }) {
                        cx.begin();
                    }
                    let Some(held) = cx.press_slot::<Held>().clone() else { continue };
                    let dragged = self.drag_to(view, snap, &held, start.origin, pos, mods, cx.style.tear_off);
                    self.report.torn = dragged.torn;
                    cx.propose(if dragged.torn { held.base.clone() } else { dragged.value });
                    self.follow(cx, dragged.selected);
                }
                PressPhase::Release { start, pos, click, .. } if start.button == PointerButton::Primary && !mods.cmd => {
                    let Some(held) = cx.press_slot::<Held>().clone() else { continue };
                    if click {
                        if held.part == LinePart::Line {
                            self.add_at(cx, view, &held.base, pos);
                        }
                    } else {
                        let dragged = self.drag_to(view, snap, &held, start.origin, pos, mods, cx.style.tear_off);
                        cx.propose(dragged.value);
                        cx.commit();
                        self.follow(cx, dragged.selected);
                    }
                }
                PressPhase::Cancel { start, .. } if start.button == PointerButton::Primary => {
                    if let Some(held) = cx.press_slot::<Held>().take() {
                        self.select(cx, held.before);
                    }
                }
                _ => {}
            }
            if context_click(&press).is_some() {
                self.report.context = Some(part);
                if let Some(selected) = selection_of(part) {
                    self.select(cx, Some(selected));
                }
            }
        }
    }

    /// The press on `part` of `value`, taken now: the gradient and the selection it starts from.
    fn held(&mut self, cx: &mut Cx<'_, Self>, part: LinePart, value: &LineGradient, mods: Mods) -> Held {
        Held { part, base: value.clone(), copy: mods.alt && self.alt_copies && matches!(part, LinePart::Stop(_)), before: self.selection(cx) }
    }

    /// The selection a drag makes, and the focus following the stop it moves.
    fn follow(&mut self, cx: &mut Cx<'_, Self>, selected: Option<Option<StopRef>>) {
        let Some(selected) = selected else { return };
        self.select(cx, selected);
        if let Some(StopRef::Color(i)) = selected {
            let target = part_ref(LinePart::Stop(i));
            if cx.active_part() != Some(target) {
                cx.focus_part(target);
            }
        }
    }

    /// A click at screen point `at` on the line of `base`: a colour stop there, coloured by the sampler, selected.
    fn add_at(&mut self, cx: &mut Cx<'_, Self>, view: &CanvasView, base: &LineGradient, at: Pos2) {
        let (t, _) = self.located(view, &base.line, at);
        let default = default_sampler(self.rules.rows);
        let sampler: &Sampler = self.sampler.unwrap_or(&default);
        if let Some((added, i)) = stops::add(&base.stops, StopRow::Color, round_location(t), sampler, &self.rules) {
            cx.set(LineGradient { line: base.line, stops: added });
            self.follow(cx, Some(Some(StopRef::Color(i))));
        }
    }

    /// The held press's gradient with its part at screen point `pointer`: the whole edit of the gradient at the press.
    #[allow(clippy::too_many_arguments)]
    fn drag_to(&self, view: &CanvasView, snap: Option<&Snap>, held: &Held, origin: Pos2, pointer: Pos2, mods: Mods, tear_off: f32) -> Dragged {
        let base = &held.base;
        let mut out = Dragged { value: base.clone(), selected: None, torn: false };
        match held.part {
            LinePart::Start | LinePart::End => {
                let (fixed, moved) = match held.part {
                    LinePart::Start => (base.line.end, &mut out.value.line.start),
                    _ => (base.line.start, &mut out.value.line.end),
                };
                let to = view.to_doc(pointer);
                let to = if mods.shift { snap45(fixed, to, self.angle_base) } else { to };
                *moved = snapped(snap, to, mods);
            }
            LinePart::Line => {
                let by = view.to_doc(pointer) - view.to_doc(origin);
                let by = snapped(snap, base.line.start + by, mods) - base.line.start;
                out.value.line = GradientLine { start: base.line.start + by, end: base.line.end + by };
            }
            LinePart::Stop(i) => {
                let (t, off) = self.located(view, &base.line, pointer);
                let at = round_location(t);
                // At the row's minimum a stop dragged off the line stays and moves.
                let torn = (!held.copy && off > tear_off).then(|| stops::remove(&base.stops, StopRow::Color, i, &self.rules)).flatten();
                if let Some((stops, _)) = torn {
                    out.value.stops = stops;
                    out.selected = Some(None);
                    out.torn = true;
                } else {
                    let edited = match held.copy {
                        true => stops::copy(&base.stops, StopRow::Color, i, at, &self.rules),
                        false => stops::move_to(&base.stops, StopRow::Color, i, at, &self.rules),
                    };
                    let (stops, j) = edited.unwrap_or_else(|| (base.stops.clone(), i));
                    out.value.stops = stops;
                    out.selected = Some(Some(StopRef::Color(j)));
                }
            }
            LinePart::Midpoint(i) => {
                let (t, _) = self.located(view, &base.line, pointer);
                if let (Some(a), Some(b)) = (base.stops.color.get(i), base.stops.color.get(i + 1))
                    && b.location - a.location > MIN_MIDPOINT_GAP
                    && let Some((stops, _)) =
                        stops::set_midpoint(&base.stops, StopRow::Color, i, (t - a.location) / (b.location - a.location), &self.rules)
                {
                    out.value.stops = stops;
                }
            }
        }
        out
    }

    /// The location on `line` nearest screen point `p`, and the screen distance from it. Measured on the screen, so a
    /// stretched view tears off at the distance it shows.
    fn located(&self, view: &CanvasView, line: &GradientLine, p: Pos2) -> (f32, f32) {
        let point = |p: Pos2| Point::new(p.x.into(), p.y.into());
        let screen = GradientLine { start: point(view.to_screen(line.start)), end: point(view.to_screen(line.end)) };
        let (t, distance) = project(&screen, point(p), self.reverse);
        (t, distance as f32)
    }
}
