//! Stories of `FreeBin`.

use super::{FreeBin, FreeformCard};
use craft_behavior::{DisplayOrder, ItemSource};
use craft_ui::contract::ShowCollection as _;
use craft_ui::list::ThumbAspect;
use craft_ui::testing::pictures::StoryPictures;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use craft_ui::tile_grid::Tile;
use egui::{Ui, vec2};

/// Cards keyed by position.
struct Cards(usize);

impl DisplayOrder<usize> for Cards {
    fn len(&self) -> usize {
        self.0
    }

    fn key(&self, at: usize) -> usize {
        at
    }

    fn index_of(&self, key: &usize) -> Option<usize> {
        (*key < self.0).then_some(*key)
    }
}

impl ItemSource<usize> for Cards {}

fn bin(ui: &mut Ui, bin: FreeBin<'_, usize>, count: usize) -> StoryOutput {
    let pictures = StoryPictures::of(ui.ctx());
    bin.show_items(ui, &Cards(count), &mut |key, _| {
        let tile = Tile::new(format!("Clip {}", key + 1)).thumb(ThumbAspect::new(1, 1), pictures.thumb(*key as u64));
        if *key == 1 { tile.stack(2) } else { tile }
    })
    .into()
}

#[story(FreeBin<'static, usize>, covers = [Default], drive = Drive::Never)]
fn free_bin(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = ui.max_rect().size();
    let at = |x: f32, y: f32| room * vec2(x, y);
    // Card 0 has no place and takes the first free cell of the flow; the others lie apart, the second row below the first.
    let cards = [
        FreeformCard::new(1, at(0.55, 0.08)).in_stack(1),
        FreeformCard::new(2, at(0.61, 0.13)).in_stack(1),
        FreeformCard::new(3, at(0.06, 0.55)),
        FreeformCard::new(4, at(0.55, 0.55)),
    ];
    bin(ui, FreeBin::new("Footage", &cards), 5)
}

#[story(FreeBin<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn free_bin_empty(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bin(ui, FreeBin::new("Footage", &[]).empty("No clips"), 0)
}

#[story(FreeBin<'static, usize>, covers = [Many], drive = Drive::Never)]
fn free_bin_many(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bin(ui, FreeBin::new("Footage", &[]).zoom(0.5), 60)
}

#[story(FreeBin<'static, usize>, covers = [One], drive = Drive::Never)]
fn free_bin_one(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    bin(ui, FreeBin::new("Footage", &[]), 1)
}
