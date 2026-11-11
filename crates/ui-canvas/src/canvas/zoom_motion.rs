//! The motion of zooming: the ease of an animated zoom, the Zoom tool's held press and its drag.
//!
//! [`ZoomAnimation`] holds no clock. The `Canvas` that shows the animated view drives a 0 to 1 progress with
//! `Animate::anim` (so reduced motion and the motion table's duration apply) and asks the animation for the view to
//! draw. Each canvas runs one, under its id: its own zoom commands start it, and the tool host and the Navigator
//! start it through [`ease`].

use super::frame_step;
use super::navigate::WheelNavigation;
use crate::runtime::frame;
use crate::runtime::memory::{global, with_session};
use craft_behavior::canvas::zoom;
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, ZoomLadder};
use egui::{Context, Id, Pos2, Rect};

/// How far the app's view may differ from the zoom anchor, in document units, before the anchor is dropped.
const ANCHOR_TOLERANCE: f64 = 0.01;

/// A zoom easing from the view drawn last pass to the app's new view.
///
/// The canvas calls [`begin`](Self::begin) when a zoom starts, feeds [`target`](Self::target) to
/// `Animate::anim("zoom", target, spec::MOVE)` every pass and passes the answer to [`shown`](Self::shown), which
/// returns the view to draw. A new zoom while one runs starts from the view in between.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ZoomAnimation {
    /// One more for every zoom begun: the value the progress runs toward.
    generation: u32,
    run: Option<Run>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
struct Run {
    from: CanvasView,
    /// A document point and the screen point it stays at, for a zoom about the pointer.
    anchor: Option<(Point, Pos2)>,
    /// The progress when the run started.
    base: f32,
    /// The app's zoom and centre the ease heads for; `None` until the first pass after `begin`.
    goal: Option<(f64, Point)>,
    /// The view drawn last pass: where an ease restarts from.
    shown: CanvasView,
}

impl ZoomAnimation {
    /// The value the owner's progress animation runs toward: it grows by one for every zoom begun.
    pub fn target(&self) -> f32 {
        self.generation as f32
    }

    /// Starts easing from `from` (the view drawn last pass) toward whatever the app's view is on the next pass, with
    /// `anchor` (a document point and the screen point it stays at) for a zoom about the pointer. `progress` is the
    /// owner's current progress value. While a zoom runs, the new one starts from the view it drew last.
    pub fn begin(&mut self, from: &CanvasView, anchor: Option<(Point, Pos2)>, progress: f32) {
        let from = self.run.map_or(*from, |run| run.shown);
        self.generation = self.generation.wrapping_add(1);
        self.run = Some(Run { from, anchor, base: progress, goal: None, shown: from });
    }

    /// The view to draw this pass for the app's `target`, at the owner's `progress`. A target the anchor no longer
    /// fits (a clamp, a canvas resize, another camera command) drops the anchor: the target always wins.
    pub fn shown(&mut self, target: &CanvasView, progress: f32) -> CanvasView {
        let end = self.generation as f32;
        let Some(run) = self.run.as_mut() else { return *target };
        let goal = (target.zoom, target.center);
        if run.goal != Some(goal) {
            // The app's view moved while easing (or this is the first pass): ease on from what was drawn.
            if run.goal.is_some() {
                run.from = run.shown;
                run.base = progress;
            }
            run.goal = Some(goal);
        }
        if let Some((doc, screen)) = run.anchor {
            let at = target.to_doc(screen);
            if (at.x - doc.x).abs() > ANCHOR_TOLERANCE || (at.y - doc.y).abs() > ANCHOR_TOLERANCE {
                run.anchor = None;
            }
        }
        let span = end - run.base;
        let t = if span > 0.0 { f64::from(((progress - run.base) / span).clamp(0.0, 1.0)) } else { 1.0 };
        if t >= 1.0 {
            self.run = None;
            return *target;
        }
        let mut view = *target;
        view.zoom = run.from.zoom + (target.zoom - run.from.zoom) * t;
        view.center = run.from.center.lerp(target.center, t);
        if let Some((doc, screen)) = run.anchor
            && view.zoom.is_finite()
            && view.zoom > 0.0
        {
            view.center = doc - view.unmap_vec(screen - view.rect.center());
        }
        run.shown = view;
        view
    }

    /// Whether a zoom is easing.
    pub fn running(&self) -> bool {
        self.run.is_some()
    }

    /// Stops easing: the next [`shown`](Self::shown) is the target (a pan or a scroll took over).
    pub fn stop(&mut self) {
        self.run = None;
    }
}

/// What a view change asks of a canvas's zoom ease, from outside the canvas's own pass.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum Steer {
    /// Ease from `from` (the view drawn when the change was asked for) to the canvas's next view, keeping the anchor.
    Ease { from: CanvasView, anchor: Option<(Point, Pos2)> },
    /// The view jumped (a tool's pan): the ease stops at the next view.
    Stop,
}

/// What a canvas keeps under its id between passes: the gesture state of its view, never a view of its own.
#[derive(Clone, Debug, Default)]
pub(crate) struct ViewMotion {
    /// The part of a ladder-step wheel notch a trackpad has not finished.
    pub(crate) wheel: WheelNavigation,
    /// The canvas's one zoom ease.
    pub(crate) animation: ZoomAnimation,
    /// What the tool host or the Navigator asked of the ease since the canvas's last pass.
    pub(crate) steer: Option<Steer>,
}

impl ViewMotion {
    fn id(canvas: Id) -> Id {
        global("craft-ui canvas view motion", canvas)
    }

    /// The motion of the canvas `canvas`, moved out of its slot for the canvas's pass.
    pub(crate) fn take(ctx: &Context, canvas: Id) -> Self {
        with_session(ctx, Self::id(canvas), std::mem::take::<ViewMotion>)
    }

    /// Puts the motion back after the canvas's pass, keeping a steer asked for during it.
    pub(crate) fn restore(self, ctx: &Context, canvas: Id) {
        with_session(ctx, Self::id(canvas), |slot: &mut ViewMotion| {
            let steer = slot.steer.take();
            *slot = self;
            slot.steer = steer.or(slot.steer);
        });
    }
}

/// Asks the canvas `canvas` (its id, `CanvasOutput::response.id`) to ease its next view or to stop easing: the
/// tool host for a tool's view, the Navigator for a preview zoom. The canvas applies it in its next pass.
pub(crate) fn steer(ctx: &Context, canvas: Id, steer: Steer) {
    with_session(ctx, ViewMotion::id(canvas), |slot: &mut ViewMotion| slot.steer = Some(steer));
    frame::request_repaint(ctx);
}

/// The Zoom tool's press held still: past [`DELAY`](Self::DELAY) it zooms in continuously about the press point (out
/// with Alt) until the button comes up, [`RATE`](Self::RATE) doublings a second of wall time at any frame rate. The
/// release is then no click step. The tool drops a hold that has not started zooming when the press turns into a
/// drag. Only with Animated Zoom on; off, a hold is a click.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct HoldZoom {
    anchor: Pos2,
    doc: Point,
    pressed_at: f64,
    last: Option<f64>,
}

impl HoldZoom {
    /// Seconds a press is held still before it zooms.
    pub const DELAY: f64 = 0.3;
    /// Doublings (halvings zooming out) per second.
    pub const RATE: f64 = 2.0;

    /// A press at `screen` over document point `doc` at `time`.
    pub fn press(screen: Pos2, doc: Point, time: f64) -> Self {
        Self { anchor: screen, doc, pressed_at: time, last: None }
    }

    /// Advances the hold for the pass at `time`: once past the delay, zooms `view` about the press point (out when
    /// `out`). Returns `true` while the hold zooms, so the tool skips its scrub and its click step.
    pub fn advance(&mut self, view: &mut CanvasView, time: f64, out: bool, size: Size, pixels_per_point: f32) -> bool {
        if self.last.is_none() {
            let begin = self.pressed_at + Self::DELAY;
            if time < begin {
                return false;
            }
            self.last = Some(begin);
        }
        let dt = frame_step(time - self.last.unwrap_or(time));
        let current = zoom::device(view, pixels_per_point);
        let next = Self::zoom_after(current, dt, out, size);
        // At a limit the view stays as it is.
        if (next / current - 1.0).abs() > 1e-12 {
            zoom::anchor(view, self.anchor, self.doc, next, pixels_per_point);
        }
        self.last = Some(time);
        true
    }

    /// Whether the hold has started zooming (the release is then no click step).
    pub fn zooming(&self) -> bool {
        self.last.is_some()
    }

    /// Whether the press still waits out the delay and may start zooming at `time`.
    pub fn waiting(&self, time: f64) -> bool {
        self.last.is_none() && time < self.pressed_at + Self::DELAY
    }

    /// The zoom after holding for a frame step of `dt` seconds at `zoom`, within the document's range. A non-finite
    /// or negative `dt` changes nothing, and `dt` is capped at [`FRAME_STEP_MAX`](super::FRAME_STEP_MAX).
    pub fn zoom_after(zoom: f64, dt: f64, out: bool, size: Size) -> f64 {
        let dt = frame_step(dt);
        let doublings = if out { -Self::RATE * dt } else { Self::RATE * dt };
        ZoomLadder::clamp(zoom * doublings.exp2(), size)
    }
}

/// A Zoom tool drag. With Scrubby Zoom on (Photoshop's default) the drag zooms continuously, right in and left out,
/// [`DOUBLING`](Self::DOUBLING) points per doubling, about the press point. Off, it draws a rectangle and the release
/// zooms to fit it (Alt: out, so the view steps one rung out about the press point). Whether a press is a drag or a
/// click is the host's call (`drag` reached the tool or not), not the drag's.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct ZoomDrag {
    anchor: Pos2,
    doc: Point,
    zoom0: f64,
    corner: Option<Pos2>,
}

impl ZoomDrag {
    /// Horizontal drag in screen points that doubles (or halves) the zoom.
    pub const DOUBLING: f32 = 100.0;

    /// A drag pressed at `screen` on `view`. `scrubby` is the Zoom tool's Scrubby Zoom option.
    pub fn begin(view: &CanvasView, screen: Pos2, scrubby: bool, pixels_per_point: f32) -> Self {
        Self { anchor: screen, doc: view.to_doc(screen), zoom0: zoom::device(view, pixels_per_point), corner: (!scrubby).then_some(screen) }
    }

    /// The pointer moved to `pointer`: a scrub zooms `view` about the press point; a rectangle moves its corner (the
    /// view is unchanged).
    pub fn drag(&mut self, view: &mut CanvasView, pointer: Pos2, size: Size, pixels_per_point: f32) {
        match self.corner.as_mut() {
            Some(corner) => *corner = pointer,
            None => {
                let next = Self::scrub_zoom(self.zoom0, pointer.x - self.anchor.x, size);
                if (next / zoom::device(view, pixels_per_point) - 1.0).abs() > 1e-12 {
                    zoom::anchor(view, self.anchor, self.doc, next, pixels_per_point);
                }
            }
        }
    }

    /// The button came up: a rectangle zooms `view` to fit it, or with `out` steps the zoom out about the press point;
    /// a scrub has nothing left to do. A rectangle narrower or shorter than `least` points is too small to zoom to:
    /// the view stays.
    pub fn release(self, view: &mut CanvasView, out: bool, size: Size, pixels_per_point: f32, least: f32) {
        let Some(corner) = self.corner else { return };
        let rect = Rect::from_two_pos(self.anchor, corner);
        if rect.width() < least || rect.height() < least || rect.width() <= 0.0 || rect.height() <= 0.0 {
            return;
        }
        let current = zoom::device(view, pixels_per_point);
        if out {
            let next = ZoomLadder::step(current, -1, size);
            zoom::anchor(view, self.anchor, self.doc, next, pixels_per_point);
            return;
        }
        // The ratio of two screen lengths scales the zoom in any unit.
        let k = f64::from((view.rect.width() / rect.width()).min(view.rect.height() / rect.height()));
        let next = ZoomLadder::clamp(current * k, size);
        let target = view.to_doc(rect.center());
        let middle = view.rect.center();
        zoom::anchor(view, middle, target, next, pixels_per_point);
    }

    /// The zoom rectangle on screen while it is drawn (Scrubby Zoom off).
    pub fn rect(&self) -> Option<Rect> {
        self.corner.map(|corner| Rect::from_two_pos(self.anchor, corner))
    }

    /// The scrubby zoom for a horizontal drag of `dx` points from a press at `zoom0`, within the document's range.
    pub fn scrub_zoom(zoom0: f64, dx: f32, size: Size) -> f64 {
        // 40 doublings already span far more than the zoom range; the bound keeps the power finite.
        let doublings = if dx.is_finite() { f64::from(dx / Self::DOUBLING).clamp(-40.0, 40.0) } else { 0.0 };
        ZoomLadder::clamp(zoom0 * doublings.exp2(), size)
    }
}
