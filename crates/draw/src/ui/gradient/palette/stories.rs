//! Stories of the gradient palette.

use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::patterns::presets::{PresetState, PresetView};
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

use super::{GradientGroup, GradientItem, GradientPalette, GradientRef};
use crate::ui::gradient::{ColorStop, GradientStops};

fn pair(a: [f32; 3], b: [f32; 3]) -> GradientStops {
    let color = |c: [f32; 3]| Srgba { r: c[0], g: c[1], b: c[2], a: 1.0 };
    GradientStops::new(vec![ColorStop::new(0.0, color(a)), ColorStop::new(1.0, color(b))])
}

fn groups() -> Vec<GradientGroup> {
    vec![GradientGroup::new(
        "Basics",
        vec![
            GradientItem::new("Black to white", pair([0.0; 3], [1.0; 3])),
            GradientItem::new("Red to blue", pair([0.9, 0.1, 0.1], [0.1, 0.1, 0.9])),
            GradientItem::new("Sunset", pair([0.95, 0.6, 0.1], [0.5, 0.1, 0.5])),
        ],
    )]
}

fn palette(ui: &mut Ui, data: &mut StoryData, view: PresetView) -> StoryOutput {
    let groups = groups();
    let recent = [pair([0.2, 0.8, 0.4], [0.1, 0.2, 0.6])];
    let mut initial = PresetState::new();
    initial.view = view;
    let state = data.value("state", initial);
    let _ = GradientPalette::new("Gradients", &groups, Bind::bound(&mut *state))
        .recent(&recent)
        .selected(GradientRef::Item { group: 0, item: 1 })
        .show(ui);
    StoryOutput::default()
}

#[story(GradientPalette<'static>, covers = [Default])]
fn gradient_palette(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, PresetView::Grid)
}

#[story(GradientPalette<'static>, covers = [Default])]
fn gradient_palette_list(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    palette(ui, data, PresetView::List)
}
