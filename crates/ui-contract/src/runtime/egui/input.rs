//! Raw input in the input hook, key events converted to craft-keymap's key types, `InputOptions`.

use super::control_click::ControlClick;
use crate::runtime::frame::PerViewport;
use crate::runtime::pen::PenPass;
use craft_behavior::one_press::split_after_first_press;
use craft_behavior::{Buttons, PointerButton, PointerEvent, PointerKind, PointerSample, WheelPhase, WheelSource, WheelUnit};
use craft_host::target::Os;
use craft_keymap::{KeyInput, Mods};
use egui::{Context, Event, Id, ImeEvent, Key, MouseWheelSource, MouseWheelUnit, Pos2, TouchPhase, Vec2, ViewportId};

pub use egui::RawInput;

/// An input event with the input time it arrived at: `None` for an event of the running frame's own input, which
/// arrived at the frame's time (the begin hook reads it, `HeldInput::stamp`); an event an earlier frame held keeps the
/// time of the frame it arrived in. The input hook's stages from the one-press rule (H0) to the key stage (H3) keep,
/// remove and hold these records, and egui's event list is built from the records they leave (`into_input`).
#[derive(Clone)]
pub(crate) struct Arrived {
    pub(crate) event: Event,
    pub(crate) at: Option<f64>,
}

impl Arrived {
    /// Takes the events out of `input`: they arrived in the running frame.
    pub(crate) fn take(input: &mut RawInput) -> Vec<Arrived> {
        input.events.drain(..).map(|event| Arrived { event, at: None }).collect()
    }

    /// Makes `events` egui's input events.
    pub(crate) fn into_input(events: Vec<Arrived>, input: &mut RawInput) {
        input.events = events.into_iter().map(|arrived| arrived.event).collect();
    }
}

/// The events that wait for the next frame, for one viewport: the one-press rule's and those the key stage cuts at a
/// focus move (`keys::input_hook`), each with the time it arrived at.
#[derive(Default)]
pub(crate) struct HeldInput {
    events: Vec<Arrived>,
    frame: Option<u64>,
}

impl HeldInput {
    /// Holds `events`, which a later step of this pass's input hook cut, for the next frame, ahead of the events held
    /// already: they arrived before them.
    pub(crate) fn hold_ahead(&mut self, mut events: Vec<Arrived>) {
        events.append(&mut self.events);
        self.events = events;
    }

    /// Gives the events this frame's input hook held the frame's input time `now`, which the begin hook reads.
    pub(crate) fn stamp(&mut self, now: f64) {
        for arrived in &mut self.events {
            if arrived.at.is_none() {
                arrived.at = Some(now);
            }
        }
    }

    /// Whether events wait for the next frame.
    #[cfg(feature = "testing")]
    pub(crate) fn holds(&self) -> bool {
        !self.events.is_empty()
    }
}

/// H0: this pass carries at most one pointer press, as its last pointer event. Events after the first press
/// wait in `held` and lead the next frame's events. A repeated pass of the same frame, a discard pass,
/// receives no input events, so it gets none of the held ones. Returns whether events were held, so the hook
/// asks for a repaint.
pub(crate) fn hold_back_presses(events: &mut Vec<Arrived>, frame: u64, held: &mut HeldInput) -> bool {
    if held.frame == Some(frame) {
        return false;
    }
    held.frame = Some(frame);
    let mut all = std::mem::take(&mut held.events);
    all.append(events);
    let (now, later) = split_after_first_press(all, |arrived| matches!(arrived.event, Event::PointerButton { pressed: true, .. }));
    *events = now;
    held.events = later;
    !held.events.is_empty()
}

/// Takes key presses and texts out of the running pass's input. `places` and `texts` are the places the key stage
/// recorded as it left them in the input (`in_egui`): a press's among the input's key presses, a text's among its text
/// events. Each press goes with its release when that is in the input too; otherwise its key is no longer down.
/// Returns, for each press place, whether the release is still to come.
pub(crate) fn withdraw(ctx: &Context, places: &[usize], texts: &[usize]) -> Vec<bool> {
    ctx.input_mut(|input| {
        // The index in the input of the event at each recorded place of its kind.
        let at_places = |kind: fn(&Event) -> bool, wanted: &[usize]| -> Vec<usize> {
            let of_kind: Vec<usize> = input.events.iter().enumerate().filter(|(_, event)| kind(event)).map(|(at, _)| at).collect();
            wanted.iter().filter_map(|place| of_kind.get(*place).copied()).collect()
        };
        let presses = at_places(|event| matches!(event, Event::Key { pressed: true, .. }), places);
        let texts = at_places(|event| matches!(event, Event::Text(_)), texts);
        let mut gone = vec![false; input.events.len()];
        for at in texts {
            gone[at] = true;
        }
        let mut open = Vec::with_capacity(presses.len());
        for at in presses {
            let Event::Key { key, .. } = input.events[at] else { continue };
            gone[at] = true;
            let release = input.events[at + 1..]
                .iter()
                .position(|event| matches!(event, Event::Key { key: up, pressed: false, .. } if *up == key))
                .map(|after| at + 1 + after);
            match release {
                Some(release) => gone[release] = true,
                None => _ = input.keys_down.remove(&key),
            }
            open.push(release.is_none());
        }
        let mut gone = gone.into_iter();
        input.events.retain(|_| !gone.next().unwrap_or(false));
        open
    })
}

/// Asks for an immediate repaint of `viewport`, from the input hook where the viewport is not yet current.
pub(crate) fn request_repaint_of(ctx: &Context, viewport: ViewportId) {
    ctx.request_repaint_of(viewport);
}

/// The options the runtime owns at install: interface scale and quit are commands, so egui's own shortcuts go.
pub(crate) fn clear_shortcuts(ctx: &Context) {
    ctx.options_mut(|options| {
        options.zoom_with_keyboard = false;
        options.quit_shortcuts.clear();
    });
}

/// The modifiers the app's wheel zooms and scrolls sideways with, as egui's input options.
pub(crate) fn set_wheel_modifiers(ctx: &Context, zoom: egui::Modifiers, sideways: egui::Modifiers) {
    ctx.options_mut(|options| {
        options.input_options.zoom_modifier = zoom;
        options.input_options.horizontal_scroll_modifier = sideways;
    });
}

/// Stops the pass's egui drag, so hover reaches other widgets again.
pub(crate) fn stop_dragging(ctx: &Context) {
    ctx.stop_dragging();
}

/// One key event as the key stage reads it.
pub(crate) enum KeyEvent {
    /// A key press, repeats included: the key as typed and everything the layout matcher reads.
    Press { key: craft_keymap::Key, input: KeyInput, repeat: bool },
    /// A key release.
    Release { input: KeyInput },
    /// Typed text.
    Text(String),
    /// egui's own clipboard events; a paste carries its text.
    Clip { command: craft_behavior::EditCommand, text: Option<String> },
    /// A pointer press, which ends keyboard modality.
    PointerPress,
    /// An AccessKit action request.
    Access(Action),
    /// The held modifiers changed.
    Modifiers(craft_keymap::Mods),
    /// The right Alt key (AltGr on layouts that have it) went down or up, or the window lost focus (up).
    RightAlt(bool),
    /// Anything else.
    Other,
}

/// The key stage's reading of `event`. `latin` is the raw input's Latin-layout characters.
pub(crate) fn key_event(event: &Event, os: Os, latin: &[(egui::Key, char)]) -> KeyEvent {
    let read = |event: &Event| {
        let mut input = KeyInput::from_egui(event, os)?;
        if let Event::Key { physical_key: Some(physical), .. } = event {
            input.latin = latin.iter().find(|(key, _)| key == physical).map(|(_, c)| *c);
        }
        Some(input)
    };
    match event {
        Event::Key { physical_key: Some(egui::Key::AltRight), pressed, .. } => KeyEvent::RightAlt(*pressed),
        Event::WindowFocused(false) => KeyEvent::RightAlt(false),
        Event::Key { pressed: true, repeat, .. } => match read(event) {
            Some(input) => input.typed.map_or(KeyEvent::Other, |key| KeyEvent::Press { key, input, repeat: *repeat }),
            None => KeyEvent::Other,
        },
        Event::Key { pressed: false, .. } => read(event).map_or(KeyEvent::Other, |input| KeyEvent::Release { input }),
        Event::Text(text) => KeyEvent::Text(text.clone()),
        Event::ModifiersChanged(modifiers) => KeyEvent::Modifiers(Mods::from_egui(modifiers, os)),
        Event::Copy => KeyEvent::Clip { command: craft_behavior::EditCommand::Copy, text: None },
        Event::Cut => KeyEvent::Clip { command: craft_behavior::EditCommand::Cut, text: None },
        Event::Paste(text) => KeyEvent::Clip { command: craft_behavior::EditCommand::Paste, text: Some(text.clone()) },
        Event::PointerButton { pressed: true, .. } => KeyEvent::PointerPress,
        Event::AccessKitActionRequest(request) => KeyEvent::Access(Action(request.clone())),
        _ => KeyEvent::Other,
    }
}

/// An AccessKit action request, read without naming AccessKit.
pub(crate) struct Action(accesskit::ActionRequest);

/// What an action request asks for.
pub(crate) enum ActionKind {
    Focus,
    Click,
    Increment,
    Decrement,
    Expand,
    Collapse,
    ShowContextMenu,
    SetValue(f64),
    Other,
}

impl Action {
    /// The node the request targets, as `node_key` numbers nodes.
    pub(crate) fn node(&self) -> u64 {
        self.0.target_node.0
    }

    pub(crate) fn kind(&self) -> ActionKind {
        match (self.0.action, &self.0.data) {
            (accesskit::Action::Focus, _) => ActionKind::Focus,
            (accesskit::Action::Click, _) => ActionKind::Click,
            (accesskit::Action::Increment, _) => ActionKind::Increment,
            (accesskit::Action::Decrement, _) => ActionKind::Decrement,
            (accesskit::Action::Expand, _) => ActionKind::Expand,
            (accesskit::Action::Collapse, _) => ActionKind::Collapse,
            (accesskit::Action::ShowContextMenu, _) => ActionKind::ShowContextMenu,
            (accesskit::Action::SetValue, Some(accesskit::ActionData::NumericValue(value))) => ActionKind::SetValue(*value),
            _ => ActionKind::Other,
        }
    }

    /// Whether the request asks a node to scroll or to be scrolled into view: it moves no focus.
    pub(crate) fn scrolls(&self) -> bool {
        use accesskit::Action as A;
        matches!(self.0.action, A::ScrollIntoView | A::ScrollUp | A::ScrollDown | A::ScrollLeft | A::ScrollRight | A::SetScrollOffset)
    }

    /// The request as an `Intent::Access` payload for the sub-node `part`.
    pub(crate) fn request(&self, part: Option<craft_behavior::PartRef>) -> craft_behavior::AccessRequest {
        craft_behavior::AccessRequest::new(self.0.action, part, self.0.data.clone())
    }
}

/// What the running pass's pointer input says, converted once.
pub(crate) struct PointerRead {
    /// The primary, middle and secondary button events in arrival order. `clicks` of each press is 1: the caller counts.
    pub events: Vec<PointerEvent>,
    /// The latest pointer position when the pass began, `None` when the pointer is not in the window.
    pub pos: Option<Pos2>,
    /// Where the pointer hovers: `None` while it is gone.
    pub hover: Option<Pos2>,
    /// The pointer movement of the pass.
    pub delta: Vec2,
    /// Whether the primary button is down.
    pub primary_down: bool,
    /// Every button held.
    pub buttons: Buttons,
    /// The button that went up last in the pass, any button.
    pub release: Option<PointerButton>,
    /// The last press of a button other than the primary one: the button and where it went down.
    pub other_press: Option<(PointerButton, Pos2)>,
    /// Whether the primary button went down in this pass.
    pub primary_pressed: bool,
    /// The modifiers held.
    pub mods: Mods,
    /// The pass's time in seconds.
    pub time: f64,
    pub kind: PointerKind,
    pub sample: Option<PointerSample>,
    /// The window lost focus in this pass.
    pub focus_lost: bool,
    /// Whether the real pointer moved or entered in this pass.
    pub stirred: bool,
    /// The zoom factor of the pass: pinch and Ctrl or Cmd with the wheel.
    pub zoom: f32,
    /// The pass's summed `Event::Rotate` radians.
    pub rotation: f32,
    /// The two-finger gesture: translation, zoom, rotation and centre.
    pub multi_touch: Option<(Vec2, f32, f32, Pos2)>,
}

const ALL_BUTTONS: [egui::PointerButton; 5] = [
    egui::PointerButton::Primary,
    egui::PointerButton::Secondary,
    egui::PointerButton::Middle,
    egui::PointerButton::Extra1,
    egui::PointerButton::Extra2,
];

pub(crate) fn button_of(button: egui::PointerButton) -> PointerButton {
    match button {
        egui::PointerButton::Primary => PointerButton::Primary,
        egui::PointerButton::Secondary => PointerButton::Secondary,
        egui::PointerButton::Middle => PointerButton::Middle,
        egui::PointerButton::Extra1 => PointerButton::Extra1,
        egui::PointerButton::Extra2 => PointerButton::Extra2,
    }
}

/// Converts the running pass's pointer input. The pen's readings since the last drain pair with the pass's pointer
/// moves (`PenPass::paired`); a touch's force pairs with its moves instead. A pen tip pressed with the barrel button
/// held is the secondary button until it lifts.
///
/// Only a pass whose events carry pointer events drains the pen's queue: a pass without one (another window's, or a
/// rerun pass, whose events egui hands only to the frame's first pass) leaves the readings for the pass with the moves.
/// A pass whose injected step carries a pen reading (`injected::pen`) reads it in place of the tablet, which it leaves
/// alone.
pub(crate) fn pointer_read(ctx: &Context) -> PointerRead {
    let os = crate::runtime::windows::os_of(ctx);
    let injected_pen = super::injected::pen(ctx);
    let pen = crate::runtime::pen::source(ctx).filter(|_| injected_pen.is_none());
    let pointed = ctx.input(|input| input.raw.events.iter().any(|event| matches!(event, Event::PointerMoved(_) | Event::PointerButton { .. })));
    let tablet = match injected_pen {
        Some(sample) => Some(PenPass::injected(sample)),
        None => pen.as_ref().map(|source| if pointed { PenPass::read(source) } else { PenPass::live(source) }),
    };
    let (read, barrel_press) = ctx.input(|input| {
        let mut events = Vec::new();
        let mut moves = Vec::new();
        let mut focus_lost = false;
        let mut stirred = false;
        let mut touched = false;
        let mut touch_sample = None;
        let mut release = None;
        let mut other_press = None;
        let mut rotation = 0.0;
        let mut barrel_press = tablet.as_ref().is_some_and(|tablet| tablet.barrel_press);
        let mut barrel_pressed = false;
        for event in &input.raw.events {
            match event {
                Event::PointerButton { pos, button, pressed, .. } if *button != egui::PointerButton::Primary => {
                    let button = button_of(*button);
                    if matches!(button, PointerButton::Middle | PointerButton::Secondary) {
                        events.push(if *pressed {
                            PointerEvent::pressed_by(button, *pos, input.time, 1)
                        } else {
                            PointerEvent::released_by(button, *pos)
                        });
                    }
                    if *pressed {
                        if button != PointerButton::Middle {
                            other_press = Some((button, *pos));
                        }
                    } else {
                        release = Some(button);
                    }
                }
                Event::PointerMoved(pos) => {
                    stirred = true;
                    moves.push(events.len());
                    events.push(PointerEvent::moved(*pos, None));
                }
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => {
                    // Only the next press sets the barrel state again, so a missed release never remaps a plain tap.
                    barrel_press = tablet.as_ref().is_some_and(|tablet| tablet.barrel);
                    if barrel_press {
                        barrel_pressed = true;
                        other_press = Some((PointerButton::Secondary, *pos));
                        events.push(PointerEvent::pressed_by(PointerButton::Secondary, *pos, input.time, 1));
                    } else {
                        events.push(PointerEvent::pressed(*pos, input.time, 1));
                    }
                }
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => {
                    if barrel_press {
                        barrel_press = false;
                        release = Some(PointerButton::Secondary);
                        events.push(PointerEvent::released_by(PointerButton::Secondary, *pos));
                    } else {
                        release = Some(PointerButton::Primary);
                        events.push(PointerEvent::released(*pos));
                    }
                }
                Event::PointerGone => events.push(PointerEvent::Gone),
                Event::WindowFocused(false) => focus_lost = true,
                Event::Touch { phase, force, .. } => {
                    touched = true;
                    if *phase != TouchPhase::End {
                        touch_sample = force.map(PointerSample::new);
                    }
                }
                Event::Rotate(radians) if radians.is_finite() => rotation += *radians,
                _ => {}
            }
        }
        let pen_active = tablet.as_ref().is_some_and(PenPass::active);
        let (kind, sample) = match &tablet {
            _ if touched => (PointerKind::Touch, touch_sample),
            Some(tablet) if pen_active => (PointerKind::Pen, tablet.current),
            _ => (PointerKind::Mouse, None),
        };
        for (nr, &at) in moves.iter().enumerate() {
            let paired = match &tablet {
                _ if touched => touch_sample,
                Some(tablet) if pen_active => tablet.paired(nr, moves.len()),
                _ => None,
            };
            if let Some(PointerEvent::Moved { sample, .. }) = events.get_mut(at) {
                *sample = paired;
            }
        }
        let multi_touch = input.multi_touch().map(|touch| (touch.translation_delta, touch.zoom_delta, touch.rotation_delta, touch.center_pos));
        let held = ALL_BUTTONS
            .into_iter()
            .filter(|button| input.pointer.button_down(*button))
            .fold(Buttons::empty(), |held, button| held | Buttons::of(button_of(button)));
        // A barrel contact holds the secondary button where egui holds the primary one.
        let buttons = if barrel_press && held.contains(Buttons::PRIMARY) { held.difference(Buttons::PRIMARY) | Buttons::SECONDARY } else { held };
        let read = PointerRead {
            events,
            pos: input.pointer.latest_pos(),
            hover: input.pointer.hover_pos(),
            delta: input.pointer.delta(),
            primary_down: input.pointer.primary_down() && !barrel_press,
            buttons,
            release,
            other_press,
            primary_pressed: input.pointer.primary_pressed() && !barrel_pressed,
            mods: Mods::from_egui(&input.modifiers, os),
            time: input.time,
            kind,
            sample,
            focus_lost,
            stirred,
            zoom: input.zoom_delta(),
            rotation,
            multi_touch,
        };
        (read, barrel_press)
    });
    if let Some(pen) = &pen {
        pen.set_barrel_press(barrel_press);
    }
    read
}

/// The ids of egui's click and drag widgets in the hit test at the latest pointer position, which is the
/// press's own hit test when the pass carries a press as its last pointer event (H0).
pub(crate) fn hit_targets(ctx: &Context) -> (Option<Id>, Option<Id>) {
    ctx.viewport(|viewport| (viewport.hits.click.map(|widget| widget.id), viewport.hits.drag.map(|widget| widget.id)))
}

/// The wheel input of the pass: the summed scroll in points (`line` and `page` points per line and page), the
/// last event's unit, phase and modifiers, and the device: fingers when any event of the pass came from them. `None`
/// when no wheel event arrived.
///
/// A wheel event's modifiers are the windowing layer's modifier state, which on Windows can miss a held Alt while the
/// Alt key's own press and release still arrive: Alt counts as held from its key's press to its release, starting from
/// whether it was down before the pass's first Alt key event (egui's held keys when the pass has none).
pub(crate) fn wheel_read(ctx: &Context, line: f32, page: f32) -> Option<(Vec2, WheelUnit, WheelPhase, Mods, WheelSource)> {
    let os = crate::runtime::windows::os_of(ctx);
    let zoom_modifier = ctx.options(|options| options.input_options.zoom_modifier);
    let is_alt = |key: &Key| matches!(key, Key::AltLeft | Key::AltRight);
    ctx.input(|input| {
        let mut read: Option<(Vec2, WheelUnit, WheelPhase, Mods, WheelSource)> = None;
        // A repeat means the key was already down; a release means it was down until then.
        let mut alt_key = input
            .raw
            .events
            .iter()
            .find_map(|event| match event {
                Event::Key { key, pressed, repeat, .. } if is_alt(key) => Some(!*pressed || *repeat),
                _ => None,
            })
            .unwrap_or_else(|| input.key_down(Key::AltLeft) || input.key_down(Key::AltRight));
        for event in &input.raw.events {
            if let Event::Key { key, pressed, .. } = event
                && is_alt(key)
            {
                alt_key = *pressed;
            }
            if let Event::MouseWheel { unit, delta, modifiers, phase, source } = event {
                let (unit, points) = match unit {
                    MouseWheelUnit::Point => (WheelUnit::Point, 1.0),
                    MouseWheelUnit::Line => (WheelUnit::Line, line),
                    MouseWheelUnit::Page => (WheelUnit::Page, page),
                };
                let phase = match phase {
                    TouchPhase::Start => WheelPhase::Start,
                    TouchPhase::Move => WheelPhase::Move,
                    TouchPhase::End => WheelPhase::End,
                    TouchPhase::Cancel => WheelPhase::Cancel,
                };
                // Wheel events under the zoom modifier zoom (`PointerFrame.zoom`) and never scroll.
                let share = if modifiers.matches_any(zoom_modifier) { Vec2::ZERO } else { *delta * points };
                let sum = read.map_or(Vec2::ZERO, |(sum, ..)| sum) + share;
                let held = egui::Modifiers { alt: modifiers.alt || alt_key, ..*modifiers };
                let source = match source {
                    MouseWheelSource::Unknown => WheelSource::Unknown,
                    MouseWheelSource::Wheel => WheelSource::Wheel,
                    MouseWheelSource::Trackpad => WheelSource::Trackpad,
                    MouseWheelSource::Momentum => WheelSource::Momentum,
                };
                let source = match read {
                    Some((.., before)) if before.fingers() && !source.fingers() => before,
                    _ => source,
                };
                read = Some((sum, unit, phase, Mods::from_egui(&held, os), source));
            }
        }
        read
    })
}

/// Removes the events a modal layer in another viewport blocks: button and key presses, wheel, text, clipboard and
/// touch. Pointer moves and releases stay, so the pointer position stays right and no press or key sticks down. The
/// events `keep` admits stay too (the canvas the modal leaves open, `runtime::sampler`); `pointer` is where the pointer
/// stood before `events`. Returns whether anything went and whether a pointer press did or was kept.
pub(crate) fn drop_under_modal(events: &mut Vec<Arrived>, keep: Option<&crate::runtime::sampler::Hole>, pointer: Option<Pos2>) -> (bool, bool) {
    let before = events.len();
    let mut press = false;
    let mut at = pointer;
    events.retain(|Arrived { event, .. }| {
        match event {
            Event::PointerMoved(pos) | Event::PointerButton { pos, .. } => at = Some(*pos),
            _ => {}
        }
        let blocked = matches!(
            event,
            Event::PointerButton { pressed: true, .. }
                | Event::MouseWheel { .. }
                | Event::Key { pressed: true, .. }
                | Event::Text(_)
                | Event::Copy
                | Event::Cut
                | Event::Paste(_)
                | Event::Touch { .. }
        );
        press |= matches!(event, Event::PointerButton { pressed: true, .. });
        !blocked || keep.is_some_and(|hole| hole.admits(event, at))
    });
    (events.len() != before, press)
}

/// Adds a release of the primary button at `pos` after the events of `input`.
pub(crate) fn push_primary_release(input: &mut RawInput, pos: Pos2) {
    input.events.push(Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, modifiers: egui::Modifiers::NONE });
}

/// Follows the primary button and the pointer through `events`: `down` ends as the button's last state and `last`
/// as the last position any pointer event named.
pub(crate) fn follow_primary(events: &[Event], down: &mut bool, last: &mut Option<Pos2>) {
    for event in events {
        match event {
            Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed, .. } => {
                *down = *pressed;
                *last = Some(*pos);
            }
            Event::PointerMoved(pos) => *last = Some(*pos),
            _ => {}
        }
    }
}

/// Removes the first commit-on character typed this pass from the input events, keeping the text typed before it in
/// the same event; returns whether there was one. Pasted text arrives as a paste event and is left alone.
pub(crate) fn take_separator(ctx: &Context, commit_on: &[char]) -> bool {
    if commit_on.is_empty() {
        return false;
    }
    ctx.input_mut(|input| {
        let Some(at) = input.events.iter().position(|event| matches!(event, egui::Event::Text(text) if text.contains(commit_on))) else {
            return false;
        };
        if let egui::Event::Text(text) = &mut input.events[at] {
            let end = text.find(commit_on).unwrap_or(text.len());
            text.truncate(end);
            if text.is_empty() {
                input.events.remove(at);
            }
        }
        true
    })
}

/// The most characters one text, paste or IME event carries into a component.
pub const INPUT_TEXT_LIMIT: usize = 1 << 20;

/// Cuts every `Event::Text`, `Event::Paste`, IME commit and preedit at `INPUT_TEXT_LIMIT` characters, at a character
/// boundary, before any component sees it; a preedit range past the cut is dropped.
pub(crate) fn cap_text(events: &mut [Event]) {
    for event in events {
        match event {
            Event::Text(text) | Event::Paste(text) | Event::Ime(ImeEvent::Commit(text)) => {
                cut_text(text);
            }
            Event::Ime(ImeEvent::Preedit { text, active_range_chars }) => {
                if cut_text(text) && active_range_chars.as_ref().is_some_and(|range| range.end > INPUT_TEXT_LIMIT) {
                    *active_range_chars = None;
                }
            }
            _ => {}
        }
    }
}

/// Truncates `text` after `INPUT_TEXT_LIMIT` characters; returns whether it was longer.
fn cut_text(text: &mut String) -> bool {
    match text.char_indices().nth(INPUT_TEXT_LIMIT) {
        Some((at, _)) => {
            text.truncate(at);
            true
        }
        None => false,
    }
}

/// How long after a pinch step a whole Ctrl+wheel notch still belongs to the pinch, in seconds: touchpad drivers
/// that send a pinch as Ctrl+wheel round some of its steps to whole notches.
const PINCH_CONTINUES: f64 = 0.25;

/// Windows and Linux: turns a fractional Ctrl+wheel (or a Wayland point-unit one) into `Event::Zoom` at egui's Ctrl-scroll
/// speed; a whole notch stays a wheel event unless within 0.25 s of a pinch step. An input that carries `Event::Zoom`
/// already is left alone and ends the run of pinch steps.
pub(crate) fn fold_legacy_pinch(ctx: &Context, input: &mut RawInput) {
    let os = crate::runtime::windows::os_of(ctx);
    if os == Os::Mac {
        return;
    }
    let (line, speed) = ctx.options(|options| (options.input_options.line_scroll_speed, options.input_options.scroll_zoom_speed));
    let now = input.time;
    let events = &mut input.events;
    PerViewport::<RawState>::with(ctx, input.viewport_id, |state| {
        // A device that reports its own pinch sends `Event::Zoom`: nothing here is a legacy pinch.
        if events.iter().any(|event| matches!(event, Event::Zoom(_))) {
            state.last_pinch = None;
            return;
        }
        for event in events.iter_mut() {
            let Event::MouseWheel { unit, delta, modifiers, .. } = event else { continue };
            let (unit, delta, modifiers) = (*unit, *delta, *modifiers);
            if !modifiers.ctrl || modifiers.alt || modifiers.shift {
                continue;
            }
            let whole = unit == MouseWheelUnit::Line && delta.x.fract() == 0.0 && delta.y.fract() == 0.0;
            let smooth = match unit {
                MouseWheelUnit::Line => !whole,
                MouseWheelUnit::Point => os == Os::Linux,
                MouseWheelUnit::Page => false,
            };
            let continues = whole && now.zip(state.last_pinch).is_some_and(|(now, last)| now - last <= PINCH_CONTINUES);
            if !(smooth || continues) {
                continue;
            }
            let points = if unit == MouseWheelUnit::Line { delta * line } else { delta };
            *event = Event::Zoom((speed * (points.x + points.y)).exp());
            state.last_pinch = now.or(state.last_pinch);
        }
    });
}

/// The raw-input state of one viewport that `prepare` keeps: the legacy pinch's run and the Control-click remap.
#[derive(Default)]
pub(crate) struct RawState {
    /// The input time of the last Ctrl+wheel folded into a pinch step.
    last_pinch: Option<f64>,
    control_click: ControlClick,
}

/// The input hook's raw-input step for every viewport, after the injected step joined the input and before the key
/// stage and the one-press rule: folds a legacy pinch, caps the text events, then turns a macOS Control-click into a
/// secondary click. Injected input passes it as real input does.
pub(crate) fn prepare(ctx: &Context, input: &mut RawInput) {
    fold_legacy_pinch(ctx, input);
    cap_text(&mut input.events);
    let macos = crate::runtime::windows::os_of(ctx) == Os::Mac;
    let events = &mut input.events;
    PerViewport::<RawState>::with(ctx, input.viewport_id, |state| state.control_click.normalize(events, macos));
}

/// The widget egui holds a drag on.
pub(crate) fn dragged(ctx: &Context) -> Option<Id> {
    ctx.dragged_id()
}

/// Where the press that is held down began, while a button is down.
pub(crate) fn held_press_origin(ctx: &Context) -> Option<Pos2> {
    ctx.input(|input| if input.pointer.any_down() { input.pointer.press_origin() } else { None })
}

/// A plain Left or Right arrow press reached egui this pass, with no modifier held.
pub(crate) fn plain_horizontal_arrow_pressed(ctx: &Context) -> bool {
    ctx.input(|input| input.modifiers.is_none() && (input.key_pressed(Key::ArrowLeft) || input.key_pressed(Key::ArrowRight)))
}

/// An Up, Down, Page Up or Page Down press reached egui this pass.
pub(crate) fn step_key_pressed(ctx: &Context) -> bool {
    ctx.input(|input| [Key::ArrowUp, Key::ArrowDown, Key::PageUp, Key::PageDown].into_iter().any(|key| input.key_pressed(key)))
}

/// Removes this pass's Tab and Shift+Tab presses from the input events; returns whether there was one. A Tab with
/// Ctrl, Cmd or Alt is the window's, not the field's, and stays.
pub(crate) fn take_tab(ctx: &Context) -> bool {
    ctx.input_mut(|input| {
        let before = input.events.len();
        input.events.retain(|event| {
            !matches!(event, Event::Key { key: Key::Tab, pressed: true, modifiers, .. }
                if !modifiers.ctrl && !modifiers.command && !modifiers.mac_cmd && !modifiers.alt)
        });
        input.events.len() != before
    })
}
