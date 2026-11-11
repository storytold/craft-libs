//! Stories of the pasteboard.

use super::{NAME, PageBorder, PageFrame, Pasteboard, PasteboardColor};
use crate::canvas::testing::canvas_story::{on_canvas_view, story_view};
use crate::contract::Show as _;
use crate::style::DataColor;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, story};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect as DocRect};
use egui::{Color32, Rect, Ui};

/// Degrees the turned story rotates the view by.
const TURN: f64 = 15.0;

/// The middle of the boards, where the views look.
const BOARDS_CENTRE: Point = Point::new(240.0, 120.0);

fn boards() -> Vec<PageFrame> {
    vec![
        PageFrame::new(DocRect::new(40.0, 40.0, 240.0, 200.0)).name("Artboard 1").active(true),
        PageFrame::new(DocRect::new(280.0, 40.0, 440.0, 160.0)).name("Artboard 2").active(false).fill(DataColor::new(Color32::from_gray(u8::MAX))),
    ]
}

/// Clicks the name of the first page.
fn click_first_name(fixture: &mut Fixture) {
    let Some(id) = fixture.root_id() else { return };
    let Some(name) = fixture.part(id, NAME.at(0)) else { return };
    fixture.click(&name);
}

/// The view values the Properties panel edits: zoom in percent, rotation in degrees, flip and pixel aspect.
fn view_values(data: &mut StoryData, rotation: f64) -> impl Fn(Rect, f32) -> CanvasView + Copy + use<> {
    let zoom = *data.ranged("zoom", 100.0, 10.0..=800.0);
    let rotation = *data.ranged("rotation", rotation, -180.0..=180.0);
    let flip = *data.value("flip", false);
    let aspect = *data.ranged("aspect", 1.0, 0.5..=2.0);
    move |room, ppp| story_view(room, ppp, BOARDS_CENTRE, zoom).rotation(rotation).flip_x(flip).aspect(aspect)
}

/// The border round each page, from the story's `border` value.
fn border_of(data: &mut StoryData, init: PageBorder) -> PageBorder {
    *data.choice("border", init, &[("Drop shadow", PageBorder::DropShadow), ("Line", PageBorder::Line), ("None", PageBorder::None)])
}

#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Custom(click_first_name))]
fn artboards(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", boards());
    on_canvas_view(ui, view, None, |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn page_with_a_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::Line);
    let pages: &[PageFrame] = data.value("pages", vec![PageFrame::new(DocRect::new(60.0, 40.0, 300.0, 220.0))]);
    on_canvas_view(ui, view, None, |ui, at, _| Pasteboard::new("Pasteboard", pages).color(PasteboardColor::DarkGray).border(border).show_at(ui, at))
}

#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn turned_view(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, TURN);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", boards());
    on_canvas_view(ui, view, None, |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}
