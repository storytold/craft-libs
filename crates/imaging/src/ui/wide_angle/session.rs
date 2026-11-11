//! Adaptive Wide Angle's session: the correction and the constraints.

use serde::{Deserialize, Serialize};

use craft_behavior::kurbo::Point;

/// Correction: the lens model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum WideAngleModel {
    #[default]
    /// The app picks the model from the lens.
    Auto,
    /// A fisheye lens.
    Fisheye,
    /// A rectilinear wide-angle lens.
    Perspective,
    /// A full spherical panorama.
    FullSpherical,
}

impl WideAngleModel {
    /// The four models, in menu order.
    pub const ALL: [WideAngleModel; 4] = [WideAngleModel::Auto, WideAngleModel::Fisheye, WideAngleModel::Perspective, WideAngleModel::FullSpherical];
}

/// A constraint's orientation: Free (a straight line), Horizontal or Vertical.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ConstraintOrientation {
    #[default]
    /// A straight line in the scene.
    Free,
    /// A horizontal line in the scene.
    Horizontal,
    /// A vertical line in the scene.
    Vertical,
}

impl ConstraintOrientation {
    /// The three orientations, in menu order.
    pub const ALL: [ConstraintOrientation; 3] = [ConstraintOrientation::Free, ConstraintOrientation::Horizontal, ConstraintOrientation::Vertical];
}

/// A constraint: the end points of a line in the scene, in document pixels. Serialised as
/// PhotoCraft's `Constraint` (`a`, `b`, `orientation`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct WideAngleConstraint {
    /// One end point in document pixels.
    pub a: [f64; 2],
    /// The other end point in document pixels.
    pub b: [f64; 2],
    /// What the line is constrained to be.
    #[serde(default)]
    pub orientation: ConstraintOrientation,
}

impl WideAngleConstraint {
    /// A constraint between `a` and `b` (document pixels) of `orientation`.
    pub fn new(a: [f64; 2], b: [f64; 2], orientation: ConstraintOrientation) -> Self {
        Self { a, b, orientation }
    }

    /// A drag from `a` to `b` with Shift held at the release or not: Shift makes it Horizontal or
    /// Vertical by the larger of the two deltas (Horizontal on a tie), else Free.
    pub fn from_drag(a: Point, b: Point, shift: bool) -> WideAngleConstraint {
        // pc: crates/ui-egui/src/dialogs/wide_angle_ui.rs:300-305.
        let orientation = match shift {
            true if (b.x - a.x).abs() >= (b.y - a.y).abs() => ConstraintOrientation::Horizontal,
            true => ConstraintOrientation::Vertical,
            false => ConstraintOrientation::Free,
        };
        WideAngleConstraint { a: [a.x, a.y], b: [b.x, b.y], orientation }
    }
}

/// A change of the constraints.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum WideAngleConstraintEdit {
    /// Adds a constraint.
    Add(WideAngleConstraint),
    /// Sets the orientation of the constraint at `index`.
    #[non_exhaustive]
    Orient {
        /// The constraint's index.
        index: usize,
        /// The new orientation.
        orientation: ConstraintOrientation,
    },
    /// Deletes the constraint at that index.
    Delete(usize),
    /// Arrow keys moved the focused constraint by `by` document pixels.
    #[non_exhaustive]
    Move {
        /// The constraint's index.
        index: usize,
        /// The move in document pixels.
        by: [f64; 2],
    },
}

/// What OK applies (`filter.adaptiveWideAngle`'s parameters, serialised as PhotoCraft's
/// `WideAngle`) and the history snapshots. Defaults: Auto, focal length 0 (from the file's
/// metadata), crop factor 1, scale 100, no constraints. The Preview check is view state outside
/// it.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct WideAngleSession {
    /// The lens model.
    pub model: WideAngleModel,
    /// 0..=200 mm (35 mm frame after the crop factor).
    pub focal_length: f64,
    /// 0.1..=10.
    pub crop_factor: f64,
    /// 50..=150 %.
    pub scale: f64,
    /// The constraints, in order.
    pub constraints: Vec<WideAngleConstraint>,
}

impl Default for WideAngleSession {
    fn default() -> Self {
        // pc: crates/algo/src/wideangle.rs:83-87.
        Self { model: WideAngleModel::Auto, focal_length: 0.0, crop_factor: 1.0, scale: 100.0, constraints: Vec::new() }
    }
}

impl WideAngleSession {
    /// The session with `constraints`.
    #[must_use]
    pub fn with_constraints(mut self, constraints: Vec<WideAngleConstraint>) -> Self {
        self.constraints = constraints;
        self
    }

    /// Applies `edit`; an index out of range changes nothing. Returns whether it changed.
    pub fn apply(&mut self, edit: WideAngleConstraintEdit) -> bool {
        // pc: crates/ui-egui/src/dialogs/wide_angle_ui.rs:232 `constraint_menu`, :402-425.
        match edit {
            WideAngleConstraintEdit::Add(constraint) => {
                self.constraints.push(constraint);
                true
            }
            WideAngleConstraintEdit::Orient { index, orientation } => match self.constraints.get_mut(index) {
                Some(constraint) if constraint.orientation != orientation => {
                    constraint.orientation = orientation;
                    true
                }
                _ => false,
            },
            WideAngleConstraintEdit::Delete(index) => {
                index < self.constraints.len() && {
                    self.constraints.remove(index);
                    true
                }
            }
            WideAngleConstraintEdit::Move { index, by } => match self.constraints.get_mut(index) {
                Some(constraint) if by.iter().all(|d| d.is_finite()) && by != [0.0, 0.0] => {
                    for end in [&mut constraint.a, &mut constraint.b] {
                        end[0] += by[0];
                        end[1] += by[1];
                    }
                    true
                }
                _ => false,
            },
        }
    }
}

/// The app's lens geometry (lead call 44): the solver stays the app's (`wideangle::solve`).
pub trait WideAngleGeometry {
    /// The curve the scene line from `a` to `b` makes on the source image under `session`'s lens,
    /// as `segments + 1` document points (PhotoCraft: `Camera::arc(a, b, 24)`).
    fn arc(&self, session: &WideAngleSession, a: [f64; 2], b: [f64; 2], segments: usize) -> Vec<Point>;

    /// Where a source point lands in the corrected preview (the solved mesh's map), or `None`
    /// while no solve is ready.
    fn corrected(&self, session: &WideAngleSession, at: Point) -> Option<Point>;
}
