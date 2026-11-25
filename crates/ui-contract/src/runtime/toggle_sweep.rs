//! A primary press on a standalone toggle can set the other toggles of its layout as it sweeps.
//!
//! A drag from a toggle that crosses no other toggle of its layout and does not end on the toggle itself is not a sweep:
//! it cancels, as a press released outside a control does, and the pressed toggle keeps its value. Only a toggle that
//! takes the sweep in the pass the drag reaches it counts as crossed: one that pass disables, hides, moves or leaves out
//! takes nothing, so the press stays one released outside the control.

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
    /// A sibling of the group took the sweep's value in a pass, or the button went up over the owner: the owner takes
    /// the value too, in this pass or the next.
    engaged: bool,
}

/// What a sweep shows its owner: it has engaged, or the button went up over the owner itself.
#[derive(Clone, Copy)]
struct Gives {
    /// The sweep has engaged: a sibling took its value in a pass, or the button went up over the owner.
    engaged: bool,
    /// Where the button went up, when it went up in this pass.
    end: Option<Pos2>,
}

impl Gives {
    /// What `sweep` shows its owner so far.
    fn of(sweep: &Sweep) -> Self {
        Gives { engaged: sweep.engaged, end: sweep.travel.released.then_some(sweep.travel.last).flatten() }
    }

    /// Whether the sweep gives the owner the value at `layer` and `visible`: it has engaged, or the button went up over
    /// the owner, which is a click rather than a sweep. The last pass's layout says nothing here: a sibling the crossing
    /// pass disables, hides, moves or leaves out takes no sweep.
    fn takes(self, ctx: &Context, layer: areas::Layer, visible: Rect) -> bool {
        self.engaged || self.end.is_some_and(|end| crosses_visible(ctx, layer, visible, &[(end, end)]))
    }
}

/// What a pass's records say about the toggle asking.
enum Ready {
    Owner { on: bool, replay: bool, gives: Gives },
    Sibling { on: bool, path: Arc<Vec<Segment>> },
}

struct CompletedSweep {
    sweep: Sweep,
    path: Arc<Vec<Segment>>,
    /// A sibling took the finished sweep's value after its owner asked in that pass, so the owner takes the value in the
    /// next pass: the finished sweep waits for it there.
    owner_late: bool,
}

#[derive(Default)]
struct Record {
    pass: u64,
    sweep: Option<Sweep>,
    path: Arc<Vec<Segment>>,
    // A press and move may arrive together; siblings drawn before the pressed control get that
    // initial path on the following pass. Visited ids keep the edit from applying twice. A sweep released
    // in the pass a sibling after its owner took the value is kept the same way, for the owner (`accept`).
    replay_first_path: bool,
    // The replay survives opening another gesture in the same pass, and one more pass for its owner when a sibling took
    // it after the owner asked.
    completed: Option<CompletedSweep>,
    /// The toggles that asked in this pass: an owner among them read the sweep before a sibling took it, so it takes the
    /// value in the next pass.
    asked: IdSet,
}

impl Stored for Record {}

impl Record {
    fn advance(&mut self, pass: u64, input: &PassPointer, cancelled: bool) {
        if self.pass == pass {
            return;
        }
        self.asked.clear();
        self.end_completed();
        if !self.replay_first_path {
            Arc::make_mut(&mut self.path).clear();
        }
        let replay = std::mem::take(&mut self.replay_first_path);
        let previous = std::mem::replace(&mut self.pass, pass);
        if cancelled || previous.saturating_add(1) != pass {
            self.sweep = None;
            self.completed = None;
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

    /// `sibling` took the running sweep's value, so the sweep crossed another enabled toggle of the owner's group and the
    /// owner takes the value too: only a toggle that takes the sweep in this pass counts as crossed. An owner that asked
    /// in this pass before the sibling gets the value in the next pass, which the caller repaints to run; a sweep released
    /// before then is kept for the owner's check in that pass. Returns whether to repaint for that pass.
    fn accept(&mut self, sibling: Id) -> bool {
        let Some(sweep) = &mut self.sweep else { return false };
        sweep.visited.insert(sibling);
        let late = !sweep.engaged && self.asked.contains(&sweep.owner);
        sweep.engaged = true;
        // A sweep still held keeps its path for the siblings it has yet to reach: only one released in this pass replays it.
        self.replay_first_path |= late && sweep.travel.released;
        late
    }

    /// A finished sweep a sibling took after its owner asked waits one pass for the owner; any other is done with when the
    /// next pass starts.
    fn end_completed(&mut self) {
        let waiting = self.completed.as_mut().is_some_and(|finished| std::mem::take(&mut finished.owner_late));
        if !waiting {
            self.completed = None;
        }
    }

    /// `sibling` took the finished sweep's value. An owner that asked in this pass before it takes the value in the next
    /// pass, so the finished sweep waits for it there. Returns whether to repaint for that pass.
    fn completed_taken(&mut self, sibling: Id) -> bool {
        let Some(finished) = &mut self.completed else { return false };
        finished.sweep.visited.insert(sibling);
        finished.sweep.engaged = true;
        let late = self.asked.contains(&finished.sweep.owner);
        finished.owner_late |= late;
        late
    }

    fn finish(&mut self) {
        self.completed = self.sweep.take().map(|sweep| CompletedSweep { sweep, path: std::mem::take(&mut self.path), owner_late: false });
    }

    fn start(&mut self, owner: Id, group: Id, press: PressStart, on: bool, input: &PassPointer) {
        let mut sweep = Sweep { owner, group, on, travel: Travel::new(press.origin), visited: IdSet::default(), engaged: false };
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
        record.asked.insert(id);
        let finished = record.completed.as_ref().filter(|finished| eligible(&finished.sweep))?;
        let sweep = &finished.sweep;
        if sweep.owner == id {
            // A sweep that finished with the next press never ran its owner's check, so it runs now, over the finished path.
            return Some(Ready::Owner { on: sweep.on, replay: false, gives: Gives::of(sweep) });
        }
        (!sweep.visited.contains(&id)).then(|| Ready::Sibling { on: sweep.on, path: Arc::clone(&finished.path) })
    })
    .and_then(|ready| match ready {
        Ready::Owner { on, gives, .. } => {
            if !gives.takes(ctx, layer, visible) {
                return None;
            }
            PerViewport::<Record>::with(ctx, viewport, |record| {
                if let Some(finished) = record.completed.as_mut().filter(|finished| finished.sweep.owner == id) {
                    finished.sweep.engaged = true;
                }
            });
            Some(on)
        }
        Ready::Sibling { on, path } => {
            if !crosses_visible(ctx, layer, visible, &path) {
                return None;
            }
            if PerViewport::<Record>::with(ctx, viewport, |record| record.completed_taken(id)) {
                ctx.request_repaint();
            }
            Some(on)
        }
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
            return Some(Ready::Owner { on: sweep.on, replay: record.replay_first_path, gives: Gives::of(sweep) });
        }
        (!record.path.is_empty() && !sweep.visited.contains(&id)).then(|| Ready::Sibling { on: sweep.on, path: Arc::clone(&record.path) })
    });
    let Some(ready) = ready else { return completed };
    match ready {
        Ready::Owner { on, replay, gives } => {
            if !gives.takes(ctx, layer, visible) {
                return completed;
            }
            PerViewport::<Record>::with(ctx, viewport, |record| {
                if let Some(sweep) = record.sweep.as_mut().filter(|sweep| sweep.owner == id) {
                    sweep.engaged = true;
                }
            });
            if replay {
                ctx.request_repaint();
            }
            Some(on)
        }
        Ready::Sibling { on, path } => {
            if !crosses_visible(ctx, layer, visible, &path) {
                return completed;
            }
            if PerViewport::<Record>::with(ctx, viewport, |record| record.accept(id)) {
                ctx.request_repaint();
            }
            Some(on)
        }
    }
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
