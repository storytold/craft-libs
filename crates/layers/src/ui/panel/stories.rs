//! Stories of `LayersPanel`: a tree with groups, masks and effects; no active layer; a filtered
//! tree; extra columns; effects and Smart Filters open; filtered by Artboard and by Group, which keep each group open or
//! closed.

use craft_ui::contract::Bind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

use super::{LayersOutput, LayersPanel};
use crate::ui::filter::{FilterBy, LayerFilter};
use crate::ui::mock::FakeLayers;
use crate::ui::tree::{LayerInfo, LayerPart, LayerSelection};
use craft_behavior::IconId;
use craft_ui::display::Icon;
use craft_ui::primitives::IconSpec;

/// The sample tree with the masked layer active and its mask targeted.
fn selected() -> FakeLayers {
    let mut tree = FakeLayers::sample();
    tree.selection = LayerSelection::new(vec![3], Some(3), LayerPart::LayerMask);
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
    // PhotoCraft's Hide Layers key, which the app hands the panel.
    let hide_layers = craft_keymap::Chord::new(craft_keymap::Mods::CMD, craft_keymap::Key::Comma);
    let shown = LayersPanel::new(&tree).visibility_key(hide_layers).show(ui);
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

/// Illustrator's target circle after each row's indicators: filled while the layer is shown.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_target_cell(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let target = |_: &u32, info: &LayerInfo| Icon::new(IconSpec::svg(IconId::new(if info.visible { "target" } else { "circle" })));
    let shown = LayersPanel::new(&tree).trailing(target, |_, _| {}).show(ui);
    with_readout(data, shown)
}

#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_extra_columns(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = selected();
    let shown = LayersPanel::new(&tree).lock_column(true).show(ui);
    with_readout(data, shown)
}

/// The row kinds PhotoCraft adds: a compact artboard heading, a frame with its content thumbnail, a smart object's badge,
/// Smart Filter rows with their Blending Options buttons and one of them selected on its own, an artboard with a mask.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_row_kinds(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::row_kinds();
    let shown = LayersPanel::new(&tree).show(ui);
    with_readout(data, shown)
}

/// A frame selected as a whole: the brackets frame both its shape and its content.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_frame_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut tree = FakeLayers::row_kinds();
    tree.selection = LayerSelection::new(vec![2], Some(2), LayerPart::Pixels);
    let shown = LayersPanel::new(&tree).show(ui);
    with_readout(data, shown)
}

/// The sample tree with every layer's effects and Smart Filters open: each header's eye one step in from its layer,
/// each item's one step further, the eye column empty beside them, and no divider until the next layer.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_effects_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut tree = selected();
    for layer in &mut tree.layers {
        layer.effects_open = layer.effects.is_some() || layer.smart_filters.is_some();
    }
    let shown = LayersPanel::new(&tree).show(ui);
    with_readout(data, shown)
}

/// The artboard that holds the selected "Oak", with its contents.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_by_artboard(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::boards();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Artboard, ..LayerFilter::default() });
    let shown = LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui);
    with_readout(data, shown)
}

/// Every group with its contents; the closed groups stay closed.
#[story(LayersPanel<'static, FakeLayers>, covers = [], drive = Drive::Never)]
fn layers_panel_by_group(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tree = FakeLayers::fifty();
    let filter = data.value("filter", LayerFilter { by: FilterBy::Group, ..LayerFilter::default() });
    let shown = LayersPanel::new(&tree).filter(Bind::bound(filter)).show(ui);
    with_readout(data, shown)
}
