//! Stories of `LayerFilterBar`: the three filter types in one card, each with the count of a fifty-layer document's
//! layers it lets through.

use super::{FilterBy, FilterKind, LayerFilter, LayerFilterBar};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::LayerTree as _;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn show(ui: &mut Ui, filter: &mut LayerFilter) -> StoryOutput {
    let modes = FakeLayers::sample().blend_modes(true, "normal");
    let output = StoryOutput::from(LayerFilterBar::new(Bind::bound(&mut *filter)).blend_modes(&modes).show(ui));
    let layers = FakeLayers::fifty().layers;
    let shown = layers.iter().filter(|layer| filter.shows(&layer.info)).count();
    output.readout(format!("{shown} of {} layers match", layers.len()))
}

#[story(LayerFilterBar<'static>, covers = [Default], drive = Drive::Never, group = "Types", title = "Filter by Kind")]
fn by_kind(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { kinds: vec![FilterKind::Pixel], ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Name")]
fn by_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Name, name: String::from("mask"), ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Mode")]
fn by_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Mode, mode: String::from("multiply"), ..LayerFilter::default() });
    show(ui, filter)
}
