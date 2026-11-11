//! Stories of `GradientEditor`.

use super::GradientEditor;
use crate::ui::gradient::{ColorStop, GradientStops, OpacityStop, StopRef, StopRows, StopRules};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn color(at: f32, [r, g, b]: [f32; 3]) -> ColorStop {
    ColorStop::new(at, Srgba::new(r, g, b, 1.0))
}

/// Two colour stops and an opacity ramp in two rows.
fn split() -> GradientStops {
    GradientStops::with_opacity(
        vec![color(0.0, [0.96, 0.65, 0.14]), color(1.0, [0.24, 0.39, 0.87])],
        vec![OpacityStop::new(0.0, 0.0), OpacityStop::new(1.0, 1.0)],
    )
}

/// The opacity in the colour's alpha.
fn combined() -> GradientStops {
    GradientStops::new(vec![
        ColorStop::new(0.0, Srgba::new(0.9, 0.28, 0.3, 1.0)),
        ColorStop::new(0.5, Srgba::new(0.9, 0.28, 0.3, 0.0)),
        ColorStop::new(1.0, Srgba::new(0.19, 0.64, 0.42, 1.0)),
    ])
}

#[story(GradientEditor<'static, u8>, covers = [Default], drive = Drive::Never)]
fn gradient_editor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let stops = data.value("stops", split());
    GradientEditor::new("Gradient", Bind::bound(stops)).show(ui).into()
}

#[story(GradientEditor<'static, u8>, covers = [], drive = Drive::Never)]
fn gradient_editor_combined(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let stops = data.value("stops", combined());
    GradientEditor::new("Gradient", Bind::bound(stops)).rules(StopRules::new(StopRows::Combined)).show(ui).into()
}

#[story(GradientEditor<'static, u8>, covers = [], drive = Drive::Never)]
fn gradient_editor_color_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let stops = data.value("stops", GradientStops::new(vec![color(0.0, [0.0, 0.0, 0.0]), color(1.0, [1.0, 1.0, 1.0])]));
    GradientEditor::new("Gradient map", Bind::bound(stops)).rules(StopRules::new(StopRows::ColorOnly).stops(2, 64)).show(ui).into()
}

/// The colour popover on, a colour stop selected.
#[story(GradientEditor<'static, u8>, covers = [], drive = Drive::Never)]
fn gradient_editor_popover(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (stops, selected) = data.value("state", (split(), Some(StopRef::Color(0))));
    GradientEditor::new("Gradient", Bind::bound(stops)).color_popover(true).shared_selection(Bind::bound(selected)).show(ui).into()
}
