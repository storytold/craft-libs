//! Stories of the text frame marks.

use super::{FrameExtras, HiddenMark, PlacedMark, TextFrameMarks};
use crate::ui::fixtures::PAPER;
use craft_behavior::CanvasView;
use craft_behavior::kurbo::{Point, Rect};
use craft_ui::canvas::Canvas;
use craft_ui::contract::{Show as _, ShowContainer as _};
use craft_ui::style::DataColor;
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

/// The layer colour of the frames.
const LAYER: DataColor = DataColor::new(egui::Color32::from_rgb(0x2a, 0x7f, 0xff));

/// The ascent of the line the marks sit on, points.
const ASCENT: f64 = 12.0;

#[story(TextFrameMarks<'static>, covers = [Default], drive = Drive::Never)]
fn hidden_characters_and_extras(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let room = craft_ui::testing::stage(ui);
    let view = CanvasView::new(room, Point::new(100.0, 75.0), 1.0);
    let frames = [
        FrameExtras::new(Rect::new(0.0, 0.0, 120.0, 80.0)).threaded_out(true),
        FrameExtras::new(Rect::new(140.0, 0.0, 200.0, 80.0)).threaded_in(true).overset(true),
    ];
    let marks = [
        PlacedMark::new(HiddenMark::Space, Point::new(20.0, 30.0), true, ASCENT),
        PlacedMark::new(HiddenMark::Tab, Point::new(30.0, 30.0), false, ASCENT),
        PlacedMark::new(HiddenMark::LineBreak, Point::new(60.0, 30.0), false, ASCENT),
        PlacedMark::new(HiddenMark::ParagraphEnd, Point::new(80.0, 30.0), false, ASCENT),
        PlacedMark::new(HiddenMark::StoryEnd, Point::new(100.0, 30.0), false, ASCENT),
    ];
    Canvas::new("Frames")
        .view(view)
        .show_at(ui, room, |ui, body| {
            TextFrameMarks::new("Frame marks", view, &frames, LAYER, PAPER).hidden_characters(&marks).show_at(ui, body.rect)
        })
        .inner
        .map(StoryOutput::from)
        .unwrap_or_default()
}
