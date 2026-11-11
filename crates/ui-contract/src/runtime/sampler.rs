//! The canvas reach under a modal layer (15.5, 8.3): the one exception to "a modal blocks the input below it".
//!
//! A modal dialog that shows a canvas keeps it usable, as Photoshop's Color Picker, Levels and Curves do. A control
//! of the dialog registers the canvas every pass with one of two reaches. `Reach::View` lets the view navigation
//! through: the wheel (scroll and zoom) and the middle press that pans. `Reach::Sample` adds the primary press, for
//! the eyedropper a control of the dialog armed. Space held over the canvas adds the primary press under either
//! reach, for the pan. Focus, keys and Escape stay with the dialog. One registration holds per app (the latest), for
//! the root frame it is made in and the next, and only while the modal layer that made it is the top modal: a modal
//! opened over it blocks the canvas again.
//!
//! One test answers a pointer input ([`Hole::answers`]): the point lies in the canvas and no layer lies over the
//! canvas's own layer there (the dialog, a popup above it, a floating panel). The input hook keeps the events the
//! reach admits ([`Hole::admits`]), `areas::layer_at` answers the canvas's layer where the test holds, the wheel and
//! sensing read it, and the scrim leaves the canvas clear and unabsorbed.

use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::input::{self as raw_input, KeyEvent, RawInput};
use crate::runtime::egui::memory::Service;
use crate::runtime::egui::pass;
use crate::runtime::frame::PerViewport;
use crate::runtime::{overlay, pointer};
use craft_keymap::KeyInput;
use egui::{Context, Event, Id, PointerButton, Pos2, Rect, ViewportId};

/// What a modal leaves of the canvas below it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) enum Reach {
    /// View navigation: the wheel and the middle press.
    View,
    /// View navigation and the primary press, for the sampler an armed control runs on the canvas.
    Sample,
}

/// The canvas a dialog keeps open, how far, and the modal layer that made the registration.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Armed {
    canvas: Id,
    reach: Reach,
    /// The modal layer the registering control belongs to; `None` for a control outside any modal layer.
    modal: Option<Id>,
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

    /// Whether the canvas takes a press of `button` at `at`: the reach admits the button and the canvas answers.
    pub(crate) fn admits_press(&self, button: PointerButton, at: Pos2) -> bool {
        let reached = match button {
            PointerButton::Primary => self.reach == Reach::Sample || self.space,
            PointerButton::Middle => true,
            _ => false,
        };
        reached && self.answers(at)
    }

    /// Whether the modal lets `event` through: a press its reach admits and the wheel, each where the canvas answers.
    /// `pointer` is where the pointer stood before the event.
    pub(crate) fn admits(&self, event: &Event, pointer: Option<Pos2>) -> bool {
        match event {
            Event::PointerButton { pos, button, pressed: true, .. } => self.admits_press(*button, *pos),
            Event::MouseWheel { .. } => pointer.is_some_and(|at| self.answers(at)),
            _ => false,
        }
    }
}

/// The registration, the root frame it was made in, and what the input hook follows of Space and the pointer.
#[derive(Default)]
struct Registry {
    made: Option<(u64, Armed)>,
    /// Space was pressed over the canvas and not released: the canvas has it.
    space: bool,
    /// The pointer stands where the canvas answers.
    over: bool,
}

/// Registers `canvas` with `reach` for the modal layer whose body is running. `Cx::arm_sampler` and
/// `Cx::keep_view_open` (contract layer) call it.
pub(crate) fn register(ctx: &Context, canvas: Id, reach: Reach) {
    if !crate::runtime::measure::effect(ctx, "sampler::register") {
        return;
    }
    let now = pass::root_frame_nr(ctx);
    let modal = overlay::modal_drawing(ctx);
    Service::<Registry>::run(ctx, |registry| {
        let reach = match registry.made {
            Some((made, held)) if made == now && held.canvas == canvas && held.modal == modal => held.reach.max(reach),
            _ => reach,
        };
        registry.made = Some((now, Armed { canvas, reach, modal }));
    });
}

/// The registration made in this root frame or the last: one per app, the latest wins.
fn armed(ctx: &Context) -> Option<Armed> {
    let now = pass::root_frame_nr(ctx);
    Service::<Registry>::run(ctx, |registry| registry.made.filter(|(made, _)| made + 1 >= now).map(|(_, armed)| armed))
}

/// The armed canvas, while the modal layer that registered it is the top modal and no popup stands over a modal.
/// The layers over the canvas are left for the caller that can read them.
fn open_hole(ctx: &Context) -> Option<Hole> {
    let armed = armed(ctx)?;
    let modal = armed.modal?;
    let (viewport, layer, rect) = pointer::root_bounds(ctx, armed.canvas)?;
    let held = overlay::is_top_modal(ctx, modal)
        && overlay::top_modal_in(ctx, viewport).is_none_or(|top| top.id == modal)
        && !overlay::popup_over_modal(ctx);
    let space = Service::<Registry>::run(ctx, |registry| registry.space);
    held.then_some(Hole { viewport, canvas: armed.canvas, layer, rect, reach: armed.reach, space, space_pressed: false, covers: Vec::new() })
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
    hole_of_pass(ctx).is_some_and(|hole| hole.admits_press(PointerButton::Primary, at))
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
    (armed(ctx)?.modal == Some(modal) && hole.viewport == viewport).then_some(hole.rect)
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
