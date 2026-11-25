//! Stories of the pasteboard.

use super::{ArtboardEdge, NAME, PageBorder, PageFrame, Pasteboard, PasteboardColor};
use crate::canvas::Canvas;
use crate::canvas::testing::canvas_story::{StoryDocument, on_canvas_view, on_canvas_with, story_view};
use crate::contract::Show as _;
use crate::primitives::Checker;
use crate::style::DataColor;
use crate::testing::{Drive, Fixture, StoryData, StoryOutput, stage, story};
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
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

/// The two artboards under `ArtboardEdge::Plain`: the active one edged in black, the other in mid grey at a hairline,
/// as a drawing app's artboards read against the paper.
#[story(Pasteboard<'static>, covers = [], drive = Drive::Never)]
fn artboards_with_plain_edges(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let pages: &[PageFrame] = data.value("pages", boards());
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| {
        Pasteboard::new("Pasteboard", pages).artboard_edge(ArtboardEdge::Plain).show_at(ui, at)
    })
}

#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn page_with_a_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::Line);
    let pages: &[PageFrame] = data.value("pages", vec![PageFrame::new(DocRect::new(60.0, 40.0, 300.0, 220.0))]);
    let room = stage(ui);
    let canvas = Canvas::new("Document").view(view(room, ui.pixels_per_point())).pasteboard(PasteboardColor::DarkGray);
    on_canvas_with(ui, room, canvas, Some(&StoryDocument::STANDARD), |ui, at| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

/// An artboard with loose layers beside it: the pasteboard leaves their rects open, so they show on the canvas.
#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn loose_layers(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", vec![PageFrame::new(DocRect::new(40.0, 40.0, 240.0, 200.0)).name("Artboard 1").active(true)]);
    let loose = [DocRect::new(280.0, 60.0, 400.0, 140.0), DocRect::new(200.0, 220.0, 320.0, 260.0)];
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| {
        Pasteboard::new("Pasteboard", pages).uncovered(&loose).border(border).show_at(ui, at)
    })
}

#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn turned_view(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, TURN);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", boards());
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

/// Double presses the name of the first page.
fn rename_first_name(fixture: &mut Fixture) {
    click_first_name(fixture);
    click_first_name(fixture);
}

/// A page with a transparency grid beside one with a plain fill.
#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn page_checker(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value(
        "pages",
        vec![
            PageFrame::new(DocRect::new(40.0, 40.0, 240.0, 200.0)).name("Transparent").active(true).checker(Checker::default()),
            PageFrame::new(DocRect::new(280.0, 40.0, 440.0, 160.0)).name("Filled").active(false).fill(DataColor::new(Color32::from_gray(u8::MAX))),
        ],
    );
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

/// Artboards whose surround lies under the pictures, so the document shows between and around them.
#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn artboards_show_outside(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", boards());
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| {
        Pasteboard::new("Pasteboard", pages).shows_outside().border(border).show_at(ui, at)
    })
}

/// A narrow page and a wide one at one zoom: the narrow page's name is cut to its width and the narrowest shows none.
#[story(Pasteboard<'static>, covers = [Default], drive = Drive::Never)]
fn long_page_names(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value(
        "pages",
        vec![
            PageFrame::new(DocRect::new(40.0, 40.0, 100.0, 200.0)).name("A very long artboard name").active(true),
            PageFrame::new(DocRect::new(120.0, 40.0, 130.0, 200.0)).name("Too narrow for any name").active(false),
            PageFrame::new(DocRect::new(150.0, 40.0, 440.0, 200.0)).name("A very long artboard name").active(false),
        ],
    );
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}

/// A double press on a page's name opens its rename field.
#[story(Pasteboard<'static>, covers = [Open], drive = Drive::Custom(rename_first_name))]
fn renaming_a_page_name(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = view_values(data, 0.0);
    let border = border_of(data, PageBorder::DropShadow);
    let pages: &[PageFrame] = data.value("pages", boards().into_iter().map(|page| page.renames(true)).collect::<Vec<_>>());
    on_canvas_view(ui, view, Some(&StoryDocument::STANDARD), |ui, at, _| Pasteboard::new("Pasteboard", pages).border(border).show_at(ui, at))
}
