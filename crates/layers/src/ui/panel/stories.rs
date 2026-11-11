//! Stories of `LayersPanel`: a tree with groups, masks and effects; no active layer; a filtered
//! tree; extra columns.

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::LayersPanel;
use crate::ui::filter::{FilterBy, LayerFilter};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::{LayerPart, LayerSelection};

/// The sample tree with the masked layer active and its mask targeted.
fn selected() -> FakeLayers {
    let mut tree = FakeLayers::sample();
    tree.selection = LayerSelection { layers: vec![3], active: Some(3), target: LayerPart::LayerMask };
    tree
}

#[story(LayersPanel<'static, FakeLayers>, covers = [Default], drive = Drive::Never)]
fn layers_panel_tree(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    LayersPanel::new(&tree).show(ui).into()
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_no_active_layer(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::sample();
    LayersPanel::new(&tree).show(ui).into()
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_filtered(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Name, name: String::from("o"), ..LayerFilter::default() });
    LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui).into()
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_extra_columns(ui: &mut Ui, _data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    LayersPanel::new(&tree).lock_column(true).show(ui).into()
}
