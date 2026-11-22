//! A group the host takes with `group_view` lays out in the group's rect, not in the room the dock area has left; a
//! tool rail the host takes has the dock's grip strip on top.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, SplitSize, Workspace};
use craft_ui::contract::{Bind, BodyCx, PlainData, Tracked};
use craft_ui::docking::{DockConfig, DockGrid, DockPanels, DockSizes, DragUnit, GroupAt, GroupDrag, PanelSpec, PanelSpecs};
use craft_ui::patterns::dock::DockArea;
use craft_ui::runtime::drag;
use craft_ui::testing::{Fixture, last_pass_runs, pointer, trial};
use egui::{Rect, Shape, Ui, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;
use std::sync::Arc;

/// Panel 0 is a canvas the host draws in its group's place; it records the group's rect and the `Ui` it was given.
struct Panels {
    seen: Rc<Cell<Option<(Rect, Rect, Rect)>>>,
}

/// Panel 0 is a canvas.
#[derive(PlainData)]
struct Specs;

impl PanelSpecs<usize> for Specs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == 0 { spec.canvas() } else { spec }
    }
}

impl DockPanels<usize> for Panels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, ui: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&0) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(0.0),
            GroupAt::Show(rect) => {
                self.seen.set(Some((rect, ui.max_rect(), ui.clip_rect())));
                Some(rect.height())
            }
        }
    }
}

#[trial]
fn a_group_view_lays_out_and_clips_to_its_group_rect() {
    let seen = Rc::new(Cell::new(None));
    let root = Node::split(Axis::X, SplitSize::Ratio(0.5), Node::tabs(vec![0], 0), Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (Panels { seen: seen.clone() }, Tracked::new(Specs));
    let mut fixture = Fixture::new(move |ui| {
        DockArea::new("Dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(1200.0, 600.0)).settle();
    let (rect, room, clip) = seen.get().expect("the canvas group was shown");
    assert!(rect.width() < 1000.0, "the canvas group takes part of the area: {rect:?}");
    assert!(rect.contains_rect(room), "the view's room {room:?} lies in its group's rect {rect:?}");
    assert!(rect.contains_rect(clip), "the view's clip {clip:?} lies in its group's rect {rect:?}");
}

/// The tool rail of the grip trial: a panel with a rail grid, which the host shows itself.
const RAIL: usize = 7;
/// The height the host's rail measures at.
const RAIL_HEIGHT: f32 = 200.0;

/// The rail is a tool rail; the rest are plain panels.
#[derive(PlainData)]
struct RailSpecs;

impl PanelSpecs<usize> for RailSpecs {
    fn spec(&self, panel: &usize) -> PanelSpec {
        let spec = PanelSpec::new(format!("Panel {panel}"));
        if *panel == RAIL { spec.grid(DockGrid::rail(4)) } else { spec }
    }
}

/// The host shows the rail itself and records the rect it was given.
struct RailPanels {
    seen: Rc<Cell<Option<Rect>>>,
}

impl DockPanels<usize> for RailPanels {
    fn body(&mut self, _: &usize, _: &mut Ui, _: &mut BodyCx<'_>) {}

    fn group_view(&mut self, panels: &[usize], _: usize, _: &mut Ui, at: GroupAt, _: &mut BodyCx<'_>) -> Option<f32> {
        if !panels.contains(&RAIL) {
            return None;
        }
        match at {
            GroupAt::Measure(_) => Some(RAIL_HEIGHT),
            GroupAt::Show(rect) => {
                self.seen.set(Some(rect));
                Some(rect.height())
            }
        }
    }
}

/// A tool rail the host shows itself has the dock's grip under its column bar, as the references' toolbar: a strip of
/// ten upright ticks, a little darker than the panel, above the host's view, which keeps the height it measured at; a
/// drag on the strip moves the rail's group.
#[trial]
fn a_tool_rail_shown_by_its_host_has_a_grip_strip_that_drags_it() {
    let seen = Rc::new(Cell::new(None));
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (RailPanels { seen: seen.clone() }, Tracked::new(RailSpecs));
    let columns: Rc<RefCell<Vec<(usize, Rect)>>> = Rc::default();
    let placed = Rc::clone(&columns);
    let mut fixture = Fixture::new(move |ui| {
        let out = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
        *placed.borrow_mut() = out.columns;
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let sizes = DockSizes::of(fixture.ctx());
    let view = seen.get().expect("the host showed the rail");
    let column = columns.borrow().iter().find(|(panel, _)| *panel == RAIL).map(|(_, rect)| *rect).expect("the rail's column shows");
    let strip = Rect::from_min_max(egui::pos2(view.left(), view.top() - sizes.grip), egui::pos2(view.right(), view.top()));
    assert!(sizes.grip > 0.0 && column.top() < strip.top(), "the strip lies under the column bar: {strip:?} in {column:?}");
    assert!((view.height() - RAIL_HEIGHT).abs() < 0.5, "the host's view keeps the height it measured at: {view:?}");

    // The strip's ticks: one mesh inside it of ten upright marks.
    let shapes = fixture.frame_shapes();
    let ticks: Vec<Rect> = shapes
        .iter()
        .find_map(|shape| {
            let Shape::Mesh(mesh) = shape else { return None };
            let quads: Vec<Rect> = mesh.vertices.chunks(4).map(|quad| Rect::from_points(&quad.iter().map(|v| v.pos).collect::<Vec<_>>())).collect();
            (!quads.is_empty() && quads.iter().all(|quad| strip.expand(0.01).contains_rect(*quad))).then_some(quads)
        })
        .expect("the strip paints its ticks");
    assert_eq!(ticks.len(), 10, "ten ticks: {ticks:?}");
    assert!(ticks.iter().all(|tick| tick.height() > tick.width()), "the ticks stand upright: {ticks:?}");
    let run = ticks.iter().fold(Rect::NOTHING, |run, tick| run.union(*tick));
    assert!((run.center().x - strip.center().x).abs() <= 0.5, "the ticks are centred across the rail: {run:?} in {strip:?}");

    fixture.drag_hold_at(strip.center(), Vec2::new(200.0, 0.0));
    let unit = drag::active::<GroupDrag<usize>>(fixture.ctx()).map(|drag| drag.unit.clone());
    assert_eq!(unit, Some(DragUnit::Group(RAIL)), "a drag on the grip moves the rail's group");
    fixture.release_at(strip.center() + Vec2::new(200.0, 0.0)).settle();
}

/// The rail's grip strip is a memoized view: a pointer moving elsewhere in the window replays it, as it replays the
/// column bars and the tab rows, and runs no grip (photocraft#16333).
#[trial]
fn a_tool_rails_grip_strip_replays_while_the_pointer_moves_elsewhere() {
    let seen = Rc::new(Cell::new(None));
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![RAIL], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let mut workspace = Arc::new(Workspace::new(Layout::new(Some(root), Vec::new())));
    let (mut panels, specs) = (RailPanels { seen: seen.clone() }, Tracked::new(RailSpecs));
    let mut fixture = Fixture::new(move |ui| {
        let _ = DockArea::new("Rail dock", Bind::bound(&mut workspace), &specs, &mut panels, DockConfig::default()).show(ui);
    });
    fixture.room(Vec2::new(800.0, 600.0)).settle();
    let grips =
        |fixture: &Fixture| last_pass_runs(fixture.ctx()).into_iter().filter(|(ty, _)| ty.contains("GroupGrip<")).map(|(_, runs)| runs).sum::<u32>();
    for step in 1..=3 {
        fixture.frame(vec![pointer::moved(egui::pos2(500.0 + step as f32, 300.0))]);
        assert_eq!(grips(&fixture), 0, "pointer step {step}: the rail's grip replays its last run");
    }
}
