//! Injected input: pointer moves, clicks, keys and text an automation client asks for, and the event builders
//! injected input and the trials' fixture share.
//!
//! A request appends frames of events to its viewport's queue. Each pass's input hook ([`input_hook`]) appends
//! the next frame, so a move, a press, a release and a key's down and up land on separate passes as real input
//! does, and pass through the same input stages. A step stays in flight until a pass of its viewport has run. A
//! step whose pass has not run is withdrawn when the focus changed since it was injected, and when the application
//! withdraws the viewport's input because its document changed ([`withdraw`]). Screen points are egui points.

use std::collections::{HashMap, VecDeque};

use craft_behavior::PointerButton;
use craft_keymap::{Key, Mods, Platform};
use egui::{Context, Event, Id, Modifiers, Pos2, Vec2, ViewportId};

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

/// `text` as one event in one frame.
pub(crate) fn text_frames(text: &str) -> Vec<Frame> {
    vec![vec![text_event(text)]]
}

/// A wheel turn of `delta` points with `modifiers`, as a mouse wheel reports it.
pub(crate) fn wheel(delta: Vec2, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit: egui::MouseWheelUnit::Point, delta, phase: egui::TouchPhase::Move, modifiers, source: egui::MouseWheelSource::Wheel }
}

/// What injected input holds down in a viewport: the modifiers, buttons and keys, and where the pointer is.
#[derive(Clone, Debug, Default, PartialEq)]
struct Held {
    mods: Modifiers,
    buttons: Vec<egui::PointerButton>,
    keys: Vec<egui::Key>,
    pos: Option<Pos2>,
}

impl Held {
    /// The state after `events`.
    fn apply(&mut self, events: &[Event]) {
        for event in events {
            match event {
                Event::PointerMoved(pos) => self.pos = Some(*pos),
                Event::PointerButton { pos, button, pressed, .. } => {
                    self.pos = Some(*pos);
                    self.buttons.retain(|held| held != button);
                    if *pressed {
                        self.buttons.push(*button);
                    }
                }
                Event::Key { key, pressed, .. } => {
                    self.keys.retain(|held| held != key);
                    if *pressed {
                        self.keys.push(*key);
                    }
                }
                Event::ModifiersChanged(mods) => self.mods = *mods,
                _ => {}
            }
        }
    }

    /// Whether a button or key is down.
    fn holds(&self) -> bool {
        !self.buttons.is_empty() || !self.keys.is_empty()
    }
}

/// One step of injected input: a frame of events, and how long the step stays in flight after its pass (a hover
/// that rests before the reply).
#[derive(Clone, Debug, Default)]
struct Step {
    events: Frame,
    rest: f64,
}

impl From<Frame> for Step {
    fn from(events: Frame) -> Self {
        Self { events, rest: 0.0 }
    }
}

/// The steps of one request, written from what is held before it. A request names the modifiers held during it:
/// they are announced with its first step when they differ from the held ones.
struct Request {
    held: Held,
    steps: Vec<Step>,
}

impl Request {
    fn new(held: Held) -> Self {
        Self { held, steps: Vec::new() }
    }

    /// Adds a step of `events`, announcing `mods` first when they differ from the held ones.
    fn step(&mut self, mods: Modifiers, events: impl IntoIterator<Item = Event>) -> &mut Step {
        let announce = (self.held.mods != mods).then(|| modifiers_changed(mods));
        let events: Frame = announce.into_iter().chain(events).collect();
        self.held.apply(&events);
        self.steps.push(events.into());
        self.steps.last_mut().expect("just pushed")
    }

    /// A move to `at` unless the pointer is there, with `mods` announced.
    fn move_to(&mut self, at: Pos2, mods: Modifiers) {
        if self.held.pos != Some(at) || self.held.mods != mods {
            self.step(mods, [pointer_moved(at)]);
        }
    }

    /// Returns the modifiers to none when the request leaves nothing held.
    fn release_mods_unless_holding(&mut self) {
        if !self.held.holds() && self.held.mods != Modifiers::NONE {
            self.step(Modifiers::NONE, []);
        }
    }
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

/// What a button or key release names: one button, one key, or everything held (the modifiers too).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Released {
    /// One pointer button.
    Button(PointerButton),
    /// One key.
    Key(Key),
    /// Every held button and key, and the held modifiers.
    All,
}

/// A step handed to the input and waiting for its pass.
struct Flight {
    step: Step,
    /// The viewport's pass count when the step was injected.
    pass: u64,
    focus: Option<Id>,
    /// The input time the step's rest ends at, when it rests.
    until: Option<f64>,
    /// What was held before the step: the state again when the step is withdrawn.
    before: Held,
}

#[derive(Default)]
struct Lane {
    queue: VecDeque<Step>,
    flight: Option<Flight>,
    /// The application withdrew the lane's input ([`withdraw`]); the next input hook carries it out.
    withdrawn: bool,
}

/// The queues of injected input, one per viewport that has any, and what injected input holds in each viewport.
#[derive(Default)]
struct Injected {
    lanes: HashMap<ViewportId, Lane>,
    /// What the steps handed to the input hold, per viewport; it outlives the lanes.
    held: HashMap<ViewportId, Held>,
    withdrawn: bool,
    /// The Caps Lock state injected input sets in place of the platform's.
    caps_lock: Option<bool>,
}

impl Injected {
    fn lane(&mut self, viewport: ViewportId) -> &mut Lane {
        self.lanes.entry(viewport).or_default()
    }

    /// What `viewport` holds once its queued steps have run.
    fn projected(&self, viewport: ViewportId) -> Held {
        let mut held = self.held.get(&viewport).cloned().unwrap_or_default();
        for step in self.lanes.get(&viewport).into_iter().flat_map(|lane| &lane.queue) {
            held.apply(&step.events);
        }
        held
    }

    /// Drops the steps in flight whose pass has run and that do not rest, and the lanes left empty. `passes` holds
    /// each lane's viewport pass count. A resting step ends in the input hook, which reads the input time.
    fn settle(&mut self, passes: &[(ViewportId, u64)]) {
        for (viewport, nr) in passes {
            if let Some(lane) = self.lanes.get_mut(viewport)
                && lane.flight.as_ref().is_some_and(|flight| flight.pass < *nr && flight.until.is_none())
            {
                lane.flight = None;
            }
        }
        self.lanes.retain(|_, lane| lane.flight.is_some() || !lane.queue.is_empty());
    }
}

/// Withdraws the step in flight of `viewport`, whose pass has not run, and the queue behind it: the integration's
/// preserved copy of the step leaves `events`, and what was held before the step is held again.
fn withdraw_flight(injected: &mut Injected, viewport: ViewportId, events: &mut Vec<Event>) {
    let Some(lane) = injected.lanes.remove(&viewport) else { return };
    if let Some(flight) = lane.flight {
        remove_event_run(events, &flight.step.events);
        injected.held.insert(viewport, flight.before);
    }
    injected.withdrawn = true;
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
/// whether a pass of the viewport must follow: steps remain queued. A step that rests holds the queue until the
/// input time passes its rest, and asks for the pass that ends it.
pub(crate) fn input_hook(ctx: &Context, input: &mut egui::RawInput) -> bool {
    let viewport = input.viewport_id;
    let (pass_nr, focused) = (pass::pass_nr(ctx, viewport), crate::runtime::focus::record(ctx, viewport).focused);
    let mut wake = None;
    let follow = Service::<Injected>::run(ctx, |injected| {
        let Some(lane) = injected.lanes.get_mut(&viewport) else { return false };
        if lane.withdrawn {
            if lane.flight.as_ref().is_some_and(|flight| flight.pass >= pass_nr) {
                withdraw_flight(injected, viewport, &mut input.events);
                return false;
            }
            lane.withdrawn = false;
            if !lane.queue.is_empty() {
                lane.queue.clear();
                injected.withdrawn = true;
            }
            if lane.flight.is_none() {
                injected.lanes.remove(&viewport);
                return false;
            }
        }
        if let Some(flight) = &lane.flight {
            if flight.pass < pass_nr {
                if let (Some(until), Some(now)) = (flight.until, input.time)
                    && now < until
                {
                    wake = Some(until - now);
                    return false;
                }
                lane.flight = None;
            } else if flight.focus != focused {
                withdraw_flight(injected, viewport, &mut input.events);
                return false;
            } else {
                return !lane.queue.is_empty();
            }
        }
        if let Some(step) = lane.queue.pop_front() {
            input.events.extend(step.events.iter().cloned());
            let until = (step.rest > 0.0).then_some(input.time).flatten().map(|now| now + step.rest);
            let held = injected.held.entry(viewport).or_default();
            let before = held.clone();
            held.apply(&step.events);
            let lane = injected.lanes.get_mut(&viewport).expect("the lane of the popped step");
            lane.flight = Some(Flight { step, pass: pass_nr, focus: focused, until, before });
            return !lane.queue.is_empty();
        }
        false
    });
    if let Some(seconds) = wake {
        repaint::request_after(ctx, viewport, seconds);
    }
    follow
}

/// Queues the steps `request` writes from what `viewport` holds once its queue has run, and asks for a pass of it.
fn queue(ctx: &Context, viewport: ViewportId, request: impl FnOnce(&mut Request)) {
    Service::<Injected>::run(ctx, |injected| {
        let mut writing = Request::new(injected.projected(viewport));
        request(&mut writing);
        injected.lane(viewport).queue.extend(writing.steps);
    });
    repaint::request_of(ctx, viewport);
}

/// The egui modifiers of `mods` on the running platform.
fn platform_modifiers(ctx: &Context, mods: Mods) -> Modifiers {
    egui_modifiers(mods, craft_keymap::egui::platform(ctx))
}

/// Queues a pointer move to `at` with `mods` held; the modifiers stay held after it. A button held by earlier
/// injected input stays down, so the move drags. With `rest` (seconds) the move's step stays in flight until the
/// pointer has rested that long: a hover that rests before the reply.
pub(crate) fn inject_move(ctx: &Context, viewport: ViewportId, at: Pos2, mods: Mods, rest: f64) {
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| request.step(mods, [pointer_moved(at)]).rest = rest.max(0.0));
}

/// Queues a move to `at`, then `count` presses and releases of `button`, one frame each, with `mods` held. With
/// `hold` the last press stays down (and the modifiers held) until a release; otherwise the modifiers are let go
/// after the last release. A button egui has no counterpart for queues nothing.
pub(crate) fn inject_click(ctx: &Context, viewport: ViewportId, at: Pos2, button: PointerButton, count: u32, mods: Mods, hold: bool) {
    let Some(pressed_button) = egui_button(button) else { return };
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| {
        request.move_to(at, mods);
        for nr in 1..=count {
            request.step(mods, [self::button(at, pressed_button, true, mods)]);
            if !(hold && nr == count) {
                request.step(mods, [self::button(at, pressed_button, false, mods)]);
            }
        }
        request.release_mods_unless_holding();
    });
}

/// Queues `key` pressed with `mods` held. Without `hold` it is released in the next step and the modifiers with
/// it; with `hold` both stay down until a release.
pub(crate) fn inject_key(ctx: &Context, viewport: ViewportId, key: Key, mods: Mods, hold: bool) {
    let (key, mods) = (key.to_egui(), platform_modifiers(ctx, mods));
    queue(ctx, viewport, |request| {
        request.step(mods, [key_press(key, mods)]);
        if !hold {
            request.step(mods, [key_release(key, mods)]);
            request.release_mods_unless_holding();
        }
    });
}

/// Queues a move to `at`, then a wheel turn of `delta` points, with `mods` held; the modifiers stay held after it.
pub(crate) fn inject_wheel(ctx: &Context, viewport: ViewportId, at: Pos2, delta: Vec2, mods: Mods) {
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| {
        request.move_to(at, mods);
        request.step(mods, [wheel(delta, mods)]);
    });
}

/// Queues the release of what `released` names, at `at` (a move there first) or where the pointer is. Releasing
/// everything lets the modifiers go too. Nothing held queues nothing.
pub(crate) fn inject_release(ctx: &Context, viewport: ViewportId, at: Option<Pos2>, released: Released) {
    queue(ctx, viewport, |request| {
        let mods = request.held.mods;
        let buttons: Vec<egui::PointerButton> = match released {
            Released::Button(button) => egui_button(button).into_iter().filter(|b| request.held.buttons.contains(b)).collect(),
            Released::All => request.held.buttons.clone(),
            Released::Key(_) => Vec::new(),
        };
        let keys: Vec<egui::Key> = match released {
            Released::Key(key) => Some(key.to_egui()).into_iter().filter(|k| request.held.keys.contains(k)).collect(),
            Released::All => request.held.keys.clone(),
            Released::Button(_) => Vec::new(),
        };
        let at = at.or(request.held.pos);
        if let Some(at) = at
            && !buttons.is_empty()
        {
            request.move_to(at, mods);
        }
        for pressed_button in buttons {
            let pos = at.unwrap_or_default();
            request.step(mods, [self::button(pos, pressed_button, false, mods)]);
        }
        for key in keys {
            request.step(mods, [key_release(key, mods)]);
        }
        if released == Released::All && mods != Modifiers::NONE {
            request.step(Modifiers::NONE, []);
        }
    });
}

/// Queues `text` as typed text.
pub(crate) fn inject_text(ctx: &Context, viewport: ViewportId, text: &str) {
    queue(ctx, viewport, |request| {
        for frame in text_frames(text) {
            let mods = request.held.mods;
            request.step(mods, frame);
        }
    });
}

/// Sets the Caps Lock state injected input holds in place of the platform's; `None` returns to the platform's.
pub(crate) fn set_caps_lock(ctx: &Context, on: Option<bool>) {
    Service::<Injected>::run(ctx, |injected| injected.caps_lock = on);
    ctx.request_repaint();
}

/// The Caps Lock state injected input holds, when it holds one.
pub(crate) fn caps_lock(ctx: &Context) -> Option<bool> {
    Service::<Injected>::run(ctx, |injected| injected.caps_lock)
}

/// Whether the running pass's input carries an injected step: the pass acts for an automation client.
pub(crate) fn in_pass(ctx: &Context) -> bool {
    let viewport = pass::current_viewport(ctx);
    let nr = pass::pass_nr(ctx, viewport);
    Service::<Injected>::run(ctx, |injected| {
        injected.lanes.get(&viewport).and_then(|lane| lane.flight.as_ref()).is_some_and(|flight| flight.pass == nr)
    })
}

/// Withdraws the injected input of `viewport`: the application's document changed. The viewport's next input hook
/// carries it out per step: a step handed to the input whose pass has not run leaves the input with the queue behind
/// it, and what was held before it is held again; after a step whose pass has run, only the queue is dropped.
/// Anything dropped counts as withdrawn ([`take_withdrawn`]). The application withdraws from its raw input hook,
/// which runs right before the framework's, so every request queued before that hook is dropped with the rest.
pub(crate) fn withdraw(ctx: &Context, viewport: ViewportId) {
    Service::<Injected>::run(ctx, |injected| {
        if let Some(lane) = injected.lanes.get_mut(&viewport) {
            lane.withdrawn = true;
        }
    });
}

/// Drops every queue and step in flight; true when anything was pending. What the delivered steps hold stays held
/// until a release.
pub(crate) fn cancel(ctx: &Context) -> bool {
    Service::<Injected>::run(ctx, |injected| {
        let pending = injected.lanes.values().any(|lane| lane.flight.is_some() || !lane.queue.is_empty());
        injected.lanes.clear();
        pending
    })
}

/// Whether nothing is queued and no step awaits its pass or its rest: the application may answer its waiters.
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
