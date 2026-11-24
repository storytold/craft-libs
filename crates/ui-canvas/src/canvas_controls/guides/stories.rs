//! Stories of `Guides`.

use super::{GUIDE, Guides, GuidesOutput};
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_with, show_document, story_view};
use crate::canvas::{Canvas, Guide, GuideKey, GuideLine, RasterSource, RulerText, Rulers};
use crate::contract::{Bind, Props as _, Role, Show as _, ShowContainer as _, Validity};
use crate::runtime::env::{Message, UnitKind};
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, stage, story};
use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_behavior::{Axis, Mixed, RulerUnits};
use egui::{Id, Rect, Ui, Vec2};

/// The most guides the invalid story accepts.
const MOST_GUIDES: usize = 2;

/// Shows the guides `control` builds as the only canvas control of a canvas over the [`stage`] of `ui`, over
/// [`StoryDocument::STANDARD`] at 100 % centred on its middle, as `on_canvas` does; the control gets the canvas's rect
/// and its id, which the guides are keyed by.
fn on_guides_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, Rect, Id) -> GuidesOutput) -> StoryOutput {
    let canvas = ui.scope_id().with("guides story canvas");
    let document: &dyn RasterSource = &StoryDocument::STANDARD;
    let room = stage(ui);
    let size = document.size();
    let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = story_view(room, ui.pixels_per_point(), centre, 100.0);
    on_canvas_with(ui, room, Canvas::new("Document").id(canvas).view(view), Some(document), |ui, at| control(ui, at, canvas))
}

/// Every line a guide draws: solid, dotted and dashed, and a locked one.
fn guides() -> Vec<Guide> {
    let mut dotted = Guide::new(GuideKey::new(2), Axis::X, 120.0);
    dotted.line = GuideLine::Dots;
    let mut dashed = Guide::new(GuideKey::new(3), Axis::X, 280.0);
    dashed.line = GuideLine::Dashed;
    let mut locked = Guide::new(GuideKey::new(4), Axis::Y, 480.0);
    locked.locked = true;
    vec![Guide::new(GuideKey::new(1), Axis::Y, 160.0), dotted, dashed, locked]
}

/// Drags the first guide across itself by twice the drag threshold.
fn drag_first(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(line) = fixture.part(id, GuideKey::new(1).part(GUIDE)) else { return };
    let travel = 2.0 * crate::testing::input::slop(fixture.ctx());
    fixture.drag(&line, Vec2::new(travel, 0.0));
}

/// Drags a guide out of the top ruler's strip onto the document.
fn drag_out_of_the_top_ruler(fixture: &mut Fixture) {
    let Some(canvas) = fixture.by_role(Role::CANVAS).first().map(|target| target.rect) else { return };
    // Inside the top strip, clear of the corner.
    let from = egui::pos2(canvas.center().x, canvas.min.y + 4.0);
    fixture.drag_at(from, Vec2::new(0.0, canvas.height() / 4.0));
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_out_of_the_top_ruler))]
fn guide_dragged_out_of_a_ruler(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    let mut zero = Point::ZERO;
    let canvas = ui.scope_id().with("guides story canvas");
    let room = stage(ui);
    let size = StoryDocument::STANDARD.size();
    let centre = Point::new(f64::from(size.width) / 2.0, f64::from(size.height) / 2.0);
    let view = story_view(room, ui.pixels_per_point(), centre, 100.0);
    let text = RulerText::new(RulerUnits::pixels(), UnitKind::Pixels);
    let strips = Rulers::new("Rulers", Bind::bound(&mut zero)).interactive(true);
    let shown = Canvas::new("Document").id(canvas).view(view).with_rulers(Some(strips)).rulers(text, text).show_at(ui, room, |ui, body| {
        show_document(ui, body.rect, &StoryDocument::STANDARD);
        Guides::new("Guides", canvas, guides_at.bind(&mut *guides)).interactive(true).show_at(ui, body.rect)
    });
    let mut output: StoryOutput = shown.inner.map(Into::into).unwrap_or_default();
    // A click on a strip is the rulers' to report.
    output.clicked |= shown.output.rulers.is_some_and(|rulers| StoryOutput::from(rulers).clicked);
    output
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_over_a_canvas(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| {
        Guides::new("Guides", canvas, guides_at.bind(&mut *guides)).interactive(true).selected_guides(&[GuideKey::new(2)]).show_at(ui, at)
    })
}

#[story(Guides<'static>, covers = [Mixed], drive = Drive::Custom(drag_first))]
fn guides_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", Mixed::<Vec<Guide>>::Mixed);
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, guides_at.bind_mixed(&mut *guides)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Invalid], drive = Drive::Custom(drag_first))]
fn too_many_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    let few = |guides: &Vec<Guide>| {
        if guides.len() > MOST_GUIDES { Validity::Refuse(Message::new("There are too many guides")) } else { Validity::Ok }
    };
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, guides_at.bind(&mut *guides).check(few)).interactive(true).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn guides_that_only_draw(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, guides_at.bind(&mut *guides)).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Custom(drag_first))]
fn bounded_guides(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    let page = DocRect::new(120.0, 80.0, 520.0, 320.0);
    on_guides_canvas(ui, |ui, at, canvas| Guides::new("Guides", canvas, guides_at.bind(&mut *guides)).interactive(true).bounds(page).show_at(ui, at))
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Never)]
fn guides_with_several_selected(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| {
        Guides::new("Guides", canvas, guides_at.bind(&mut *guides))
            .interactive(true)
            .selected_guides(&[GuideKey::new(1), GuideKey::new(2)])
            .show_at(ui, at)
    })
}

#[story(Guides<'static>, covers = [Default], drive = Drive::Never)]
fn guides_with_a_key_guide(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (guides, guides_at) = data.value_at("guides", guides());
    on_guides_canvas(ui, |ui, at, canvas| {
        Guides::new("Guides", canvas, guides_at.bind(&mut *guides))
            .interactive(true)
            .selected_guides(&[GuideKey::new(2)])
            .key_guide(GuideKey::new(1))
            .show_at(ui, at)
    })
}
