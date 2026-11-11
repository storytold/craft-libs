//! What a pointer drag does to the crop frame: the kinds of drag, the frame each step gives, and how image mode moves
//! the view with it.

use craft_behavior::CanvasView;
use craft_behavior::geometry::crop::{direction, drawn, image_turn, normalized_angle};
use craft_behavior::geometry::{BoxPart, CropBox, SNAP_PX, SnapLine, SnapTargets, crop_view_about, screen_step};
use craft_behavior::kurbo::{Point, Rect, Vec2};
use craft_keymap::Mods;
use egui::Pos2;

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
}

/// The rules every step of a drag follows.
#[derive(Clone, Copy)]
pub(super) struct Rules<'a> {
    pub ratio: Option<f64>,
    pub bounds: Option<Rect>,
    pub turn_step: f64,
    pub image: bool,
    pub auto_center: bool,
    pub snap: Option<&'a SnapTargets>,
}

impl Rules<'_> {
    /// The frame a drag gives with the pointer at document point `to`, kept inside the bounds; `None` when the frame is
    /// as it was (a draw not yet sized) or cannot be kept inside.
    pub fn proposal(&self, drag: &mut Drag, to: Point, mods: Mods, space: bool) -> Option<CropBox> {
        let at = drag.at_press;
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
            Kind::Move if self.image => (at.moved(drag.from - to), false, false),
            Kind::Move => (at.moved(to - drag.from), false, false),
            Kind::Resize(part) => {
                let r = at.rect.abs();
                let keep = (r.width() > 0.0 && r.height() > 0.0).then(|| r.width() / r.height());
                let ratio = self.ratio.or(if mods.shift { keep } else { None });
                let by = to - drag.from;
                let next = if self.image && self.auto_center {
                    at.resized_centred(*part, by, ratio, mods.alt)?
                } else {
                    at.resized(*part, by, ratio, mods.alt)
                };
                (next, true, ratio.is_some())
            }
            Kind::Turn { grab } => {
                let step = mods.shift.then_some(self.turn_step);
                (at.turned_to(*grab, to, step, self.image), true, false)
            }
            Kind::Line { .. } => return None,
        };
        if !next.is_finite() {
            return None;
        }
        match self.bounds {
            Some(bounds) => next.within(bounds, resizing, held).or(Some(drag.last)),
            None => Some(next),
        }
    }

    /// The screen point of `pos` after the snap targets: an end of the Straighten line.
    pub fn snap_screen(&self, view: &CanvasView, pos: Pos2) -> Pos2 {
        let Some(targets) = self.snap else { return pos };
        let (point, _) = targets.snap_point(view.to_doc(pos), view.doc_len(SNAP_PX as f32));
        view.to_screen(point)
    }

    /// The drag's document point for the pointer at `to` after the host's snap targets, with the lines that snapped. In
    /// image mode the point is in the view of the press, which the moving view never feeds back into: the correction
    /// is turned back into pointer travel.
    pub fn snapped(&self, drag: &Drag, view: &CanvasView, to: Point, mods: Mods, space: bool) -> (Point, Vec<SnapLine>) {
        let Some(targets) = self.snap else { return (to, Vec::new()) };
        let tolerance = drag.press_view.as_ref().unwrap_or(view).doc_len(SNAP_PX as f32);
        let mut probe = drag.clone();
        let Some(candidate) = self.proposal(&mut probe, to, mods, space) else { return (to, Vec::new()) };
        let (by, lines, travel_per_unit) = match probe.kind {
            Kind::Draw { cur, .. } => {
                let (snapped, lines) = targets.snap_point(cur, tolerance);
                (snapped - cur, lines, 1.0)
            }
            Kind::Move => {
                let (by, lines) = targets.snap_rect(candidate.turned_bounds(), tolerance);
                (by, lines, if self.image { -1.0 } else { 1.0 })
            }
            Kind::Resize(part) => {
                let Some(point) = candidate.part_point(part) else { return (to, Vec::new()) };
                let (snapped, lines) = targets.snap_point(point, tolerance);
                (snapped - point, lines, if self.image && self.auto_center && !mods.alt { 0.5 } else { 1.0 })
            }
            Kind::Turn { .. } | Kind::Line { .. } => return (to, Vec::new()),
        };
        if by.x.is_finite() && by.y.is_finite() { (to + by * travel_per_unit, lines) } else { (to, Vec::new()) }
    }
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
        Kind::Turn { .. } => crop_view_about(next, &press, drag.at_press.center()),
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
pub(super) fn nudged(frame: &CropBox, part: BoxPart, by: Vec2, view: &CanvasView, rules: &Rules<'_>) -> (CropBox, Option<CanvasView>) {
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
