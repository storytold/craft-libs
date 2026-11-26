//! Stories of the gradient fill editor.

use craft_color_ui::ui::gradient::{ColorStop, GradientStops};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::patterns::presets::PresetState;
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

use super::GradientFillEditor;
use crate::ui::gradient::{GradientFill, GradientGroup, GradientItem, GradientRef, GradientStyle};

fn pair(from: [f32; 3], to: [f32; 3]) -> GradientStops {
    GradientStops::new(vec![ColorStop::new(0.0, Srgba::rgb(from[0], from[1], from[2])), ColorStop::new(1.0, Srgba::rgb(to[0], to[1], to[2]))])
}

fn groups() -> Vec<GradientGroup> {
    vec![GradientGroup::new(
        "Basics",
        vec![GradientItem::new("Black to white", pair([0.0; 3], [1.0; 3])), GradientItem::new("Red to blue", pair([0.9, 0.1, 0.1], [0.1, 0.1, 0.9]))],
    )]
}

/// The story's fill and the palette's state.
fn state(data: &mut StoryData, fill: GradientFill) -> (&mut GradientFill, &mut PresetState<GradientRef>) {
    let both = data.value("state", (fill, PresetState::new()));
    (&mut both.0, &mut both.1)
}

#[story(GradientFillEditor<'static>, covers = [Default])]
fn gradient_fill_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = groups();
    let recent = [pair([0.2, 0.8, 0.4], [0.1, 0.2, 0.6])];
    let (fill, palette) = state(data, GradientFill::new(pair([0.13, 0.35, 0.72], [0.95, 0.75, 0.2])));
    let shown = GradientFillEditor::new("Gradient", Bind::bound(fill), &groups, Bind::bound(palette)).recent(&recent).show(ui);
    StoryOutput::from(shown)
}

#[story(GradientFillEditor<'static>, covers = [Default])]
fn gradient_fill_editor_without_presets(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut initial = GradientFill::new(pair([0.13, 0.35, 0.72], [0.95, 0.75, 0.2]));
    initial.style = GradientStyle::Radial;
    initial.reverse = true;
    let (fill, palette) = state(data, initial);
    let shown = GradientFillEditor::new("Gradient", Bind::bound(fill), &[], Bind::bound(palette)).show(ui);
    StoryOutput::from(shown)
}
