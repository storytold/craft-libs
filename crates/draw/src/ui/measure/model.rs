//! [`RulerLine`]: the Ruler tool's line with its optional protractor arm, and its
//! [`RulerReading`]: the numbers the options bar and AccessKit show. No egui.

use kurbo::Point;

/// A new line shorter than this, in document units, is cleared on release (PhotoCraft's 0.5).
pub const MIN_LINE: f64 = 0.5;

/// The Ruler line in document units: from `start` to `end`, and with a protractor a second arm
/// from `start` (the vertex) to `protractor`.
/// Serialises camelCase, points as kurbo's: the app stores the line in its document.
#[derive(Clone, Copy, Debug, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RulerLine {
    /// The first end, and the protractor's vertex.
    pub start: Point,
    /// The second end of the main arm.
    pub end: Point,
    /// The free end of the protractor's second arm, when there is one.
    pub protractor: Option<Point>,
}

/// A part of the line a press grabs and the keys move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RulerPart {
    /// The line's start.
    Start,
    /// The line's end.
    End,
    /// The protractor arm's free end.
    Protractor,
}

impl RulerPart {
    /// The part's index in the overlay's part group: 0, 1, 2 in keyboard order.
    pub(super) const fn key(self) -> u64 {
        match self {
            RulerPart::Start => 0,
            RulerPart::End => 1,
            RulerPart::Protractor => 2,
        }
    }

    /// The part keyed `key`.
    pub(super) fn of_key(key: u64) -> Option<RulerPart> {
        match key {
            0 => Some(RulerPart::Start),
            1 => Some(RulerPart::End),
            2 => Some(RulerPart::Protractor),
            _ => None,
        }
    }
}

/// What the Ruler measures, as the engine's `ruler_info` gives it: X and Y of the start, W and H
/// (end minus start, signed), the angle A and the lengths L1 and L2.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct RulerReading {
    /// X of the start.
    pub x: f64,
    /// Y of the start.
    pub y: f64,
    /// End minus start, signed, along x.
    pub w: f64,
    /// End minus start, signed, along y.
    pub h: f64,
    /// Degrees. A line: counter-clockwise from +x with y up, -180 to 180. With a protractor: the
    /// angle between the two arms, 0 to 180.
    pub angle: f64,
    /// The first arm's length.
    pub l1: f64,
    /// The protractor arm's length; `None` without a protractor.
    pub l2: Option<f64>,
}

impl RulerLine {
    /// A line from `start` to `end` without a protractor.
    pub fn new(start: Point, end: Point) -> Self {
        Self { start, end, protractor: None }
    }

    /// The line with a protractor arm from `start` to `protractor`.
    #[must_use]
    pub fn with_protractor(mut self, protractor: Point) -> Self {
        self.protractor = Some(protractor);
        self
    }

    /// Whether every point is finite.
    pub fn is_finite(&self) -> bool {
        self.start.is_finite() && self.end.is_finite() && self.protractor.is_none_or(Point::is_finite)
    }

    /// The point of `part`; `None` for the protractor of a line without one.
    pub fn point(&self, part: RulerPart) -> Option<Point> {
        match part {
            RulerPart::Start => Some(self.start),
            RulerPart::End => Some(self.end),
            RulerPart::Protractor => self.protractor,
        }
    }

    /// The part a press at `at` grabs within `reach` document units: the protractor's end first,
    /// then the end, then the start (PhotoCraft's order); `None` elsewhere.
    pub fn part_at(&self, at: Point, reach: f64) -> Option<RulerPart> {
        // pc: crates/ui-egui/src/tools/analysis_ui.rs:250 (the Down branch's match order).
        let near = |p: Point| p.distance(at) <= reach;
        if self.protractor.is_some_and(near) {
            Some(RulerPart::Protractor)
        } else if near(self.end) {
            Some(RulerPart::End)
        } else if near(self.start) {
            Some(RulerPart::Start)
        } else {
            None
        }
    }

    /// The numbers of the line. The engine's `ruler_info` duplicates them on PhotoCraft's side
    /// (lead call 19); a PhotoCraft test compares the two.
    pub fn reading(&self) -> RulerReading {
        // pc: crates/engine/src/cmd/analysis_cmds.rs:614 `ruler_info` (without the rounding to 4
        // decimals and the scale, which `RulerReading::scaled` is).
        let d = self.end - self.start;
        let l1 = d.hypot();
        let (angle, l2) = match self.protractor {
            None => ((-d.y).atan2(d.x).to_degrees(), None),
            Some(q) => {
                let e = q - self.start;
                let mut angle = ((-d.y).atan2(d.x) - (-e.y).atan2(e.x)).to_degrees().abs();
                if angle > 180.0 {
                    angle = 360.0 - angle;
                }
                (angle, Some(e.hypot()))
            }
        };
        RulerReading { x: self.start.x, y: self.start.y, w: d.x, h: d.y, angle, l1, l2 }
    }
}

impl RulerReading {
    /// Use Measurement Scale: X, Y, W, H, L1 and L2 multiplied by `factor`; the angle unchanged.
    pub fn scaled(self, factor: f64) -> RulerReading {
        // pc: crates/ui-egui/src/tools/analysis_ui.rs:497 `options_bar` (the Ruler branch's `f`).
        RulerReading {
            x: self.x * factor,
            y: self.y * factor,
            w: self.w * factor,
            h: self.h * factor,
            angle: self.angle,
            l1: self.l1 * factor,
            l2: self.l2.map(|l| l * factor),
        }
    }
}
