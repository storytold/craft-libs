//! Stories of the cylinder editor over a cylinder mesh built from its values.

use craft_behavior::kurbo::Point;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, vec2};

use crate::ui::transform_preview::PreviewMap;
use crate::ui::transform_preview::stories::{Document, Zoom, draw};
use crate::ui::warp::{CylinderValues, CylinderWarp, WarpMesh};

/// The one-patch mesh of the cylinder `v`: each edge a cubic whose inner control points sit 4/3 of the arc's depth off
/// the corners, so the middle of the arc is `depth` half axes down.
fn cylinder_mesh(v: &CylinderValues) -> WarpMesh {
    let a = (v.right - v.left) / 2.0;
    let mut points = Vec::with_capacity(16);
    for j in 0..4 {
        for i in 0..4 {
            let bow = if i == 1 || i == 2 { 4.0 / 3.0 * a } else { 0.0 };
            let (top, bottom) = (v.top + bow * v.top_depth, v.bottom + bow * v.bottom_depth);
            points.push(Point::new(v.left + (v.right - v.left) * i as f64 / 3.0, top + (bottom - top) * j as f64 / 3.0));
        }
    }
    WarpMesh::new(vec![0.0, 1.0], vec![0.0, 1.0], points).unwrap_or_else(|| WarpMesh::identity(Document::standard().layer(), 1, 1))
}

/// Shows the editor over the preview stage, the layer's picture drawn through the cylinder's mesh under it.
fn show(ui: &mut Ui, values: &mut CylinderValues) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    let mesh = cylinder_mesh(values);
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        draw(ui, at, &doc, &textures, PreviewMap::mesh(&mesh, doc.layer()), 1.0, false);
        CylinderWarp::new("Cylinder", Bind::bound(&mut *values), &mesh).show_at(ui, at).into()
    })
}

fn start() -> CylinderValues {
    let layer = Document::standard().layer();
    CylinderValues::for_box([layer.x0, layer.y0, layer.x1, layer.y1])
}

#[story(CylinderWarp<'static>, covers = [Default], drive = Drive::Drag(Grip::part(super::HANDLE.at(4)).toward(vec2(1.0, 0.0))))]
fn default(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    show(ui, data.value("values", start()))
}

#[story(CylinderWarp<'static>, covers = [])]
fn bent(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mut bent = start();
    bent.top_depth = -0.4;
    bent.bottom_depth = 0.8;
    bent.wrap = 0.3;
    show(ui, data.value("values", bent))
}
