//! Stories of `Tile` and `ThumbnailSlider`.

use super::{
    BadgeKind, BadgeSlot, BadgeVisibility, CaptionPlace, ThumbnailSlider, Tile, TileBadge, TileFlag, TileMark, TilePartEvent, TileStatus, TileZoom,
};
use crate::contract::{Bind, Props as _, Show as _, Validity};
use crate::list::{ThumbAspect, ToggleState};
use crate::runtime::env::Message;
use crate::testing::pictures::StoryPictures;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::{IconId, Mixed};
use craft_theme::Srgba;
use craft_theme::variant::Tone;
use egui::Ui;

const CAP: f64 = 4.0;
const ZOOM_RANGE: TileZoom = TileZoom::bounded(0.5, 4.0, 0.25, 1.0);

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
    ThumbnailSlider::new("Thumbnail Size", Bind::bound(data.value("zoom", 1.0)), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [], drive = Drive::Custom(larger))]
fn thumbnail_slider_range(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ThumbnailSlider::new("Thumbnail Size", Bind::bound(data.value("zoom", 1.0)), ZOOM_RANGE).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [Mixed], drive = Drive::Never)]
fn thumbnail_slider_mixed(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    ThumbnailSlider::new("Thumbnail Size", Bind::bound_mixed(data.value("mixed", Mixed::Mixed)), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [Invalid], drive = Drive::Never)]
fn thumbnail_slider_invalid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_over = |zoom: &f64| if *zoom > CAP { Validity::Refuse(Message::new("Too large for this grid")) } else { Validity::Ok };
    ThumbnailSlider::new("Thumbnail Size", Bind::bound(data.value("over", 5.0)).check(refuse_over), TileZoom::Ladder).show(ui).into()
}

#[story(ThumbnailSlider<'static>, covers = [], disabled_text = "No tiles to size")]
fn thumbnail_slider_with_reason(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let tooltip = data.tooltip("The size of the tiles");
    ThumbnailSlider::new("Thumbnail Size", Bind::bound(data.value("zoom", 1.0)), TileZoom::Ladder).tooltip(tooltip).show(ui).into()
}
