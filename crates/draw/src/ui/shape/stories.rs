//! Stories of the shape panel and the options bar. A fake shape layer (a `LiveShape` and a `ShapeAppearance`) stands
//! for the document, and the panel and the bar edit it as an app does: each field's change is applied to the fake.

use super::{
    Arrowheads, ArrowheadsPopover, BoxConstraintsPopover, LiveShape, ShapeAppearance, ShapeBar, ShapeChange, ShapePanel, ShapeToolSettings,
    StrokeCells, VectorMode, VectorModeSelect,
};
use crate::ui::shape_drag::{BoxConstraints, ShapeKind};
use craft_behavior::EditEvent;
use craft_color_ui::ui::Paint;
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
            LiveShape::Rect { width, height, .. }
            | LiveShape::Ellipse { width, height }
            | LiveShape::Polygon { width, height, .. }
            | LiveShape::Triangle { width, height, .. },
        ) => {
            (*width, *height) = (w, h);
        }
        (ShapeChange::Radii(next), LiveShape::Rect { radii, .. }) => *radii = next,
        (ShapeChange::CornerRadius(next), LiveShape::Polygon { corner_radius, .. } | LiveShape::Triangle { corner_radius, .. }) => {
            *corner_radius = next
        }
        (ShapeChange::Sides(next), LiveShape::Polygon { sides, .. }) => *sides = next,
        (ShapeChange::StarRatio(next), LiveShape::Polygon { star_ratio, .. }) => *star_ratio = next,
        (ShapeChange::Weight(next), LiveShape::Line { weight, .. }) => *weight = next,
        (ShapeChange::Arrowheads(next), LiveShape::Line { arrows, .. }) => *arrows = Some(next),
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
    let mut linked = *data.value("linked", false);
    let out = ShapePanel::new("Shape properties", &shape, &appearance, DOCUMENT_PIXELS_PER_INCH)
        .stroke_open(Bind::bound(&mut open))
        .arrow_default(Bind::bound(&mut linked))
        .show(ui);
    *data.value("open", false) = open;
    *data.value("linked", false) = linked;
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
fn shape_panel_triangle(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, LiveShape::triangle(240.0, 200.0, 0.0))
}

#[story(ShapePanel<'static>, covers = [Default], drive = Drive::Never)]
fn shape_panel_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    panel(ui, data, LiveShape::line(3.0))
}

#[story(ShapePanel<'static>, covers = [Default], drive = Drive::Never)]
fn shape_panel_arrow_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut arrows = Arrowheads::new();
    arrows.end = true;
    panel(ui, data, LiveShape::arrow_line(3.0, arrows))
}

fn bar(ui: &mut Ui, data: &mut StoryData, kind: ShapeKind, paint: bool) -> StoryOutput {
    let settings = *data.value("settings", ShapeToolSettings::default());
    let appearance = data.value("look", look()).clone();
    let mut open = *data.value("open", false);
    let out = Bar::new("Shape options", BarKind::Options)
        .show(ui, |ui, _| {
            ShapeBar::new(kind, &settings, &appearance, DOCUMENT_PIXELS_PER_INCH).paint(paint).stroke_open(Bind::bound(&mut open)).show(ui)
        })
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
    bar(ui, data, ShapeKind::Rectangle, true)
}

#[story(ShapeBar<'static>, covers = [Default], drive = Drive::Never)]
fn shape_bar_path_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, ShapeKind::Rectangle, false)
}

#[story(ShapeBar<'static>, covers = [Default], drive = Drive::Never)]
fn shape_bar_line(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    bar(ui, data, ShapeKind::Line, true)
}

#[story(StrokeCells<'static>, covers = [Default], drive = Drive::Never)]
fn stroke_cells(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    stroke_cells_with(ui, data, true)
}

#[story(StrokeCells<'static>, covers = [Default], drive = Drive::Never)]
fn stroke_cells_ellipse(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    stroke_cells_with(ui, data, false)
}

fn stroke_cells_with(ui: &mut Ui, data: &mut StoryData, corners: bool) -> StoryOutput {
    let appearance = data.value("look", look()).clone();
    let mut open = *data.value("open", false);
    let events = Bar::new("Stroke options", BarKind::Options)
        .show(ui, |ui, _| {
            StrokeCells::new(&appearance, DOCUMENT_PIXELS_PER_INCH).corners(corners).stroke_open(Bind::bound(&mut open)).show(ui).events
        })
        .inner
        .unwrap_or_default();
    *data.value("open", false) = open;
    let (mut shape, mut next_look) = (rectangle(), appearance);
    apply_all(events, &mut shape, &mut next_look);
    *data.value("look", look()) = next_look;
    StoryOutput::default()
}

#[story(ArrowheadsPopover<'static>, covers = [Default], drive = Drive::Never)]
fn arrowheads_popover_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = *data.value("arrows", Arrowheads::default());
    let mut open = *data.value("open", true);
    let out = ArrowheadsPopover::new(Bind::given(value), Bind::bound(&mut open), DOCUMENT_PIXELS_PER_INCH).show(ui);
    *data.value("open", true) = open;
    if let Some(next) = out.edits.iter().rev().find_map(|event| event.value()) {
        *data.value("arrows", Arrowheads::default()) = *next;
    }
    StoryOutput::default()
}

#[story(BoxConstraintsPopover<'static>, covers = [Default], drive = Drive::Never)]
fn box_constraints_popover_open(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = *data.value("box", BoxConstraints::default());
    let mut open = *data.value("open", true);
    let out = BoxConstraintsPopover::new(Bind::given(value), Bind::bound(&mut open), DOCUMENT_PIXELS_PER_INCH).show(ui);
    *data.value("open", true) = open;
    if let Some(next) = out.edits.iter().rev().find_map(|event| event.value()) {
        *data.value("box", BoxConstraints::default()) = *next;
    }
    StoryOutput::default()
}

#[story(VectorModeSelect<'static>, covers = [Default], drive = Drive::Never)]
fn vector_mode_select_greyed_pixels(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mode = *data.value("mode", VectorMode::Shape);
    let offered = [(VectorMode::Shape, true), (VectorMode::Path, true), (VectorMode::Pixels, false)];
    let out = VectorModeSelect::new(Bind::given(mode), &offered).show(ui);
    if let Some(next) = out.value().copied() {
        *data.value("mode", VectorMode::Shape) = next;
    }
    StoryOutput::default()
}
