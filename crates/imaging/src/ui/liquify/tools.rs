//! Liquify's tools, its brush and the stroke recorder.

use serde::{Deserialize, Serialize};

use craft_behavior::CancelReason;
use craft_ui::rails::{RailTool, ToolSlot};

use super::LiquifyCommands;
use super::session::{LiquifyStroke, StrokeEvent};

/// The tools of Liquify's strip, in strip order, with Photoshop's letters. Serialised with the
/// engine's names (`forwardWarp`, `twirlCw`, `lassoMask`, …) so a [`LiquifyStroke`] passes to
/// PhotoCraft's engine unchanged.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub enum LiquifyTool {
    /// W.
    #[default]
    ForwardWarp,
    /// R.
    Reconstruct,
    /// E.
    Smooth,
    /// C.
    #[serde(rename = "twirlCw")]
    TwirlClockwise,
    /// No letter (Photoshop's Twirl is one tool; PhotoCraft shows both directions).
    #[serde(rename = "twirlCcw")]
    TwirlCounterclockwise,
    /// S.
    Pucker,
    /// B.
    Bloat,
    /// O.
    PushLeft,
    /// F.
    Freeze,
    /// D.
    Thaw,
    /// L: drag to freeze an area, Alt-drag to thaw it.
    #[serde(rename = "lassoMask")]
    FreezeLasso,
    /// H.
    Hand,
    /// Z.
    Zoom,
}

impl LiquifyTool {
    /// The strip's tools in order.
    pub const ALL: [LiquifyTool; 13] = [
        LiquifyTool::ForwardWarp,
        LiquifyTool::Reconstruct,
        LiquifyTool::Smooth,
        LiquifyTool::TwirlClockwise,
        LiquifyTool::TwirlCounterclockwise,
        LiquifyTool::Pucker,
        LiquifyTool::Bloat,
        LiquifyTool::PushLeft,
        LiquifyTool::Freeze,
        LiquifyTool::Thaw,
        LiquifyTool::FreezeLasso,
        LiquifyTool::Hand,
        LiquifyTool::Zoom,
    ];

    /// The tool a press with Alt held paints with: the twirls reverse, Pucker and Bloat swap; the
    /// others are themselves. (Alt on the Freeze Lasso thaws: [`LiquifyStroke::amount`].)
    pub fn alternate(self) -> LiquifyTool {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:658 `alt_tool`.
        match self {
            LiquifyTool::TwirlClockwise => LiquifyTool::TwirlCounterclockwise,
            LiquifyTool::TwirlCounterclockwise => LiquifyTool::TwirlClockwise,
            LiquifyTool::Pucker => LiquifyTool::Bloat,
            LiquifyTool::Bloat => LiquifyTool::Pucker,
            other => other,
        }
    }

    /// Whether the tool paints with the round brush (all but the Freeze Lasso, Hand and Zoom).
    pub fn is_brush(self) -> bool {
        !matches!(self, LiquifyTool::FreezeLasso | LiquifyTool::Hand | LiquifyTool::Zoom)
    }

    /// Whether the tool's strokes change the freeze mask (Freeze, Thaw, Freeze Lasso).
    pub fn edits_mask(self) -> bool {
        matches!(self, LiquifyTool::Freeze | LiquifyTool::Thaw | LiquifyTool::FreezeLasso)
    }

    /// The strip: one slot per tool, each running the tool's command (its label, icon and letter), with a group
    /// end after Smooth, Push Left, Thaw and the Freeze Lasso (the fourth separates Hand and Zoom).
    pub fn slots() -> Vec<ToolSlot<LiquifyTool>> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:888 `draw_tool_strip` (gaps at :904).
        (0_u32..)
            .zip(Self::ALL)
            .map(|(id, tool)| {
                let tools = vec![RailTool::new(tool, LiquifyCommands::of(tool))];
                let group_end = matches!(tool, LiquifyTool::Smooth | LiquifyTool::PushLeft | LiquifyTool::Thaw | LiquifyTool::FreezeLasso);
                if group_end { ToolSlot::ending_group(id, tools) } else { ToolSlot::new(id, tools) }
            })
            .collect()
    }
}

/// The brush of the painting tools. Serialised in [`LiquifySettings`](super::LiquifySettings).
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
#[non_exhaustive]
pub struct LiquifyBrush {
    /// Diameter in document pixels, 1..=15000 ([`Self::SIZE_MAX`]); the slider shows 1..=3000
    /// ([`Self::SLIDER_MAX`]), the keys and the resize reach further.
    pub size: f64,
    /// How far the full effect reaches towards the edge, 0..=100.
    pub density: f64,
    /// Strength, 0..=100.
    pub pressure: f64,
    /// Speed of the tools that act while the pointer rests, 0..=100.
    pub rate: f64,
}

impl Default for LiquifyBrush {
    fn default() -> Self {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:73-76.
        Self { size: 100.0, density: 50.0, pressure: 100.0, rate: 80.0 }
    }
}

impl LiquifyBrush {
    /// The largest size the keys and the resize gesture set.
    pub const SIZE_MAX: f64 = 15000.0;
    /// The Size slider's top.
    pub const SLIDER_MAX: f64 = 3000.0;

    /// `[`: the size × 0.9 rounded, at least one pixel smaller, not below 1.
    pub fn smaller(mut self) -> Self {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:765-768.
        self.size = (self.size * 0.9).round().min(self.size - 1.0).clamp(1.0, Self::SIZE_MAX);
        self
    }

    /// `]`: the size × 1.1 rounded, at least one pixel larger, not above [`Self::SIZE_MAX`].
    pub fn larger(mut self) -> Self {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:769-771.
        self.size = (self.size * 1.1).round().max(self.size + 1.0).min(Self::SIZE_MAX);
        self
    }
}

/// Turns a press, drags and a release into one [`LiquifyStroke`] with its [`EditEvent`]s. Ui-free:
/// the canvas tool feeds it document points.
///
/// - [`begin`](Self::begin) starts a stroke of `tool` (its [`alternate`](LiquifyTool::alternate)
///   with Alt) with the brush at the press, and reports `Begin` with the one-point stroke.
/// - [`extend`](Self::extend) adds a point (skipped when equal to the last) and reports `Update`
///   with the brush of the stroke and that one new point.
/// - [`end`](Self::end) adds the release point when it differs from the last, so a short stroke is
///   recorded whole (storytold/photocraft#8055: a stroke shorter than the drag threshold still paints), and
///   reports `Commit` with the whole stroke.
/// - [`cancel`](Self::cancel) drops the stroke and reports `Cancel`.
///
/// The Freeze Lasso is not recorded here: the canvas tool draws it with an `OutlineDraft` and
/// records the closed polygon with `amount` 1 (0 with Alt at the press).
#[derive(Clone, Debug, Default)]
pub struct StrokeRecorder {
    stroke: Option<LiquifyStroke>,
}

impl StrokeRecorder {
    /// A stroke already in progress is replaced without an event: the caller ends it first.
    pub fn begin(&mut self, tool: LiquifyTool, brush: &LiquifyBrush, at: [f64; 3], alt: bool) -> StrokeEvent {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:263 `begin`, :240 `template`.
        let tool = if alt { tool.alternate() } else { tool };
        let stroke = LiquifyStroke {
            tool,
            size: brush.size,
            density: brush.density,
            pressure: brush.pressure,
            rate: brush.rate,
            points: vec![at],
            amount: None,
        };
        self.stroke = Some(stroke.clone());
        StrokeEvent::Begin(stroke)
    }

    /// Adds the point `at` (x, y, pressure) unless it equals the last one, and reports `Update` with it. `None` with no stroke.
    pub fn extend(&mut self, at: [f64; 3]) -> Option<StrokeEvent> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:279 `extend`; :704-713 (an equal point is
        // skipped).
        let stroke = self.stroke.as_mut()?;
        if stroke.points.last().is_some_and(|last| last[0] == at[0] && last[1] == at[1]) {
            return None;
        }
        stroke.points.push(at);
        Some(StrokeEvent::Update(stroke.with_points(vec![at])))
    }

    /// A held dab: adds `at` even when it equals the last point, because a tool that acts while
    /// the brush rests dabs again in place (unlike [`extend`](Self::extend), which skips a
    /// repeated point), and reports `Update` with that one point. `None` with no stroke.
    pub fn hold(&mut self, at: [f64; 3]) -> Option<StrokeEvent> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:1203-1207 (a stationary tool's `Move` at
        // the same point, 30 a second).
        let stroke = self.stroke.as_mut()?;
        stroke.points.push(at);
        Some(StrokeEvent::Update(stroke.with_points(vec![at])))
    }

    /// Ends the stroke at `at`, adding the release point if it differs from the last, and reports `Commit`. `None` with no stroke.
    pub fn end(&mut self, at: [f64; 2]) -> Option<StrokeEvent> {
        // pc: crates/ui-egui/src/dialogs/liquify_ui.rs:714-727 (`Up` adds the release point with
        // the last pressure, then `end`).
        let mut stroke = self.stroke.take()?;
        if let Some(&[x, y, pressure]) = stroke.points.last()
            && (x != at[0] || y != at[1])
        {
            stroke.points.push([at[0], at[1], pressure]);
        }
        Some(StrokeEvent::Commit(stroke))
    }

    /// Drops the stroke and reports `Cancel` with `reason`. `None` with no stroke.
    pub fn cancel(&mut self, reason: CancelReason) -> Option<StrokeEvent> {
        // PhotoCraft has no stroke cancel; the field already took the dabs, so the workspace
        // follows a Cancel with a Replay.
        self.stroke.take()?;
        Some(StrokeEvent::Cancel(reason))
    }

    /// Whether a stroke is in progress.
    pub fn is_active(&self) -> bool {
        self.stroke.is_some()
    }
}
