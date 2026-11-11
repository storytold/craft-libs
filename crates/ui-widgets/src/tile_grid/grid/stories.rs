//! Stories of `TileGrid`.

use super::{GridEvent, TileGrid, TileGridOutput, TileGroup};
use crate::contract::ShowCollection as _;
use crate::list::ThumbAspect;
use crate::testing::pictures::StoryPictures;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use crate::tile_grid::{BadgeKind, BadgeSlot, Tile, TileBadge, TileFlag};
use craft_behavior::{DisplayOrder, ItemSource, SizeHint};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::Ui;

/// The keys of the group header rows, apart from the tiles' own.
const GROUP_KEYS: [usize; 3] = [1000, 1001, 1002];

/// Tiles keyed by position, each with a picture aspect that cycles through the usual photo shapes.
struct Photos(usize);

/// Width over height of the picture of tile `at`.
fn aspect(at: usize) -> f32 {
    [1.5, 1.0, 0.75, 1.78][at % 4]
}

/// [`aspect`] as a thumbnail shape.
fn thumb_aspect(at: usize) -> ThumbAspect {
    [ThumbAspect::new(3, 2), ThumbAspect::SQUARE, ThumbAspect::new(3, 4), ThumbAspect::new(16, 9)][at % 4]
}

/// A hash of `key` that spreads small keys.
fn mix(key: usize) -> usize {
    (key.wrapping_mul(2_654_435_761) >> 7) & 0xff_ffff
}

/// The label colours the photos cycle through.
const LABELS: [Srgba; 4] = [Srgba::rgb(0.86, 0.27, 0.24), Srgba::rgb(0.9, 0.67, 0.16), Srgba::rgb(0.27, 0.63, 0.35), Srgba::rgb(0.27, 0.47, 0.86)];

/// The library's ratings, one per photo, kept in the story's data: the first `count` start from the key, and a
/// rating the grid reports replaces them.
struct Ratings<'d>(&'d mut Vec<u8>);

impl<'d> Ratings<'d> {
    fn of(data: &'d mut StoryData, count: usize) -> Self {
        Ratings(data.value("ratings", (0..count).map(|key| (mix(key) % 6) as u8).collect()))
    }

    fn apply(&mut self, events: &[GridEvent<usize>]) {
        for event in events {
            if let GridEvent::Rate { keys, rating } = event {
                for key in keys {
                    self.0[*key] = *rating;
                }
            }
        }
    }
}

/// Photo `key` rated `rating`: its picture, and from the key the odd pick, reject, colour label and Duplicate badge.
fn photo(pictures: &StoryPictures, key: usize, rating: u8) -> Tile<'static> {
    let mut tile = Tile::new(format!("Photo {}", key + 1)).thumb(thumb_aspect(key), pictures.thumb(key as u64)).rating(rating);
    if key.is_multiple_of(11) {
        tile = tile.flag(TileFlag::Picked);
    }
    if key.is_multiple_of(17) {
        tile = tile.flag(TileFlag::Rejected).rejected();
    }
    if key.is_multiple_of(5) {
        tile = tile.label_color(LABELS[key / 5 % 4]);
    }
    if key.is_multiple_of(13) {
        tile = tile.badge(TileBadge::new(
            BadgeSlot::EndTop,
            BadgeKind::Text("Duplicate".into()),
            Tone::Neutral,
            Some("Duplicate of another photo".into()),
        ));
    }
    tile
}

impl DisplayOrder<usize> for Photos {
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

impl ItemSource<usize> for Photos {
    fn size_hint(&self, at: usize) -> SizeHint {
        SizeHint::Aspect(aspect(at))
    }
}

fn grid(ui: &mut Ui, data: &mut StoryData, grid: TileGrid<'_, usize>, count: usize) -> TileGridOutput<usize> {
    let pictures = StoryPictures::of(ui.ctx());
    let mut ratings = Ratings::of(data, count);
    let out = grid.show_items(ui, &Photos(count), &mut |key, _| photo(&pictures, *key, ratings.0[*key]));
    ratings.apply(&out.events);
    out
}

#[story(TileGrid<'static, usize>, covers = [Default, Many], drive = Drive::Never)]
fn tile_grid_many(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 60).into()
}

#[story(TileGrid<'static, usize>, covers = [One], drive = Drive::Never)]
fn tile_grid_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 1).into()
}

#[story(TileGrid<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn tile_grid_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 0).into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_justified(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").justified(), 40).into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_groups(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = *data.value("open", [true, false, true]);
    let groups = [
        TileGroup::new(GROUP_KEYS[0], 0, 12, "October 2026").detail("12 photos").open(open[0]),
        TileGroup::new(GROUP_KEYS[1], 12, 8, "September 2026").detail("8 photos").open(open[1]),
        TileGroup::new(GROUP_KEYS[2], 20, 10, "August 2026").detail("10 photos").open(open[2]),
    ];
    let out = grid(ui, data, TileGrid::new("Library").groups(&groups).collapsible().zoom(2.0), 30);
    let open = data.value("open", open);
    for event in &out.events {
        if let GridEvent::GroupOpen { group, open: wanted } = event {
            open[*group] = *wanted;
        }
    }
    out.into()
}
