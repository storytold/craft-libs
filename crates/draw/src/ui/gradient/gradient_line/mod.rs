//! The geometry of a gradient line on a canvas: where a colour stop sits on it, the location under
//! a point and the 45° snap of an end. Pure functions, so an app's own
//! canvas code and [`GradientOverlay`](crate::ui::gradient::GradientOverlay) agree on where things are.
//!
//! A stop's location `t` (0..=1) is the stop's own. With `reverse` it is drawn at `1 − t` from the
//! start (PhotoCraft's `u_of`), and every location read back from a point is turned back the same
//! way.

use craft_behavior::geometry::draft::turn_to_45;
use egui::Pos2;
use kurbo::{Point, Vec2};

use crate::ui::gradient::{GradientStops, StopRef, StopRow};

/// Midpoint diamonds show only between stops farther apart than this, as the stop bar's do.
pub const MIN_MIDPOINT_GAP: f32 = 0.001;

/// A gradient's line between its start and end handles, in document units.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct GradientLine {
    /// The start handle.
    pub start: Point,
    /// The end handle.
    pub end: Point,
}

impl GradientLine {
    /// The line from `start` to `end`.
    pub fn new(start: Point, end: Point) -> Self {
        Self { start, end }
    }
}

/// What [`GradientOverlay`](crate::ui::gradient::GradientOverlay) edits: the line and the stops on it.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LineGradient {
    /// The line between the handles.
    pub line: GradientLine,
    /// The colour and opacity stops along it.
    pub stops: GradientStops,
}

impl LineGradient {
    /// The gradient with `line` and `stops`.
    pub fn new(line: GradientLine, stops: GradientStops) -> Self {
        Self { line, stops }
    }

    /// The host's tag of the colour stop `stop` names; a midpoint has the tag of the stop before
    /// it. `None` for opacity stops and indices past the row.
    pub fn tag_of(&self, stop: StopRef) -> Option<u64> {
        match stop {
            StopRef::Color(i) | StopRef::Midpoint(StopRow::Color, i) => self.stops.color.get(i).map(|s| s.tag),
            _ => None,
        }
    }

    /// The tag of the colour stop `part` is, as [`tag_of`](Self::tag_of); `None` for the ends and
    /// the line. For the parts [`GradientOverlayOutput`](crate::ui::gradient::GradientOverlayOutput)
    /// reports (`picked`, `hovered`, `context`), read from the gradient the overlay showed.
    pub fn part_tag(&self, part: LinePart) -> Option<u64> {
        match part {
            LinePart::Stop(i) => self.tag_of(StopRef::Color(i)),
            LinePart::Midpoint(i) => self.tag_of(StopRef::Midpoint(StopRow::Color, i)),
            _ => None,
        }
    }
}

/// The shape a gradient's style adds round the line.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum GradientExtent {
    /// Only the line (linear).
    #[default]
    None,
    /// A circle of the line's length round the start (radial, angle).
    Circle,
    /// A square with the end as one corner, centred on the start (diamond).
    Square,
    /// The line mirrored through the start, dashed (reflected).
    Mirror,
}

/// A part of the line a pointer or key takes. Indices are into the colour row's stops.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LinePart {
    /// The line between the handles.
    #[default]
    Line,
    /// The start handle.
    Start,
    /// The end handle.
    End,
    /// Colour stop `i`.
    Stop(usize),
    /// The diamond between stop `i` and stop `i + 1`.
    Midpoint(usize),
}

/// Where on the line, as a fraction from the start, location `t` is drawn.
fn along(t: f32, reverse: bool) -> f32 {
    if reverse { 1.0 - t } else { t }
}

/// The document point where location `t` sits on `line`.
pub fn at(line: &GradientLine, t: f32, reverse: bool) -> Point {
    line.start + (line.end - line.start) * f64::from(along(t, reverse))
}

/// The location of the point on `line` nearest `p`, clamped to 0..=1, and the distance from `p` to
/// that point (document units). A zero-length line projects to location 0.
pub fn project(line: &GradientLine, p: Point, reverse: bool) -> (f32, f64) {
    let d = line.end - line.start;
    let len_sq = d.hypot2();
    let u = if len_sq > 0.0 { ((p - line.start).dot(d) / len_sq).clamp(0.0, 1.0) } else { 0.0 };
    let nearest = line.start + d * u;
    (along(u as f32, reverse), nearest.distance(p))
}

/// `moved` turned about `fixed` onto the nearest multiple of 45° counted from `base_deg`, keeping
/// its distance from `fixed`. Angles are those of the document's axes (y down).
pub fn snap45(fixed: Point, moved: Point, base_deg: f64) -> Point {
    let base = Vec2::from_angle(base_deg.to_radians());
    let turn = |v: Vec2, by: Vec2| Vec2::new(v.x * by.x - v.y * by.y, v.x * by.y + v.y * by.x);
    let local = turn(moved - fixed, Vec2::new(base.x, -base.y));
    let snapped = turn(turn_to_45(Point::ORIGIN, local.to_point()).to_vec2(), base);
    fixed + snapped
}

/// The location of the midpoint diamond after colour stop `i`: its stop's location plus the
/// midpoint's share of the way to the next; `None` for the last stop and for stops no farther
/// apart than [`MIN_MIDPOINT_GAP`].
pub fn midpoint_location(stops: &GradientStops, i: usize) -> Option<f32> {
    let (a, b) = (stops.color.get(i)?, stops.color.get(i + 1)?);
    (b.location - a.location > MIN_MIDPOINT_GAP).then_some(a.location + a.midpoint * (b.location - a.location))
}

/// The screen point where location `t` sits on the screen line `start` to `end`.
pub fn at_screen(start: Pos2, end: Pos2, t: f32, reverse: bool) -> Pos2 {
    start + (end - start) * along(t, reverse)
}
