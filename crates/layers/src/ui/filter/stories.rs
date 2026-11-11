//! Stories of `LayerFilterBar`.

use super::{FilterBy, FilterKind, LayerFilter, LayerFilterBar};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::LayerTree as _;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn show(ui: &mut Ui, filter: &mut LayerFilter) -> StoryOutput {
    let modes = FakeLayers::sample().blend_modes(true, "normal");
    LayerFilterBar::new(Bind::bound(filter)).blend_modes(&modes).show(ui).into()
}

#[story(LayerFilterBar<'static>, covers = [Default], drive = Drive::Never)]
fn by_kind(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { kinds: vec![FilterKind::Pixel], ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never)]
fn by_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Name, name: String::from("Layer"), ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never)]
fn by_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Mode, mode: String::from("multiply"), ..LayerFilter::default() });
    show(ui, filter)
}
