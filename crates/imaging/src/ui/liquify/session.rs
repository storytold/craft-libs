//! Liquify's session (the steps the engine replays) and its settings.

use serde::{Deserialize, Serialize};

use craft_behavior::CancelReason;

use super::tools::{LiquifyBrush, LiquifyTool};

/// One brush stroke or Freeze Lasso polygon as the engine replays it: the tool that painted (with
/// Alt at the press already applied: [`LiquifyTool::alternate`]), the brush at the press and the
/// points. Serialised as PhotoCraft's `LiquifyStroke` (`photocraft_algo::liquify`), so the app can
/// pass it on unchanged.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct LiquifyStroke {
    /// The tool that made the stroke.
    pub tool: LiquifyTool,
    /// Brush diameter in document pixels.
    pub size: f64,
    /// 0..=100.
    pub density: f64,
    /// 0..=100.
    pub pressure: f64,
    /// 0..=100: the speed of the tools that act while the pointer rests.
    pub rate: f64,
    /// `[x, y, pressure]` in document pixels; a lasso's points carry pressure 1 and end where they
    /// began.
    pub points: Vec<[f64; 3]>,
    /// The Freeze Lasso only: 1 freezes the polygon, 0 thaws it (Alt at the press).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub amount: Option<f64>,
}

impl LiquifyStroke {
    /// A stroke of `tool` with `brush`'s size, density, pressure and rate over `points` (x, y, pressure).
    pub fn new(tool: LiquifyTool, brush: &LiquifyBrush, points: Vec<[f64; 3]>) -> LiquifyStroke {
        LiquifyStroke { tool, size: brush.size, density: brush.density, pressure: brush.pressure, rate: brush.rate, points, amount: None }
    }

    /// The same brush and tool over `points`: what an `Update` carries (only the points added
    /// since the last event, so a long stroke costs no more per dab than a short one).
    pub(super) fn with_points(&self, points: Vec<[f64; 3]>) -> LiquifyStroke {
        LiquifyStroke {
            tool: self.tool,
            size: self.size,
            density: self.density,
            pressure: self.pressure,
            rate: self.rate,
            points,
            amount: self.amount,
        }
    }
}

/// A stroke's phases as the app applies them to its field.
///
/// `Begin` carries the stroke with its first point, `Update` the stroke's tool and brush with only the points
/// added since the last event, `Commit` the whole stroke, `Cancel` why the stroke ended without being recorded.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum StrokeEvent {
    /// A stroke began.
    Begin(LiquifyStroke),
    /// A stroke grew by the points it carries.
    Update(LiquifyStroke),
    /// A stroke ended; the session records it.
    Commit(LiquifyStroke),
    /// A stroke ended without a record.
    Cancel(CancelReason),
}

/// A whole-field operation, recorded and replayed like a stroke.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum LiquifyGlobal {
    /// The Reconstruct button: scales the unfrozen field towards zero by `amount` percent.
    #[non_exhaustive]
    Reconstruct {
        /// How much of the field is reconstructed, 0..=100.
        amount: f64,
    },
    /// Mask Options › Mask All: freezes everything.
    MaskAll,
    /// Mask Options › None (and ⌘D): thaws everything.
    MaskNone,
    /// Mask Options › Invert All (and ⌘I).
    InvertAll,
}

/// One recorded step of the session, in replay order.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum LiquifyStep {
    /// A brush stroke.
    Stroke(LiquifyStroke),
    /// A whole-mesh operation such as Reconstruct All or Invert.
    Global(LiquifyGlobal),
}

/// What OK applies and the history snapshots: the recorded steps. Restore All empties it (one
/// undo step, where PhotoCraft's could not be undone). The brush, view options and the tool are
/// [`LiquifySettings`], outside the history: undo never changes them.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct LiquifySession {
    steps: Vec<LiquifyStep>,
}

/// A lasso whose last point is within this many document pixels of its first (on both axes) is
/// closed already.
const LASSO_CLOSE_REACH: f64 = 0.5;

impl LiquifyGlobal {
    /// The Reconstruct button at `amount` percent.
    pub const fn reconstruct(amount: f64) -> LiquifyGlobal {
        LiquifyGlobal::Reconstruct { amount }
    }
}

impl LiquifySession {
    /// A session holding `steps` (a smart filter opened again).
    pub fn from_steps(steps: Vec<LiquifyStep>) -> Self {
        Self { steps }
    }

    /// The steps in replay order.
    pub fn steps(&self) -> &[LiquifyStep] {
        &self.steps
    }

    /// Records a finished stroke. A stroke without points records nothing; a lasso of fewer than 3
    /// points records nothing, and one whose last point is more than half a pixel from the first
    /// gets the first point again at its end.
    pub fn record(&mut self, mut stroke: LiquifyStroke) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:291 `end`, :299 `close_lasso`.
        if stroke.points.is_empty() {
            return;
        }
        if stroke.tool == LiquifyTool::FreezeLasso {
            if stroke.points.len() < 3 {
                return;
            }
            if let (Some(&first), Some(&last)) = (stroke.points.first(), stroke.points.last())
                && ((first[0] - last[0]).abs() > LASSO_CLOSE_REACH || (first[1] - last[1]).abs() > LASSO_CLOSE_REACH)
            {
                stroke.points.push(first);
            }
        }
        self.steps.push(LiquifyStep::Stroke(stroke));
    }

    /// Records a whole-field operation.
    pub fn record_global(&mut self, op: LiquifyGlobal) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:319 `global`.
        self.steps.push(LiquifyStep::Global(op));
    }

    /// Restore All: drops every step.
    pub fn restore_all(&mut self) {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:361 `restore_all`.
        self.steps.clear();
    }

    /// The strokes recorded so far (for the app's status line; PhotoCraft's "N stroke(s)").
    pub fn len(&self) -> usize {
        self.steps.len()
    }

    /// Whether no step is recorded.
    pub fn is_empty(&self) -> bool {
        self.steps.is_empty()
    }
}

/// Show Mesh's Mesh Size. PhotoCraft's mesh has 64, 40 or 24 cells across the document's longer
/// side (`liquify_ui.rs:1250`); the app builds [`LiquifyMesh`](super::LiquifyMesh) for it.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum MeshSize {
    /// 64 cells across the document's longer side.
    Small,
    #[default]
    /// 40 cells across the document's longer side.
    Medium,
    /// 24 cells across the document's longer side.
    Large,
}

impl MeshSize {
    /// The three sizes, smallest first.
    pub const ALL: [MeshSize; 3] = [MeshSize::Small, MeshSize::Medium, MeshSize::Large];
}

/// Show Backdrop › Mode, as Photoshop draws it (measured on its Liquify): In Front lays the
/// backdrop over the image at the Opacity, Behind lays the image over the backdrop at the Opacity,
/// Blend fades the image to 100 − Opacity under the backdrop at the Opacity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BackdropMode {
    #[default]
    /// The backdrop over the image.
    InFront,
    /// The image over the backdrop.
    Behind,
    /// The image faded under the backdrop.
    Blend,
}

impl BackdropMode {
    /// The three modes, in menu order.
    pub const ALL: [BackdropMode; 3] = [BackdropMode::InFront, BackdropMode::Behind, BackdropMode::Blend];
}

/// Show Backdrop › Use.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum BackdropUse {
    /// The document as it is, the liquified layer included (as Photoshop).
    #[default]
    AllLayers,
    /// One layer alone, by the app's id. A layer that is gone shows All Layers.
    Layer(u64),
}

/// Liquify's remembered settings: the tool, the brush, Reconstruct's amount and the view options.
/// Persisted by the app (serde, every field defaulted, so an old or partial record loads); never in
/// the session's history. Defaults: Forward Warp; size 100, density 50, pressure 100, rate 80;
/// amount 100; mesh off at Medium; mask shown; backdrop off, All Layers, In Front, opacity 50.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct LiquifySettings {
    /// The selected tool.
    pub tool: LiquifyTool,
    /// The brush options.
    pub brush: LiquifyBrush,
    /// Brush Reconstruct Options › Amount, 0..=100.
    pub reconstruct_amount: f64,
    /// Whether Show Mesh is on.
    pub show_mesh: bool,
    /// The mesh density.
    pub mesh_size: MeshSize,
    /// Whether Show Mask is on.
    pub show_mask: bool,
    /// Whether Show Backdrop is on.
    pub show_backdrop: bool,
    /// Which layers the backdrop shows.
    pub backdrop: BackdropUse,
    /// How the backdrop combines with the image.
    pub backdrop_mode: BackdropMode,
    /// 0..=100.
    pub backdrop_opacity: f64,
}

impl Default for LiquifySettings {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:69-88 `LiquifyOpts::default`.
        Self {
            tool: LiquifyTool::ForwardWarp,
            brush: LiquifyBrush::default(),
            reconstruct_amount: 100.0,
            show_mesh: false,
            mesh_size: MeshSize::Medium,
            show_mask: true,
            show_backdrop: false,
            backdrop: BackdropUse::AllLayers,
            backdrop_mode: BackdropMode::InFront,
            backdrop_opacity: 50.0,
        }
    }
}

impl LiquifySettings {
    /// The settings with every number in its range (size 1..=15000, the others 0..=100); a
    /// non-finite number keeps its value in `old` (the settings in force, `Self::default()` at
    /// start-up): what the app calls after loading a record.
    pub fn clamped(mut self, old: &LiquifySettings) -> Self {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:97 `LiquifyOpts::apply` (the clamps; a
        // non-finite number is skipped, so the old value stays).
        let size = |n: f64, old: f64| if n.is_finite() { n.clamp(1.0, LiquifyBrush::SIZE_MAX) } else { old };
        let percent = |n: f64, old: f64| if n.is_finite() { n.clamp(0.0, 100.0) } else { old };
        self.brush.size = size(self.brush.size, old.brush.size);
        self.brush.density = percent(self.brush.density, old.brush.density);
        self.brush.pressure = percent(self.brush.pressure, old.brush.pressure);
        self.brush.rate = percent(self.brush.rate, old.brush.rate);
        self.reconstruct_amount = percent(self.reconstruct_amount, old.reconstruct_amount);
        self.backdrop_opacity = percent(self.backdrop_opacity, old.backdrop_opacity);
        self
    }
}
