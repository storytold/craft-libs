//! The motion service (13): tweens over one clock, the motion table, and one repaint request per viewport.

use crate::runtime::egui::{memory as store, pass, repaint};
use crate::runtime::env::MotionPolicy;
use crate::runtime::frame::{self, PerViewport};
use crate::runtime::memory::{self, MemoryStage};
use craft_behavior::{MotionKind, Playback};
use craft_theme::Motion;
use egui::{Context, Id, Pos2, Rect, Vec2};

/// Which token of `MotionTokens` gives a spec its duration and curve.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Token {
    Hover,
    Open,
    Move,
    Caret,
    Outline,
}

/// How a value approaches its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Curve {
    /// From where it was to the new target over the token's duration.
    Tween,
    /// Toward a target that moves every frame, by exponential smoothing with the token's duration as its settle time.
    Follow,
}

/// A motion constant: a component picks one from `spec`, never writes seconds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MotionSpec {
    token: Token,
    kind: MotionKind,
    curve: Curve,
}

impl MotionSpec {
    const fn new(token: Token, kind: MotionKind, curve: Curve) -> Self {
        MotionSpec { token, kind, curve }
    }

    /// The kind that picks this spec's row of the motion table.
    pub const fn kind(self) -> MotionKind {
        self.kind
    }

    fn motion(self, policy: &MotionPolicy) -> Motion {
        let tokens = &policy.tokens;
        match self.token {
            Token::Hover => tokens.hover,
            Token::Open => tokens.open,
            Token::Move => tokens.r#move,
            Token::Caret => tokens.caret,
            Token::Outline => tokens.outline,
        }
    }
}

/// The motion constants.
pub mod spec {
    use super::{Curve, MotionKind, MotionSpec, Token};

    /// Hover and press feedback.
    pub const HOVER: MotionSpec = MotionSpec::new(Token::Hover, MotionKind::Fade, Curve::Tween);
    /// A check, a toggle or a selection turning on or off.
    pub const CHECK: MotionSpec = MotionSpec::new(Token::Hover, MotionKind::Fade, Curve::Tween);
    /// A popup's opacity.
    pub const OPEN_FADE: MotionSpec = MotionSpec::new(Token::Open, MotionKind::Fade, Curve::Tween);
    /// A popup's travel while it opens.
    pub const OPEN_RISE: MotionSpec = MotionSpec::new(Token::Open, MotionKind::Travel, Curve::Tween);
    /// An element moving within the screen.
    pub const MOVE: MotionSpec = MotionSpec::new(Token::Move, MotionKind::Travel, Curve::Tween);
    /// A text caret gliding to its new place.
    pub const CARET: MotionSpec = MotionSpec::new(Token::Caret, MotionKind::Travel, Curve::Tween);
    /// A selection outline gliding to its new cells.
    pub const OUTLINE: MotionSpec = MotionSpec::new(Token::Outline, MotionKind::Travel, Curve::Tween);
    /// A target that moves every frame: zoom ease, remote cursors, autoscroll.
    pub const FOLLOW: MotionSpec = MotionSpec::new(Token::Move, MotionKind::Travel, Curve::Follow);
    /// A spinner, marching ants or a pulse.
    pub const LOOP: MotionSpec = MotionSpec::new(Token::Move, MotionKind::Loop, Curve::Tween);
    /// A text caret's blink.
    pub const BLINK: MotionSpec = MotionSpec::new(Token::Caret, MotionKind::Blink, Curve::Tween);
}

/// How long a blinking caret shows or hides its stroke, in seconds: the platform's usual caret rate.
const BLINK_HALF_PERIOD: f64 = 0.53;

/// A value a tween can run over.
pub trait Interpolate: Copy + PartialEq + Send + Sync + 'static {
    /// The value `t` of the way from `a` to `b`; `t` is 0 to 1.
    fn lerp(a: Self, b: Self, t: f32) -> Self;
    /// How far apart two values are, in the value's own unit.
    fn distance(a: Self, b: Self) -> f32;
}

impl Interpolate for f32 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }

    fn distance(a: Self, b: Self) -> f32 {
        (a - b).abs()
    }
}

impl Interpolate for Vec2 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }

    fn distance(a: Self, b: Self) -> f32 {
        (a - b).length()
    }
}

impl Interpolate for Pos2 {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        a + (b - a) * t
    }

    fn distance(a: Self, b: Self) -> f32 {
        (a - b).length()
    }
}

impl Interpolate for Rect {
    fn lerp(a: Self, b: Self, t: f32) -> Self {
        Rect::from_min_max(<Pos2 as Interpolate>::lerp(a.min, b.min, t), <Pos2 as Interpolate>::lerp(a.max, b.max, t))
    }

    fn distance(a: Self, b: Self) -> f32 {
        <Pos2 as Interpolate>::distance(a.min, b.min).max(<Pos2 as Interpolate>::distance(a.max, b.max))
    }
}

/// A distance under this ends a follow; a thousandth of a point or of a unit fraction is below anything painted.
const FOLLOW_EPSILON: f32 = 0.001;

/// Time constants a follow's settle time spans: 95% of the way after the token's duration.
const FOLLOW_TIME_CONSTANTS: f32 = 3.0;

/// A running animation of one value. Evaluating it at the same time twice gives the same value, so a
/// discarded pass repeats a frame exactly.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Tween<T: Interpolate> {
    from: T,
    to: T,
    /// The start of a tween; the last evaluation of a follow.
    started: f64,
    shown: T,
}

impl<T: Interpolate> Tween<T> {
    /// A tween at rest at `value`.
    pub fn at(value: T) -> Self {
        Tween { from: value, to: value, started: 0.0, shown: value }
    }

    /// The value to paint.
    pub fn shown(&self) -> T {
        self.shown
    }

    /// The value it runs toward.
    pub fn target(&self) -> T {
        self.to
    }

    /// True once the shown value is the target.
    pub fn is_settled(&self) -> bool {
        self.shown == self.to
    }

    /// Runs toward `to` from the value shown now.
    pub fn retarget(&mut self, to: T, now: f64) {
        self.from = self.shown;
        self.to = to;
        self.started = now;
    }

    /// Evaluates at `now` with `motion`'s duration and curve.
    pub(crate) fn step(&mut self, now: f64, motion: Motion, curve: Curve) {
        let seconds = f64::from(motion.millis) / 1000.0;
        match curve {
            Curve::Tween => {
                let progress = if seconds > 0.0 { ((now - self.started) / seconds) as f32 } else { 1.0 };
                self.shown = if progress >= 1.0 { self.to } else { T::lerp(self.from, self.to, motion.easing.at(progress)) };
            }
            Curve::Follow => {
                let dt = (now - self.started).max(0.0);
                self.started = now;
                let rate = if seconds > 0.0 { 1.0 - (-(dt as f32) * FOLLOW_TIME_CONSTANTS / seconds as f32).exp() } else { 1.0 };
                let next = T::lerp(self.shown, self.to, rate);
                self.shown = if T::distance(next, self.to) < FOLLOW_EPSILON { self.to } else { next };
            }
        }
    }
}

/// An open-close presence: `t` is 0 closed and 1 open, and a closing one stays visible until `t` reaches 0.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct Presence {
    /// The animated openness, 0 to 1.
    pub t: f32,
    /// Whether anything of it should be shown.
    pub visible: bool,
    /// Closing but still fading out.
    pub exiting: bool,
}

/// What a viewport's animations asked for in the running pass.
#[derive(Default)]
struct Wanted {
    unsettled: bool,
    wake: Option<f64>,
}

/// The wall clock's reading, for a host that drives a store on real time (`ShellApp::tick`). It is the one place
/// outside a pass that reads the clock.
pub fn now() -> web_time::Instant {
    web_time::Instant::now()
}

/// Marks the running viewport as having an animation that is not at its end value.
fn note_unsettled(ctx: &Context) {
    PerViewport::<Wanted>::with(ctx, pass::current_viewport(ctx), |wanted| wanted.unsettled = true);
}

/// The end hook: one repaint request for the running viewport, now when an animation is unsettled, else at the
/// earliest wake.
pub(crate) fn end_pass(ctx: &Context) {
    let viewport = pass::current_viewport(ctx);
    let wanted = PerViewport::<Wanted>::with(ctx, viewport, std::mem::take);
    if wanted.unsettled {
        repaint::request_of(ctx, viewport);
    } else if let Some(at) = wanted.wake {
        repaint::request_after(ctx, viewport, at - frame::clock(ctx));
    }
}

/// Whether the open tween of the layer `base` still shows it: it opens, shows, or fades out. Reads without driving.
pub(crate) fn layer_fading(ctx: &Context, base: Id) -> bool {
    store::peek_clone::<Tween<f32>>(ctx, memory::sub(base, ("anim", "open")))
        .is_some_and(|tween| !tween.is_settled() || tween.target() > 0.0 || tween.shown() > 0.0)
}

/// The open-close presence of an overlay layer `base` that no component context drives: the overlay host reads it.
/// A layer that is closed and fully faded out answers without opening a stage.
pub(crate) fn layer_presence(ctx: &Context, base: Id, owner: Id, policy: MotionPolicy, open: bool, spec: MotionSpec) -> Presence {
    let hidden = Presence { t: 0.0, visible: false, exiting: false };
    if !open && !layer_fading(ctx, base) {
        return hidden;
    }
    let stage = MemoryStage::open_owned(ctx, memory::sub(base, "presence"), owner);
    let presence = Driver { stage: &stage, base, policy }.presence("open", open, spec);
    stage.write_back();
    presence
}

/// The hover, press and on tweens of one component.
#[derive(Clone)]
struct StateTweens([Tween<f32>; 3]);

/// Everything an `Animate` call needs from the context type that makes it.
pub(crate) struct Driver<'a> {
    pub(crate) stage: &'a MemoryStage,
    pub(crate) base: Id,
    pub(crate) policy: MotionPolicy,
}

/// Whose animation a slot is: a component's own `anim` names, or the pipeline's state mix. The lanes never share
/// a slot, whatever the names.
#[derive(Clone, Copy, Debug)]
enum Lane {
    Author,
    Pipeline,
}

impl Driver<'_> {
    fn slot(&self, lane: Lane, name: &'static str) -> Id {
        match lane {
            Lane::Author => memory::sub(self.base, ("anim", name)),
            Lane::Pipeline => memory::sub(self.base, ("state-anim", name)),
        }
    }

    pub(crate) fn anim<T: Interpolate>(&self, name: &'static str, target: T, spec: MotionSpec) -> T {
        self.run(Lane::Author, name, target, spec)
    }

    /// The pipeline's state values (`hover`, `press`, `on`) in one slot of the pipeline lane: one clock read and one
    /// context round trip for the three tweens.
    pub(crate) fn state_anims(&self, targets: [bool; 3], specs: [MotionSpec; 3]) -> [f32; 3] {
        let ctx = self.stage.ctx();
        let id = self.slot(Lane::Pipeline, "state");
        let sizing = self.stage.is_peek();
        let now = frame::clock(ctx);
        let existing = if sizing { store::peek_clone::<StateTweens>(ctx, id) } else { store::take::<StateTweens>(ctx, id) };
        let mut tweens = match existing {
            Some(tweens) => tweens,
            None => {
                if !sizing {
                    memory::adopt::<StateTweens>(ctx, self.stage.owner(), id);
                }
                StateTweens(targets.map(|on| Tween::at(f32::from(u8::from(on)))))
            }
        };
        let mut shown = [0.0; 3];
        for (at, tween) in tweens.0.iter_mut().enumerate() {
            let target = f32::from(u8::from(targets[at]));
            if self.policy.mode.playback(specs[at].kind) == Playback::Run {
                if tween.target() != target {
                    tween.retarget(target, now);
                }
                tween.step(now, specs[at].motion(&self.policy), specs[at].curve);
            } else {
                *tween = Tween::at(target);
            }
            shown[at] = tween.shown();
        }
        if !sizing {
            if tweens.0.iter().any(|tween| !tween.is_settled()) {
                note_unsettled(ctx);
            }
            store::put(ctx, id, tweens);
        }
        shown
    }

    /// A sizing pass evaluates a copy and stores nothing: it must not retarget a running tween or adopt a slot
    /// for an owner that registers no pass.
    fn run<T: Interpolate>(&self, lane: Lane, name: &'static str, target: T, spec: MotionSpec) -> T {
        let ctx = self.stage.ctx();
        let id = self.slot(lane, name);
        let sizing = self.stage.is_peek();
        let now = frame::clock(ctx);
        let existing = if sizing { store::peek_clone::<Tween<T>>(ctx, id) } else { store::take::<Tween<T>>(ctx, id) };
        let runs = self.policy.mode.playback(spec.kind) == Playback::Run;
        let mut tween = match existing {
            Some(tween) => tween,
            None => {
                if !sizing {
                    memory::adopt::<Tween<T>>(ctx, self.stage.owner(), id);
                }
                Tween::at(target)
            }
        };
        if !runs {
            tween = Tween::at(target);
        } else {
            if tween.target() != target {
                tween.retarget(target, now);
            }
            tween.step(now, spec.motion(&self.policy), spec.curve);
        }
        let shown = tween.shown();
        if !sizing {
            if !tween.is_settled() {
                note_unsettled(ctx);
            }
            store::put(ctx, id, tween);
        }
        shown
    }

    pub(crate) fn anim_bool(&self, name: &'static str, on: bool, spec: MotionSpec) -> f32 {
        self.anim(name, f32::from(u8::from(on)), spec)
    }

    pub(crate) fn presence(&self, name: &'static str, open: bool, spec: MotionSpec) -> Presence {
        let t = self.anim_bool(name, open, spec);
        Presence { t, visible: open || t > 0.0, exiting: !open && t > 0.0 }
    }

    pub(crate) fn jump<T: Interpolate>(&self, name: &'static str, value: T) {
        if self.stage.is_peek() {
            return;
        }
        let ctx = self.stage.ctx();
        let id = self.slot(Lane::Author, name);
        if store::peek_clone::<Tween<T>>(ctx, id).is_none() {
            memory::adopt::<Tween<T>>(ctx, self.stage.owner(), id);
        }
        store::put(ctx, id, Tween::at(value));
    }

    pub(crate) fn phase(&self, period: f64, tick: Option<f64>) -> Option<f32> {
        loop_phase(self.stage.ctx(), self.policy, period, tick)
    }

    /// Whether a caret that last moved or was typed at `restart` shows its stroke now. It shows solid for a
    /// half period after `restart`, then alternates; a mode whose Blink row is solid never hides it.
    pub(crate) fn blink(&self, restart: f64) -> bool {
        if self.policy.mode.playback(spec::BLINK.kind()) != Playback::Run {
            return true;
        }
        let now = frame::clock(self.stage.ctx());
        let since = (now - restart).max(0.0);
        let step = (since / BLINK_HALF_PERIOD).floor();
        self.wake_at(restart + (step + 1.0) * BLINK_HALF_PERIOD);
        step % 2.0 == 0.0
    }

    pub(crate) fn now(&self) -> f64 {
        frame::clock(self.stage.ctx())
    }

    pub(crate) fn wake_at(&self, at: f64) {
        wake_at(self.stage.ctx(), at);
    }
}

/// Asks for a pass of the running viewport at `at`; the earliest request wins.
fn wake_at(ctx: &Context, at: f64) {
    PerViewport::<Wanted>::with(ctx, pass::current_viewport(ctx), |wanted| wanted.wake = Some(wanted.wake.map_or(at, |old| old.min(at))));
}

/// The 0 to 1 position of a loop of `period` seconds on the pass clock, stepped every `tick` seconds when given
/// (and then waking the viewport at the next step; an unstepped loop keeps it repainting); `None` when `policy`'s
/// mode asks for the still frame. It is the one reading of a loop, for components (`Animate::phase`) and for a canvas
/// tool's paint, which has no component context.
#[doc(hidden)]
pub fn loop_phase(ctx: &Context, policy: MotionPolicy, period: f64, tick: Option<f64>) -> Option<f32> {
    if policy.mode.playback(MotionKind::Loop) != Playback::Run || period <= 0.0 {
        return None;
    }
    let now = frame::clock(ctx);
    match tick.filter(|tick| *tick > 0.0) {
        Some(tick) => {
            let step = (now / tick).floor();
            wake_at(ctx, (step + 1.0) * tick);
            Some((((step * tick) % period) / period) as f32)
        }
        None => {
            note_unsettled(ctx);
            Some(((now % period) / period) as f32)
        }
    }
}

/// Motion on `Cx`, `BodyCx` and `ItemCx`. Every method takes `&self`: slots live in context data under the
/// component's id and go with its shown entries.
pub trait Animate: crate::runtime::memory::sealed::Sealed {
    /// `target` approached over `spec`'s duration; the first call returns `target` without animating.
    fn anim<T: Interpolate>(&self, name: &'static str, target: T, spec: MotionSpec) -> T;
    /// A 0 to 1 value approaching 1 when `on`, else 0.
    fn anim_bool(&self, name: &'static str, on: bool, spec: MotionSpec) -> f32;
    /// Open-close presence for something shown while it opens and fades out.
    fn presence(&self, name: &'static str, open: bool, spec: MotionSpec) -> Presence;
    /// Sets `name` to `value` with no animation.
    fn jump<T: Interpolate>(&self, name: &'static str, value: T);
    /// The 0 to 1 position of a loop of `period` seconds, stepped every `tick` seconds when given; `None` when
    /// the mode asks for the still frame.
    fn phase(&self, period: f64, tick: Option<f64>) -> Option<f32>;
    /// Whether a caret last moved at `restart` (a pass time) shows now: solid right after `restart`, then
    /// blinking; always solid where the motion table's Blink row is solid.
    fn blink(&self, restart: f64) -> bool;
    /// The pass's time in seconds, read once per frame.
    fn now(&self) -> f64;
    /// Asks for a pass at `at` (timers, holds, polls), whatever the motion mode.
    fn wake_at(&self, at: f64);
}

/// The types that reach the motion service.
pub(crate) trait Drives {
    fn driver(&self) -> Driver<'_>;
}

impl<X: Drives + crate::runtime::memory::sealed::Sealed> Animate for X {
    fn anim<T: Interpolate>(&self, name: &'static str, target: T, spec: MotionSpec) -> T {
        self.driver().anim(name, target, spec)
    }

    fn anim_bool(&self, name: &'static str, on: bool, spec: MotionSpec) -> f32 {
        self.driver().anim_bool(name, on, spec)
    }

    fn presence(&self, name: &'static str, open: bool, spec: MotionSpec) -> Presence {
        self.driver().presence(name, open, spec)
    }

    fn jump<T: Interpolate>(&self, name: &'static str, value: T) {
        self.driver().jump(name, value);
    }

    fn phase(&self, period: f64, tick: Option<f64>) -> Option<f32> {
        self.driver().phase(period, tick)
    }

    fn blink(&self, restart: f64) -> bool {
        self.driver().blink(restart)
    }

    fn now(&self) -> f64 {
        self.driver().now()
    }

    fn wake_at(&self, at: f64) {
        self.driver().wake_at(at);
    }
}
