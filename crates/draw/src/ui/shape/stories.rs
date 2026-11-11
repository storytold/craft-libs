//! Stories of the shape panel and the options bar. A fake shape layer (a `LiveShape` and a `ShapeAppearance`) stands
//! for the document, and the panel and the bar edit it as an app does: each field's change is applied to the fake.

use super::{LiveShape, ShapeAppearance, ShapeBar, ShapeChange, ShapePanel, ShapeToolSettings};
use crate::ui::shape_drag::ShapeKind;
use craft_behavior::EditEvent;
use craft_color::ui::Paint;
use craft_theme::Srgba;
use craft_ui::bars::{Bar, BarKind};
use craft_ui::contract::{Bind, ShowContainer as _};
use craft_ui::testing::{Drive, StoryData, StoryOutput, story};
use egui::Ui;

fn look() -> ShapeAppearance {
    ShapeAppearance::new(Paint::Solid(Srgba::rgb(0.9, 0.2, 0.2)), Paint::None, 2.0)
}

fn rectangle() -> LiveShape {
    LiveShape::rect(320.0, 180.0, [8.0; 4])
}

fn polygon() -> LiveShape {
    LiveShape::polygon(200.0, 200.0, 0.0, 5, 0.5)
}

/// Applies `change` to the fake layer, as an app turns it into a document command.
fn apply(change: ShapeChange, shape: &mut LiveShape, look: &mut ShapeAppearance) {
    match (change, shape) {
        (ShapeChange::Fill(paint), _) => look.fill = paint,
        (ShapeChange::Stroke { paint, width, .. }, _) => {
            look.stroke = paint;
            look.stroke_width = width.unwrap_or(look.stroke_width);
        }
        (ShapeChange::StrokeWidth(width), _) => look.stroke_width = width,
        (ShapeChange::StrokeStyle { style, width, .. }, _) => {
            look.stroke_style = style;
            look.stroke_width = width.unwrap_or(look.stroke_width);
        }
        (
            ShapeChange::Size { width: w, height: h },
            LiveShape::Rect { width, height, .. } | LiveShape::Ellipse { width, height } | LiveShape::Polygon { width, height, .. },
        ) => {
            (*width, *height) = (w, h);
        }
        (ShapeChange::Radii(next), LiveShape::Rect { radii, .. }) => *radii = next,
        (ShapeChange::CornerRadius(next), LiveShape::Polygon { corner_radius, .. }) => *corner_radius = next,
        (ShapeChange::Sides(next), LiveShape::Polygon { sides, .. }) => *sides = next,
        (ShapeChange::StarRatio(next), LiveShape::Polygon { star_ratio, .. }) => *star_ratio = next,
        (ShapeChange::Weight(next), LiveShape::Line { weight, .. }) => *weight = next,
        _ => {}
    }
}

/// Applies the values the events carry.
fn apply_all(events: Vec<EditEvent<ShapeChange>>, shape: &mut LiveShape, look: &mut ShapeAppearance) {
    for event in events {
        if let Some(change) = event.value() {
            apply(change.clone(), shape, look);
        }
    }
}

/// The resolution of the stories' document.
const DOCUMENT_PIXELS_PER_INCH: f64 = 72.0;

fn panel(ui: &mut Ui, data: &mut StoryData, start: LiveShape) -> StoryOutput {
    let shape = data.value("shape", start).clone();
    let appearance = data.value("look", look()).clone();
    let mut open = *data.value("open", false);
    let out = ShapePanel::new("Shape properties", &shape, &appearance, DOCUMENT_PIXELS_PER_INCH).stroke_open(Bind::bound(&mut open)).show(ui);
    *data.value("open", false) = open;
    let (mut next, mut next_look) = (shape, appearance);
    apply_all(out.events, &mut next, &mut next_look);
    *data.value("shape", rectangle()) = next;
    *data.value("look", look()) = next_look;
    StoryOutput::default()
}

#[story(ShapePanel<'static>, covers = [Default], drive = Drive::Never)]
fn shape_panel_rect(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, rectangle())
}

#[story(ShapePanel<'static>, covers = [Default], drive = Drive::Never)]
fn shape_panel_polygon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, polygon())
}

#[story(ShapePanel<'static>, covers = [Default], drive = Drive::Never)]
fn shape_panel_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, LiveShape::line(3.0))
}

fn bar(ui: &mut Ui, data: &mut StoryData, kind: ShapeKind) -> StoryOutput {
    let settings = *data.value("settings", ShapeToolSettings::default());
    let appearance = data.value("look", look()).clone();
    let mut open = *data.value("open", false);
    let out = Bar::new("Shape options", BarKind::Options)
        .show(ui, |ui, _| ShapeBar::new(kind, &settings, &appearance, DOCUMENT_PIXELS_PER_INCH).stroke_open(Bind::bound(&mut open)).show(ui))
        .inner;
    *data.value("open", false) = open;
    let (mut shape, mut next_look) = (rectangle(), appearance);
    let mut next = settings;
    for event in out.into_iter().flat_map(|out| out.events) {
        match event.value() {
            Some(ShapeChange::CornerRadius(value)) => next.corner_radius = *value,
            Some(ShapeChange::Sides(value)) => next.sides = *value,
            Some(ShapeChange::Weight(value)) => next.line_weight = *value,
            Some(ShapeChange::FillOn(on)) => next.fill_on = *on,
            Some(change) => apply(change.clone(), &mut shape, &mut next_look),
            None => {}
        }
    }
    *data.value("settings", ShapeToolSettings::default()) = next;
    *data.value("look", look()) = next_look;
    StoryOutput::default()
}

#[story(ShapeBar<'static>, covers = [Default], drive = Drive::Never)]
fn shape_bar(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, ShapeKind::Rectangle)
}

#[story(ShapeBar<'static>, covers = [Default], drive = Drive::Never)]
fn shape_bar_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, ShapeKind::Line)
}
