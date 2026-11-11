//! Pointer values and the press lifecycle (8.1, 8.2), egui-free: the runtime converts egui's events once per
//! pass and feeds them to a [`PressMachine`].

use crate::sense::Sense;
use bitflags::bitflags;
use craft_keymap::Mods;
use emath::{Pos2, Vec2};

/// A pointer button.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum PointerButton {
    /// The left button, a touch contact or a pen tip.
    #[default]
    Primary,
    /// The right button.
    Secondary,
    /// The wheel button.
    Middle,
    /// The first extra button, "back".
    Extra1,
    /// The second extra button, "forward".
    Extra2,
}

bitflags! {
    /// The pointer buttons held.
    #[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
    pub struct Buttons: u8 {
        /// [`PointerButton::Primary`].
        const PRIMARY = 1;
        /// [`PointerButton::Secondary`].
        const SECONDARY = 2;
        /// [`PointerButton::Middle`].
        const MIDDLE = 4;
        /// [`PointerButton::Extra1`].
        const EXTRA1 = 8;
        /// [`PointerButton::Extra2`].
        const EXTRA2 = 16;
    }
}

impl Buttons {
    /// The flag of one button.
    pub const fn of(button: PointerButton) -> Buttons {
        match button {
            PointerButton::Primary => Buttons::PRIMARY,
            PointerButton::Secondary => Buttons::SECONDARY,
            PointerButton::Middle => Buttons::MIDDLE,
            PointerButton::Extra1 => Buttons::EXTRA1,
            PointerButton::Extra2 => Buttons::EXTRA2,
        }
    }
}

/// What kind of device the pointer is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum PointerKind {
    /// A mouse or trackpad.
    #[default]
    Mouse,
    /// A pen.
    Pen,
    /// A finger.
    Touch,
}

/// Where and when a press began.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PressStart {
    /// The pointer position at the press.
    pub origin: Pos2,
    /// The button that went down.
    pub button: PointerButton,
    /// The pass time of the press, in seconds.
    pub time: f64,
    /// 1 for a single press, 2 for a double press, and so on.
    pub clicks: u32,
}

impl PressStart {
    /// A press of `button` at `origin`.
    pub const fn new(origin: Pos2, button: PointerButton, time: f64, clicks: u32) -> Self {
        PressStart { origin, button, time, clicks }
    }
}

/// Pressure and tilt of the current contact.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PointerSample {
    /// 0 to 1.
    pub pressure: f32,
    /// Tilt around x and y, in radians.
    pub tilt: [f32; 2],
    /// Barrel rotation, in radians.
    pub rotation: f32,
    /// The eraser end of a pen.
    pub eraser: bool,
}

impl PointerSample {
    /// A sample with no tilt or rotation.
    pub const fn new(pressure: f32) -> Self {
        PointerSample { pressure, tilt: [0.0, 0.0], rotation: 0.0, eraser: false }
    }
}

/// A two-finger gesture.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MultiTouch {
    /// Pan since the last pass.
    pub translation: Vec2,
    /// Zoom factor since the last pass.
    pub zoom: f32,
    /// Rotation since the last pass, in radians.
    pub rotation: f32,
    /// The centre of the fingers.
    pub center: Pos2,
}

impl MultiTouch {
    /// A gesture around `center`.
    pub const fn new(translation: Vec2, zoom: f32, rotation: f32, center: Pos2) -> Self {
        MultiTouch { translation, zoom, rotation, center }
    }
}

/// Why a captured press ended without a release.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PressCancel {
    /// Escape.
    Escape,
    /// The window lost focus.
    FocusLost,
    /// The part turned disabled.
    Disabled,
    /// The owner was not drawn for a pass.
    Removed,
}

/// One part's press state in one pass.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PressPhase {
    /// Nothing under the pointer.
    Idle,
    /// The pointer is over the part.
    Hover,
    /// The button went down on the part: the first pass that reports the press, once per press.
    #[non_exhaustive]
    Down {
        /// The press.
        start: PressStart,
    },
    /// The button stays down on the part, within slop, in the passes after `Down`.
    #[non_exhaustive]
    Held {
        /// The press.
        start: PressStart,
    },
    /// The press moved past slop on a part that senses a drag.
    #[non_exhaustive]
    DragStart {
        /// The press.
        start: PressStart,
        /// The pointer position.
        pos: Pos2,
    },
    /// The drag continues.
    #[non_exhaustive]
    Drag {
        /// The press.
        start: PressStart,
        /// The pointer position.
        pos: Pos2,
        /// The pointer movement of this pass.
        delta: Vec2,
    },
    /// The button went up.
    #[non_exhaustive]
    Release {
        /// The press.
        start: PressStart,
        /// The pointer position at the release.
        pos: Pos2,
        /// A release within slop of a part that senses a click.
        click: bool,
    },
    /// A captured press ended without a release.
    #[non_exhaustive]
    Cancel {
        /// The press.
        start: PressStart,
        /// Why it ended.
        reason: PressCancel,
    },
}

impl PressPhase {
    /// A press is in progress or just ended: not `Idle` or `Hover`.
    pub const fn is_press(&self) -> bool {
        !matches!(self, PressPhase::Idle | PressPhase::Hover)
    }

    /// The press the phase belongs to.
    pub const fn start(&self) -> Option<PressStart> {
        match self {
            PressPhase::Down { start }
            | PressPhase::Held { start }
            | PressPhase::DragStart { start, .. }
            | PressPhase::Drag { start, .. }
            | PressPhase::Release { start, .. }
            | PressPhase::Cancel { start, .. } => Some(*start),
            PressPhase::Idle | PressPhase::Hover => None,
        }
    }

    /// Whether the button is down: `Down`, `Held`, `DragStart` or `Drag`.
    pub const fn button_down(&self) -> bool {
        matches!(self, PressPhase::Down { .. } | PressPhase::Held { .. } | PressPhase::DragStart { .. } | PressPhase::Drag { .. })
    }

    /// Whether the phase is a drag in progress.
    pub const fn is_drag(&self) -> bool {
        matches!(self, PressPhase::DragStart { .. } | PressPhase::Drag { .. })
    }

    /// Whether the phase is a click: a release within slop.
    pub const fn is_click(&self) -> bool {
        matches!(self, PressPhase::Release { click: true, .. })
    }

    /// The phase with its positions mapped by `point` and its drag delta by `delta`, for a part whose space
    /// differs from the screen's.
    pub fn mapped(self, point: impl Fn(Pos2) -> Pos2, delta: impl Fn(Vec2) -> Vec2) -> PressPhase {
        let at = |mut start: PressStart| {
            start.origin = point(start.origin);
            start
        };
        match self {
            PressPhase::Idle | PressPhase::Hover => self,
            PressPhase::Down { start } => PressPhase::Down { start: at(start) },
            PressPhase::Held { start } => PressPhase::Held { start: at(start) },
            PressPhase::DragStart { start, pos } => PressPhase::DragStart { start: at(start), pos: point(pos) },
            PressPhase::Drag { start, pos, delta: d } => PressPhase::Drag { start: at(start), pos: point(pos), delta: delta(d) },
            PressPhase::Release { start, pos, click } => PressPhase::Release { start: at(start), pos: point(pos), click },
            PressPhase::Cancel { start, reason } => PressPhase::Cancel { start: at(start), reason },
        }
    }
}

/// A part's phase, the modifiers held, the pen sample and whether it holds the pointer.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PressState {
    /// The phase.
    pub phase: PressPhase,
    /// The modifiers held.
    pub mods: Mods,
    /// Pressure and tilt of the current contact.
    pub sample: Option<PointerSample>,
    /// A drag holds the pointer: nothing else hovers.
    pub captured: bool,
}

impl PressState {
    /// A part with no press history.
    pub const IDLE: PressState = PressState::new(PressPhase::Idle);

    /// A state in `phase` with no modifiers, sample or capture.
    pub const fn new(phase: PressPhase) -> Self {
        PressState { phase, mods: Mods::NONE, sample: None, captured: false }
    }

    /// The modifiers held.
    pub const fn mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    /// The pen sample of the contact.
    pub const fn sample(mut self, sample: Option<PointerSample>) -> Self {
        self.sample = sample;
        self
    }

    /// Whether a drag holds the pointer.
    pub const fn captured(mut self, captured: bool) -> Self {
        self.captured = captured;
        self
    }

    /// Whether the phase is the `Down` of the second press of a double click: true once, false while the button is
    /// held and for a first press. Every double-click reader uses this, never `clicks >= 2` on a later phase.
    pub const fn double_pressed(&self) -> bool {
        match self.phase {
            PressPhase::Down { start } => start.clicks >= 2 && matches!(start.button, PointerButton::Primary),
            _ => false,
        }
    }
}

impl Default for PressState {
    fn default() -> Self {
        PressState::IDLE
    }
}

/// The longest gap between presses that still count as one multi-click, in seconds.
const MULTI_CLICK_TIME: f64 = 0.3;

/// The click count of a press at `pos` and `time`, given the previous press `(pos, time, clicks)`: one more when
/// it follows within [`MULTI_CLICK_TIME`] and `slop` of the previous one, else 1.
pub fn count_clicks(previous: Option<(Pos2, f64, u32)>, pos: Pos2, time: f64, slop: f32) -> u32 {
    match previous {
        Some((place, when, count)) if time - when <= MULTI_CLICK_TIME && place.distance(pos) <= slop => count + 1,
        _ => 1,
    }
}

/// A button event of one pass, in the order it arrived. The press lifecycle follows the primary button and the
/// middle button; other buttons are not events.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PointerEvent {
    /// The pointer moved.
    Moved(Pos2),
    /// A button went down.
    #[non_exhaustive]
    Pressed {
        /// Which button.
        button: PointerButton,
        /// Where.
        pos: Pos2,
        /// The pass time.
        time: f64,
        /// 1 for a single press, 2 for a double press, and so on; see [`count_clicks`].
        clicks: u32,
    },
    /// A button went up.
    #[non_exhaustive]
    Released {
        /// Which button.
        button: PointerButton,
        /// Where.
        pos: Pos2,
    },
    /// The pointer left the window or was lost: a press under way can no longer click.
    Gone,
}

impl PointerEvent {
    /// The primary button going down at `pos`, `time`, as the `clicks`th press in a row.
    pub const fn pressed(pos: Pos2, time: f64, clicks: u32) -> Self {
        PointerEvent::Pressed { button: PointerButton::Primary, pos, time, clicks }
    }

    /// The primary button going up at `pos`.
    pub const fn released(pos: Pos2) -> Self {
        PointerEvent::Released { button: PointerButton::Primary, pos }
    }

    /// `button` going down at `pos`, `time`, as the `clicks`th press in a row.
    pub const fn pressed_by(button: PointerButton, pos: Pos2, time: f64, clicks: u32) -> Self {
        PointerEvent::Pressed { button, pos, time, clicks }
    }

    /// `button` going up at `pos`.
    pub const fn released_by(button: PointerButton, pos: Pos2) -> Self {
        PointerEvent::Released { button, pos }
    }
}

/// What the machine reads of one pass.
#[derive(Clone, Copy, Debug)]
#[non_exhaustive]
pub struct PassInput<'a> {
    /// The pass's pointer events.
    pub events: &'a [PointerEvent],
    /// The movement that turns a press into a drag.
    pub slop: f32,
    /// The latest pointer position when the pass began.
    pub pos: Pos2,
    /// The pointer movement of the pass.
    pub delta: Vec2,
    /// Whether the primary or the middle button is down when the pass ends.
    pub held: bool,
    /// A reason the captured press ends now.
    pub cancel: Option<PressCancel>,
}

impl<'a> PassInput<'a> {
    /// A pass with these events, no movement and no cancel.
    pub const fn new(events: &'a [PointerEvent], slop: f32, pos: Pos2, held: bool) -> Self {
        PassInput { events, slop, pos, delta: Vec2::ZERO, held, cancel: None }
    }

    /// The pointer movement of the pass.
    pub const fn delta(mut self, delta: Vec2) -> Self {
        self.delta = delta;
        self
    }

    /// A reason the captured press ends now.
    pub const fn cancel(mut self, cancel: Option<PressCancel>) -> Self {
        self.cancel = cancel;
        self
    }
}

/// What one pass did to the press.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PressStep<T> {
    /// The press winner while a press is open, or the one released or cancelled in this pass.
    pub target: Option<T>,
    /// The target's phase, in screen space.
    pub phase: Option<PressPhase>,
    /// A drag is in progress or ended in this pass.
    pub dragging: bool,
    /// The target released as a click in this pass.
    pub activated: Option<T>,
}

impl<T> Default for PressStep<T> {
    fn default() -> Self {
        PressStep { target: None, phase: None, dragging: false, activated: None }
    }
}

#[derive(Clone, Copy, Debug)]
struct Open<T> {
    target: T,
    sense: Sense,
    start: PressStart,
    dragging: bool,
    /// Moved past slop on a target that senses no drag: the click is gone and nothing starts.
    abandoned: bool,
    /// A pass reported the press: later passes within slop report `Held`, not `Down`.
    reported: bool,
    /// Where the button went up, when it did in the pass the drag started. That pass reports `DragStart` and the
    /// next one the release, so every part reads a drag's start before its end.
    released_at: Option<Pos2>,
}

/// The press lifecycle of one root: the press winner it picked, kept until release or cancel.
#[derive(Clone, Copy, Debug)]
pub struct PressMachine<T> {
    open: Option<Open<T>>,
}

impl<T> Default for PressMachine<T> {
    fn default() -> Self {
        PressMachine { open: None }
    }
}

impl<T: Copy> PressMachine<T> {
    /// A machine with no press.
    pub const fn new() -> Self {
        PressMachine { open: None }
    }

    /// The target of the open press.
    pub fn target(&self) -> Option<T> {
        self.open.map(|open| open.target)
    }

    /// The button of the open press.
    pub fn button(&self) -> Option<PointerButton> {
        self.open.map(|open| open.start.button)
    }

    /// Whether a drag holds the pointer.
    pub fn dragging(&self) -> bool {
        self.open.is_some_and(|open| open.dragging)
    }

    /// Runs one pass. `pick` chooses the press winner and what it senses at a press position; it returns `None`
    /// when no shape of this root takes the press.
    pub fn step(&mut self, input: PassInput<'_>, mut pick: impl FnMut(Pos2) -> Option<(T, Sense)>) -> PressStep<T> {
        let mut pos = input.pos;
        let mut drag_started = false;
        let mut released: Option<(T, PressStart, Pos2, bool, bool)> = None;
        let mut cancelled: Option<(T, PressStart, PressCancel)> = None;
        if let Some(open) = self.open
            && let Some(at) = open.released_at
        {
            released = Some((open.target, open.start, at, true, false));
            self.open = None;
        }
        if let (Some(reason), Some(open)) = (input.cancel, self.open) {
            cancelled = Some((open.target, open.start, reason));
            self.open = None;
        }
        for event in input.events {
            match *event {
                PointerEvent::Moved(at) => {
                    pos = at;
                    if let Some(open) = &mut self.open
                        && !open.dragging
                        && !open.abandoned
                        && at.distance(open.start.origin) > input.slop
                    {
                        if open.sense.contains(Sense::DRAG) {
                            open.dragging = true;
                            drag_started = true;
                        } else {
                            open.abandoned = true;
                        }
                    }
                }
                PointerEvent::Gone => {
                    if let Some(open) = &mut self.open {
                        open.abandoned = true;
                    }
                }
                PointerEvent::Pressed { button, pos: at, time, clicks } => {
                    pos = at;
                    // A release held for the next pass gives way to a new press; it ends in this pass.
                    if let Some(open) = self.open
                        && let Some(up) = open.released_at
                    {
                        released = Some((open.target, open.start, up, true, false));
                        self.open = None;
                    }
                    // A middle or secondary press captures for a drag (a canvas pan, a drag with the secondary button) and
                    // never clicks or starts over an open press; a primary press does not replace one of them. A target
                    // that senses `MIDDLE_CLICK` also takes a middle press that can click (a tab closes on it), and one
                    // that senses `CONTEXT_CLICK` takes a secondary press, which a click-only target would not capture.
                    let capture_only = button != PointerButton::Primary;
                    let over_other = self.open.is_some_and(|open| capture_only || open.start.button != button);
                    if !matches!(button, PointerButton::Primary | PointerButton::Middle | PointerButton::Secondary) || over_other {
                        continue;
                    }
                    self.open = pick(at)
                        .map(|(target, sense)| {
                            let clicks = match button {
                                PointerButton::Middle => sense.contains(Sense::MIDDLE_CLICK),
                                PointerButton::Secondary => sense.contains(Sense::CONTEXT_CLICK),
                                _ => false,
                            };
                            (target, if capture_only && !clicks { sense - Sense::CLICK } else { sense }, clicks)
                        })
                        .filter(|(_, sense, clicks)| !capture_only || *clicks || sense.contains(Sense::DRAG))
                        .map(|(target, sense, _)| (target, sense))
                        .map(|(target, sense)| Open {
                            target,
                            sense,
                            start: PressStart::new(at, button, time, clicks),
                            dragging: false,
                            abandoned: false,
                            reported: false,
                            released_at: None,
                        });
                    drag_started = false;
                }
                PointerEvent::Released { button, pos: at } => {
                    pos = at;
                    if self.open.is_some_and(|open| open.start.button != button || open.released_at.is_some()) {
                        continue;
                    }
                    if let Some(open) = &mut self.open
                        && drag_started
                    {
                        open.released_at = Some(at);
                    } else if let Some(open) = self.open.take() {
                        let click = !open.dragging && !open.abandoned && open.sense.contains(Sense::CLICK);
                        released = Some((open.target, open.start, at, open.dragging, click));
                    }
                }
            }
        }
        // A press that ended without a release event (pointer lost) is gone.
        if self.open.is_some_and(|open| open.released_at.is_none()) && !input.held && released.is_none() {
            self.open = None;
        }
        if let Some((target, start, reason)) = cancelled {
            let phase = PressPhase::Cancel { start, reason };
            return PressStep { target: Some(target), phase: Some(phase), dragging: false, activated: None };
        }
        if let Some((target, start, at, was_dragging, click)) = released {
            let phase = PressPhase::Release { start, pos: at, click };
            return PressStep {
                target: Some(target),
                phase: Some(phase),
                dragging: was_dragging,
                activated: (click && start.button == PointerButton::Primary).then_some(target),
            };
        }
        let Some(open) = self.open.as_mut() else { return PressStep::default() };
        let phase = if drag_started {
            PressPhase::DragStart { start: open.start, pos }
        } else if open.dragging {
            PressPhase::Drag { start: open.start, pos, delta: input.delta }
        } else if open.reported {
            PressPhase::Held { start: open.start }
        } else {
            PressPhase::Down { start: open.start }
        };
        open.reported = true;
        PressStep { target: Some(open.target), phase: Some(phase), dragging: open.dragging, activated: None }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;

    const SLOP: f32 = 6.0;

    fn pass(machine: &mut PressMachine<u8>, events: &[PointerEvent], sense: Sense) -> PressStep<u8> {
        let held = machine.open.is_some() || matches!(events.last(), Some(PointerEvent::Pressed { .. }));
        machine.step(PassInput::new(events, SLOP, pos2(0.0, 0.0), held), |_| Some((1, sense)))
    }

    fn press(x: f32, time: f64) -> PointerEvent {
        PointerEvent::pressed(pos2(x, 0.0), time, 1)
    }

    #[test]
    fn a_release_and_a_new_press_in_one_pass_activate_the_first_click() {
        let mut machine = PressMachine::new();
        pass(&mut machine, &[press(0.0, 1.0)], Sense::CLICK);
        let events = [PointerEvent::released(pos2(0.0, 0.0)), press(0.0, 1.1)];
        let step = pass(&mut machine, &events, Sense::CLICK);
        assert_eq!(step.activated, Some(1));
        assert!(step.phase.is_some_and(|phase| phase.is_click()));
        let step = pass(&mut machine, &[], Sense::CLICK);
        assert!(matches!(step.phase, Some(PressPhase::Down { .. })), "the second press is reported once it opens");
        let step = pass(&mut machine, &[], Sense::CLICK);
        assert!(matches!(step.phase, Some(PressPhase::Held { .. })), "and stays open");
        let step = pass(&mut machine, &[PointerEvent::released(pos2(0.0, 0.0))], Sense::CLICK);
        assert_eq!(step.activated, Some(1));
    }
}
