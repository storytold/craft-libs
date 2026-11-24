//! `MiddleAutoscroll`: the press, dead zone, mark and end of a middle-button autoscroll.

use super::{Autoscroll, MiddleAutoscrollFeel};
use crate::canvas::frame_step;
use crate::primitives::{Draw, PaintShape, StrokeWidth};
use crate::style::StyleColor;
use craft_behavior::CursorIcon;
use egui::{Pos2, Vec2, vec2};

/// The mark's art in points around its centre, drawn like an OS cursor bitmap, so these are data, not layout sizes:
/// the ring's radius, the dot's, and the up and down arrows' corners.
const RING: f32 = 13.0;
const DOT: f32 = 2.0;
const ARROWS: [[[f32; 2]; 3]; 2] = [[[-4.0, -6.0], [4.0, -6.0], [0.0, -10.0]], [[-4.0, 6.0], [4.0, 6.0], [0.0, 10.0]]];

/// Paints the autoscroll mark at `at`: a ring filled with `fill` and outlined in `ink`, with an `ink` dot and arrows up
/// and down.
pub(crate) fn paint_mark(draw: &Draw<'_>, at: Pos2, fill: StyleColor, ink: StyleColor) {
    let ring = PaintShape::Circle(at, RING);
    draw.fill(&ring, fill);
    draw.stroke(&ring, StrokeWidth::Thin, ink);
    draw.fill(&PaintShape::Circle(at, DOT), ink);
    for arrow in ARROWS {
        draw.fill(&PaintShape::Polygon(arrow.iter().map(|&[x, y]| at + vec2(x, y)).collect()), ink);
    }
}

/// A middle press's autoscroll: the press, the dead zone, the mark and the end. The speed is the canvas's one
/// autoscroll rule, `Autoscroll::step`.
///
/// The press starts it. A click leaves it running until the next press; a drag while the button is held scrolls the
/// same way, and letting go after a drag ends it.
///
/// It scrolls up and down only, as a document reader's middle click does: the mark shows two vertical arrows, and a
/// sideways pointer offset would scroll a page that fits its width into the pasteboard edge for nothing.
#[derive(Clone, Debug, Default)]
pub(crate) struct MiddleAutoscroll {
    origin: Option<Pos2>,
    /// The press that started the run is still down.
    held: bool,
    /// The held press moved past the drag threshold.
    dragged: bool,
    /// A press ended the run: it and its release reach no tool and start nothing.
    swallow: bool,
    last: Option<f64>,
    /// Under [`MiddleAutoscrollFeel::Reader`], the stepped passes the held press rested past the dead zone.
    held_passes: u32,
}

impl MiddleAutoscroll {
    /// Pointer distance around the mark, in screen points, that does not scroll.
    pub(crate) const DEAD_ZONE: f32 = 15.0;
    /// Under [`MiddleAutoscrollFeel::Reader`], the held passes past the dead zone after which a release ends the run.
    const READER_HELD_PASSES: u32 = 2;
    /// Under [`MiddleAutoscrollFeel::Reader`], the power of the pointer's distance the speed grows with.
    const READER_EXPONENT: f32 = 2.2;
    /// Under [`MiddleAutoscrollFeel::Reader`], points per second per unit of the distance raised to the exponent.
    const READER_GAIN: f32 = 0.04;
    /// The farthest distance the reader's speed is computed from, so the power of a hostile pointer stays finite.
    const READER_MAX_DISTANCE: f32 = 1_000_000.0;

    /// Starts the autoscroll with the mark at `at`, the point of the press that is now held. A point that is not finite
    /// starts nothing.
    pub(crate) fn press(&mut self, at: Pos2) {
        if at.is_finite() {
            self.origin = Some(at);
            self.held = true;
            self.dragged = false;
            self.last = None;
            self.held_passes = 0;
        }
    }

    /// The held press moved past the drag threshold.
    pub(crate) fn drag(&mut self) {
        self.dragged |= self.held;
    }

    /// The press that started the run came up (or was cancelled): after a drag that ends the run, after a click the run
    /// goes on.
    ///
    /// Under [`MiddleAutoscrollFeel::Reader`] the drag threshold does not count: the release ends the run after the held
    /// press rested past the dead zone for two stepped passes, and keeps it after a shorter hold.
    pub(crate) fn release(&mut self, feel: MiddleAutoscrollFeel) {
        let ends = match feel {
            MiddleAutoscrollFeel::Reader => self.held && self.held_passes >= Self::READER_HELD_PASSES,
            _ => self.held && self.dragged,
        };
        self.held = false;
        self.dragged = false;
        if ends {
            self.end();
        }
    }

    /// Whether the press that started the run is still down.
    pub(crate) fn held(&self) -> bool {
        self.held
    }

    /// The view's step this pass for the pointer at `pointer`, `clock` being the pass clock in seconds: screen points
    /// toward the pointer, zero inside the dead zone, while stopped and in the first pass after the press. Later passes
    /// of one frame share the clock, so they step zero.
    ///
    /// Under [`MiddleAutoscrollFeel::Reader`] the speed is the pointer's whole vertical distance from the mark raised to
    /// the power 2.2, times 0.04, in points per second, and a held press resting past the dead zone counts its stepped
    /// passes for [`release`](Self::release).
    pub(crate) fn step(&mut self, pointer: Pos2, clock: f64, feel: MiddleAutoscrollFeel) -> Vec2 {
        let Some(origin) = self.origin else { return Vec2::ZERO };
        let dt = self.last.map_or(0.0, |last| frame_step(clock - last));
        if clock.is_finite() {
            self.last = Some(clock);
        }
        let past = Self::past(origin, pointer);
        match feel {
            MiddleAutoscrollFeel::Reader => {
                if past == Vec2::ZERO || dt <= 0.0 {
                    return Vec2::ZERO;
                }
                // A frame's later passes take no time: counting them would count one frame's hold twice.
                if self.held {
                    self.held_passes = self.held_passes.saturating_add(1);
                }
                let dy = pointer.y - origin.y;
                let speed = dy.abs().min(Self::READER_MAX_DISTANCE).powf(Self::READER_EXPONENT) * Self::READER_GAIN;
                let step = dy.signum() * speed * dt as f32;
                if step.is_finite() { Vec2::new(0.0, step) } else { Vec2::ZERO }
            }
            _ => Autoscroll::step(past, dt as f32),
        }
    }

    /// The pointer's vertical offset from `origin` beyond the dead zone; zero inside it.
    fn past(origin: Pos2, pointer: Pos2) -> Vec2 {
        let dy = pointer.y - origin.y;
        if !dy.is_finite() || dy.abs() <= Self::DEAD_ZONE {
            return Vec2::ZERO;
        }
        Vec2::new(0.0, dy - dy.signum() * Self::DEAD_ZONE)
    }

    /// Ends the autoscroll.
    pub(crate) fn end(&mut self) {
        self.origin = None;
        self.held = false;
        self.dragged = false;
        self.last = None;
        self.held_passes = 0;
    }

    /// The press that ended the run is still down: it is the autoscroll's to its release, so a stopping click cannot
    /// also move the caret or start a selection.
    pub(crate) fn swallow_press(&mut self) {
        self.swallow = true;
    }

    /// Whether a press that ended the run is still down.
    pub(crate) fn swallowing(&self) -> bool {
        self.swallow
    }

    /// The swallowed press came up.
    pub(crate) fn stop_swallowing(&mut self) {
        self.swallow = false;
    }

    /// The mark's position while the autoscroll runs.
    pub(crate) fn mark(&self) -> Option<Pos2> {
        self.origin
    }

    /// The cursor for the pointer at `pointer`: the up-down arrow in the dead zone (or with no pointer), otherwise the
    /// arrow toward the scroll.
    pub(crate) fn cursor(&self, pointer: Option<Pos2>) -> CursorIcon {
        let past = match (self.origin, pointer) {
            (Some(origin), Some(pointer)) => Self::past(origin, pointer),
            _ => Vec2::ZERO,
        };
        if past.y < 0.0 {
            CursorIcon::ResizeNorth
        } else if past.y > 0.0 {
            CursorIcon::ResizeSouth
        } else {
            CursorIcon::ResizeVertical
        }
    }
}
