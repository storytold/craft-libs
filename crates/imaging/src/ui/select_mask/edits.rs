//! Select and Mask's edits and Refine Edge Brush strokes as typed data.

use serde::{Deserialize, Serialize};

/// Whether an edit adds to the selection or subtracts from it: the options bar's + and −; Alt at
/// the press gives the other.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum EditMode {
    #[default]
    /// Adds to the selection.
    Add,
    /// Subtracts from the selection.
    Subtract,
}

/// What Object Selection picks: the object under a click (a drag shorter than 4 screen points), or
/// the one inside a dragged rect `[x, y, width, height]`. Serialised as PhotoCraft's `point` or
/// `rect` key.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ObjectTarget {
    /// The object under this document point.
    Point([f64; 2]),
    /// The object inside this rectangle `[x, y, width, height]` in document pixels.
    Rect([f64; 4]),
}

/// One tool edit, in the order the engine applies them (`select.refineEdge`'s `edits`). Points are
/// document pixels rounded to a tenth. Serialised as PhotoCraft's JSON edits
/// (`{"tool": "quick", "points": …, "size": …, "mode": "add", "sampleAllLayers": false}`), so the
/// app passes [`SelectMaskCommand`](super::SelectMaskCommand) to its engine as it is.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "tool", rename_all = "camelCase")]
#[non_exhaustive]
pub enum SelectMaskEdit {
    /// A Quick Selection stroke.
    #[non_exhaustive]
    Quick {
        /// The stroke or outline in document pixels.
        points: Vec<[f64; 2]>,
        /// The brush diameter in document pixels.
        size: f64,
        /// Add or subtract.
        mode: EditMode,
        #[serde(rename = "sampleAllLayers")]
        /// Samples all layers, not only the active one.
        sample_all_layers: bool,
    },
    /// A Brush stroke.
    #[non_exhaustive]
    Brush {
        /// The stroke or outline in document pixels.
        points: Vec<[f64; 2]>,
        /// The brush diameter in document pixels.
        size: f64,
        /// The brush hardness, 0 to 100.
        hardness: f64,
        /// Add or subtract.
        mode: EditMode,
    },
    /// A Lasso or Polygonal Lasso polygon (3 points or more).
    #[non_exhaustive]
    Lasso {
        /// The stroke or outline in document pixels.
        points: Vec<[f64; 2]>,
        /// Add or subtract.
        mode: EditMode,
    },
    /// An Object Selection click or rect.
    #[non_exhaustive]
    Object {
        #[serde(flatten)]
        /// The point or rectangle.
        target: ObjectTarget,
        /// Add or subtract.
        mode: EditMode,
        #[serde(rename = "sampleAllLayers")]
        /// Samples all layers, not only the active one.
        sample_all_layers: bool,
    },
    /// Select Subject.
    #[non_exhaustive]
    Subject {
        #[serde(rename = "sampleAllLayers")]
        /// Samples all layers, not only the active one.
        sample_all_layers: bool,
    },
    /// Clear Selection.
    Clear,
    /// Invert.
    Invert,
}

impl SelectMaskEdit {
    /// A Quick Selection stroke.
    pub fn quick(points: Vec<[f64; 2]>, size: f64, mode: EditMode, sample_all_layers: bool) -> Self {
        Self::Quick { points, size, mode, sample_all_layers }
    }

    /// A Brush stroke.
    pub fn brush(points: Vec<[f64; 2]>, size: f64, hardness: f64, mode: EditMode) -> Self {
        Self::Brush { points, size, hardness, mode }
    }

    /// A Lasso or Polygonal Lasso polygon.
    pub fn lasso(points: Vec<[f64; 2]>, mode: EditMode) -> Self {
        Self::Lasso { points, mode }
    }

    /// An Object Selection click or rect.
    pub fn object(target: ObjectTarget, mode: EditMode, sample_all_layers: bool) -> Self {
        Self::Object { target, mode, sample_all_layers }
    }

    /// Select Subject.
    pub fn subject(sample_all_layers: bool) -> Self {
        Self::Subject { sample_all_layers }
    }

    /// The points the edit carries, against `SelectMaskLimits::edit_points` (an Object edit and
    /// the buttons carry none).
    pub fn points_len(&self) -> usize {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:510 `points_in`.
        match self {
            SelectMaskEdit::Quick { points, .. } | SelectMaskEdit::Brush { points, .. } | SelectMaskEdit::Lasso { points, .. } => points.len(),
            SelectMaskEdit::Object { .. } | SelectMaskEdit::Subject { .. } | SelectMaskEdit::Clear | SelectMaskEdit::Invert => 0,
        }
    }
}

/// A Refine Edge Brush stroke: kept apart from the edits (`select.refineEdge`'s `refineStrokes`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct RefineStroke {
    /// The brush path in document pixels, rounded to a tenth.
    pub points: Vec<[f64; 2]>,
    /// The brush diameter in document pixels.
    pub size: f64,
    /// Whether the stroke adds to the refined edge or subtracts from it.
    pub mode: EditMode,
}

impl RefineStroke {
    /// A stroke of diameter `size` along `points` that adds or subtracts per `mode`.
    pub fn new(points: Vec<[f64; 2]>, size: f64, mode: EditMode) -> Self {
        Self { points, size, mode }
    }
}

/// A point rounded to a tenth of a pixel, as edits store them.
pub fn round_point(at: [f64; 2]) -> [f64; 2] {
    // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:555 `round`.
    [(at[0] * 10.0).round() / 10.0, (at[1] * 10.0).round() / 10.0]
}
