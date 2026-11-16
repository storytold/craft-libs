//! `ToolHost` (8.3): maps a canvas's background press, hover and keys to the calls of a [`CanvasTool`].
//!
//! The host runs after `Canvas::show`, with the [`CanvasOutput`] the canvas returned:
//!
//! 1. `CanvasOutput::press` is the press the arena's winner left for the background: a press on a canvas control
//!    part never reaches the tool. One pass carries one phase; a click inside one pass reaches the tool as `press` then
//!    `release`, and a press that moved past slop before it came up as `press`, `drag` and `release`: the tool decides
//!    a drag from whether `drag` was called.
//! 2. `takes(button, mods)` decides whether a button starts a gesture. A middle-button pan belongs to the canvas.
//! 3. Escape during a drag arrives as a cancelled press ([`PressCancel`]) and calls `cancel` once.
//! 4. Keys the canvas holds as intents (arrows, Enter, Escape) go to `key` while the canvas has focus; a text part
//!    inside the canvas takes its own keys before they are routed here. Enter and Escape carry Alt and Shift
//!    (`ToolContext::mods`). While nothing in the window has focus, the arrows, Enter and Escape are offered too, so a
//!    tool holding a gesture or a pending edit commits and cancels before the canvas is clicked; one the tool leaves
//!    goes on to the scopes around it. A focused field or control keeps its own Enter and Escape. A session control
//!    shown in the canvas body (a crop frame, a transform box: `KeyUse::session`) takes these keys first.
//! 5. The deletes and the plain keys are the app's unless the tool claims them ([`CanvasTool::claims`]). The host
//!    reads the claims at the end of every pass, after the pointer events and keys, and holds the claimed chords from
//!    the app's shortcuts in the next pass: a plain Delete with nothing to delete runs the app's Edit > Clear.
//!    A tool that claims the nudges has AccessKit Increment and Decrement on its node, and they reach `key` as the
//!    arrows toward and away from the reading end.
//! 6. The tool paints over the canvas and requests its cursor at tool priority.
//! 7. A drag past the canvas edge ([`CanvasTool::autoscrolls`]), and a hover in the band inside it
//!    ([`CanvasTool::autoscrolls_on_hover`]), report [`ToolHostResponse::past_edge`] and the pass's
//!    [`ToolHostResponse::scroll`]; the app moves its view by it with [`Autoscroll::scroll`].
//!
//! The host keeps its state under the canvas's id (`CanvasOutput::response.id`), so two canvases never share a
//! gesture, and hands the tool the canvas's view from the output ([`ToolContext::space`]). A tool changes the view
//! with [`ToolContext::set_view`]; the host keeps it inside the document as the canvas does (Overscroll off) and
//! reports the last one in [`ToolHostResponse::view`]. A view set with [`ToolContext::ease_view`] eases in the
//! canvas's one zoom ease, which the canvas shows from its next pass. `run` also paints the tool's readout;
//! [`ToolHostResponse::readout`] repeats it as data.
//!
//! The context carries the pass's env ([`ToolContext::units`], [`ToolContext::words`]), the canvas's snapping
//! ([`Canvas::snap`](super::Canvas::snap), read with [`ToolContext::snap`]) and, in `paint`, the style context ([`ToolContext::style`]). After
//! `paint`, `run` emits what [`CanvasTool::access`] returns as nodes under the canvas's node.
//!
//! Space and the momentary Hand: while a gesture runs the host withholds Space from the shortcuts for the tool, so a
//! Space pressed mid-drag never starts a momentary Hand; the tool reads it with [`ToolContext::held`]. Away from a
//! gesture, Space is the [`MomentaryCommands`](super::MomentaryCommands) Hand, routed to the canvas like the view
//! commands. [`ToolHost::momentary`] takes the app's [`Momentary`] set and `run` feeds it the commands fired for the
//! canvas, so the app plumbs nothing: while a momentary key is held the host runs the set's Hand or Zoom tool in
//! place of the selected tool, and a gesture keeps the tool it began with. An app that switches tools itself from
//! other input asks [`ToolHost::in_gesture`] first and keeps the tool it started the gesture with.
//!
//! Stand-ins: [`ToolHost::stand_in`] gives the tool a press runs while Command or Alt is held and the selected tool
//! does not take the press, or answers it [`ToolAction::Ignored`]. The order is the momentary tool, then the Command
//! stand-in, then the Alt stand-in, then the selected tool, and the gesture keeps the tool it began with. While the key
//! is held and no gesture runs, hover, cursor and paint are the stand-in's. [`ToolHost::cancel`], on a host built
//! with the same stand-ins and momentary set, ends a gesture with the tool that holds it, so the app never decides
//! which tool that is.
//!
//! Automation: [`ToolHost::inject`] hands each input to the window host's injected input of the canvas's viewport,
//! which delivers it one step per pass in the order injected. A press, move or release is the pointer's own event,
//! with the pen's reading, so it reaches the tool through the canvas's press as a hand's does, and the pass pointer
//! and everything that reads it see it. A held key ([`ToolInput::hold`]) is a step with no key event: it is down
//! ([`ToolContext::held`]) from its pass until the pass of its [`ToolInput::let_go`], and a held key the keymap binds
//! to a [`MomentaryCommands`](super::MomentaryCommands) command runs that momentary tool as the real key does. The window
//! host ends such keys where it drops injected input (a withdrawal, a focus loss, a cancel, the end of the session).
//!
//! Readouts: a tool's [`ToolContext::readout`] rows paint only while the root env's `ShowReadouts` holds; its
//! [`ToolContext::hud`] rows always paint. A secondary press the tool did not take is reported in
//! [`ToolHostResponse::context`], and so are a secondary click with the momentary Hand (whose drag pans) and the Menu
//! key.
//!
//! ```no_run
//! # extern crate craft_ui_canvas as craft_ui;
//! use craft_ui::canvas::{Canvas, CanvasTool, Momentary, ToolHost};
//! use craft_ui::contract::ShowContainer as _;
//! # fn demo(ui: &mut egui::Ui, canvas: Canvas, tool: &mut dyn CanvasTool, momentary: &mut Momentary, view: &mut craft_behavior::CanvasView) {
//! let shown = canvas.show(ui, |_, _| {});
//! let reply = ToolHost::new().momentary(momentary).run(ui, &shown.output, tool);
//! if let Some(next) = reply.view { *view = next; }
//! # }
//! ```

use super::frame_step;
use super::momentary::{Momentary, MomentaryCommands, MomentaryTool};
use super::navigate::Autoscroll;
use super::readout::{paint_readout, shown as readouts_shown};
use super::tool::{CanvasTool, March, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPointer, ToolRequests};
use super::zoom_motion::{self, Steer};
use super::{CanvasOutput, CanvasSpace, Snap, context_click};
use crate::canvas_controls::paint_lines;
use crate::canvas_controls::{SmartGuides, SmartLine};
use crate::contract::{Component, Meta as _};
use crate::primitives::{ContentPainterCx, Draw, DrawEnv, PaintRecord};
use crate::runtime::access::{self, Name, Role, SubDecl, SubNode};
use crate::runtime::commands;
use crate::runtime::edit::next_gesture_id;
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::{global, with_session};
use crate::runtime::overlay::{self, OpenFilter};
use crate::runtime::pointer::{self, PointerFrame};
use crate::runtime::sampler::{self, CanvasGesture};
use crate::runtime::windows::{InjectedPointer, WindowHost};
use crate::runtime::{cursor, frame, keys, lock_keys, measure};
use crate::style::{DataColor, StyleCx, resolve_style};
use craft_behavior::geometry::SnapLine;
use craft_behavior::kurbo;
use craft_behavior::{
    AccessAction, AccessRequest, CancelReason, CanvasView, Cursor, CursorPriority, GestureId, GesturePhase, Intent, PartKey, PartRef, PointerButton,
    PointerEvent, PointerSample, PressCancel, PressPhase, PressStart, PressState,
};
use craft_keymap::{Chord, Key, Mods};
use craft_theme::Variant;
use egui::{Context, Id, Pos2, Rect, Ui, Vec2, ViewportId};
use std::cell::RefCell;

/// A pointer event or held key from automation, handed to the window host's injected input with
/// [`ToolHost::inject`] and delivered one step per pass in the order injected, as the pointer and the keyboard would
/// deliver it. Positions are screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToolInput {
    /// A press of `button`, after a move to `pos` when the injected pointer is elsewhere. It reaches the tool as a
    /// hand's press does, through the canvas's press: a press on a canvas control part or off the canvas is not the
    /// tool's, and presses close together in time and place count as a double press.
    #[non_exhaustive]
    Down {
        /// The button.
        button: PointerButton,
        /// The pointer.
        pos: Pos2,
        /// The pen's reading; `None` for a mouse.
        sample: Option<PointerSample>,
        /// The modifiers held.
        mods: Mods,
    },
    /// The pointer moved: a drag while a gesture runs, else a hover.
    #[non_exhaustive]
    Move {
        /// The pointer.
        pos: Pos2,
        /// The pen's reading; `None` for a mouse.
        sample: Option<PointerSample>,
        /// The modifiers held.
        mods: Mods,
    },
    /// The held button comes up at `pos`, after a move there; the modifiers go with it when no button or key stays
    /// down.
    #[non_exhaustive]
    Up {
        /// The pointer.
        pos: Pos2,
        /// The modifiers held.
        mods: Mods,
    },
    /// A key goes down with no key event: the passes from this one on run with the key held
    /// ([`ToolContext::held`]) until its [`ToolInput::LetGo`].
    #[non_exhaustive]
    Hold {
        /// The key.
        key: Key,
    },
    /// A key a [`ToolInput::Hold`] held comes up: the passes after it run without it.
    #[non_exhaustive]
    LetGo {
        /// The key.
        key: Key,
    },
}

impl ToolInput {
    /// A mouse press of `button` at `pos`, with no modifiers.
    pub const fn down(button: PointerButton, pos: Pos2) -> Self {
        ToolInput::Down { button, pos, sample: None, mods: Mods::NONE }
    }

    /// A mouse move to `pos`, with no modifiers.
    pub const fn moved(pos: Pos2) -> Self {
        ToolInput::Move { pos, sample: None, mods: Mods::NONE }
    }

    /// The button coming up at `pos`, with no modifiers.
    pub const fn up(pos: Pos2) -> Self {
        ToolInput::Up { pos, mods: Mods::NONE }
    }

    /// `key` going down, held until its [`ToolInput::let_go`].
    pub const fn hold(key: Key) -> Self {
        ToolInput::Hold { key }
    }

    /// `key`, held by an earlier [`ToolInput::hold`], coming up.
    pub const fn let_go(key: Key) -> Self {
        ToolInput::LetGo { key }
    }

    /// The input with the modifiers `held`; a held key carries none.
    #[must_use]
    pub fn mods(mut self, held: Mods) -> Self {
        match &mut self {
            ToolInput::Down { mods, .. } | ToolInput::Move { mods, .. } | ToolInput::Up { mods, .. } => *mods = held,
            ToolInput::Hold { .. } | ToolInput::LetGo { .. } => {}
        }
        self
    }

    /// The input as a pen's, with its reading `pen`; a button coming up and a held key have none.
    #[must_use]
    pub fn sample(mut self, pen: PointerSample) -> Self {
        match &mut self {
            ToolInput::Down { sample, .. } | ToolInput::Move { sample, .. } => *sample = Some(pen),
            ToolInput::Up { .. } | ToolInput::Hold { .. } | ToolInput::LetGo { .. } => {}
        }
        self
    }
}

/// Labelled values for the pointer's side, as a tool handed them over with [`ToolContext::readout`].
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolReadout {
    /// The pointer on screen.
    pub at: Pos2,
    /// The rows: a label and its value.
    pub rows: Vec<(Label, String)>,
}

/// What the host did in a pass.
#[derive(Clone, Debug, Default, PartialEq)]
#[must_use = "apply the view and the edge overshoot to the app's view and read the gesture"]
#[non_exhaustive]
pub struct ToolHostResponse {
    /// The gesture's phase, when one began, moved or ended this pass. A pass that began and ended a gesture reports
    /// the end.
    pub gesture: Option<GesturePhase>,
    /// The press the tool answered with [`ToolAction::Ignored`]: the app may use it.
    pub ignored_press: Option<ToolPointer>,
    /// The key the tool took this pass.
    pub took_key: Option<ToolKey>,
    /// How far past the edge the pointer is, in screen points: past the canvas edge while a gesture of a tool that
    /// autoscrolls runs, or into the band inside the view's edge (`size.hit.edge`) while a tool that autoscrolls on
    /// hover hovers there.
    pub past_edge: Option<Vec2>,
    /// The autoscroll of this pass, in screen points: the [`Autoscroll`] speed for [`past_edge`](Self::past_edge) over
    /// the pass's step of the runtime's pass clock. The host moves no view: the app moves its own with
    /// `Autoscroll::scroll(&mut view, scroll, size)` and asks for the next pass while it moves.
    pub scroll: Option<Vec2>,
    /// The cursor the tool asked for, after the host requested it.
    pub cursor: Option<Cursor>,
    /// The values the tool wants beside the pointer. The host has painted them already; this is the data for
    /// automation.
    pub readout: Option<ToolReadout>,
    /// The view a tool set with [`ToolContext::set_view`] this pass (the last one, inside the document with Overscroll
    /// off), else the canvas's own ([`CanvasOutput::view`]): the one view the app copies into its own. A view that
    /// eases is its target; the canvas shows the way there.
    pub view: Option<CanvasView>,
    /// The snap lines of this pass from `snap_rect` and `add_snapped`.
    pub snapped: Vec<SnapLine>,
    /// Where the app opens its canvas menu: a secondary press on the background the running tool did not take, a
    /// secondary click with the momentary Hand (its secondary drag pans), or the Menu key or Shift+F10.
    pub context: Option<ContextRequest>,
    /// The document distance the view centre moved this pass from a `pan_view`, after the Overscroll clamp.
    pub pan: Option<ViewPan>,
}

/// Where and how the app opens its canvas menu, from [`ToolHostResponse::context`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContextRequest {
    /// The pointer on screen, or the view centre for a key.
    pub at: Pos2,
    /// The modifiers held when the secondary button went down, or those held at the key.
    pub mods: Mods,
    /// The momentary tool in effect at the press (a Space-held Hand).
    pub momentary: Option<MomentaryTool>,
}

/// A pan of the view, from [`ToolHostResponse::pan`].
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewPan {
    /// The document distance the view centre moved.
    pub delta: kurbo::Vec2,
    /// The press's scroll-all-windows choice.
    pub scroll_all: bool,
}

impl ViewPan {
    /// A pan of `delta` document units with the press's `scroll_all` choice.
    pub const fn new(delta: kurbo::Vec2, scroll_all: bool) -> Self {
        ViewPan { delta, scroll_all }
    }
}

/// A key whose hold at a press makes the host run a stand-in tool, from [`ToolHost::stand_in`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum StandInKey {
    /// The platform command key.
    Command,
    /// The Alt key.
    Alt,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    /// The gesture's id, from `runtime::edit::next_gesture_id` when the tool claimed the press.
    id: GestureId,
    last: ToolPointer,
    /// The tool has had a `drag` call in this gesture.
    dragged: bool,
    /// The stand-in the gesture runs with, latched at the press until the gesture ends; `None` is the selected tool.
    stand_in: Option<StandInKey>,
}

/// A secondary press the selected tool refused, or the momentary Hand took: what its menu request carries when the
/// button comes up.
#[derive(Clone, Copy, Debug)]
struct SecondaryPress {
    /// The modifiers held when the button went down.
    mods: Mods,
    /// The momentary tool in effect at the press.
    momentary: Option<MomentaryTool>,
    /// The momentary Hand took the press: it asks for the menu only when its release is a click.
    taken: bool,
}

/// What the host keeps between passes, in a session slot keyed by the canvas's id.
#[derive(Clone, Default)]
struct HostState {
    gesture: Option<Active>,
    /// The open press was refused, ignored or finished whole: its later phases are not the tool's.
    spent: bool,
    /// The keys the tool claimed at the end of the last pass.
    claims: ToolClaims,
    /// The canvas of the last pass, for [`ToolHost::cancel`] outside a pass.
    rect: Option<Rect>,
    space: Option<CanvasSpace>,
    /// The view a tool set while [`ToolHost::cancel`] ended its gesture outside a pass: the next pass reports it.
    cancelled_view: Option<CanvasView>,
    /// The viewport the canvas last ran in: where [`ToolHost::inject`] sends its input.
    viewport: ViewportId,
    /// The pass clock of the last pass that was not a measuring pass: an autoscroll moves by the time since.
    time: Option<f64>,
    /// The open secondary press the tool refused or the momentary Hand took: its release asks for the canvas menu.
    secondary: Option<SecondaryPress>,
}

fn state_id(canvas: Id) -> Id {
    global("craft-ui tool host", canvas)
}

/// Whether the key of a stand-in is held in `mods`.
fn held_at(key: StandInKey, mods: Mods) -> bool {
    match key {
        StandInKey::Command => mods.cmd,
        StandInKey::Alt => mods.alt,
    }
}

/// The tool the host runs for `tool`, the momentary tool in effect: the set's Hand or Zoom, else `selected`.
fn run_as<'t>(set: Option<&'t mut Momentary>, tool: Option<MomentaryTool>, selected: &'t mut dyn CanvasTool) -> &'t mut dyn CanvasTool {
    match set {
        Some(set) => set.run_as(tool, selected),
        None => selected,
    }
}

/// The momentary command the keys injected input holds with no key event (`holds`) run with `mods`: the keymap's
/// command for a key with the pass's modifiers, else for the key alone, since modifiers pressed after a momentary key
/// still count.
fn held_momentary(ctx: &Context, canvas: Id, around: &commands::CommandChain, holds: &[Key], mods: Mods) -> Option<MomentaryCommands> {
    holds.iter().find_map(|key| {
        [mods, Mods::NONE].into_iter().find_map(|mods| commands::bound_around::<MomentaryCommands>(ctx, canvas, around, &Chord::new(mods, *key)))
    })
}

/// The keys down: the window's, and `holds`, those injected input holds with no key event.
fn keys_down(ctx: &Context, holds: &[Key]) -> Vec<Key> {
    let mut down = keys::down(ctx);
    for key in holds {
        if !down.contains(key) {
            down.push(*key);
        }
    }
    down
}

/// The on-canvas tool host. Built each pass, shown once with the canvas's output and the tool.
#[derive(Default)]
pub struct ToolHost<'m> {
    disabled: bool,
    momentary: Option<&'m mut Momentary>,
    command_stand_in: Option<&'m mut (dyn CanvasTool + 'm)>,
    alt_stand_in: Option<&'m mut (dyn CanvasTool + 'm)>,
    smart_guide_color: Option<DataColor>,
}

impl<'m> ToolHost<'m> {
    /// A host that runs the tool of the canvas whose output it is given; the canvas's id keys its state.
    pub fn new() -> Self {
        ToolHost::default()
    }

    /// Whether the host runs the tool. A disabled host cancels a running gesture and delivers nothing; the press it
    /// was disabled during stays spent, so enabling the host mid-press starts no gesture.
    #[must_use]
    pub fn enabled(mut self, enabled: bool) -> Self {
        self.disabled = !enabled;
        self
    }

    /// Runs the momentary tools of `momentary` in place of the selected tool while their keys are held (Space: Hand;
    /// with Cmd or Alt: Zoom). A gesture keeps the tool it began with, and a flick released with the momentary Hand
    /// keeps gliding after the key comes up. `run` feeds the set the [`MomentaryCommands`](super::MomentaryCommands)s fired
    /// for the canvas (the canvas, or the instance of the `canvasView` scope around it, such as a full-window
    /// dialog); the app passes the set and nothing else. Without a set, `run` still takes and drops those commands.
    #[must_use]
    pub fn momentary(mut self, momentary: &'m mut Momentary) -> Self {
        self.momentary = Some(momentary);
        self
    }

    /// The tool a press runs when `key` is held at the press and the selected tool does not take it, latched for the
    /// gesture. Order: `Momentary` first, then Command, then Alt.
    #[must_use]
    pub fn stand_in(mut self, key: StandInKey, tool: &'m mut (dyn CanvasTool + 'm)) -> Self {
        match key {
            StandInKey::Command => self.command_stand_in = Some(tool),
            StandInKey::Alt => self.alt_stand_in = Some(tool),
        }
        self
    }

    /// The user's Smart Guide colour; `None` is the guides' own `data.magenta`.
    #[must_use]
    pub fn smart_guide_color(mut self, color: Option<DataColor>) -> Self {
        self.smart_guide_color = color;
        self
    }

    /// Delivers the pass's pointer and key events to `tool`, paints it over the canvas, and paints the readout it
    /// asked for beside the pointer.
    pub fn run(self, ui: &Ui, canvas: &CanvasOutput, tool: &mut dyn CanvasTool) -> ToolHostResponse {
        let ToolHost { disabled, mut momentary, mut command_stand_in, mut alt_stand_in, smart_guide_color } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let key = state_id(canvas.response.id);
        // A measuring run delivers no input: the tool is drawn and sized from the state the last real run left, and
        // the momentary commands and cancelled view wait for the real run.
        let measuring = measure::active(&ctx);
        let mut state = with_session(&ctx, key, |slot: &mut HostState| slot.clone());
        let cancelled_view = if measuring { None } else { state.cancelled_view.take() };
        let space = canvas.space;
        let viewport = WindowHost::of(&ctx).current();
        // The keys injected input holds with no key event are down for the pass, and run their momentary command
        // through the keymap as the window's keys fire it.
        let holds = WindowHost::of(&ctx).injected_holds(viewport);
        let down = keys_down(&ctx, &holds);
        let caps = lock_keys::caps_lock(&ctx);
        let rect = canvas.response.rect;
        let input = pointer::frame(&ctx);
        let fired = std::iter::from_fn(|| {
            if measuring {
                return None;
            }
            commands::take_around::<MomentaryCommands>(&ctx, canvas.response.id, env.view().command_chain())
        });
        let momentary_tool = match momentary.as_deref_mut() {
            Some(momentary) => {
                for (command, event) in fired {
                    momentary.fired(command, event.phase);
                }
                let injected = held_momentary(&ctx, canvas.response.id, env.view().command_chain(), &holds, input.mods);
                momentary.tool_holding(injected, input.mods, state.gesture.is_some())
            }
            None => {
                fired.for_each(drop);
                None
            }
        };
        let (view_only, takes_primary) = {
            let pass = run_as(momentary.as_deref_mut(), momentary_tool, &mut *tool);
            (pass.view_only(), pass.takes(PointerButton::Primary, input.mods))
        };
        if !measuring && view_only {
            sampler::pass_tool_gesture(&ctx, canvas.response.id, CanvasGesture::PRIMARY);
        }
        // A momentary tool takes the next press anywhere on the canvas, over the session members on its body.
        if !measuring {
            pointer::withhold_members(&ctx, canvas.response.id, momentary_tool.is_some());
        }
        // With no gesture and no momentary tool, a held stand-in key makes the stand-in the tool of hover, cursor and
        // paint while the selected tool does not take a primary press.
        let lent = if state.gesture.is_none() && momentary_tool.is_none() && !takes_primary {
            [StandInKey::Command, StandInKey::Alt].into_iter().find(|key| {
                let set = match key {
                    StandInKey::Command => command_stand_in.is_some(),
                    StandInKey::Alt => alt_stand_in.is_some(),
                };
                set && held_at(*key, input.mods)
            })
        } else {
            None
        };
        let mut run = Run {
            selected: &mut *tool,
            set: momentary.as_deref_mut(),
            command: command_stand_in.as_deref_mut(),
            alt: alt_stand_in.as_deref_mut(),
            lent: None,
            stand_ins: momentary_tool.is_none(),
            momentary: momentary_tool,
            space,
            slop: input.slop,
            down: &down,
            caps,
            rect,
            mods: input.mods,
            time: input.time,
            ctx: &ctx,
            env: &env,
            snap: canvas.snap.as_ref(),
            state: &mut state,
            response: ToolHostResponse::default(),
            requests: ToolRequests::default(),
            cancelled: false,
            ending: None,
            hover_scrolls: false,
        };
        if !measuring {
            run.call(None, |tool, cx| tool.each_pass(cx));
            run.hover_scrolls = run.pick_as(None).autoscrolls_on_hover();
            run.lent = lent;
            run.glide();
            if disabled {
                run.cancel(CancelReason::Dismissed);
                run.state.spent |= canvas.press.phase.is_press();
            } else {
                run.phase(&canvas.press, &input);
                run.hover_outside(canvas.response.id, &canvas.press, &input);
                run.keys(&ctx, canvas.response.id, env.direction.is_rtl());
                run.menu_key();
            }
        }
        let Run { mut response, requests, momentary: momentary_tool, .. } = run;
        let before = space.map(|space| space.target());
        if !measuring {
            if let Some(past) = response.past_edge {
                let step = frame_step(input.time - state.time.unwrap_or(input.time));
                response.scroll = Some(Autoscroll::step(past, step as f32));
            }
            state.time = Some(input.time);
        }
        if response.view.is_none() {
            response.view = cancelled_view;
        }
        // A tool's view goes through the canvas's clamp, and its ease through the canvas's one zoom ease; a view that
        // jumps stops that ease.
        if let Some(view) = response.view.as_mut() {
            *view = canvas.clamped(*view);
            if let (Some(scroll_all), Some(before)) = (requests.pan, before) {
                response.pan = Some(ViewPan::new(view.center - before.center, scroll_all));
            }
            let steer = match (requests.ease, canvas.space) {
                (Some(ease), Some(drawn)) => Steer::Ease { from: drawn.view, anchor: ease.anchor },
                _ => Steer::Stop,
            };
            zoom_motion::steer(&ctx, canvas.response.id, steer);
        }
        // The canvas's own view, when no tool moved the view: the app copies one view from the host.
        if response.view.is_none() {
            response.view = canvas.view;
        }
        let phase = canvas.press.phase;
        let over = phase.is_press() || matches!(phase, PressPhase::Hover) || state.gesture.is_some();
        let current = state.gesture.map(|active| active.last).or_else(|| over.then(|| hover_pointer(&input, canvas.press)).flatten());
        // The tool of hover, cursor, paint and claims: the one the running gesture latched, else the stand-in lent for
        // the held key, else the momentary tool in effect, else the selected tool.
        let pass: &dyn CanvasTool = run_as(momentary, momentary_tool, &mut *tool);
        let effective: &dyn CanvasTool = match state.gesture.map_or(lent, |active| active.stand_in) {
            Some(StandInKey::Command) => command_stand_in.as_deref().unwrap_or(pass),
            Some(StandInKey::Alt) => alt_stand_in.as_deref().unwrap_or(pass),
            None => pass,
        };
        let context = || {
            let cx = ToolContext::new(rect, space)
                .pointer(current)
                .mods(input.mods)
                .time(input.time)
                .slop(input.slop)
                .down(&down)
                .caps_lock(caps)
                .gesture(state.gesture.map(|active| active.id))
                .env(&env);
            match canvas.snap.as_ref() {
                Some(snap) => cx.snapping(snap),
                None => cx,
            }
        };
        paint_tool(ui, &env, effective, context());
        if canvas.snap.as_ref().is_some_and(|snap| snap.options().smart_guides) {
            paint_smart_lines(ui, &env, rect, space, &requests.snapped, smart_guide_color);
        }
        response.snapped = requests.snapped;
        let cx = context();
        // A disabled host delivers no key, so it holds none from the app.
        state.claims = if disabled { ToolClaims::NONE } else { effective.claims() };
        if let Some(access) = effective.access(&cx) {
            report_access(&ctx, canvas, rect, &access, state.claims.nudges);
        }
        if over {
            let wanted = effective.cursor(&cx);
            cursor::request(&ctx, wanted.clone(), CursorPriority::Tool);
            response.cursor = Some(wanted);
        }
        if let Some(at) = current.map(|pointer| pointer.pos) {
            let gate = readouts_shown(&ctx);
            // One box per pass: a hud replaces a readout asked for in the same pass.
            let (rows, paints) = if requests.hud.is_some() { (requests.hud, true) } else { (requests.readout, gate) };
            if let Some(rows) = rows {
                if paints {
                    paint_readout(&ctx, at, &rows);
                }
                response.readout = Some(ToolReadout { at, rows });
            }
        }
        if requests.repaint {
            frame::request_repaint(&ctx);
        }
        for chord in claimed_chords(state.claims) {
            keys::claim(&ctx, canvas.response.id, chord);
        }
        if !state.claims.keyboard && state.gesture.is_some() {
            keys::claim(&ctx, canvas.response.id, Chord::new(Mods::NONE, Key::Space));
        }
        state.rect = Some(rect);
        state.space = space;
        state.viewport = viewport;
        with_session(&ctx, key, |slot: &mut HostState| *slot = state);
        response
    }

    /// Hands an automation input for the canvas `canvas` (`CanvasOutput::response.id`) to the window host's injected
    /// input of the viewport the canvas last ran in, behind the input injected before it; it runs one step per pass.
    /// A press, move or release is the pointer's own event with the pen's reading, so it reaches the tool through the
    /// canvas's press as a hand's would. A key is held from the pass of its [`ToolInput::hold`] to the pass of its
    /// [`ToolInput::let_go`], so one lane orders the keys against the pointer; a held key that the keymap binds to a
    /// [`MomentaryCommands`](super::MomentaryCommands) command runs that momentary tool while it is down, as the real
    /// key does (a held Space pans with the Hand). [`WindowHost::injected_idle`] turns true once the last step's pass
    /// ran.
    pub fn inject(ctx: &Context, canvas: Id, input: ToolInput) {
        let viewport = with_session(ctx, state_id(canvas), |state: &mut HostState| state.viewport);
        let host = WindowHost::of(ctx);
        match input {
            ToolInput::Down { button, pos, sample, mods } => {
                host.inject_pointer(viewport, InjectedPointer::down(button, pos, mods, sample));
            }
            ToolInput::Move { pos, sample, mods } => host.inject_pointer(viewport, InjectedPointer::move_to(pos, mods, sample)),
            ToolInput::Up { pos, mods } => host.inject_pointer(viewport, InjectedPointer::up(pos, mods)),
            ToolInput::Hold { key } => host.inject_hold(viewport, key),
            ToolInput::LetGo { key } => host.inject_let_go(viewport, key),
        }
    }

    /// Ends the gesture in progress on the canvas `canvas` now, without a commit (the app switched tools or
    /// documents, a modal opened). Build the host as for [`ToolHost::run`], with the same stand-ins and momentary
    /// set: it cancels the gesture with the tool that holds it, picked as the pass picks it (the stand-in or
    /// momentary tool the press latched, else `selected`). Call it before the app replaces its tools; it runs against
    /// the canvas of the last pass. Returns whether a gesture was cancelled.
    pub fn cancel(self, ctx: &Context, canvas: Id, selected: &mut dyn CanvasTool, reason: CancelReason) -> bool {
        let ToolHost { mut momentary, command_stand_in, alt_stand_in, .. } = self;
        // The press is still held: its later moves and release are not the next gesture's.
        let taken = with_session(ctx, state_id(canvas), |slot: &mut HostState| {
            let taken = slot.gesture.take().map(|active| (active, slot.rect, slot.space, slot.viewport));
            slot.spent |= taken.is_some();
            // A cancelled press of the momentary Hand is no click: its release opens no menu.
            if taken.is_some() {
                slot.secondary = None;
            }
            taken
        });
        let Some((active, rect, space, viewport)) = taken else { return false };
        // A press that latched a stand-in ran without the momentary set, so the stand-in comes first.
        let stand_in = match active.stand_in {
            Some(StandInKey::Command) => command_stand_in,
            Some(StandInKey::Alt) => alt_stand_in,
            None => None,
        };
        let latched = momentary.as_deref_mut().and_then(|momentary| momentary.tool(Mods::NONE, true));
        let tool: &mut dyn CanvasTool = match stand_in {
            Some(stand_in) => stand_in,
            None => run_as(momentary, latched, selected),
        };
        let down = keys_down(ctx, &WindowHost::of(ctx).injected_holds(viewport));
        let mut cx = ToolContext::new(rect.unwrap_or(Rect::NOTHING), space)
            .pointer(Some(active.last))
            .gesture(Some(active.id))
            .down(&down)
            .caps_lock(lock_keys::caps_lock(ctx));
        tool.cancel(reason, &mut cx);
        if let Some(view) = cx.take_requests().view {
            with_session(ctx, state_id(canvas), |state: &mut HostState| state.cancelled_view = Some(view));
            frame::request_repaint(ctx);
        }
        true
    }

    /// Whether the canvas `canvas` has a gesture in progress: the app finishes a drag with the tool it started
    /// with, whatever tool a key or command selects meanwhile, and starts no momentary tool during one.
    pub fn in_gesture(ctx: &Context, canvas: Id) -> bool {
        with_session(ctx, state_id(canvas), |state: &mut HostState| state.gesture.is_some())
    }

    /// Whether a control the body of the canvas `canvas` shows (a canvas control, a box, a handle set) has a gesture
    /// in progress: a drag of one of them, not the tool's (that is [`in_gesture`](Self::in_gesture)). It reads the
    /// controls' own gesture records in the viewport the canvas last ran in; false before the canvas's first pass.
    pub fn controls_pressing(ctx: &Context, canvas: Id) -> bool {
        let viewport = with_session(ctx, state_id(canvas), |state: &mut HostState| state.viewport);
        crate::primitives::member_gesture_running(ctx, viewport, canvas)
    }

    /// The canvas space the host of `canvas` used in its last pass, `None` before its first.
    pub fn shown_space(ctx: &Context, canvas: Id) -> Option<CanvasSpace> {
        with_session(ctx, state_id(canvas), |state: &mut HostState| state.space)
    }
}

/// The part of the node that describes the tool's drawing, below the canvas's node.
const TOOL_NODE: PartKey = PartKey::new("tool");
/// The parts of the nodes that describe the tool's anchors, repeated by the anchor's place in reading order, below
/// the tool's node.
const TOOL_ANCHOR: PartKey = PartKey::new("tool anchor");

/// Emits what the tool is to assistive technology through the sub-node path of the canvas's component: a hover-only
/// graphics object over the drawing's bounds (no size at the canvas centre without bounds), and a child object of no
/// size at each anchor, clamped to the canvas. The object declares Increment and Decrement while the tool claims the
/// nudges. The ids derive from the canvas id, so they stay the same from pass to pass.
fn report_access(ctx: &Context, canvas: &CanvasOutput, rect: Rect, tool: &ToolAccess, nudges: bool) {
    let owner = canvas.response.id;
    let to_screen = |point: kurbo::Point| canvas.space.map(|space| space.view.to_screen(point));
    let bounds = tool.bounds.and_then(|bounds| {
        let corners = [(bounds.x0, bounds.y0), (bounds.x1, bounds.y0), (bounds.x1, bounds.y1), (bounds.x0, bounds.y1)];
        let corners: Option<Vec<Pos2>> = corners.into_iter().map(|(x, y)| to_screen(kurbo::Point::new(x, y))).collect();
        corners.map(|corners| Rect::from_points(&corners).intersect(rect)).filter(|bounds| bounds.is_positive())
    });
    let at_centre = Rect::from_center_size(rect.center(), Vec2::ZERO);
    let tool_part = PartRef::from(TOOL_NODE);
    let mut node = SubNode::new(tool_part, Role::GRAPHICS_OBJECT);
    node.name = Name::Text(tool.name.clone());
    node.description.clone_from(&tool.description);
    if let Some(value) = &tool.value {
        node.value(value.clone());
    }
    if nudges {
        node.steps();
    }
    access::report_sub(ctx, &sub_decl(owner, &node, bounds.unwrap_or(at_centre), owner));
    let parent = access::sub_node_id(owner, tool_part);
    for (index, anchor) in (0u64..).zip(&tool.anchors) {
        let mut node = SubNode::new(PartRef::from((TOOL_ANCHOR, index)), Role::GRAPHICS_OBJECT);
        node.name = Name::Text(anchor.name.clone());
        node.description.clone_from(&anchor.description);
        let place = to_screen(anchor.at).map_or(at_centre, |at| Rect::from_center_size(rect.clamp(at), Vec2::ZERO));
        access::report_sub(ctx, &sub_decl(owner, &node, place, parent));
    }
}

/// An AccessKit Increment or Decrement on the tool's node.
fn is_tool_step(request: &AccessRequest) -> bool {
    matches!(request.action, AccessAction::Increment | AccessAction::Decrement) && request.target == Some(TOOL_NODE.into())
}

/// A sub-node of the canvas that is read only: not focusable, not clickable, never the active part.
fn sub_decl(owner: Id, node: &SubNode, rect: Rect, parent: Id) -> SubDecl<'_> {
    SubDecl { owner, component: "", node, rect, enabled: true, focusable: false, clickable: false, active: false, face: None, label: "", parent }
}

/// Paints the tool over the canvas, clipped to it.
fn paint_tool(ui: &Ui, env: &Env, tool: &dyn CanvasTool, cx: ToolContext<'_>) {
    let painter = ui.painter().with_clip_rect(cx.rect);
    let record = RefCell::new(PaintRecord::default());
    let draw = Draw::new(&painter, &record, DrawEnv { tokens: &env.tokens, direction: env.direction, ink: env.ink });
    let content = ContentPainterCx::new(painter.clone(), env, env.ink);
    let march = March { policy: env.motion, dash: env.tokens.stroke.dash };
    tool.paint(&draw, &cx.painter(&content, march).styled(host_style(ui, env)));
}

/// The style context the host paints with: the secondary variant of the pass's env.
fn host_style<'a>(ui: &'a Ui, env: &'a Env) -> StyleCx<'a> {
    StyleCx::new(
        ui.ctx(),
        &env.tokens,
        &env.values,
        env.size,
        Variant::SECONDARY,
        env.direction,
        env.scale,
        env.os,
        env.active,
        env.key,
        env.line_ratio,
    )
}

/// Paints the smart kinds of the snap `lines` over the canvas through the smart guides' one line loop, in `color` or
/// the theme's smart guide colour. `SmartGuides` itself paints only inside a canvas body, which the host is not in.
fn paint_smart_lines(ui: &Ui, env: &Env, rect: Rect, space: Option<CanvasSpace>, lines: &[SnapLine], color: Option<DataColor>) {
    let Some(space) = space else { return };
    let smart: Vec<SmartLine> = lines.iter().filter(|line| line.kind.is_smart()).map(|line| SmartLine::of(line, space.view.visible())).collect();
    if smart.is_empty() {
        return;
    }
    let content = ContentPainterCx::new(ui.painter().with_clip_rect(rect), env, env.ink);
    let style = resolve_style(&host_style(ui, env), SmartGuides::new(&[], &[]).style_key(), ui.ctx(), <SmartGuides<'static> as Component>::style);
    paint_lines(&content, &space.view, &smart, &style, color);
}

/// The hover sample of the pass: the pointer's latest position with the pen's reading.
fn hover_pointer(frame: &PointerFrame, press: PressState) -> Option<ToolPointer> {
    let pos = frame.pos?;
    Some(
        ToolPointer::new(pos, PointerButton::Primary, frame.time)
            .mods(press.mods)
            .sample(press.sample.or(frame.sample).unwrap_or(PointerSample::new(1.0))),
    )
}

/// Every key without Ctrl or Cmd, with and without Shift: the keys a tool that kept the keyboard types with. The
/// app's chords (Cmd+Z, Cmd+S) still run. Enter, Escape, Delete and Backspace are left out: Enter and Escape reach the
/// tool as `Submit` and `Cancel` with their modifiers in `ToolContext::mods`, and the deletes as [`delete_chords`].
fn kept_chords() -> impl Iterator<Item = Chord> {
    Key::ALL
        .iter()
        .filter(|key| !matches!(key, Key::Enter | Key::Escape | Key::Delete | Key::Backspace))
        .flat_map(|key| [Mods::NONE, Mods::SHIFT].map(|mods| Chord::new(mods, *key)))
}

/// The deletes a tool that claims them takes: the plain ones, and with the keyboard kept also with Alt held. They
/// reach it as `ToolKey::Delete`. Alt+Backspace is otherwise the app's (Photoshop's Fill with the foreground colour);
/// a polygonal lasso keeps the keyboard while its outline is open, so Backspace removes its last point while Alt
/// holds the straight-segment switch.
fn delete_chords(claims: ToolClaims) -> impl Iterator<Item = Chord> {
    let mods: &[Mods] = match (claims.deletes, claims.keyboard) {
        (false, _) => &[],
        (true, false) => &[Mods::NONE],
        (true, true) => &[Mods::NONE, Mods::ALT],
    };
    mods.iter().flat_map(|mods| [Key::Delete, Key::Backspace].map(|key| Chord::new(*mods, key)))
}

/// Every chord `claims` holds from the app's shortcuts.
fn claimed_chords(claims: ToolClaims) -> impl Iterator<Item = Chord> {
    kept_chords().filter(move |_| claims.keyboard).chain(delete_chords(claims)).chain(claims.keys.iter().copied())
}

/// The sample of a press at `pos` taken at `time`: the button the press runs with, and its sample.
fn press_pointer(start: PressStart, pos: Pos2, time: f64, press: &PressState) -> ToolPointer {
    ToolPointer::new(pos, start.button, time).mods(press.mods).sample(press.sample.unwrap_or(PointerSample::new(1.0))).clicks(start.clicks)
}

fn cancel_reason(reason: PressCancel) -> CancelReason {
    match reason {
        PressCancel::Escape => CancelReason::Escape,
        PressCancel::FocusLost => CancelReason::Blur,
        PressCancel::Disabled => CancelReason::Dismissed,
        _ => CancelReason::Removed,
    }
}

/// One pass of the host: the tool, the state and what the calls produced.
///
/// `'t` is the stand-ins' own lifetime: the host lends them for one run, so `Run`'s borrows need not outlive the host.
struct Run<'a, 't> {
    /// The selected tool.
    selected: &'a mut dyn CanvasTool,
    /// The momentary set: its tool in `momentary` runs in place of the selected one.
    set: Option<&'a mut Momentary>,
    space: Option<CanvasSpace>,
    slop: f32,
    down: &'a [Key],
    caps: bool,
    rect: Rect,
    mods: Mods,
    time: f64,
    ctx: &'a Context,
    env: &'a Env,
    snap: Option<&'a Snap>,
    state: &'a mut HostState,
    response: ToolHostResponse,
    requests: ToolRequests,
    /// A gesture was cancelled in this pass: its Escape is not a key for the tool.
    cancelled: bool,
    /// The gesture whose press, release or cancel is being delivered: the gesture is already out of the state, or not
    /// yet in it.
    ending: Option<GestureId>,
    /// The tool autoscrolls on hover, in its state after `each_pass`.
    hover_scrolls: bool,
    /// The Command stand-in.
    command: Option<&'a mut (dyn CanvasTool + 't)>,
    /// The Alt stand-in.
    alt: Option<&'a mut (dyn CanvasTool + 't)>,
    /// The stand-in lent to hover, cursor and paint while no gesture runs (its key is held and the selected tool does
    /// not take a primary press).
    lent: Option<StandInKey>,
    /// A press may run a stand-in: no momentary tool is in effect.
    stand_ins: bool,
    /// The momentary tool in effect in the pass.
    momentary: Option<MomentaryTool>,
}

impl<'a, 't: 'a> Run<'a, 't> {
    fn context(&self, pointer: Option<ToolPointer>) -> ToolContext<'a> {
        let cx = ToolContext::new(self.rect, self.space)
            .pointer(pointer)
            .mods(self.mods)
            .time(self.time)
            .slop(self.slop)
            .down(self.down)
            .caps_lock(self.caps)
            .gesture(self.state.gesture.map(|active| active.id).or(self.ending));
        let cx = cx.env(self.env);
        match self.snap {
            Some(snap) => cx.snapping(snap),
            None => cx,
        }
    }

    /// Takes what a call asked for. A view it set is the view the calls after it in the pass start from.
    fn absorb(&mut self, mut cx: ToolContext<'_>) {
        let asked = cx.take_requests();
        self.requests.readout = asked.readout.or(self.requests.readout.take());
        self.requests.repaint |= asked.repaint;
        self.requests.hud = asked.hud.or(self.requests.hud.take());
        self.requests.snapped.extend(asked.snapped);
        self.requests.pan = asked.pan.or(self.requests.pan);
        if let Some(at) = asked.autoscroll_at {
            self.scroll_past(at);
        }
        if let Some(view) = asked.view {
            if let Some(space) = self.space.as_mut() {
                space.set_view(view);
            }
            self.response.view = Some(view);
            self.requests.ease = asked.ease;
        }
    }

    /// The tool `who` names: a stand-in, or for `None` and for a stand-in that is not set the momentary tool in effect,
    /// else the selected tool.
    fn pick_as(&mut self, who: Option<StandInKey>) -> &mut dyn CanvasTool {
        let stand = match who {
            Some(StandInKey::Command) => self.command.as_deref_mut(),
            Some(StandInKey::Alt) => self.alt.as_deref_mut(),
            None => None,
        };
        match stand {
            Some(stand) => stand,
            None => run_as(self.set.as_deref_mut(), self.momentary, &mut *self.selected),
        }
    }

    /// A call on the tool of the moment: the running gesture's, else the lent stand-in, else the selected tool.
    fn call<R>(&mut self, pointer: Option<ToolPointer>, run: impl FnOnce(&mut dyn CanvasTool, &mut ToolContext<'_>) -> R) -> R {
        let who = self.state.gesture.map_or(self.lent, |active| active.stand_in);
        self.call_as(who, pointer, run)
    }

    fn call_as<R>(
        &mut self,
        who: Option<StandInKey>,
        pointer: Option<ToolPointer>,
        run: impl FnOnce(&mut dyn CanvasTool, &mut ToolContext<'_>) -> R,
    ) -> R {
        let mut cx = self.context(pointer);
        let out = run(self.pick_as(who), &mut cx);
        self.absorb(cx);
        out
    }

    /// The stand-in whose key `pointer` holds and which takes the press.
    fn stand_for(&self, pointer: &ToolPointer) -> Option<StandInKey> {
        if !self.stand_ins {
            return None;
        }
        [StandInKey::Command, StandInKey::Alt].into_iter().find(|key| {
            let stand = match key {
                StandInKey::Command => self.command.as_deref(),
                StandInKey::Alt => self.alt.as_deref(),
            };
            held_at(*key, pointer.mods) && stand.is_some_and(|tool| tool.takes(pointer.button, pointer.mods))
        })
    }

    /// The one past-edge rule: how far `pos` lies past the canvas edge, for a drag and for a canvas control that asks to
    /// scroll. An empty canvas (a window shrunk to nothing) has no edge to scroll past.
    fn scroll_past(&mut self, pos: Pos2) {
        if self.rect.is_positive() {
            let past = pos - self.rect.clamp(pos);
            if past != Vec2::ZERO {
                self.response.past_edge = Some(past);
            }
        }
    }

    /// Advances the glide of a momentary Hand that is not the tool of this pass.
    fn glide(&mut self) {
        if self.momentary == Some(MomentaryTool::Hand) {
            return;
        }
        let mut cx = self.context(None);
        let Some(hand) = self.set.as_deref_mut().and_then(Momentary::gliding_hand) else { return };
        hand.each_pass(&mut cx);
        self.absorb(cx);
    }

    fn report(&mut self, phase: GesturePhase) {
        let ends = matches!(phase, GesturePhase::Commit | GesturePhase::Cancel);
        if ends || self.response.gesture.is_none() {
            self.response.gesture = Some(phase);
        }
    }

    /// Offers a press to the tool when it takes the button, else to the stand-in whose key is held; a press the selected
    /// tool answers `Ignored` goes to that stand-in too. A refused, ignored or whole press is spent.
    fn begin(&mut self, pointer: ToolPointer) {
        let mut who = None;
        if !self.pick_as(None).takes(pointer.button, pointer.mods) {
            who = self.stand_for(&pointer);
            if who.is_none() {
                self.state.spent = true;
                if pointer.button == PointerButton::Secondary {
                    self.state.secondary = Some(SecondaryPress { mods: pointer.mods, momentary: self.momentary, taken: false });
                }
                return;
            }
        }
        let id = next_gesture_id(self.ctx);
        self.ending = Some(id);
        let mut action = self.call_as(who, Some(pointer), |tool, cx| tool.press(&pointer, cx));
        if action == ToolAction::Ignored
            && who.is_none()
            && let Some(key) = self.stand_for(&pointer)
        {
            who = Some(key);
            action = self.call_as(who, Some(pointer), |tool, cx| tool.press(&pointer, cx));
        }
        self.ending = None;
        match action {
            ToolAction::Claimed => {
                self.state.gesture = Some(Active { id, last: pointer, dragged: false, stand_in: who });
                if pointer.button == PointerButton::Secondary && who.is_none() && self.momentary == Some(MomentaryTool::Hand) {
                    self.state.secondary = Some(SecondaryPress { mods: pointer.mods, momentary: self.momentary, taken: true });
                }
                self.report(GesturePhase::Begin);
            }
            ToolAction::Done => {
                self.state.spent = true;
                self.report(GesturePhase::Commit);
            }
            _ => {
                self.state.spent = true;
                self.response.ignored_press = Some(pointer);
            }
        }
    }

    fn drag(&mut self, pointer: ToolPointer) {
        let Some(active) = self.state.gesture.as_mut() else { return };
        active.last = pointer;
        active.dragged = true;
        self.call(Some(pointer), |tool, cx| tool.drag(&pointer, cx));
        self.report(GesturePhase::Update);
        let who = self.state.gesture.and_then(|active| active.stand_in);
        if self.pick_as(who).autoscrolls() {
            self.scroll_past(pointer.pos);
        }
    }

    /// The drag of this pass past the click distance: one call per pointer move of the pass for a tool that takes
    /// every move ([`Self::every_move`]); one call at `pos` for any other tool.
    fn drag_moves(&mut self, start: PressStart, pos: Pos2, press: &PressState) {
        if self.state.gesture.is_none() {
            return;
        }
        // A pass delivers only the moves it was handed: a rerun pass has none. A held pointer resting past the edge
        // keeps one drag per pass, since the view scrolls under it.
        let stirred = pointer::with_pass_pointer(self.ctx, |pass| pass.stirred);
        if !stirred && pos == self.rect.clamp(pos) {
            return;
        }
        if !self.every_move(start, press) {
            self.drag(press_pointer(start, pos, self.time, press));
        }
    }

    /// For a gesture whose tool takes every move: one drag call per pointer move of the pass after its press, inside
    /// the click distance too, their times spread from the last delivered move to the pass time. False when the tool
    /// does not take every move or the pass has no move to give it.
    fn every_move(&mut self, start: PressStart, press: &PressState) -> bool {
        let Some(active) = self.state.gesture else { return false };
        if !self.pick_as(active.stand_in).takes_every_move() {
            return false;
        }
        let moves: Vec<(Pos2, Option<PointerSample>)> = pointer::with_pass_pointer(self.ctx, |pass| {
            let from = pass.events.iter().rposition(|event| matches!(event, PointerEvent::Pressed { .. })).map_or(0, |at| at + 1);
            pass.events[from..]
                .iter()
                .filter_map(|event| match event {
                    PointerEvent::Moved { at, sample, .. } => Some((*at, *sample)),
                    _ => None,
                })
                .collect()
        });
        let (from, count) = (active.last.time, moves.len());
        for (k, (at, sample)) in moves.into_iter().enumerate() {
            let time = if k + 1 == count { self.time } else { from + (self.time - from) * (k + 1) as f64 / count as f64 };
            self.drag(press_pointer(start, at, time, press).sample(sample.or(press.sample).unwrap_or(PointerSample::new(1.0))));
        }
        count > 0
    }

    /// A hover with the pointer outside the canvas, for a tool that autoscrolls on hover: the tool follows the pointer
    /// and the view scrolls at the distance past the edge. Nothing runs with a button down, a momentary tool or stand-in
    /// in effect, an open modal or popup layer, another canvas's background under the pointer or the window unfocused.
    fn hover_outside(&mut self, canvas: Id, press: &PressState, frame: &PointerFrame) {
        let Some(pos) = frame.pos else { return };
        if !self.hover_scrolls
            || self.state.gesture.is_some()
            || self.lent.is_some()
            || self.momentary.is_some()
            || press.phase.is_press()
            || pointer::shapes_at(self.ctx, pos)
                .iter()
                .any(|(owner, part)| *owner != canvas && part.is_some_and(|part| part.key == super::BACKGROUND))
            || !self.rect.is_positive()
            || self.rect.contains(pos)
            || !frame.buttons.is_empty()
            || !self.ctx.input(|input| input.focused)
            || overlay::any_open(self.ctx, OpenFilter::Keys)
        {
            return;
        }
        let pointer =
            ToolPointer::new(pos, PointerButton::Primary, frame.time).mods(frame.mods).sample(frame.sample.unwrap_or(PointerSample::new(1.0)));
        self.call(Some(pointer), |tool, cx| tool.hover(&pointer, cx));
        self.scroll_past(pos);
    }

    /// The Menu key or Shift+F10 reached the focused canvas: the app opens its menu at the view's centre.
    fn menu_key(&mut self) {
        let Some(space) = self.space else { return };
        if space.background().context_key && self.response.context.is_none() {
            self.response.context = Some(ContextRequest { at: space.view.rect.center(), mods: self.mods, momentary: self.momentary });
        }
    }

    fn end(&mut self, pointer: ToolPointer) {
        if let Some(active) = self.state.gesture.take() {
            self.ending = Some(active.id);
            self.call_as(active.stand_in, Some(pointer), |tool, cx| tool.release(&pointer, cx));
            self.ending = None;
            self.report(GesturePhase::Commit);
        }
    }

    fn cancel(&mut self, reason: CancelReason) {
        if let Some(active) = self.state.gesture.take() {
            self.ending = Some(active.id);
            self.call_as(active.stand_in, Some(active.last), |tool, cx| tool.cancel(reason, cx));
            self.ending = None;
            self.report(GesturePhase::Cancel);
            self.cancelled = true;
        }
    }

    fn hover(&mut self, pointer: ToolPointer) {
        self.call(Some(pointer), |tool, cx| tool.hover(&pointer, cx));
        if let Some(past) = self.hover_edge(pointer.pos) {
            self.response.past_edge = Some(past);
        }
    }

    /// How far into the band inside the view's edge (`size.hit.edge`, at most a quarter of the view's shorter side) a
    /// hover at `pos` is, toward the edge, for a tool that autoscrolls on hover. The view's rect leaves out the
    /// rulers; a canvas without a view measures from its own rect.
    fn hover_edge(&self, pos: Pos2) -> Option<Vec2> {
        let rect = self.space.map_or(self.rect, |space| space.view.rect);
        if !self.hover_scrolls || !rect.is_positive() {
            return None;
        }
        let band = self.env.tokens.size.hit.edge.min(rect.width().min(rect.height()) / 4.0);
        let past = pos - rect.shrink(band).clamp(pos);
        (past != Vec2::ZERO).then_some(past)
    }

    /// The canvas background's press phase of this pass. A pass carries one phase, so a press that moved or
    /// released in the pass it began in starts here.
    fn phase(&mut self, press: &PressState, frame: &PointerFrame) {
        match press.phase {
            PressPhase::Idle => self.state.spent = false,
            PressPhase::Hover => {
                self.state.spent = false;
                // A pointer that stands still keeps the position its last move gave the tool. One that rests in
                // the edge band gets a hover at its own position in every pass, since the view scrolls under it.
                if let Some(pointer) = hover_pointer(frame, *press)
                    && (pointer::with_pass_pointer(self.ctx, |pass| pass.stirred) || self.hover_edge(pointer.pos).is_some())
                {
                    self.hover(pointer);
                }
            }
            PressPhase::Down { start, .. } => {
                self.state.spent = false;
                self.begin(press_pointer(start, start.origin, start.time, press));
                self.every_move(start, press);
            }
            PressPhase::Held { start, .. } => {
                self.every_move(start, press);
            }
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } => {
                if self.state.gesture.is_none() && !self.state.spent {
                    self.begin(press_pointer(start, start.origin, start.time, press));
                }
                self.drag_moves(start, pos, press);
            }
            PressPhase::Release { start, pos, click, .. } => {
                if self.state.gesture.is_none() && !self.state.spent {
                    self.begin(press_pointer(start, start.origin, start.time, press));
                }
                self.every_move(start, press);
                let pointer = press_pointer(start, pos, self.time, press);
                // A press that moved past slop and came up within one pass is still a drag.
                if !click && self.state.gesture.is_some_and(|active| !active.dragged) {
                    self.drag(pointer);
                }
                self.end(pointer);
                self.state.spent = false;
                // A secondary press the tool refused asks for the menu when its button comes up; one the momentary
                // Hand took asks only as a click, so its drag pans.
                if let (Some(asked), Some(at)) = (self.state.secondary.take(), context_click(press))
                    && (!asked.taken || press.phase.is_click())
                {
                    self.response.context = Some(ContextRequest { at, mods: asked.mods, momentary: asked.momentary });
                }
            }
            PressPhase::Cancel { reason, .. } => {
                self.cancel(cancel_reason(reason));
                self.state.spent = false;
                self.state.secondary = None;
            }
            _ => {}
        }
    }

    /// Offers one key to the tool; the intent is taken from the canvas only when the tool takes the key.
    fn offer(&mut self, intents: &mut keys::Intents, read: impl FnOnce(&mut keys::Intents) -> Option<ToolKey>) -> bool {
        let mut next = intents.clone();
        let Some(key) = read(&mut next) else { return false };
        if !self.call(None, |tool, cx| tool.key(&key, cx)) {
            return false;
        }
        *intents = next;
        self.response.took_key = Some(key);
        true
    }

    /// The keys the canvas holds: intents routed to it while it has focus, the arrows, Enter and Escape while nothing
    /// in the window has focus, and the chords the tool claimed at the end of the last pass. A session control in the
    /// canvas body (`KeyUse::session`) has taken the arrows, Enter and Escape it acts on before this runs. The Escape
    /// of the focused canvas reaches the tool once, through the window's intents (`with_window_intents` offers the
    /// focused document canvas's Escape); one the tool leaves stays the canvas's, for the scopes around it.
    fn keys(&mut self, ctx: &Context, canvas: Id, rtl: bool) {
        keys::with_window_intents(ctx, rtl, |window| {
            let mut taken = self.offer(window, |intents| intents.take_nudge().map(|(direction, mods)| ToolKey::Nudge { direction, mods }));
            taken |= self.offer(window, |intents| intents.take(Intent::Submit).then_some(ToolKey::Submit));
            if !self.cancelled {
                taken |= self.offer(window, |intents| intents.take(Intent::Cancel).then_some(ToolKey::Cancel));
            }
            taken
        });
        let mut intents = keys::intents_of(ctx, Some(canvas), rtl);
        let mut taken = self.offer(&mut intents, |intents| intents.take_nudge().map(|(direction, mods)| ToolKey::Nudge { direction, mods }));
        if self.state.claims.nudges {
            // Increment is the arrow toward the reading end, as for any focused control.
            let forward = if rtl { -Vec2::X } else { Vec2::X };
            taken |= self.offer(&mut intents, |intents| {
                intents.take_access_where(is_tool_step).map(|request| {
                    let direction = if request.action == AccessAction::Increment { forward } else { -forward };
                    ToolKey::Nudge { direction, mods: Mods::NONE }
                })
            });
        }
        taken |= self.offer(&mut intents, |intents| intents.take(Intent::Submit).then_some(ToolKey::Submit));
        if taken {
            keys::settle(ctx, Some(canvas), &intents);
        }
        let claims = self.state.claims;
        for chord in delete_chords(claims).filter(|chord| keys::take_claimed(ctx, canvas, *chord)) {
            let delete = ToolKey::delete(chord.key == Key::Delete);
            if self.call(None, |tool, cx| tool.key(&delete, cx)) {
                self.response.took_key = Some(delete);
            }
        }
        if claims.keyboard {
            for chord in kept_chords().filter(|chord| keys::take_claimed(ctx, canvas, *chord)) {
                let kept = ToolKey::Kept(chord);
                if self.call(None, |tool, cx| tool.key(&kept, cx)) {
                    self.response.took_key = Some(kept);
                }
            }
        }
        for chord in claims.keys.iter().copied().filter(|chord| keys::take_claimed(ctx, canvas, *chord)) {
            let kept = ToolKey::Kept(chord);
            if self.call(None, |tool, cx| tool.key(&kept, cx)) {
                self.response.took_key = Some(kept);
            }
        }
    }
}
