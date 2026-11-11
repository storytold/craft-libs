//! Stories of the warp grid, its options-bar group and the mesh lines.

use craft_behavior::kurbo::{Point, Rect as DocRect};
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Fixture, Grip, StoryData, StoryOutput, pointer, story};
use egui::{Ui, vec2};

use crate::ui::transform_preview::PreviewMap;
use crate::ui::transform_preview::stories::{Document, Zoom, draw};

use super::{MeshLines, POINT, WarpFieldValues, WarpFields, WarpGrid, WarpGridChoice, WarpMesh, WarpSplit, WarpStyleEntry};

/// The layer the mesh starts over: the transform preview stage's layer.
fn picture() -> DocRect {
    Document::standard().layer()
}

/// Shows `control` (built per run: the body may run twice) over the transform preview stage, the layer's picture drawn
/// through `mesh` by the real `TransformPreview` under it.
fn on_canvas<O: Into<StoryOutput>>(
    ui: &mut Ui,
    mesh: &mut WarpMesh,
    mut control: impl FnMut(&mut Ui, egui::Rect, &mut WarpMesh) -> O,
) -> StoryOutput {
    let doc = Document::standard();
    let textures = doc.textures();
    doc.canvas(ui, Zoom::Fit, None, |ui, at| {
        draw(ui, at, &doc, &textures, PreviewMap::mesh(mesh, doc.layer()), 1.0, false);
        control(ui, at, mesh).into()
    })
}

fn hover_middle(fixture: &mut Fixture) {
    let middle = fixture.shown().center();
    fixture.frame(vec![pointer::moved(middle)]);
}

#[story(WarpGrid<'static>, covers = [Default])]
fn one_patch_with_thirds(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mesh = data.value("mesh", WarpMesh::identity(picture(), 1, 1));
    on_canvas(ui, mesh, |ui, rect, mesh| WarpGrid::new("Warp", Bind::bound(mesh), picture()).show_at(ui, rect))
}

#[story(WarpGrid<'static>, covers = [Default], drive = Drive::Drag(Grip::part(POINT.at(0)).toward(vec2(24.0, 16.0))))]
fn three_by_three_after_a_drag(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mesh = data.value("mesh", WarpMesh::identity(picture(), 3, 3));
    on_canvas(ui, mesh, |ui, rect, mesh| WarpGrid::new("Warp", Bind::bound(mesh), picture()).show_at(ui, rect))
}

#[story(WarpGrid<'static>, covers = [Default], drive = Drive::Custom(hover_middle))]
fn cross_split_over_a_patch(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mesh = data.value("mesh", WarpMesh::identity(picture(), 3, 3));
    on_canvas(ui, mesh, |ui, rect, mesh| WarpGrid::new("Warp", Bind::bound(mesh), picture()).split(WarpSplit::Cross).show_at(ui, rect))
}

#[story(WarpGrid<'static>, covers = [Default])]
fn lines_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let mesh = data.value("mesh", WarpMesh::identity(picture(), 3, 3));
    on_canvas(ui, mesh, |ui, rect, mesh| WarpGrid::new("Warp", Bind::bound(mesh), picture()).lines_only().show_at(ui, rect))
}

fn styles() -> Vec<WarpStyleEntry> {
    vec![
        WarpStyleEntry::new("none", "None"),
        WarpStyleEntry::new("custom", "Custom").custom(true),
        WarpStyleEntry::new("arc", "Arc").preset(true),
        WarpStyleEntry::new("flag", "Flag").preset(true),
    ]
}

#[story(WarpFields<'static>, covers = [Default])]
fn fields_on_a_preset_style(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let styles = styles();
    let mut values = WarpFieldValues::new("arc");
    values.bend = 50.0;
    values.grid = WarpGridChoice::Three;
    WarpFields::new(&styles, values).show(ui);
    StoryOutput::default()
}

#[story(WarpFields<'static>, covers = [Default])]
fn fields_on_a_type_layer(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let styles = styles();
    WarpFields::new(&styles, WarpFieldValues::new("none")).on_type().show(ui);
    StoryOutput::default()
}

#[story(MeshLines<'static>, covers = [Default])]
fn lines_over_a_small_grid(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let (columns, rows) = (4, 3);
    let picture = picture();
    let (width, height) = (picture.width() / f64::from(columns), picture.height() / f64::from(rows));
    let at = |i: i32, j: i32| Point::new(picture.x0 + f64::from(i) * width, picture.y0 + f64::from(j) * height + f64::from(i) * 4.0);
    let mut edges = Vec::new();
    for j in 0..=rows {
        for i in 0..=columns {
            if i < columns {
                edges.push([at(i, j), at(i + 1, j)]);
            }
            if j < rows {
                edges.push([at(i, j), at(i, j + 1)]);
            }
        }
    }
    Document::standard().canvas(ui, Zoom::Fit, None, |ui, rect| MeshLines::new(&edges).show_at(ui, rect).into())
}
