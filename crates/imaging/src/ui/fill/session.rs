//! Content-Aware Fill's session (what OK and Apply fill with, with undo), its limits and its
//! remembered settings.

use serde::{Deserialize, Serialize};

use craft_behavior::CancelReason;
use craft_ui::runtime::env::Env;

use super::lasso::FillAreaEdit;
use super::words::FillWords;
use crate::ui::select_mask::EditMode;

/// Sampling Area Options: how the sampling area starts before the brush strokes. Auto and
/// Rectangular start full (the brush subtracts), Custom starts empty (the brush adds).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum SamplingMode {
    #[default]
    /// Auto.
    Auto,
    /// Rectangular.
    Rectangular,
    /// Custom.
    Custom,
}

impl SamplingMode {
    /// The three modes, in menu order.
    pub const ALL: [SamplingMode; 3] = [SamplingMode::Auto, SamplingMode::Rectangular, SamplingMode::Custom];

    /// Whether the Sampling Brush adds in this mode (Custom) or subtracts (Auto, Rectangular).
    pub fn brush_adds(self) -> bool {
        self == SamplingMode::Custom
    }
}

/// Fill Settings › Color Adaptation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum ColorAdaptation {
    /// None.
    None,
    #[default]
    /// Default.
    Default,
    /// High.
    High,
    /// Very high.
    VeryHigh,
}

/// Fill Settings › Rotation Adaptation.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum RotationAdaptation {
    #[default]
    /// None.
    None,
    /// Low.
    Low,
    /// Medium.
    Medium,
    /// High.
    High,
    /// Full.
    Full,
}

/// Output Settings › Output To. Photoshop opens on New Layer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum FillOutputTo {
    #[serde(rename = "current")]
    /// Current layer.
    CurrentLayer,
    #[default]
    #[serde(rename = "new")]
    /// New layer.
    NewLayer,
    #[serde(rename = "duplicate")]
    /// Duplicate layer.
    DuplicateLayer,
}

/// A Sampling Brush stroke: a hard round brush that adds to or subtracts from the sampling area.
/// Serialised as PhotoCraft's `samplingStrokes` entries (`{"mode": "add", "size": …, "points":
/// …}`, `caf_ui.rs:216` `stroke_json`).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct SamplingStroke {
    /// Whether the stroke adds to or subtracts from the sampling area.
    pub mode: EditMode,
    /// Diameter in document pixels.
    pub size: f64,
    /// The brush path in document pixels.
    pub points: Vec<[f64; 2]>,
}

/// The engine's limits, given by the app: PhotoCraft's `MAX_CAF_EDITS` (1000) strokes, and as many
/// lasso edits, and `MAX_CAF_POINTS` (200 000) points in all
/// (`engine/src/cmd/edit_menu_cmds/content_aware.rs:105`, `:107`). The brush stops there, so
/// Apply and the preview never fail on what was painted.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct FillLimits {
    /// Most strokes, and separately most lasso edits.
    pub edits: usize,
    /// Most points over all strokes.
    pub points: usize,
}

/// The limit a stroke or edit ran into, with the number it is. The workspace words it
/// ([`FillLimit::message`]) under the panel; the app gets it in [`FillEvent::Refused`].
///
/// [`FillEvent::Refused`]: super::FillEvent::Refused
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum FillLimit {
    /// At most this many sampling strokes.
    Strokes(usize),
    /// At most this many lasso edits.
    LassoEdits(usize),
    /// At most this many points in all strokes and lasso edits.
    Vertices(usize),
}

impl FillLimit {
    /// The limit as a sentence in the installed language ("at most 2 sampling strokes: undo some,
    /// or reset the sampling area"), the count in the locale's digits and plural form.
    pub fn message(&self, env: &Env) -> String {
        // pc: crates/ui-egui/src/dialogs/caf_ui/preview.rs:84 `points_limit`, caf_ui.rs:353
        // `begin_stroke`, caf_ui/lifecycle.rs:196 `fill_edit` (the texts).
        let words = env.words::<FillWords>();
        let (counted, n) = match *self {
            FillLimit::Strokes(n) => (&words.stroke_limit, n),
            FillLimit::LassoEdits(n) => (&words.lasso_edit_limit, n),
            FillLimit::Vertices(n) => (&words.point_limit, n),
        };
        counted.text(n as u64, &env.format().integer(n as u64))
    }
}

/// What the Sampling Brush reports about a stroke. `Update` carries only the points added since the last event, so
/// the cost of a stroke stays linear.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum FillStrokeEvent {
    /// The press: the stroke with its first point.
    Begin(SamplingStroke),
    /// New points: the stroke's mode and size with the points added since the last event.
    Update(SamplingStroke),
    /// The release: the whole stroke, now recorded in the session.
    Commit(SamplingStroke),
    /// The stroke was dropped (Esc, a lost pointer, a tool switch, an undo): the app takes back what it painted.
    Cancel(CancelReason),
}

/// What a Reset button resets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum FillReset {
    /// Sampling Area Options' Reset: Auto, Sample All Layers off, no strokes.
    Sampling,
    /// Fill Settings' Reset: the adaptation, Scale and Mirror to their defaults (not undoable,
    /// as PhotoCraft).
    Fill,
    /// The Lasso's Reset in the options bar: no fill-area edits.
    FillArea,
    /// The footer's Reset: everything, the settings included.
    All,
}

/// What OK and Apply fill with and the history snapshots: the sampling option, the brush strokes
/// and the lasso edits of the fill area. The tool, the brush size and the overlay are outside it
/// (risk 8).
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct FillSession {
    /// How the sampling area starts.
    pub sampling: SamplingMode,
    /// The Sampling Brush strokes, in order.
    pub strokes: Vec<SamplingStroke>,
    /// The fill-area edits, in order.
    pub fill_edits: Vec<FillAreaEdit>,
}

impl FillSession {
    /// Points in the strokes and lasso edits so far, against [`FillLimits::points`]; `extra` is a
    /// stroke in progress.
    pub fn points_used(&self, extra: Option<&SamplingStroke>) -> usize {
        // pc: crates/ui-egui/src/dialogs/caf_ui.rs:342 `points_used`.
        let strokes: usize = self.strokes.iter().chain(extra).map(|stroke| stroke.points.len()).sum();
        let lassos: usize = self.fill_edits.iter().map(FillAreaEdit::points_len).sum();
        strokes + lassos
    }

    /// Whether a new stroke may begin: `Err` with the stroke limit or the points limit it ran into.
    pub fn may_begin_stroke(&self, limits: &FillLimits) -> Result<(), FillLimit> {
        // pc: crates/ui-egui/src/dialogs/caf_ui.rs:353 `begin_stroke` (the checks), :371
        // `extend_stroke`; caf_ui/preview.rs:84 `points_limit`.
        if self.strokes.len() >= limits.edits {
            return Err(FillLimit::Strokes(limits.edits));
        }
        if self.points_used(None) >= limits.points {
            return Err(FillLimit::Vertices(limits.points));
        }
        Ok(())
    }

    /// Records a finished stroke.
    pub fn add_stroke(&mut self, stroke: SamplingStroke) {
        // pc: crates/ui-egui/src/dialogs/caf_ui.rs:389 `end_stroke`.
        self.strokes.push(stroke);
    }

    /// Records a fill-area edit within `limits`, or returns the limit it ran into. An edit the app
    /// then finds empties the fill area is taken back by the workspace
    /// (`ContentAwareFillWorkspace::refuse_fill_edit`).
    pub fn add_fill_edit(&mut self, edit: FillAreaEdit, limits: &FillLimits) -> Result<(), FillLimit> {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:196 `fill_edit` (the count first,
        // then the points; a total equal to the limit passes).
        if self.fill_edits.len() >= limits.edits {
            return Err(FillLimit::LassoEdits(limits.edits));
        }
        if self.points_used(None) + edit.points_len() > limits.points {
            return Err(FillLimit::Vertices(limits.points));
        }
        self.fill_edits.push(edit);
        Ok(())
    }

    /// A new sampling option: the strokes start over. Nothing changes when the option is the same
    /// and there are no strokes.
    pub fn set_sampling(&mut self, sampling: SamplingMode) -> bool {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:224 `set_sampling`.
        if self.sampling == sampling && self.strokes.is_empty() {
            return false;
        }
        self.strokes.clear();
        self.sampling = sampling;
        true
    }

    /// A Reset: [`FillReset::Sampling`] and [`FillReset::All`] change the session and `settings`,
    /// [`FillReset::Fill`] only `settings`, [`FillReset::FillArea`] only the fill edits. Returns
    /// whether the session changed.
    pub fn reset(&mut self, what: FillReset, settings: &mut FillSettings) -> bool {
        // pc: crates/ui-egui/src/dialogs/caf_ui/lifecycle.rs:238 `reset`.
        let defaults = FillSettings::default();
        match what {
            FillReset::Fill => {
                settings.color_adaptation = defaults.color_adaptation;
                settings.rotation = defaults.rotation;
                settings.scale = defaults.scale;
                settings.mirror = defaults.mirror;
                false
            }
            FillReset::Sampling => {
                settings.sample_all_layers = defaults.sample_all_layers;
                settings.sampling = defaults.sampling;
                self.set_sampling(SamplingMode::Auto)
            }
            FillReset::FillArea => {
                let changed = !self.fill_edits.is_empty();
                self.fill_edits.clear();
                changed
            }
            FillReset::All => {
                let changed = *self != FillSession::default();
                *self = FillSession::default();
                *settings = defaults;
                changed
            }
        }
    }
}

/// Content-Aware Fill's remembered settings (PhotoCraft `CafOpts`,
/// `edit.contentAwareFill.workspace`), persisted by the app on Apply, OK and Cancel (serde, every
/// field defaulted). Defaults: Show Sampling Area on, Opacity 50, Color (142, 208, 106) (the 50 %
/// tint reads as Photoshop's (171, 204, 153)), Indicates Sampling Area, Auto, Sample All Layers
/// off, Color Adaptation Default, Rotation Adaptation None, Scale and Mirror off, Output To New
/// Layer.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct FillSettings {
    /// Whether the overlay shows.
    pub show_overlay: bool,
    /// 0..=100.
    pub opacity: f64,
    /// The overlay tint as sRGB bytes.
    pub color: [u8; 3],
    /// The tint marks what is left out (Indicates › Excluded Area), not the sampling area.
    pub excluded: bool,
    /// How the sampling area starts.
    pub sampling: SamplingMode,
    /// Sample all layers.
    pub sample_all_layers: bool,
    /// Color adaptation.
    pub color_adaptation: ColorAdaptation,
    /// Rotation.
    pub rotation: RotationAdaptation,
    /// Scale.
    pub scale: bool,
    /// Mirror.
    pub mirror: bool,
    /// Output.
    pub output: FillOutputTo,
}

impl Default for FillSettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/caf_ui.rs:121-136 `CafOpts::default`, :62 `OVERLAY_COLOR`.
        Self {
            show_overlay: true,
            opacity: 50.0,
            color: [142, 208, 106],
            excluded: false,
            sampling: SamplingMode::Auto,
            sample_all_layers: false,
            color_adaptation: ColorAdaptation::Default,
            rotation: RotationAdaptation::None,
            scale: false,
            mirror: false,
            output: FillOutputTo::NewLayer,
        }
    }
}

impl SamplingStroke {
    /// A stroke of `mode` and `size` document pixels along `points`.
    pub fn new(mode: EditMode, size: f64, points: Vec<[f64; 2]>) -> Self {
        Self { mode, size, points }
    }
}

impl FillLimits {
    /// At most `edits` strokes and as many lasso edits, and `points` points in all.
    pub const fn new(edits: usize, points: usize) -> Self {
        Self { edits, points }
    }
}

impl FillSettings {
    /// The settings that change the fill: a change asks for a new preview.
    pub(crate) fn fill_params(&self) -> (bool, ColorAdaptation, RotationAdaptation, bool, bool) {
        (self.sample_all_layers, self.color_adaptation, self.rotation, self.scale, self.mirror)
    }
}

/// `edit.contentAwareFill`'s parameters, serialised as PhotoCraft's `plan_params` without the
/// layer (the app adds `"layer"`): `sampling`, `sampleAllLayers`, `colorAdaptation`,
/// `rotationAdaptation`, `scale`, `mirror`, `output`, `fillEdits`, and `samplingStrokes` when
/// there are strokes.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct FillCommand {
    /// How the sampling area starts.
    pub sampling: SamplingMode,
    /// Sample all layers.
    pub sample_all_layers: bool,
    /// Color adaptation.
    pub color_adaptation: ColorAdaptation,
    /// Rotation adaptation.
    pub rotation_adaptation: RotationAdaptation,
    /// Scale.
    pub scale: bool,
    /// Mirror.
    pub mirror: bool,
    /// Output.
    pub output: FillOutputTo,
    /// The fill-area edits, in order.
    pub fill_edits: Vec<FillAreaEdit>,
    /// The Sampling Brush strokes, in order.
    #[serde(rename = "samplingStrokes", skip_serializing_if = "Vec::is_empty")]
    pub strokes: Vec<SamplingStroke>,
}

impl FillCommand {
    /// The command for `session` with `settings`.
    pub fn new(session: &FillSession, settings: &FillSettings) -> FillCommand {
        // pc: crates/ui-egui/src/dialogs/caf_ui/preview.rs:65 `plan_params`.
        FillCommand {
            sampling: session.sampling,
            sample_all_layers: settings.sample_all_layers,
            color_adaptation: settings.color_adaptation,
            rotation_adaptation: settings.rotation,
            scale: settings.scale,
            mirror: settings.mirror,
            output: settings.output,
            fill_edits: session.fill_edits.clone(),
            strokes: session.strokes.clone(),
        }
    }
}

/// What a preview job fills with: the session and the settings as they stood when the pass asked for it.
/// [`FillSource::preview_job`](super::FillSource::preview_job) turns it into the job.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct FillPreviewRequest {
    /// The sampling option, the strokes and the fill-area edits.
    pub session: FillSession,
    /// The settings, of which the fill parameters count.
    pub settings: FillSettings,
    /// Document pixels per texel of the sampling and fill-area planes.
    pub texel: u32,
}

impl FillPreviewRequest {
    /// The request for `session` and `settings` over planes of `texel`.
    pub fn new(session: &FillSession, settings: &FillSettings, texel: u32) -> Self {
        Self { session: session.clone(), settings: settings.clone(), texel }
    }

    /// Whether a fill with `session`, `settings` and `texel` differs from this request: the session, the texel or a
    /// fill parameter differs. The overlay's colour, opacity and Indicates change nothing of the fill.
    pub(crate) fn differs_from(&self, session: &FillSession, settings: &FillSettings, texel: u32) -> bool {
        self.session != *session || self.texel != texel || self.settings.fill_params() != settings.fill_params()
    }
}
