//! Stories of `BezierOverlay`.

use super::{
    ANCHOR, AnchorsShown, BezierOutput, BezierOverlay, BezierTool, EditPath, EditSubpath, HandleStyle, HandlesShown, Knot, KnotRef,
    PathOptionsSettings, PathSelection, SEGMENT, index,
};
use craft_behavior::kurbo::Point;
use craft_theme::Srgba;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Drive, Grip, StoryData, StoryOutput, story};
use egui::{Ui, Vec2};

/// An open subpath of three anchors, the last smooth with handles, and a closed triangle.
fn paths() -> Vec<EditPath> {
    let curve = Knot { anchor: Point::new(400.0, 130.0), in_ctrl: Point::new(370.0, 90.0), out_ctrl: Point::new(430.0, 170.0), smooth: true };
    let open = vec![Knot::corner(Point::new(200.0, 130.0)), Knot::corner(Point::new(300.0, 130.0)), curve];
    let triangle = [(200.0, 230.0), (300.0, 230.0), (250.0, 310.0)].map(|(x, y)| Knot::corner(Point::new(x, y))).to_vec();
    let subpaths = vec![EditSubpath { knots: triangle, closed: true }];
    vec![EditPath::new(1, vec![EditSubpath { knots: open, closed: false }]), EditPath::new(2, subpaths)]
}

/// Shows the overlay `overlay` builds and shows as the only canvas control of a canvas whose view has the document
/// origin at its top left. The canvas body runs more than once per pass, so `overlay` builds it on every run.
fn on_canvas(ui: &mut Ui, mut overlay: impl FnMut(&mut Ui, egui::Rect) -> BezierOutput) -> StoryOutput {
    craft_ui::testing::canvas_story::on_canvas(ui, |ui, at, _| overlay(ui, at))
}

fn selected() -> PathSelection {
    PathSelection::new(Some(1), vec![KnotRef::new(0, 2)], None)
}

#[story(BezierOverlay<'static>, covers = [], drive = Drive::Never)]
fn anchors_selected_in_two_paths(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected().also(2, vec![KnotRef::new(0, 0), KnotRef::new(0, 1)])).show_at(ui, at)
    })
}

/// One anchor over both paths shows its handles; the second story's two anchors show none.
#[story(BezierOverlay<'static>, covers = [], drive = Drive::Never)]
fn one_anchor_shows_its_handles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).handles(HandlesShown::OneAnchor).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [], drive = Drive::Never)]
fn two_anchors_show_no_handles(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected().also(2, vec![KnotRef::new(0, 0)]))
            .handles(HandlesShown::OneAnchor)
            .show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [], drive = Drive::Never)]
fn curvature_anchors(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).anchors(AnchorsShown::Curvature).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [], drive = Drive::Never)]
fn anchors_in_the_users_size_and_style(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    let settings = PathOptionsSettings { anchor_size: Some(12.0), handles: HandleStyle::Large, ..PathOptionsSettings::default() };
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).path_options(&settings).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn direct_selection_with_a_selected_anchor(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).show_at(ui, at))
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn all_handles_shown(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &PathSelection::default()).handles(HandlesShown::All).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(ANCHOR.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::X)))]
fn curvature_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).tool(BezierTool::Curvature).show_at(ui, at))
}

/// Add Anchor takes a press on an anchor and moves nothing: the drag is `add_anchor_tool_dragged`'s, on a segment.
#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn add_anchor_tool(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &PathSelection::default()).tool(BezierTool::AddAnchor).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(SEGMENT.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::Y)))]
fn add_anchor_tool_dragged(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &PathSelection::default()).tool(BezierTool::AddAnchor).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Drag(Grip::part(SEGMENT.at(index(0, KnotRef { subpath: 0, knot: 0 }))).toward(Vec2::Y)))]
fn convert_tool_straight_segment(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &PathSelection::default()).tool(BezierTool::Convert).show_at(ui, at)
    })
}

#[story(BezierOverlay<'static>, covers = [Default], drive = Drive::Never)]
fn outlines_only(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let (paths, paths_at) = data.value_at("paths", paths());
    on_canvas(ui, |ui, at| {
        BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &PathSelection::default())
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
    let (paths, paths_at) = data.value_at("paths", coloured());
    on_canvas(ui, |ui, at| BezierOverlay::new("Paths", paths_at.bind(&mut *paths), &selected()).show_at(ui, at))
}
