//! Stories of the corner widgets: a rectangle and a hexagon on a canvas.

use super::widgets::WIDGET;
use super::{CornerKind, CornerPoint, CornerTarget, CornerWidgets, CornerWidgetsOutput};
use craft_behavior::kurbo::Point;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

/// The rectangle's corner radius and the hexagon's.
const RECT_RADIUS: f64 = 12.0;
const HEXAGON_RADIUS: f64 = 8.0;

/// A closed polygon whose vertices are `points`, each corner rounded by `radius`.
fn polygon(key: u64, points: &[(f64, f64)], radius: f64, individual: bool) -> CornerTarget {
    let vertices: Vec<Point> = points.iter().map(|&(x, y)| Point::new(x, y)).collect();
    let n = vertices.len();
    let corners = (0..n)
        .map(|i| CornerPoint { vertex: vertices[i], prev: vertices[(i + n - 1) % n], next: vertices[(i + 1) % n], radius, kind: CornerKind::Round })
        .collect();
    CornerTarget { key, corners, individual }
}

/// A rectangle (each corner its own radius) and a hexagon (one radius for all corners), side by side.
fn targets() -> Vec<CornerTarget> {
    let rect = polygon(1, &[(130.0, 150.0), (290.0, 150.0), (290.0, 250.0), (130.0, 250.0)], RECT_RADIUS, true);
    let hexagon =
        polygon(2, &[(390.0, 170.0), (450.0, 150.0), (510.0, 170.0), (510.0, 230.0), (450.0, 250.0), (390.0, 230.0)], HEXAGON_RADIUS, false);
    vec![rect, hexagon]
}

/// Shows the control `control` builds and shows as the only canvas control of a canvas whose view has the document
/// origin at its centre. The canvas body runs more than once per pass, so `control` builds it on every run.
fn on_canvas(ui: &mut Ui, mut control: impl FnMut(&mut Ui, egui::Rect) -> CornerWidgetsOutput) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| StoryOutput::from(control(ui, at).edit))
}

#[story(CornerWidgets<'static>, covers = [Default], drive = Drive::Drag(Grip::part(WIDGET.at(0)).toward(vec2(1.0, 1.0))))]
fn rectangle_and_hexagon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = data.value("targets", targets());
    on_canvas(ui, |ui, at| CornerWidgets::new("Corner radius", Bind::bound(&mut *targets)).show_at(ui, at))
}

#[story(CornerWidgets<'static>, covers = [], drive = Drive::Never)]
fn cycling_kinds_and_a_wide_angle_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = data.value("kinds", targets());
    on_canvas(ui, |ui, at| CornerWidgets::new("Corner radius", Bind::bound(&mut *targets)).kinds(true).hide_above(120.0).show_at(ui, at))
}
