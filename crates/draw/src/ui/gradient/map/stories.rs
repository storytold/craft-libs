//! Stories of the Gradient Map editor.

use craft_theme::Srgba;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use egui::Ui;

use super::{GradientMap, GradientMapEditor, GradientMethod};
use craft_color_ui::ui::gradient::{ColorStop, GradientStops};

#[story(GradientMapEditor<'static>, covers = [Default])]
fn gradient_map(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, data, &[])
}

/// Opens the Method list with a click on it.
fn open_method(fixture: &mut Fixture) {
    if let Some(list) = fixture.by_label("Method") {
        fixture.click(&list);
        fixture.idle(3);
    }
}

/// The Method list open, with Stripes listed but disabled, as for a host that cannot render it yet.
#[story(GradientMapEditor<'static>, covers = [Open], drive = Drive::Custom(open_method))]
fn gradient_map_method_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, data, &[GradientMethod::Stripes])
}

fn shown(ui: &mut Ui, data: &mut StoryData, disabled: &[GradientMethod]) -> StoryOutput {
    let color = |r, g, b| Srgba { r, g, b, a: 1.0 };
    let initial = GradientMap {
        stops: GradientStops::new(vec![ColorStop::new(0.0, color(0.0, 0.0, 0.0)), ColorStop::new(1.0, color(1.0, 1.0, 1.0))]),
        reverse: false,
        dither: true,
        method: GradientMethod::Smooth,
    };
    let value = data.value("map", initial);
    let output = GradientMapEditor::new("Gradient Map", &*value).disabled_methods(disabled).show(ui);
    if let Some(after) = output.event.as_ref().and_then(craft_behavior::EditEvent::value) {
        *value = after.clone();
    }
    StoryOutput::default()
}
