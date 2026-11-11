//! Stories of `Slicer`.

use super::{Slicer, SlicerCard, SlicerTile, SlicerTiles, Values};
use craft_ui::contract::{Props as _, ShowCollection as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, show_as_only_item, story};
use egui::Ui;

fn colours() -> Vec<(String, bool)> {
    ["Red", "Green", "Blue", "Cyan", "Magenta"].iter().map(|name| ((*name).to_owned(), *name == "Green")).collect()
}

#[story(Slicer<'static>, covers = [Default], drive = Drive::Never)]
fn slicer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let items = colours();
    let rect = ui.available_rect_before_wrap();
    Slicer::new("Colour", &items).columns(2).filtered(true).show(ui, rect).into()
}

fn card(ui: &mut Ui, filtered: bool) -> StoryOutput {
    let room = ui.available_rect_before_wrap();
    SlicerCard::new("Colour".into(), filtered).show_at_taking_room(ui, room, |_, _| {}).into()
}

#[story(SlicerCard, covers = [Default], drive = Drive::Never)]
fn slicer_card(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    card(ui, false)
}

#[story(SlicerCard, covers = [], drive = Drive::Never)]
fn slicer_card_filtered(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    card(ui, true)
}

fn tiles(ui: &mut Ui, values: &[(String, bool)]) -> StoryOutput {
    let source = Values(values);
    SlicerTiles::new(2, 1.0).label("Colour").show_items(ui, &source, &mut |key, _| SlicerTile::new(values[*key].0.clone()).on(values[*key].1)).into()
}

#[story(SlicerTiles, covers = [Empty], drive = Drive::Never)]
fn slicer_tiles_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, &[])
}

#[story(SlicerTiles, covers = [One], drive = Drive::Never)]
fn slicer_tiles_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, &colours()[..1])
}

#[story(SlicerTiles, covers = [Default, Many], drive = Drive::Never)]
fn slicer_tiles_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    tiles(ui, &colours())
}

#[story(SlicerTile, covers = [Default], drive = Drive::Never)]
fn slicer_tile(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, SlicerTile::new("Red".to_owned())).into()
}

#[story(SlicerTile, covers = [], drive = Drive::Never)]
fn slicer_tile_on(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    show_as_only_item(ui, SlicerTile::new("Green".to_owned()).on(true)).into()
}
