//! Dock columns: room sharing between groups, and a dock side's total width shared between its columns.

use craft_behavior::Axis;
use craft_behavior::dock::{Action, Column, DockEdge, Group, Layout, Node, Permissions, SplitSize, Workspace, node_at};
use craft_ui::contract::{Bind, BodyCx, Role, Show as _, SizeLimits};
use craft_ui::display::Text;
use craft_ui::docking::{DockConfig, DockGrid, DockHeights, DockPanels, GroupSize, PanelSpec, group_heights};
use craft_ui::patterns::dock::DockArea;
use craft_ui::testing::{Fixture, trial};
use egui::{Rect, Ui, Vec2};
use std::cell::RefCell;
use std::rc::Rc;

const STRIP: f32 = 20.0;
const MIN: f32 = 60.0;
const ROOM: f32 = 400.0;

fn open(min: f32) -> GroupSize {
    GroupSize::new(min)
}

fn sum(heights: &[f32]) -> f32 {
    heights.iter().sum()
}

#[trial]
fn default_heights_give_way_to_the_group_that_fills_before_it_gets_less() {
    let mut roomy = open(MIN);
    roomy.content = Some(DockHeights::new(250.0).compact(100.0));
    let mut filling = open(MIN);
    filling.content = Some(DockHeights::new(0.0).fill(200.0));
    let heights = group_heights(&[roomy, filling], STRIP, ROOM);
    assert_eq!(heights[1], 200.0, "the filling group gets its fill height");
    assert_eq!(heights[0], ROOM - 200.0, "the other group gave way from its default of 250");
    assert!(sum(&heights) <= ROOM, "the groups fit the room");
}

#[trial]
fn only_the_last_open_group_fills_the_column() {
    let asks = || {
        let mut group = open(MIN);
        group.content = Some(DockHeights::new(100.0).compact(80.0).fill(150.0));
        group
    };
    let mut minimized = asks();
    minimized.minimized = true;
    let heights = group_heights(&[asks(), asks(), minimized], STRIP, ROOM);
    assert_eq!(heights[0], 100.0, "an earlier group keeps its default although it asks for a fill");
    assert!(heights[1] > heights[0], "the last open group takes the rest");
    assert_eq!(sum(&heights), ROOM, "the column is filled");
}

/// Canvas 1 beside an end dock: column A holds panel 2 (200 to 400 wide), column B panel 3 (at least 150), and
/// column C, iconic, panel 4.
fn end_dock_layout() -> Layout<u32> {
    let mut iconic = Column::new(vec![Group::new(vec![4], 0)]).with_width(300.0);
    iconic.iconic = true;
    let columns =
        vec![Column::new(vec![Group::new(vec![2], 0)]).with_width(250.0), Column::new(vec![Group::new(vec![3], 0)]).with_width(250.0), iconic];
    Layout::new(Some(Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns))), Vec::new())
}

fn spec(panel: &u32) -> PanelSpec {
    let limits = match panel {
        2 => SizeLimits::new(Vec2::new(200.0, 0.0), Vec2::new(400.0, 10_000.0)),
        3 => SizeLimits::at_least(Vec2::new(150.0, 0.0)),
        _ => SizeLimits::default(),
    };
    PanelSpec::new("Panel").limits(limits)
}

fn end_columns(layout: &Layout<u32>) -> Vec<Column<u32>> {
    let path = layout.dock_path(DockEdge::End).expect("the layout has an end dock");
    match node_at(layout.root.as_ref().unwrap(), &path) {
        Some(Node::Dock { columns, .. }) => columns.clone(),
        other => panic!("{other:?}"),
    }
}

#[trial]
fn a_dock_sides_total_width_scales_its_columns_within_their_bounds() {
    let sizes = super::sizes(4.0, 100.0, STRIP);
    let spec: &dyn Fn(&u32) -> PanelSpec = &spec;
    let mut live = Workspace::new(end_dock_layout());
    let widths = |live: &Workspace<u32>| end_columns(&live.layout).iter().map(|column| sizes.width(column, spec)).collect::<Vec<f32>>();
    let within = |live: &Workspace<u32>| {
        end_columns(&live.layout).iter().zip(widths(live)).all(|(column, width)| {
            let (lo, hi) = sizes.shown_bounds(column, spec);
            (lo - 0.01..=hi + 0.01).contains(&width)
        })
    };
    let start = widths(&live);

    for total in [sum(&start) + 120.0, sum(&start) - 60.0] {
        let actions = sizes.resize_side(&live.layout, DockEdge::End, spec, total);
        live.apply_all(actions, true, |_| Permissions::ALL).expect("resizing works while the workspace is locked");
        assert!((sum(&widths(&live)) - total).abs() < 0.5, "the columns add up to {total}: {:?}", widths(&live));
        assert!(within(&live), "every column stays within its bounds: {:?}", widths(&live));
    }

    let actions = sizes.resize_side(&live.layout, DockEdge::End, spec, 10.0);
    live.apply_all(actions, false, |_| Permissions::ALL).unwrap();
    let least: f32 = end_columns(&live.layout).iter().map(|column| sizes.shown_bounds(column, spec).0).sum();
    assert!((sum(&widths(&live)) - least).abs() < 0.5, "a total below the bounds stops at their sum");
    let iconic = &end_columns(&live.layout)[2];
    assert_eq!(iconic.width, Some(300.0), "an iconic column keeps the width it expands to");
    assert!(sizes.resize_side(&live.layout, DockEdge::End, spec, least).is_empty(), "the width it has asks for nothing");
    assert!(sizes.resize_side(&live.layout, DockEdge::Start, spec, 500.0).is_empty(), "a side without a dock asks for nothing");
}

/// The panel that lays its content in a grid, as a toolbar does.
const TOOLS: u32 = 5;

fn grid() -> DockGrid {
    DockGrid::new(40.0, 32.0, 6)
}

/// A stepped column keeps its width, which only its steps set, while the side's other columns scale; a width that
/// its edge is dragged to takes the most whole steps that fit it.
#[trial]
fn a_stepped_column_keeps_its_width_while_the_others_scale() {
    let sizes = super::sizes(4.0, 100.0, STRIP);
    let with_tools = |panel: &u32| if *panel == TOOLS { PanelSpec::new("Tools").grid(grid()) } else { spec(panel) };
    let spec: &dyn Fn(&u32) -> PanelSpec = &with_tools;
    let columns = vec![Column::new(vec![Group::new(vec![2], 0)]).with_width(250.0), Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(2)];
    let root = Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![1], 0), Node::dock(DockEdge::End, columns));
    let mut live = Workspace::new(Layout::new(Some(root), Vec::new()));
    let widths = |live: &Workspace<u32>| end_columns(&live.layout).iter().map(|column| sizes.width(column, spec)).collect::<Vec<f32>>();
    let before = widths(&live);
    assert_eq!(before[1], grid().width(2), "a stepped column is as wide as its steps");
    let actions = sizes.resize_side(&live.layout, DockEdge::End, spec, sum(&before) + 80.0);
    let touches_tools = |action: &Action<u32>| matches!(action, Action::ResizeColumn { panel: TOOLS, .. } | Action::SetColumnSteps { .. });
    assert!(!actions.iter().any(touches_tools), "the side's width leaves the stepped column alone: {actions:?}");
    live.apply_all(actions, false, |_| Permissions::ALL).expect("the side resizes");
    let after = widths(&live);
    assert_eq!(after[1], before[1], "the stepped column keeps its width");
    assert!(after[0] > before[0], "the other column took the room: {after:?}");

    let grid = grid();
    assert_eq!(grid.steps_within(grid.width(3) - 1.0), 2, "a width short of three steps takes two");
    assert_eq!(grid.steps_within(grid.width(3)), 3, "the width of three steps takes three");
    assert_eq!(grid.steps_within(0.0), 1, "a column keeps one step");
    assert_eq!(grid.steps_within(f32::MAX), grid.most, "no more steps than the grid has columns");
}

/// A toolbar: a panel whose two lines of content lay out in a grid and never collapse.
struct Tools;

impl DockPanels<u32> for Tools {
    fn spec(&self, panel: &u32) -> PanelSpec {
        if *panel == TOOLS { PanelSpec::new("Tools").grid(grid()) } else { PanelSpec::new("Canvas") }
    }

    fn permissions(&self, panel: &u32) -> Permissions {
        if *panel == TOOLS { Permissions::ALL.collapses(false) } else { Permissions::ALL }
    }

    fn body(&mut self, panel: &u32, ui: &mut Ui, _: &mut BodyCx<'_>) {
        if *panel == TOOLS {
            Text::new("One").show(ui);
            Text::new("Two").show(ui);
        }
    }
}

/// A docked stepped column is only as tall as its content, not the dock's height; the double arrow of a column that
/// does not collapse switches it between one and two steps.
#[trial]
fn a_docked_stepped_column_fits_its_content_and_its_double_arrow_switches_one_and_two_steps() {
    let dock = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![TOOLS], 0)]).with_steps(1)]);
    let root = Node::split(Axis::X, SplitSize::FixedFirst(120.0), dock, Node::tabs(vec![1], 0));
    let workspace = Rc::new(RefCell::new(Workspace::new(Layout::new(Some(root), Vec::new()))));
    let placed: Rc<RefCell<Vec<(u32, Rect)>>> = Rc::default();
    let (held, seen) = (Rc::clone(&workspace), Rc::clone(&placed));
    let mut fixture = Fixture::new(move |ui| {
        let mut workspace = held.borrow().clone();
        let out = DockArea::new("Stepped dock", Bind::bound(&mut workspace), &mut Tools, DockConfig::default()).show(ui);
        *held.borrow_mut() = workspace;
        *seen.borrow_mut() = out.columns;
    });
    fixture.settle();
    let tools = placed.borrow().iter().find(|(panel, _)| *panel == TOOLS).map(|(_, rect)| *rect).expect("the stepped column shows");
    let canvas =
        fixture.by_role(Role::TAB_PANEL).into_iter().map(|group| group.rect).find(|rect| !tools.contains_rect(*rect)).expect("the canvas shows");
    assert!(tools.bottom() < canvas.bottom() - 10.0, "the column ends below its content, above the dock's foot: {tools:?}, canvas {canvas:?}");

    let steps = |workspace: &Workspace<u32>| match &workspace.layout.root {
        Some(Node::Split { first, .. }) => match first.as_ref() {
            Node::Dock { columns, .. } => columns[0].steps,
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    };
    for expected in [2, 1] {
        let tree = fixture.tree();
        let arrow = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .find(|button| craft_ui::testing::access::name(tree, button.node) == Some("Collapse column to icons"))
            .expect("the column bar shows its double arrow");
        fixture.click(&arrow).settle();
        assert_eq!(steps(&workspace.borrow()), Some(expected), "the double arrow switches the steps");
    }
}
