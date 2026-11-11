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
//!    (`ToolContext::mods`). While nothing in the window has focus, Enter and Escape are offered too, so a tool
//!    holding a gesture or a pending edit (a transform box) commits and cancels before the canvas is clicked; one the
//!    tool leaves goes on to the scopes around it. A focused field or control keeps its own Enter and Escape.
//! 5. The deletes and the plain keys are the app's unless the tool claims them ([`CanvasTool::claims`]). The host
//!    reads the claims at the end of every pass, after the pointer events and keys, and holds the claimed chords from
//!    the app's shortcuts in the next pass: a plain Delete with nothing to delete runs the app's Edit > Clear.
//! 6. The tool paints over the canvas and requests its cursor at tool priority.
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

use super::momentary::{Momentary, MomentaryCommands};
use super::readout::paint_readout;
use super::tool::{CanvasTool, March, ToolAccess, ToolAction, ToolClaims, ToolContext, ToolKey, ToolPointer, ToolRequests};
use super::tools::HandTool;
use super::zoom_motion::{self, Steer};
use super::{CanvasOutput, CanvasSpace, Snap};
use crate::primitives::{ContentPainterCx, Draw, DrawEnv, PaintRecord};
use crate::runtime::access::{self, Name, Role, SubDecl, SubNode};
use crate::runtime::commands;
use crate::runtime::edit::next_gesture_id;
use crate::runtime::env::{Env, Label};
use crate::runtime::memory::{global, with_session};
use crate::runtime::pointer::{self, PointerFrame};
use crate::runtime::{cursor, frame, keys, lock_keys, measure};
use crate::style::StyleCx;
use craft_behavior::kurbo;
use craft_behavior::{
    CancelReason, CanvasView, Cursor, CursorPriority, GestureId, GesturePhase, Intent, PartKey, PartRef, PointerButton, PointerSample, PressCancel,
    PressPhase, PressStart, PressState,
};
use craft_keymap::{Chord, Key, Mods};
use craft_theme::Variant;
use egui::{Context, Id, Pos2, Rect, Ui, Vec2};
use std::cell::RefCell;

/// A pointer event from automation, queued with [`ToolHost::inject`] and delivered on the next pass as if the
/// pointer did it. Positions are screen points.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToolInput {
    /// A press of `button`. The host starts nothing when the tool takes no such button with `mods`.
    #[non_exhaustive]
    Down {
        /// The button.
        button: PointerButton,
        /// The pointer.
        pos: Pos2,
        /// The pen's reading.
        sample: PointerSample,
        /// The modifiers held.
        mods: Mods,
    },
    /// The pointer moved: a drag while a gesture runs, else a hover.
    #[non_exhaustive]
    Move {
        /// The pointer.
        pos: Pos2,
        /// The pen's reading.
        sample: PointerSample,
        /// The modifiers held.
        mods: Mods,
    },
    /// The button came up.
    #[non_exhaustive]
    Up {
        /// The pointer.
        pos: Pos2,
        /// The modifiers held.
        mods: Mods,
    },
}

impl ToolInput {
    /// A mouse press of `button` at `pos`, with no modifiers.
    pub const fn down(button: PointerButton, pos: Pos2) -> Self {
        ToolInput::Down { button, pos, sample: PointerSample::new(1.0), mods: Mods::NONE }
    }

    /// A mouse move to `pos`, with no modifiers.
    pub const fn moved(pos: Pos2) -> Self {
        ToolInput::Move { pos, sample: PointerSample::new(1.0), mods: Mods::NONE }
    }

    /// The button coming up at `pos`, with no modifiers.
    pub const fn up(pos: Pos2) -> Self {
        ToolInput::Up { pos, mods: Mods::NONE }
    }

    /// The input with the modifiers `held`.
    #[must_use]
    pub fn mods(mut self, held: Mods) -> Self {
        match &mut self {
            ToolInput::Down { mods, .. } | ToolInput::Move { mods, .. } | ToolInput::Up { mods, .. } => *mods = held,
        }
        self
    }

    /// The input with the pen's reading `pen`; a button coming up has none.
    #[must_use]
    pub fn sample(mut self, pen: PointerSample) -> Self {
        match &mut self {
            ToolInput::Down { sample, .. } | ToolInput::Move { sample, .. } => *sample = pen,
            ToolInput::Up { .. } => {}
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
    /// How far past the canvas edge the pointer is, in screen points, while a gesture of a tool that autoscrolls
    /// runs. The host moves no view: the app scrolls its view toward the pointer.
    pub past_edge: Option<Vec2>,
    /// The cursor the tool asked for, after the host requested it.
    pub cursor: Option<Cursor>,
    /// The values the tool wants beside the pointer. The host has painted them already; this is the data for
    /// automation.
    pub readout: Option<ToolReadout>,
    /// The view a tool set with [`ToolContext::set_view`] this pass (the last one, inside the document with Overscroll
    /// off), else the canvas's own ([`CanvasOutput::view`]): the one view the app copies into its own. A view that
    /// eases is its target; the canvas shows the way there.
    pub view: Option<CanvasView>,
}

#[derive(Clone, Copy, Debug)]
struct Active {
    /// The gesture's id, from `runtime::edit::next_gesture_id` when the tool claimed the press.
    id: GestureId,
    button: PointerButton,
    last: ToolPointer,
    /// Started by [`ToolHost::inject`]: the pointer neither moves nor ends it.
    injected: bool,
    /// The tool has had a `drag` call in this gesture.
    dragged: bool,
}

/// What the host keeps between passes, in a session slot keyed by the canvas's id.
#[derive(Clone, Default)]
struct HostState {
    gesture: Option<Active>,
    /// The open press was refused, ignored or finished whole: its later phases are not the tool's.
    spent: bool,
    queued: Vec<ToolInput>,
    /// The keys the tool claimed at the end of the last pass.
    claims: ToolClaims,
    /// The canvas of the last pass, for [`ToolHost::cancel`] outside a pass.
    rect: Option<Rect>,
    space: Option<CanvasSpace>,
    /// The view a tool set while [`ToolHost::cancel`] ended its gesture outside a pass: the next pass reports it.
    cancelled_view: Option<CanvasView>,
}

fn state_id(canvas: Id) -> Id {
    global("craft-ui tool host", canvas)
}

/// The on-canvas tool host. Built each pass, shown once with the canvas's output and the tool.
#[derive(Default)]
pub struct ToolHost<'m> {
    disabled: bool,
    momentary: Option<&'m mut Momentary>,
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

    /// Delivers the pass's pointer and key events to `tool`, paints it over the canvas, and paints the readout it
    /// asked for beside the pointer.
    pub fn run(self, ui: &Ui, canvas: &CanvasOutput, tool: &mut dyn CanvasTool) -> ToolHostResponse {
        let ToolHost { disabled, momentary } = self;
        let ctx = ui.ctx().clone();
        let env = Env::of(ui);
        let key = state_id(canvas.response.id);
        // A measuring run delivers no input: the tool is drawn and sized from the state the last real run left, and
        // the queued events, momentary commands and cancelled view wait for the real run.
        let measuring = measure::active(&ctx);
        let mut state = with_session(&ctx, key, |slot: &mut HostState| {
            let seen = slot.clone();
            if !measuring {
                slot.queued.clear();
            }
            seen
        });
        let queued = std::mem::take(&mut state.queued);
        let cancelled_view = if measuring { None } else { state.cancelled_view.take() };
        let space = canvas.space;
        let down = keys::down(&ctx);
        let caps = lock_keys::caps_lock(&ctx);
        let rect = canvas.response.rect;
        let input = pointer::frame(&ctx);
        let mut glide = None;
        let fired = std::iter::from_fn(|| {
            if measuring {
                return None;
            }
            commands::take_around::<MomentaryCommands>(&ctx, canvas.response.id, env.view().command_chain())
        });
        let tool: &mut dyn CanvasTool = match momentary {
            Some(momentary) => {
                for (command, event) in fired {
                    momentary.fired(command, event.phase);
                }
                let (tool, hand) = momentary.substitute(tool, input.mods, state.gesture.is_some());
                glide = hand;
                tool
            }
            None => {
                fired.for_each(drop);
                tool
            }
        };
        let mut run = Run {
            tool: &mut *tool,
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
        };
        if !measuring {
            run.call(None, |tool, cx| tool.each_pass(cx));
            if let Some(hand) = glide {
                run.glide(hand);
            }
            if disabled {
                run.cancel(CancelReason::Dismissed);
                run.state.spent |= canvas.press.phase.is_press();
            } else {
                run.phase(&canvas.press, &input);
                for event in queued {
                    run.injected(event);
                }
                run.keys(&ctx, canvas.response.id, env.direction.is_rtl());
            }
        }
        let Run { mut response, requests, .. } = run;
        if response.view.is_none() {
            response.view = cancelled_view;
        }
        // A tool's view goes through the canvas's clamp, and its ease through the canvas's one zoom ease; a view that
        // jumps stops that ease.
        if let Some(view) = response.view.as_mut() {
            *view = canvas.clamped(*view);
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
        paint_tool(ui, &env, tool, context());
        let cx = context();
        if let Some(access) = tool.access(&cx) {
            report_access(&ctx, canvas, rect, &access);
        }
        if over {
            let wanted = tool.cursor(&cx);
            cursor::request(&ctx, wanted.clone(), CursorPriority::Tool);
            response.cursor = Some(wanted);
        }
        if let (Some(rows), Some(at)) = (requests.readout, current) {
            paint_readout(&ctx, at.pos, &rows);
            response.readout = Some(ToolReadout { at: at.pos, rows });
        }
        if requests.repaint {
            frame::request_repaint(&ctx);
        }
        // A disabled host delivers no key, so it holds none from the app.
        state.claims = if disabled { ToolClaims::NONE } else { tool.claims() };
        for chord in claimed_chords(state.claims) {
            keys::claim(&ctx, canvas.response.id, chord);
        }
        if !state.claims.keyboard && state.gesture.is_some() {
            keys::claim(&ctx, canvas.response.id, Chord::new(Mods::NONE, Key::Space));
        }
        state.rect = Some(rect);
        state.space = space;
        with_session(&ctx, key, |slot: &mut HostState| {
            let later = std::mem::take(&mut slot.queued);
            *slot = state;
            slot.queued.extend(later);
        });
        response
    }

    /// Queues an automation pointer event for the host of the canvas `canvas` (`CanvasOutput::response.id`); it is
    /// delivered on the next pass, after the pointer's own events. A `Down` while a gesture runs is dropped.
    pub fn inject(ctx: &Context, canvas: Id, input: ToolInput) {
        with_session(ctx, state_id(canvas), |state: &mut HostState| state.queued.push(input));
        frame::request_repaint(ctx);
    }

    /// Ends the gesture in progress on the canvas `canvas` now, without a commit (the app switched tools or
    /// documents, a modal opened). `tool` is the tool that holds the gesture, so call it before the app replaces
    /// it; it runs against the canvas of the last pass. Returns whether a gesture was cancelled.
    pub fn cancel(ctx: &Context, canvas: Id, tool: &mut dyn CanvasTool, reason: CancelReason) -> bool {
        // The press is still held: its later moves and release are not the next gesture's.
        let taken = with_session(ctx, state_id(canvas), |slot: &mut HostState| {
            let taken = slot.gesture.take().map(|active| (active, slot.rect, slot.space));
            slot.spent |= taken.is_some();
            taken
        });
        let Some((active, rect, space)) = taken else { return false };
        let down = keys::down(ctx);
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
}

/// The part of the node that describes the tool's drawing, below the canvas's node.
const TOOL_NODE: PartKey = PartKey::new("tool");
/// The parts of the nodes that describe the tool's anchors, repeated by the anchor's place in reading order, below
/// the tool's node.
const TOOL_ANCHOR: PartKey = PartKey::new("tool anchor");

/// Emits what the tool is to assistive technology through the sub-node path of the canvas's component: a hover-only
/// graphics object over the drawing's bounds (no size at the canvas centre without bounds), and a child object of no
/// size at each anchor, clamped to the canvas. The ids derive from the canvas id, so they stay the same from pass to
/// pass.
fn report_access(ctx: &Context, canvas: &CanvasOutput, rect: Rect, tool: &ToolAccess) {
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
    let style = StyleCx::new(
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
    );
    let march = March { policy: env.motion, dash: env.tokens.stroke.dash };
    tool.paint(&draw, &cx.painter(&content, march).styled(style));
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
    kept_chords().filter(move |_| claims.keyboard).chain(delete_chords(claims))
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
struct Run<'a> {
    tool: &'a mut dyn CanvasTool,
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
}

impl<'a> Run<'a> {
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
        if let Some(view) = asked.view {
            if let Some(space) = self.space.as_mut() {
                space.view = view;
            }
            self.response.view = Some(view);
            self.requests.ease = asked.ease;
        }
    }

    fn call<R>(&mut self, pointer: Option<ToolPointer>, run: impl FnOnce(&mut dyn CanvasTool, &mut ToolContext<'_>) -> R) -> R {
        let mut cx = self.context(pointer);
        let out = run(&mut *self.tool, &mut cx);
        self.absorb(cx);
        out
    }

    /// Advances the glide of a momentary Hand that is not the tool of this pass.
    fn glide(&mut self, hand: &mut HandTool) {
        let mut cx = self.context(None);
        hand.each_pass(&mut cx);
        self.absorb(cx);
    }

    fn report(&mut self, phase: GesturePhase) {
        let ends = matches!(phase, GesturePhase::Commit | GesturePhase::Cancel);
        if ends || self.response.gesture.is_none() {
            self.response.gesture = Some(phase);
        }
    }

    /// Offers a press to the tool when it takes the button; a refused, ignored or whole press is spent.
    fn begin(&mut self, pointer: ToolPointer, injected: bool) {
        if !self.tool.takes(pointer.button, pointer.mods) {
            self.state.spent = true;
            return;
        }
        let id = next_gesture_id(self.ctx);
        self.ending = Some(id);
        let action = self.call(Some(pointer), |tool, cx| tool.press(&pointer, cx));
        self.ending = None;
        match action {
            ToolAction::Claimed => {
                self.state.gesture = Some(Active { id, button: pointer.button, last: pointer, injected, dragged: false });
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
        // An empty canvas (a window shrunk to nothing) has no edge to scroll past.
        if self.tool.autoscrolls() && self.rect.is_positive() {
            let past = pointer.pos - self.rect.clamp(pointer.pos);
            if past != Vec2::ZERO {
                self.response.past_edge = Some(past);
            }
        }
    }

    fn end(&mut self, pointer: ToolPointer) {
        if let Some(active) = self.state.gesture.take() {
            self.ending = Some(active.id);
            self.call(Some(pointer), |tool, cx| tool.release(&pointer, cx));
            self.ending = None;
            self.report(GesturePhase::Commit);
        }
    }

    fn cancel(&mut self, reason: CancelReason) {
        if let Some(active) = self.state.gesture.take() {
            self.ending = Some(active.id);
            self.call(Some(active.last), |tool, cx| tool.cancel(reason, cx));
            self.ending = None;
            self.report(GesturePhase::Cancel);
            self.cancelled = true;
        }
    }

    fn hover(&mut self, pointer: ToolPointer) {
        self.call(Some(pointer), |tool, cx| tool.hover(&pointer, cx));
    }

    /// The canvas background's press phase of this pass. A pass carries one phase, so a press that moved or
    /// released in the pass it began in starts here.
    fn phase(&mut self, press: &PressState, frame: &PointerFrame) {
        if self.state.gesture.is_some_and(|active| active.injected) {
            return;
        }
        match press.phase {
            PressPhase::Idle => self.state.spent = false,
            PressPhase::Hover => {
                self.state.spent = false;
                // A pointer that stands still keeps the position an injected move gave the tool.
                if pointer::with_pass_pointer(self.ctx, |pass| pass.stirred)
                    && let Some(pointer) = hover_pointer(frame, *press)
                {
                    self.hover(pointer);
                }
            }
            PressPhase::Down { start, .. } => {
                self.state.spent = false;
                self.begin(press_pointer(start, start.origin, start.time, press), false);
            }
            PressPhase::DragStart { start, pos, .. } | PressPhase::Drag { start, pos, .. } => {
                if self.state.gesture.is_none() && !self.state.spent {
                    self.begin(press_pointer(start, start.origin, start.time, press), false);
                }
                self.drag(press_pointer(start, pos, self.time, press));
            }
            PressPhase::Release { start, pos, click, .. } => {
                if self.state.gesture.is_none() && !self.state.spent {
                    self.begin(press_pointer(start, start.origin, start.time, press), false);
                }
                let pointer = press_pointer(start, pos, self.time, press);
                // A press that moved past slop and came up within one pass is still a drag.
                if !click && self.state.gesture.is_some_and(|active| !active.dragged) {
                    self.drag(pointer);
                }
                self.end(pointer);
                self.state.spent = false;
            }
            PressPhase::Cancel { reason, .. } => {
                self.cancel(cancel_reason(reason));
                self.state.spent = false;
            }
            _ => {}
        }
    }

    fn injected(&mut self, input: ToolInput) {
        let own = self.state.gesture.is_none_or(|active| active.injected);
        match input {
            ToolInput::Down { button, pos, sample, mods } if self.state.gesture.is_none() => {
                let pointer = ToolPointer::new(pos, button, self.time).mods(mods).sample(sample).clicks(1);
                self.begin(pointer, true);
                self.state.spent = false;
            }
            ToolInput::Move { pos, sample, mods } if own => {
                let button = self.state.gesture.map_or(PointerButton::Primary, |active| active.button);
                let pointer = ToolPointer::new(pos, button, self.time).mods(mods).sample(sample);
                if self.state.gesture.is_some() {
                    self.drag(pointer);
                } else {
                    self.hover(pointer);
                }
            }
            ToolInput::Up { pos, mods } if own => {
                if let Some(active) = self.state.gesture {
                    self.end(ToolPointer::new(pos, active.button, self.time).mods(mods));
                }
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

    /// The keys the canvas holds: intents routed to it while it has focus, Enter and Escape while nothing in the
    /// window has focus, and the chords the tool claimed at the end of the last pass.
    fn keys(&mut self, ctx: &Context, canvas: Id, rtl: bool) {
        keys::with_window_intents(ctx, rtl, |window| {
            let mut taken = self.offer(window, |intents| intents.take(Intent::Submit).then_some(ToolKey::Submit));
            if !self.cancelled {
                taken |= self.offer(window, |intents| intents.take(Intent::Cancel).then_some(ToolKey::Cancel));
            }
            taken
        });
        let mut intents = keys::intents_of(ctx, Some(canvas), rtl);
        let mut taken = self.offer(&mut intents, |intents| intents.take_nudge().map(|(direction, mods)| ToolKey::Nudge { direction, mods }));
        taken |= self.offer(&mut intents, |intents| intents.take(Intent::Submit).then_some(ToolKey::Submit));
        if !self.cancelled {
            taken |= self.offer(&mut intents, |intents| intents.take(Intent::Cancel).then_some(ToolKey::Cancel));
        }
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
    }
}
