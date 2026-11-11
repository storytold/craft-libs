//! Injected input: pointer moves, clicks, keys and text an automation client asks for, and the event builders
//! injected input and the trials' fixture share.
//!
//! A request appends frames of events to its viewport's queue. Each pass's input hook ([`input_hook`]) appends
//! the next frame, so a move, a press, a release and a key's down and up land on separate passes as real input
//! does, and pass through the same input stages. A step stays in flight until a pass of its viewport has run. A
//! step whose pass has not run is withdrawn when the focus changed since it was injected, and the application
//! withdraws a viewport's queue when its document changed ([`withdraw`]). Screen points are egui points.

use std::collections::{HashMap, VecDeque};

use craft_behavior::PointerButton;
use craft_keymap::{Key, Mods, Platform};
use egui::{Context, Event, Id, Modifiers, Pos2, ViewportId};

use super::memory::Service;
use super::{pass, repaint};

/// One frame's events.
pub(crate) type Frame = Vec<Event>;

/// The pointer moving to `pos`.
pub(crate) fn pointer_moved(pos: Pos2) -> Event {
    Event::PointerMoved(pos)
}

/// `button` going down or up at `pos` with `modifiers`.
pub(crate) fn button(pos: Pos2, button: egui::PointerButton, pressed: bool, modifiers: Modifiers) -> Event {
    Event::PointerButton { pos, button, pressed, modifiers }
}

/// A key press with `modifiers`. Keys carry their `physical_key`, which the shortcut dispatcher's position
/// fallback reads.
pub(crate) fn key_press(key: egui::Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: false, modifiers }
}

/// The auto-repeat of a held `key`.
#[cfg(feature = "testing")]
pub(crate) fn key_repeat(key: egui::Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: true, repeat: true, modifiers }
}

/// A key release.
pub(crate) fn key_release(key: egui::Key, modifiers: Modifiers) -> Event {
    Event::Key { key, physical_key: Some(key), pressed: false, repeat: false, modifiers }
}

/// The modifiers held from this event on.
pub(crate) fn modifiers_changed(modifiers: Modifiers) -> Event {
    Event::ModifiersChanged(modifiers)
}

/// Typed text.
pub(crate) fn text_event(text: &str) -> Event {
    Event::Text(text.to_owned())
}

/// The announcement of `modifiers`, none when no modifier is held.
fn announced(modifiers: Modifiers) -> Option<Event> {
    (modifiers != Modifiers::NONE).then(|| modifiers_changed(modifiers))
}

/// The frame that clears `modifiers`, none when none was held.
fn cleared(modifiers: Modifiers) -> Option<Frame> {
    announced(modifiers).map(|_| vec![modifiers_changed(Modifiers::NONE)])
}

/// A move to `pos` (with `modifiers` announced first), then `count` times a press and a release of `button`, one
/// frame each; `modifiers` are cleared in a last frame when they were set.
pub(crate) fn click_frames(pos: Pos2, pressed_button: egui::PointerButton, count: u32, modifiers: Modifiers) -> Vec<Frame> {
    let mut frames = vec![announced(modifiers).into_iter().chain([pointer_moved(pos)]).collect()];
    for _ in 0..count {
        frames.push(vec![button(pos, pressed_button, true, modifiers)]);
        frames.push(vec![button(pos, pressed_button, false, modifiers)]);
    }
    frames.extend(cleared(modifiers));
    frames
}

/// `key` down with `modifiers` announced in one frame, up (and the modifiers cleared) in the next.
pub(crate) fn key_frames(key: egui::Key, modifiers: Modifiers) -> Vec<Frame> {
    let down = announced(modifiers).into_iter().chain([key_press(key, modifiers)]).collect();
    let up = std::iter::once(key_release(key, modifiers)).chain(cleared(modifiers).into_iter().flatten()).collect();
    vec![down, up]
}

/// `text` as one event in one frame.
pub(crate) fn text_frames(text: &str) -> Vec<Frame> {
    vec![vec![text_event(text)]]
}

/// The egui button for `button`; none for a button egui has no counterpart for.
fn egui_button(button: PointerButton) -> Option<egui::PointerButton> {
    match button {
        PointerButton::Primary => Some(egui::PointerButton::Primary),
        PointerButton::Secondary => Some(egui::PointerButton::Secondary),
        PointerButton::Middle => Some(egui::PointerButton::Middle),
        PointerButton::Extra1 => Some(egui::PointerButton::Extra1),
        PointerButton::Extra2 => Some(egui::PointerButton::Extra2),
        _ => None,
    }
}

/// The modifiers egui reports for `mods` on `platform`, as the integration reports them: on macOS cmd is `mac_cmd`
/// with `command` and ctrl stays the Control key; elsewhere cmd is Ctrl, so `ctrl` with `command`.
pub(super) fn egui_modifiers(mods: Mods, platform: Platform) -> Modifiers {
    if platform.is_mac() {
        Modifiers { alt: mods.alt, ctrl: mods.ctrl, shift: mods.shift, mac_cmd: mods.cmd, command: mods.cmd }
    } else {
        Modifiers { alt: mods.alt, ctrl: mods.cmd, shift: mods.shift, mac_cmd: false, command: mods.cmd }
    }
}

/// A step handed to the input and waiting for its pass.
struct Flight {
    step: Frame,
    /// The viewport's pass count when the step was injected.
    pass: u64,
    focus: Option<Id>,
}

#[derive(Default)]
struct Lane {
    queue: VecDeque<Frame>,
    flight: Option<Flight>,
}

/// The queues of injected input, one per viewport that has any.
#[derive(Default)]
struct Injected {
    lanes: HashMap<ViewportId, Lane>,
    withdrawn: bool,
}

impl Injected {
    fn lane(&mut self, viewport: ViewportId) -> &mut Lane {
        self.lanes.entry(viewport).or_default()
    }

    /// Drops the steps in flight whose pass has run, and the lanes left empty. `passes` holds each lane's
    /// viewport pass count.
    fn settle(&mut self, passes: &[(ViewportId, u64)]) {
        for (viewport, nr) in passes {
            if let Some(lane) = self.lanes.get_mut(viewport)
                && lane.flight.as_ref().is_some_and(|flight| flight.pass < *nr)
            {
                lane.flight = None;
            }
        }
        self.lanes.retain(|_, lane| lane.flight.is_some() || !lane.queue.is_empty());
    }
}

/// Removes one contiguous run equal to `step` from `events` (the integration preserves events of a pass it
/// skipped). A no-op when the run is not present.
fn remove_event_run(events: &mut Vec<Event>, step: &[Event]) {
    if step.is_empty() || step.len() > events.len() {
        return;
    }
    if let Some(at) = events.windows(step.len()).position(|window| window == step) {
        events.drain(at..at + step.len());
    }
}

/// The input hook's first step: appends the next queued step of `input.viewport_id` to `input.events`. Returns
/// whether a pass of the viewport must follow: steps remain queued.
pub(crate) fn input_hook(ctx: &Context, input: &mut egui::RawInput) -> bool {
    let viewport = input.viewport_id;
    let (pass_nr, focused) = (pass::pass_nr(ctx, viewport), crate::runtime::focus::record(ctx, viewport).focused);
    Service::<Injected>::run(ctx, |injected| {
        let Some(lane) = injected.lanes.get_mut(&viewport) else { return false };
        if let Some(flight) = &lane.flight {
            if flight.pass < pass_nr {
                lane.flight = None;
            } else if flight.focus != focused {
                remove_event_run(&mut input.events, &flight.step);
                lane.queue.clear();
                lane.flight = None;
                injected.withdrawn = true;
                injected.lanes.remove(&viewport);
                return false;
            } else {
                return !lane.queue.is_empty();
            }
        }
        if let Some(step) = lane.queue.pop_front() {
            input.events.extend(step.iter().cloned());
            lane.flight = Some(Flight { step, pass: pass_nr, focus: focused });
        }
        !lane.queue.is_empty()
    })
}

/// Queues `frames` for `viewport` and asks for a pass of it.
fn queue(ctx: &Context, viewport: ViewportId, frames: Vec<Frame>) {
    Service::<Injected>::run(ctx, |injected| injected.lane(viewport).queue.extend(frames));
    repaint::request_of(ctx, viewport);
}

/// Queues a pointer move to `at`.
pub(crate) fn inject_move(ctx: &Context, viewport: ViewportId, at: Pos2) {
    queue(ctx, viewport, vec![vec![pointer_moved(at)]]);
}

/// Queues a move to `at`, then `count` presses and releases of `button`, one frame each. A button egui has no
/// counterpart for queues nothing.
pub(crate) fn inject_click(ctx: &Context, viewport: ViewportId, at: Pos2, button: PointerButton, count: u32) {
    if let Some(button) = egui_button(button) {
        queue(ctx, viewport, click_frames(at, button, count, Modifiers::NONE));
    }
}

/// Queues `key` pressed and released, `mods` announced with the press and cleared with the release.
pub(crate) fn inject_key(ctx: &Context, viewport: ViewportId, key: Key, mods: Mods) {
    queue(ctx, viewport, key_frames(key.to_egui(), egui_modifiers(mods, craft_keymap::egui::platform(ctx))));
}

/// Queues `text` as typed text.
pub(crate) fn inject_text(ctx: &Context, viewport: ViewportId, text: &str) {
    queue(ctx, viewport, text_frames(text));
}

/// Drops the queue of `viewport`: the application's document changed. A pending queue counts as withdrawn
/// ([`take_withdrawn`]).
pub(crate) fn withdraw(ctx: &Context, viewport: ViewportId) {
    Service::<Injected>::run(ctx, |injected| {
        if let Some(lane) = injected.lanes.remove(&viewport)
            && (lane.flight.is_some() || !lane.queue.is_empty())
        {
            injected.withdrawn = true;
        }
    });
}

/// Drops every queue and step in flight; true when anything was pending.
pub(crate) fn cancel(ctx: &Context) -> bool {
    Service::<Injected>::run(ctx, |injected| {
        let pending = injected.lanes.values().any(|lane| lane.flight.is_some() || !lane.queue.is_empty());
        injected.lanes.clear();
        pending
    })
}

/// Whether nothing is queued and no step awaits its pass: the application may answer its waiters.
pub(crate) fn idle(ctx: &Context) -> bool {
    let viewports: Vec<ViewportId> = Service::<Injected>::run(ctx, |injected| injected.lanes.keys().copied().collect());
    let passes: Vec<(ViewportId, u64)> = viewports.into_iter().map(|viewport| (viewport, pass::pass_nr(ctx, viewport))).collect();
    Service::<Injected>::run(ctx, |injected| {
        injected.settle(&passes);
        injected.lanes.is_empty()
    })
}

/// Whether a step was withdrawn since the last call.
pub(crate) fn take_withdrawn(ctx: &Context) -> bool {
    Service::<Injected>::run(ctx, |injected| std::mem::take(&mut injected.withdrawn))
}

/// The input of one hook call for `viewport` with no pass behind it: the events the hook appends. Stands in for
/// the integration handing input to a pass it then skips.
#[cfg(feature = "testing")]
pub fn hook_without_pass(ctx: &Context, viewport: ViewportId) -> Vec<Event> {
    let mut input = egui::RawInput { viewport_id: viewport, ..egui::RawInput::default() };
    input_hook(ctx, &mut input);
    input.events
}

/// Moves the focus the end hook recorded for `viewport` to the component `id` with no pass, as the application does
/// between handing input over and its pass.
#[cfg(feature = "testing")]
pub fn move_focus_without_pass(ctx: &Context, viewport: ViewportId, id: Id) {
    crate::runtime::focus::with_viewport(ctx, viewport, |state| state.record.focused = Some(id));
}
