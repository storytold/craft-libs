//! Stories of `Tile` and `ThumbnailSlider`.

use super::{
    BadgeKind, BadgeSlot, BadgeVisibility, CaptionPlace, CardText, StackLook, ThumbnailSlider, Tile, TileBadge, TileFlag, TileMark, TilePartEvent,
    TileStatus, TileZoom,
};
use crate::contract::{Props as _, Show as _, Validity};
use crate::list::{ThumbAspect, ToggleState};
use crate::runtime::env::Message;
use crate::style::DataColor;
use crate::testing::pictures::StoryPictures;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, pointer, story};
use craft_behavior::{IconId, Mixed};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::{Color32, Ui};

const CAP: f64 = 4.0;
const ZOOM_RANGE: TileZoom = TileZoom::bounded(0.5, 4.0, 0.25, 1.0);

/// The hover actions of the tile and grid action stories, in the order the tiles add them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "tileStory", story)]
pub(super) enum TileStoryCommand {
    #[command(label = "Rotate", icon = "rotate_cw")]
    Rotate,
    #[command(label = "Remove", icon = "trash")]
    Remove,
}

/// The action stories' actions, by the index `TilePartEvent::Action` reports. A variant is named as its label, so the
/// stories find an action by the variant's name.
pub(super) const ACTIONS: [TileStoryCommand; 2] = [TileStoryCommand::Rotate, TileStoryCommand::Remove];

#[story(Tile<'static>, covers = [Default], drive = Drive::Activate)]
fn tile(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("IMG_0001.jpg").detail("00:12").show(ui).into()
}

#[story(Tile<'static>, covers = [LongText], drive = Drive::Never)]
fn tile_long_text(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("IMG_20261101_123456_a_name_much_longer_than_the_tile_is_wide.jpeg").show(ui).into()
}

#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_badges_and_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let rating = data.value("rating", 3u8);
    let duplicate = TileBadge::new(BadgeSlot::StartBottom, BadgeKind::Text("Duplicate".into()), Tone::Warn, Some("Already in the bin".into()));
    let edited = TileBadge::new(BadgeSlot::EndBottom, BadgeKind::Glyph(IconId::new("flag")), Tone::Neutral, Some("Edited".into()));
    let out = Tile::new("IMG_0002.jpg")
        .thumb(ThumbAspect::new(3, 2), StoryPictures::of(ui.ctx()).thumb(2))
        .rating(*rating)
        .flag(TileFlag::Picked)
        .label_color(Srgba::rgb(0.86, 0.31, 0.24))
        .label_name("Red")
        .bar(BadgeVisibility::Always)
        .stack(4)
        .badge(duplicate)
        .badge(edited)
        .range((0.2, 0.7))
        .scrub(0.4)
        .show(ui);
    if let Some(TilePartEvent::Rate(stars)) = out.event {
        *rating = stars;
    }
    out.into()
}

/// A stack of pages shown as two offset sheets behind the picture instead of the ×N badge; the count stays in the
/// tile's accessible name.
#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_stack_sheets(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Scans.pdf").thumb(ThumbAspect::new(3, 4), StoryPictures::of(ui.ctx()).thumb(5)).stack(3).stack_look(StackLook::Sheets).show(ui).into()
}

/// Hovers the tile, so its actions show, and clicks the first.
fn click_action(fixture: &mut Fixture) {
    let Some(tile) = fixture.by_label("IMG_0005.jpg") else { return };
    fixture.frame(vec![pointer::moved(tile.center())]).idle(1);
    if let Some(action) = fixture.by_label(&format!("{:?}", ACTIONS[0])) {
        fixture.click(&action);
    }
}

/// Two hover actions in the picture's corner, shown while the pointer is over the tile or it has visible focus; the
/// readout names the last one run.
#[story(Tile<'static>, covers = [], drive = Drive::Custom(click_action))]
fn tile_actions(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let out = Tile::new("IMG_0005.jpg")
        .thumb(ThumbAspect::new(3, 2), StoryPictures::of(ui.ctx()).thumb(5))
        .action(IconId::new("rotate_cw"), TileStoryCommand::Rotate)
        .action(IconId::new("trash"), TileStoryCommand::Remove)
        .show(ui);
    let last = data.value("last", None::<String>);
    if let Some(TilePartEvent::Action(at)) = out.event
        && let Some(name) = ACTIONS.get(at)
    {
        *last = Some(format!("{name:?}"));
    }
    let readout = format!("Last action: {}", last.as_deref().unwrap_or("none"));
    StoryOutput::from(out).readout(readout)
}

/// An icon tile whose glyph takes a data hue, inked for contrast on the picture's surface.
#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_hue(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Quarterly report.pdf").icon(IconId::new("file")).hue(DataColor::new(Color32::from_rgb(196, 64, 52))).show(ui).into()
}

/// A page card (`CardText::Page`): a letter-size page on its shadow on a flat card, its name on up to two lines, as
/// VectorCraft's New Document presets and page templates show it.
#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_page(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Social Square Post 1080×1080").card().card_text(CardText::Page).detail("1080 × 1080 px").page(1080.0, 1080.0).show(ui).into()
}

/// A compact action card: icon and name, a description, and the action that activates the whole card.
#[story(Tile<'static>, covers = [], drive = Drive::Activate)]
fn tile_summary_card(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Arrange pages").icon(IconId::new("layout_grid")).detail("Move · Rotate · Delete").summary_action("Use now").card().show(ui).into()
}

fn click_check(fixture: &mut Fixture) {
    if let Some(check) = fixture.by_label("Select IMG_0003.jpg") {
        fixture.click(&check);
    }
}

#[story(Tile<'static>, covers = [], drive = Drive::Custom(click_check))]
fn tile_check_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let checked = data.value("checked", false);
    let state = if *checked { ToggleState::On } else { ToggleState::Off };
    let counter = TileBadge::new(BadgeSlot::StartBottom, BadgeKind::Text("1/3".into()), Tone::Accent, None);
    let out = Tile::new("IMG_0003.jpg").check(state).badge(counter).show(ui);
    if matches!(out.event, Some(TilePartEvent::Check { .. })) {
        *checked = !*checked;
    }
    out.into()
}

#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_offline(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Interview_B.mov").status(TileStatus::Offline(None)).rejected().flag(TileFlag::Rejected).show(ui).into()
}

#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_unreadable_folder(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Scans").folder().status(TileStatus::Unreadable).show(ui).into()
}

#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_icon_face(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("Settings").icon(IconId::new("settings")).show(ui).into()
}

#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_start_caption(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("7")
        .caption(CaptionPlace::Start)
        .thumb(ThumbAspect::new(16, 9), StoryPictures::of(ui.ctx()).thumb(7))
        .hidden()
        .mark(TileMark::new(IconId::new("play"), "Has animations"))
        .show(ui)
        .into()
}

/// A disabled tile: its picture, badge, stars and label colour fade with it.
#[story(Tile<'static>, covers = [], drive = Drive::Never)]
fn tile_disabled(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    Tile::new("IMG_0006.jpg")
        .thumb(ThumbAspect::new(3, 2), StoryPictures::of(ui.ctx()).thumb(6))
        .rating(3)
        .label_color(Srgba::rgb(0.27, 0.63, 0.35))
        .badge(TileBadge::new(BadgeSlot::EndTop, BadgeKind::Text("Duplicate".into()), Tone::Neutral, None))
        .enabled(false)
        .show(ui)
        .into()
}

#[story(Tile<'static>, covers = [], disabled_text = "The file is not available")]
fn tile_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("A duplicate of IMG_0001.jpg");
    Tile::new("IMG_0004.jpg").tooltip(tooltip).show(ui).into()
}

fn larger(fixture: &mut Fixture) {
    if let Some(button) = fixture.by_label("Larger Thumbnails") {
        fixture.click(&button);
    }
}

#[story(ThumbnailSlider<'static>, covers = [Default], drive = Drive::Custom(larger))]
fn thumbnail_slider(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ThumbnailSlider::new("Thumbnail Size", data.bind("zoom", 1.0), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [], drive = Drive::Custom(larger))]
fn thumbnail_slider_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ThumbnailSlider::new("Thumbnail Size", data.bind("zoom", 1.0), ZOOM_RANGE).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [Mixed], drive = Drive::Never)]
fn thumbnail_slider_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ThumbnailSlider::new("Thumbnail Size", data.bind_mixed("mixed", Mixed::Mixed), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [Invalid], drive = Drive::Never)]
fn thumbnail_slider_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |zoom: &f64| if *zoom > CAP { Validity::Refuse(Message::new("Too large for this grid")) } else { Validity::Ok };
    ThumbnailSlider::new("Thumbnail Size", data.bind("over", 5.0).check(refuse_over), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [], disabled_text = "No tiles to size")]
fn thumbnail_slider_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The size of the tiles");
    ThumbnailSlider::new("Thumbnail Size", data.bind("zoom", 1.0), TileZoom::Ladder).tooltip(tooltip).show(ui).into()
}
