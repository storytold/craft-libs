//! Stories of the gradient overlay.

use craft_behavior::kurbo::Point;
use craft_theme::Srgba;
use craft_ui::canvas::RasterSource;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::canvas_story::{StoryDocument, StoryPicture};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui, vec2};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use super::GradientOverlay;
use super::parts::END;
use crate::ui::gradient::stops::sample;
use crate::ui::gradient::{ColorStop, GradientEditor, GradientExtent, GradientLine, GradientStops, LineGradient, StopRef, StopRows, gradient_line};

fn gradient() -> LineGradient {
    LineGradient {
        line: GradientLine { start: Point::new(150.0, 230.0), end: Point::new(470.0, 150.0) },
        stops: GradientStops::new(vec![
            ColorStop::new(0.0, Srgba::rgb(0.13, 0.35, 0.72)),
            ColorStop::new(0.3, Srgba::rgb(0.95, 0.75, 0.2)).with_midpoint(0.35),
            ColorStop::new(0.7, Srgba::rgb(0.85, 0.25, 0.3)),
            ColorStop::new(1.0, Srgba::rgb(0.9, 0.9, 0.95)),
        ]),
    }
}

/// The gradient `value` lays on the story document, `reverse`d or not: 64 by 40 cells, each the colour at its centre.
fn ramp(value: &LineGradient, reverse: bool) -> StoryPicture<impl Fn(Point) -> Color32 + use<>> {
    const COLUMNS: usize = 64;
    const ROWS: usize = 40;
    let document = StoryDocument::STANDARD.rect();
    let (width, height) = (document.width() / COLUMNS as f64, document.height() / ROWS as f64);
    let cells: Vec<Color32> = (0..COLUMNS * ROWS)
        .map(|cell| {
            let centre = Point::new(((cell % COLUMNS) as f64 + 0.5) * width, ((cell / COLUMNS) as f64 + 0.5) * height);
            let (location, _) = gradient_line::project(&value.line, centre, reverse);
            Color32::from(sample(&value.stops, StopRows::Split, location))
        })
        .collect();
    let mut hasher = DefaultHasher::new();
    let line = [value.line.start, value.line.end].map(|p| [p.x.to_bits(), p.y.to_bits()]);
    (line, reverse).hash(&mut hasher);
    for stop in &value.stops.color {
        [stop.location, stop.midpoint, stop.color.r, stop.color.g, stop.color.b, stop.color.a].map(f32::to_bits).hash(&mut hasher);
    }
    for stop in &value.stops.opacity {
        [stop.location, stop.midpoint, stop.opacity].map(f32::to_bits).hash(&mut hasher);
    }
    StoryPicture::new(document.width() as u32, document.height() as u32, move |p: Point| {
        let (column, row) = (((p.x / width) as usize).min(COLUMNS - 1), ((p.y / height) as usize).min(ROWS - 1));
        cells[row * COLUMNS + column]
    })
    .revision(hasher.finish())
}

/// The story's gradient and the stop it has selected.
fn state(data: &mut StoryData, selected: StopRef) -> (&mut LineGradient, &mut Option<StopRef>) {
    let state = data.value("state", (gradient(), Some(selected)));
    (&mut state.0, &mut state.1)
}

/// Shows the overlay `overlay` builds as the only canvas control of a canvas over `picture`. The canvas body runs more
/// than once per pass, so `overlay` builds the overlay on every run.
fn on_canvas(
    ui: &mut Ui,
    picture: &dyn RasterSource,
    mut overlay: impl FnMut(&mut Ui, egui::Rect) -> craft_ui::contract::EditResponse<LineGradient>,
) -> StoryOutput {
    craft_ui::testing::canvas_story::on_picture(ui, picture, |ui, at, _| overlay(ui, at))
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, selected) = state(data, StopRef::Color(1));
    let picture = ramp(value, false);
    on_canvas(ui, &picture, |ui, at| {
        GradientOverlay::new("Gradient", Bind::bound(&mut *value)).shared_selection(Bind::bound(&mut *selected)).show_at(ui, at).edit
    })
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_extent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("gradient", gradient());
    let picture = ramp(value, true);
    on_canvas(ui, &picture, |ui, at| {
        GradientOverlay::new("Gradient", Bind::bound(&mut *value)).extent(GradientExtent::Circle).reverse().show_at(ui, at).edit
    })
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_mirror(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("gradient", gradient());
    let picture = ramp(value, false);
    on_canvas(ui, &picture, |ui, at| GradientOverlay::new("Gradient", Bind::bound(&mut *value)).extent(GradientExtent::Mirror).show_at(ui, at).edit)
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_shared(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, selected) = state(data, StopRef::Color(0));
    let mut stops = value.stops.clone();
    GradientEditor::new("Stops", Bind::bound(&mut stops)).shared_selection(Bind::bound(&mut *selected)).show(ui);
    value.stops = stops;
    let picture = ramp(value, false);
    on_canvas(ui, &picture, |ui, at| {
        GradientOverlay::new("Gradient", Bind::bound(&mut *value)).shared_selection(Bind::bound(&mut *selected)).show_at(ui, at).edit
    })
}
