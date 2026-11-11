//! Stories of the Ruler tool, run live, and of its overlay and options bar.

use super::model::RulerPart;
use super::overlay::POINT;
use super::{RulerBar, RulerLine, RulerOverlay, RulerTool};
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, ToolStory, story, tool_story};
use egui::{Color32, Ui};

fn line() -> RulerLine {
    RulerLine::new(Point::new(220.0, 140.0), Point::new(420.0, 200.0))
}

/// The object the ruler measures, centred on the story document.
const OBJECT: Size = Size::new(220.0, 140.0);

/// The story document with the object the ruler measures, filled and outlined in the accent.
fn object(ui: &Ui) -> StoryPicture<impl Fn(Point) -> Color32 + use<>> {
    let document = StoryDocument::STANDARD;
    let object = StoryShape::filled(&Rect::from_center_size(document.centre(), OBJECT), accent(ui, 0.25)).outlined(OUTLINE, accent(ui, 1.0));
    let size = document.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(std::slice::from_ref(&object), Color32::TRANSPARENT, p))
}

/// Shows the overlay over `line` as the only canvas control of a canvas over the object.
fn on_canvas(ui: &mut Ui, line: &mut Option<RulerLine>) -> StoryOutput {
    on_picture(ui, &object(ui), |ui, at, _| RulerOverlay::new("Ruler", Bind::bound(&mut *line)).show_at(ui, at))
}

/// The Ruler tool live over the object: a press on the background draws a new line, Shift holds its end to 45
/// degrees, a line too short is cleared, the ends drag, and Alt from an end draws the protractor arm. The line the
/// tool and its overlay report is the story's document.
#[story(RulerOverlay<'static>, covers = [], drive = Drive::Never)]
fn ruler_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let picture = object(ui);
    let body = |ui: &mut Ui, at: egui::Rect, _: &CanvasView, tool: &mut RulerTool, line: &Option<RulerLine>| tool.pass(*line).show_line(ui, at);
    tool_story(ui, data, ToolStory::new("Document", &picture), RulerTool::new, Some(line()), body, |run| {
        let _ = run.host.run(&mut run.tool.pass(*run.document));
        for event in run.tool.take_events() {
            let _ = event.apply(run.document);
        }
        StoryOutput::default()
    })
}

#[story(RulerOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(POINT.at(RulerPart::End.key()))))]
fn ruler_overlay_with_a_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data.value("line", Some(line())))
}

#[story(RulerOverlay<'static>, covers = [], drive = Drive::Never)]
fn ruler_overlay_with_a_protractor_arm(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    on_canvas(ui, data.value("arm", Some(line().with_protractor(Point::new(280.0, 260.0)))))
}

#[story(RulerBar<'static>, covers = [], drive = Drive::Never)]
fn ruler_bar_with_a_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let use_scale = data.value("scale", false);
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| output = Some(RulerBar::new(Some(line()), Bind::bound(&mut *use_scale)).show(ui)));
    output.map(StoryOutput::from).unwrap_or_default()
}

#[story(RulerBar<'static>, covers = [], drive = Drive::Never)]
fn ruler_bar_without_a_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let use_scale = data.value("none", false);
    let mut output = None;
    Bar::new("Options", BarKind::Options).show(ui, |ui, _| output = Some(RulerBar::new(None, Bind::bound(&mut *use_scale)).show(ui)));
    output.map(StoryOutput::from).unwrap_or_default()
}
