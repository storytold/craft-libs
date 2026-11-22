//! Stories of `LayerCompsPanel`: no comps, two comps with one applied, a comp with a warning.

use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{LayerCompRow, LayerCompsPanel};

#[story(LayerCompsPanel<'static>, covers = [Default], drive = Drive::Never)]
fn layer_comps_panel_no_comps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (selected, selected_at) = data.value_at("selected", None::<u64>);
    StoryOutput::from(LayerCompsPanel::new("Layer Comps", &[], selected_at.bind(selected)).last_state_applied(true).show(ui))
}

#[story(LayerCompsPanel<'static>, covers = [], drive = Drive::Never)]
fn layer_comps_panel_two_comps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = vec![
        LayerCompRow::new(1, "Daylight").applied(true).visibility(true).position(true).appearance(true),
        LayerCompRow::new(2, "Night").comment("The scene with the lamps on").visibility(true),
    ];
    let (selected, selected_at) = data.value_at("selected", Some(1_u64));
    StoryOutput::from(LayerCompsPanel::new("Layer Comps", &rows, selected_at.bind(selected)).show(ui))
}

#[story(LayerCompsPanel<'static>, covers = [], drive = Drive::Never)]
fn layer_comps_panel_warning(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rows = vec![LayerCompRow::new(1, "Draft").missing(3).visibility(true), LayerCompRow::new(2, "Final").missing(1).position(true)];
    let (selected, selected_at) = data.value_at("selected", None::<u64>);
    StoryOutput::from(LayerCompsPanel::new("Layer Comps", &rows, selected_at.bind(selected)).show(ui))
}
