//! Stories of Perspective Warp: the planes over the transform preview stage, the layer's picture drawn through them by
//! the real `TransformPreview`, and the options bar.

use craft_behavior::geometry::quad::Homography;
use craft_behavior::kurbo::Point;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

use super::overlay::CORNER;
use super::{PerspectiveBar, PerspectiveEdit, PerspectiveSession, PerspectiveWarp, Plane, PlaneMode};
use crate::ui::transform_preview::stories::{Document, Zoom, draw};
use crate::ui::transform_preview::{PreviewMap, PreviewQuality};

/// How far the warp story pulls the corners, in percent of the layer's size.
const PULL: f64 = 8.0;

/// Two planes side by side across the stage's layer, sharing the corners of the middle edge, each target `pull`
/// percent of the layer's size from its source (a corner shared by planes is pulled once).
fn two_linked_planes(pull: f64) -> Vec<Plane> {
    let layer = Document::standard().layer();
    let (dx, dy) = (layer.width() * pull / 100.0, layer.height() * pull / 100.0);
    // Every corner moves by a function of where it is, so linked corners stay linked.
    let pulled = |p: Point| {
        let (fx, fy) = ((p.x - layer.x0) / layer.width() - 0.5, (p.y - layer.y0) / layer.height() - 0.5);
        Point::new(p.x - fx * 2.0 * dx * (0.5 - fy), p.y + dy * (0.5 - fy))
    };
    let middle = layer.center().x;
    [(layer.x0, middle), (middle, layer.x1)]
        .into_iter()
        .map(|(x0, x1)| {
            let source = [Point::new(x0, layer.y0), Point::new(x1, layer.y0), Point::new(x1, layer.y1), Point::new(x0, layer.y1)];
            Plane::new(source, source.map(pulled))
        })
        .collect()
}

/// Where the planes take a point of the layer: through the plane whose source holds it, from its source corners to
/// its target corners; a point on no plane stays.
fn warped_by(planes: &[Plane]) -> impl Fn(Point) -> Point + use<> {
    let inverses: Vec<(Homography, Plane)> =
        planes.iter().filter_map(|plane| Homography::unit_to_quad(plane.source).and_then(|h| h.inverse()).map(|h| (h, *plane))).collect();
    move |p| {
        let inside = |v: f64| (-1e-9..=1.0 + 1e-9).contains(&v);
        inverses
            .iter()
            .find_map(|(to_unit, plane)| {
                let unit = to_unit.apply(p);
                (inside(unit.x) && inside(unit.y)).then(|| plane.point(unit.x, unit.y, true))
            })
            .unwrap_or(p)
    }
}

/// Shows `planes` over the transform preview stage: the layer's picture drawn through the planes (in Layout the
/// planes' targets are their sources, so it stays), the planes over it.
fn on_canvas(ui: &mut Ui, planes: &mut Vec<Plane>, mode: PlaneMode) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let layer = doc.layer();
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        let map = warped_by(planes);
        draw(ui, at, &doc, &textures, PreviewMap::function(layer, PreviewQuality::PLANE_CELLS, &map), 1.0, false);
        StoryOutput::from(PerspectiveWarp::new("Planes", Bind::bound(&mut *planes), mode, 0).show_at(ui, at))
    })
}

#[story(PerspectiveWarp<'static>, covers = [Default], drive = Drive::Drag(Grip::part(CORNER.at(1)).toward(vec2(1.0, -1.0))), group = "Planes", title = "Layout, two linked planes")]
fn two_linked_planes_in_layout(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let planes = data.value("planes", two_linked_planes(0.0));
    on_canvas(ui, planes, PlaneMode::Layout)
}

#[story(PerspectiveWarp<'static>, covers = [Default], drive = Drive::Drag(Grip::part(CORNER.at(1)).toward(vec2(1.0, -1.0))), group = "Planes", title = "Warp, the picture follows")]
fn two_linked_planes_in_warp(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let planes = data.value("warped", two_linked_planes(PULL));
    on_canvas(ui, planes, PlaneMode::Warp)
}

/// A session over two linked planes, in `mode`.
fn session(mode: PlaneMode) -> PerspectiveSession {
    let mut session = PerspectiveSession::new(Document::standard().layer());
    session.apply(PerspectiveEdit::replace(&[], two_linked_planes(if mode == PlaneMode::Warp { PULL } else { 0.0 }), "Planes"));
    session.apply(PerspectiveEdit::Mode(mode));
    session
}

#[story(PerspectiveBar<'static>, covers = [Default], drive = Drive::Never)]
fn bar_in_layout(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    PerspectiveBar::new(&session(PlaneMode::Layout)).show(ui);
    StoryOutput::default()
}

#[story(PerspectiveBar<'static>, covers = [], drive = Drive::Never)]
fn bar_in_warp(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    PerspectiveBar::new(&session(PlaneMode::Warp)).show(ui);
    StoryOutput::default()
}
