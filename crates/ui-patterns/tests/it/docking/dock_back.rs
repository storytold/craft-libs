//! Dock Panel with no docked place recorded: the floating unit joins the first docked group that takes it.

use craft_behavior::Axis;
use craft_behavior::dock::{Column, DockEdge, Floating, Group, Layout, Node, Permissions, SplitSize};
use craft_ui::docking::{DockScene, DockTo, DragUnit, FloatSpace, move_actions};
use craft_ui::testing::trial;
use egui::{Pos2, Rect};

const CANVAS: usize = 1;
const TOOLBAR: usize = 2;
const LAYERS: usize = 3;
const FLOATED: usize = 4;

fn rights(panel: &usize) -> Permissions {
    match *panel {
        CANVAS => Permissions::PROTECTED,
        TOOLBAR => Permissions::ALL.accepts_tabs(false).accepts_splits(false).collapses(false).shares_column(false),
        _ => Permissions::ALL,
    }
}

/// The toolbar alone in the start dock's only column, the canvas, Layers in the end dock, and a floating panel.
fn layout() -> Layout<usize> {
    let start = Node::dock(DockEdge::Start, vec![Column::new(vec![Group::new(vec![TOOLBAR], 0)])]);
    let end = Node::dock(DockEdge::End, vec![Column::new(vec![Group::new(vec![LAYERS], 0)])]);
    let root = Node::split(Axis::X, SplitSize::Ratio(0.2), start, Node::split(Axis::X, SplitSize::Ratio(0.7), Node::tabs(vec![CANVAS], 0), end));
    let window = Rect::from_min_size(Pos2::new(100.0, 100.0), egui::vec2(200.0, 150.0));
    Layout::new(Some(root), vec![Floating::single(100, vec![FLOATED], 0, window)])
}

#[trial]
fn a_floating_panel_docks_into_the_first_group_that_takes_tabs_and_shares_its_column() {
    let mut layout = layout();
    let space = FloatSpace::new(Rect::from_min_size(Pos2::ZERO, egui::vec2(800.0, 600.0)), &super::sizes(4.0, 60.0, 20.0));
    let actions = {
        let scene = DockScene::new(&layout, &space, false, &rights);
        move_actions(&scene, &DragUnit::Panel(FLOATED), DockTo::Docked).expect("a docked group takes the panel")
    };
    layout.apply_all(actions, rights).expect("the model takes the move");
    let group_of = |panel: usize| layout.place(&panel).and_then(|place| layout.group(&place)).map(|group| group.panels.clone());
    assert!(group_of(FLOATED).is_some_and(|panels| panels.contains(&LAYERS)), "the panel joins Layers' tabs, past the toolbar's group");
    assert_eq!(group_of(TOOLBAR), Some(vec![TOOLBAR]), "the toolbar keeps its group to itself");
    assert!(layout.floating.is_empty(), "the window closes with its only panel");
}
