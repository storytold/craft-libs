//! Stories of the Artboard tool: the boards, the bar and the Properties section.

use super::{ArtboardBackground, ArtboardBar, ArtboardItem, ArtboardOverlay, ArtboardPreset, ArtboardProperties};
use craft_behavior::kurbo::{Rect, Size};
use craft_theme::Srgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn presets() -> Vec<ArtboardPreset> {
    vec![ArtboardPreset::new("Letter", Size::new(612.0, 792.0)), ArtboardPreset::new("Phone", Size::new(390.0, 844.0))]
}

fn boards() -> Vec<ArtboardItem> {
    vec![ArtboardItem::new("Artboard 1", Rect::new(0.0, 0.0, 180.0, 240.0)), ArtboardItem::new("Artboard 2", Rect::new(240.0, 0.0, 420.0, 240.0))]
}

#[story(ArtboardOverlay<'static>, covers = [Default])]
fn two_artboards(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (items, selected) = (boards(), [0_usize]);
    let stage = craft_ui::testing::stage(ui);
    let bounds = items.iter().fold(items[0].rect, |bounds, item| bounds.union(item.rect));
    let view = craft_ui::testing::canvas_story::story_view(stage, ui.pixels_per_point(), bounds.center(), 100.0);
    Canvas::new("Document").view(view).show_at(ui, stage, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &craft_ui::testing::canvas_story::StoryDocument::STANDARD);
        ArtboardOverlay::new("Artboards", &items, &selected).show_at(ui, body.rect)
    });
    StoryOutput::default()
}

#[story(ArtboardBar<'static>, covers = [Default], drive = Drive::Never)]
fn bar_for_a_landscape_board(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let board = ArtboardItem::new("Artboard 1", Rect::new(0.0, 0.0, 480.0, 270.0));
    let presets = presets();
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| ArtboardBar::new(Some(&board), &presets).show(ui)).output.into()
}

#[story(ArtboardProperties<'static>, covers = [Default], drive = Drive::Never)]
fn properties_with_a_custom_background(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let mut board = ArtboardItem::new("Artboard 1", Rect::new(0.0, 0.0, 390.0, 844.0));
    board.background = ArtboardBackground::Custom(Srgba::rgb(0.9, 0.4, 0.2));
    let presets = presets();
    ArtboardProperties::new(&board, &presets).show(ui);
    StoryOutput::default()
}
