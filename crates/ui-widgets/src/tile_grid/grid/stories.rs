//! Stories of `TileGrid`.

use super::{GridEvent, GridZoom, RatingKeys, TileGrid, TileGridOutput, TileGroup};
use crate::contract::{Bind, CollectionEvent, MiddleButton, ShowCollection as _, SpaceAction};
use crate::list::ThumbAspect;
use crate::runtime::env::Label;
use crate::testing::pictures::StoryPictures;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use crate::tile_grid::stories::{ACTIONS, TileStoryCommand};
use crate::tile_grid::{BadgeKind, BadgeSlot, Tile, TileActionPlace, TileBadge, TileFlag, TilePartEvent, TileZoom};
use craft_behavior::{DisplayOrder, IconId, ItemSource, SizeHint};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::{Key, Modifiers, Ui, Vec2};

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

/// The grid with Bridge's keys (Cmd/Ctrl+digit ratings, Space previews, Cmd/Ctrl + = / - / 0 and Ctrl/Cmd with the
/// wheel zoom) over a ladder zoom bound to the story's data, starting at `zoom`.
fn grid(ui: &mut Ui, data: &mut StoryData, grid: TileGrid<'_, usize>, count: usize, zoom: f64) -> TileGridOutput<usize> {
    let pictures = StoryPictures::of(ui.ctx());
    // The grid writes a copy of the story's zoom, which goes back after the pass, so the ratings can borrow the data.
    let mut held = *data.value("zoom", zoom);
    let bound = GridZoom::Bound(Bind::bound(&mut held), TileZoom::Ladder);
    let grid = grid.rating_keys(RatingKeys::CommandDigits).space(SpaceAction::Preview).zoom(bound);
    let mut ratings = Ratings::of(data, count);
    let out = grid.show_items(ui, &Photos(count), &mut |key, _| photo(&pictures, *key, ratings.0[*key]));
    ratings.apply(&out.events);
    *data.value("zoom", zoom) = held;
    out
}

#[story(TileGrid<'static, usize>, covers = [Default, Many], drive = Drive::Never)]
fn tile_grid_many(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 60, 1.0).into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_delete_under_pointer(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tiles = TileGrid::new("Library").deletable(true).delete_target(crate::contract::DeleteTarget::FocusOrHover);
    grid(ui, data, tiles, 20, 1.0).into()
}

#[story(TileGrid<'static, usize>, covers = [One], drive = Drive::Never)]
fn tile_grid_one(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 1, 1.0).into()
}

#[story(TileGrid<'static, usize>, covers = [Empty], drive = Drive::Never)]
fn tile_grid_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").empty("No photos"), 0, 1.0).into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_justified(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").justified(), 40, 1.0).into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_groups(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let open = *data.value("open", [true, false, true]);
    let groups = [
        TileGroup::new(GROUP_KEYS[0], 0, 12, "October 2026").detail("12 photos").open(open[0]),
        TileGroup::new(GROUP_KEYS[1], 12, 8, "September 2026").detail("8 photos").open(open[1]),
        TileGroup::new(GROUP_KEYS[2], 20, 10, "August 2026").detail("10 photos").open(open[2]),
    ];
    let out = grid(ui, data, TileGrid::new("Library").groups(&groups).collapsible(), 30, 1.0);
    let open = data.value("open", open);
    for event in &out.events {
        if let GridEvent::GroupOpen { group, open: wanted } = event {
            open[*group] = *wanted;
        }
    }
    out.into()
}

#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Never)]
fn tile_grid_rule_heads(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let groups = [
        TileGroup::new(GROUP_KEYS[0], 0, 12, "October 2026"),
        TileGroup::new(GROUP_KEYS[1], 12, 8, "September 2026"),
        TileGroup::new(GROUP_KEYS[2], 20, 10, "August 2026"),
    ];
    grid(ui, data, TileGrid::new("Library").groups(&groups).group_heads(super::GroupHeads::Rule), 30, 1.0).into()
}

/// Focuses the grid and steps its zoom in.
fn zoom_in(fixture: &mut Fixture) {
    fixture.key(Key::Tab, Modifiers::NONE).idle(1);
    fixture.key(Key::Equals, Modifiers::COMMAND).idle(2);
}

/// A zoom the grid steps itself: Cmd/Ctrl + = / - / 0 and Ctrl/Cmd with the wheel, keeping the tile under the pointer
/// in place; the readout shows the bound factor.
#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Custom(zoom_in))]
fn tile_grid_bound_zoom(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let out = grid(ui, data, TileGrid::new("Library"), 40, 1.0);
    let zoom = *data.value("zoom", 1.0);
    StoryOutput::from(out).readout(format!("Tile size: {zoom:.2}×"))
}

/// Pages of the band story.
const PAGES: usize = 12;

/// Hovers the second page, so its actions show, and clicks the first.
fn band_action(fixture: &mut Fixture) {
    let Some(page) = fixture.by_label("Page 2") else { return };
    fixture.frame(vec![pointer::moved(page.center())]).idle(1);
    if let Some(action) = fixture.by_label(&format!("{:?}", ACTIONS[0])) {
        fixture.click(&action);
    }
}

/// Page tiles whose hover actions sit in a band along the picture's lower edge, the place the grid gives every tile;
/// the readout names the last action and its page.
#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Custom(band_action))]
fn tile_grid_band_actions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let pictures = StoryPictures::of(ui.ctx());
    let grid = TileGrid::new("Pages").action_place(TileActionPlace::Band);
    let out = grid.show_items(ui, &Photos(PAGES), &mut |key, _| {
        Tile::new(format!("Page {}", key + 1))
            .thumb(ThumbAspect::new(3, 4), pictures.thumb(*key as u64))
            .action(IconId::new("rotate_cw"), TileStoryCommand::Rotate)
            .action(IconId::new("trash"), TileStoryCommand::Remove)
    });
    let last = data.value("last", None::<String>);
    for event in &out.response.events {
        if let CollectionEvent::Item(key, TilePartEvent::Action(at)) = event
            && let Some(name) = ACTIONS.get(*at)
        {
            *last = Some(format!("{name:?} page {}", key + 1));
        }
    }
    let readout = format!("Last action: {}", last.as_deref().unwrap_or("none"));
    StoryOutput::from(out).readout(readout)
}

/// Photos of the insert gap story.
const GAP_PHOTOS: usize = 12;

/// Clicks the gap before the third photo.
fn insert_gap(fixture: &mut Fixture) {
    if let Some(gap) = fixture.by_label("Insert before photo 3") {
        fixture.click(&gap);
    }
}

/// Insert gaps between the tiles: the gap before each tile in view and the end gap with the last tile in view, each
/// named by the display position it inserts before; the readout names the last one pressed.
#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Custom(insert_gap))]
fn tile_grid_insert_gaps(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let name = |at: usize| {
        if at == GAP_PHOTOS { Label::from("Insert at the end") } else { Label::from(format!("Insert before photo {}", at + 1)) }
    };
    let out = grid(ui, data, TileGrid::new("Library").insert_gaps(&name), GAP_PHOTOS, 1.0);
    let inserted = data.value("inserted", None::<usize>);
    for event in &out.events {
        if let GridEvent::Insert { at, .. } = event {
            *inserted = Some(*at);
        }
    }
    let readout = match *inserted {
        Some(at) => format!("Inserted at position {}", at + 1),
        None => "Nothing inserted".to_owned(),
    };
    StoryOutput::from(out).readout(readout)
}

/// How far below the press the pointer moves while the middle-button scroll is latched.
const LATCHED_MOVE: Vec2 = Vec2::new(0.0, 60.0);

/// A middle click in the grid latches auto-scroll, and the pointer below the latch point scrolls the grid down; Escape
/// ends the latch, so the driven story rests.
fn middle_scroll(fixture: &mut Fixture) {
    let at = fixture.shown().center();
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::middle(at, true)]);
    fixture.frame(vec![pointer::middle(at, false)]);
    fixture.frame(vec![pointer::moved(at + LATCHED_MOVE)]).idle(4);
    fixture.key(Key::Escape, Modifiers::NONE).settle();
}

/// The middle button latches auto-scroll instead of panning: the items step paints the latch mark at the press point
/// while the grid scrolls toward the pointer.
#[story(TileGrid<'static, usize>, covers = [], drive = Drive::Custom(middle_scroll))]
fn tile_grid_middle_button_scroll(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    grid(ui, data, TileGrid::new("Library").middle_button(MiddleButton::AutoScroll), 120, 1.0).into()
}
