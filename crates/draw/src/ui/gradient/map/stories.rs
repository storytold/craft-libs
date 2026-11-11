//! Stories of the Gradient Map editor.

use craft_theme::Srgba;
use craft_ui::testing::{StoryData, StoryOutput, story};
use egui::Ui;

use super::{GradientMap, GradientMapEditor};
use crate::ui::gradient::{ColorStop, GradientStops};

#[story(GradientMapEditor<'static>, covers = [Default])]
fn gradient_map(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let color = |r, g, b| Srgba { r, g, b, a: 1.0 };
    let initial = GradientMap {
        stops: GradientStops::new(vec![ColorStop::new(0.0, color(0.0, 0.0, 0.0)), ColorStop::new(1.0, color(1.0, 1.0, 1.0))]),
        reverse: false,
        dither: true,
    };
    let value = data.value("map", initial);
    let output = GradientMapEditor::new("Gradient Map", &*value).show(ui);
    if let Some(after) = output.event.as_ref().and_then(craft_behavior::EditEvent::value) {
        *value = after.clone();
    }
    StoryOutput::default()
}
