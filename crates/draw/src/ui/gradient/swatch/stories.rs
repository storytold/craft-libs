//! Stories of `GradientSwatch`.

use super::GradientSwatch;
use crate::ui::gradient::{ColorStop, GradientStops, OpacityStop, StopRows};
use craft_theme::Srgba;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::layout_components::Stack;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn stop(at: f32, [r, g, b]: [f32; 3]) -> ColorStop {
    ColorStop::new(at, Srgba::new(r, g, b, 1.0))
}

/// Three gradients: fading in, with a skewed midpoint, and with the opacity in the colour.
#[story(GradientSwatch, covers = [Default], drive = Drive::Never)]
fn gradient_swatch(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut fading = GradientStops::new(vec![stop(0.0, [0.96, 0.65, 0.14]), stop(1.0, [0.24, 0.39, 0.87])]);
    let mut skewed = fading.clone();
    skewed.color[0].midpoint = 0.15;
    fading.opacity = vec![OpacityStop::new(0.0, 0.0), OpacityStop::new(1.0, 1.0)];
    let combined = GradientStops::new(vec![
        ColorStop::new(0.0, Srgba::new(0.9, 0.28, 0.3, 1.0)),
        ColorStop::new(0.5, Srgba::new(0.9, 0.28, 0.3, 0.0)),
        ColorStop::new(1.0, Srgba::new(0.19, 0.64, 0.42, 1.0)),
    ]);
    Stack::new().show(ui, |ui, _| {
        GradientSwatch::new("Fading in", &fading).fill().has_popup().show(ui);
        GradientSwatch::new("Midpoint at 15 %", &skewed).fill().show(ui);
        GradientSwatch::new("Opacity in the colour", &combined).rows(StopRows::Combined).fill().show(ui);
    });
    StoryOutput::default()
}
