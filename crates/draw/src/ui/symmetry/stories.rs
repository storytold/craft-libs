//! Stories of painting symmetry: the menu, the guides and the handles of Transform Symmetry.

use super::handles::{CENTRE, SYMMETRY};
use super::{Symmetry, SymmetryChoice, SymmetryHandles, SymmetryMenu, SymmetryMode, SymmetryOverlay, SymmetryPathRow};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::Size;
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::Ui;

/// The document the canvas shows, whose page the guides run over.
const SHOWN: craft_ui::testing::canvas_story::StoryDocument = craft_ui::testing::canvas_story::StoryDocument::STANDARD;
/// The page the guides run over, in document units: the document's.
const PAGE: Size = Size::new(640.0, 400.0);

/// The symmetry of the stories: Dual Axis at the page centre, turned a little.
fn sample(mode: SymmetryMode) -> Symmetry {
    Symmetry { mode, center: SHOWN.centre(), rotation: 20.0 }
}

/// A view that shows the page at 100 %, centred.
fn view(ui: &Ui, room: egui::Rect) -> CanvasView {
    SHOWN.view(room, ui.pixels_per_point(), 100.0)
}

#[story(SymmetryMenu<'static>, covers = [Default], drive = Drive::Never)]
fn symmetry_off(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choice = SymmetryChoice::Off;
    SymmetryMenu::new("Symmetry", &choice).show(ui);
    StoryOutput::default()
}

#[story(SymmetryMenu<'static>, covers = [Default], drive = Drive::Never)]
fn symmetry_on(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let choice = SymmetryChoice::Mode(SymmetryMode::Dual);
    let paths = [SymmetryPathRow::new("work", "Work Path")];
    SymmetryMenu::new("Symmetry", &choice).paths(&paths).can_transform(true).show(ui);
    StoryOutput::default()
}

#[story(SymmetryOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn dual_axis(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let symmetry = sample(SymmetryMode::Dual);
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryOverlay::new("Symmetry", &symmetry, PAGE).show_at(ui, body.rect);
    });
    StoryOutput::default()
}

#[story(SymmetryOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn editing_guides(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let symmetry = sample(SymmetryMode::Vertical);
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryOverlay::new("Symmetry", &symmetry, PAGE).editing(true).show_at(ui, body.rect);
    });
    StoryOutput::default()
}

#[story(SymmetryHandles<'static>, covers = [Default], drive = Drive::Drag(Grip::part(SYMMETRY.at(CENTRE))))]
fn handles_on_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let symmetry = data.value("symmetry", sample(SymmetryMode::Vertical));
    let room = craft_ui::testing::stage(ui);
    let view = view(ui, room);
    let shown = Canvas::new("Document").view(view).show_at(ui, room, |ui, body| {
        craft_ui::testing::canvas_story::show_document(ui, body.rect, &SHOWN);
        SymmetryHandles::new("Symmetry", Bind::bound(&mut *symmetry), PAGE).show_at(ui, body.rect)
    });
    shown.inner.map(|output| StoryOutput::from(output.edit)).unwrap_or_default()
}
