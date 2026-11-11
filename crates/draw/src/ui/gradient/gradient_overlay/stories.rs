//! Stories of the gradient overlay.

use craft_behavior::kurbo::Point;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

use super::GradientOverlay;
use super::parts::END;
use crate::ui::gradient::{ColorStop, GradientEditor, GradientExtent, GradientLine, GradientStops, LineGradient, StopRef};

fn gradient() -> LineGradient {
    let stop = |location, r, g, b| ColorStop::new(location, Srgba { r, g, b, a: 1.0 });
    LineGradient {
        line: GradientLine { start: Point::new(220.0, 200.0), end: Point::new(420.0, 200.0) },
        stops: GradientStops::new(vec![stop(0.0, 0.1, 0.1, 0.9), stop(0.5, 0.9, 0.9, 0.2), stop(1.0, 0.9, 0.1, 0.1)]),
    }
}

/// The story's gradient and the stop it has selected.
fn state(data: &mut StoryData, selected: StopRef) -> (&mut LineGradient, &mut Option<StopRef>) {
    let state = data.value("state", (gradient(), Some(selected)));
    (&mut state.0, &mut state.1)
}

/// Shows the overlay `overlay` builds as the only canvas control of a canvas whose view is the room it is given. The
/// canvas body runs more than once per pass, so `overlay` builds the overlay on every run.
fn on_canvas(ui: &mut Ui, mut overlay: impl FnMut(&mut Ui, egui::Rect) -> craft_ui::contract::EditResponse<LineGradient>) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| overlay(ui, at))
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, selected) = state(data, StopRef::Color(1));
    on_canvas(ui, |ui, at| {
        GradientOverlay::new("Gradient", Bind::bound(&mut *value)).shared_selection(Bind::bound(&mut *selected)).show_at(ui, at).edit
    })
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_extent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("gradient", gradient());
    on_canvas(ui, |ui, at| GradientOverlay::new("Gradient", Bind::bound(&mut *value)).extent(GradientExtent::Circle).reverse().show_at(ui, at).edit)
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_mirror(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let value = data.value("gradient", gradient());
    on_canvas(ui, |ui, at| GradientOverlay::new("Gradient", Bind::bound(&mut *value)).extent(GradientExtent::Mirror).show_at(ui, at).edit)
}

#[story(GradientOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::new(END).toward(vec2(0.0, 1.0))))]
fn gradient_overlay_shared(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (value, selected) = state(data, StopRef::Color(0));
    let mut stops = value.stops.clone();
    GradientEditor::new("Stops", Bind::bound(&mut stops)).shared_selection(Bind::bound(&mut *selected)).show(ui);
    value.stops = stops;
    on_canvas(ui, |ui, at| {
        GradientOverlay::new("Gradient", Bind::bound(&mut *value)).shared_selection(Bind::bound(&mut *selected)).show_at(ui, at).edit
    })
}
