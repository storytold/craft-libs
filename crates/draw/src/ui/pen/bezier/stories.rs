//! Stories of `BezierOverlay`.

use super::{
    ANCHOR, AnchorsShown, BezierOutput, BezierOverlay, BezierTool, EditPath, EditSubpath, HandlesShown, Knot, KnotRef, PathSelection, index,
};
use craft_behavior::kurbo::Point;
use craft_theme::Srgba;
use craft_ui::contract::{Bind, Show as _};
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, Vec2};

/// An open subpath of three anchors, the last smooth with handles, and a closed triangle.
fn paths() -> Vec<EditPath> {
    let curve = Knot { anchor: Point::new(400.0, 130.0), in_ctrl: Point::new(370.0, 90.0), out_ctrl: Point::new(430.0, 170.0), smooth: true };
    let open = vec![Knot::corner(Point::new(200.0, 130.0)), Knot::corner(Point::new(300.0, 130.0)), curve];
    let triangle = [(200.0, 230.0), (300.0, 230.0), (250.0, 310.0)].map(|(x, y)| Knot::corner(Point::new(x, y))).to_vec();
    let subpaths = vec![EditSubpath { knots: triangle, closed: true }];
    vec![EditPath { key: 1, subpaths: vec![EditSubpath { knots: open, closed: false }], color: None }, EditPath { key: 2, subpaths, color: None }]
}

/// Shows the overlay `overlay` builds and shows as the only canvas control of a canvas whose view has the document
/// origin at its top left. The canvas body runs more than once per pass, so `overlay` builds it on every run.
fn on_canvas(ui: &mut Ui, mut overlay: impl FnMut(&mut Ui, egui::Rect) -> BezierOutput) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| overlay(ui, at))
}

fn selected() -> PathSelection {
    PathSelection { path: Some(1), anchors: vec![KnotRef::new(0, 2)], segment: None }
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn direct_selection_with_a_selected_anchor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paths = data.value("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", Bind::bound(&mut *paths), &selected()).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn all_handles_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paths = data.value("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", Bind::bound(&mut *paths), &PathSelection::default()).handles(HandlesShown::All).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn curvature_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paths = data.value("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", Bind::bound(&mut *paths), &selected()).tool(BezierTool::Curvature).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn add_anchor_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paths = data.value("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", Bind::bound(&mut *paths), &PathSelection::default()).tool(BezierTool::AddAnchor).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn outlines_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let paths = data.value("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", Bind::bound(&mut *paths), &PathSelection::default())
            .tool(BezierTool::Anchors)
            .anchors(AnchorsShown::Hidden)
            .show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn a_path_in_its_own_colour(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let coloured = || {
        let mut paths = paths();
        paths[0].color = Some(Srgba::rgb(0.9, 0.43, 0.08));
        paths
    };
    let paths = data.value("paths", coloured());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", Bind::bound(&mut *paths), &selected()).show_at(ui, at))
}
