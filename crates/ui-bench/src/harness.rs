//! Runs a scenario headless: an egui `Context` driven directly, with no renderer.
//!
//! Each frame is `Context::run_ui` (every pass of the frame) plus dropping its `FullOutput`. Shapes
//! are not tessellated, so the numbers are layout, widget and paint-list cost.

use std::time::Instant;

use egui::{Context, Event, Pos2, RawInput, Rect, Ui, Vec2, vec2};

use crate::alloc::allocations;
use crate::report::{Measurement, Report, ScenarioResult};
use crate::scenario::SCENARIOS;

/// Frames run and discarded before measuring, so caches and fonts are warm.
pub const WARMUP_FRAMES: u64 = 30;

/// Frames measured per scenario.
pub const MEASURED_FRAMES: u64 = 200;

const FRAME_SECONDS: f64 = 1.0 / 60.0;

/// What a scenario draws each frame.
pub trait Scene {
    /// The screen size in points.
    fn size(&self) -> Vec2 {
        vec2(800.0, 600.0)
    }

    /// Input for `frame` (the first warm-up frame is 0).
    fn events(&mut self, _frame: u64, _events: &mut Vec<Event>) {}

    /// Draws the frame.
    fn show(&mut self, ui: &mut Ui, frame: u64);
}

/// A library's answer for one scenario.
pub enum SceneBuild {
    /// The library draws it; `equivalent` names the components used.
    Scene {
        /// The scene.
        scene: Box<dyn Scene>,
        /// What the scene uses in place of the spec's component.
        equivalent: &'static str,
    },
    /// The library has nothing comparable.
    Missing {
        /// Why.
        reason: &'static str,
    },
}

/// A UI library under test: its setup and its scene for each scenario in [`SCENARIOS`].
pub trait Library {
    /// The name in the report and the baseline file.
    fn name(&self) -> &'static str;

    /// Prepares a fresh context (theme, fonts, style).
    fn install(&self, ctx: &Context);

    /// Runs before each frame, outside the frame's passes (the foundation's per-frame root input).
    fn before_frame(&self, _ctx: &Context) {}

    /// The scene for the scenario called `name`.
    fn build(&self, name: &str) -> SceneBuild;
}

/// The screen rect of `size` at the origin.
pub fn screen_rect(size: Vec2) -> Rect {
    Rect::from_min_size(Pos2::ZERO, size)
}

struct Sample {
    seconds: f64,
    allocations: u64,
    bytes: u64,
    passes: usize,
}

fn run_frame(library: &dyn Library, ctx: &Context, scene: &mut dyn Scene, frame: u64, size: Vec2) -> Sample {
    let mut events = Vec::new();
    scene.events(frame, &mut events);
    let raw = RawInput {
        screen_rect: Some(screen_rect(size)),
        time: Some(frame as f64 * FRAME_SECONDS),
        predicted_dt: FRAME_SECONDS as f32,
        events,
        ..Default::default()
    };
    library.before_frame(ctx);
    let (count, bytes) = allocations();
    let start = Instant::now();
    let output = ctx.run_ui(raw, |ui| scene.show(ui, frame));
    let passes = output.platform_output.num_completed_passes;
    drop(output);
    let seconds = start.elapsed().as_secs_f64();
    let (count_after, bytes_after) = allocations();
    Sample { seconds, allocations: count_after - count, bytes: bytes_after - bytes, passes }
}

fn measure(library: &dyn Library, mut scene: Box<dyn Scene>) -> Measurement {
    let ctx = Context::default();
    library.install(&ctx);
    let size = scene.size();
    for frame in 0..WARMUP_FRAMES {
        run_frame(library, &ctx, scene.as_mut(), frame, size);
    }
    let samples: Vec<Sample> =
        (WARMUP_FRAMES..WARMUP_FRAMES + MEASURED_FRAMES).map(|frame| run_frame(library, &ctx, scene.as_mut(), frame, size)).collect();
    Measurement::from_samples(
        samples.iter().map(|s| s.seconds * 1000.0).collect(),
        samples.iter().map(|s| s.allocations as f64).collect(),
        samples.iter().map(|s| s.bytes as f64).collect(),
        samples.iter().map(|s| s.passes.saturating_sub(1)).collect(),
    )
}

/// Measures every scenario of [`SCENARIOS`] on `library`.
pub fn run_all(library: &dyn Library) -> Report {
    let scenarios = SCENARIOS
        .iter()
        .map(|scenario| {
            eprintln!("{}: {}", library.name(), scenario.name);
            let (equivalent, missing, measurement) = match library.build(scenario.name) {
                SceneBuild::Scene { scene, equivalent } => (Some(equivalent.to_owned()), None, Some(measure(library, scene))),
                SceneBuild::Missing { reason } => (None, Some(reason.to_owned()), None),
            };
            ScenarioResult {
                name: scenario.name.to_owned(),
                description: scenario.description.to_owned(),
                no_discard: scenario.no_discard,
                equivalent,
                no_equivalent_reason: missing,
                measurement,
            }
        })
        .collect();
    Report { library: library.name().to_owned(), warmup_frames: WARMUP_FRAMES, measured_frames: MEASURED_FRAMES, scenarios }
}
