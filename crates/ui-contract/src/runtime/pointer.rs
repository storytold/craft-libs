//! The pointer snapshot, press targets, the shape record and press cancels (8.1, 8.3).
//!
//! The begin hook converts the pass's pointer input once. The sensing core in `primitives` runs the press
//! lifecycle over it and writes the shapes it registers into the record the focus service reads.

use crate::runtime::drag;
use crate::runtime::egui::areas::{self, Layer};
use crate::runtime::egui::{input, pass};
use crate::runtime::frame::{PerViewport, Stamped};
use crate::runtime::inputs::{self, Input};
use crate::runtime::sampler;
use craft_behavior::press::{MULTI_TAP_SLOP, MULTI_TAP_TIME};
use craft_behavior::{
    Buttons, MultiTap, MultiTouch, PartRef, PointerButton, PointerEvent, PointerKind, PointerSample, PressCancel, PressStart, ResolveFor, Shape,
    Winner, count_clicks, resolve,
};
use craft_keymap::Mods;
use craft_theme::HitSizes;
use egui::{Context, Id, IdMap, IdSet, Pos2, Rect, TouchId, TouchPhase, Vec2, ViewportId};
use smallvec::SmallVec;
use std::collections::HashMap;
use std::sync::Arc;

/// Points one wheel line adds to `PointerFrame.scroll`; scroll owners apply their own notch (the wheel service).
pub(crate) const WHEEL_LINE_POINTS: f32 = 40.0;

/// Points one wheel page adds to `PointerFrame.scroll`.
pub(crate) const WHEEL_PAGE_POINTS: f32 = 400.0;

/// The pointer input of one pass of one viewport, egui-converted. Input enters a frame's first pass only (egui hands
/// the frame's events to it, and injected steps and held-back presses follow the frame): a rerun pass of the
/// frame carries no events, so its `delta`, `press`, `release` and `scroll` are empty while the held facts (`pos`,
/// `buttons`) persist. A consumer applies what its pass hands it and never turns a held fact into an event.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct PointerFrame {
    /// The latest pointer position, `None` while the pointer is not in the window.
    pub pos: Option<Pos2>,
    /// The pointer movement of the pass.
    pub delta: Vec2,
    /// The buttons held.
    pub buttons: Buttons,
    /// The primary press of the pass.
    pub press: Option<PressStart>,
    /// Modifiers carried by `press`, including the app's latched modifiers. A pass has at most one press.
    pub press_mods: Option<Mods>,
    /// The button released in the pass.
    pub release: Option<PointerButton>,
    /// The modifiers held, with the ones the app latches for the user (a Modifier Keys panel) added: the app holds
    /// them down for every press and drag. Key chords read the keyboard's own modifiers instead.
    pub mods: Mods,
    /// The modifiers of the pass's last button release, with the latched ones added; the held `mods` in a pass with no
    /// release and in a rerun pass. The one source of a pick's and a drop's modifiers.
    pub pick_mods: Mods,
    /// Where the pass's last button release went up; the latest `pos` in a pass with no release and in a rerun pass. The
    /// one source of a drop's position: a pointer that moves on or leaves the window after the release in the same
    /// frame does not move the drop.
    pub pick_pos: Option<Pos2>,
    /// The tap a per-viewport recognizer reports when the last finger lifts, in the frame's first pass only.
    pub multi_tap: Option<craft_behavior::MultiTap>,
    /// The pass's time in seconds.
    pub time: f64,
    /// The device.
    pub kind: PointerKind,
    /// Pressure and tilt of the current contact.
    pub sample: Option<PointerSample>,
    /// The two-finger gesture.
    pub multi_touch: Option<MultiTouch>,
    /// Wheel and trackpad scroll, in points.
    pub scroll: Vec2,
    /// Pinch and Ctrl or Cmd with the wheel, folded into one factor.
    pub zoom: f32,
    /// The pass's summed `Event::Rotate` radians.
    pub rotation: f32,
    /// The movement that turns a press into a drag.
    pub slop: f32,
    /// The least hit size across for this pointer kind.
    pub target: f32,
}

/// egui's click and drag widgets for the latest press.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[doc(hidden)]
pub struct PressTargets {
    pub click: Option<Id>,
    pub drag: Option<Id>,
}

/// What the begin hook read of the pass, for the sensing core and the services. Built from the pass's own events,
/// so a rerun pass of a frame (see [`PointerFrame`]) reads none.
#[derive(Clone, Debug, Default)]
#[doc(hidden)]
pub struct PassPointer {
    pub frame: Option<PointerFrame>,
    /// The primary events, presses carrying their click counts.
    pub events: Vec<PointerEvent>,
    pub hover: Option<Pos2>,
    pub primary_down: bool,
    pub press_targets: PressTargets,
    pub focus_lost: bool,
    /// Whether the real pointer moved or entered, or a held modifier or key changed, in the pass.
    pub stirred: bool,
    /// A finger landed in the pass while another was down: a press open now is a gesture's.
    pub gesture: bool,
}

#[derive(Default)]
pub(crate) struct Memory {
    /// The latest press of any button, until it moves past slop: where, when, its place in the click run and the
    /// modifiers held with it.
    last_press: Option<(Pos2, f64, u32, Mods)>,
    /// A button is down since `last_press`.
    held: bool,
    press_targets: PressTargets,
    /// A cancel a service requested for the running pass.
    requested: Option<(u64, PressCancel)>,
    /// Whether any arena of the viewport holds an open press; sensing registers it, as the layer above.
    press_probe: Option<fn(&Context, ViewportId) -> bool>,
    /// The pass in which a container last held a press on its own surface (`hold_surface_press`); the end sweep
    /// drops it after a pass with none.
    surface_held: Option<u64>,
    /// The arena roots whose members the next pass resolves without (`withhold_members`), with the pass that asked.
    withheld: IdMap<u64>,
    /// The touch under way, for the multi-finger tap.
    taps: Taps,
}

/// What one pass's finger events made of the touch.
struct Touched {
    /// A finger landed while another was down.
    landed: bool,
    /// The tap whose last finger lifted in the pass.
    tap: Option<MultiTap>,
    /// The centre of the fingers' landing points while a finger is still down.
    tracked: Option<Pos2>,
}

/// The recognizer of the multi-finger tap: the fingers of one touch, where each landed, the most down at once, when
/// the first landed and whether the touch is spoiled (a finger moved past [`MULTI_TAP_SLOP`], stayed past [`MULTI_TAP_TIME`] or
/// was cancelled).
#[derive(Default)]
struct Taps {
    down: Vec<TouchId>,
    landed: Vec<(TouchId, Pos2)>,
    most: usize,
    start: f64,
    spoiled: bool,
}

impl Taps {
    /// Follows the pass's finger events at time `now`.
    fn feed(&mut self, fingers: &[input::Finger], now: f64) -> Touched {
        let mut touched = Touched { landed: false, tap: None, tracked: None };
        for finger in fingers {
            match finger.phase {
                TouchPhase::Start => {
                    if self.down.is_empty() {
                        *self = Taps { start: now, ..Taps::default() };
                    }
                    if !self.down.contains(&finger.id) {
                        touched.landed |= !self.down.is_empty();
                        self.down.push(finger.id);
                        self.landed.push((finger.id, finger.pos));
                    }
                    self.most = self.most.max(self.down.len());
                }
                TouchPhase::Move => {
                    let moved =
                        self.landed.iter().rev().find(|(id, _)| *id == finger.id).is_some_and(|(_, at)| at.distance(finger.pos) > MULTI_TAP_SLOP);
                    self.spoiled |= moved && self.down.contains(&finger.id);
                }
                TouchPhase::End | TouchPhase::Cancel => {
                    let known = self.down.len();
                    self.down.retain(|id| *id != finger.id);
                    if self.down.len() == known {
                        continue;
                    }
                    self.spoiled |= finger.phase == TouchPhase::Cancel || now - self.start > MULTI_TAP_TIME;
                    if self.down.is_empty() && !self.spoiled && self.most >= 2 {
                        let fingers = u8::try_from(self.most).unwrap_or(u8::MAX);
                        touched.tap = Some(MultiTap::new(fingers, self.centre()));
                    }
                }
            }
        }
        touched.tracked = (!self.down.is_empty()).then(|| self.centre());
        touched
    }

    /// The centre of the landing points.
    fn centre(&self) -> Pos2 {
        let sum = self.landed.iter().fold(Vec2::ZERO, |sum, (_, at)| sum + at.to_vec2());
        (sum / self.landed.len().max(1) as f32).to_pos2()
    }
}

/// What one root registered on one layer: its shapes and its egui widgets.
#[derive(Clone, Default)]
struct RootRecord {
    shapes: Vec<Shape<Id>>,
    /// Where each shape of `shapes` stands, at the same index.
    placed: Vec<Placed>,
    /// The root's egui widgets: the click and drag targets that stand for it.
    widgets: Vec<Id>,
    /// The root's widgets sense a click.
    click: bool,
}

/// Where a recorded shape stands: its place in the pass's paint order (the registration that recorded it; later is on
/// top) and the clip it shows through, in its layer's space.
#[derive(Clone, Copy)]
struct Placed {
    order: u32,
    clip: Rect,
}

type LayerRecord = HashMap<Layer, IdMap<RootRecord>>;

/// The transformed body layers of a pass: each layer's root and the layer the root itself is shown on.
type BodyRecord = HashMap<Layer, (Id, Layer)>;

#[derive(Default)]
pub(crate) struct Shapes {
    this: LayerRecord,
    last: LayerRecord,
    this_bodies: BodyRecord,
    last_bodies: BodyRecord,
    /// The egui widgets components made for their own text (a field's text, a selection part) in the running pass and
    /// the last, each with its component: such a widget takes its press itself.
    this_texts: IdMap<Id>,
    last_texts: IdMap<Id>,
    /// Every id components registered or made in the running pass: the roots, their egui widgets and the widgets made
    /// for their text and editor parts (`made_by_component`).
    this_made: IdSet,
    /// The layers' own egui widgets (an overlay `Area`'s, which senses a click where nothing on the layer does) in the
    /// running pass and the last: such a widget lies under every widget and shape on its layer and stands for none.
    this_grounds: IdSet,
    last_grounds: IdSet,
    /// The paint order the running pass's next registration takes.
    next: u32,
    /// What lies under the pass's hover point, found over `last` at the begin hook with the pass's egui targets:
    /// every root's hover reads it, and a press at the same point and targets reuses it.
    hover: Option<(Pos2, PressTargets, Under)>,
    /// A press was open when the pass began: only its target hovers.
    captured: bool,
    /// The pass's tap went to a component.
    tap_taken: bool,
    /// Parts holding a press at the start of this pass, before enclosing arenas process its release.
    pressed_parts: Vec<(Id, PartRef)>,
    /// What lies under another point a press asked for in the pass, with its targets: every root asks for the same
    /// press.
    winner: Option<(Pos2, PressTargets, Under)>,
    /// The resolutions the running pass made (`find_under`), for the count trials.
    #[cfg(feature = "testing")]
    resolutions: u32,
    /// The `made_by_component` lookups the running pass made, for the count trials.
    #[cfg(feature = "testing")]
    made_lookups: u32,
}

/// The begin hook (H5): converts the pass's pointer input.
pub(crate) fn begin(ctx: &Context, hit: HitSizes, latched: craft_keymap::Mods) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let mut read = input::pointer_read(ctx);
    // Every pass reads its own input and the frame's latches, so a second pass of a frame builds the same
    // modifiers from its own (event-free) input and counts no press twice.
    let mods = with_latched(read.mods, latched);
    let press_mods = read.press_mods.map(|mods| with_latched(mods, latched));
    // A drag that started in another window reaches this one through the desktop pointer (8.4), never at the place
    // this window saw the pointer last: the source window holds the pointer, and the window may have had no exit.
    if drag::runs_elsewhere(ctx, viewport) {
        read.pos = drag::cross_window_pointer(ctx, viewport);
    }
    let wheel = input::wheel_read(ctx, WHEEL_LINE_POINTS, WHEEL_PAGE_POINTS);
    let hits = input::hit_targets(ctx);
    let other_pressed =
        read.events.iter().any(|event| matches!(event, PointerEvent::Pressed { button: PointerButton::Middle | PointerButton::Secondary, .. }));
    let (events, targets, touched) = PerViewport::<Memory>::with(ctx, viewport, |memory| {
        let mut events = read.events.clone();
        // Every button's press runs on the click run, as a click of any button did in the old apps: a middle-button pan
        // click between two clicks makes the second a triple click. Only a primary press carries its count.
        for event in &mut events {
            match event {
                PointerEvent::Pressed { button, pos, time, clicks, .. } => {
                    let press_mods = press_mods.unwrap_or(mods);
                    let counted = count_clicks(memory.last_press, *pos, *time, press_mods, hit.slop);
                    memory.last_press = Some((*pos, *time, counted, press_mods));
                    memory.held = true;
                    if *button == PointerButton::Primary {
                        *clicks = counted;
                    }
                }
                // A press moved past slop is a drag, not a click: the next press starts a new run, as only clicks
                // counted toward a double click in the old apps.
                PointerEvent::Moved { at, .. } if memory.held && memory.last_press.is_some_and(|(from, ..)| from.distance(*at) > hit.slop) => {
                    memory.last_press = None;
                }
                PointerEvent::Released { .. } => memory.held = false,
                _ => {}
            }
        }
        if read.primary_pressed || other_pressed {
            memory.press_targets = PressTargets { click: hits.0, drag: hits.1 };
        }
        (events, memory.press_targets, memory.taps.feed(&read.touches, read.time))
    });
    // Where a modal holds the wheel off the canvas, a tap is not reported and a touch under way is dropped, by the
    // wheel's own rule.
    let tap = touched.tap.filter(|tap| sampler::wheel_allowed(ctx, tap.at));
    if touched.tracked.is_some_and(|at| !sampler::wheel_allowed(ctx, at)) {
        PerViewport::<Memory>::with(ctx, viewport, |memory| memory.taps = Taps::default());
    }
    let press = events.iter().rev().find_map(|event| match event {
        PointerEvent::Pressed { button, pos, time, clicks, .. } => Some(PressStart::new(*pos, *button, *time, *clicks)),
        _ => None,
    });
    let press = press.or_else(|| read.other_press.map(|(button, at)| PressStart::new(at, button, read.time, 1)));
    let (scroll, wheel_zoom) = (wheel.map_or(Vec2::ZERO, |(scroll, ..)| scroll), read.zoom);
    let frame = PointerFrame {
        pos: read.pos,
        delta: read.delta,
        buttons: read.buttons,
        press,
        press_mods,
        release: read.release,
        mods,
        pick_mods: read.released_at.map_or(mods, |(_, released)| with_latched(released, latched)),
        pick_pos: read.released_at.map_or(read.pos, |(at, _)| Some(at)),
        multi_tap: tap,
        time: read.time,
        kind: read.kind,
        sample: read.sample,
        multi_touch: read.multi_touch.map(|(translation, zoom, rotation, center)| MultiTouch::new(translation, zoom, rotation, center)),
        scroll,
        zoom: wheel_zoom,
        rotation: read.rotation,
        slop: hit.slop,
        target: hit.target,
    };
    let pass_pointer = PassPointer {
        frame: Some(frame),
        events,
        hover: read.hover,
        primary_down: read.primary_down,
        press_targets: targets,
        focus_lost: read.focus_lost,
        stirred: read.stirred,
        gesture: touched.landed,
    };
    let hover_at = pass_pointer.hover;
    // The pointer and the modifiers are tracked inputs: their generations move when the held facts change or the pass
    // brings pointer events.
    if let Some(frame) = &pass_pointer.frame {
        inputs::observe(ctx, Input::Modifiers, frame.mods);
        inputs::observe(ctx, Input::Pointer, (frame.pos, frame.buttons, pass_pointer.hover, pass_pointer.primary_down));
        let events = frame.delta != Vec2::ZERO
            || frame.press.is_some()
            || frame.release.is_some()
            || frame.scroll != Vec2::ZERO
            || frame.zoom != 1.0
            || frame.rotation != 0.0
            || frame.multi_touch.is_some()
            || frame.multi_tap.is_some()
            || pass_pointer.gesture
            || pass_pointer.stirred
            || pass_pointer.focus_lost
            || !pass_pointer.events.is_empty();
        if events {
            inputs::bump(ctx, Input::Pointer);
        }
        // A press or release, or an outside press the layer stack's input hook took before egui saw it.
        if frame.press.is_some() || frame.release.is_some() || crate::runtime::overlay::absorbed_press_raw(ctx).is_some() {
            inputs::bump(ctx, Input::Press);
        }
    }
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass_nr, |_, _| {}, |record| *record = Arc::new(pass_pointer));
    PerViewport::<Shapes>::with(ctx, viewport, |shapes| {
        shapes.last = std::mem::take(&mut shapes.this);
        shapes.last_bodies = std::mem::take(&mut shapes.this_bodies);
        shapes.last_texts = std::mem::take(&mut shapes.this_texts);
        shapes.this_made.clear();
        shapes.last_grounds = std::mem::take(&mut shapes.this_grounds);
        shapes.next = 0;
        shapes.hover = None;
        shapes.winner = None;
        shapes.tap_taken = false;
        #[cfg(feature = "testing")]
        {
            shapes.resolutions = 0;
            shapes.made_lookups = 0;
        }
    });
    // The pointer's hover is resolved once for the pass, over last pass's complete record, with the targets egui's hit
    // test gives at the pointer now.
    let hover_targets = PressTargets { click: hits.0, drag: hits.1 };
    let hover = hover_at.map(|at| (at, hover_targets, find_under(ctx, viewport, at, hover_targets, false)));
    let captured = press_captured(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |shapes| {
        shapes.hover = hover;
        shapes.captured = captured;
        shapes.pressed_parts.clear();
    });
}

/// The parts holding a press at the start of the running pass: the press arenas set them from the begin hook
/// (`frame::register_begin_hook`), before an enclosing arena processes a release.
pub(crate) fn set_pressed_parts(ctx: &Context, parts: Vec<(Id, PartRef)>) {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |shapes| shapes.pressed_parts = parts);
}

/// The modifiers `held` with the app's `latched` ones added, field by field: `Mods` has no operator for it.
fn with_latched(held: Mods, latched: Mods) -> Mods {
    Mods { cmd: held.cmd || latched.cmd, ctrl: held.ctrl || latched.ctrl, alt: held.alt || latched.alt, shift: held.shift || latched.shift }
}

/// The running viewport's pass number.
pub(crate) fn pass_of(ctx: &Context) -> u64 {
    pass::pass_nr(ctx, pass::current_viewport(ctx))
}

/// The primary press of this pass when it went down in `rect` on `layer` (`areas::lies_in`), whatever takes it. Read
/// as the press step reads it, without noting a pointer input: the press engages the sensing of what it lands on.
pub(crate) fn primary_press_in(ctx: &Context, layer: Layer, rect: egui::Rect) -> Option<PressStart> {
    with_pass_raw(ctx, |pass| pass.frame.as_ref().and_then(|frame| frame.press))
        .filter(|press| press.button == PointerButton::Primary && areas::lies_in(ctx, layer, rect, press.origin))
}

/// What the begin hook read of the running pass.
pub(crate) fn pass_pointer(ctx: &Context) -> Arc<PassPointer> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass::pass_nr(ctx, viewport), |_, _| {}, |record| Arc::clone(record))
}

/// Reads what the begin hook read of the running pass in place, without cloning its events. A tracked input
/// (`runtime::inputs`): a memoized view that reads it runs when the pointer changes.
#[doc(hidden)]
pub fn with_pass_pointer<R>(ctx: &Context, f: impl FnOnce(&PassPointer) -> R) -> R {
    crate::runtime::replay::note_input(ctx, Input::Pointer);
    with_pass_raw(ctx, f)
}

/// `with_pass_pointer` without noting the read: for the hit sensing, press and drag steps of the component under the
/// pointer, which the pointer engages, and for the pass machinery.
pub(crate) fn with_pass_raw<R>(ctx: &Context, f: impl FnOnce(&PassPointer) -> R) -> R {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(ctx, viewport, pass_nr, |_, _| {}, |record| f(record))
}

/// The buttons held in the latest pass of `viewport`, which may be a child window whose pass ran inside this one.
pub(crate) fn buttons_of(ctx: &Context, viewport: ViewportId) -> Buttons {
    PerViewport::<Stamped<Arc<PassPointer>>>::with_pass(
        ctx,
        viewport,
        pass::pass_nr(ctx, viewport),
        |_, _| {},
        |record| record.frame.as_ref().map_or(Buttons::empty(), |frame| frame.buttons),
    )
}

/// Reads the running pass's pointer frame, `None` before the begin hook, and its hover position in place.
pub(crate) fn with_frame<R>(ctx: &Context, f: impl FnOnce(Option<&PointerFrame>, Option<Pos2>) -> R) -> R {
    with_pass_raw(ctx, |pointer| f(pointer.frame.as_ref(), pointer.hover))
}

/// The running pass's pointer input. A tracked input (`runtime::inputs`): a memoized view that reads it runs when the
/// pointer changes. A view that reads only the modifiers reads [`mods`].
pub fn frame(ctx: &Context) -> PointerFrame {
    crate::runtime::replay::note_input(ctx, Input::Pointer);
    frame_raw(ctx)
}

/// The modifiers held in the running pass, with the latched ones (`PointerFrame::mods`). A tracked input: a memoized
/// view that reads them runs when they change, and only then.
pub fn mods(ctx: &Context) -> Mods {
    crate::runtime::replay::note_input(ctx, Input::Modifiers);
    with_pass_raw(ctx, |pointer| pointer.frame.as_ref().map_or(Mods::NONE, |frame| frame.mods))
}

/// Where the primary press of the running pass started. A tracked input (`Input::Press`): a layer's dismissal decides by
/// it, so a memoized view that shows a layer runs on a press anywhere, and a pointer move leaves it alone.
pub(crate) fn primary_press(ctx: &Context) -> Option<Pos2> {
    crate::runtime::replay::note_input(ctx, Input::Press);
    frame_raw(ctx).press.filter(|press| press.button == craft_behavior::PointerButton::Primary).map(|press| press.origin)
}

/// `frame` without noting the read: for the steps of the component under the pointer and the pass machinery.
pub(crate) fn frame_raw(ctx: &Context) -> PointerFrame {
    with_pass_raw(ctx, |pointer| pointer.frame.clone()).unwrap_or_else(|| PointerFrame {
        pos: None,
        delta: Vec2::ZERO,
        buttons: Buttons::empty(),
        press: None,
        press_mods: None,
        release: None,
        mods: Mods::NONE,
        pick_mods: Mods::NONE,
        pick_pos: None,
        multi_tap: None,
        time: 0.0,
        kind: PointerKind::Mouse,
        sample: None,
        multi_touch: None,
        scroll: Vec2::ZERO,
        zoom: 1.0,
        rotation: 0.0,
        slop: 0.0,
        target: 0.0,
    })
}

/// Ends every captured press of the running viewport at its next lifecycle run in this pass (Escape, a part
/// turning disabled). The next begin hook ends egui's own drag so hover reaches other widgets again.
pub(crate) fn cancel_press(ctx: &Context, reason: PressCancel) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| memory.requested = Some((pass_nr, reason)));
}

/// Whether the arena of `root` resolves the presses and hover of the next pass without its members' shapes. A canvas
/// host sets it each pass from the tool it runs: while a momentary tool (the Hand of a held Space, the Zoom of a held
/// Command) is in effect, a press anywhere on the canvas lands on the canvas itself, over the session members on its
/// body (a transform box, a crop frame), and the host gives it to that tool. A press already open keeps its target.
pub fn withhold_members(ctx: &Context, root: Id, withheld: bool) {
    if !crate::runtime::measure::effect(ctx, "pointer::withhold_members") {
        return;
    }
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| {
        if withheld {
            memory.withheld.insert(root, pass_nr);
        } else {
            memory.withheld.remove(&root);
        }
    });
}

/// The arena roots of `viewport` whose presses and hover the running pass resolves without their members: their host
/// asked so in this pass or the one before, the pass whose shapes this one resolves over.
fn withheld_roots(ctx: &Context, viewport: ViewportId) -> SmallVec<[Id; 2]> {
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| {
        memory.withheld.iter().filter(|(_, asked)| **asked + 1 >= pass_nr).map(|(root, _)| *root).collect()
    })
}

/// Registers how the viewport tells that a press is captured. Sensing owns the arenas' machines.
pub(crate) fn register_press_probe(ctx: &Context, probe: fn(&Context, ViewportId) -> bool) {
    PerViewport::<Memory>::with(ctx, pass::current_viewport(ctx), |memory| memory.press_probe = Some(probe));
}

/// Whether a press of the running viewport is open, in an arena or on a container's own surface: Escape cancels it
/// before anything else.
pub(crate) fn press_captured(ctx: &Context) -> bool {
    press_captured_in(ctx, pass::current_viewport(ctx))
}

/// Whether a press of `viewport` is open, as [`press_captured`] tells for the running one; the key stage reads it
/// before the viewport's pass runs.
pub(crate) fn press_captured_in(ctx: &Context, viewport: ViewportId) -> bool {
    let (probe, surface) = PerViewport::<Memory>::with(ctx, viewport, |memory| (memory.press_probe, memory.surface_held));
    surface.is_some() || probe.is_some_and(|probe| probe(ctx, viewport))
}

/// Records that a container holds a press on its own surface in the running pass, so the press counts as captured
/// until the end of the next pass that holds none.
pub(crate) fn hold_surface_press(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| memory.surface_held = Some(pass_nr));
    crate::runtime::frame::register_end_sweep(ctx, std::any::TypeId::of::<Memory>(), sweep_surface_press);
}

/// Drops the surface press of a pass in which no container held one: it ended, or its container was not shown.
fn sweep_surface_press(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    PerViewport::<Memory>::with(ctx, viewport, |memory| memory.surface_held = memory.surface_held.filter(|held| *held == pass_nr));
}

/// Why captured presses end in the running pass: a cancel a service requested, else the window losing focus, else a
/// second finger landing.
pub(crate) fn cancel_of_pass(ctx: &Context) -> Option<PressCancel> {
    let viewport = pass::current_viewport(ctx);
    let pass_nr = pass::pass_nr(ctx, viewport);
    let requested = PerViewport::<Memory>::with(ctx, viewport, |memory| memory.requested.filter(|(nr, _)| *nr == pass_nr).map(|(_, reason)| reason));
    requested
        .or_else(|| with_pass_raw(ctx, |pointer| pointer.focus_lost).then_some(PressCancel::FocusLost))
        .or_else(|| with_pass_raw(ctx, |pointer| pointer.gesture).then_some(PressCancel::Gesture))
}

/// The running pass's tap, once, for the component `id` when `accepts` takes its centre (the component's rect check, in
/// its own space) and `id` or one of its members wins a press there.
pub(crate) fn take_multi_tap(ctx: &Context, id: Id, accepts: impl FnOnce(Pos2) -> bool) -> Option<MultiTap> {
    let tap = with_frame(ctx, |frame, _| frame.and_then(|frame| frame.multi_tap))?;
    let viewport = pass::current_viewport(ctx);
    if PerViewport::<Shapes>::with(ctx, viewport, |shapes| shapes.tap_taken) || !accepts(tap.at) {
        return None;
    }
    let targets = with_pass_raw(ctx, |pointer| pointer.press_targets);
    press_winner(ctx, tap.at, targets).filter(|(winner, root)| *root == id || winner.owner == id)?;
    PerViewport::<Shapes>::with(ctx, viewport, |shapes| shapes.tap_taken = true);
    Some(tap)
}

/// Writes `shapes` of the arena rooted at `root`, shown through `clip` (layer space), into the running pass's shape
/// record under `layer`, above every shape recorded before them in the pass. Stack positions are per arena, so each
/// root's shapes stay apart.
pub(crate) fn record_shapes(ctx: &Context, layer: Layer, root: Id, clip: Rect, shapes: impl IntoIterator<Item = Shape<Id>>) {
    record_shapes_in(ctx, pass::current_viewport(ctx), layer, root, clip, shapes);
}

/// `record_shapes` for a caller that already read the running viewport.
pub(crate) fn record_shapes_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, clip: Rect, shapes: impl IntoIterator<Item = Shape<Id>>) {
    let shapes: Vec<Shape<Id>> = shapes.into_iter().collect();
    crate::runtime::replay::journal(
        ctx,
        || (layer, root, crate::runtime::Clip::new(clip), shapes.clone()),
        |ctx, (layer, root, clip, shapes)| record_shapes(ctx, *layer, *root, clip.rect(), shapes.iter().cloned()),
    );
    add_shapes_in(ctx, viewport, layer, root, clip, shapes);
}

/// `record_shapes_in` with no note in the run records, for a caller whose own note makes the shapes again: shapes at
/// stack positions a replay takes from the arena it joins (`sensing`), not from the recorded run.
pub(crate) fn add_shapes_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, clip: Rect, shapes: Vec<Shape<Id>>) {
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let order = record.next;
        record.next += 1;
        record.this_made.insert(root);
        let entry = record.this.entry(layer).or_default().entry(root).or_default();
        entry.shapes.extend(shapes);
        entry.placed.resize(entry.shapes.len(), Placed { order, clip });
    });
}

/// Records the egui widget `widget` that the component `owner` made for its own text (a field's text, a selection
/// part, an editor part's focus widget): a press egui gives it is the widget's own and goes to no root, and the hover
/// goes to `owner`.
pub(crate) fn record_text_widget(ctx: &Context, widget: Id, owner: Id) {
    crate::runtime::replay::journal(ctx, || (widget, owner), |ctx, &(widget, owner)| record_text_widget(ctx, widget, owner));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record.this_texts.insert(widget, owner);
        record.this_made.insert(widget);
    });
}

/// Records the egui widget `widget` that a layer shows beneath everything on it (an overlay `Area`'s own): egui gives
/// it the press where no widget on the layer is, and the press and hover there go to the layer's shapes as if egui
/// had named no target.
pub(crate) fn record_layer_widget(ctx: &Context, widget: Id) {
    crate::runtime::replay::journal(ctx, || widget, |ctx, &widget| record_layer_widget(ctx, widget));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record.this_grounds.insert(widget);
    });
}

/// Records that `layer` shows the transformed body of the root `root`, which is itself shown on `parent`: a press on
/// the body layer reaches the root's own shapes beneath the body's.
pub(crate) fn record_body_layer(ctx: &Context, layer: Layer, parent: Layer, root: Id) {
    crate::runtime::replay::journal(ctx, || (layer, parent, root), |ctx, &(layer, parent, root)| record_body_layer(ctx, layer, parent, root));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record.this_bodies.insert(layer, (root, parent));
    });
}

/// Records the egui widgets that stand for the root at `root` (`click` when they sense a click): the root's own at
/// step 8, and the `over` widgets of its members after their bodies.
pub(crate) fn record_widgets(ctx: &Context, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    record_widgets_in(ctx, pass::current_viewport(ctx), layer, root, widgets, click);
}

/// `record_widgets` for a caller that already read the running viewport.
pub(crate) fn record_widgets_in(ctx: &Context, viewport: ViewportId, layer: Layer, root: Id, widgets: &[Id], click: bool) {
    crate::runtime::replay::journal(
        ctx,
        || (layer, root, widgets.to_vec(), click),
        |ctx, (layer, root, widgets, click)| record_widgets(ctx, *layer, *root, widgets, *click),
    );
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record.this_made.insert(root);
        record.this_made.extend(widgets.iter().copied());
        let entry = record.this.entry(layer).or_default().entry(root).or_default();
        entry.widgets.extend_from_slice(widgets);
        entry.click |= click;
    });
}

/// What the root `root` registered in the running pass of `viewport`, read for its part `part`: the egui widgets
/// that stand for the root, and the screen bounds of the root's shapes that take a press and outrank the part.
pub(crate) struct OverPart {
    /// The root's egui widgets: a press on them resolves among the root's shapes, not as a widget over the part.
    pub widgets: Vec<Id>,
    /// The shapes a press on the part goes to instead: a higher stack position (a member, an `over` part), level or
    /// sub-level (8.3). A shape of the same rank wins only by its distance to the point, so it is not one of them.
    pub above: Vec<Rect>,
}

/// What lies over the part `part` of the root `root` in the running pass of `viewport`, among the root's widgets and
/// shapes. Read at the end of the pass, when the record is complete.
pub(crate) fn over_part(ctx: &Context, viewport: ViewportId, root: Id, part: PartRef) -> OverPart {
    let (widgets, layers) = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let entries = || record.this.iter().filter_map(|(layer, roots)| roots.get(&root).map(|entry| (*layer, entry)));
        let rank = |shape: &Shape<Id>| (shape.position, shape.priority.level, shape.priority.sub);
        let own =
            entries().flat_map(|(_, entry)| entry.shapes.iter()).filter(|shape| shape.owner == root && shape.part == Some(part)).map(rank).max();
        let widgets: Vec<Id> = entries().flat_map(|(_, entry)| entry.widgets.iter().copied()).collect();
        let layers: Vec<(Layer, Vec<Rect>)> = entries()
            .map(|(layer, entry)| {
                let above = entry.shapes.iter().filter(|shape| shape.sense.takes_press() && own.is_some_and(|own| rank(shape) > own));
                (layer, above.map(|shape| shape.shape.bounds()).collect())
            })
            .collect();
        (widgets, layers)
    });
    let above = layers
        .into_iter()
        .flat_map(|(layer, bounds)| {
            let to_screen = areas::layer_to_screen(ctx, layer);
            bounds.into_iter().map(move |rect| to_screen * rect)
        })
        .collect();
    OverPart { widgets, above }
}

/// The hit areas the running pass has registered so far for the components `of` takes, in screen points: each shape's
/// bounds grown to its least size across, with the component and part it belongs to.
pub(crate) fn hit_areas(ctx: &Context, of: impl Fn(Id) -> bool) -> Vec<(Id, Option<PartRef>, egui::Rect)> {
    let viewport = pass::current_viewport(ctx);
    let found: Vec<(Layer, Id, Option<PartRef>, egui::Rect)> = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let mut found = Vec::new();
        for (layer, roots) in &record.this {
            for shape in roots.values().flat_map(|root| root.shapes.iter()).filter(|shape| of(shape.owner)) {
                let bounds = shape.shape.bounds();
                let grow = (egui::Vec2::splat(shape.min_across) - bounds.size()).max(egui::Vec2::ZERO) / 2.0;
                found.push((*layer, shape.owner, shape.part, bounds.expand2(grow)));
            }
        }
        found
    });
    found
        .into_iter()
        .map(|(layer, owner, part, rect)| {
            let to_screen = areas::layer_to_screen(ctx, layer);
            (owner, part, to_screen * rect)
        })
        .collect()
}

/// The parts of the root `root` that sense `sense` in last pass's shape record, in registration order.
#[cfg(feature = "testing")]
pub(crate) fn sensing_parts(ctx: &Context, root: Id, sense: craft_behavior::Sense) -> Vec<craft_behavior::PartRef> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let mut parts = Vec::new();
        for layer in record.last.values() {
            if let Some(entry) = layer.get(&root) {
                parts.extend(entry.shapes.iter().filter(|shape| shape.sense.contains(sense)).filter_map(|shape| shape.part));
            }
        }
        parts
    })
}

/// The bounds of the part `part` of `owner` (the arena root or a member) in last pass's shape record: pointer-only
/// parts have no access node, so this record is where their place is read.
pub(crate) fn part_bounds(ctx: &Context, owner: Id, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    owned_part_bounds(ctx, |shape_owner| shape_owner == owner, part)
}

/// Whether the part `part` of `owner` (the arena root or a member) senses all of `sense` in last pass's shape record.
pub(crate) fn part_senses(ctx: &Context, owner: Id, part: craft_behavior::PartRef, sense: craft_behavior::Sense) -> bool {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .any(|shape| shape.owner == owner && shape.part == Some(part) && shape.sense.contains(sense))
    })
}

/// [`part_bounds`] of the owner whose AccessKit node is `node`, for a test that holds the node it found, not the id.
#[cfg(feature = "testing")]
pub(crate) fn node_part_bounds(ctx: &Context, node: crate::runtime::egui::access::NodeId, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    owned_part_bounds(ctx, |shape_owner| crate::runtime::egui::access::node_id(shape_owner) == node, part)
}

fn owned_part_bounds(ctx: &Context, owner: impl Fn(Id) -> bool, part: craft_behavior::PartRef) -> Option<egui::Rect> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .find(|shape| owner(shape.owner) && shape.part == Some(part))
            .map(|shape| shape.shape.bounds())
    })
}

/// The bounds of the part `part` that senses `sense`, on a component other than `subject` and lying inside the
/// subject's own shapes in last pass's shape record: where a component's child keeps the part a drive acts on. When
/// several children match, the first by top edge, then start edge, then root id.
#[cfg(feature = "suite")]
pub(crate) fn child_part_bounds(ctx: &Context, subject: Id, part: craft_behavior::PartRef, sense: craft_behavior::Sense) -> Option<egui::Rect> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        let shapes = || record.last.values().flat_map(|roots| roots.values()).flat_map(|root| root.shapes.iter());
        let room = shapes().filter(|shape| shape.owner == subject).map(|shape| shape.shape.bounds()).reduce(|all, next| all.union(next))?;
        shapes()
            .filter(|shape| {
                shape.owner != subject && shape.part == Some(part) && shape.sense.contains(sense) && room.contains_rect(shape.shape.bounds())
            })
            .min_by(|a, b| {
                let (a_bounds, b_bounds) = (a.shape.bounds(), b.shape.bounds());
                a_bounds.min.y.total_cmp(&b_bounds.min.y).then(a_bounds.min.x.total_cmp(&b_bounds.min.x)).then(a.owner.value().cmp(&b.owner.value()))
            })
            .map(|shape| shape.shape.bounds())
    })
}

/// The parts in last pass's shape record that take a press (a click or a drag), each with its owner and bounds: with
/// the access tree, where the pointer-only ones lie, which have no node to read a place from.
#[cfg(feature = "suite")]
pub(crate) fn press_parts(ctx: &Context) -> Vec<(Id, craft_behavior::PartRef, egui::Rect)> {
    let viewport = pass::current_viewport(ctx);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        record
            .last
            .values()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .filter(|shape| shape.sense.intersects(craft_behavior::Sense::CLICK | craft_behavior::Sense::DRAG))
            .filter_map(|shape| Some((shape.owner, shape.part?, shape.shape.bounds())))
            .collect()
    })
}

/// The roots and egui widgets the root viewport's latest pass registered: what bypass detection counts as made by
/// components.
#[cfg(feature = "testing")]
pub(crate) fn registered_widgets(ctx: &Context) -> std::collections::HashSet<Id> {
    PerViewport::<Shapes>::with(ctx, egui::ViewportId::ROOT, |record| {
        record
            .this
            .values()
            .flat_map(|roots| roots.iter())
            .flat_map(|(root, rec)| std::iter::once(*root).chain(rec.widgets.iter().copied()))
            .collect()
    })
}

/// Whether `id` is a root, an egui widget that a component registered, or a widget a component made for its text or
/// editor part, in the running pass of `viewport`.
pub(crate) fn made_by_component(ctx: &Context, viewport: ViewportId, id: Id) -> bool {
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        #[cfg(feature = "testing")]
        {
            record.made_lookups += 1;
        }
        record.this_made.contains(&id)
    })
}

/// How many `made_by_component` lookups the running pass of `viewport` made so far, for the count trials.
#[cfg(feature = "testing")]
pub(crate) fn made_lookups(ctx: &Context, viewport: ViewportId) -> u32 {
    PerViewport::<Shapes>::with(ctx, viewport, |record| record.made_lookups)
}

/// The roots a press at a point goes to, each with the shape of it that wins it: its click's and its drag's (8.3, H7).
/// Among the roots under the point ([`Under`]), each resolves its own shapes by the arena rule, and the roots rank by
/// their winning shapes' place in the pass's paint order, top first. The top one takes every kind of press its shape
/// senses. Under a top shape that only drags (a window's resize grip), the click goes to the first root below whose
/// shape clicks; under one that only clicks (a button), the drag goes to the first root below whose shape drags, when
/// that root takes no click anywhere. Under a top shape that clicks and passes its drag (`Sense::PASSES_DRAG`), the
/// drag goes to the next shape under it that drags: its own root's, else the first root's below.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct PressWinners {
    pub click: Option<(Winner<Id>, Id)>,
    pub drag: Option<(Winner<Id>, Id)>,
}

impl PressWinners {
    /// The root that starts the press for its kinds of press, with the kinds it takes.
    pub(crate) fn senses_of(&self, root: Id) -> craft_behavior::Sense {
        let mut sense = craft_behavior::Sense::empty();
        if self.click.is_some_and(|(_, owner)| owner == root) {
            sense |= craft_behavior::Sense::CLICK;
        }
        if self.drag.is_some_and(|(_, owner)| owner == root) {
            sense |= craft_behavior::Sense::DRAG;
        }
        sense
    }

    /// The shape of `root` a press goes to: its click's, else its drag's.
    pub(crate) fn hit_of(&self, root: Id) -> Option<Winner<Id>> {
        [self.click, self.drag].into_iter().flatten().find(|(_, owner)| *owner == root).map(|(winner, _)| winner)
    }
}

/// What lies under one point of a viewport, from last pass's shape record: the one answer a press and the hover read
/// (8.3). On the topmost layer under the point, the roots recorded there in the last pass stack in paint order; a
/// transformed body's layer shows its root's own shapes beneath everything on it. A shape counts only where its clip
/// shows it; at a point no clip on the layer holds (one automation names past the window), a shape counts when some of
/// it shows inside its clip. The members of an arena whose host withholds them (`withhold_members`) do not count.
/// Where egui's hit test gives the point to a widget no component made (a raw egui widget on top), no press or hover
/// goes to a root; where it gives it to a widget a component made for its own text, the press is that widget's and the
/// hover goes to that component's topmost shape under the point. A layer's own widget, beneath everything on the layer
/// ([`record_layer_widget`]), is no widget there. The roots that contain the point are those with a shape under it
/// either way.
///
/// Hover shows where a press goes: the press winners' shapes hover, and so does the topmost shape under the point when
/// it takes no press (a label with a tooltip over a clickable row). A shape that takes a press and loses it does not.
#[derive(Clone, Debug, Default)]
pub(crate) struct Under {
    /// The roots a press at the point starts on.
    pub press: PressWinners,
    /// The shape that hovers beside the press winners, with its root: the topmost shape under the point when it takes
    /// no press, or the topmost one of the component whose text widget takes the press.
    pub hover_top: Option<(Winner<Id>, Id)>,
    /// Every root with a shape under the point, on top or not: the roots that contain the pointer.
    pub over: SmallVec<[Id; 4]>,
}

impl Under {
    /// The shape of `root` that hovers beside the press winners ([`Under::hover_top`]).
    pub(crate) fn hover_top_of(&self, root: Id) -> Option<Winner<Id>> {
        self.hover_top.filter(|(_, owner)| *owner == root).map(|(winner, _)| winner)
    }

    /// Whether a shape of `root` lies under the point.
    pub(crate) fn contains(&self, root: Id) -> bool {
        self.over.contains(&root)
    }
}

/// The press winners at `at` for egui's press targets `targets` ([`Under::press`]): the pass's hover answer when the
/// press is at the hover point, else found once per point and targets. A secondary press (`button`) is resolved among
/// the shapes that sense `Sense::CONTEXT_CLICK` alone: a shape on top that takes no context click passes it to the next
/// one under it, so a box that only drags leaves a right-click to the page beneath.
pub(crate) fn press_winners(ctx: &Context, at: Pos2, targets: PressTargets, button: PointerButton) -> PressWinners {
    let viewport = pass::current_viewport(ctx);
    if button == PointerButton::Secondary {
        return find_under(ctx, viewport, at, targets, true).press;
    }
    let found = PerViewport::<Shapes>::with(ctx, viewport, |record| {
        [&record.hover, &record.winner]
            .into_iter()
            .flatten()
            .find(|(point, asked, _)| *point == at && *asked == targets)
            .map(|(_, _, under)| under.press)
    });
    if let Some(found) = found {
        return found;
    }
    let under = find_under(ctx, viewport, at, targets, false);
    let found = under.press;
    PerViewport::<Shapes>::with(ctx, viewport, |record| record.winner = Some((at, targets, under)));
    found
}

/// The root a press at `at` belongs to, with its winning shape: the click's root (`press_winners`), else the drag's.
/// What the focus service classifies a press by and a host asks a press about.
pub(crate) fn press_winner(ctx: &Context, at: Pos2, targets: PressTargets) -> Option<(Winner<Id>, Id)> {
    let winners = press_winners(ctx, at, targets, PointerButton::Primary);
    winners.click.or(winners.drag)
}

/// Reads what lies under the running pass's hover point in `viewport` (`None` while the pointer is not in the window),
/// and whether a press was open when the pass began: then only that press's target hovers. `f` runs inside the shape
/// record's borrow.
pub(crate) fn with_hover<R>(ctx: &Context, viewport: ViewportId, f: impl FnOnce(Option<&Under>, bool) -> R) -> R {
    PerViewport::<Shapes>::with(ctx, viewport, |record| f(record.hover.as_ref().map(|(_, _, under)| under), record.captured))
}

/// Whether a shape the component `id` registered shows hover in the running pass, by the pass's one resolution of
/// what lies under the pointer: it takes the press there, or it is the topmost shape and takes none. A
/// container reads it for the components in its body before they run. While a press is open it holds for none: only
/// the press's target hovers, and that target reads it from its press.
pub fn hovers(ctx: &Context, id: Id) -> bool {
    with_hover(ctx, pass::current_viewport(ctx), |under, captured| {
        !captured
            && under.is_some_and(|under| {
                [under.press.click, under.press.drag, under.hover_top].into_iter().flatten().any(|(winner, _)| winner.owner == id)
            })
    })
}

/// The resolutions of what lies under a point the running viewport's latest pass made.
#[cfg(feature = "testing")]
pub(crate) fn resolutions(ctx: &Context) -> u32 {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| record.resolutions)
}

/// What lies under `at` ([`Under`]); with `context`, for a secondary press: its press winners only among the shapes
/// that sense `Sense::CONTEXT_CLICK`.
fn find_under(ctx: &Context, viewport: ViewportId, at: Pos2, targets: PressTargets, context: bool) -> Under {
    #[cfg(feature = "testing")]
    PerViewport::<Shapes>::with(ctx, viewport, |record| record.resolutions += 1);
    let Some(layer) = areas::layer_at(ctx, at).or_else(|| layer_past_window(ctx, viewport, at)) else {
        return Under::default();
    };
    let to_layer = |layer: Layer| areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
    let body = PerViewport::<Shapes>::with(ctx, viewport, |record| record.last_bodies.get(&layer).copied());
    let beneath = body.map(|(root, parent)| (root, parent, to_layer(parent)));
    let on_layer = to_layer(layer);
    let withheld = withheld_roots(ctx, viewport);
    PerViewport::<Shapes>::with(ctx, viewport, |record| {
        // Bottom to top: the body layer's root on its own layer, then the roots on the layer in paint order.
        let mut stack: Vec<(u8, Id, &RootRecord, Pos2)> = Vec::new();
        if let Some((root, parent, point)) = beneath
            && let Some(entry) = record.last.get(&parent).and_then(|roots| roots.get(&root))
        {
            stack.push((0, root, entry, point));
        }
        if let Some(roots) = record.last.get(&layer) {
            stack.extend(roots.iter().map(|(root, entry)| (1, *root, entry, on_layer)));
        }
        let shown = stack.iter().any(|(_, _, entry, point)| entry.placed.iter().any(|placed| placed.clip.contains(*point)));
        let click = targets.click.filter(|widget| !record.last_grounds.contains(widget));
        let text_of = click.and_then(|widget| record.last_texts.get(&widget).copied());
        let foreign = text_of.is_none() && click.is_some_and(|widget| !registered_in(&record.last, widget));
        let mut under = Under::default();
        // The topmost shape under the point (of the text widget's component, when one takes the press), and each root's
        // topmost shape that takes a press, ranked by tier and then by the place in paint order of the registration
        // that recorded it.
        let mut top: Option<((u8, u32), Winner<Id>, Id)> = None;
        let mut ranked = Vec::new();
        for (tier, root, entry, point) in stack {
            let members = !withheld.contains(&root);
            let shows = |index: usize| {
                let (shape, placed) = (&entry.shapes[index], entry.placed[index]);
                if shown { placed.clip.contains(point) } else { shape.shape.bounds().intersects(placed.clip) }
            };
            let kept: Vec<usize> = (0..entry.shapes.len())
                .filter(|&index| (members || entry.shapes[index].owner == root) && entry.shapes[index].hits(point) && shows(index))
                .collect();
            if kept.is_empty() {
                continue;
            }
            if !under.over.contains(&root) {
                under.over.push(root);
            }
            // The winner among the shapes at `indices` by `kind`, with its rank.
            let pick = |indices: &[usize], kind: ResolveFor| {
                let mut shapes: Vec<Shape<Id>> = indices.iter().map(|&index| entry.shapes[index].clone()).collect();
                if context && kind == ResolveFor::Press {
                    // A context-only part is a click candidate for this secondary press, while
                    // its unmodified sense must still let primary presses reach what is below.
                    for shape in &mut shapes {
                        if shape.sense.contains(craft_behavior::Sense::CONTEXT_CLICK) {
                            shape.sense.insert(craft_behavior::Sense::CLICK);
                        }
                    }
                }
                resolve(&shapes, point, kind).map(|mut winner| {
                    winner.index = indices[winner.index];
                    ((tier, entry.placed[winner.index].order), winner)
                })
            };
            let owned: Vec<usize>;
            let hovering: &[usize] = match text_of {
                Some(owner) => {
                    owned = kept.iter().copied().filter(|&index| entry.shapes[index].owner == owner).collect();
                    &owned
                }
                None => &kept,
            };
            if let Some((rank, winner)) = pick(hovering, ResolveFor::Hover)
                && top.as_ref().is_none_or(|(best, ..)| rank > *best)
            {
                top = Some((rank, winner, root));
            }
            let takes_context = |index: &usize| {
                let sense = entry.shapes[*index].sense;
                !sense.takes_press() || sense.contains(craft_behavior::Sense::CONTEXT_CLICK)
            };
            let context_kept: Vec<usize>;
            let pressable: &[usize] = if context {
                context_kept = kept.iter().copied().filter(takes_context).collect();
                &context_kept
            } else {
                &kept
            };
            if let Some((rank, winner)) = pick(pressable, ResolveFor::Press) {
                // A shape that passes its drag keeps the click; the root's next shape under it that drags follows it
                // in the ranking, at its rank: the stable sort in `winners_of` keeps it right after the winner.
                let passes = winner.sense.contains(craft_behavior::Sense::CLICK | craft_behavior::Sense::PASSES_DRAG)
                    && !winner.sense.contains(craft_behavior::Sense::DRAG);
                let below = passes
                    .then(|| {
                        let dragging: Vec<usize> = pressable
                            .iter()
                            .copied()
                            .filter(|&index| {
                                let shape = &entry.shapes[index];
                                index != winner.index
                                    && shape.sense.contains(craft_behavior::Sense::DRAG)
                                    && (!winner.drag_to_root || shape.owner == root)
                            })
                            .collect();
                        pick(&dragging, ResolveFor::Press)
                    })
                    .flatten();
                ranked.push((rank, winner, root, entry.click));
                if let Some((_, under)) = below {
                    ranked.push((rank, under, root, entry.click));
                }
            }
        }
        if foreign {
            return under;
        }
        if text_of.is_none() {
            under.press = winners_of(ranked);
        }
        under.hover_top = top.filter(|(_, winner, _)| text_of.is_some() || !winner.sense.takes_press()).map(|(_, winner, root)| (winner, root));
        under
    })
}

/// A candidate shape under a point: its rank, the winner it makes, its root and whether the root takes clicks
/// anywhere.
type Ranked = ((u8, u32), Winner<Id>, Id, bool);

/// The press winners among `ranked`, each root's topmost shape under a point that takes a press with its rank and
/// whether the root takes clicks anywhere, and under a top shape that passes its drag the root's next shape that drags:
/// by the per-kind rule of [`PressWinners`].
fn winners_of(mut ranked: Vec<Ranked>) -> PressWinners {
    ranked.sort_by_key(|entry| std::cmp::Reverse(entry.0));
    let mut winners = PressWinners::default();
    for (_, winner, root, root_clicks) in ranked {
        let (clicks, drags) = (winner.sense.contains(craft_behavior::Sense::CLICK), winner.sense.contains(craft_behavior::Sense::DRAG));
        match (winners.click.is_some(), winners.drag.is_some()) {
            // The top root takes what its shape senses; one that takes both kinds takes the whole press.
            (false, false) => {
                winners.click = clicks.then_some((winner, root));
                winners.drag = drags.then_some((winner, root));
                if clicks && drags {
                    break;
                }
            }
            // Under a top shape that only drags, the first one below that clicks takes the click.
            (false, true) => {
                if clicks {
                    winners.click = Some((winner, root));
                    break;
                }
            }
            // Under a top shape that only clicks, the first root below whose shape drags takes the drag, unless the
            // root takes clicks anywhere: a drag-only root (a scroll view) is dragged from a button in it. Under a top
            // shape that passes its drag, the next shape that drags takes it, in the same root or one below.
            (true, false) => {
                let passes = winners.click.is_some_and(|(top, _)| top.sense.contains(craft_behavior::Sense::PASSES_DRAG));
                let root_target = winners.click.is_some_and(|(top, owner)| top.drag_to_root && root != owner);
                if drags && !root_target && (passes || !(clicks || root_clicks)) {
                    winners.drag = Some((winner, root));
                }
                break;
            }
            (true, true) => break,
        }
    }
    winners
}

/// The layer a press past the window goes to, where egui has none (only automation names such a point: a document
/// point off the window): the topmost layer of last pass's shape record with a shape there that takes a press and
/// shows some of itself inside its clip.
fn layer_past_window(ctx: &Context, viewport: ViewportId, at: Pos2) -> Option<Layer> {
    let mut layers: Vec<Layer> = PerViewport::<Shapes>::with(ctx, viewport, |record| record.last.keys().copied().collect());
    layers.sort_by_key(|layer| std::cmp::Reverse(areas::layer_position(ctx, *layer)));
    layers.into_iter().find(|layer| {
        let point = areas::layer_map(ctx, *layer).map_or(at, |map| map.to_layer(at));
        PerViewport::<Shapes>::with(ctx, viewport, |record| {
            record.last.get(layer).is_some_and(|roots| {
                roots.values().any(|entry| {
                    entry
                        .shapes
                        .iter()
                        .zip(&entry.placed)
                        .any(|(shape, placed)| shape.sense.takes_press() && shape.hits(point) && shape.shape.bounds().intersects(placed.clip))
                })
            })
        })
    })
}

/// Whether some root registered the egui widget `widget` in `layers`.
fn registered_in(layers: &LayerRecord, widget: Id) -> bool {
    layers.values().any(|roots| roots.values().any(|root| root.widgets.contains(&widget)))
}

/// Whether egui gives the latest press to a component's text widget (a focused field's), as `targets` name its
/// widgets: the press is that text's, though it goes to no root and has no winner (`find_under`).
pub(crate) fn text_takes_press(ctx: &Context, targets: PressTargets) -> bool {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        targets.click.is_some_and(|widget| !record.last_grounds.contains(&widget) && record.last_texts.contains_key(&widget))
    })
}

/// Whether the egui widget `widget`, one of the latest press's targets, is one no component registered or made for its
/// text, and no layer shows beneath everything on it, in the pass the press was hit-tested against: a raw egui widget.
pub(crate) fn foreign_target(ctx: &Context, widget: Id) -> bool {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        !registered_in(&record.last, widget) && !record.last_texts.contains_key(&widget) && !record.last_grounds.contains(&widget)
    })
}

/// Every shape of the last pass on the topmost layer at `at` (screen points) whose area contains it, as
/// `(owner, part)`, sensing or not, in no particular order; empty where no layer is. The hit record a press is
/// classified on, for rules that ask what lies under a point rather than who wins it.
pub fn shapes_at(ctx: &Context, at: Pos2) -> Vec<(Id, Option<PartRef>)> {
    let Some(layer) = areas::layer_at(ctx, at) else { return Vec::new() };
    // Every shape recorded on a layer lies in that layer's space.
    let at = areas::layer_map(ctx, layer).map_or(at, |map| map.to_layer(at));
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record
            .last
            .get(&layer)
            .into_iter()
            .flat_map(|roots| roots.values())
            .flat_map(|root| root.shapes.iter())
            .filter(|shape| shape.hits(at))
            .map(|shape| (shape.owner, shape.part))
            .collect()
    })
}

/// The part of `owner` holding a press in this viewport that was open when the pass began: in the pass that ends the
/// press too, and in none after it.
pub fn pressed_part(ctx: &Context, owner: Id) -> Option<PartRef> {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        record.pressed_parts.iter().find_map(|(id, part)| (*id == owner).then_some(*part))
    })
}

/// The part of `owner` holding the pointer's hover or an open press in this viewport.
///
/// An open press keeps its part while the pointer moves away. A press open at the start of the pass keeps its part
/// for that whole pass, so a child can still declare it after an enclosing arena has processed the release.
pub fn active_part(ctx: &Context, owner: Id) -> Option<PartRef> {
    let pressed = pressed_part(ctx, owner);
    if pressed.is_some() {
        return pressed;
    }
    with_hover(ctx, pass::current_viewport(ctx), |under, captured| {
        if captured {
            return None;
        }
        let under = under?;
        [under.press.click, under.press.drag, under.hover_top]
            .into_iter()
            .flatten()
            .find_map(|(winner, _)| (winner.owner == owner).then_some(winner.part).flatten())
    })
}

/// The clip the root of the arena `component` is in (its own, or that of the root it is a member of) was shown
/// through, from its latest record in the running viewport: where its parts and members can be seen. `None` for a
/// component no root recorded.
#[cfg(feature = "testing")]
pub(crate) fn arena_clip(ctx: &Context, component: Id) -> Option<Rect> {
    PerViewport::<Shapes>::with(ctx, pass::current_viewport(ctx), |record| {
        [&record.this, &record.last].into_iter().find_map(|layers| {
            let holds = |root: Id, entry: &RootRecord| root == component || entry.shapes.iter().any(|shape| shape.owner == component);
            let root = layers.values().find_map(|roots| roots.iter().find(|(root, entry)| holds(**root, entry)).map(|(root, _)| *root))?;
            let background =
                |shape: &Shape<Id>| shape.owner == root && shape.part.is_none() && shape.priority.level == craft_behavior::HitLevel::Background;
            layers
                .values()
                .filter_map(|roots| roots.get(&root))
                .find_map(|entry| entry.shapes.iter().zip(&entry.placed).find(|(shape, _)| background(shape)).map(|(_, placed)| placed.clip))
        })
    })
}

/// The screen bounds of the root `root` from its shape record, with the viewport and layer they were recorded in: the
/// running pass's shapes when the root has registered in it, else the previous pass's. Searched over the live
/// viewports, so the input hook reads it before the pass begins.
pub(crate) fn root_bounds(ctx: &Context, root: Id) -> Option<(ViewportId, Layer, egui::Rect)> {
    pass::live_viewports(ctx).into_iter().find_map(|viewport| {
        let (layer, bounds) = PerViewport::<Shapes>::with(ctx, viewport, |record| {
            [&record.this, &record.last].into_iter().find_map(|layers| {
                layers.iter().find_map(|(layer, roots)| {
                    let shapes = &roots.get(&root)?.shapes;
                    let bounds = shapes.iter().fold(egui::Rect::NOTHING, |bounds, shape| bounds.union(shape.shape.bounds()));
                    bounds.is_positive().then_some((*layer, bounds))
                })
            })
        })?;
        let screen =
            areas::layer_map(ctx, layer).map_or(bounds, |map| egui::Rect::from_two_pos(map.to_screen(bounds.min), map.to_screen(bounds.max)));
        Some((viewport, layer, screen))
    })
}
