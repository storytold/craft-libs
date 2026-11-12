//! Stories of the corner widgets: a rectangle, a star and a path's corners being rounded, and a rectangle and a
//! hexagon with the kinds cycling.

use super::widgets::WIDGET;
use super::{CornerKind, CornerPoint, CornerTarget, CornerWidgets, CornerWidgetsOutput};
use std::f64::consts::{PI, TAU};
use std::hash::{DefaultHasher, Hash as _, Hasher as _};

use crate::geometry::fillet::fillet;
use craft_behavior::kurbo::{BezPath, Point, Vec2};
use craft_ui::canvas::RasterSource as _;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::runtime::env::Env;
use craft_ui::testing::canvas_story::{OUTLINE, StoryDocument, StoryPicture, StoryShape, accent, on_picture, paint_shapes};
use craft_ui::testing::{Drive, EditKind, Grip, StoryData, StoryOutput, story};
use egui::{Color32, Ui, vec2};

craft_ui::words! {
    /// The status line of the corner stories.
    #[non_exhaustive]
    pub(crate) struct CornerStoryWords {
        /// No event has been reported yet.
        no_event: "cornerStory.noEvent" = "No event yet";
        /// The last event the widgets reported.
        last_event: template "cornerStory.lastEvent" = "Last event: {event}";
        /// A drag began.
        began: "cornerStory.began" = "Begin";
        /// A drag moved the radius.
        updated: "cornerStory.updated" = "Update";
        /// A drag ended.
        committed: "cornerStory.committed" = "Commit";
        /// A drag was cancelled.
        cancelled: "cornerStory.cancelled" = "Cancel";
        /// A key set the radius.
        set: "cornerStory.set" = "Set";
        /// ⌥-click cycled a corner's kind.
        cycled: "cornerStory.cycled" = "Kind changed";
        /// A double press or Enter asked for the Corners dialog.
        activated: "cornerStory.activated" = "Activated";
    }
}

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

/// A rectangle (each corner its own radius), a star (one radius for all corners) and a path along the page's foot
/// (each corner its own radius when `individual`), on the story document.
fn edited(individual: bool) -> Vec<CornerTarget> {
    let page = StoryDocument::STANDARD.rect();
    let at = |x: f64, y: f64| (page.x0 + page.width() * x, page.y0 + page.height() * y);
    let radius = page.height() * 0.04;
    let rect = [at(0.06, 0.18), at(0.34, 0.18), at(0.34, 0.5), at(0.06, 0.5)];
    let (centre, outer, inner) = (Point::from(at(0.64, 0.34)), page.height() * 0.28, page.height() * 0.13);
    let star: Vec<(f64, f64)> = (0..10)
        .map(|i| {
            let reach = if i % 2 == 0 { outer } else { inner };
            let point = centre + Vec2::from_angle(TAU * f64::from(i) / 10.0 - PI / 2.0) * reach;
            (point.x, point.y)
        })
        .collect();
    let path = [at(0.12, 0.92), at(0.2, 0.62), at(0.5, 0.58), at(0.82, 0.66), at(0.9, 0.92)];
    vec![polygon(1, &rect, radius, true), polygon(2, &star, radius, false), polygon(3, &path, radius, individual)]
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

/// Shapes with rounded corners and the widgets over them: drag a widget to round a corner (⇧ and the arrows work from
/// the keyboard), ⌥ on the rectangle changes one corner, ⌥-click cycles the kind, a double click asks for the Corners
/// dialog. Story values: the kinds, the widest inner angle that shows a widget, and whether the path's corners take
/// their own radius.
#[story(CornerWidgets<'static>, covers = [Default], drive = Drive::Drag(Grip::part(WIDGET.at(0)).toward(vec2(1.0, 1.0))))]
fn edit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let words = Env::of(ui).words::<CornerStoryWords>();
    let kinds = *data.value("kinds", true);
    let hide_above = *data.ranged("hide above", 177.0, 90.0..=180.0);
    let individual = *data.value("individual", true);
    let mut last = data.note::<Option<String>>("last", None).clone();
    let targets = data.value("targets", edited(individual));
    if let Some(path) = targets.iter_mut().find(|target| target.key == 3) {
        path.individual = individual;
    }
    let output = on_canvas(ui, targets, |ui, at, targets| {
        let shown = CornerWidgets::new("Corner radius", Bind::bound(&mut *targets)).kinds(kinds).hide_above(hide_above).show_at(ui, at);
        let kind = shown.edit.events.iter().filter_map(EditKind::of).last().and_then(|kind| match kind {
            EditKind::Begin => Some(&words.began),
            EditKind::Update => Some(&words.updated),
            EditKind::Commit => Some(&words.committed),
            EditKind::Cancel => Some(&words.cancelled),
            EditKind::Set => Some(&words.set),
            _ => None,
        });
        if let Some(kind) = kind {
            last = Some(kind.as_str().to_owned());
        }
        if let Some(corner) = shown.cycle_kind
            && let Some(point) =
                targets.iter_mut().find(|target| target.key == corner.target).and_then(|target| target.corners.get_mut(corner.corner))
        {
            point.kind = point.kind.next();
            last = Some(words.cycled.as_str().to_owned());
        }
        if shown.activate.is_some() {
            last = Some(words.activated.as_str().to_owned());
        }
        shown
    });
    let text =
        last.as_deref().map_or_else(|| words.no_event.as_str().to_owned(), |event| craft_i18n::fmt(words.last_event.as_str(), &[("event", event)]));
    *data.note::<Option<String>>("last", None) = last;
    output.readout(text)
}

#[story(CornerWidgets<'static>, covers = [], drive = Drive::Never)]
fn cycling_kinds_and_a_wide_angle_limit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let targets = data.value("kinds", targets());
    on_canvas(ui, targets, |ui, at, targets| CornerWidgets::new("Corner radius", Bind::bound(targets)).kinds(true).hide_above(120.0).show_at(ui, at))
}
