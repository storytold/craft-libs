//! Stories of the corner widgets: a rectangle and a hexagon on a canvas.

use super::widgets::WIDGET;
use super::{CornerKind, CornerPoint, CornerTarget, CornerWidgets, CornerWidgetsOutput};
use std::f64::consts::{PI, TAU};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use crate::geometry::fillet::fillet;
use craft_behavior::kurbo::{BezPath, Point, Vec2};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui, vec2};

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

/// The outline of `target` with each corner cut as its kind and radius say.
fn outline(target: &CornerTarget) -> BezPath {
    const ARC_STEPS: usize = 12;
    let mut points = Vec::new();
    for corner in &target.corners {
        let Some(cut) = fillet(corner.prev, corner.vertex, corner.next, corner.radius) else {
            points.push(corner.vertex);
            continue;
        };
        match corner.kind {
            CornerKind::Chamfer => points.extend([cut.from, cut.to]),
            kind => {
                let centre = if kind == CornerKind::InvertedRound { cut.from + cut.to.to_vec2() - cut.centre.to_vec2() } else { cut.centre };
                let start = (cut.from - centre).atan2();
                let sweep = ((cut.to - centre).atan2() - start + PI).rem_euclid(TAU) - PI;
                points.extend((0..=ARC_STEPS).map(|step| centre + Vec2::from_angle(start + sweep * step as f64 / ARC_STEPS as f64) * cut.radius));
            }
        }
    }
    let mut path = BezPath::new();
    for (index, point) in points.into_iter().enumerate() {
        if index == 0 { path.move_to(point) } else { path.line_to(point) }
    }
    path.close_path();
    path
}

/// The story document with `targets` drawn on it, filled and outlined in the accent.
fn shapes(ui: &Ui, targets: &[CornerTarget]) -> StoryPicture<impl Fn(Point) -> Color32 + use<>> {
    let (fill, line) = (accent(ui, 0.25), accent(ui, 1.0));
    let shapes: Vec<StoryShape> = targets.iter().map(|target| StoryShape::filled(&outline(target), fill).outlined(OUTLINE, line)).collect();
    let mut hasher = DefaultHasher::new();
    for corner in targets.iter().flat_map(|target| &target.corners) {
        (corner.radius.to_bits(), corner.kind as u8).hash(&mut hasher);
    }
    let size = StoryDocument::STANDARD.size();
    StoryPicture::new(size.width, size.height, move |p| paint_shapes(&shapes, Color32::TRANSPARENT, p)).revision(hasher.finish())
}

/// Shows the widgets `control` builds over `targets` as the only canvas control of a canvas over the shapes. The canvas
/// body runs more than once per pass, so `control` builds it on every run.
fn on_canvas(
    ui: &mut Ui,
    targets: &mut Vec<CornerTarget>,
    mut control: impl FnMut(&mut Ui, egui::Rect, &mut Vec<CornerTarget>) -> CornerWidgetsOutput,
) -> StoryOutput {
    let picture = shapes(ui, targets);
    on_picture(ui, &picture, |ui, at, _| StoryOutput::from(control(ui, at, targets).edit))
}

#[story(CornerWidgets<'static>, covers = [Default], drive = Drive::Drag(Grip::part(WIDGET.at(0)).toward(vec2(1.0, 1.0))))]
fn rectangle_and_hexagon(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = data.value("targets", targets());
    on_canvas(ui, targets, |ui, at, targets| CornerWidgets::new("Corner radius", Bind::bound(targets)).show_at(ui, at))
}

#[story(CornerWidgets<'static>, covers = [], drive = Drive::Never)]
fn cycling_kinds_and_a_wide_angle_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = data.value("kinds", targets());
    on_canvas(ui, targets, |ui, at, targets| CornerWidgets::new("Corner radius", Bind::bound(targets)).kinds(true).hide_above(120.0).show_at(ui, at))
}
