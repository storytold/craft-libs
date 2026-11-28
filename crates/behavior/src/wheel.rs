//! Wheel gesture values, as craft-behavior's copy of egui's types so the wheel split is egui-free.

/// How far one wheel event reaches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WheelUnit {
    /// Points.
    #[default]
    Point,
    /// Lines of text.
    Line,
    /// Pages.
    Page,
}

/// A wheel gesture's phase.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WheelPhase {
    /// The first event of a gesture.
    Start,
    /// A later event.
    #[default]
    Move,
    /// The fingers lifted.
    End,
    /// The system cancelled the gesture.
    Cancel,
}

/// The device a wheel event came from, where the system says (egui reports it on macOS only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum WheelSource {
    /// The system does not say.
    #[default]
    Unknown,
    /// A physical wheel.
    Wheel,
    /// Fingers on a trackpad or a touch surface.
    Trackpad,
    /// The system continuing a finger scroll after the fingers lifted.
    Momentum,
}

impl WheelSource {
    /// Whether fingers drive the scroll: a trackpad or its momentum.
    pub const fn fingers(self) -> bool {
        matches!(self, WheelSource::Trackpad | WheelSource::Momentum)
    }
}

use crate::Sense;
use craft_keymap::Mods;
use emath::{Pos2, Rect, Vec2};

/// One wheel event, or the sum of one pass's, as a taker reads it with `take_wheel`.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Wheel {
    /// Scroll in points, wheel notches and lines already converted by the service.
    pub scroll: Vec2,
    /// The share's scroll in the device's lines (wheel notches) when `unit` is [`WheelUnit::Line`], as the service
    /// read it before it turned them into the taker's points; zero for points and pages. A value control steps once
    /// per whole line ([`wheel_step`](crate::nav::wheel_step)).
    pub lines: Vec2,
    /// Zoom factor; 1 for none.
    pub zoom: f32,
    /// The wheel motion that reached the share as `zoom` instead of `scroll`, in wheel lines: wheel events under the
    /// zoom modifier, and the Ctrl+wheel steps the service reads as a touchpad's pinch on Windows and Linux. A line
    /// event counts its lines (notches) as the device sent them, a point or page event its points over egui's line
    /// scroll speed. It travels with the zoom share, so a taker can count the notches or lines of a Ctrl+wheel that
    /// zooms without the zoom speed; zero for a native pinch and for a wheel without the modifier.
    pub zoom_lines: Vec2,
    /// The running gesture's `zoom_lines` gathered so far, this share included, for the taker that owns the gesture;
    /// another taker's share gathers only itself, as with `gathered`.
    pub zoom_gathered: Vec2,
    /// The modifiers held.
    pub mods: Mods,
    /// The unit the device reported.
    pub unit: WheelUnit,
    /// The gesture phase.
    pub phase: WheelPhase,
    /// The device; a trackpad steps only a value control that is focused or armed (`split`).
    pub source: WheelSource,
    /// Where the pointer was; a zoom is about this point. In screen points as the wheel service splits the wheel; a
    /// component's `take_wheel` hands it in the component's layer space.
    pub at: Option<Pos2>,
    /// The vertical scroll came from over a zoom zone (`Sense::WHEEL_ZOOMS_VIEW`): the taker zooms its view by it,
    /// about `at`, instead of scrolling.
    pub zooms: bool,
    /// Zoom factor of a two-finger touch; 1 for none. `zoom` is the trackpad pinch only.
    pub touch_zoom: f32,
    /// Rotation in radians, clockwise on screen (egui's `Event::Rotate`).
    pub rotation: f32,
    /// Rotation of a two-finger touch, in radians.
    pub touch_rotation: f32,
    /// Two fingers' movement this pass, in points.
    pub touch_pan: Vec2,
    /// The running gesture's scroll gathered so far, this share included, in its owner's points.
    pub gathered: Vec2,
    /// Whether the owner marked the running gesture moved (`Pointer::mark_wheel_moved`).
    pub moved: bool,
}

/// The eased motion of a canvas's line wheel: notches gather into owed motion that later passes pay out.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct WheelEase {
    owed: kurbo::Vec2,
    burst: u32,
    last_notch: f64,
    last_sign: Vec2,
    last_paid: f64,
}

/// Notches closer than this many seconds are a fast spin and grow the burst.
const FAST_NOTCH: f64 = 0.08;
/// Notches this many seconds apart, or more, start a new burst.
const BURST_RESET: f64 = 0.3;
/// The multiplier one fast notch adds.
const BURST_STEP: f32 = 0.4;
/// The burst notches that count toward the multiplier.
const BURST_TICKS: u32 = 13;
/// Notches kept past `BURST_TICKS`, so a few slower notches in a spin do not drop the multiplier at once.
const BURST_HOLD: u32 = 4;
/// The largest multiplier.
const BURST_CAP: f32 = 6.0;
/// The most motion owed on an axis, in points: a hostile run of notches cannot grow it without bound.
const MAX_OWED: f32 = 16384.0;
/// The longest pass the ease pays for, in seconds: after a stall it does not jump the whole way at once.
const LONGEST_PASS: f64 = 0.1;
/// The seconds a pass is taken to last when the ease starts from rest, so the first notch moves in its own pass.
const FIRST_PASS: f64 = 1.0 / 60.0;
/// Owed motion this small, in points, is paid whole.
const SETTLE: f32 = 0.01;

/// The sign of `v`: 0 for zero, where `f32::signum` gives 1.
fn sign(v: f32) -> f32 {
    if v > 0.0 {
        1.0
    } else if v < 0.0 {
        -1.0
    } else {
        0.0
    }
}

impl WheelEase {
    /// Adds the line pan `pan` of this pass, at time `now`, to the owed motion; a burst of notches under 0.08 s apart
    /// accelerates it, a reversal or 0.3 s idle resets the burst.
    pub fn feed(&mut self, pan: Vec2, now: f64) {
        if !now.is_finite() || !pan.is_finite() || pan == Vec2::ZERO {
            return;
        }
        let signs = Vec2::new(sign(pan.x), sign(pan.y));
        let idle = now - self.last_notch;
        let reversed = signs.x * self.last_sign.x < 0.0 || signs.y * self.last_sign.y < 0.0;
        if self.last_sign == Vec2::ZERO || !(0.0..BURST_RESET).contains(&idle) || reversed {
            self.burst = 0;
        } else if idle < FAST_NOTCH {
            self.burst = (self.burst + 1).min(BURST_TICKS + BURST_HOLD);
        } else {
            // A slower notch in the same direction sheds one, rather than extending the burst.
            self.burst = self.burst.saturating_sub(1);
        }
        self.last_notch = now;
        self.last_sign = signs;
        let boost = (1.0 + BURST_STEP * self.burst.min(BURST_TICKS) as f32).min(BURST_CAP);
        if !self.owes() {
            self.last_paid = now - FIRST_PASS;
        }
        self.owed = kurbo::Vec2::new(owe(self.owed.x, f64::from(pan.x * boost)), owe(self.owed.y, f64::from(pan.y * boost)));
    }

    /// Drops the owed motion on the axes marked.
    pub fn cancel_axis(&mut self, x: bool, y: bool) {
        if x {
            self.owed.x = 0.0;
        }
        if y {
            self.owed.y = 0.0;
        }
    }

    /// The step this pass pays at time `now`: `1 − 0.1^(dt/0.1)` of the owed motion, with `dt` since the last paid pass.
    pub fn step(&mut self, now: f64) -> Vec2 {
        if !now.is_finite() || now < self.last_paid {
            // A clock that runs backwards or breaks cannot tell how much to pay: the tail goes.
            self.owed = kurbo::Vec2::ZERO;
            self.last_paid = if now.is_finite() { now } else { self.last_paid };
            return Vec2::ZERO;
        }
        let dt = (now - self.last_paid).min(LONGEST_PASS);
        self.last_paid = now;
        if !self.owes() || dt <= 0.0 {
            return Vec2::ZERO;
        }
        let fraction = 1.0 - 0.1_f64.powf(dt / 0.1);
        let paid = Vec2::new(pay(self.owed.x, fraction), pay(self.owed.y, fraction));
        self.owed -= kurbo::Vec2::new(f64::from(paid.x), f64::from(paid.y));
        // A tail below normal f32 precision cannot move a view and must not keep scheduling frames.
        if self.owed.x.abs() < f64::from(f32::MIN_POSITIVE) {
            self.owed.x = 0.0;
        }
        if self.owed.y.abs() < f64::from(f32::MIN_POSITIVE) {
            self.owed.y = 0.0;
        }
        paid
    }

    /// Whether motion is still owed.
    pub fn owes(&self) -> bool {
        self.owed != kurbo::Vec2::ZERO
    }
}

/// `owed` with `more` added on one axis: motion the other way drops what was owed, and the sum stays within
/// `MAX_OWED`.
fn owe(owed: f64, more: f64) -> f64 {
    let owed = if more * owed < 0.0 { 0.0 } else { owed };
    (owed + more).clamp(-f64::from(MAX_OWED), f64::from(MAX_OWED))
}

/// The part of `owed` one pass pays, `fraction` of it; a remainder under `SETTLE` is paid whole.
fn pay(owed: f64, fraction: f64) -> f32 {
    if owed.abs() <= f64::from(SETTLE) { owed as f32 } else { (owed * fraction) as f32 }
}

impl Wheel {
    /// A scroll of `scroll` points with no zoom.
    pub const fn new(scroll: Vec2, mods: Mods, unit: WheelUnit, phase: WheelPhase) -> Self {
        Wheel {
            scroll,
            lines: Vec2::ZERO,
            zoom: 1.0,
            zoom_lines: Vec2::ZERO,
            zoom_gathered: Vec2::ZERO,
            mods,
            unit,
            phase,
            source: WheelSource::Unknown,
            at: None,
            zooms: false,
            touch_zoom: 1.0,
            rotation: 0.0,
            touch_rotation: 0.0,
            touch_pan: Vec2::ZERO,
            gathered: Vec2::ZERO,
            moved: false,
        }
    }

    /// The same gesture with zoom factor `zoom`.
    pub const fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// The same gesture with `lines` wheel lines of motion that zoom (`Wheel::zoom_lines`).
    #[must_use]
    pub const fn zoom_lines(mut self, lines: Vec2) -> Self {
        self.zoom_lines = lines;
        self
    }

    /// The same gesture with the two-finger touch zoom factor `zoom`.
    pub const fn touch_zoom(mut self, zoom: f32) -> Self {
        self.touch_zoom = zoom;
        self
    }

    /// The same gesture with rotation `radians`, clockwise on screen.
    pub const fn rotation(mut self, radians: f32) -> Self {
        self.rotation = radians;
        self
    }

    /// The same gesture with the two-finger touch rotation `radians`.
    pub const fn touch_rotation(mut self, radians: f32) -> Self {
        self.touch_rotation = radians;
        self
    }

    /// The same gesture with the two-finger touch movement `pan`, in points.
    #[must_use]
    pub const fn touch_pan(mut self, pan: Vec2) -> Self {
        self.touch_pan = pan;
        self
    }

    /// The same gesture from `source`.
    pub const fn source(mut self, source: WheelSource) -> Self {
        self.source = source;
        self
    }

    /// The same gesture with the pointer at `at`.
    pub const fn at(mut self, at: Pos2) -> Self {
        self.at = Some(at);
        self
    }
}

/// The seconds between wheel events that still belong to one gesture.
pub const GESTURE_GAP: f64 = 0.3;

/// A wheel gesture: events closer than [`GESTURE_GAP`] seconds, or inside one device phase. `owner` is `None` while
/// no taker took it. [`split`] keeps it across events; its caller calls [`WheelGesture::end`] before each split.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelGesture<O> {
    /// The taker the gesture started on.
    pub owner: Option<O>,
    /// The time of the gesture's last event, in seconds; −∞ while no gesture is open.
    pub last: f64,
    /// The device ended the gesture (`WheelPhase::End` or `Cancel`), and its owner had that end as the device sent it.
    device_ended: bool,
}

impl<O> Default for WheelGesture<O> {
    fn default() -> Self {
        WheelGesture { owner: None, last: f64::NEG_INFINITY, device_ended: false }
    }
}

impl<O> WheelGesture<O> {
    /// Whether an event at `now` continues the gesture: one is open, the device has not ended it and the gap since
    /// its last event is shorter than [`GESTURE_GAP`].
    pub fn runs(&self, now: f64) -> bool {
        self.last.is_finite() && !self.device_ended && now - self.last < GESTURE_GAP
    }
}

impl<O: Copy> WheelGesture<O> {
    /// Closes the gesture once `now - last` reaches [`GESTURE_GAP`], or at once after a device end, and returns the
    /// owner it had. A gesture the device ended returns `None`: its owner already had the device's end, and a second
    /// end would close its run twice. `now = f64::INFINITY` closes any open gesture (a press).
    pub fn end(&mut self, now: f64) -> Option<O> {
        if !self.last.is_finite() || !(self.device_ended || now - self.last >= GESTURE_GAP) {
            return None;
        }
        let owner = if self.device_ended { None } else { self.owner };
        *self = WheelGesture::default();
        owner
    }
}

/// A component that takes wheel input, as registered last pass.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct WheelTaker<O> {
    /// The component.
    pub owner: O,
    /// Its view rect.
    pub rect: Rect,
    /// It takes horizontal scroll.
    pub horizontal: bool,
    /// It takes vertical scroll.
    pub vertical: bool,
    /// It takes zoom.
    pub zoom: bool,
    /// It takes any axis (scroll or zoom) only while focused or armed: a value control, unless its scope frees it,
    /// or any taker inside a gated scope.
    pub gated: bool,
    /// A scroll region (a scrolling body or collection): Shift turns the vertical wheel sideways over it.
    pub scrolls: bool,
    /// The distance of one wheel line.
    pub notch: WheelNotch,
    /// How far a scroll region can still move each way; `None` for a taker without an end (a value control, a canvas).
    pub room: Option<WheelRoom>,
    /// A zoom zone: a vertical wheel over it goes to the innermost enclosing open taker of the zoom share, with
    /// `Wheel::zooms` set. A zone that takes no axis itself never takes a share.
    pub zooms_view: bool,
    /// A value field: while it holds key focus it takes the field's share of the wheel wherever the pointer is
    /// (`split`).
    pub field: bool,
    /// A value control (a slider, a field, a closed select), in any wheel scope: when a gesture it owns ends by the
    /// gap or a press, the wheel service hands it one zero-scroll share with `WheelPhase::End`.
    pub value: bool,
}

/// The scroll left in a region, in points of wheel scroll: `positive` is what a wheel with positive `scroll` can
/// still move, `negative` the same for negative `scroll`.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub struct WheelRoom {
    /// Room for a positive scroll on each axis.
    pub positive: Vec2,
    /// Room for a negative scroll on each axis.
    pub negative: Vec2,
}

impl WheelRoom {
    /// A room of `positive` and `negative` points.
    pub const fn new(positive: Vec2, negative: Vec2) -> Self {
        WheelRoom { positive, negative }
    }
}

/// The distance one wheel line scrolls a taker.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum WheelNotch {
    /// This many points.
    Points(f32),
    /// This share of the taker's view along the axis.
    ViewShare(f32),
}

impl WheelNotch {
    /// The distance of a taker that states none.
    pub const DEFAULT: WheelNotch = WheelNotch::Points(40.0);

    /// The notch in points, for a taker whose view is `extent` points along the axis.
    pub fn points(self, extent: f32) -> f32 {
        match self {
            WheelNotch::Points(points) => points,
            WheelNotch::ViewShare(share) => extent * share,
        }
    }
}

impl<O> WheelTaker<O> {
    /// A taker over `rect` that takes no axis yet.
    pub const fn new(owner: O, rect: Rect) -> Self {
        WheelTaker {
            owner,
            rect,
            horizontal: false,
            vertical: false,
            zoom: false,
            gated: false,
            scrolls: false,
            notch: WheelNotch::DEFAULT,
            room: None,
            zooms_view: false,
            field: false,
            value: false,
        }
    }

    /// The taker a shape with `sense` registers, `None` when `sense` has no wheel bit: `WHEEL` takes both scroll
    /// axes and zoom; `WHEEL_X`, `WHEEL_Y` and `WHEEL_ZOOM` take one share each and combine; `WHEEL_ZOOMS_VIEW`
    /// makes a zoom zone.
    pub fn for_sense(owner: O, rect: Rect, sense: Sense) -> Option<Self> {
        let all = sense.contains(Sense::WHEEL);
        let taker = WheelTaker::new(owner, rect)
            .horizontal(all || sense.contains(Sense::WHEEL_X))
            .vertical(all || sense.contains(Sense::WHEEL_Y))
            .zoom(all || sense.contains(Sense::WHEEL_ZOOM))
            .zooms_view(sense.contains(Sense::WHEEL_ZOOMS_VIEW));
        (taker.horizontal || taker.vertical || taker.zoom || taker.zooms_view).then_some(taker)
    }

    /// Whether it is a zoom zone (`WheelTaker::zooms_view`).
    #[must_use]
    pub const fn zooms_view(mut self, on: bool) -> Self {
        self.zooms_view = on;
        self
    }

    /// Whether it is a value field (`WheelTaker::field`).
    #[must_use]
    pub const fn field(mut self, on: bool) -> Self {
        self.field = on;
        self
    }

    /// Whether it is a value control (`WheelTaker::value`).
    #[must_use]
    pub const fn value(mut self, on: bool) -> Self {
        self.value = on;
        self
    }

    /// Whether it takes horizontal scroll.
    pub const fn horizontal(mut self, on: bool) -> Self {
        self.horizontal = on;
        self
    }

    /// Whether it takes vertical scroll.
    pub const fn vertical(mut self, on: bool) -> Self {
        self.vertical = on;
        self
    }

    /// Whether it takes zoom.
    pub const fn zoom(mut self, on: bool) -> Self {
        self.zoom = on;
        self
    }

    /// Whether every axis needs focus or arming.
    pub const fn gated(mut self, on: bool) -> Self {
        self.gated = on;
        self
    }

    /// The distance of one wheel line.
    pub const fn notch(mut self, notch: WheelNotch) -> Self {
        self.notch = notch;
        self
    }

    /// The taker is a scroll region (`WheelTaker::scrolls`).
    #[must_use]
    pub const fn scrolls(mut self, on: bool) -> Self {
        self.scrolls = on;
        self
    }

    /// The taker's scroll room: it passes a wheel on an axis it has no room left for to the next enclosing taker.
    #[must_use]
    pub const fn room(mut self, room: WheelRoom) -> Self {
        self.room = Some(room);
        self
    }

    /// Whether the taker can move by `delta` on the axis (`vertical` or horizontal); a taker without a room always can.
    fn has_room(&self, vertical: bool, delta: f32) -> bool {
        let Some(room) = self.room else { return true };
        let side = if delta > 0.0 { room.positive } else { room.negative };
        (if vertical { side.y } else { side.x }) > 0.0
    }

    /// `scroll`, given in `unit`s of the device, as points of this taker's view: a line is one notch, a page is
    /// the view.
    pub fn points(&self, scroll: Vec2, unit: WheelUnit) -> Vec2 {
        let size = self.rect.size();
        match unit {
            WheelUnit::Point => scroll,
            WheelUnit::Line => Vec2::new(scroll.x * self.notch.points(size.x), scroll.y * self.notch.points(size.y)),
            WheelUnit::Page => scroll * size,
        }
    }
}

/// Who holds key focus, as the split reads it.
#[derive(Clone, Copy, Debug, PartialEq, Default)]
#[non_exhaustive]
pub struct WheelFocus<O> {
    /// The taker that holds key focus.
    pub taker: Option<O>,
    /// Key focus is in a text field the app drew itself, which reads its own wheel: its share goes to no taker.
    pub app_field: bool,
    /// The pointer is over a layer other than the focused field's that takes the wheel by position: a popup opened
    /// above the field's layer, or the canvas a modal dialog around the field keeps open below it. The field takes no
    /// share, and the wheel goes by position.
    pub by_position: bool,
}

impl<O> WheelFocus<O> {
    /// Focus on `taker`, or on nothing that takes the wheel.
    pub const fn new(taker: Option<O>) -> Self {
        WheelFocus { taker, app_field: false, by_position: false }
    }

    /// The same focus, in a text field the app drew itself when `on`.
    #[must_use]
    pub const fn app_field(mut self, on: bool) -> Self {
        self.app_field = on;
        self
    }

    /// The same focus, with the pointer over a layer that takes the wheel by position when `on`
    /// ([`by_position`](Self::by_position)).
    #[must_use]
    pub const fn by_position(mut self, on: bool) -> Self {
        self.by_position = on;
        self
    }
}

/// Whether `wheel` is a field's: a vertical scroll, or Shift's (which some devices send sideways), without Alt or
/// Cmd/Ctrl, which stay the view's.
fn for_field(wheel: &Wheel) -> bool {
    let mods = wheel.mods;
    !(mods.alt || mods.cmd || mods.ctrl) && (wheel.scroll.y != 0.0 || (mods.shift && wheel.scroll.x != 0.0))
}

/// Whether `wheel` carries a pinch, a touch zoom, a rotation or a two-finger pan: the zoom share's values.
fn pinches(wheel: &Wheel) -> bool {
    wheel.zoom != 1.0
        || wheel.zoom_lines != Vec2::ZERO
        || wheel.touch_zoom != 1.0
        || wheel.rotation != 0.0
        || wheel.touch_rotation != 0.0
        || wheel.touch_pan != Vec2::ZERO
}

/// `wheel` without the zoom share's values: a scroll share carries no pinch or rotation, so a field or a scroll region
/// never reads one.
const fn scroll_only(wheel: Wheel) -> Wheel {
    Wheel {
        zoom: 1.0,
        zoom_lines: Vec2::ZERO,
        zoom_gathered: Vec2::ZERO,
        touch_zoom: 1.0,
        rotation: 0.0,
        touch_rotation: 0.0,
        touch_pan: Vec2::ZERO,
        zooms: false,
        ..wheel
    }
}

/// The split rule. A focused field (a `WheelTaker::field` taker that holds `focus`, or an app's own text field) takes
/// the field's share of the scroll wherever the pointer is, except over another layer that goes by position (a popup
/// opened above it, the canvas its modal dialog keeps open: `WheelFocus::by_position`): a vertical or Shift wheel without Alt or Cmd/Ctrl; an app's field reads it itself, so
/// no taker gets it. A running `gesture` (an event at `now` less than [`GESTURE_GAP`] after
/// its last one, inside its device phase) keeps the taker it started on: each scroll axis that taker takes goes to it
/// wherever the pointer is, and the rest by position to a taker that is no value control (`WheelTaker::value`), so a
/// control the pointer passes over does not take the scroll over. A value control that is not the focused field keeps
/// its gesture only while the pointer is over it: when the pointer leaves, it gets a zero-scroll `WheelPhase::End`
/// share and the gesture goes on by position without it. A new gesture splits by position: each axis of `wheel` goes
/// to the innermost taker under `at` that takes it; a share no taker takes, or that a scroll region at its end in that
/// direction cannot move, passes to the next enclosing one. A gated taker takes no axis unless it is `focused` or
/// `armed`, and a trackpad (`WheelSource::fingers`) opens a value control only then. Shift turns a
/// vertical wheel sideways when its vertical owner is a scroll region. When the innermost open taker under `at` for the
/// vertical share is a zoom zone, the share goes to the innermost open zoom taker enclosing it, with `Wheel::zooms`
/// set; without one the zone is passed over. The pinch, touch zoom and rotations travel only in the zoom share, which
/// goes by position. Shares of one owner merge.
///
/// The gesture takes the first owner of its shares and the time `now`; a device `End` or `Cancel` ends it, and
/// reaches its owner even when it moves nothing. The caller ends a gesture ([`WheelGesture::end`]) before each split.
pub fn split<O: Copy + PartialEq>(
    takers: &[WheelTaker<O>],
    at: Pos2,
    wheel: Wheel,
    focus: WheelFocus<O>,
    armed: Option<O>,
    gesture: &mut WheelGesture<O>,
    now: f64,
) -> Vec<(O, Wheel)> {
    let running = gesture.runs(now);
    if !running {
        *gesture = WheelGesture::default();
    }
    let field = focus.taker.filter(|focused| !focus.by_position && takers.iter().any(|taker| taker.field && taker.owner == *focused));
    let app_field = focus.app_field && !focus.by_position;
    // A value control keeps its gesture only while the pointer is over it: once the pointer leaves, the gesture goes
    // on by position as a running one (value controls excluded) and the control hears its end now, so its edit run
    // commits and coming back in the same gesture does not step it again.
    let left = gesture
        .owner
        .filter(|owner| running && field != Some(*owner))
        .filter(|owner| takers.iter().any(|taker| taker.owner == *owner && taker.value && !taker.rect.contains(at)));
    if left.is_some() {
        gesture.owner = None;
    }
    let latch = running.then_some(gesture.owner);
    let mut shares = if (field.is_some() || app_field) && wheel.scroll != Vec2::ZERO && for_field(&wheel) {
        let mut shares =
            if pinches(&wheel) { split_at(takers, at, Wheel { scroll: Vec2::ZERO, ..wheel }, focus.taker, armed, latch) } else { Vec::new() };
        if let Some(owner) = field {
            let scroll = scroll_only(wheel);
            match shares.iter_mut().find(|(o, _)| *o == owner) {
                Some((_, share)) => share.scroll += scroll.scroll,
                None => shares.push((owner, scroll)),
            }
        }
        shares
    } else {
        split_at(takers, at, wheel, focus.taker, armed, latch)
    };
    gesture.last = now;
    if gesture.owner.is_none() {
        gesture.owner = shares.first().map(|(owner, _)| *owner);
    }
    if let Some(owner) = left {
        shares.insert(0, (owner, Wheel { scroll: Vec2::ZERO, phase: WheelPhase::End, ..scroll_only(wheel) }));
    }
    if matches!(wheel.phase, WheelPhase::End | WheelPhase::Cancel) {
        gesture.device_ended = true;
        if let Some(owner) = gesture.owner
            && takers.iter().any(|taker| taker.owner == owner)
            && !shares.iter().any(|(o, _)| *o == owner)
        {
            shares.push((owner, Wheel { scroll: Vec2::ZERO, ..scroll_only(wheel) }));
        }
    }
    shares
}

/// The split of the scroll and zoom shares. `latch` is `None` for a new gesture, which splits by position, and the
/// running gesture's owner otherwise.
fn split_at<O: Copy + PartialEq>(
    takers: &[WheelTaker<O>],
    at: Pos2,
    wheel: Wheel,
    focused: Option<O>,
    armed: Option<O>,
    latch: Option<Option<O>>,
) -> Vec<(O, Wheel)> {
    // Takers register outer first, so among equal areas the later one is the deeper (or the upper) and goes first.
    let mut under: Vec<&WheelTaker<O>> = takers.iter().rev().filter(|taker| taker.rect.contains(at)).collect();
    under.sort_by(|a, b| a.rect.area().total_cmp(&b.rect.area()));
    // A trackpad steps a value control only while it is focused or armed: over anything else it scrolls the panel.
    let fingers = wheel.source.fingers();
    let open = |taker: &WheelTaker<O>| (!taker.gated && !(fingers && taker.value)) || armed == Some(taker.owner) || focused == Some(taker.owner);
    // The running gesture's owner, while it is registered and open; it takes its axes wherever the pointer is. A
    // focused field holds the wheel by the field rule alone, so a wheel with Alt or Cmd/Ctrl still reaches the view.
    let held: Option<&WheelTaker<O>> = latch
        .flatten()
        .and_then(|owner| takers.iter().find(|taker| taker.owner == owner && open(taker) && !(taker.field && focused == Some(taker.owner))));
    // Who takes a scroll axis by position: any open taker for a new gesture; during a running one no value control,
    // so a control the pointer passes over does not take the scroll over.
    let free = |taker: &WheelTaker<O>| open(taker) && (latch.is_none() || !taker.value);
    let zone = (wheel.scroll.y != 0.0).then(|| under.iter().position(|t| (t.vertical || t.zooms_view) && open(t))).flatten();
    let zone_view = zone.filter(|&at| !under[at].vertical).and_then(|at| under[at + 1..].iter().find(|t| t.zoom && open(t))).map(|t| t.owner);
    // A running gesture zooms a view by the zone only when it is that view's gesture.
    let zone_view = if latch.is_none() { zone_view } else { zone_view.filter(|view| held.is_some_and(|t| t.owner == *view && !t.vertical)) };
    let held_vertical = held.filter(|t| t.vertical);
    let vertical_taker = held_vertical.or_else(|| under.iter().copied().find(|t| t.vertical && free(t)));
    // Shift turns the vertical wheel sideways over a scroll region (Photoshop); a value control that takes the
    // vertical wheel keeps Shift as its modifier, and a device that already sends a sideways axis keeps it.
    let wheel = if zone_view.is_none() && wheel.mods.shift && wheel.scroll.x == 0.0 && vertical_taker.is_some_and(|t| t.scrolls) {
        Wheel { scroll: Vec2::new(wheel.scroll.y, 0.0), ..wheel }
    } else {
        wheel
    };
    let vertical = (wheel.scroll.y != 0.0)
        .then(|| {
            zone_view
                .or(held_vertical.map(|t| t.owner))
                .or_else(|| under.iter().find(|t| t.vertical && free(t) && t.has_room(true, wheel.scroll.y)).map(|t| t.owner))
        })
        .flatten();
    let horizontal = (wheel.scroll.x != 0.0)
        .then(|| {
            held.filter(|t| t.horizontal)
                .map(|t| t.owner)
                .or_else(|| under.iter().find(|t| t.horizontal && free(t) && t.has_room(false, wheel.scroll.x)).map(|t| t.owner))
        })
        .flatten();
    let zoom = pinches(&wheel).then(|| under.iter().find(|t| t.zoom && open(t))).flatten().map(|t| t.owner);
    let scroll = scroll_only(wheel);
    let parts = [
        (vertical, Wheel { scroll: Vec2::new(0.0, wheel.scroll.y), zooms: zone_view.is_some(), ..scroll }),
        (horizontal, Wheel { scroll: Vec2::new(wheel.scroll.x, 0.0), ..scroll }),
        (zoom, Wheel { scroll: Vec2::ZERO, zooms: false, ..wheel }),
    ];
    let mut shares: Vec<(O, Wheel)> = Vec::new();
    for (owner, part) in parts {
        let Some(owner) = owner else { continue };
        match shares.iter_mut().find(|(o, _)| *o == owner) {
            Some((_, share)) => {
                share.scroll += part.scroll;
                share.zoom *= part.zoom;
                share.zoom_lines += part.zoom_lines;
                share.touch_zoom *= part.touch_zoom;
                share.rotation += part.rotation;
                share.touch_rotation += part.touch_rotation;
                share.touch_pan += part.touch_pan;
                share.zooms |= part.zooms;
            }
            None => shares.push((owner, part)),
        }
    }
    shares
}

#[cfg(test)]
mod tests {
    use super::*;
    use emath::pos2;

    fn wheel(x: f32, y: f32) -> Wheel {
        Wheel::new(Vec2::new(x, y), Mods::NONE, WheelUnit::Point, WheelPhase::Move)
    }

    fn rect(size: f32) -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), Vec2::splat(size))
    }

    #[test]
    fn a_scroll_region_at_its_end_hands_the_wheel_to_the_enclosing_one() {
        let outer = WheelTaker::new(1u8, rect(200.0)).vertical(true).scrolls(true).room(WheelRoom::new(Vec2::splat(50.0), Vec2::splat(50.0)));
        let inner = WheelTaker::new(2u8, rect(100.0)).vertical(true).scrolls(true).room(WheelRoom::new(Vec2::ZERO, Vec2::splat(30.0)));
        let at = pos2(10.0, 10.0);
        let owner =
            |y| split(&[outer, inner], at, wheel(0.0, y), WheelFocus::new(None), None, &mut WheelGesture::default(), 0.0).first().map(|(o, _)| *o);
        assert_eq!(owner(7.0), Some(1), "no room toward positive: the outer takes it");
        assert_eq!(owner(-7.0), Some(2), "room toward negative: the inner keeps it");
    }

    #[test]
    fn a_focused_field_takes_the_plain_wheel_wherever_the_pointer_is() {
        let canvas = WheelTaker::new(1u8, rect(300.0)).horizontal(true).vertical(true).zoom(true);
        let field = WheelTaker::new(2u8, Rect::from_min_size(pos2(400.0, 0.0), Vec2::splat(20.0))).vertical(true).gated(true).field(true);
        let at = pos2(10.0, 10.0);
        let owners = |wheel: Wheel, focus: WheelFocus<u8>| {
            split(&[canvas, field], at, wheel, focus, None, &mut WheelGesture::default(), 0.0).iter().map(|(o, _)| *o).collect::<Vec<_>>()
        };
        let focused = WheelFocus::new(Some(2));
        assert_eq!(owners(wheel(0.0, 5.0), WheelFocus::new(None)), [1], "no focus: the canvas under the pointer");
        assert_eq!(owners(wheel(0.0, 5.0), focused), [2], "the focused field, away from the pointer");
        let alt = Wheel { mods: Mods::ALT, ..wheel(0.0, 5.0) };
        let cmd = Wheel { mods: Mods::CMD, ..wheel(0.0, 5.0) };
        assert_eq!(owners(alt, focused), [1], "Alt stays the view's");
        assert_eq!(owners(cmd, focused), [1], "Cmd/Ctrl stays the view's");
        assert_eq!(owners(wheel(5.0, 0.0), focused), [1], "a sideways swipe stays the view's");
        assert_eq!(owners(wheel(0.0, 5.0), WheelFocus::new(None).app_field(true)), [] as [u8; 0], "an app's field reads its own");
        assert_eq!(owners(alt, WheelFocus::new(None).app_field(true)), [1]);
        assert_eq!(owners(wheel(0.0, 5.0), focused.by_position(true)), [1], "over a popup above the field or a canvas below its modal: by position");
        assert_eq!(owners(wheel(0.0, 5.0), WheelFocus::new(None).app_field(true).by_position(true)), [1]);
    }

    #[test]
    fn a_vertical_wheel_over_a_zoom_zone_zooms_the_enclosing_view() {
        let region = WheelTaker::new(1u8, rect(300.0)).vertical(true).scrolls(true);
        let view = WheelTaker::new(2u8, rect(200.0)).horizontal(true).zoom(true);
        let zone = WheelTaker::new(3u8, rect(100.0)).zooms_view(true);
        let at = pos2(10.0, 10.0);
        let shares = split(&[region, view, zone], at, wheel(0.0, 5.0), WheelFocus::new(None), None, &mut WheelGesture::default(), 0.0);
        assert_eq!(shares.len(), 1);
        assert_eq!((shares[0].0, shares[0].1.scroll.y, shares[0].1.zooms), (2, 5.0, true), "the view zooms by the vertical share");
        let shares = split(&[region, zone], at, wheel(0.0, 5.0), WheelFocus::new(None), None, &mut WheelGesture::default(), 0.0);
        assert_eq!(shares.iter().map(|(o, w)| (*o, w.zooms)).collect::<Vec<_>>(), [(1, false)], "no zoom taker: the zone is passed over");
        let shares = split(&[region, view, zone], at, wheel(4.0, 0.0), WheelFocus::new(None), None, &mut WheelGesture::default(), 0.0);
        assert_eq!(shares.iter().map(|(o, w)| (*o, w.zooms)).collect::<Vec<_>>(), [(2, false)], "the horizontal share passes the zone");
    }

    #[test]
    fn fast_notches_accelerate_to_a_cap_and_reset_on_reversal_or_idle_but_a_lone_notch_does_not() {
        const NOTCH: f64 = 40.0;
        // What one notch of `y` points adds to the owed motion.
        let notch = |ease: &mut WheelEase, y: f64, now: f64| {
            let before = ease.owed.y;
            ease.feed(Vec2::new(0.0, y as f32), now);
            ease.owed.y - before
        };
        let mut ease = WheelEase::default();
        assert_eq!(notch(&mut ease, NOTCH, 1.0), NOTCH, "a lone notch is one notch");
        assert_eq!(notch(&mut ease, NOTCH, 1.5), NOTCH, "a slow notch is one notch");

        let mut gains = Vec::new();
        for i in 1..=20 {
            gains.push(notch(&mut ease, NOTCH, 1.5 + f64::from(i) * 0.02));
        }
        // The owed sum rounds in its last bits, so per-notch gains compare within a hundredth of a point.
        let near = |a: f64, b: f64| (a - b).abs() < 0.01;
        assert!(gains.windows(2).all(|pair| pair[1] > pair[0] - 0.01), "a fast run never pays less per notch: {gains:?}");
        let fastest = gains.iter().copied().fold(0.0, f64::max);
        assert!(fastest > 2.0 * NOTCH, "a fast run accelerates: {gains:?}");
        assert!(fastest < 6.0 * NOTCH + 0.01, "the acceleration is capped: {gains:?}");
        assert!(gains.last().is_some_and(|&last| near(last, 6.0 * NOTCH)), "a long run reaches the cap and stays there: {gains:?}");

        notch(&mut ease, -NOTCH, 1.5 + 21.0 * 0.02);
        assert_eq!(ease.owed.y, -NOTCH, "a reversal drops what was owed and starts again at one notch");

        for i in 1..=10 {
            notch(&mut ease, -NOTCH, 2.0 + f64::from(i) * 0.02);
        }
        assert!(notch(&mut ease, -NOTCH, 2.0 + 11.0 * 0.02) < -NOTCH - 1.0, "precondition: the run is fast again");
        assert!(near(notch(&mut ease, -NOTCH, 5.0), -NOTCH), "an idle gap starts again at one notch");
    }
}
