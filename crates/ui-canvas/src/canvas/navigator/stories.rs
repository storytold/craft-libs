//! Stories of the Navigator and its zoom row.

use super::preview::WELL;
use super::{Navigator, NavigatorPaint, NavigatorPreview, NavigatorZoomRow};
use crate::contract::{Bind, Show as _, Validity};
use crate::runtime::env::Message;
use crate::testing::{Drive, Grip, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, Mixed};
use egui::{Color32, Pos2, Rect, Ui};

/// The document the row zooms, in document units.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// The canvas rect of the story's view, in points.
const CANVAS: Rect = Rect::from_min_max(Pos2::new(0.0, 0.0), Pos2::new(800.0, 600.0));

/// The grey of the picture an app paints, as a channel value.
const PICTURE_GRAY: u8 = 96;

/// The view centre past which the checked story refuses a view, in document units.
const CAP: f64 = 1200.0;

#[story(NavigatorZoomRow<'static>, covers = [], drive = Drive::Never)]
fn zoom_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    NavigatorZoomRow::new(Bind::bound(view), DOCUMENT).show(ui).into()
}

#[story(Navigator<'static>, covers = [Default], drive = Drive::Step)]
fn navigator(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    Navigator::new("Navigator", Bind::bound(view), DOCUMENT).show(ui).into()
}

#[story(Navigator<'static>, covers = [], drive = Drive::Drag(Grip::new(WELL)))]
fn navigator_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    Navigator::new("Navigator", Bind::bound(view), DOCUMENT).show(ui).into()
}

#[story(Navigator<'static>, covers = [Invalid], drive = Drive::Never)]
fn view_refused(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let refuse_right = |view: &CanvasView| if view.center.x > CAP { Validity::Refuse(Message::new("Too far right")) } else { Validity::Ok };
    let view = data.value("over", CanvasView::new(CANVAS, Point::new(1500.0, 750.0), 1.0));
    Navigator::new("Navigator", Bind::bound(view).check(refuse_right), DOCUMENT).show(ui).into()
}

#[story(Navigator<'static>, covers = [Mixed], drive = Drive::Never)]
fn views_that_differ(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", Mixed::<CanvasView>::Mixed);
    Navigator::new("Navigator", Bind::bound_mixed(view), DOCUMENT).show(ui).into()
}

#[story(Navigator<'static>, covers = [], drive = Drive::Never)]
fn painted_by_the_app(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let view = data.value("view", CanvasView::new(CANVAS, Point::new(1000.0, 750.0), 1.0));
    Navigator::new("Navigator", Bind::bound(view), DOCUMENT)
        .preview(NavigatorPreview::Paint(NavigatorPaint::new(|painter, rect| {
            painter.painter.rect_filled(rect, 0.0, Color32::from_gray(PICTURE_GRAY));
        })))
        .show(ui)
        .into()
}
