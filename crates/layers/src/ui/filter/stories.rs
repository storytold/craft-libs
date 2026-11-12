//! Stories of `LayerFilterBar`: each filter type in one card, with the count of a document's layers it lets through;
//! and `LayersPanel` filtered by Artboard and by Group, which keep each group open or closed.

use super::{EffectKind, FilterBy, FilterKind, LayerAttribute, LayerFilter, LayerFilterBar, SmartKind};
use crate::ui::mock::FakeLayers;
use crate::ui::panel::LayersPanel;
use crate::ui::tree::LayerTree as _;
use craft_theme::LabelColor;
use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The bar over `filter`, with the count of `tree`'s layers the filter lists.
fn show_over(ui: &mut Ui, filter: &mut LayerFilter, tree: &FakeLayers) -> StoryOutput {
    let modes = tree.blend_modes(false, "normal");
    let output = StoryOutput::from(LayerFilterBar::new(Bind::bound(&mut *filter)).blend_modes(&modes).show(ui));
    let filter: &LayerFilter = filter;
    let shown = tree.layers.iter().filter(|layer| !filter.active() || tree.listed(&layer.id, filter)).count();
    output.readout(format!("{shown} of {} layers match", tree.layers.len()))
}

/// The bar over `filter`, counted over the fifty-layer tree.
fn show(ui: &mut Ui, filter: &mut LayerFilter) -> StoryOutput {
    show_over(ui, filter, &FakeLayers::fifty())
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

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Effect")]
fn by_effect(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Effect, effect: EffectKind::Stroke, ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Mode")]
fn by_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Mode, mode: String::from("multiply"), ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Attribute")]
fn by_attribute(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Attribute, attribute: LayerAttribute::Clipped, ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Color")]
fn by_color(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Color, color: Some(LabelColor::Blue), ..LayerFilter::default() });
    show(ui, filter)
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Smart Object")]
fn by_smart_object(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::SmartObject, smart: vec![SmartKind::Missing], ..LayerFilter::default() });
    show_over(ui, filter, &FakeLayers::boards())
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Artboard")]
fn by_artboard(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Artboard, ..LayerFilter::default() });
    show_over(ui, filter, &FakeLayers::boards())
}

#[story(LayerFilterBar<'static>, covers = [], drive = Drive::Never, group = "Types", title = "Filter by Group")]
fn by_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let filter = data.value("filter", LayerFilter { by: FilterBy::Group, ..LayerFilter::default() });
    show(ui, filter)
}

/// The artboard that holds the selected "Oak", with its contents.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_by_artboard(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::boards();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Artboard, ..LayerFilter::default() });
    StoryOutput::from(LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui))
}

/// Every group with its contents; the closed groups stay closed.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_by_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::fifty();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Group, ..LayerFilter::default() });
    StoryOutput::from(LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui))
}
