//! Navigation over a canvas, as in Photoshop: the wheel, trackpad pinch, the page keys, a Hand drag's flick,
//! touchpad momentum and autoscroll during a tool drag. The user's choices are [`NavigationOptions`].
//!
//! Each piece is a pure rule over a [`CanvasView`]: the app (or the canvas's body) reads the pass's input from the
//! runtime, asks the rule for the new view and copies it back into its own view. Zooms go through
//! `craft_behavior::canvas::zoom` and clamp to the [`ZoomLadder`](craft_behavior::ZoomLadder). With Overscroll off
//! the caller clamps the result with the canvas's scroll limits.

mod keys;
mod options;
#[cfg(feature = "testing")]
mod stories;
mod wheel;

pub use keys::PageKey;
pub use options::NavigationOptions;
pub use wheel::{WheelAction, WheelInput, WheelNavigation};

use super::frame_step;
use craft_behavior::kurbo::{Rect as DocRect, Size, Vec2 as DocVec};
use craft_behavior::{CanvasView, Wheel, WheelPhase, WheelUnit};
use egui::{Rect, Vec2};

/// Moves `view` by `delta` screen points (a Hand drag, a scroll): the document follows the pointer, in any rotation
/// or mirror.
pub fn pan(view: &mut CanvasView, delta: Vec2) {
    let d = view.unmap_vec(delta);
    if d.x.is_finite() && d.y.is_finite() {
        view.center -= d;
    }
}

/// Flick panning: releasing a Hand drag (the selected Hand, the Space Hand or a middle-button drag) while the pointer
/// still moves keeps the view gliding that way, slowing to a stop. A click, a new drag or a key stops it
/// ([`stop`](Self::stop)). The Hand tool or the app keeps one per view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct FlickPan {
    /// Recent drag steps: time, pass interval, delta.
    samples: Vec<(f64, f32, Vec2)>,
    dragging: bool,
    /// The glide's speed in points per second, while gliding.
    velocity: Option<Vec2>,
    last: Option<f64>,
}

impl FlickPan {
    /// Pointer movement this recent (seconds) sets the release velocity.
    const WINDOW: f64 = 0.08;
    /// Time constant of the exponential friction (seconds).
    const TAU: f32 = 0.18;
    /// A release slower than this (points per second) does not glide.
    const MIN_START: f32 = 150.0;
    /// The glide stops below this speed.
    const MIN_SPEED: f32 = 15.0;
    /// Fastest glide; a glitchy delta cannot fling the view away.
    const MAX_SPEED: f32 = 6000.0;

    /// The frame step since the previous pass; records `now`.
    fn tick(&mut self, now: f64) -> f32 {
        let dt = self.last.map_or(0.0, |last| frame_step(now - last));
        self.last = Some(now);
        dt as f32
    }

    /// One pass of a Hand drag at `now`: the pointer moved by `delta` screen points. Stops a glide.
    pub fn drag(&mut self, now: f64, delta: Vec2) {
        let dt = self.tick(now);
        self.velocity = None;
        self.dragging = true;
        if delta.is_finite() {
            self.samples.push((now, dt, delta));
        }
        self.samples.retain(|sample| now - sample.0 <= Self::WINDOW);
    }

    /// The drag ended at `now`. With `enabled` (Enable Flick Panning) and a release still moving, a glide starts.
    pub fn release(&mut self, now: f64, enabled: bool) {
        if !self.dragging {
            return;
        }
        self.dragging = false;
        let v = self.release_velocity(now);
        self.samples.clear();
        self.velocity = (enabled && v.length() >= Self::MIN_START).then_some(v);
    }

    /// Pointer velocity over the last `WINDOW` seconds before `now`.
    fn release_velocity(&self, now: f64) -> Vec2 {
        let (mut moved, mut seconds) = (Vec2::ZERO, 0.0f32);
        for &(at, dt, delta) in &self.samples {
            if now - at <= Self::WINDOW {
                moved += delta;
                seconds += dt;
            }
        }
        if seconds <= 1e-4 {
            return Vec2::ZERO;
        }
        let v = moved / seconds;
        if !v.is_finite() {
            return Vec2::ZERO;
        }
        let speed = v.length();
        if speed > Self::MAX_SPEED { v * (Self::MAX_SPEED / speed) } else { v }
    }

    /// The distance the glide covers in the pass at `now` (the exact integral of the decaying speed), in screen
    /// points; `None` when nothing glides.
    pub fn advance(&mut self, now: f64) -> Option<Vec2> {
        let dt = self.tick(now);
        let v = self.velocity?;
        let decay = (-dt / Self::TAU).exp();
        let moved = v * (Self::TAU * (1.0 - decay));
        let next = v * decay;
        self.velocity = (next.is_finite() && next.length() >= Self::MIN_SPEED).then_some(next);
        moved.is_finite().then_some(moved)
    }

    /// Whether a glide runs: the app keeps passes coming while it does.
    pub fn gliding(&self) -> bool {
        self.velocity.is_some()
    }

    /// Stops a glide and forgets the drag (the view hit a limit, a click, a key).
    pub fn stop(&mut self) {
        self.velocity = None;
        self.samples.clear();
        self.dragging = false;
    }
}

/// Touchpad momentum where the platform leaves it to the app (Wayland, which reports a lift as the end of the
/// scroll): a two-finger swipe keeps gliding after the fingers lift. The app creates it enabled only on a platform
/// that sends no momentum of its own.
#[derive(Clone, Debug, PartialEq)]
pub struct TouchpadGlide {
    enabled: bool,
    samples: Vec<(f64, Vec2)>,
    /// The glide's speed in points per second, or zero.
    velocity: Vec2,
    /// When the glide last moved.
    at: f64,
}

impl TouchpadGlide {
    /// Seconds of the gesture the lift speed is measured over.
    const WINDOW: f64 = 0.1;
    /// A lift this long after the last motion is a finger that stopped first: no glide.
    const STILL: f64 = 0.05;
    /// The glide slows by `e` every this many seconds.
    const TAU: f32 = 0.35;
    /// Below this speed (points per second) a lift does not glide.
    const START_SPEED: f32 = 150.0;
    /// A glide stops below this speed.
    const STOP_SPEED: f32 = 10.0;
    /// Fastest glide, so a lift measured over a hiccup cannot throw the document to its end.
    const MAX_SPEED: f32 = 8000.0;
    /// A pass this long after the last one (the view was hidden, the app stalled) ends a glide.
    const STALE: f64 = 0.1;

    /// A glide that runs when `enabled`.
    pub fn new(enabled: bool) -> Self {
        Self { enabled, samples: Vec::new(), velocity: Vec2::ZERO, at: 0.0 }
    }

    fn halt(&mut self) {
        self.velocity = Vec2::ZERO;
    }

    fn push(&mut self, now: f64, delta: Vec2) {
        if !delta.is_finite() {
            return;
        }
        match self.samples.last_mut() {
            // Several events in one pass are one sample.
            Some((at, sum)) if *at == now => *sum += delta,
            _ => self.samples.push((now, delta)),
        }
        self.samples.retain(|(at, _)| now - at <= Self::WINDOW);
    }

    /// The finger's speed as it lifted at `now`, or zero when it should not glide.
    fn lift_velocity(&self, now: f64) -> Vec2 {
        let recent: Vec<&(f64, Vec2)> = self.samples.iter().filter(|(at, _)| now - at <= Self::WINDOW).collect();
        let (Some(first), Some(last)) = (recent.first(), recent.last()) else { return Vec2::ZERO };
        let span = (last.0 - first.0) as f32;
        if recent.len() < 2 || span <= 0.0 || now - last.0 > Self::STILL {
            return Vec2::ZERO;
        }
        // Each sample is the motion since the pass before it, so the first one's motion happened before the span
        // starts.
        let moved = recent.iter().skip(1).fold(Vec2::ZERO, |sum, (_, d)| sum + *d);
        let v = moved / span;
        if !v.is_finite() || v.length() < Self::START_SPEED {
            return Vec2::ZERO;
        }
        if v.length() > Self::MAX_SPEED { v * (Self::MAX_SPEED / v.length()) } else { v }
    }

    /// Reads the pass's wheel share at `now`; any wheel input stops a glide, a touchpad gesture's end may start one.
    /// `glide` is false where the view must not glide this pass (a zoom, a dialog).
    pub fn read(&mut self, wheel: &Wheel, now: f64, glide: bool) {
        self.halt();
        // Zooming, a mouse wheel and sideways Shift scrolling never glide.
        if wheel.unit != WheelUnit::Point || wheel.mods.cmd || wheel.mods.ctrl || wheel.mods.shift || wheel.zoom != 1.0 {
            self.samples.clear();
            return;
        }
        match wheel.phase {
            WheelPhase::Start => {
                self.samples.clear();
                self.push(now, wheel.scroll);
            }
            WheelPhase::Move => self.push(now, wheel.scroll),
            WheelPhase::End => {
                if glide && self.enabled {
                    self.velocity = self.lift_velocity(now);
                    self.at = now;
                }
                self.samples.clear();
            }
            _ => self.samples.clear(),
        }
    }

    /// The scroll this pass at `now`, in points; zero once stopped.
    pub fn step(&mut self, now: f64) -> Vec2 {
        if self.velocity == Vec2::ZERO {
            return Vec2::ZERO;
        }
        let dt = now - self.at;
        if !(0.0..=Self::STALE).contains(&dt) {
            self.halt();
            return Vec2::ZERO;
        }
        self.at = now;
        let decay = (-(dt as f32) / Self::TAU).exp();
        let moved = self.velocity * Self::TAU * (1.0 - decay);
        self.velocity *= decay;
        if self.velocity.length() < Self::STOP_SPEED {
            self.halt();
        }
        moved
    }

    /// Whether the view moved in the pass of the last [`step`](Self::step): a pass that left it where it was (the
    /// edge clamp) stops the glide.
    pub fn moved(&mut self, moved: bool) {
        if !moved {
            self.halt();
        }
    }
}

/// Autoscroll while a tool drag runs past the canvas edge (marquee, lasso, brush, crop): the view scrolls toward the
/// pointer, faster the further past the edge, and stops at the image's edge (an image that fits is never scrolled
/// into empty pasteboard). The app runs it from [`ToolHostResponse::past_edge`](crate::canvas::ToolHostResponse) for
/// tools whose `autoscrolls` is true.
pub struct Autoscroll;

impl Autoscroll {
    /// Slowest scroll, points per second.
    pub const MIN_SPEED: f32 = 120.0;
    /// Fastest scroll, points per second.
    pub const MAX_SPEED: f32 = 2400.0;
    /// Extra speed per point the pointer is past the edge.
    pub const GAIN: f32 = 12.0;

    /// Moves `view` by `step` screen points, never past the image's on-screen bounds in the direction of travel (a
    /// view already past a limit is not pulled back). Returns the move in document units, or `None` when the view did
    /// not move.
    pub fn scroll(view: &mut CanvasView, step: Vec2, size: Size) -> Option<DocVec> {
        if !step.is_finite() || step == Vec2::ZERO || !view.zoom.is_finite() || view.zoom <= 0.0 {
            return None;
        }
        let image = document_bounds(view, size)?;
        let canvas = view.rect;
        // Screen axes are compared before unrotating: a document axis that fits before rotation may be clipped
        // afterwards, and a vertical screen drag stays vertical at any angle.
        let axis = |step: f32, lo: f32, hi: f32, v0: f32, v1: f32| {
            if hi - lo <= v1 - v0 {
                0.0
            } else if step > 0.0 {
                step.min((hi - v1).max(0.0))
            } else {
                step.max((lo - v0).min(0.0))
            }
        };
        let step = Vec2::new(
            axis(step.x, image.left(), image.right(), canvas.left(), canvas.right()),
            axis(step.y, image.top(), image.bottom(), canvas.top(), canvas.bottom()),
        );
        let d = view.unmap_vec(step);
        let moved = d.x.is_finite() && d.y.is_finite() && (d.x.abs() > 1e-3 || d.y.abs() > 1e-3);
        if moved {
            view.center += d;
        }
        moved.then_some(d)
    }
}

/// The document's bounds on screen as `view` draws it (the bounding box of the turned quad).
fn document_bounds(view: &CanvasView, size: Size) -> Option<Rect> {
    if !(size.width > 0.0 && size.height > 0.0) {
        return None;
    }
    let quad = view.quad(DocRect::new(0.0, 0.0, size.width, size.height));
    let bounds = quad.iter().skip(1).fold(Rect::from_min_max(quad[0], quad[0]), |r, &p| r.union(Rect::from_min_max(p, p)));
    (bounds.is_finite() && view.rect.is_finite()).then_some(bounds)
}
