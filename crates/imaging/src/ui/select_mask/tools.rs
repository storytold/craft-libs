//! Select and Mask's tools, their options and the canvas tool.

mod canvas;

pub use canvas::{SelectMaskCanvas, SelectMaskCanvasTool};

use craft_behavior::{CancelReason, GestureId};
use craft_ui::rails::{RailTool, ToolSlot};

use super::commands::SelectMaskCommands;

/// What a tool edit or stroke reports as it goes: the gesture's lifecycle with the value built from it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum EditPhase<T> {
    /// The gesture began. The value is the first point's edit, when it makes one.
    #[non_exhaustive]
    Begin {
        /// The gesture.
        id: GestureId,
        /// The edit at the press.
        value: Option<T>,
    },
    /// The gesture grew (only with Real-Time Refinement on): the edit so far.
    #[non_exhaustive]
    Update {
        /// The gesture.
        id: GestureId,
        /// The edit so far.
        value: T,
    },
    /// The gesture ended with this edit. The workspace has already recorded it in the session.
    #[non_exhaustive]
    Commit {
        /// The gesture.
        id: GestureId,
        /// The finished edit.
        value: T,
    },
    /// The gesture ended without an edit: Esc, a lost pointer, a tool switch, an undo, or the limits refused it.
    #[non_exhaustive]
    Cancel {
        /// The gesture.
        id: GestureId,
        /// Why it ended.
        reason: CancelReason,
    },
    /// A button made this edit at once (Select Subject, Clear Selection, Invert).
    #[non_exhaustive]
    Set {
        /// The edit's id.
        id: GestureId,
        /// The edit.
        value: T,
    },
}

impl<T> EditPhase<T> {
    /// The gesture began, with the edit at the press when it makes one.
    pub fn begin(id: GestureId, value: Option<T>) -> Self {
        Self::Begin { id, value }
    }

    /// The gesture grew: the edit so far.
    pub fn update(id: GestureId, value: T) -> Self {
        Self::Update { id, value }
    }

    /// The gesture ended with `value`.
    pub fn commit(id: GestureId, value: T) -> Self {
        Self::Commit { id, value }
    }

    /// The gesture ended without an edit, for `reason`.
    pub fn cancel(id: GestureId, reason: CancelReason) -> Self {
        Self::Cancel { id, reason }
    }

    /// A button made `value` at once.
    pub fn set(id: GestureId, value: T) -> Self {
        Self::Set { id, value }
    }
}

/// The tools of Select and Mask's strip, with Photoshop's letters. Quick Selection and Object
/// Selection share the W slot, Lasso and Polygonal Lasso the L slot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SelectMaskTool {
    /// W.
    #[default]
    Quick,
    /// R.
    RefineEdge,
    /// B.
    Brush,
    /// W (Shift+W cycles with Quick Selection).
    Object,
    /// L.
    Lasso,
    /// L (Shift+L cycles with Lasso).
    PolygonalLasso,
    /// H.
    Hand,
    /// Z.
    Zoom,
}

impl SelectMaskTool {
    /// The eight tools, in strip order.
    pub const ALL: [SelectMaskTool; 8] = [
        SelectMaskTool::Quick,
        SelectMaskTool::RefineEdge,
        SelectMaskTool::Brush,
        SelectMaskTool::Object,
        SelectMaskTool::Lasso,
        SelectMaskTool::PolygonalLasso,
        SelectMaskTool::Hand,
        SelectMaskTool::Zoom,
    ];

    /// Paints with a round brush whose size `[` and `]` change: Quick Selection, Refine Edge
    /// Brush, Brush.
    pub fn brushes(self) -> bool {
        matches!(self, SelectMaskTool::Quick | SelectMaskTool::RefineEdge | SelectMaskTool::Brush)
    }

    /// Adds to or subtracts from the selection (the bar's + and −): all but Hand and Zoom.
    pub fn edits(self) -> bool {
        !matches!(self, SelectMaskTool::Hand | SelectMaskTool::Zoom)
    }

    /// The strip: slot W (Quick Selection, Object Selection), R, B, slot L (Lasso, Polygonal
    /// Lasso; `ending_group`), H, Z. Each tool runs its command, which gives its label, icon and letter.
    pub fn slots() -> Vec<ToolSlot<SelectMaskTool>> {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1845-1868 (the strip, the shared L
        // slot with its corner triangle and menu), :101 `icon`.
        let tools = |tools: &[SelectMaskTool]| tools.iter().map(|tool| RailTool::new(*tool, SelectMaskCommands::of(*tool))).collect::<Vec<_>>();
        vec![
            ToolSlot::new(0, tools(&[SelectMaskTool::Quick, SelectMaskTool::Object])),
            ToolSlot::new(1, tools(&[SelectMaskTool::RefineEdge])),
            ToolSlot::new(2, tools(&[SelectMaskTool::Brush])),
            ToolSlot::ending_group(3, tools(&[SelectMaskTool::Lasso, SelectMaskTool::PolygonalLasso])),
            ToolSlot::new(4, tools(&[SelectMaskTool::Hand])),
            ToolSlot::new(5, tools(&[SelectMaskTool::Zoom])),
        ]
    }
}

/// The diameters the three brush tools remember apart (1..=5000; 30 each when the workspace
/// opens). Not in the history, not persisted.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushSizes {
    /// The Quick Selection diameter.
    pub quick: f64,
    /// The Refine Edge Brush diameter.
    pub refine: f64,
    /// The Brush diameter.
    pub brush: f64,
}

impl Default for BrushSizes {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:749-751.
        Self { quick: 30.0, refine: 30.0, brush: 30.0 }
    }
}

impl BrushSizes {
    /// The sizes' range.
    pub const RANGE: std::ops::RangeInclusive<f64> = 1.0..=5000.0;

    /// The size of `tool` (the Brush's for a tool without its own).
    pub fn get(&self, tool: SelectMaskTool) -> f64 {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:467 `size`.
        match tool {
            SelectMaskTool::Quick => self.quick,
            SelectMaskTool::RefineEdge => self.refine,
            _ => self.brush,
        }
    }

    /// Sets the size of a brush tool, rounded and clamped to [`Self::RANGE`]; a non-finite value
    /// or another tool changes nothing.
    pub fn set(&mut self, tool: SelectMaskTool, size: f64) {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:475 `set_size`.
        if !size.is_finite() {
            return;
        }
        let size = size.round().clamp(*Self::RANGE.start(), *Self::RANGE.end());
        match tool {
            SelectMaskTool::Quick => self.quick = size,
            SelectMaskTool::RefineEdge => self.refine = size,
            SelectMaskTool::Brush => self.brush = size,
            _ => {}
        }
    }

    /// `[` (`larger` false) and `]`: × 0.9 or × 1.1, at least one pixel of change.
    pub fn step(&mut self, tool: SelectMaskTool, larger: bool) {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1405-1412.
        let size = self.get(tool);
        self.set(tool, if larger { (size * 1.1).max(size + 1.0) } else { (size * 0.9).min(size - 1.0) });
    }
}

/// The tools' options: the bar's values, outside the history and not persisted.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SelectMaskToolOptions {
    /// + (true) or − in the bar.
    pub add: bool,
    /// The diameters of the three brush tools.
    pub sizes: BrushSizes,
    /// The Brush's hardness, 0..=100 (100).
    pub hardness: f64,
    /// Sample All Layers (Quick Selection, Refine Edge Brush, Object Selection, Select Subject).
    pub sample_all_layers: bool,
}

impl Default for SelectMaskToolOptions {
    fn default() -> Self {
        Self { add: true, sizes: BrushSizes::default(), hardness: 100.0, sample_all_layers: false }
    }
}
