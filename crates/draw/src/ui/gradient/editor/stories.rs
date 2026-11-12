//! Stories of `GradientEditor`.

use super::{GradientEditor, GradientEditorOutput};
use crate::ui::gradient::{ColorStop, GradientStops, OpacityStop, StopRef, StopRows, StopRules};
use craft_theme::Srgba;
use craft_ui::contract::Bind;
use craft_ui::runtime::env::Env;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

craft_ui::words! {
    /// The status line of the gradient editor stories.
    #[non_exhaustive]
    pub(crate) struct GradientEditorStoryWords {
        /// How to ask for a stop's colour.
        hint: "gradientEditorStory.hint" = "Double-click a color stop, or press Enter on it, to edit its color";
        /// The editor asked the host for a stop's colour: `{number}` from 1.
        requested: template "gradientEditorStory.requested" = "Color requested for stop {number}";
    }
}

/// The story's output from the editor's, with the last colour request as its readout, or the hint before one.
fn hinted<P>(ui: &Ui, data: &mut StoryData, output: GradientEditorOutput<P>) -> StoryOutput {
    let words = Env::of(ui).words::<GradientEditorStoryWords>();
    let requested = data.note("requested", None::<usize>);
    if let Some(stop) = &output.color_requested {
        *requested = Some(stop.index);
    }
    let readout = match *requested {
        Some(index) => craft_i18n::fmt(words.requested.as_str(), &[("number", (index + 1).to_string().as_str())]),
        None => words.hint.as_str().to_owned(),
    };
    StoryOutput::from(output).readout(readout)
}

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
    let output = GradientEditor::new("Gradient", Bind::bound(stops)).show(ui);
    hinted(ui, data, output)
}

#[story(GradientEditor<'static, u8>, covers = [], drive = Drive::Never)]
fn gradient_editor_combined(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let stops = data.value("stops", combined());
    let output = GradientEditor::new("Gradient", Bind::bound(stops)).rules(StopRules::new(StopRows::Combined)).show(ui);
    hinted(ui, data, output)
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
    let output = GradientEditor::new("Gradient", Bind::bound(stops)).color_popover(true).shared_selection(Bind::bound(selected)).show(ui);
    hinted(ui, data, output)
}
