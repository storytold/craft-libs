//! Photoshop's on-canvas brush resize.

use super::{FalloffPreview, TipFalloff};
use crate::canvas_controls::CanvasControlWords;
use crate::runtime::env::{Label, UnitKind, Units};
use crate::style::DataColor;
use craft_behavior::kurbo::Point;
use craft_behavior::{CanvasView, GesturePhase, PointerButton};
use craft_keymap::Mods;
use egui::Pos2;

/// Screen points of vertical travel from level 0 to 1.
pub const RESIZE_TRAVEL: f32 = 200.0;
/// The smallest diameter a resize sets, in document units.
const MIN_DIAMETER: f64 = 1.0;
/// A level is reported in whole percents.
const PERCENT: f64 = 100.0;

/// What the vertical travel of a resize changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum BrushLevel {
    /// The tip's hardness (Photoshop's default, "Vary Round Brush Hardness on HUD" on).
    #[default]
    Hardness,
    /// The brush opacity ("Vary Round Brush Hardness on HUD" off).
    Opacity,
    /// The brush density (Liquify).
    Density,
}

/// The two values a resize sets: the diameter in document units and the level (0 to 1) its [`BrushLevel`] names.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct BrushDimensions {
    /// The new diameter, in document units.
    pub diameter: f64,
    /// The new level, 0 to 1, of the [`BrushLevel`] being resized.
    pub level: f64,
}

impl BrushDimensions {
    /// Dimensions with `diameter` and `level`.
    pub const fn new(diameter: f64, level: f64) -> Self {
        BrushDimensions { diameter, level }
    }
}

/// How a resize behaves, given by the app at the press.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct BrushResizeOptions {
    /// What up and down change.
    pub level: BrushLevel,
    /// The largest diameter the app accepts; the smallest is 1.
    pub max_diameter: f64,
    /// Screen points of vertical travel from level 0 to 1.
    pub travel: f32,
    /// The preview's tint: the user's Brush Preview colour. `None` uses the preview's themed tint.
    pub tint: Option<DataColor>,
}

impl BrushResizeOptions {
    /// Hardness, `max_diameter`, [`RESIZE_TRAVEL`] and the themed tint.
    pub fn new(max_diameter: f64) -> Self {
        BrushResizeOptions { level: BrushLevel::Hardness, max_diameter, travel: RESIZE_TRAVEL, tint: None }
    }

    /// What up and down change.
    #[must_use]
    pub fn level(mut self, level: BrushLevel) -> Self {
        self.level = level;
        self
    }

    /// Screen points of vertical travel from level 0 to 1.
    #[must_use]
    pub fn travel(mut self, travel: f32) -> Self {
        self.travel = travel;
        self
    }

    /// The preview's tint (the user's Brush Preview colour).
    #[must_use]
    pub fn tint(mut self, tint: DataColor) -> Self {
        self.tint = Some(tint);
        self
    }
}

/// One step of a resize: where the gesture stands and the values it holds.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct ResizeStep {
    /// `Begin` at the press, `Update` when a value changed, `Commit` at the release, `Cancel` when it ended without
    /// one.
    pub phase: GesturePhase,
    /// The values to set. At `Cancel` they are the values the resize started from, for the app to put back.
    pub dimensions: BrushDimensions,
    /// Whether `dimensions` differ from the start; false for a click without a change.
    pub changed: bool,
}

/// A resize in progress.
#[derive(Clone, Copy, Debug)]
struct ResizeRun {
    /// Where the press was, in document units: the circle stays there.
    anchor: Point,
    start: BrushDimensions,
    now: BrushDimensions,
    options: BrushResizeOptions,
}

impl ResizeRun {
    fn step(&self, phase: GesturePhase, dimensions: BrushDimensions) -> ResizeStep {
        ResizeStep { phase, dimensions, changed: dimensions != self.start }
    }
}

/// Photoshop's brush resize as a value the tool owns: with a chord ([`chord`](Self::chord)) a press on the canvas
/// resizes the brush instead of painting. Moving right makes the diameter bigger and down makes the level harder, in
/// screen directions however the view is turned or flipped and in every text direction. The circle stays where the
/// press was, tinted by the tip's falloff ([`preview`](Self::preview)); the tool hides the pointer while
/// [`active`](Self::active), and the readout ([`readout_rows`](Self::readout_rows)) follows the pointer.
///
/// The gesture reports [`ResizeStep`]s. It has no keyboard path (Photoshop has none): the bracket keys and the
/// Shift bracket keys are the app's size and hardness commands.
#[derive(Clone, Debug, Default)]
pub struct BrushResize {
    run: Option<ResizeRun>,
}

impl BrushResize {
    /// The values for the pointer at `at`: [`track`](Self::track)'s rule.
    fn values(run: &ResizeRun, view: &CanvasView, at: Pos2) -> BrushDimensions {
        let zoom = if view.zoom.is_finite() && view.zoom > 0.0 { view.zoom } else { 1.0 };
        let from = view.to_screen(run.anchor);
        let finite = |v: f32| if v.is_finite() { f64::from(v) } else { 0.0 };
        let (dx, dy) = (finite(at.x - from.x) / zoom, finite(at.y - from.y) / zoom);
        let max = run.options.max_diameter.max(MIN_DIAMETER);
        let diameter = (run.start.diameter + 2.0 * dx).round().clamp(MIN_DIAMETER, max);
        let travel = f64::from(run.options.travel);
        let shift = if travel.is_finite() && travel > 0.0 { dy * zoom / travel } else { 0.0 };
        let level = ((run.start.level + shift).clamp(0.0, 1.0) * PERCENT).round() / PERCENT;
        BrushDimensions::new(diameter, level)
    }

    /// Whether a press of `button` with `mods` is a resize: Alt with the secondary button (Photoshop on Windows and
    /// Linux), or Control and Alt with the primary button (Control and Option on a Mac). Alt with the primary button
    /// alone is not (the painting tools' eyedropper). [`takes`](Self::takes) lets the secondary button through.
    pub fn chord(button: PointerButton, mods: Mods) -> bool {
        let alt_secondary = button == PointerButton::Secondary && mods.alt;
        let control_alt_primary = button == PointerButton::Primary && mods.alt && mods.ctrl;
        alt_secondary || control_alt_primary
    }

    /// Whether a brush tool takes a press of `button` with `mods`: the primary button always, the secondary only with
    /// Alt (Photoshop's Alt + right-drag resize), so a plain right click stays the canvas menu's. Pass it to
    /// `CanvasTool::takes` beside [`chord`](Self::chord).
    pub fn takes(button: PointerButton, mods: Mods) -> bool {
        button == PointerButton::Primary || (button == PointerButton::Secondary && mods.alt)
    }

    /// Starts a resize at the screen point `at` from the brush's `start` values. A resize whose release was lost is
    /// dropped first without a step. Returns `Begin`, or `None` when `view` cannot map `at` (a non-finite point or
    /// zoom): the press is then not a resize.
    pub fn begin(&mut self, view: &CanvasView, at: Pos2, start: BrushDimensions, options: &BrushResizeOptions) -> Option<ResizeStep> {
        self.run = None;
        if !(at.x.is_finite() && at.y.is_finite() && view.zoom.is_finite() && view.zoom > 0.0) {
            return None;
        }
        let anchor = view.to_doc(at);
        if !(anchor.x.is_finite() && anchor.y.is_finite()) {
            return None;
        }
        let run = ResizeRun { anchor, start, now: start, options: *options };
        let step = run.step(GesturePhase::Begin, start);
        self.run = Some(run);
        Some(step)
    }

    /// The pointer moved to `at` (screen). With `d` the screen offset from the anchor's screen point over the zoom:
    /// `diameter = start + 2 d.x`, rounded, clamped to 1 through `max_diameter`; `level = start + d.y * zoom /
    /// travel`, clamped to 0 through 1 and rounded to a whole percent. Non-finite offsets count as 0, a non-finite or
    /// non-positive zoom as 1. Returns `Update` when either value changed, else `None`.
    pub fn track(&mut self, view: &CanvasView, at: Pos2) -> Option<ResizeStep> {
        let run = self.run.as_mut()?;
        let now = Self::values(run, view, at);
        if now == run.now {
            return None;
        }
        run.now = now;
        Some(run.step(GesturePhase::Update, now))
    }

    /// The release at `at`: [`track`](Self::track)'s values, then `Commit`, with `changed` false when they equal the
    /// start. `None` when no resize runs.
    pub fn finish(&mut self, view: &CanvasView, at: Pos2) -> Option<ResizeStep> {
        let mut run = self.run.take()?;
        run.now = Self::values(&run, view, at);
        Some(run.step(GesturePhase::Commit, run.now))
    }

    /// Escape, a lost pointer or a disabled host: `Cancel` carrying the start values, for the app to put back.
    /// `None` when no resize runs. Photoshop's resize has no cancel; the host's Escape still ends it.
    pub fn cancel(&mut self) -> Option<ResizeStep> {
        let run = self.run.take()?;
        Some(run.step(GesturePhase::Cancel, run.start))
    }

    /// Whether a resize runs.
    pub fn active(&self) -> bool {
        self.run.is_some()
    }

    /// The values of the resize in progress.
    pub fn dimensions(&self) -> Option<BrushDimensions> {
        self.run.map(|run| run.now)
    }

    /// The preview of the resize in progress, for the tool to show at the canvas rect: `falloff` (the tip at the
    /// current values) centred where the press was, in the options' tint, with the circle of the current diameter.
    /// `None` when no resize runs.
    pub fn preview<'a>(&self, falloff: &'a TipFalloff) -> Option<FalloffPreview<'a>> {
        let run = self.run.as_ref()?;
        let preview = FalloffPreview::new(falloff, run.anchor, run.now.diameter / 2.0);
        Some(match run.options.tint {
            Some(tint) => preview.tint(tint),
            None => preview,
        })
    }

    /// The rows of the readout beside the pointer: the diameter in whole pixels and the level (hardness, opacity or
    /// density) in whole percents. No rows when no resize runs. The tool hands them to `ToolContext::readout`.
    pub fn readout_rows(&self, units: &Units, words: &CanvasControlWords) -> Vec<(Label, String)> {
        let Some(run) = &self.run else { return Vec::new() };
        let label = match run.options.level {
            BrushLevel::Hardness => &words.hardness,
            BrushLevel::Opacity => &words.opacity,
            BrushLevel::Density => &words.density,
        };
        let pixels = units.pixels(run.now.diameter.round().max(0.0));
        let level = units.text(UnitKind::Percent, run.now.level * PERCENT, 0..=0);
        vec![(words.diameter.clone(), pixels), (label.clone(), level)]
    }
}
