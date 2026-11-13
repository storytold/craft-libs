//! Dock columns: room sharing between groups, and a dock side's total width shared between its columns.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Group, Layout, Node, Permissions, SplitSize, Workspace, node_at};
use craft_ui::contract::SizeLimits;
use craft_ui::docking::{DockHeights, GroupSize, PanelSpec, group_heights};
use craft_ui::testing::trial;
use egui::Vec2;

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
