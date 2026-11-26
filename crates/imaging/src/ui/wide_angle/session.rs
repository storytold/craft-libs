//! Adaptive Wide Angle's session: the correction and the constraints.

use craft_behavior::kurbo::Point;

pub use crate::wide_angle::{ConstraintOrientation, WideAngleConstraint, WideAngleModel, WideAngleSession};

impl WideAngleConstraint {
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

impl WideAngleSession {
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
