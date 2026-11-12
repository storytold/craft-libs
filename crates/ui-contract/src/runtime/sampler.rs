//! The canvas reach under a modal layer (15.5, 8.3): the one exception to "a modal blocks the input below it".
//!
//! A modal dialog that shows a canvas keeps it usable, as Photoshop's Color Picker, Levels and Curves do. A control
//! of the dialog registers the canvas every pass with its reach: the view navigation always passes (the wheel that
//! scrolls and zooms, the middle press that pans, and the primary press while Space is held over the canvas, for the
//! pan), and so do the gestures the dialog names ([`CanvasGesture`]: an eyedropper's press, a shadow drag, Curves'
//! ⌘-click), each a button with the modifiers it needs, its drag included. Every other press stays blocked. Focus,
//! keys and Escape stay with the dialog. One registration holds per app (the latest; the controls of one pass add
//! up), for the root frame it is made in and the next, and only while the modal layer that made it is the top modal: a
//! modal opened over it blocks the canvas again.
//!
//! One test answers a pointer input ([`Hole::answers`]): the point lies in the canvas and no layer lies over the
//! canvas's own layer there (the dialog, a popup above it, a floating panel). The input hook keeps the events the
//! reach admits ([`Hole::admits`]), `areas::layer_at` answers the canvas's layer where the test holds, the wheel and
//! sensing read it, and the scrim leaves the canvas clear and unabsorbed.

use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::input::{self as raw_input, KeyEvent, RawInput};
use crate::runtime::egui::memory::Service;
use crate::runtime::egui::pass;
use crate::runtime::frame::{PerViewport, Stored};
use crate::runtime::{overlay, pointer};
use craft_behavior::PointerButton;
use craft_host::target::Os;
use craft_keymap::{KeyInput, Mods};
use egui::{Context, Event, Id, Pos2, Rect, ViewportId};
use smallvec::SmallVec;

/// A gesture on the canvas that a dialog takes under its modal layer: a press of `button` with at least `mods` held,
/// and the drag that follows it. Other modifiers may be held too: an eyedropper's Shift and Alt add and subtract.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CanvasGesture {
    /// The button that starts it.
    pub button: PointerButton,
    /// The modifiers it needs held at the press.
    pub mods: Mods,
}

impl CanvasGesture {
    /// A press of the primary button, with any modifiers: an eyedropper, a shadow or pattern drag.
    pub const PRIMARY: CanvasGesture = CanvasGesture::press(PointerButton::Primary);

    /// A press of `button`, with any modifiers.
    pub const fn press(button: PointerButton) -> CanvasGesture {
        CanvasGesture { button, mods: Mods::NONE }
    }

    /// The same gesture, needing `mods` held at the press.
    #[must_use]
    pub const fn with(mut self, mods: Mods) -> CanvasGesture {
        self.mods = mods;
        self
    }

    /// Whether a press of `button` with `held` (read for `os`) starts this gesture.
    fn starts(self, button: PointerButton, held: Mods, os: Os) -> bool {
        let need = self.mods.effective(os);
        let held = held.effective(os);
        button == self.button && (!need.cmd || held.cmd) && (!need.ctrl || held.ctrl) && (!need.alt || held.alt) && (!need.shift || held.shift)
    }
}

/// What a modal leaves of the canvas below it: the view navigation, and the gestures it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Reach {
    gestures: SmallVec<[CanvasGesture; 2]>,
}

impl Reach {
    /// The view navigation alone.
    pub(crate) fn view() -> Reach {
        Reach::default()
    }

    /// The view navigation and `gesture`.
    pub(crate) fn gesture(gesture: CanvasGesture) -> Reach {
        Reach { gestures: SmallVec::from_slice(&[gesture]) }
    }

    /// What this reach and `other` let through together.
    fn join(mut self, other: Reach) -> Reach {
        for gesture in other.gestures {
            if !self.gestures.contains(&gesture) {
                self.gestures.push(gesture);
            }
        }
        self
    }
}

/// The canvas a dialog keeps open, how far, and the modal layer that made the registration.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Armed {
    canvas: Id,
    reach: Reach,
    /// The modal layer the registration belongs to.
    modal: Id,
}

/// The armed canvas in one viewport, and how far the modal lets input through.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Hole {
    /// The viewport that shows the canvas.
    pub viewport: ViewportId,
    /// The canvas component's id.
    pub canvas: Id,
    /// The layer the canvas's shapes are on.
    pub layer: Layer,
    /// The canvas's screen rect.
    pub rect: Rect,
    pub reach: Reach,
    /// The platform the gestures' modifiers are read for.
    os: Os,
    /// Space is held over the canvas.
    pub space: bool,
    /// A Space press that the hook read this pass belongs to the canvas: the key stage takes it as the canvas's
    /// momentary command instead of the dialog's key.
    pub space_pressed: bool,
    /// The screen rects of the layers over the canvas's layer, as of the last pass that took the snapshot.
    covers: Vec<Rect>,
}

impl Hole {
    /// The one let-through test: the canvas answers a pointer at `at` when `at` lies in its rect and no layer lies
    /// over its layer there.
    pub(crate) fn answers(&self, at: Pos2) -> bool {
        self.rect.contains(at) && !self.covers.iter().any(|cover| cover.contains(at))
    }

    /// Whether the canvas takes a press of `button` with `modifiers` at `at`: the middle press and, with Space held,
    /// the primary one pan; any other press starts a gesture the dialog named; and the canvas answers.
    pub(crate) fn admits_press(&self, button: egui::PointerButton, modifiers: &egui::Modifiers, at: Pos2) -> bool {
        let reached = match button {
            egui::PointerButton::Middle => true,
            egui::PointerButton::Primary if self.space => true,
            _ => {
                let (button, held) = (crate::runtime::egui::input::button_of(button), Mods::from_egui(modifiers, self.os));
                self.reach.gestures.iter().any(|gesture| gesture.starts(button, held, self.os))
            }
        };
        reached && self.answers(at)
    }

    /// Whether the modal lets `event` through: a press its reach admits and the wheel, each where the canvas answers.
    /// `pointer` is where the pointer stood before the event.
    pub(crate) fn admits(&self, event: &Event, pointer: Option<Pos2>) -> bool {
        match event {
            Event::PointerButton { pos, button, pressed: true, modifiers } => self.admits_press(*button, modifiers, *pos),
            Event::MouseWheel { .. } => pointer.is_some_and(|at| self.answers(at)),
            _ => false,
        }
    }
}

/// The registrations, each with the root frame it was made in, and what the input hook follows of Space and the
/// pointer.
#[derive(Default)]
struct Registry {
    /// The dialog's part: the canvas a modal body keeps, its reach and that modal.
    body: Option<(u64, Armed)>,
    /// The host's part: the canvas whose tool the host runs and the gestures the tool takes. It holds only beside a
    /// body part for the same canvas, whatever order the two are made in.
    tool: Option<(u64, Id, Reach)>,
    /// Space was pressed over the canvas and not released: the canvas has it.
    space: bool,
    /// The pointer stands where the canvas answers.
    over: bool,
}

/// Registers `canvas` with `reach` for the modal layer whose body is running. Made outside every modal body it is
/// dropped, so a picker armed in a panel passes no press under a modal that opens later. `Sampler::pass_canvas_gesture`
/// and `Sampler::keep_view_open` (contract layer) call it.
pub(crate) fn register(ctx: &Context, canvas: Id, reach: Reach) {
    if !crate::runtime::measure::effect(ctx, "sampler::register") {
        return;
    }
    let now = pass::root_frame_nr(ctx);
    let Some(modal) = overlay::modal_drawing(ctx) else { return };
    Service::<Registry>::run(ctx, |registry| {
        let reach = match registry.body.take() {
            Some((made, held)) if made == now && held.canvas == canvas && held.modal == modal => held.reach.join(reach),
            _ => reach,
        };
        registry.body = Some((now, Armed { canvas, reach, modal }));
    });
}

/// The canvas host's entry (`ToolHost`): passes `gesture` on `canvas`, the canvas whose tool the host runs. It takes
/// effect only while a modal body keeps the same canvas (`Sampler::keep_view_open` or `pass_canvas_gesture`), and
/// then for that modal: a modal that keeps nothing, an alert say, leaves the canvas blocked. The host calls it in
/// every pass for the gestures its running tool takes; controls use `Sampler::pass_canvas_gesture`.
pub fn pass_tool_gesture(ctx: &Context, canvas: Id, gesture: CanvasGesture) {
    if !crate::runtime::measure::effect(ctx, "sampler::pass_tool_gesture") {
        return;
    }
    let now = pass::root_frame_nr(ctx);
    let reach = Reach::gesture(gesture);
    Service::<Registry>::run(ctx, |registry| {
        let reach = match registry.tool.take() {
            Some((made, held, gestures)) if made == now && held == canvas => gestures.join(reach),
            _ => reach,
        };
        registry.tool = Some((now, canvas, reach));
    });
}

/// The body part made in this root frame or the last (one per app, the latest wins), with the host's gestures for the
/// same canvas made in that window added.
fn armed(ctx: &Context) -> Option<Armed> {
    let now = pass::root_frame_nr(ctx);
    let live = |made: u64| made + 1 >= now;
    Service::<Registry>::run(ctx, |registry| {
        let (_, mut armed) = registry.body.clone().filter(|(made, _)| live(*made))?;
        if let Some((_, _, gestures)) = registry.tool.clone().filter(|(made, canvas, _)| live(*made) && *canvas == armed.canvas) {
            armed.reach = armed.reach.join(gestures);
        }
        Some(armed)
    })
}

/// The armed canvas, while the modal layer that registered it is the top modal and no popup stands over a modal.
/// The layers over the canvas are left for the caller that can read them.
fn open_hole(ctx: &Context) -> Option<Hole> {
    let armed = armed(ctx)?;
    let modal = armed.modal;
    let (viewport, layer, rect) = pointer::root_bounds(ctx, armed.canvas)?;
    let held = overlay::is_top_modal(ctx, modal)
        && overlay::top_modal_in(ctx, viewport).is_none_or(|top| top.id == modal)
        && !overlay::popup_over_modal(ctx);
    let space = Service::<Registry>::run(ctx, |registry| registry.space);
    let os = crate::runtime::windows::os_of(ctx);
    held.then_some(Hole { viewport, canvas: armed.canvas, layer, rect, reach: armed.reach, os, space, space_pressed: false, covers: Vec::new() })
}

/// The canvas a modal layer keeps open in this pass, while the registration holds.
pub(crate) fn held_canvas(ctx: &Context) -> Option<Id> {
    open_hole(ctx).map(|hole| hole.canvas)
}

/// The snapshot of the last pass of a viewport.
#[derive(Default)]
struct Seen {
    /// The pass the snapshot was taken in.
    pass: Option<u64>,
    hole: Option<Hole>,
}

impl Stored for Seen {}

/// The hole of the running viewport's pass, computed once per pass on the first read and read from then on.
/// `wheel::begin` reads it at the start of every pass.
pub(crate) fn hole_of_pass(ctx: &Context) -> Option<Hole> {
    let viewport = pass::current_viewport(ctx);
    let nr = pass::pass_nr(ctx, viewport);
    let seen = PerViewport::<Seen>::with(ctx, viewport, |seen| (seen.pass == Some(nr)).then(|| seen.hole.clone()));
    if let Some(hole) = seen {
        return hole;
    }
    let hole = open_hole(ctx).filter(|hole| hole.viewport == viewport).map(|mut hole| {
        let scrim = overlay::top_modal_in(ctx, viewport).and_then(|top| top.cover);
        hole.covers = areas::rects_over(ctx, hole.layer, scrim);
        hole
    });
    PerViewport::<Seen>::with(ctx, viewport, |seen| {
        seen.pass = Some(nr);
        seen.hole = hole.clone();
    });
    hole
}

/// The canvas's layer where the canvas answers the running viewport's pointer at `at`.
pub(crate) fn layer_at(ctx: &Context, at: Pos2) -> Option<Layer> {
    hole_of_pass(ctx).filter(|hole| hole.answers(at)).map(|hole| hole.layer)
}

/// Whether the canvas took the primary press at `at` in the running pass: it moves no key focus.
pub(crate) fn admitted_press(ctx: &Context, at: Pos2) -> bool {
    let Some(hole) = hole_of_pass(ctx) else { return false };
    let modifiers = ctx.input(|input| {
        input
            .events
            .iter()
            .find_map(|event| match event {
                Event::PointerButton { pos, button: egui::PointerButton::Primary, pressed: true, modifiers } if *pos == at => Some(*modifiers),
                _ => None,
            })
            .unwrap_or(input.modifiers)
    });
    hole.admits_press(egui::PointerButton::Primary, &modifiers, at)
}

/// Whether the root `root` may start a press that egui gave no target at `at`: where the canvas answers, only the
/// armed canvas.
pub(crate) fn takes_untargeted(ctx: &Context, root: Option<Id>, at: Pos2) -> bool {
    match hole_of_pass(ctx) {
        Some(hole) if hole.answers(at) => root == Some(hole.canvas),
        _ => true,
    }
}

/// Whether the wheel at `at` reaches the takers of the running viewport: with a modal layer open here, only where the
/// canvas answers or inside the modal's own family.
pub(crate) fn wheel_allowed(ctx: &Context, at: Pos2) -> bool {
    let viewport = pass::current_viewport(ctx);
    overlay::top_modal_in(ctx, viewport).is_none() || hole_of_pass(ctx).is_some_and(|hole| hole.answers(at)) || areas::in_modal_family(ctx, at)
}

/// The canvas rect the cover of the modal layer `modal`, shown in `viewport`, leaves clear: the canvas, when that
/// layer made the registration.
pub(crate) fn through(ctx: &Context, viewport: ViewportId, modal: Id) -> Option<Rect> {
    let hole = open_hole(ctx)?;
    (armed(ctx)?.modal == modal && hole.viewport == viewport).then_some(hole.rect)
}

/// The input hook's step for the canvas a modal keeps open, on the events of any viewport. It follows where the
/// pointer stands, keeps Space as held state while the pointer is over the canvas (the key stage hands the press to the canvas's momentary command)
/// (the primary press then pans), and, when the modal is in the canvas's own viewport, removes the presses on the
/// canvas that the reach does not admit: egui's modal gate leaves such a canvas with no press target, which would let
/// any press through that the scrim's clear rect uncovers. Returns the hole as the hook sees it and whether it
/// removed an event.
pub(crate) fn hook(ctx: &Context, input: &mut RawInput) -> (Option<Hole>, bool) {
    let Some(mut hole) = open_hole(ctx) else {
        Service::<Registry>::run(ctx, |registry| (registry.space, registry.over) = (false, false));
        return (None, false);
    };
    let here = hole.viewport == input.viewport_id;
    if here {
        hole.covers = PerViewport::<Seen>::with(ctx, hole.viewport, |seen| seen.hole.as_ref().map(|seen| seen.covers.clone()).unwrap_or_default());
    }
    let barred = here && overlay::top_modal_in(ctx, hole.viewport).is_some();
    let (mut space, mut over) = Service::<Registry>::run(ctx, |registry| (registry.space, registry.over));
    let before = input.events.len();
    let os = crate::runtime::windows::os_of(ctx);
    let latin = input.latin_layout_chars.clone();
    let is_space = |key: &KeyInput| key.physical.or(key.typed) == Some(craft_keymap::Key::Space);
    input.events.retain(|event| {
        match event {
            Event::PointerMoved(pos) if here => over = hole.answers(*pos),
            Event::PointerGone if here => over = false,
            Event::PointerButton { pos, .. } if here => over = hole.answers(*pos),
            _ => {}
        }
        match raw_input::key_event(event, os, &latin) {
            KeyEvent::Press { input: key, .. } if is_space(&key) && (space || over) => {
                space = true;
                hole.space = true;
                hole.space_pressed = true;
                true
            }
            KeyEvent::Release { input: key } if is_space(&key) && space => {
                space = false;
                hole.space = false;
                true
            }
            _ => !matches!(event, Event::PointerButton { pos, pressed: true, .. } if barred && hole.answers(*pos) && !hole.admits(event, None)),
        }
    });
    hole.space = space;
    Service::<Registry>::run(ctx, |registry| (registry.space, registry.over) = (space, over));
    (Some(hole), input.events.len() != before)
}
