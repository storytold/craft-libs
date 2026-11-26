//! Injected input: pointer moves, clicks, keys and text an automation client asks for, and the event builders
//! injected input and the trials' fixture share.
//!
//! A request appends frames of events to its viewport's queue. Each pass's input hook (`input_hook`) appends
//! the next frame, so a move, a press, a release and a key's down and up land on separate passes as real input
//! does, and pass through the same input stages. A step stays in flight until a pass of its viewport has run. A
//! step whose pass has not run is withdrawn when the focus changed since it was injected, and when the application
//! withdraws the viewport's input because its document changed (`withdraw`). Screen points are egui points.
//!
//! A pointer step can carry the pen's reading ([`InjectedPointer`]): its pass reads it in place of the tablet's
//! (`pen`), so its moves and its press pair with it as with a tablet reading. A held key with no key event
//! (`inject_hold`) is a step of its own: no key stage or shortcut sees it, and only a reader that asks (`holds`)
//! does, from the pass of its hold to the pass of its release (the tool host's `ToolInput::hold`). Such keys end
//! where the lane drops input: a withdrawal, a focus loss, a cancel, the end of the session.

use std::collections::{HashMap, VecDeque};

use craft_behavior::{PointerButton, PointerSample};
use craft_keymap::{Key, Mods, Platform};
use egui::{Context, Event, Id, Modifiers, Pos2, Vec2, ViewportId};

use super::access::{Declared, NodeKey};
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

/// A move to `pos`, then `count` times a press and a release of `button` carrying the held `modifiers`, one frame each.
#[cfg(feature = "testing")]
pub(crate) fn click_frames(pos: Pos2, pressed_button: egui::PointerButton, count: u32, modifiers: Modifiers) -> Vec<Frame> {
    let mut frames = vec![vec![pointer_moved(pos)]];
    for _ in 0..count {
        frames.push(vec![button(pos, pressed_button, true, modifiers)]);
        frames.push(vec![button(pos, pressed_button, false, modifiers)]);
    }
    frames
}

/// `text` as one event in one frame.
pub(crate) fn text_frames(text: &str) -> Vec<Frame> {
    vec![vec![text_event(text)]]
}

/// A wheel turn of `delta` in `unit` with `modifiers`, as a mouse wheel reports it.
pub(crate) fn wheel(unit: egui::MouseWheelUnit, delta: Vec2, modifiers: Modifiers) -> Event {
    Event::MouseWheel { unit, delta, phase: egui::TouchPhase::Move, modifiers, source: egui::MouseWheelSource::Wheel }
}

/// What injected input holds down in a viewport: the modifiers, buttons and keys, where the pointer is, and the keys
/// held with no key event.
#[derive(Clone, Debug, Default, PartialEq)]
struct Held {
    mods: Modifiers,
    buttons: Vec<egui::PointerButton>,
    keys: Vec<egui::Key>,
    pos: Option<Pos2>,
    /// The keys [`inject_hold`] holds: no event carries them.
    silent: Vec<Key>,
}

impl Held {
    /// The state after `step`.
    fn apply(&mut self, step: &Step) {
        match step.hold {
            Some(Hold::Down(key)) if !self.silent.contains(&key) => self.silent.push(key),
            Some(Hold::Up(key)) => self.silent.retain(|held| *held != key),
            _ => {}
        }
        for event in &step.events {
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

/// A key a step holds down or lets go of with no key event.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Hold {
    Down(Key),
    Up(Key),
}

/// One step of injected input: a frame of events, how long the step stays in flight after its pass (a hover that
/// rests before the reply), the pen's reading its pass reads, and the key it holds or lets go of with no event.
#[derive(Clone, Debug, Default)]
struct Step {
    events: Frame,
    rest: f64,
    pen: Option<PointerSample>,
    hold: Option<Hold>,
    /// Files the step hovers over the window, or drops.
    pub files: Option<InjectedFiles>,
    /// The serial of the request that queued the step ([`Injected::requests`]).
    request: u64,
}

/// Files an injected step hovers over a window, and whether the step after it drops them.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct InjectedFiles {
    /// The paths of the files.
    pub paths: Vec<std::path::PathBuf>,
    /// Whether the files are dropped in the following injected step.
    pub dropped: bool,
}

impl From<Frame> for Step {
    fn from(events: Frame) -> Self {
        Self { events, ..Self::default() }
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
        let step = Step::from(events);
        self.held.apply(&step);
        let at = self.steps.len();
        self.steps.push(step);
        &mut self.steps[at]
    }

    /// Adds a step with no event that holds `hold`'s key down or lets it go.
    fn hold(&mut self, hold: Hold) {
        let step = Step { hold: Some(hold), ..Step::default() };
        self.held.apply(&step);
        self.steps.push(step);
    }

    /// A move to `at` unless the pointer is there, with `mods` announced and the pen's reading `pen`.
    fn move_to(&mut self, at: Pos2, mods: Modifiers, pen: Option<PointerSample>) {
        if self.held.pos != Some(at) || self.held.mods != mods {
            self.step(mods, [pointer_moved(at)]).pen = pen;
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
    /// Every held button and key (also those held with no key event), and the held modifiers.
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
    /// The frame whose first pass took the latest step: later passes of that frame take none.
    entered: Option<u64>,
    queue: VecDeque<Step>,
    flight: Option<Flight>,
    /// The request of the latest step handed to the input: a withdrawal keeps the rest of it once that step's pass ran.
    running: Option<u64>,
    /// The application withdrew the lane's input ([`withdraw`]); the next input hook carries it out.
    withdrawn: bool,
}

/// The queues of injected input, one per viewport that has any, and what injected input holds in each viewport.
#[derive(Default)]
struct Injected {
    lanes: HashMap<ViewportId, Lane>,
    /// What the steps handed to the input hold, per viewport; it outlives the lanes.
    held: HashMap<ViewportId, Held>,
    /// How many requests were queued: the serial of the latest.
    requests: u64,
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
            held.apply(step);
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
/// preserved copy of the step leaves `events`, and what was held before the step is held again, except the keys held
/// with no event, whose releases went with the queue.
fn withdraw_flight(injected: &mut Injected, viewport: ViewportId, events: &mut Vec<Event>) {
    let Some(lane) = injected.lanes.remove(&viewport) else { return };
    if let Some(flight) = lane.flight {
        remove_event_run(events, &flight.step.events);
        injected.held.insert(viewport, flight.before);
    }
    if let Some(held) = injected.held.get_mut(&viewport) {
        held.silent.clear();
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
/// input time passes its rest, and asks for the pass that ends it. A window focus loss in the input ends what injected
/// input does there: egui forgets held buttons, keys and modifiers at a focus loss, so the step in flight is withdrawn,
/// the queue dropped and nothing counts as held.
pub(crate) fn input_hook(ctx: &Context, input: &mut egui::RawInput) -> bool {
    let viewport = input.viewport_id;
    let (pass_nr, focused) = (pass::pass_nr(ctx, viewport), crate::runtime::focus::record(ctx, viewport).focused);
    let frame = pass::frame_nr(ctx, viewport);
    let blurred = input.events.iter().any(|event| matches!(event, Event::WindowFocused(false)));
    let held = crate::runtime::frame::held_for(ctx, viewport, frame);
    let mut wake = None;
    let follow = Service::<Injected>::run(ctx, |injected| {
        if blurred {
            let unrun = injected.lanes.get(&viewport).and_then(|lane| lane.flight.as_ref()).is_some_and(|flight| flight.pass >= pass_nr);
            if unrun {
                withdraw_flight(injected, viewport, &mut input.events);
            } else if injected.lanes.remove(&viewport).is_some_and(|lane| !lane.queue.is_empty()) {
                injected.withdrawn = true;
            }
            injected.held.remove(&viewport);
            return false;
        }
        let Some(lane) = injected.lanes.get_mut(&viewport) else { return false };
        if lane.withdrawn {
            if lane.flight.as_ref().is_some_and(|flight| flight.pass >= pass_nr) {
                withdraw_flight(injected, viewport, &mut input.events);
                return false;
            }
            lane.withdrawn = false;
            // The request whose step ran keeps its rest (a key's own release); the requests behind it go.
            let running = lane.running;
            let queued = lane.queue.len();
            lane.queue.retain(|step| Some(step.request) == running);
            if lane.queue.len() != queued {
                injected.withdrawn = true;
            }
            if lane.flight.is_none() && lane.queue.is_empty() {
                injected.lanes.remove(&viewport);
                return false;
            }
        }
        if let Some(flight) = &mut lane.flight {
            if flight.pass < pass_nr {
                if let (Some(until), Some(now)) = (flight.until, input.time)
                    && now < until
                {
                    wake = Some(until - now);
                    return false;
                }
                // Events of the step that the frame it entered held back (behind a real press there, H0) arrive in this
                // pass: the step's pass is this one too, so it acts for the client, and the next step waits for it.
                if flight.step.events.iter().any(|event| held.contains(event)) {
                    flight.pass = pass_nr;
                    return !lane.queue.is_empty();
                }
                lane.flight = None;
            } else if flight.focus != focused {
                withdraw_flight(injected, viewport, &mut input.events);
                return false;
            } else {
                return !lane.queue.is_empty();
            }
        }
        if lane.entered == Some(frame) {
            // A later pass of the frame whose first pass took a step takes none: input enters a frame's first pass
            // only. The step in flight still ends, or is withdrawn, as above.
            return !lane.queue.is_empty();
        }
        if let Some(step) = lane.queue.pop_front() {
            input.events.extend(step.events.iter().cloned());
            if let Some(files) = &step.files {
                give_files(input, files);
            }
            let until = (step.rest > 0.0).then_some(input.time).flatten().map(|now| now + step.rest);
            let held = injected.held.entry(viewport).or_default();
            let before = held.clone();
            held.apply(&step);
            lane.running = Some(step.request);
            lane.flight = Some(Flight { step, pass: pass_nr, focus: focused, until, before });
            lane.entered = Some(frame);
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
        injected.requests = injected.requests.wrapping_add(1);
        let serial = injected.requests;
        injected.lane(viewport).queue.extend(writing.steps.into_iter().map(|step| Step { request: serial, ..step }));
    });
    repaint::request_of(ctx, viewport);
}

/// The egui modifiers of `mods` on the running platform.
fn platform_modifiers(ctx: &Context, mods: Mods) -> Modifiers {
    egui_modifiers(mods, craft_keymap::egui::platform(ctx))
}

/// `at` when it is a point a window could report: a window never delivers a NaN or infinite pointer, so injected
/// input never passes one to egui.
fn finite(at: Pos2) -> Option<Pos2> {
    at.is_finite().then_some(at)
}

/// Queues a pointer move to `at` with `mods` held; the modifiers stay held after it. A button held by earlier
/// injected input stays down, so the move drags. With `rest` (seconds) the move's step stays in flight until the
/// pointer has rested that long: a hover that rests before the reply. A non-finite `at` queues nothing.
pub(crate) fn inject_move(ctx: &Context, viewport: ViewportId, at: Pos2, mods: Mods, rest: f64) {
    let Some(at) = finite(at) else { return };
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| request.step(mods, [pointer_moved(at)]).rest = rest.max(0.0));
}

/// Queues a move to `at`, then `count` presses and releases of `button`, one frame each, with `mods` held. With
/// `hold` the last press stays down (and the modifiers held) until a release; otherwise the modifiers are let go
/// after the last release. A button egui has no counterpart, or a non-finite `at`, queues nothing.
pub(crate) fn inject_click(ctx: &Context, viewport: ViewportId, at: Pos2, button: PointerButton, count: u32, mods: Mods, hold: bool) {
    let (Some(pressed_button), Some(at)) = (egui_button(button), finite(at)) else { return };
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| {
        request.move_to(at, mods, None);
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

/// Queues a move to `at`, then a wheel turn of `delta` in `unit`, with `mods` held; the modifiers stay held after it. A
/// non-finite `at` or `delta` queues nothing.
pub(crate) fn inject_wheel(ctx: &Context, viewport: ViewportId, at: Pos2, delta: Vec2, unit: egui::MouseWheelUnit, mods: Mods) {
    let Some(at) = finite(at).filter(|_| delta.is_finite()) else { return };
    let mods = platform_modifiers(ctx, mods);
    queue(ctx, viewport, |request| {
        request.move_to(at, mods, None);
        request.step(mods, [wheel(unit, delta, mods)]);
    });
}

/// Whether injected input holds `button` down in `viewport`, from the step that pressed it to the one that releases it
/// (between steps no event carries the hold).
pub(crate) fn held_button(ctx: &Context, viewport: ViewportId, button: egui::PointerButton) -> bool {
    Service::<Injected>::run(ctx, |injected| injected.held.get(&viewport).is_some_and(|held| held.buttons.contains(&button)))
}

/// Queues the release of what `released` names, at `at` (a move there first) or where the pointer is. Releasing
/// everything lets the modifiers go too. Nothing held queues nothing. A non-finite `at` releases where the pointer is.
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
        let at = at.and_then(finite).or(request.held.pos);
        if let Some(at) = at
            && !buttons.is_empty()
        {
            request.move_to(at, mods, None);
        }
        for pressed_button in buttons {
            let pos = at.unwrap_or_default();
            request.step(mods, [self::button(pos, pressed_button, false, mods)]);
        }
        for key in keys {
            request.step(mods, [key_release(key, mods)]);
        }
        if released == Released::All {
            for key in request.held.silent.clone() {
                request.hold(Hold::Up(key));
            }
            if mods != Modifiers::NONE {
                request.step(Modifiers::NONE, []);
            }
        }
    });
}

/// One pointer step an automation client drives as a device would, with the modifiers held and, for a pen, its
/// reading, which the step's pass reads in place of the tablet's.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum InjectedPointer {
    /// `button` goes down at `at`, after a move there when the injected pointer is elsewhere. A button injected input
    /// holds already queues nothing.
    #[non_exhaustive]
    Down {
        /// The button.
        button: PointerButton,
        /// The pointer.
        at: Pos2,
        /// The modifiers held.
        mods: Mods,
        /// The pen's reading; `None` for a mouse.
        pen: Option<PointerSample>,
    },
    /// The pointer moves to `at`; a button held by injected input drags.
    #[non_exhaustive]
    Move {
        /// The pointer.
        at: Pos2,
        /// The modifiers held.
        mods: Mods,
        /// The pen's reading; `None` for a mouse.
        pen: Option<PointerSample>,
        /// Seconds the pointer must rest at `at` before the step counts as processed; 0 for none.
        rest: f64,
    },
    /// Every button injected input holds comes up at `at`, after a move there; the modifiers go with them when no
    /// button or key stays down. Nothing held queues nothing.
    #[non_exhaustive]
    Up {
        /// The pointer.
        at: Pos2,
        /// The modifiers held.
        mods: Mods,
    },
}

impl InjectedPointer {
    /// `button` goes down at `at` with `mods` held and the pen's reading `pen`.
    pub const fn down(button: PointerButton, at: Pos2, mods: Mods, pen: Option<PointerSample>) -> Self {
        InjectedPointer::Down { button, at, mods, pen }
    }

    /// The pointer moves to `at` with `mods` held and the pen's reading `pen`.
    pub const fn move_to(at: Pos2, mods: Mods, pen: Option<PointerSample>) -> Self {
        InjectedPointer::Move { at, mods, pen, rest: 0.0 }
    }

    /// The pointer moves to `at` with `mods` held and the pen's reading `pen`, and the step counts as processed once
    /// the pointer has rested there `rest` seconds.
    pub const fn move_resting(at: Pos2, mods: Mods, pen: Option<PointerSample>, rest: f64) -> Self {
        InjectedPointer::Move { at, mods, pen, rest }
    }

    /// Every injected button comes up at `at` with `mods` held.
    pub const fn up(at: Pos2, mods: Mods) -> Self {
        InjectedPointer::Up { at, mods }
    }
}

/// Queues `step` for `viewport`, one pass per event. A button egui has no counterpart, or a non-finite point of a
/// press or move, queues nothing; a release at a non-finite point releases where the pointer is.
pub(crate) fn inject_pointer(ctx: &Context, viewport: ViewportId, step: InjectedPointer) {
    match step {
        InjectedPointer::Down { button, at, mods, pen } => {
            let (Some(pressed_button), Some(at)) = (egui_button(button), finite(at)) else { return };
            let mods = platform_modifiers(ctx, mods);
            queue(ctx, viewport, |request| {
                // A window never sends a press of a button that is down already.
                if request.held.buttons.contains(&pressed_button) {
                    return;
                }
                request.move_to(at, mods, pen);
                request.step(mods, [self::button(at, pressed_button, true, mods)]).pen = pen;
            });
        }
        InjectedPointer::Move { at, mods, pen, rest } => {
            let Some(at) = finite(at) else { return };
            let mods = platform_modifiers(ctx, mods);
            queue(ctx, viewport, |request| {
                let step = request.step(mods, [pointer_moved(at)]);
                step.pen = pen;
                step.rest = rest.max(0.0);
            });
        }
        InjectedPointer::Up { at, mods } => {
            let mods = platform_modifiers(ctx, mods);
            queue(ctx, viewport, |request| {
                let buttons = request.held.buttons.clone();
                if buttons.is_empty() {
                    return;
                }
                let at = finite(at).or(request.held.pos);
                if let Some(at) = at {
                    request.move_to(at, mods, None);
                }
                for pressed_button in buttons {
                    request.step(mods, [self::button(at.unwrap_or_default(), pressed_button, false, mods)]);
                }
                request.release_mods_unless_holding();
            });
        }
    }
}

/// Queues a step with no event that holds `key` down (`down`) or lets it go: only [`holds`] reads it. A key held
/// already, or one let go that is not held, queues nothing.
pub(crate) fn inject_hold(ctx: &Context, viewport: ViewportId, key: Key, down: bool) {
    queue(ctx, viewport, |request| {
        if request.held.silent.contains(&key) != down {
            request.hold(if down { Hold::Down(key) } else { Hold::Up(key) });
        }
    });
}

/// The keys injected input holds in `viewport` with no key event ([`inject_hold`]), as of its running pass.
pub(crate) fn holds(ctx: &Context, viewport: ViewportId) -> Vec<Key> {
    Service::<Injected>::run(ctx, |injected| injected.held.get(&viewport).map(|held| held.silent.clone()).unwrap_or_default())
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

/// Queues the hover of `paths` over `at` (a move there first; with no `at` the pointer stays) and, with `drop`, their
/// drop in the following step. No path, or a non-finite `at`, queues nothing.
pub(crate) fn inject_files(ctx: &Context, viewport: ViewportId, at: Option<Pos2>, paths: Vec<std::path::PathBuf>, drop: bool) {
    if paths.is_empty() || at.is_some_and(|at| finite(at).is_none()) {
        return;
    }
    queue(ctx, viewport, |request| {
        let mods = request.held.mods;
        if let Some(at) = at {
            request.move_to(at, mods, None);
        }
        request.step(mods, []).files = Some(InjectedFiles { paths: paths.clone(), dropped: false });
        if drop {
            request.step(mods, []).files = Some(InjectedFiles { paths, dropped: true });
        }
    });
}

/// Gives `input` the files of a step through the input a window's file drag fills: hovered files, or dropped ones,
/// which the window's drag session reads as from the OS.
fn give_files(input: &mut egui::RawInput, files: &InjectedFiles) {
    if files.dropped {
        input.hovered_files.clear();
        input
            .dropped_files
            .extend(files.paths.iter().map(|path| std::sync::Arc::new(super::files::DroppedPath(path.clone())) as egui::DroppedFileHandle));
    } else {
        input.hovered_files = files.paths.iter().map(|path| egui::HoveredFile { path: Some(path.clone()), mime: String::new() }).collect();
    }
}

/// Queues one AccessKit action request on `node` as one step, behind earlier injected input: a click, a focus or a
/// context menu request, the actions that carry no data. Any other action queues nothing.
pub(crate) fn inject_access(ctx: &Context, viewport: ViewportId, node: NodeKey, action: Declared) {
    if !matches!(action, Declared::Click | Declared::Focus | Declared::ContextMenu) {
        return;
    }
    let request = accesskit::ActionRequest {
        action: action.action(),
        target_tree: accesskit::TreeId::ROOT,
        target_node: accesskit::NodeId(node.raw()),
        data: None,
    };
    queue(ctx, viewport, move |writing| {
        let mods = writing.held.mods;
        writing.step(mods, [Event::AccessKitActionRequest(request)]);
    });
}

/// Queues one IME event (a preedit or a commit) with the held modifiers, behind earlier injected input.
pub(crate) fn inject_ime(ctx: &Context, viewport: ViewportId, event: egui::ImeEvent) {
    queue(ctx, viewport, move |request| {
        let mods = request.held.mods;
        request.step(mods, [Event::Ime(event)]);
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

/// What `f` reads of the step the running pass's input carries, when it carries one.
fn with_running<R>(ctx: &Context, f: impl FnOnce(Option<&Step>) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let nr = pass::pass_nr(ctx, viewport);
    Service::<Injected>::run(ctx, |injected| {
        f(injected.lanes.get(&viewport).and_then(|lane| lane.flight.as_ref()).filter(|flight| flight.pass == nr).map(|flight| &flight.step))
    })
}

/// Whether the running pass's input carries an injected step: the pass acts for an automation client.
pub(crate) fn in_pass(ctx: &Context) -> bool {
    with_running(ctx, |step| step.is_some())
}

/// The pen's reading the running pass's injected step carries: the pass reads it in place of the tablet's.
pub(crate) fn pen(ctx: &Context) -> Option<PointerSample> {
    with_running(ctx, |step| step.and_then(|step| step.pen))
}

/// Withdraws the injected input of `viewport`: the application's document changed. The keys held with no event end at
/// once. The viewport's next input hook carries out the rest per step: a step handed to the input whose pass has not
/// run leaves the input with the queue behind it, and what was held before it is held again; after a step whose pass
/// has run, the rest of that step's request still runs (a key press's own release, when the press opened a dialog) and
/// the requests queued behind it are dropped. Anything dropped counts as withdrawn ([`take_withdrawn`]). The application
/// withdraws from its raw input hook, which runs right before the framework's, so every request queued before that
/// hook is dropped with the rest.
pub(crate) fn withdraw(ctx: &Context, viewport: ViewportId) {
    Service::<Injected>::run(ctx, |injected| {
        if let Some(lane) = injected.lanes.get_mut(&viewport) {
            lane.withdrawn = true;
        }
        if let Some(held) = injected.held.get_mut(&viewport) {
            held.silent.clear();
        }
    });
}

/// Drops every queue and step in flight, and the keys held with no event; true when anything was pending. The buttons
/// and keys the delivered steps hold stay held until a release.
pub(crate) fn cancel(ctx: &Context) -> bool {
    Service::<Injected>::run(ctx, |injected| {
        let pending = injected.lanes.values().any(|lane| lane.flight.is_some() || !lane.queue.is_empty());
        injected.lanes.clear();
        for held in injected.held.values_mut() {
            held.silent.clear();
        }
        pending
    })
}

/// Ends the injection session of the context: drops every queue and step in flight ([`cancel`]), queues the release of
/// everything held in each viewport where injected input holds a button, a key or a modifier, and returns Caps Lock to
/// the platform's. The releases run on the viewports' next passes; `cancel` alone leaves the delivered steps' keys
/// held.
pub(crate) fn end(ctx: &Context) {
    cancel(ctx);
    let holding: Vec<ViewportId> = Service::<Injected>::run(ctx, |injected| {
        injected.held.iter().filter(|(_, held)| held.holds() || held.mods != Modifiers::NONE).map(|(viewport, _)| *viewport).collect()
    });
    for viewport in holding {
        inject_release(ctx, viewport, None, Released::All);
    }
    set_caps_lock(ctx, None);
}

/// Whether nothing is queued and no step awaits its pass or its rest: the application may answer its waiters.
pub(crate) fn idle(ctx: &Context) -> bool {
    settled(ctx, None)
}

/// [`idle`] asked at the end of the running pass, after its UI: the step that pass carried counts as run, so the
/// reply goes out with the pass that ran it and not one pass later. A pass egui will run again (`request_discard`)
/// does not count: the pass that runs again draws what the step did. Outside a pass it is [`idle`].
pub(crate) fn idle_at_pass_end(ctx: &Context) -> bool {
    let ending = crate::runtime::frame::running(ctx).filter(|_| !ctx.will_discard()).map(|facts| facts.viewport);
    settled(ctx, ending)
}

/// Settles the steps whose pass has run, `ending`'s running pass included, and says whether nothing is left.
fn settled(ctx: &Context, ending: Option<ViewportId>) -> bool {
    let viewports: Vec<ViewportId> = Service::<Injected>::run(ctx, |injected| injected.lanes.keys().copied().collect());
    let passes: Vec<(ViewportId, u64)> =
        viewports.into_iter().map(|viewport| (viewport, pass::pass_nr(ctx, viewport).saturating_add(u64::from(ending == Some(viewport))))).collect();
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
