//! Stories of `Slicer`.

use super::{Slicer, SlicerCard, SlicerTile, SlicerTiles, Values};
use craft_behavior::Proposal;
use craft_ui::contract::{CollectionResponse, Props as _, ShowCollection as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, stage_fitting, stage_width, story};
use egui::Ui;

fn colours() -> Vec<(String, bool)> {
    ["Red", "Green", "Blue", "Cyan", "Magenta"].iter().map(|name| ((*name).to_owned(), *name == "Green")).collect()
}

/// Six regions, every other one on.
fn regions() -> Vec<(String, bool)> {
    ["North", "South", "East", "West", "Central", "Export"].iter().enumerate().map(|(at, name)| ((*name).to_owned(), at % 2 == 0)).collect()
}

#[story(Slicer<'static>, covers = [Default], drive = Drive::Never)]
fn slicer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let items = regions();
    let filtered = items.iter().any(|(_, on)| !on);
    let slicer = Slicer::new("Region", &items).columns(2).filtered(filtered);
    let height = slicer.height_for(ui, stage_width(ui));
    let rect = stage_fitting(ui, |_| height);
    slicer.show(ui, rect).into()
}

/// The card with the six regions' tiles in two columns, as tall as the tiles at the stage's width, as `slicer` shows it.
fn card(ui: &mut Ui, filtered: bool) -> StoryOutput {
    let items = regions();
    let mut card = SlicerCard::new("Region".into(), filtered);
    let height = card.measure_in(ui, Proposal::Width(stage_width(ui)), |ui, _| tiles(ui, "Region", &items)).y;
    let rect = stage_fitting(ui, |_| height);
    card.show_at(ui, rect, |ui, _| tiles(ui, "Region", &items)).into()
}

#[story(SlicerCard, covers = [Default], drive = Drive::Never)]
fn slicer_card(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    card(ui, false)
}

#[story(SlicerCard, covers = [], drive = Drive::Never)]
fn slicer_card_filtered(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    card(ui, true)
}

/// `values` as tiles in two columns, named `label`.
fn tiles(ui: &mut Ui, label: &'static str, values: &[(String, bool)]) -> CollectionResponse<usize, ()> {
    let source = Values(values);
    SlicerTiles::new(2, 1.0).label(label).show_items(ui, &source, &mut |key, _| SlicerTile::new(values[*key].0.clone()).on(values[*key].1))
}

#[story(SlicerTiles, covers = [Empty], drive = Drive::Never)]
fn slicer_tiles_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, "Colour", &[]).into()
}

#[story(SlicerTiles, covers = [One], drive = Drive::Never)]
fn slicer_tiles_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, "Colour", &colours()[..1]).into()
}

#[story(SlicerTiles, covers = [Default, Many], drive = Drive::Never)]
fn slicer_tiles_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, "Colour", &colours()).into()
}

#[story(SlicerTile, covers = [Default], drive = Drive::Never)]
fn slicer_tile(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, SlicerTile::new("Red".to_owned())).into()
}

#[story(SlicerTile, covers = [], drive = Drive::Never)]
fn slicer_tile_on(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, SlicerTile::new("Green".to_owned()).on(true)).into()
}
