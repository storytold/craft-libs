//! Stories of the Ruler tool's overlay and options bar.

use super::model::RulerPart;
use super::overlay::POINT;
use super::{RulerBar, RulerLine, RulerOverlay};
use craft_behavior::kurbo::{Point, Rect, Size};
use craft_ui::bars::{Bar, BarKind};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui};

fn line() -> RulerLine {
    RulerLine::new(Point::new(220.0, 140.0), Point::new(420.0, 200.0))
}

/// The object the ruler measures, centred on the story document.
const OBJECT: Size = Size::new(220.0, 140.0);

/// Shows the overlay over `line` as the only canvas control of a canvas over the object, filled and outlined in the
/// accent.
fn on_canvas(ui: &mut Ui, line: &mut Option<RulerLine>) -> StoryOutput {
    let document = StoryDocument::STANDARD;
    let object = StoryShape::filled(&Rect::from_center_size(document.centre(), OBJECT), accent(ui, 0.25)).outlined(OUTLINE, accent(ui, 1.0));
    let size = document.size();
    let picture = StoryPicture::new(size.width, size.height, move |p| paint_shapes(std::slice::from_ref(&object), Color32::TRANSPARENT, p));
    on_picture(ui, &picture, |ui, at, _| RulerOverlay::new("Ruler", Bind::bound(&mut *line)).show_at(ui, at))
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
