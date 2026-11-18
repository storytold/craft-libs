//! Stories of `ZoomField`.

use super::{ZoomField, ZoomScale};
use crate::contract::{GlyphLabel, LabelKind, Props as _, ShowContainer as _, SideLabel, Sizing};
use crate::form::Field;
use crate::testing::{Drive, StoryData, StoryOutput, story};
use craft_behavior::kurbo::{Point, Size};
use craft_behavior::{CanvasView, IconId};
use egui::{Pos2, Rect, Ui};

/// The document the field zooms, in document units.
const DOCUMENT: Size = Size::new(2000.0, 1500.0);

/// A view of [`DOCUMENT`] at two thirds of its size, centred.
fn view() -> CanvasView {
    let canvas = Rect::from_min_max(Pos2::ZERO, Pos2::new(800.0, 600.0));
    CanvasView::new(canvas, Point::new(1000.0, 750.0), 0.6667)
}

/// Shows `field` over the story's view and writes the view it lands on back into the story data.
fn shown(ui: &mut Ui, data: &mut StoryData, field: impl FnOnce(CanvasView) -> ZoomField) -> StoryOutput {
    let view = data.value("view", view());
    let out = field(*view).show(ui);
    if let Some(landed) = out.view.landed() {
        *view = *landed;
    }
    out.into()
}

#[story(ZoomField, covers = [Default], drive = Drive::Type("200"))]
fn zoom_field(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, data, |view| ZoomField::new("Zoom", Some(view), DOCUMENT))
}

/// The zoom on the document-point scale: 100 % is the document's actual size at any screen scale.
#[story(ZoomField, covers = [], drive = Drive::Type("100"))]
fn zoom_field_points(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, data, |view| ZoomField::new("Zoom", Some(view), DOCUMENT).scale(ZoomScale::Points))
}

#[story(ZoomField, covers = [], drive = Drive::Step)]
fn zoom_field_stepper(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown(ui, data, |view| ZoomField::new("Zoom", Some(view), DOCUMENT).stepper(true))
}

/// The field inside a field row whose glyph label scrubs the zoom, as the status bar shows it.
#[story(ZoomField, covers = [], drive = Drive::Never)]
fn zoom_field_scrub_by_label(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let glyph = SideLabel::new().kind(LabelKind::Glyph(GlyphLabel::icon(IconId::new("zoom_in")))).scrub(true);
    let row = Field::new("Zoom")
        .side(glyph)
        .sizing(Sizing::Content)
        .show(ui, |ui, _| shown(ui, data, |view| ZoomField::new("Zoom", Some(view), DOCUMENT).scrub(true)));
    row.inner.unwrap_or_default()
}

/// No view is known: the field is empty and disabled.
#[story(ZoomField, covers = [Empty], drive = Drive::Never)]
fn zoom_field_no_view(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    ZoomField::new("Zoom", None, DOCUMENT).show(ui).into()
}
