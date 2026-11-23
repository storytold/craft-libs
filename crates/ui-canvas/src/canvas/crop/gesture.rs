//! What a pointer drag does to the crop frame: the kinds of drag, the frame each step gives, and how image mode moves
//! the view with it.

use craft_behavior::geometry::crop::{direction, drawn, image_turn, normalized_angle};
use craft_behavior::geometry::{AspectHold, BoxPart, CropBox, SnapLine, crop_view_about, screen_step};
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_behavior::{Axis, CanvasView};
use craft_keymap::{Mods, Platform};
use egui::Pos2;

use crate::canvas::{GestureSnap, Snap, snap_tolerance};

/// What a drag does to the frame.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum Kind {
    /// A new frame from `anchor`; `cur` is the opposite corner, `last` the pointer of the last step (Space moves the
    /// frame by the pointer's step).
    Draw { anchor: Point, cur: Point, last: Point },
    /// Moves the frame on the page.
    Move,
    /// Drags the corner or edge, along the frame's own axes.
    Resize(BoxPart),
    /// Turns the frame about its centre; `grab` is the pointer's direction from it at the press, in radians.
    Turn { grab: f64 },
    /// A Straighten line from screen point `from` to `to`.
    Line { from: Pos2, to: Pos2 },
}

/// A drag in progress, kept between passes.
#[derive(Clone, Debug)]
pub(super) struct Drag {
    pub kind: Kind,
    /// The frame at the press.
    pub at_press: CropBox,
    /// The press in document space.
    pub from: Point,
    /// Image mode: the view at the press. Every pointer point of the drag is read in it, because the view moves and
    /// turns with the image while the drag runs.
    pub press_view: Option<CanvasView>,
    /// The frame the drag last produced.
    pub last: CropBox,
    /// A step of this drag has proposed a frame other than the press frame. It stays set until the drag ends, so a
    /// drag that brings the frame back keeps the started look.
    pub changed: bool,
    /// The pointer of the last step, in screen points.
    pub pointer: Pos2,
}

impl Drag {
    /// The pointer that asks to scroll the view past the canvas edge: a draw, a classic move, and a resize the view
    /// does not keep centred. A turn, an image-mode move (the view follows the frame) and a Straighten line never
    /// scroll. Read from the running drag, not this pass's input, so every pass of a frame reports the same.
    pub fn scrolls_at(&self, image: bool, auto_center: bool) -> Option<Pos2> {
        let scrolls = match self.kind {
            Kind::Draw { .. } => true,
            Kind::Move => !image,
            Kind::Resize(_) => !(image && auto_center),
            Kind::Turn { .. } | Kind::Line { .. } => false,
        };
        scrolls.then_some(self.pointer)
    }
}

/// The rules every step of a drag follows.
#[derive(Clone)]
pub(super) struct Rules {
    pub ratio: Option<f64>,
    pub bounds: Option<Rect>,
    pub turn_step: f64,
    pub image: bool,
    pub auto_center: bool,
    pub fills_uncovered: bool,
    /// The smallest side a resize may leave, in document units: a step that would go under it keeps the last frame.
    pub min_side: f64,
    /// The quad the frame is bounded to: a step that would leave the frame outside it keeps the last frame.
    pub area: Option<[Point; 4]>,
    /// An edge or corner drag, and a move, may take the frame past `bounds`, turned or not: the crop grows the canvas
    /// there, as in the reference application (a document crop, not one bounded by a placed picture's source).
    pub resize_grows: bool,
    /// The canvas's snapping, with this pass's tolerance.
    pub snap: Option<Snap>,
    pub os: Platform,
}

impl Rules {
    /// What a resize holds: the set ratio; else, with Shift, the frame's own proportions along its diagonal.
    fn hold(&self, mods: Mods) -> AspectHold {
        match self.ratio {
            Some(k) => AspectHold::Ratio(k),
            None if mods.shift => AspectHold::Diagonal,
            None => AspectHold::Free,
        }
    }

    /// The frame a drag gives with the pointer at document point `to`, kept inside the bounds; `None` when the frame is
    /// as it was (a draw not yet sized) or cannot be kept inside.
    pub fn proposal(&self, drag: &mut Drag, to: Point, mods: Mods, space: bool) -> Option<CropBox> {
        let at = drag.at_press;
        let mut grows = false;
        let (next, resizing, held) = match &mut drag.kind {
            Kind::Draw { anchor, cur, last } => {
                if space {
                    // Space: the frame being drawn follows the pointer, keeping its size.
                    let step = to - *last;
                    *anchor += step;
                    *cur += step;
                } else {
                    *cur = to;
                }
                *last = to;
                if *anchor == *cur {
                    return None;
                }
                (CropBox::upright(drawn(*anchor, *cur, self.ratio, mods)), true, self.ratio.is_some() || mods.shift)
            }
            Kind::Move => {
                // A moved document crop may take in the area past the canvas, as an edge drag does (photocraft#16404),
                // also once turned (photocraft#16460): only the turn itself keeps the frame inside the image (#10869).
                grows = self.resize_grows;
                let by = if self.image { drag.from - to } else { to - drag.from };
                (at.moved(by), false, false)
            }
            Kind::Resize(part) => {
                let by = to - drag.from;
                let hold = self.hold(mods);
                let next = if self.image && self.auto_center {
                    at.resized_centred(*part, by, hold, mods.alt)?
                } else {
                    at.resized(*part, by, hold, mods.alt)
                };
                // A step that would leave a side under the minimum keeps the last frame, as a step the bounds refuse
                // does: the resize stops there instead of crossing it.
                if !next.size_ok(self.min_side) {
                    return Some(drag.last);
                }
                grows = self.resize_grows;
                (next, true, self.ratio.is_some() || mods.shift)
            }
            Kind::Turn { grab } => {
                let step = mods.shift.then_some(self.turn_step);
                let turned = at.turned_to(*grab, to, step, self.image);
                match self.bounds.filter(|_| self.image) {
                    // An image-mode turn keeps the frame inside the turned image (#10869), each step from the press
                    // frame so turning back gives it back. ⌘ keeps the press frame, and so does a press frame that
                    // was already past the bounds, and a crop that fills what the turn uncovers.
                    Some(bounds) => {
                        let keeps_press = mods.cmd || self.fills_uncovered || at.kept_inside(bounds) != Some(at);
                        let kept = if keeps_press { Some(turned) } else { turned.kept_inside(bounds) };
                        return kept.filter(CropBox::is_finite).or(Some(drag.last));
                    }
                    None => (turned, true, false),
                }
            }
            Kind::Line { .. } => return None,
        };
        if !next.is_finite() {
            return None;
        }
        let kept = match self.bounds.filter(|_| !grows) {
            Some(bounds) => next.within(bounds, resizing, held),
            None => Some(next),
        };
        kept.filter(|kept| self.area.is_none_or(|area| inside_quad(kept, &area))).or(Some(drag.last))
    }

    /// The screen point of `pos` after the canvas's snapping: an end of the Straighten line.
    pub fn snap_screen(&self, view: &CanvasView, pos: Pos2, mods: Mods) -> Pos2 {
        let Some(snap) = &self.snap else { return pos };
        let (point, lines) = snap.point(view.to_doc(pos), mods, GestureSnap::default());
        if lines.is_empty() { pos } else { view.to_screen(point) }
    }

    /// The drag's document point for the pointer at `to` after the canvas's snapping, with the lines that snapped. In
    /// image mode the point is in the view of the press, which the moving view never feeds back into: the correction
    /// is turned back into pointer travel. A corner that holds its proportions (a ratio, or Shift) slides along its
    /// diagonal onto the line it snapped to, so the handle lands on the line with the proportions kept.
    pub fn snapped(&self, drag: &Drag, view: &CanvasView, to: Point, mods: Mods, space: bool) -> (Point, Vec<SnapLine>) {
        let Some(snap) = &self.snap else { return (to, Vec::new()) };
        let mut probe = drag.clone();
        let Some(candidate) = self.proposal(&mut probe, to, mods, space) else { return (to, Vec::new()) };
        let (by, lines, travel_per_unit) = match probe.kind {
            Kind::Draw { cur, .. } => {
                let (snapped, lines) = snap.point(cur, mods, GestureSnap::default());
                (snapped - cur, lines, 1.0)
            }
            Kind::Move => {
                let (by, lines) = snap.rect(candidate.turned_bounds(), mods);
                (by, lines, if self.image { -1.0 } else { 1.0 })
            }
            Kind::Resize(part) => {
                let Some(point) = candidate.part_point(part) else { return (to, Vec::new()) };
                let (snapped, lines) = snap.point(point, mods, GestureSnap::default());
                let travel = if self.image && self.auto_center && !mods.alt { 0.5 } else { 1.0 };
                let corner = matches!(part, BoxPart::Corner(_));
                if corner && !lines.is_empty() && self.hold(mods) != AspectHold::Free {
                    // The reach at the press view: the rule `Snap::at` sets the tolerance with; a crop drag never
                    // changes the zoom.
                    let Some(reach) = snap_tolerance(drag.press_view.as_ref().unwrap_or(view), mods, self.os, snap.options().suspend, false) else {
                        return (to, Vec::new());
                    };
                    match slid_along(point, point - candidate.center(), lines, reach) {
                        Some((by, lines)) => (by, lines, travel),
                        // The diagonal meets no snapped line within reach: the handle stays where the drag put it.
                        None => return (to, Vec::new()),
                    }
                } else {
                    (snapped - point, lines, travel)
                }
            }
            Kind::Turn { .. } | Kind::Line { .. } => return (to, Vec::new()),
        };
        if by.x.is_finite() && by.y.is_finite() { (to + by * travel_per_unit, lines) } else { (to, Vec::new()) }
    }
}

/// How far (a cross product, in document units squared) a corner may lie past a quad's edge and still count as inside
/// it: rounding, not a visible gap.
const QUAD_TOLERANCE: f64 = 1e-6;

/// Whether every corner of `frame` lies in the convex `quad`, whichever way the quad winds. A quad with no area bounds
/// nothing.
fn inside_quad(frame: &CropBox, quad: &[Point; 4]) -> bool {
    let side = |a: Point, b: Point, p: Point| (b.x - a.x) * (p.y - a.y) - (b.y - a.y) * (p.x - a.x);
    let winding = side(quad[0], quad[1], quad[2]);
    if !winding.is_finite() || winding == 0.0 {
        return true;
    }
    let sign = winding.signum();
    frame.corners().iter().all(|p| (0..4).all(|i| side(quad[i], quad[(i + 1) % 4], *p) * sign >= -QUAD_TOLERANCE))
}

/// The move that slides corner `point` along direction `along` (its diagonal) onto the nearest of `lines` it crosses
/// within `reach` of `point` on each axis, the box the snap read the handle in, with the lines the slid corner lies
/// on. `None` when the diagonal meets none of them that near: a diagonal nearly parallel to a line meets it far along.
fn slid_along(point: Point, along: Vec2, lines: Vec<SnapLine>, reach: f64) -> Option<(Vec2, Vec<SnapLine>)> {
    // A vertical line (along y) is met where x reaches it, a horizontal one where y does.
    let gap = |line: &SnapLine| if matches!(line.along, Axis::Y) { (line.position - point.x, along.x) } else { (line.position - point.y, along.y) };
    let s = lines
        .iter()
        .filter_map(|line| {
            let (gap, step) = gap(line);
            (step.abs() > 1e-9).then(|| gap / step)
        })
        .filter(|s| s.is_finite() && (s * along.x).abs() <= reach && (s * along.y).abs() <= reach)
        .min_by(|a, b| a.abs().total_cmp(&b.abs()))?;
    let by = along * s;
    let at = point + by;
    let on = lines
        .into_iter()
        .filter(|line| {
            let v = if matches!(line.along, Axis::Y) { at.x } else { at.y };
            (v - line.position).abs() < 1e-6
        })
        .collect();
    Some((by, on))
}

/// The view after a drag step that gave frame `next`, from the view of the press, so the view moving with the image
/// never feeds back into the drag. `None` outside image mode.
pub(super) fn followed(drag: &Drag, auto_center: bool, next: &CropBox) -> Option<CanvasView> {
    let press = drag.press_view?;
    let slid = || {
        let mut view = press;
        view.center = press.center + (next.center() - drag.at_press.center());
        view
    };
    Some(match drag.kind {
        Kind::Move => slid(),
        Kind::Resize(_) if auto_center => slid(),
        // The view first follows the frame's centre, which a turn kept inside the image may move, then turns about
        // it: the box stays put on screen.
        Kind::Turn { .. } => crop_view_about(next, &slid(), next.center()),
        Kind::Draw { .. } | Kind::Resize(_) | Kind::Line { .. } => press,
    })
}

/// A drawn frame turns the view upright on release, and with Auto Center Preview moves to the middle of the canvas.
pub(super) fn centred(drag: &Drag, auto_center: bool, view: &CanvasView, shown: &CropBox) -> Option<CanvasView> {
    if !matches!(drag.kind, Kind::Draw { .. }) || drag.press_view.is_none() {
        return None;
    }
    let mut upright = craft_behavior::geometry::crop_view(shown, view);
    if auto_center {
        upright.center = shown.center();
    }
    Some(upright)
}

/// The frame after an arrow key moved `part` by the screen vector `by`, or X swapped it, with the view image mode
/// moves along. `None` when the key changes nothing.
pub(super) fn nudged(frame: &CropBox, part: BoxPart, by: Vec2, view: &CanvasView, rules: &Rules) -> (CropBox, Option<CanvasView>) {
    let step = screen_step(view, by);
    if part == BoxPart::Inside && rules.image {
        // The image moves under the frame: the frame goes the other way and the view follows it.
        let next = frame.moved(-step);
        let mut moved = *view;
        moved.center += next.center() - frame.center();
        return (next, Some(moved));
    }
    (frame.nudged(part, step, rules.ratio), None)
}

/// The angle the Straighten readout shows for a line from `from` to `to` on screen: counter-clockwise, as a protractor
/// reads, from -180 (shown as 180) to 180. `None` for a point.
pub(super) fn line_degrees(from: Pos2, to: Pos2) -> Option<f64> {
    let d = to - from;
    if d == egui::Vec2::ZERO {
        return None;
    }
    let deg = normalized_angle(f64::from(-d.y).atan2(f64::from(d.x)).to_degrees());
    Some(if deg == -180.0 { 180.0 } else { deg })
}

/// The angle the turn readout shows: the frame's own, or the image's turn in image mode, never -0.
pub(super) fn turn_degrees(frame: &CropBox, image: bool) -> f64 {
    let turn = if image { image_turn(frame.angle) } else { normalized_angle(frame.angle) };
    (turn * 10.0).round() / 10.0 + 0.0
}

/// The direction of a document point from the frame's centre, in radians.
pub(super) fn grab(frame: &CropBox, from: Point) -> f64 {
    direction(frame.center(), from)
}
