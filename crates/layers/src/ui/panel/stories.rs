//! Stories of `LayersPanel`: a tree with groups, masks and effects; no active layer; a filtered
//! tree; extra columns.

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{LayersOutput, LayersPanel};
use crate::ui::filter::{FilterBy, LayerFilter};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::{LayerPart, LayerSelection};

/// The sample tree with the masked layer active and its mask targeted.
fn selected() -> FakeLayers {
    let mut tree = FakeLayers::sample();
    tree.selection = LayerSelection { layers: vec![3], active: Some(3), target: LayerPart::LayerMask };
    tree
}

/// The panel's output with the readout of its latest event, kept in the story's data across passes.
fn with_readout(data: &mut StoryData, shown: LayersOutput<u32>) -> StoryOutput {
    let last = data.note("last", None::<String>);
    if let Some(event) = shown.events.last() {
        *last = Some(format!("{event:?}"));
    }
    let readout = last.as_ref().map_or_else(|| "No event yet".to_owned(), |event| format!("Last event: {event}"));
    StoryOutput::from(shown).readout(readout)
}

#[story(LayersPanel<'static, FakeLayers>, covers = [Default], drive = Drive::Never)]
fn layers_panel_tree(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let shown = LayersPanel::new(&tree).show(ui);
    with_readout(data, shown)
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_no_active_layer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::sample();
    let shown = LayersPanel::new(&tree).show(ui);
    with_readout(data, shown)
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_filtered(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Name, name: String::from("o"), ..LayerFilter::default() });
    let shown = LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui);
    with_readout(data, shown)
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_extra_columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let shown = LayersPanel::new(&tree).lock_column(true).show(ui);
    with_readout(data, shown)
}
