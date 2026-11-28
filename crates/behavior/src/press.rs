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
    /// The press went down on another target, which handed it to this one ([`PressMachine::take_over`]).
    pub handed: bool,
}

impl PressStart {
    /// A press of `button` at `origin`.
    pub const fn new(origin: Pos2, button: PointerButton, time: f64, clicks: u32) -> Self {
        PressStart { origin, button, time, clicks, handed: false }
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
    /// An airbrush pen's finger wheel, 0 to 1; `None` when the pen has none.
    pub wheel: Option<f32>,
}

impl PointerSample {
    /// A sample with no tilt or rotation.
    pub const fn new(pressure: f32) -> Self {
        PointerSample { pressure, tilt: [0.0, 0.0], rotation: 0.0, eraser: false, wheel: None }
    }

    /// The sample with every field in range: pressure within 0 to 1, tilt and rotation finite radians, a stray value
    /// replaced by its neutral one.
    ///
    /// A tablet driver's pressure outside 0 to 1 is clamped and one that is no number reads as full pressure, so a
    /// stroke keeps drawing; tilt is clamped to a quarter turn each way and an unreadable tilt or rotation is none.
    #[must_use]
    pub fn sanitized(self) -> Self {
        let pressure = if self.pressure.is_nan() { 1.0 } else { self.pressure.clamp(0.0, 1.0) };
        let quarter = std::f32::consts::FRAC_PI_2;
        let tilt = self.tilt.map(|tilt| if tilt.is_finite() { tilt.clamp(-quarter, quarter) } else { 0.0 });
        let rotation = if self.rotation.is_finite() { self.rotation } else { 0.0 };
        let wheel = self.wheel.filter(|wheel| wheel.is_finite()).map(|wheel| wheel.clamp(0.0, 1.0));
        PointerSample { pressure, tilt, rotation, eraser: self.eraser, wheel }
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
    /// The host ended what the press acted on (`primitives::cancel_press_of`).
    Host,
    /// A second finger landed while a press was open.
    Gesture,
}

/// Two or more fingers came down and all lifted within [`MULTI_TAP_TIME`] without moving past [`MULTI_TAP_SLOP`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct MultiTap {
    /// How many fingers tapped.
    pub fingers: u8,
    /// The centre of the fingers.
    pub at: Pos2,
}

impl MultiTap {
    /// A tap of `fingers` around `at`.
    pub const fn new(fingers: u8, at: Pos2) -> Self {
        MultiTap { fingers, at }
    }
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
        /// The pointer position this pass.
        pos: Pos2,
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
            | PressPhase::Held { start, .. }
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
            PressPhase::Held { start, pos } => PressPhase::Held { start: at(start), pos: point(pos) },
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
    /// Where a secondary press made during another open press came up on this part in this pass
    /// ([`PressStep::aside`]): a context request beside the running phase, which goes on.
    pub context: Option<Pos2>,
}

impl PressState {
    /// A part with no press history.
    pub const IDLE: PressState = PressState::new(PressPhase::Idle);

    /// A state in `phase` with no modifiers, sample, capture or aside context request.
    pub const fn new(phase: PressPhase) -> Self {
        PressState { phase, mods: Mods::NONE, sample: None, captured: false, context: None }
    }

    /// The aside secondary press that came up on the part at `at` in this pass.
    pub const fn context(mut self, at: Option<Pos2>) -> Self {
        self.context = at;
        self
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
pub const MULTI_CLICK_TIME: f64 = 0.3;

/// The longest the fingers of a multi-finger tap may stay down, in seconds.
pub const MULTI_TAP_TIME: f64 = 0.5;

/// How far a finger of a multi-finger tap may move from where it landed, in points: fingers drift more than a
/// pointer does while the others land.
pub const MULTI_TAP_SLOP: f32 = 16.0;

/// The click count of a press at `pos` and `time` with `mods` held, given the previous press `(pos, time, clicks,
/// mods)`: one more when it follows within [`MULTI_CLICK_TIME`] and `slop` of the previous one with the same
/// modifiers, else 1.
pub fn count_clicks(previous: Option<(Pos2, f64, u32, Mods)>, pos: Pos2, time: f64, mods: Mods, slop: f32) -> u32 {
    match previous {
        Some((place, when, count, held)) if held == mods && time - when <= MULTI_CLICK_TIME && place.distance(pos) <= slop => count + 1,
        _ => 1,
    }
}

/// A button event of one pass, in the order it arrived. The press lifecycle follows the primary button and the
/// middle button; other buttons are not events.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum PointerEvent {
    /// The pointer moved, with the pen's reading for the move when the runtime has one.
    #[non_exhaustive]
    Moved {
        /// Where the pointer is.
        at: Pos2,
        /// The pen's reading paired with this move, interpolated from the tablet's samples.
        sample: Option<PointerSample>,
    },
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
    /// The pointer moved to `at`, with the pen's reading for the move when the runtime has one.
    pub const fn moved(at: Pos2, sample: Option<PointerSample>) -> Self {
        PointerEvent::Moved { at, sample }
    }

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
    /// The start of an open press whose sense has `PASSES_DRAG` and no `DRAG` that moved past slop in this pass:
    /// the caller may hand it to the shape below with [`PressMachine::hand_over`].
    pub passes: Option<PressStart>,
    /// A secondary press that came up in this pass while a primary press stayed open: a right-click during a drag. The
    /// open press goes on; the aside asks its target for a context menu.
    pub aside: Option<PressAside<T>>,
}

impl<T> Default for PressStep<T> {
    fn default() -> Self {
        PressStep { target: None, phase: None, dragging: false, activated: None, passes: None, aside: None }
    }
}

/// A secondary press made and released while a primary press was open ([`PressStep::aside`]). Its target is the one
/// `pick` gave the secondary press, so it senses `Sense::CONTEXT_CLICK`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PressAside<T> {
    /// The target of the secondary press.
    pub target: T,
    /// Where and when the secondary button went down.
    pub start: PressStart,
    /// Where it came up, in screen space.
    pub pos: Pos2,
}

/// The press winner a root picks at a press position, as [`PressMachine::step`]'s `pick` returns it.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct PressPick<T> {
    /// The press winner.
    pub target: T,
    /// What it senses of the press.
    pub sense: Sense,
    /// The press's click went to a shape above that passes its drag to this target (`Sense::PASSES_DRAG`): the press
    /// opens silent and is the target's only once its drag starts.
    pub passed: bool,
}

impl<T> PressPick<T> {
    /// `target`, sensing `sense`, takes the press.
    pub fn new(target: T, sense: Sense) -> Self {
        PressPick { target, sense, passed: false }
    }

    /// The press's click went to a shape above that passes its drag to this target.
    pub fn passed(mut self) -> Self {
        self.passed = true;
        self
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
    /// Moved past slop in this pass on a target that clicks and passes its drag (`Sense::PASSES_DRAG`), with the
    /// pointer at the pass's end: no click and no drag until [`PressMachine::hand_over`] gives the press its new
    /// target. The next pass abandons a press no shape took.
    passing: Option<Pos2>,
    /// A press passed to this target ([`PressPick::passed`]) whose drag has not started: it reports nothing, and a
    /// release or cancel ends it unreported. Its move past slop starts the drag with `DragStart`, as a hand-over does.
    silent: bool,
    /// The pointer left the window ([`PointerEvent::Gone`]): the click is gone, but a target that senses a drag still
    /// starts it from the moves that follow. The button is still down and the window keeps getting the pointer's moves
    /// and release while it is outside, so a press near a window edge (a window's resize grip) that leaves the window
    /// before it passes slop still drags.
    left: bool,
    /// A pass reported the press: later passes within slop report `Held`, not `Down`.
    reported: bool,
    /// A pass reported the drag's start: later passes report `Drag`.
    drag_reported: bool,
    /// Where the button went up and whether that was a click, when it did in a pass that still owed the press's `Down`
    /// or `DragStart`. That pass reports the owed phase and a later one the release, so every part reads a press's
    /// phases in order, one per pass.
    released_at: Option<(Pos2, bool)>,
}

impl<T> Open<T> {
    /// A phase before the release is still to be reported, or the press waits for its hand-over.
    fn owes_phase(&self) -> bool {
        !self.reported || (self.dragging && !self.drag_reported) || self.passing.is_some()
    }

    /// A release now is a click.
    fn clicks(&self) -> bool {
        !self.dragging && !self.abandoned && !self.left && self.passing.is_none() && self.sense.contains(Sense::CLICK)
    }
}

/// The press lifecycle of one root: the press winner it picked, kept until release or cancel.
#[derive(Clone, Copy, Debug)]
pub struct PressMachine<T> {
    open: Option<Open<T>>,
    /// The secondary press held beside the open primary press, with its target and start.
    aside: Option<(T, PressStart)>,
}

impl<T> Default for PressMachine<T> {
    fn default() -> Self {
        PressMachine { open: None, aside: None }
    }
}

impl<T: Copy> PressMachine<T> {
    /// A machine with no press.
    pub const fn new() -> Self {
        PressMachine { open: None, aside: None }
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

    /// The press the open press began as.
    pub fn start(&self) -> Option<PressStart> {
        self.open.map(|open| open.start)
    }

    /// Takes over a press whose button is down and went down elsewhere, as a drag of `target` from `start`: the next
    /// pass reports `DragStart`, and later ones the press's own `Drag`, `Release` or `Cancel` from their input, each
    /// with a start that says it was handed over ([`PressStart::handed`]). A press this machine holds already stays as
    /// it is.
    // PLAN: the sheet's Files table has no row for this file; the handover of a held press (decision q346) needs the
    // machine to open a press that did not begin on it, which only the machine can build.
    pub fn take_over(&mut self, target: T, sense: Sense, start: PressStart) {
        if self.open.is_some() {
            return;
        }
        self.open = Some(Open {
            target,
            sense: sense - Sense::CLICK,
            start: PressStart { handed: true, ..start },
            dragging: true,
            abandoned: false,
            left: false,
            reported: true,
            drag_reported: false,
            released_at: None,
            passing: None,
            silent: false,
        });
    }

    /// Whether the button went up and the release waits for a later pass, which the caller must ask for: no input may
    /// come to start it.
    pub fn release_held(&self) -> bool {
        self.open.is_some_and(|open| open.released_at.is_some())
    }

    /// Runs one pass. `pick` chooses the press winner and what it senses at a press position, for the button pressed
    /// there; it returns `None` when no shape of this root takes the press. A secondary press goes only to a shape that
    /// senses `Sense::CONTEXT_CLICK`, so `pick` passes over the others for it, to the next shape under the press.
    pub fn step(&mut self, input: PassInput<'_>, mut pick: impl FnMut(Pos2, PointerButton) -> Option<PressPick<T>>) -> PressStep<T> {
        let mut pos = input.pos;
        // The press released or cancelled in this pass, and whether it was silent: one that ends silent reports nothing.
        let mut released: Option<(T, PressStart, Pos2, bool, bool, bool)> = None;
        let mut cancelled: Option<(T, PressStart, PressCancel, bool)> = None;
        let mut aside: Option<PressAside<T>> = None;
        // A press passed in the last pass that no shape took is abandoned; that pass reported its `Down`.
        if let Some(open) = &mut self.open
            && open.passing.is_some()
        {
            open.passing = None;
            open.abandoned = true;
            open.reported = true;
        }
        if let Some(open) = self.open
            && let Some((at, click)) = open.released_at
            && !open.owes_phase()
        {
            released = Some((open.target, open.start, at, open.dragging, click, open.silent));
            self.open = None;
        }
        if let (Some(reason), Some(open)) = (input.cancel, self.open) {
            cancelled = Some((open.target, open.start, reason, open.silent));
            self.open = None;
        }
        for event in input.events {
            match *event {
                PointerEvent::Moved { at, .. } => {
                    pos = at;
                    if let Some(open) = &mut self.open
                        && open.released_at.is_none()
                        && !open.dragging
                        && !open.abandoned
                        && open.passing.is_none()
                        && at.distance(open.start.origin) > input.slop
                    {
                        if open.sense.contains(Sense::DRAG) {
                            open.dragging = true;
                            open.silent = false;
                        } else if open.sense.contains(Sense::CLICK | Sense::PASSES_DRAG) {
                            open.passing = Some(at);
                        } else {
                            open.abandoned = true;
                        }
                    }
                }
                PointerEvent::Gone => {
                    if let Some(open) = &mut self.open {
                        open.left = true;
                    }
                }
                PointerEvent::Pressed { button, pos: at, time, clicks } => {
                    pos = at;
                    // A release held for the next pass gives way to a new press; it ends in this pass.
                    if let Some(open) = self.open
                        && let Some((up, click)) = open.released_at
                    {
                        released = Some((open.target, open.start, up, open.dragging, click, open.silent));
                        self.open = None;
                    }
                    // A secondary press during an open primary press goes beside it, to the context target `pick` gives
                    // it, and the primary press goes on (a right-click during a drag asks for the menu there).
                    if button == PointerButton::Secondary
                        && self.aside.is_none()
                        && self.open.is_some_and(|open| open.start.button == PointerButton::Primary)
                    {
                        let start = PressStart::new(at, button, time, clicks);
                        self.aside = pick(at, button).filter(|pick| pick.sense.contains(Sense::CONTEXT_CLICK)).map(|pick| (pick.target, start));
                        continue;
                    }
                    // An open press keeps the pointer until its button comes up: no press starts over it, of its own
                    // button (a repeated Down, as a window never sends one) or another. Its moves and its release still
                    // reach it when a modal opened over its target since. A middle press captures for a drag (a canvas
                    // pan) and never clicks, unless its target senses `MIDDLE_CLICK` (a tab closes on it). A secondary
                    // press is only a target's that senses `CONTEXT_CLICK`, which clicks and drags with it; a target that
                    // only drags (a transform box) leaves it to the context target under it.
                    let capture_only = button != PointerButton::Primary;
                    if !matches!(button, PointerButton::Primary | PointerButton::Middle | PointerButton::Secondary) || self.open.is_some() {
                        continue;
                    }
                    self.open = pick(at, button)
                        .map(|pick| {
                            let clicks = match button {
                                PointerButton::Middle => pick.sense.contains(Sense::MIDDLE_CLICK),
                                PointerButton::Secondary => pick.sense.contains(Sense::CONTEXT_CLICK),
                                _ => false,
                            };
                            let sense = if capture_only && !clicks { pick.sense - Sense::CLICK } else { pick.sense };
                            (pick, sense, clicks)
                        })
                        .filter(|(_, sense, clicks)| match button {
                            PointerButton::Primary => true,
                            PointerButton::Middle => *clicks || sense.contains(Sense::DRAG),
                            _ => *clicks,
                        })
                        .map(|(pick, sense, _)| {
                            // A silent press owes no `Down`: its first phase is its `DragStart`.
                            let silent = pick.passed && sense.contains(Sense::DRAG);
                            Open {
                                target: pick.target,
                                sense,
                                start: PressStart::new(at, button, time, clicks),
                                dragging: false,
                                abandoned: false,
                                left: false,
                                passing: None,
                                silent,
                                reported: silent,
                                drag_reported: false,
                                released_at: None,
                            }
                        });
                }
                PointerEvent::Released { button, pos: at } => {
                    pos = at;
                    if button == PointerButton::Secondary
                        && let Some((target, start)) = self.aside.take()
                    {
                        aside = Some(PressAside { target, start, pos: at });
                        continue;
                    }
                    if self.open.is_some_and(|open| open.start.button != button || open.released_at.is_some()) {
                        continue;
                    }
                    if let Some(open) = &mut self.open
                        && open.owes_phase()
                    {
                        open.released_at = Some((at, open.clicks()));
                    } else if let Some(open) = self.open.take() {
                        released = Some((open.target, open.start, at, open.dragging, open.clicks(), open.silent));
                    }
                }
            }
        }
        // A press that ended without a release event (pointer lost) is gone.
        if self.open.is_some_and(|open| open.released_at.is_none()) && !input.held && released.is_none() {
            self.open = None;
        }
        // An aside press lasts no longer than the press it went beside.
        if self.open.is_none() {
            self.aside = None;
        }
        let quiet = PressStep { aside, ..PressStep::default() };
        if let Some((target, start, reason, silent)) = cancelled {
            if silent {
                return quiet;
            }
            let phase = PressPhase::Cancel { start, reason };
            return PressStep { target: Some(target), phase: Some(phase), ..quiet };
        }
        if let Some((target, start, at, was_dragging, click, silent)) = released {
            if silent {
                return quiet;
            }
            let phase = PressPhase::Release { start, pos: at, click };
            return PressStep {
                target: Some(target),
                phase: Some(phase),
                dragging: was_dragging,
                activated: (click && start.button == PointerButton::Primary).then_some(target),
                ..quiet
            };
        }
        let Some(open) = self.open.as_mut() else { return quiet };
        if open.silent {
            return quiet;
        }
        if let Some(at) = &mut open.passing {
            *at = pos;
        }
        let phase = if !open.reported {
            PressPhase::Down { start: open.start }
        } else if open.dragging && !open.drag_reported {
            open.drag_reported = true;
            PressPhase::DragStart { start: open.start, pos }
        } else if open.dragging {
            PressPhase::Drag { start: open.start, pos, delta: input.delta }
        } else {
            PressPhase::Held { start: open.start, pos }
        };
        // A passing press owes its `Down` to the shape `hand_over` gives it when this pass is its first.
        if open.passing.is_none() {
            open.reported = true;
        }
        let passes = open.passing.map(|_| open.start);
        PressStep { target: Some(open.target), phase: Some(phase), dragging: open.drag_reported, passes, ..quiet }
    }

    /// Makes `target` the open press's target in the pass that reported `passes`, dragging from the same
    /// `PressStart`. It returns the `DragStart` step when an earlier pass reported the press, and the `Down` step
    /// (the `DragStart` follows in the next pass) when the move past slop came in the pass of the press itself.
    /// `None` when no press passes in this pass or `sense` has no `DRAG`; the next pass abandons the press. The step
    /// has no [`aside`](PressStep::aside): that stays the pass's own step's.
    pub fn hand_over(&mut self, target: T, sense: Sense) -> Option<PressStep<T>> {
        let open = self.open.as_mut().filter(|open| !open.abandoned && sense.contains(Sense::DRAG))?;
        let at = open.passing.take()?;
        open.target = target;
        open.sense = sense;
        open.dragging = true;
        let phase = if open.reported {
            open.drag_reported = true;
            PressPhase::DragStart { start: open.start, pos: at }
        } else {
            open.reported = true;
            PressPhase::Down { start: open.start }
        };
        Some(PressStep { target: Some(target), phase: Some(phase), dragging: open.drag_reported, ..PressStep::default() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;

    const SLOP: f32 = 6.0;

    fn pass(machine: &mut PressMachine<u8>, events: &[PointerEvent], sense: Sense) -> PressStep<u8> {
        let held = machine.open.is_some() || matches!(events.last(), Some(PointerEvent::Pressed { .. }));
        machine.step(PassInput::new(events, SLOP, pos2(0.0, 0.0), held), |_, _| Some(PressPick::new(1, sense)))
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

    /// A right-click during a drag goes beside it: the secondary release reports an aside on the context target, and the
    /// drag goes on to its own release. A secondary press on a target without `CONTEXT_CLICK` reports none.
    #[test]
    fn a_secondary_click_during_a_drag_reports_an_aside_and_the_drag_goes_on() {
        let (from, to, menu) = (pos2(0.0, 0.0), pos2(40.0, 0.0), pos2(40.0, 5.0));
        let mut machine = PressMachine::new();
        let mut pass = |events: &[PointerEvent]| {
            machine.step(PassInput::new(events, SLOP, to, true), |_, button| match button {
                PointerButton::Secondary => Some(PressPick::new(2, Sense::CLICK | Sense::CONTEXT_CLICK)),
                _ => Some(PressPick::new(1, Sense::DRAG)),
            })
        };
        pass(&[PointerEvent::pressed(from, 1.0, 1)]);
        pass(&[PointerEvent::moved(to, None)]);
        let down = pass(&[PointerEvent::pressed_by(PointerButton::Secondary, menu, 1.2, 1)]);
        assert!(down.aside.is_none(), "nothing asks before the secondary button comes up");
        assert!(matches!(down.phase, Some(PressPhase::Drag { .. })), "the drag goes on: {down:?}");
        let up = pass(&[PointerEvent::released_by(PointerButton::Secondary, menu)]);
        assert_eq!(up.aside.map(|aside| (aside.target, aside.pos)), Some((2, menu)), "the release asks the context target");
        assert_eq!(up.target, Some(1), "the drag keeps its target");
        assert!(up.phase.is_some_and(|phase| phase.is_drag()), "and its drag: {up:?}");
        let end = pass(&[PointerEvent::released(to)]);
        assert!(matches!(end.phase, Some(PressPhase::Release { .. })) && end.aside.is_none(), "the drag ends on its own release: {end:?}");

        let mut machine = PressMachine::new();
        let mut pass = |events: &[PointerEvent]| machine.step(PassInput::new(events, SLOP, to, true), |_, _| Some(PressPick::new(1, Sense::DRAG)));
        pass(&[PointerEvent::pressed(from, 1.0, 1)]);
        pass(&[PointerEvent::pressed_by(PointerButton::Secondary, menu, 1.2, 1)]);
        assert!(pass(&[PointerEvent::released_by(PointerButton::Secondary, menu)]).aside.is_none(), "a drag-only target takes no aside");
    }

    /// The phases a sequence of passes reports, as `D`own, `H`eld, `S` (drag start), `G` (drag), `R`elease with `!` for an
    /// activation, `-` for none.
    fn phases(passes: &[&[PointerEvent]], sense: Sense) -> String {
        let mut machine = PressMachine::new();
        passes
            .iter()
            .map(|events| {
                let step = pass(&mut machine, events, sense);
                match step.phase {
                    Some(PressPhase::Down { .. }) => "D",
                    Some(PressPhase::Held { .. }) => "H",
                    Some(PressPhase::DragStart { .. }) => "S",
                    Some(PressPhase::Drag { .. }) => "G",
                    Some(PressPhase::Release { .. }) if step.activated.is_some() => "R!",
                    Some(PressPhase::Release { .. }) => "R",
                    _ => "-",
                }
            })
            .collect::<Vec<_>>()
            .join(" ")
    }

    /// A press of the held button, where nothing takes a press (a modal opened over the drag's target), leaves the open
    /// press its drag, and the release ends that press from its own start. Replacing it would end the drag with no
    /// release, and its owner would never see the button come up.
    #[test]
    fn a_press_of_the_held_button_leaves_the_open_drag_its_moves_and_release() {
        let first = pos2(0.0, 0.0);
        let mut machine = PressMachine::new();
        let mut step = |events: &[PointerEvent]| {
            machine.step(PassInput::new(events, SLOP, first, true), |at, _| (at == first).then_some(PressPick::new(1u8, Sense::DRAG)))
        };
        step(&[press(0.0, 1.0)]);
        step(&[PointerEvent::Moved { at: pos2(40.0, 0.0), sample: None }]);
        let repeated = step(&[press(50.0, 1.2)]);
        assert_eq!(repeated.target, Some(1), "the open press keeps its target");
        assert!(matches!(repeated.phase, Some(PressPhase::Drag { start, .. }) if start.origin == first), "{:?}", repeated.phase);
        let moved = step(&[PointerEvent::Moved { at: pos2(60.0, 0.0), sample: None }]);
        assert!(matches!(moved.phase, Some(PressPhase::Drag { pos, .. }) if pos == pos2(60.0, 0.0)), "{:?}", moved.phase);
        let up = step(&[PointerEvent::released(pos2(60.0, 0.0))]);
        assert!(
            matches!(up.phase, Some(PressPhase::Release { start, pos, .. }) if start.origin == first && pos == pos2(60.0, 0.0)),
            "{:?}",
            up.phase
        );
        assert!(machine.target().is_none(), "the release ends the press");
    }

    /// A press taken over from another machine reports its drag's start first, then the held button's own drag and
    /// release, which never clicks.
    #[test]
    fn a_press_taken_over_reports_its_drag_start_then_its_own_phases() {
        let origin = pos2(10.0, 0.0);
        let mut machine = PressMachine::new();
        machine.take_over(1u8, Sense::DRAG | Sense::CLICK, PressStart::new(origin, PointerButton::Primary, 1.0, 1));
        let first = pass(&mut machine, &[], Sense::DRAG);
        assert!(matches!(first.phase, Some(PressPhase::DragStart { start, .. }) if start.origin == origin), "{:?}", first.phase);
        let moved = pass(&mut machine, &[PointerEvent::moved(pos2(30.0, 0.0), None)], Sense::DRAG);
        assert!(matches!(moved.phase, Some(PressPhase::Drag { pos, .. }) if pos == pos2(30.0, 0.0)), "{:?}", moved.phase);
        let up = pass(&mut machine, &[PointerEvent::released(pos2(30.0, 0.0))], Sense::DRAG);
        assert!(matches!(up.phase, Some(PressPhase::Release { click: false, .. })), "{:?}", up.phase);
        assert_eq!(up.activated, None, "a taken-over press is no click");
    }

    /// The pointer leaving the window (a press on a window's edge grip pulled outward) ends the press's click but not
    /// its drag: the window keeps getting the held button's moves from outside, and they start the drag once past slop.
    /// The resize from a window edge used to fail whenever the pointer left the window before it passed slop.
    #[test]
    fn a_press_whose_pointer_left_the_window_still_drags_but_no_longer_clicks() {
        let moved = |x: f32| PointerEvent::Moved { at: pos2(x, 0.0), sample: None };
        let up = |x: f32| PointerEvent::released(pos2(x, 0.0));
        let gone = PointerEvent::Gone;
        let out = [moved(SLOP / 2.0), gone];
        assert_eq!(phases(&[&[press(0.0, 1.0)], &out, &[moved(SLOP * 2.0)], &[moved(SLOP * 3.0)], &[up(SLOP * 3.0)]], Sense::DRAG), "D H S G R");
        assert_eq!(phases(&[&[press(0.0, 1.0)], &out, &[up(SLOP / 2.0)]], Sense::CLICK), "D H R", "a click target loses its click");
        assert_eq!(
            phases(&[&[press(0.0, 1.0)], &out, &[moved(0.0)], &[up(0.0)]], Sense::CLICK | Sense::DRAG),
            "D H H R",
            "coming back does not bring the click back"
        );
        assert_eq!(phases(&[&[press(0.0, 1.0)], &[moved(0.0)], &[up(0.0)]], Sense::CLICK), "D H R!", "a press that stays in still clicks");
    }

    #[test]
    fn held_reports_the_pointer_of_its_pass_within_slop() {
        let mut machine = PressMachine::new();
        pass(&mut machine, &[press(0.0, 1.0)], Sense::DRAG);
        let near = pos2(SLOP / 2.0, 0.0);
        let step = pass(&mut machine, &[PointerEvent::Moved { at: near, sample: None }], Sense::DRAG);
        assert!(matches!(step.phase, Some(PressPhase::Held { pos, .. }) if pos == near), "{:?}", step.phase);
    }

    #[test]
    fn two_clicks_whose_passes_share_a_release_and_a_press_report_every_phase_in_order() {
        let up = PointerEvent::released(pos2(0.0, 0.0));
        let second = press(0.0, 1.1);
        assert_eq!(phases(&[&[press(0.0, 1.0)], &[up, second], &[up], &[]], Sense::CLICK), "D R! D R!");
        // The second press's drag starts in the pass that owes its Down, and the button goes up there too.
        let far = PointerEvent::Moved { at: pos2(40.0, 0.0), sample: None };
        let passes: [&[PointerEvent]; 5] = [&[press(0.0, 1.0)], &[up, second], &[far, up], &[], &[]];
        assert_eq!(phases(&passes, Sense::CLICK | Sense::DRAG), "D R! D S R");
    }

    const PASSES: Sense = Sense::CLICK.union(Sense::PASSES_DRAG);

    /// One pass with the button held at its end, over a root whose press winner is target 1 and passes its drag.
    fn passing_pass(machine: &mut PressMachine<u8>, events: &[PointerEvent]) -> PressStep<u8> {
        machine.step(PassInput::new(events, SLOP, pos2(0.0, 0.0), true), |_, _| Some(PressPick::new(1, PASSES)))
    }

    fn moved(x: f32) -> PointerEvent {
        PointerEvent::Moved { at: pos2(x, 0.0), sample: None }
    }

    fn up(x: f32) -> PointerEvent {
        PointerEvent::released(pos2(x, 0.0))
    }

    #[test]
    fn a_drag_past_slop_on_a_part_that_passes_it_goes_to_the_shape_below() {
        let origin = pos2(0.0, 0.0);
        assert_eq!(phases(&[&[press(0.0, 1.0)], &[up(SLOP / 2.0)]], PASSES), "D R!", "within slop the press is the shape's click");

        // The press was reported in an earlier pass: the shape below drags from the pass of the move.
        let mut machine = PressMachine::new();
        let down = passing_pass(&mut machine, &[press(0.0, 1.0)]);
        assert!(matches!(down.phase, Some(PressPhase::Down { .. })) && down.passes.is_none(), "{down:?}");
        assert!(machine.hand_over(2, Sense::DRAG).is_none(), "a press within slop passes nothing");
        let passed = passing_pass(&mut machine, &[moved(40.0)]);
        assert_eq!(passed.passes.map(|start| start.origin), Some(origin), "{passed:?}");
        let handed = machine.hand_over(2, Sense::DRAG).expect("the shape below takes the drag");
        assert_eq!(handed.target, Some(2));
        assert!(
            matches!(handed.phase, Some(PressPhase::DragStart { start, pos }) if start.origin == origin && pos == pos2(40.0, 0.0)),
            "{:?}",
            handed.phase
        );
        let drag = passing_pass(&mut machine, &[moved(60.0)]);
        assert_eq!(drag.target, Some(2));
        assert!(matches!(drag.phase, Some(PressPhase::Drag { .. })), "{:?}", drag.phase);
        let end = passing_pass(&mut machine, &[up(60.0)]);
        assert_eq!(end.target, Some(2));
        assert!(matches!(end.phase, Some(PressPhase::Release { click: false, .. })), "{:?}", end.phase);
        assert_eq!(end.activated, None, "a passed press is no click");
    }

    /// The move past slop came in the press's own pass: the shape below gets the Down, its DragStart next pass, and a
    /// release in between waits for it.
    #[test]
    fn a_drag_past_slop_in_the_press_pass_hands_the_down_over() {
        let origin = pos2(0.0, 0.0);
        let mut machine = PressMachine::new();
        let first = passing_pass(&mut machine, &[press(0.0, 1.0), moved(40.0)]);
        assert!(matches!(first.phase, Some(PressPhase::Down { .. })) && first.passes.is_some(), "{first:?}");
        let handed = machine.hand_over(2, Sense::DRAG).expect("the shape below takes the drag");
        assert_eq!(handed.target, Some(2));
        assert!(matches!(handed.phase, Some(PressPhase::Down { start }) if start.origin == origin), "{:?}", handed.phase);
        let start = passing_pass(&mut machine, &[up(40.0)]);
        assert_eq!(start.target, Some(2));
        assert!(matches!(start.phase, Some(PressPhase::DragStart { .. })), "{:?}", start.phase);
        assert!(machine.release_held(), "the release waits for the owed DragStart");
        let end = passing_pass(&mut machine, &[]);
        assert_eq!(end.target, Some(2));
        assert!(matches!(end.phase, Some(PressPhase::Release { click: false, .. })), "{:?}", end.phase);
    }

    /// No shape took the passed press: it is abandoned, and its release is no click.
    #[test]
    fn a_passed_press_no_shape_takes_is_abandoned() {
        let mut machine = PressMachine::new();
        let _ = passing_pass(&mut machine, &[press(0.0, 1.0)]);
        assert!(passing_pass(&mut machine, &[moved(40.0)]).passes.is_some());
        let end = passing_pass(&mut machine, &[up(40.0)]);
        assert_eq!(end.target, Some(1));
        assert!(matches!(end.phase, Some(PressPhase::Release { click: false, .. })), "{:?}", end.phase);
        assert_eq!(end.activated, None);
    }
}
