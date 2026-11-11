//! Select and Mask's session (what OK applies, with undo), its limits and its remembered settings
//!.

use craft_theme::Srgba;
use serde::{Deserialize, Serialize};

use crate::selection::ColorIndicates;
use crate::ui::mask_view::{MaskViewMode, MaskViewState};

use super::edits::{RefineStroke, SelectMaskEdit};

/// The engine's limits, given by the app: a stroke or edit that would pass them is refused when
/// it ends, with a message under the panel, and records nothing. PhotoCraft's: 4096 edits,
/// 100 000 points in all edits, 4096 Refine Edge Brush strokes, 100 000 points in all strokes
/// (`engine/src/select_mask_edits.rs:27-28`, `cmd/smartselect_cmds.rs:350-351`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SelectMaskLimits {
    /// Most tool edits.
    pub edits: usize,
    /// Most points over all tool edits.
    pub edit_points: usize,
    /// Most Refine Edge Brush strokes.
    pub strokes: usize,
    /// Most points over all Refine Edge Brush strokes.
    pub stroke_points: usize,
}

impl SelectMaskLimits {
    /// Limits of `edits` edits, `edit_points` points in all edits, `strokes` strokes and `stroke_points` points in all strokes.
    pub const fn new(edits: usize, edit_points: usize, strokes: usize, stroke_points: usize) -> Self {
        Self { edits, edit_points, strokes, stroke_points }
    }
}

/// Which limit refused an edit or a stroke, with the number it passed. The workspace words it
/// (`SelectMaskWords` through [`LimitRefused::message`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LimitRefused {
    /// At most this many edits.
    Edits(usize),
    /// At most this many points in all edits.
    EditPoints(usize),
    /// At most this many Refine Edge Brush strokes.
    Strokes(usize),
    /// At most this many points in all Refine Edge Brush strokes.
    StrokePoints(usize),
}

/// Output Settings › Output To.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum OutputTo {
    #[default]
    /// The selection.
    Selection,
    /// The layer mask.
    LayerMask,
    /// A new layer.
    NewLayer,
    /// A new layer with a layer mask.
    NewLayerWithMask,
    /// A new document.
    NewDocument,
    /// A new document with a layer mask.
    NewDocumentWithMask,
}

impl OutputTo {
    /// The six targets, in menu order.
    pub const ALL: [OutputTo; 6] = [
        OutputTo::Selection,
        OutputTo::LayerMask,
        OutputTo::NewLayer,
        OutputTo::NewLayerWithMask,
        OutputTo::NewDocument,
        OutputTo::NewDocumentWithMask,
    ];

    /// Whether the target is offered: Decontaminate Colors makes new pixels, so it allows only the
    /// new layer and new document targets.
    pub fn allowed(self, decontaminate: bool) -> bool {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:1625-1630.
        !decontaminate || !matches!(self, OutputTo::Selection | OutputTo::LayerMask)
    }
}

/// The refinement values of the Properties panel (Edge Detection, Global Refinements, Output
/// Settings). Part of the session (a slider drag is one undo step) and of the remembered settings.
/// Defaults: everything 0 and off, Amount 100, Output To Selection.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct RefineSettings {
    /// 0..=250 px.
    pub radius: f64,
    /// Smart Radius.
    pub smart_radius: bool,
    /// 0..=100.
    pub smooth: f64,
    /// 0..=1000 px.
    pub feather: f64,
    /// 0..=100 %.
    pub contrast: f64,
    /// −100..=100 %.
    pub shift_edge: f64,
    /// Decontaminate Colors.
    pub decontaminate: bool,
    /// 0..=100 %, enabled with Decontaminate Colors.
    pub amount: f64,
    /// Where the result goes.
    pub output: OutputTo,
}

impl Default for RefineSettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:177-185.
        Self {
            radius: 0.0,
            smart_radius: false,
            smooth: 0.0,
            feather: 0.0,
            contrast: 0.0,
            shift_edge: 0.0,
            decontaminate: false,
            amount: 100.0,
            output: OutputTo::Selection,
        }
    }
}

impl RefineSettings {
    /// Turning Decontaminate Colors on moves Output To from Selection or Layer Mask to New Layer
    /// with Mask.
    pub fn follow_decontaminate(&mut self) {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:266 `follow_decontaminate`.
        if self.decontaminate && matches!(self.output, OutputTo::Selection | OutputTo::LayerMask) {
            self.output = OutputTo::NewLayerWithMask;
        }
    }
}

/// What OK applies and the history snapshots: the tool edits, the Refine Edge Brush strokes and
/// the refinement values. The view, the tool and its options are outside it (risk 8): undo never
/// changes them.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SelectMaskSession {
    /// The tool edits, in order.
    pub edits: Vec<SelectMaskEdit>,
    /// The Refine Edge Brush strokes, in order.
    pub strokes: Vec<RefineStroke>,
    /// The refinement values.
    pub refine: RefineSettings,
}

/// `select.refineEdge`'s parameters, serialised as PhotoCraft's (`commit_params`): the refinement
/// values at the top level, `sampleAllLayers`, `edits` and `refineStrokes` when not empty,
/// `"target": "mask"` when a layer mask is refined.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SelectMaskCommand {
    /// The refinement values.
    #[serde(flatten)]
    pub refine: RefineSettings,
    /// Sample All Layers.
    pub sample_all_layers: bool,
    /// The tool edits, in order.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub edits: Vec<SelectMaskEdit>,
    /// The Refine Edge Brush strokes, in order.
    #[serde(rename = "refineStrokes", skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<RefineStroke>,
    /// `Some("mask")` when a layer mask is refined, else `None`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub target: Option<&'static str>,
}

impl SelectMaskCommand {
    /// Parameters for `refine` with no edits, no strokes and no mask target.
    pub fn new(refine: RefineSettings, sample_all_layers: bool) -> Self {
        Self { refine, sample_all_layers, edits: Vec::new(), strokes: Vec::new(), target: None }
    }
}

impl SelectMaskSession {
    /// Adds a tool edit within `limits`, or returns which limit refused it and changes nothing.
    pub fn add_edit(&mut self, edit: SelectMaskEdit, limits: &SelectMaskLimits) -> Result<(), LimitRefused> {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:522 `add_edit`.
        if self.edits.len() >= limits.edits {
            return Err(LimitRefused::Edits(limits.edits));
        }
        let held: usize = self.edits.iter().map(SelectMaskEdit::points_len).sum();
        if held + edit.points_len() > limits.edit_points {
            return Err(LimitRefused::EditPoints(limits.edit_points));
        }
        self.edits.push(edit);
        Ok(())
    }

    /// Adds a Refine Edge Brush stroke within `limits`, or returns which limit refused it and
    /// changes nothing.
    pub fn add_stroke(&mut self, stroke: RefineStroke, limits: &SelectMaskLimits) -> Result<(), LimitRefused> {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:538 `add_stroke`.
        if self.strokes.len() >= limits.strokes {
            return Err(LimitRefused::Strokes(limits.strokes));
        }
        let held: usize = self.strokes.iter().map(|s| s.points.len()).sum();
        if held + stroke.points.len() > limits.stroke_points {
            return Err(LimitRefused::StrokePoints(limits.stroke_points));
        }
        self.strokes.push(stroke);
        Ok(())
    }

    /// The Reset button: no edits, no strokes, the default refinement values. One undo step that
    /// gives back what it replaced. (PhotoCraft's also reset the view; the view is not session
    /// state here.)
    pub fn reset(&mut self) {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:701 `reset`.
        *self = Self::default();
    }

    /// OK's parameters with the bar's Sample All Layers and whether a layer mask is refined.
    pub fn command(&self, sample_all_layers: bool, target_mask: bool) -> SelectMaskCommand {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:803 `commit_params`.
        SelectMaskCommand {
            refine: self.refine.clone(),
            sample_all_layers,
            edits: self.edits.clone(),
            strokes: self.strokes.clone(),
            target: target_mask.then_some("mask"),
        }
    }
}

/// Select and Mask's remembered settings (PhotoCraft `SmOpts`, `select.refineEdge.workspace`): the
/// view's choices, Real-Time Refinement, High Quality Preview, the refinement values, Remember
/// Settings. The app persists them when `remember` is on (serde, every field defaulted). Defaults:
/// Onion Skin, the views' default opacities, edge and original off, Real-Time Refinement on, High
/// Quality off, overlay red (255, 0, 0) indicating masked areas, [`RefineSettings::default`],
/// Remember off.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct SelectMaskSettings {
    /// The selected view.
    pub view: MaskViewMode,
    /// Each view's Transparency or Opacity (percent), in [`MaskViewMode::ALL`] order.
    pub opacities: [f32; 7],
    /// Show Edge.
    pub show_edge: bool,
    /// Show Original.
    pub show_original: bool,
    /// The overlay tint as sRGB bytes.
    pub overlay_color: [u8; 3],
    /// Whether the overlay marks masked or selected areas.
    pub overlay_indicates: ColorIndicates,
    /// The stroke being drawn joins the preview while the pointer moves.
    pub real_time: bool,
    /// The preview runs on the document at full size (reduction 1).
    pub high_quality: bool,
    /// The refinement values.
    pub refine: RefineSettings,
    /// Remember Settings: the app persists these settings while it is on.
    pub remember: bool,
}

impl Default for SelectMaskSettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/select_mask_ui.rs:166-188 `SmOpts::default`; the
        // opacities from `MaskViewMode::default_opacity` (100 where a view has none), which
        // `MaskViewState::default` already takes.
        let view = MaskViewState::default();
        let [r, g, b, _] = view.overlay_color.to_rgba().0;
        Self {
            view: view.mode,
            opacities: view.opacities,
            show_edge: view.show_edge,
            show_original: view.show_original,
            overlay_color: [r, g, b],
            overlay_indicates: view.overlay_indicates,
            real_time: true,
            high_quality: false,
            refine: RefineSettings::default(),
            remember: false,
        }
    }
}

impl SelectMaskSettings {
    /// The view state the workspace starts with (`hide_view` off).
    pub fn view_state(&self) -> MaskViewState {
        // new: the settings carry `MaskViewState`'s persisted fields (it has no serde).
        let [r, g, b] = self.overlay_color.map(|v| f32::from(v) / 255.0);
        MaskViewState {
            mode: self.view,
            opacities: self.opacities,
            show_edge: self.show_edge,
            show_original: self.show_original,
            hide_view: false,
            overlay_color: Srgba::rgb(r, g, b),
            overlay_indicates: self.overlay_indicates,
        }
    }

    /// Takes the view's persisted fields from `state`.
    pub fn take_view(&mut self, state: &MaskViewState) {
        // new: the inverse of `view_state`.
        let [r, g, b, _] = state.overlay_color.to_rgba().0;
        self.view = state.mode;
        self.opacities = state.opacities;
        self.show_edge = state.show_edge;
        self.show_original = state.show_original;
        self.overlay_color = [r, g, b];
        self.overlay_indicates = state.overlay_indicates;
    }
}
