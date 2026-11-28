//! Adaptive Wide Angle's session, egui-free: the lens model, the constraints and what OK applies. The app's develop
//! reads it without craft-imaging's `ui`; `ui::wide_angle` re-exports it.

use serde::{Deserialize, Serialize};

/// Correction: the lens model.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
}
