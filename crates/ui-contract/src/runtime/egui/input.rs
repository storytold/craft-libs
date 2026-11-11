//! Raw input in the input hook, key events converted to craft-keymap's key types, `InputOptions`.

use craft_behavior::one_press::split_after_first_press;
use craft_behavior::{Buttons, PointerButton, PointerEvent, PointerKind, PointerSample, WheelPhase, WheelUnit};
use craft_host::target::Os;
use craft_keymap::{KeyInput, Mods};
use egui::{Context, Event, Id, MouseWheelUnit, Pos2, TouchPhase, Vec2, ViewportId};

pub use egui::RawInput;

/// What the one-press rule needs between frames, for one viewport.
#[derive(Default)]
pub(crate) struct HeldInput {
    events: Vec<Event>,
    frame: Option<u64>,
}

impl HeldInput {
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
pub(crate) fn hold_back_presses(input: &mut RawInput, frame: u64, held: &mut HeldInput) -> bool {
    if held.frame == Some(frame) {
        return false;
    }
    held.frame = Some(frame);
    let mut events = std::mem::take(&mut held.events);
    events.append(&mut input.events);
    let (now, later) = split_after_first_press(events, |event| matches!(event, Event::PointerButton { pressed: true, .. }));
    input.events = now;
    held.events = later;
    !held.events.is_empty()
}

/// Asks for an immediate repaint of `viewport`, from the input hook where the viewport is not yet current.
pub(crate) fn request_repaint_of(ctx: &Context, viewport: ViewportId) {
    ctx.request_repaint_of(viewport);
}

/// H2: a click is a release within `slop`, whatever its duration, so egui's click and the press lifecycle agree.
pub(crate) fn set_click_options(ctx: &Context, slop: f32) {
    let (dist, duration) = ctx.options(|options| (options.input_options.max_click_dist, options.input_options.max_click_duration));
    if dist != slop || duration != f64::INFINITY {
        ctx.options_mut(|options| {
            options.input_options.max_click_dist = slop;
            options.input_options.max_click_duration = f64::INFINITY;
        });
    }
}

/// The options the runtime owns at install: interface scale and quit are commands, so egui's own shortcuts go.
pub(crate) fn clear_shortcuts(ctx: &Context) {
    ctx.options_mut(|options| {
        options.zoom_with_keyboard = false;
        options.quit_shortcuts.clear();
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
    /// The two-finger gesture: translation, zoom, rotation and centre.
    pub multi_touch: Option<(Vec2, f32, f32, Pos2)>,
    /// egui's longest movement that still makes a click.
    pub slop: f32,
}

const ALL_BUTTONS: [egui::PointerButton; 5] = [
    egui::PointerButton::Primary,
    egui::PointerButton::Secondary,
    egui::PointerButton::Middle,
    egui::PointerButton::Extra1,
    egui::PointerButton::Extra2,
];

fn button_of(button: egui::PointerButton) -> PointerButton {
    match button {
        egui::PointerButton::Primary => PointerButton::Primary,
        egui::PointerButton::Secondary => PointerButton::Secondary,
        egui::PointerButton::Middle => PointerButton::Middle,
        egui::PointerButton::Extra1 => PointerButton::Extra1,
        egui::PointerButton::Extra2 => PointerButton::Extra2,
    }
}

/// Converts the running pass's pointer input.
pub(crate) fn pointer_read(ctx: &Context) -> PointerRead {
    let slop = ctx.options(|options| options.input_options.max_click_dist);
    let os = crate::runtime::windows::os_of(ctx);
    ctx.input(|input| {
        let mut events = Vec::new();
        let mut focus_lost = false;
        let mut stirred = false;
        let mut kind = PointerKind::Mouse;
        let mut sample = None;
        let mut release = None;
        let mut other_press = None;
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
                    events.push(PointerEvent::Moved(*pos));
                }
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, .. } => {
                    events.push(PointerEvent::pressed(*pos, input.time, 1));
                }
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: false, .. } => {
                    release = Some(PointerButton::Primary);
                    events.push(PointerEvent::released(*pos));
                }
                Event::PointerGone => events.push(PointerEvent::Gone),
                Event::WindowFocused(false) => focus_lost = true,
                Event::Touch { phase, force, .. } => {
                    kind = PointerKind::Touch;
                    if *phase != TouchPhase::End {
                        sample = force.map(PointerSample::new);
                    }
                }
                _ => {}
            }
        }
        let multi_touch = input.multi_touch().map(|touch| (touch.translation_delta, touch.zoom_delta, touch.rotation_delta, touch.center_pos));
        PointerRead {
            events,
            pos: input.pointer.latest_pos(),
            hover: input.pointer.hover_pos(),
            delta: input.pointer.delta(),
            primary_down: input.pointer.primary_down(),
            buttons: ALL_BUTTONS
                .into_iter()
                .filter(|button| input.pointer.button_down(*button))
                .fold(Buttons::empty(), |held, button| held | Buttons::of(button_of(button))),
            release,
            other_press,
            primary_pressed: input.pointer.primary_pressed(),
            mods: Mods::from_egui(&input.modifiers, os),
            time: input.time,
            kind,
            sample,
            focus_lost,
            stirred,
            zoom: input.zoom_delta(),
            multi_touch,
            slop,
        }
    })
}

/// The ids of egui's click and drag widgets in the hit test at the latest pointer position, which is the
/// press's own hit test when the pass carries a press as its last pointer event (H0).
pub(crate) fn hit_targets(ctx: &Context) -> (Option<Id>, Option<Id>) {
    ctx.viewport(|viewport| (viewport.hits.click.map(|widget| widget.id), viewport.hits.drag.map(|widget| widget.id)))
}

/// The wheel input of the pass: the summed scroll in points (`line` and `page` points per line and page), the
/// last event's unit, phase and modifiers. `None` when no wheel event arrived.
pub(crate) fn wheel_read(ctx: &Context, line: f32, page: f32) -> Option<(Vec2, WheelUnit, WheelPhase, Mods)> {
    let os = crate::runtime::windows::os_of(ctx);
    let zoom_modifier = ctx.options(|options| options.input_options.zoom_modifier);
    ctx.input(|input| {
        let mut read: Option<(Vec2, WheelUnit, WheelPhase, Mods)> = None;
        for event in &input.raw.events {
            if let Event::MouseWheel { unit, delta, modifiers, phase, .. } = event {
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
                read = Some((sum, unit, phase, Mods::from_egui(modifiers, os)));
            }
        }
        read
    })
}

/// Removes the events a modal layer in another viewport blocks: button and key presses, wheel, text, clipboard and
/// touch. Pointer moves and releases stay, so the pointer position stays right and no press or key sticks down. The
/// events `keep` admits stay too (the canvas the modal leaves open, `runtime::sampler`); `pointer` is where the pointer
/// stood before `input`. Returns whether anything went and whether a pointer press did or was kept.
pub(crate) fn drop_under_modal(input: &mut RawInput, keep: Option<&crate::runtime::sampler::Hole>, pointer: Option<Pos2>) -> (bool, bool) {
    let before = input.events.len();
    let mut press = false;
    let mut at = pointer;
    input.events.retain(|event| {
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
    (input.events.len() != before, press)
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
