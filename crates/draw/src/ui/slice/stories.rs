//! Stories of the Slice tools: the slices in each mode and the bar in each mode.

use super::{SliceBar, SliceItem, SliceMode, SliceOrigin, SliceOverlay, SliceToolSettings};
use craft_behavior::PixelSize;
use craft_behavior::geometry::Constraint;
use craft_behavior::kurbo::Rect;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Entry, Show as _, ShowContainer as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// A document of 240 by 160 pixels at 72 pixels per inch.
const DOCUMENT: PixelSize = PixelSize::new(240, 160);
const PIXELS_PER_INCH: f64 = 72.0;
/// The document the canvas shows under the slices.
const SHOWN: craft_ui::testing::canvas_story::StoryDocument = craft_ui::testing::canvas_story::StoryDocument::new(DOCUMENT.width, DOCUMENT.height);

/// Two user slices, a layer slice and the auto slice between them.
fn slices() -> Vec<SliceItem> {
    vec![
        SliceItem::new(1, Some(1), Rect::new(0.0, 0.0, 120.0, 80.0), SliceOrigin::User),
        SliceItem::new(2, Some(2), Rect::new(120.0, 80.0, 240.0, 160.0), SliceOrigin::Layer),
        SliceItem::new(3, None, Rect::new(120.0, 0.0, 240.0, 80.0), SliceOrigin::Auto),
        SliceItem::new(4, Some(4), Rect::new(24.0, 96.0, 96.0, 144.0), SliceOrigin::User),
    ]
}

fn on_canvas(ui: &mut Ui, configure: impl for<'a> FnOnce(SliceOverlay<'a>) -> SliceOverlay<'a>) -> StoryOutput {
    let items = slices();
    let mut configure = Some(configure);
    let stage = craft_ui::testing::stage(ui);
    let _ = Canvas::new("Document").view(SHOWN.view(stage, ui.pixels_per_point(), 100.0)).show_at(ui, stage, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        configure.take().map(|configure| configure(SliceOverlay::new("Slices", &items)).show_at(ui, body.rect))
    });
    StoryOutput::default()
}

#[story(SliceOverlay<'static>, covers = [Default])]
fn slice_mode(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay)
}

#[story(SliceOverlay<'static>, covers = [Default])]
fn select_mode(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay.pick(true).selected_slice(4))
}

#[story(SliceOverlay<'static>, covers = [])]
fn locked_slices(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    on_canvas(ui, |overlay| overlay.pick(true).selected_slice(1).locked(true))
}

fn bar(ui: &mut Ui, data: &mut StoryData, mode: SliceMode, style: Constraint) -> StoryOutput {
    let settings = data.value("settings", SliceToolSettings::new(style));
    let units = [Entry::new(UnitKind::Pixels, "px"), Entry::new(UnitKind::Inches, "in")];
    Bar::new("Options", BarKind::Options)
        .show(ui, |ui, _| {
            SliceBar::new(mode, settings).size_units(UnitKind::Pixels, &units, PIXELS_PER_INCH, DOCUMENT).show(ui);
        })
        .output
        .into()
}

#[story(SliceBar<'static>, covers = [Default], drive = Drive::Never)]
fn slice_bar_slice_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Slice, Constraint::Free)
}

#[story(SliceBar<'static>, covers = [], drive = Drive::Never)]
fn slice_bar_fixed_size(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Slice, Constraint::Size(64.0, 64.0))
}

#[story(SliceBar<'static>, covers = [], drive = Drive::Never)]
fn slice_bar_select_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, SliceMode::Select, Constraint::Free)
}
