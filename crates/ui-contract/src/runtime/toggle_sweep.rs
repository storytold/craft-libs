//! A primary press on a standalone toggle can set the other toggles of its layout as it sweeps.

use std::sync::Arc;

use craft_behavior::{PointerButton, PointerEvent, PressPhase, PressStart};
use egui::{Context, Id, IdSet, Pos2, Rect};

use crate::runtime::egui::{areas, pass};
use crate::runtime::frame::{PerViewport, Stored};
use crate::runtime::pointer::{self, PassPointer};

pub(crate) type Segment = (Pos2, Pos2);

/// Pointer travel of one primary press, shared by standalone checks and collection columns.
#[derive(Clone)]
pub(crate) struct Travel {
    origin: Pos2,
    last: Option<Pos2>,
    dragging: bool,
    released: bool,
}

impl Travel {
    pub(crate) fn new(origin: Pos2) -> Self {
        Self { origin, last: Some(origin), dragging: false, released: false }
    }

    pub(crate) fn advance(&mut self, events: &[PointerEvent], slop: f32, path: &mut Vec<Segment>) {
        if self.released {
            return;
        }
        for event in events {
            let (at, released) = match event {
                PointerEvent::Moved { at, .. } => (*at, false),
                PointerEvent::Released { button: PointerButton::Primary, pos, .. } => (*pos, true),
                PointerEvent::Gone => {
                    self.last = None;
                    continue;
                }
                _ => continue,
            };
            let from = if self.dragging { self.last.unwrap_or(at) } else { self.origin };
            self.last = Some(at);
            self.released = released;
            self.dragging |= self.origin.distance(at) > slop;
            if self.dragging && from != at {
                path.push((from, at));
            }
            if released {
                break;
            }
        }
    }
}

/// Ignore any motion in the batch before this press began.
pub(crate) fn after_press(events: &[PointerEvent], press: PressStart) -> &[PointerEvent] {
    events
        .iter()
        .position(|event| {
            matches!(event, PointerEvent::Pressed { button: PointerButton::Primary, pos, time, .. } if *time == press.time && *pos == press.origin)
        })
        .map_or(events, |at| &events[at + 1..])
}

struct Sweep {
    owner: Id,
    group: Id,
    on: bool,
    travel: Travel,
    visited: IdSet,
}

struct CompletedSweep {
    sweep: Sweep,
    path: Arc<Vec<Segment>>,
}

#[derive(Default)]
struct Record {
    pass: u64,
    sweep: Option<Sweep>,
    path: Arc<Vec<Segment>>,
    // A press and move may arrive together; siblings drawn before the pressed control get that
    // initial path on the following pass. Visited ids keep the edit from applying twice.
    replay_first_path: bool,
    // The replay survives opening another gesture in the same pass.
    completed: Option<CompletedSweep>,
}

impl Stored for Record {}

impl Record {
    fn advance(&mut self, pass: u64, input: &PassPointer, cancelled: bool) {
        if self.pass == pass {
            return;
        }
        self.completed = None;
        if !self.replay_first_path {
            Arc::make_mut(&mut self.path).clear();
        }
        let replay = std::mem::take(&mut self.replay_first_path);
        let previous = std::mem::replace(&mut self.pass, pass);
        if cancelled || previous.saturating_add(1) != pass {
            self.sweep = None;
            return;
        }
        if self.sweep.as_ref().is_some_and(|sweep| sweep.travel.released) {
            if replay {
                self.finish();
            } else {
                self.sweep = None;
            }
            return;
        }
        let next_press = input.events.iter().position(|event| matches!(event, PointerEvent::Pressed { button: PointerButton::Primary, .. }));
        if let Some(sweep) = &mut self.sweep {
            // Input following the first press is held for the next pass, which can then carry both its release and a new press.
            let events = &input.events[..next_press.unwrap_or(input.events.len())];
            sweep.travel.advance(events, input.frame.as_ref().map_or(0.0, |frame| frame.slop), Arc::make_mut(&mut self.path));
            if next_press.is_some() && sweep.travel.released {
                self.finish();
                return;
            }
        }
        if next_press.is_some() || (!input.primary_down && self.sweep.as_ref().is_some_and(|sweep| !sweep.travel.released)) {
            self.sweep = None;
        }
    }

    fn finish(&mut self) {
        self.completed = self.sweep.take().map(|sweep| CompletedSweep { sweep, path: std::mem::take(&mut self.path) });
    }

    fn start(&mut self, owner: Id, group: Id, press: PressStart, on: bool, input: &PassPointer) {
        let mut sweep = Sweep { owner, group, on, travel: Travel::new(press.origin), visited: IdSet::default() };
        let path = Arc::make_mut(&mut self.path);
        path.clear();
        sweep.travel.advance(after_press(&input.events, press), input.frame.as_ref().map_or(0.0, |frame| frame.slop), path);
        self.replay_first_path = !path.is_empty();
        self.sweep = Some(sweep);
    }
}

pub(crate) struct ToggleTarget {
    pub id: Id,
    pub layer: areas::Layer,
    pub visible: Rect,
    pub enabled: bool,
    pub press: PressPhase,
}

pub(crate) fn value(ctx: &Context, target: ToggleTarget, group: Id, on: Option<bool>) -> Option<bool> {
    let ToggleTarget { id, layer, visible, enabled, press } = target;
    let viewport = pass::current_viewport(ctx);
    let pass = pointer::pass_of(ctx);
    let input = pointer::pass_pointer(ctx);
    let cancelled = pointer::cancel_of_pass(ctx).is_some();
    let eligible = |sweep: &Sweep| enabled && sweep.group == group && sweep.travel.dragging;
    let completed = PerViewport::<Record>::with(ctx, viewport, |record| {
        record.advance(pass, &input, cancelled);
        let finished = record.completed.as_ref().filter(|finished| eligible(&finished.sweep))?;
        let sweep = &finished.sweep;
        if sweep.owner == id {
            return Some((sweep.on, None));
        }
        (!sweep.visited.contains(&id)).then(|| (sweep.on, Some(Arc::clone(&finished.path))))
    })
    .and_then(|(on, path)| {
        if let Some(path) = path {
            if !crosses_visible(ctx, layer, visible, &path) {
                return None;
            }
            PerViewport::<Record>::with(ctx, viewport, |record| {
                if let Some(finished) = &mut record.completed {
                    finished.sweep.visited.insert(id);
                }
            });
        }
        Some(on)
    });
    let ready = PerViewport::<Record>::with(ctx, viewport, |record| {
        match press {
            PressPhase::Down { start, .. } if start.button == PointerButton::Primary && enabled => {
                record.start(id, group, start, !completed.or(on).unwrap_or(false), &input);
            }
            PressPhase::Cancel { .. } if record.sweep.as_ref().is_some_and(|sweep| sweep.owner == id) => record.sweep = None,
            _ => {}
        }
        let sweep = record.sweep.as_ref().filter(|sweep| eligible(sweep))?;
        if sweep.owner == id {
            return Some((sweep.on, None, record.replay_first_path));
        }
        (!record.path.is_empty() && !sweep.visited.contains(&id)).then(|| (sweep.on, Some(Arc::clone(&record.path)), false))
    });
    let Some(ready) = ready else { return completed };
    let (on, path, replay) = ready;
    if replay {
        ctx.request_repaint();
    }
    if let Some(path) = path {
        if !crosses_visible(ctx, layer, visible, &path) {
            return completed;
        }
        PerViewport::<Record>::with(ctx, viewport, |record| {
            if let Some(sweep) = &mut record.sweep {
                sweep.visited.insert(id);
            }
        });
    }
    Some(on)
}

/// Keep the entire part of the path inside the rect: a floating layer may cover only part of it.
fn clipped((from, to): Segment, rect: Rect) -> Option<Segment> {
    if !rect.is_positive() {
        return None;
    }
    let delta = to - from;
    let (mut start, mut end) = (0.0_f32, 1.0_f32);
    for (step, low, high, from) in [(delta.x, rect.min.x, rect.max.x, from.x), (delta.y, rect.min.y, rect.max.y, from.y)] {
        if step == 0.0 {
            if from < low || from > high {
                return None;
            }
        } else {
            let (a, b) = ((low - from) / step, (high - from) / step);
            start = start.max(a.min(b));
            end = end.min(a.max(b));
        }
    }
    (start <= end).then(|| (from + delta * start, from + delta * end))
}

pub(crate) fn crosses_visible(ctx: &Context, layer: areas::Layer, rect: Rect, path: &[Segment]) -> bool {
    let map = areas::layer_map(ctx, layer);
    let rect = map.map_or(rect, |map| Rect::from_min_max(map.to_screen(rect.min), map.to_screen(rect.max)));
    path.iter().copied().filter_map(|segment| clipped(segment, rect)).any(|(from, to)| areas::layer_between(ctx, layer, from, to))
}
