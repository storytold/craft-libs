//! `CanvasTool` (8.3): the tool a canvas hands its background presses to, with what a call reads.
//!
//! A tool holds its own state between passes and reads nothing from `egui`. [`ToolHost`](super::ToolHost) maps the
//! canvas's press, hover and keys to the trait's calls; pointer positions are screen points, and the tool maps
//! them to document units with the canvas's view ([`ToolContext::space`]), so it holds no copy of the view.

use super::{CanvasSpace, MARCH_SECONDS, MARCH_TICK, Snap, ToolPath, march_shift};
use crate::contract::{DragPreview, Pattern, PatternKind, PatternProbe, Tooltip};
use crate::primitives::{ContentPainterCx, Draw};
use crate::runtime::env::{Direction, Env, Label, MotionPolicy, PointSize, UnitWords, Units, WordSet, Words};
use crate::runtime::motion::loop_phase;
use crate::style::StyleCx;
use craft_behavior::geometry::{SnapKinds, SnapLine};
use craft_behavior::kurbo::{self, Point};
use craft_behavior::{Axis, CancelReason, CanvasView, Cursor, CursorIcon, GestureId, PointerButton, PointerKind, PointerSample};
use craft_i18n::{Digits, Format, Region};
use craft_keymap::{Chord, Key, Mods};
use egui::{Pos2, Rect, Vec2};
use std::any::Any;
use std::sync::{Arc, OnceLock};

/// A pointer sample delivered to a tool, in screen points, with the pen's reading. A mouse reports pressure 1, no
/// tilt and no eraser.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolPointer {
    /// The pointer on screen.
    pub pos: Pos2,
    /// The modifiers held when the sample was taken.
    pub mods: Mods,
    /// Pressure, tilt, rotation and the eraser end.
    pub sample: PointerSample,
    /// The button the gesture runs with; the primary for a hover.
    pub button: PointerButton,
    /// How many presses make up the click run the gesture began with: 1 for a lone press, 2 for a double press.
    /// 0 for a sample with no press (a hover).
    pub clicks: u32,
    /// The time of the sample, in seconds: the pass that took it. A press carries the time the button went down,
    /// and a drag or release sample the time of the pass it arrived in.
    pub time: f64,
    /// Mouse, pen or touch, from the pass's pointer frame.
    pub kind: PointerKind,
}

impl ToolPointer {
    /// A mouse sample of `button` at `pos` and `time`, with no modifiers and no press.
    pub const fn new(pos: Pos2, button: PointerButton, time: f64) -> Self {
        ToolPointer { pos, mods: Mods::NONE, sample: PointerSample::new(1.0), button, clicks: 0, time, kind: PointerKind::Mouse }
    }

    /// The modifiers held.
    #[must_use]
    pub const fn mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    /// The pen's reading.
    #[must_use]
    pub const fn sample(mut self, sample: PointerSample) -> Self {
        self.sample = sample;
        self
    }

    /// The device kind of the sample.
    #[must_use]
    pub const fn kind(mut self, kind: PointerKind) -> Self {
        self.kind = kind;
        self
    }

    /// The length of the click run.
    #[must_use]
    pub const fn clicks(mut self, clicks: u32) -> Self {
        self.clicks = clicks;
        self
    }
}

/// What a tool does with a press.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolAction {
    /// The tool owns the gesture: it gets the drags and the release.
    Claimed,
    /// A click the tool handled whole (a Zoom step, an eyedropper sample): no drag follows.
    Done,
    /// The press is not the tool's: the host drops the whole press.
    Ignored,
}

/// A key the host passes to the tool while the canvas has the keyboard and no text part keeps it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ToolKey {
    /// An arrow key: a unit direction on screen (not mirrored in a right-to-left interface), the modifiers and the
    /// keys held with it.
    #[non_exhaustive]
    Nudge {
        /// The direction.
        direction: Vec2,
        /// The modifiers held.
        mods: Mods,
        /// The keys down when the arrow went down, the arrow not among them ([`Intents::nudge_held`]): a key held
        /// with the arrow (Backtick, to move only a fill's pattern), read in the order of the frame's events, where
        /// [`ToolContext::held`] is the state after them.
        ///
        /// [`Intents::nudge_held`]: crate::runtime::keys::Intents::nudge_held
        held: Vec<Key>,
    },
    /// Delete (`forward`) or Backspace, while the tool claims the deletes ([`ToolClaims::deletes`]).
    #[non_exhaustive]
    Delete {
        /// Delete toward the reading end.
        forward: bool,
    },
    /// Enter, with Alt or Shift held or not ([`ToolContext::mods`] says which).
    Submit,
    /// Escape with no gesture running.
    Cancel,
    /// A key press the tool kept with [`ToolClaims::keyboard`]: every key without Ctrl or Cmd, so the app's chords
    /// still run, but Enter, Escape, Delete and Backspace, which arrive as `Submit`, `Cancel` and `Delete`. It carries
    /// no release; [`ToolContext::held`] says whether the key is still down.
    Kept(Chord),
    /// Alt (Option) pressed and let go with nothing between, while the tool claims it ([`ToolClaims::alt_tap`]).
    AltTap,
}

impl ToolKey {
    /// An arrow key: a unit direction on screen and the modifiers held, with no other key held; what the host passes
    /// for an arrow, built by a test of a tool.
    pub fn nudge(direction: Vec2, mods: Mods) -> Self {
        ToolKey::Nudge { direction, mods, held: Vec::new() }
    }

    /// Delete (`forward`) or Backspace.
    pub fn delete(forward: bool) -> Self {
        ToolKey::Delete { forward }
    }
}

/// How a view change eases to its target instead of jumping, as a zoom click does under Animated Zoom.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ViewEase {
    /// A document point and the screen point it stays at while the change eases, for a zoom about the pointer.
    pub anchor: Option<(Point, Pos2)>,
}

/// What a tool is to assistive technology this pass: one graphics object over the tool's drawing, with a child for
/// each anchor. The host emits it through the canvas's access path in [`ToolHost::run`](super::ToolHost::run), as
/// a hover-only node that is not a Tab stop. The texts are the tool's words in the app's language, with the live
/// size in the units of the document ([`ToolContext::units`]) as the value: `Rectangle` valued `120 x 80 px` while a
/// rectangle is dragged.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct ToolAccess {
    /// The object's name.
    pub name: String,
    /// What the object is or does, beyond its name.
    pub description: Option<String>,
    /// The object's current value, such as the live size a drag reads out (`120 x 80 px`).
    pub value: Option<String>,
    /// The drawing's bounds in document units. `None` gives a node of no size at the canvas centre.
    pub bounds: Option<craft_behavior::kurbo::Rect>,
    /// The points of the drawing the user can grab or reach, in reading order.
    pub anchors: Vec<ToolAnchor>,
}

impl ToolAccess {
    /// An object named `name`, with no description, value, bounds or anchors.
    pub fn new(name: impl Into<String>) -> Self {
        ToolAccess { name: name.into(), description: None, value: None, bounds: None, anchors: Vec::new() }
    }

    /// The object's description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }

    /// The object's current value.
    #[must_use]
    pub fn value(mut self, value: impl Into<String>) -> Self {
        self.value = Some(value.into());
        self
    }

    /// The drawing's bounds in document units.
    #[must_use]
    pub fn bounds(mut self, bounds: craft_behavior::kurbo::Rect) -> Self {
        self.bounds = Some(bounds);
        self
    }

    /// Adds an anchor after those already given.
    #[must_use]
    pub fn anchor(mut self, anchor: ToolAnchor) -> Self {
        self.anchors.push(anchor);
        self
    }
}

/// A point of a tool's drawing for assistive technology (a pen's anchor point, a crop corner).
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolAnchor {
    /// The anchor's name.
    pub name: String,
    /// Where it lies, in document units.
    pub at: Point,
    /// What the anchor adds to its name, such as its coordinates.
    pub description: Option<String>,
}

impl ToolAnchor {
    /// An anchor named `name` at `at`.
    pub fn new(name: impl Into<String>, at: Point) -> Self {
        ToolAnchor { name: name.into(), at, description: None }
    }

    /// The anchor's description.
    #[must_use]
    pub fn description(mut self, description: impl Into<String>) -> Self {
        self.description = Some(description.into());
        self
    }
}

/// What a tool asked the host for in a call; the host applies it after the call.
#[derive(Clone, Debug, Default)]
pub(crate) struct ToolRequests {
    pub(crate) readout: Option<Vec<(Label, String)>>,
    pub(crate) repaint: bool,
    pub(crate) view: Option<CanvasView>,
    pub(crate) ease: Option<ViewEase>,
    pub(crate) snapped: Vec<SnapLine>,
    pub(crate) autoscroll_at: Option<Pos2>,
    /// The pan's scroll-all-windows choice, from [`ToolContext::pan_view`].
    pub(crate) pan: Option<bool>,
    /// Rows the Show Transformation Values gate never silences, from [`ToolContext::hud`].
    pub(crate) hud: Option<Vec<(Label, String)>>,
    /// The content a tool handed to a drag session, from [`ToolContext::start_drag`].
    pub(crate) drag: Option<DragRequest>,
    /// The pointer tip, from [`ToolContext::tip`].
    pub(crate) tip: Option<Tooltip>,
}

/// A drag session a tool asked for: the payload, the extra content some targets read, and the ghost.
#[derive(Clone, Debug)]
pub(crate) struct DragRequest {
    pub(crate) payload: Arc<dyn Any + Send + Sync>,
    pub(crate) also: Option<Arc<dyn Any + Send + Sync>>,
    pub(crate) preview: DragPreview,
}

/// What a tool sees and may ask for during a call.
#[non_exhaustive]
pub struct ToolContext<'a> {
    /// The pointer's last sample this pass; `None` while the pointer is off the canvas and no gesture runs.
    pub pointer: Option<ToolPointer>,
    /// The modifiers held now.
    pub mods: Mods,
    /// The canvas on screen.
    pub rect: Rect,
    /// The pass time, in seconds.
    pub time: f64,
    /// The press arena's slop, in screen points: the least movement that makes a press a drag, and the least size a
    /// drawn rectangle has to count. 0 for a context built without one.
    pub slop: f32,
    /// Whether Caps Lock is on now (a brush shows its precise cursor): the platform's toggle state, read once per
    /// pass, not a count of key presses.
    pub caps_lock: bool,
    space: Option<CanvasSpace>,
    gesture: Option<GestureId>,
    down: &'a [Key],
    painter: Option<&'a ContentPainterCx<'a>>,
    style: Option<StyleCx<'a>>,
    env: Option<&'a Env>,
    snap: Option<&'a Snap>,
    march: Option<March>,
    requests: ToolRequests,
    over_drop_target: bool,
}

/// What a paint call needs to march ants: the motion mode and the ants' dash length.
#[derive(Clone, Copy)]
pub(crate) struct March {
    pub(crate) policy: MotionPolicy,
    pub(crate) dash: f32,
}

impl<'a> ToolContext<'a> {
    /// A context over the canvas `rect`, which shows its document through `space` (`None` for a canvas without a
    /// view), with no key held: the host builds one per call; a test of a tool builds its own.
    pub fn new(rect: Rect, space: Option<CanvasSpace>) -> Self {
        ToolContext {
            pointer: None,
            mods: Mods::NONE,
            rect,
            time: 0.0,
            slop: 0.0,
            space,
            gesture: None,
            down: &[],
            caps_lock: false,
            painter: None,
            style: None,
            env: None,
            snap: None,
            march: None,
            requests: ToolRequests::default(),
            over_drop_target: false,
        }
    }

    /// The pointer's last sample.
    #[must_use]
    pub fn pointer(mut self, pointer: Option<ToolPointer>) -> Self {
        self.pointer = pointer;
        self
    }

    /// The modifiers held now.
    #[must_use]
    pub fn mods(mut self, mods: Mods) -> Self {
        self.mods = mods;
        self
    }

    /// The pass time.
    #[must_use]
    pub fn time(mut self, time: f64) -> Self {
        self.time = time;
        self
    }

    /// The id of the gesture the host runs, which [`gesture_id`](Self::gesture_id) reads; the host sets it.
    #[must_use]
    pub(crate) fn gesture(mut self, gesture: Option<GestureId>) -> Self {
        self.gesture = gesture;
        self
    }

    /// The keys held now.
    #[must_use]
    pub fn down(mut self, down: &'a [Key]) -> Self {
        self.down = down;
        self
    }

    /// Whether Caps Lock is on now.
    #[must_use]
    pub fn caps_lock(mut self, on: bool) -> Self {
        self.caps_lock = on;
        self
    }

    /// Whether the tool shows its precise cursor now: the canvas's [`CanvasSpace::precise`]; never without a space.
    pub fn precise(&self) -> bool {
        self.space.is_some_and(|space| space.precise())
    }

    /// Whether a drag session the running gesture started has a target that would take it now; the host sets it from
    /// the session's mark.
    #[must_use]
    pub(crate) fn over_target(mut self, over: bool) -> Self {
        self.over_drop_target = over;
        self
    }

    /// The press arena's slop.
    #[must_use]
    pub fn slop(mut self, slop: f32) -> Self {
        self.slop = slop;
        self
    }

    /// The painter of a paint call; the host sets it.
    #[must_use]
    pub(crate) fn painter(mut self, painter: &'a ContentPainterCx<'a>, march: March) -> Self {
        self.painter = Some(painter);
        self.march = Some(march);
        self
    }

    /// The style context of a paint call; the host sets it.
    #[must_use]
    pub(crate) fn styled(mut self, style: StyleCx<'a>) -> Self {
        self.style = Some(style);
        self
    }

    /// The env of the pass, which [`units`](Self::units) and [`words`](Self::words) read; the host sets it. A test of
    /// a tool that formats values sets the env of its `Ui` (`Env::of`).
    #[must_use]
    pub fn env(mut self, env: &'a Env) -> Self {
        self.env = Some(env);
        self
    }

    /// The canvas's snapping, which [`snap`](Self::snap) applies; the host sets the one the canvas has.
    #[must_use]
    pub fn snapping(mut self, snap: &'a Snap) -> Self {
        self.snap = Some(snap);
        self
    }

    /// The canvas's view and interface scale this pass, to map screen points to document units and back at run
    /// time; `None` for a canvas without a view. After [`set_view`](Self::set_view) it shows the view the tool
    /// asked for, so the calls after it in the pass start from there.
    pub fn space(&self) -> Option<CanvasSpace> {
        self.space
    }

    /// Sets the canvas's view: the host reports it in [`ToolHostResponse::view`](super::ToolHostResponse::view) and
    /// the app copies it into its bound view. The last view set in a pass wins.
    pub fn set_view(&mut self, view: CanvasView) {
        self.requests.view = Some(view);
        self.requests.ease = None;
        if let Some(space) = self.space.as_mut() {
            space.set_view(view);
        }
    }

    /// [`set_view`](Self::set_view) for a change that eases to `view` ([`ZoomAnimation`](super::ZoomAnimation)),
    /// keeping `anchor` (a document point and the screen point it stays at) fixed on the way.
    pub fn ease_view(&mut self, view: CanvasView, anchor: Option<(Point, Pos2)>) {
        self.set_view(view);
        self.requests.ease = Some(ViewEase { anchor });
    }

    /// Paints marching ants along `points` (closed when `closed`) in a paint call; it paints nothing in any other.
    /// The ants march on the runtime's pass clock, step by step, and the pass asks for its next repaint a step ahead
    /// only while some are painted; where motion is off they stand still.
    pub fn marching_ants(&self, points: &[Pos2], closed: bool) {
        let (Some(painter), Some(march)) = (self.painter, self.march) else { return };
        let turn = loop_phase(painter.painter.ctx(), march.policy, MARCH_SECONDS, Some(MARCH_TICK));
        painter.marching_ants(points, closed, march_shift(turn, march.dash));
    }

    /// Paints a swath `width` points wide along `points` in the theme's text-selection fill in a paint call; it paints
    /// nothing in any other.
    pub fn swath(&self, points: &[Pos2], width: f32) {
        if let Some(painter) = self.painter {
            painter.swath(points, width);
        }
    }

    /// `point`, a document point under the pointer, moved to what the canvas snaps it to (View > Snap To) with the
    /// modifiers held now; the point itself when the canvas has no snapping or the Control key suspends it
    /// ([`Snap::apply`] decides). The tool builds no snapping of its own.
    pub fn snap(&self, point: Point) -> Point {
        Point::new(self.snap_along(Axis::Y, point.x), self.snap_along(Axis::X, point.y))
    }

    /// [`snap`](Self::snap) to the targets of a kind in `kinds` only, with no lines: a click that selects at the point
    /// leaves out the selection it replaces.
    pub fn snap_of(&self, point: Point, kinds: SnapKinds) -> Point {
        self.snap.map_or(point, |snap| snap.point(point, self.mods, super::GestureSnap::default().kinds(kinds)).0)
    }

    /// The coordinate `position` across a guide that would run along `along`, snapped as [`snap`](Self::snap) does:
    /// an x coordinate is across `Axis::Y`.
    pub fn snap_along(&self, along: Axis, position: f64) -> f64 {
        self.snap.map_or(position, |snap| snap.apply(along, position, self.mods))
    }

    /// The offset that moves `rect` (document units) onto a View > Snap To target with the modifiers held now; zero
    /// when Control suspends snapping or the canvas has no snap.
    pub fn snap_rect(&mut self, rect: kurbo::Rect) -> kurbo::Vec2 {
        self.snap_rect_of(rect, SnapKinds::ALL)
    }

    /// [`snap_rect`](Self::snap_rect) to the targets of a kind in `kinds` only: moving a selection leaves out the
    /// selection's own edges.
    pub fn snap_rect_of(&mut self, rect: kurbo::Rect, kinds: SnapKinds) -> kurbo::Vec2 {
        let Some(snap) = self.snap else { return kurbo::Vec2::ZERO };
        let (offset, lines) = snap.rect_of(rect, self.mods, kinds);
        self.requests.snapped.extend(lines);
        offset
    }

    /// Adds the snap lines a canvas control the tool shows snapped this pass, handed to the host in the same pass.
    pub fn add_snapped(&mut self, lines: impl IntoIterator<Item = SnapLine>) {
        self.requests.snapped.extend(lines);
    }

    /// Sets the view for a pan (as [`set_view`](Self::set_view)); the host reports it as [`ViewPan`](super::ViewPan).
    pub fn pan_view(&mut self, view: CanvasView, scroll_all: bool) {
        self.set_view(view);
        self.requests.pan = Some(scroll_all);
    }

    /// A canvas control the tool shows asks the app to scroll: the host reports `past_edge` for `pointer` through the
    /// tool's drag rule.
    pub fn autoscroll_at(&mut self, pointer: Pos2) {
        self.requests.autoscroll_at = Some(pointer);
    }

    /// The scope's number format and unit words, to write the values of a readout in the document's units and the
    /// app's language. English with the system region's number format for a context built without an env.
    pub fn units(&self) -> Units {
        match self.env {
            Some(env) => env.units(),
            None => {
                let (format, words) = fallback();
                Units::new(Arc::clone(format), words.set::<UnitWords>(), Arc::clone(words), PointSize::default())
            }
        }
    }

    /// The word set `L` in the app's language: the tool's own words for its readout and its [`ToolAccess`]. English
    /// for a context built without an env.
    pub fn words<L: WordSet + Send + Sync + 'static>(&self) -> Arc<L> {
        match self.env {
            Some(env) => env.words::<L>(),
            None => fallback().1.set::<L>(),
        }
    }

    /// The one reach source for tool handles and hit tests: how far from a handle, point or edge a press still takes
    /// it, in screen points. The theme's `size.hit.target` for the pass; the default theme's for a context built
    /// without an env.
    pub fn reach(&self) -> f32 {
        match self.env {
            Some(env) => env.hit_reach(),
            None => crate::runtime::frame::resolved_hit(&craft_theme::AppSizes::default()).target,
        }
    }

    /// The icon size a tool draws its on-canvas icons at, in screen points: the theme's `size.icon.md` for the pass;
    /// the default theme's for a context built without an env.
    pub fn icon_side(&self) -> f32 {
        match self.env {
            Some(env) => env.tokens.size.icon.md,
            None => crate::runtime::frame::resolved_icon_side(&craft_theme::AppSizes::default()),
        }
    }

    /// The interface's reading direction this pass: a tool mirrors what sits at a reading end (a handle on the end
    /// side, an arrow key's step along the text) by it. Left to right for a context built without an env.
    pub fn direction(&self) -> Direction {
        self.env.map_or(Direction::Ltr, |env| env.direction)
    }

    /// The id of the gesture the host runs, from the press the tool claimed through its release or cancel, for
    /// an edit the tool opens under the same id as the host's gesture report. `None` outside a gesture and in a
    /// context built by hand.
    pub fn gesture_id(&self) -> Option<GestureId> {
        self.gesture
    }

    /// The style context of a paint call, to pick the theme colours the tool draws its draft in
    /// ([`StyleCx::color`]) for [`Draw::fill`] and [`Draw::stroke`]; `None` in any other call.
    pub fn style(&self) -> Option<&StyleCx<'a>> {
        self.style.as_ref()
    }

    /// Whether `key` is down now, whatever the tool kept: a marquee that sizes from its centre while Space is held
    /// reads `held(Key::Space)` on every drag. The host withholds Space from the app's shortcuts while a gesture
    /// runs, so a momentary Hand does not take over mid-drag.
    pub fn held(&self, key: Key) -> bool {
        self.down.contains(&key)
    }

    /// Labelled values to show beside the pointer this pass (a marquee's W and H). The last request of a pass wins.
    pub fn readout(&mut self, rows: Vec<(Label, String)>) {
        self.requests.readout = Some(rows);
    }

    /// Labelled values to show beside the pointer this pass, never gated by Show Transformation Values (brush size
    /// readouts). The last request of a pass wins. A hud asked for in the same pass as a readout replaces it.
    pub fn hud(&mut self, rows: Vec<(Label, String)>) {
        self.requests.hud = Some(rows);
    }

    /// Hands `payload` to a drag session shown with `preview`: the content of the gesture the tool runs. The gesture
    /// goes on; the tool keeps its `drag` calls until the release.
    pub fn start_drag<P: Clone + Send + Sync + 'static>(&mut self, payload: P, preview: DragPreview) {
        self.requests.drag = Some(DragRequest { payload: Arc::new(payload), also: None, preview });
    }

    /// [`start_drag`](Self::start_drag) with `also`, extra content that some targets read beside the payload.
    pub fn start_drag_also<P: Clone + Send + Sync + 'static>(&mut self, payload: P, also: Arc<dyn Any + Send + Sync>, preview: DragPreview) {
        self.requests.drag = Some(DragRequest { payload: Arc::new(payload), also: Some(also), preview });
    }

    /// While a drag session this tool started runs, whether a target would take it at the pointer now.
    pub fn over_drop_target(&self) -> bool {
        self.over_drop_target
    }

    /// A tip to show at the pointer where it rests (a link's address and the key that follows it), asked from
    /// [`CanvasTool::hover`]. The host shows it through the framework tooltip, with its look and delay, while no
    /// gesture runs, and drops it when the pointer leaves the canvas or a press starts. The last request of a pass
    /// wins, and a hover that asks for none drops the tip.
    pub fn tip(&mut self, tip: impl Into<Tooltip>) {
        self.requests.tip = Some(tip.into());
    }

    /// Asks for another pass (an animation, a held key that repeats).
    pub fn request_repaint(&mut self) {
        self.requests.repaint = true;
    }

    pub(crate) fn take_requests(&mut self) -> ToolRequests {
        std::mem::take(&mut self.requests)
    }
}

/// The number format and English words of a context built without an env.
fn fallback() -> &'static (Arc<Format>, Arc<Words>) {
    static FALLBACK: OnceLock<(Arc<Format>, Arc<Words>)> = OnceLock::new();
    FALLBACK.get_or_init(|| (Arc::new(Format::new(&Region::system(), Digits::Locale)), Arc::new(Words::english())))
}

/// The keys a tool takes from the app's shortcuts in its current state, read by the host at the end of every pass
/// ([`CanvasTool::claims`]) and held for the next. A key the tool does not claim stays the app's: a plain Delete
/// with nothing to delete runs Edit > Clear, and Alt+Backspace fills with the foreground colour.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolClaims {
    /// The canvas's plain keys: every key without Ctrl or Cmd, with Shift or not, arrives as [`ToolKey::Kept`] and the
    /// app's single-key shortcuts leave it. Enter and Escape stay `Submit` and `Cancel`; the deletes are
    /// [`deletes`](Self::deletes).
    pub keyboard: bool,
    /// The plain Delete and Backspace arrive as [`ToolKey::Delete`] (a path's selected anchor, an open outline's last
    /// point). With [`keyboard`](Self::keyboard) they are also claimed with Alt held: a polygonal lasso removes its
    /// last point while Alt holds the straight-segment switch. Shift, Cmd and Ctrl deletes stay the app's (Fill).
    pub deletes: bool,
    /// The arrows move something the tool holds (a ruler line, selected boards): the host declares Increment and
    /// Decrement on the tool's node and turns them into [`ToolKey::Nudge`], so a tool that claims the nudges
    /// describes what they move in [`CanvasTool::access`]. The arrows reach [`CanvasTool::key`] either way.
    pub nudges: bool,
    /// The tool takes a lone Alt tap, delivered as [`ToolKey::AltTap`].
    pub alt_tap: bool,
    /// Single chords the tool takes while the rest of the keyboard stays the app's, delivered as [`ToolKey::Kept`].
    pub keys: &'static [Chord],
}

impl ToolClaims {
    /// No keys: every key stays the app's.
    pub const NONE: ToolClaims = ToolClaims { keyboard: false, deletes: false, nudges: false, alt_tap: false, keys: &[] };

    /// The same claims with the single chords `keys`.
    #[must_use]
    pub const fn keys(mut self, keys: &'static [Chord]) -> Self {
        self.keys = keys;
        self
    }

    /// The same claims with the canvas's plain keys.
    #[must_use]
    pub const fn keyboard(mut self) -> Self {
        self.keyboard = true;
        self
    }

    /// The same claims with the deletes.
    #[must_use]
    pub const fn deletes(mut self) -> Self {
        self.deletes = true;
        self
    }

    /// The same claims with the nudges.
    #[must_use]
    pub const fn nudges(mut self) -> Self {
        self.nudges = true;
        self
    }

    /// The same claims with the lone Alt tap.
    #[must_use]
    pub const fn alt_tap(mut self) -> Self {
        self.alt_tap = true;
        self
    }
}

/// An area of the canvas where the active tool takes a press over every canvas control
/// ([`CanvasTool::regions`]): a polygon in document units.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ToolRegion {
    /// The corners in order, in document units.
    pub points: Vec<Point>,
}

impl ToolRegion {
    /// The polygon with the corners `points`, in document units.
    pub fn polygon(points: Vec<Point>) -> Self {
        ToolRegion { points }
    }
}

/// A tool on the canvas. The app owns the value and hands it to [`ToolHost::run`](super::ToolHost::run) each
/// pass; a tool with a gesture in progress keeps its state in itself. One `Begin`...`Commit` makes a drag: `press`
/// returning [`ToolAction::Claimed`] begins it, `drag` updates it, `release` commits it, `cancel` ends it.
pub trait CanvasTool {
    /// Called first in every pass, whatever the pointer does: the tool takes this pass's app state and advances
    /// what runs without the pointer (an animation). Default: nothing.
    fn each_pass(&mut self, _cx: &mut ToolContext<'_>) {}

    /// A button the tool takes went down on the canvas away from every canvas control.
    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction;

    /// The pointer moved while the tool owns the gesture.
    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>);

    /// The button came up: commit the gesture.
    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>);

    /// The pointer moved over the canvas with no gesture running (hover outlines, a pen's rubber band). Default:
    /// nothing.
    fn hover(&mut self, _pointer: &ToolPointer, _cx: &mut ToolContext<'_>) {}

    /// The gesture ends without a commit; `reason` says why ([`CancelReason::Escape`] for Escape). The tool drops
    /// its drag state and reports `reason` on the gesture's events.
    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>);

    /// The keys the tool takes in its state at the end of this pass, after its pointer events and keys: the host
    /// holds them from the app's shortcuts for the next pass, so a release that opens a draft has its deletes in the
    /// frame after. Claim only what the tool takes in that state; a claimed key the tool refuses reaches no one.
    /// Default: none.
    fn claims(&self) -> ToolClaims {
        ToolClaims::NONE
    }

    /// The areas where the tool takes a press over every canvas control in the coming pass, in its state before the
    /// pass (a type tool's other text while an edit is open, where a press is the tool's and not the edit's move
    /// reach). The app asks it before it shows the canvas and hands the result to
    /// [`Canvas::tool_regions`](super::Canvas::tool_regions); a press there is the canvas's background press. A tool
    /// that runs a gesture from a press in a region keeps claiming that region until the release: a region that is
    /// gone in the next pass takes the press with it. Default: none.
    fn regions(&self) -> Vec<ToolRegion> {
        Vec::new()
    }

    /// A key while the canvas has the keyboard and no text part keeps it. Return `true` to take it from the app's
    /// shortcuts. Escape during a gesture never reaches here: it cancels. Default: `false`.
    fn key(&mut self, _key: &ToolKey, _cx: &mut ToolContext<'_>) -> bool {
        false
    }

    /// Draws the tool's feedback over the document (a marquee in progress, pen segments). Runs after the canvas
    /// controls.
    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>);

    /// The cursor over the canvas. Default: the system arrow.
    fn cursor(&self, _cx: &ToolContext<'_>) -> Cursor {
        Cursor::System(CursorIcon::Default)
    }

    /// Whether a press of `button` with `mods` held starts a gesture with this tool. A gesture keeps its button
    /// until that button comes up. A brush takes the primary button, and the secondary while Alt is held (the
    /// resize chord). The middle button never reaches `takes`: the canvas turns every middle press into a pan before
    /// the host asks. Default: the primary button.
    fn takes(&self, button: PointerButton, _mods: Mods) -> bool {
        button == PointerButton::Primary
    }

    /// Whether a drag into the band inside the canvas edge or past it asks the app to scroll the view (marquees,
    /// lassos, brushes, crop: yes; the Hand and Zoom tools: no). Default: `true`.
    fn autoscrolls(&self) -> bool {
        true
    }

    /// Whether, in its state after [`each_pass`](Self::each_pass), the tool asks the app to scroll the view while the
    /// pointer hovers near the canvas edge with no button down (a Polygonal Lasso with its outline open, whose rubber band
    /// follows the pointer). While it does, the host calls `hover` in every pass the pointer stays in the edge band,
    /// since the view scrolls under it. Default: `false`.
    fn autoscrolls_on_hover(&self) -> bool {
        false
    }

    /// Whether the tool changes only the view: its primary gestures pass under a modal that keeps the canvas.
    /// Default: `false`.
    fn view_only(&self) -> bool {
        false
    }

    /// Whether, in its state after the press, the host calls `drag` once per pointer move from the press on, the moves
    /// inside the click distance included, instead of once at the latest position past it: the freehand family, and a
    /// gesture that moves content (a selection drag, a layer move), so a move of a pixel or two moves. The host's click
    /// distance does not apply to such a gesture and the tool never reads the release's click: it decides its click
    /// itself, and only there. A brush has none (it paints from where it went down); a content move is a click while
    /// its pointer never left the press ([`MoveGesture::left_press`](craft_behavior::geometry::MoveGesture::left_press)).
    /// `each_pass` runs before the pass's `drag` calls, so a tool that takes every move updates its live stroke once
    /// per pass after `run` returns, never per `drag` call. Default: `false`.
    fn takes_every_move(&self) -> bool {
        false
    }

    /// What the tool is to assistive technology this pass: its drawing as one object with its anchors (a rectangle
    /// being dragged: `Rectangle 120 x 80 px`), or `None` for a tool with nothing to describe. The host emits it
    /// after [`paint`](Self::paint), with the same context. Default: `None`.
    fn access(&self, _cx: &ToolContext<'_>) -> Option<ToolAccess> {
        None
    }

    /// How the tool is reached without a pointer: a tool that creates or selects geometry names the command whose
    /// form takes the same numbers; a freehand tool is `ToolPath::Freehand` with its reason.
    fn command_form(&self) -> ToolPath;
}

/// A borrowed tool is the tool: an app hands a tool it keeps (`&mut tool`) to the host with no wrapper. Every method
/// forwards, the defaults included, so the tool's own overrides hold.
impl<T: CanvasTool + ?Sized> CanvasTool for &mut T {
    fn each_pass(&mut self, cx: &mut ToolContext<'_>) {
        (**self).each_pass(cx);
    }

    fn press(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) -> ToolAction {
        (**self).press(pointer, cx)
    }

    fn drag(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        (**self).drag(pointer, cx);
    }

    fn release(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        (**self).release(pointer, cx);
    }

    fn hover(&mut self, pointer: &ToolPointer, cx: &mut ToolContext<'_>) {
        (**self).hover(pointer, cx);
    }

    fn cancel(&mut self, reason: CancelReason, cx: &mut ToolContext<'_>) {
        (**self).cancel(reason, cx);
    }

    fn claims(&self) -> ToolClaims {
        (**self).claims()
    }

    fn regions(&self) -> Vec<ToolRegion> {
        (**self).regions()
    }

    fn key(&mut self, key: &ToolKey, cx: &mut ToolContext<'_>) -> bool {
        (**self).key(key, cx)
    }

    fn paint(&self, draw: &Draw<'_>, cx: &ToolContext<'_>) {
        (**self).paint(draw, cx);
    }

    fn cursor(&self, cx: &ToolContext<'_>) -> Cursor {
        (**self).cursor(cx)
    }

    fn takes(&self, button: PointerButton, mods: Mods) -> bool {
        (**self).takes(button, mods)
    }

    fn autoscrolls(&self) -> bool {
        (**self).autoscrolls()
    }

    fn autoscrolls_on_hover(&self) -> bool {
        (**self).autoscrolls_on_hover()
    }

    fn view_only(&self) -> bool {
        (**self).view_only()
    }

    fn takes_every_move(&self) -> bool {
        (**self).takes_every_move()
    }

    fn access(&self, cx: &ToolContext<'_>) -> Option<ToolAccess> {
        (**self).access(cx)
    }

    fn command_form(&self) -> ToolPath {
        (**self).command_form()
    }
}

/// The suite's probe of the canvas tool `T`: `#[pattern(tool)]` registers through it, so only a `CanvasTool` takes
/// the attribute.
#[doc(hidden)]
pub const fn tool_probe<T: CanvasTool + Pattern + 'static>(
    crate_name: &'static str,
    crate_target: &'static str,
    module: &'static str,
    purpose: &'static str,
) -> PatternProbe {
    PatternProbe::kinded::<T>(PatternKind::Tool, crate_name, crate_target, module, purpose)
}

/// The tool of a canvas that has none: it takes no press and paints nothing, and its keyboard path is freehand.
#[derive(Clone, Copy, Debug, Default)]
pub struct NoTool;

impl CanvasTool for NoTool {
    fn press(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) -> ToolAction {
        ToolAction::Ignored
    }

    fn drag(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn release(&mut self, _: &ToolPointer, _: &mut ToolContext<'_>) {}

    fn cancel(&mut self, _: CancelReason, _: &mut ToolContext<'_>) {}

    fn paint(&self, _: &Draw<'_>, _: &ToolContext<'_>) {}

    fn command_form(&self) -> ToolPath {
        ToolPath::Freehand("the canvas has no tool, so there is nothing to do by keyboard")
    }
}
