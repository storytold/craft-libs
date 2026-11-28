//! Dock Panel returns a floated panel to the place it left: its own frame beside the same neighbour, at the split size
//! it had, also after the float closed and reopened.

use craft_behavior::Axis;
use craft_behavior::dock::{Layout, Node, PanelPlace, Permissions, SplitSize, Workspace};
use craft_theme::{Contrast, ControlSize, Mode, Variant};
use craft_ui::docking::{DockCommands, DockScene, DockSizes, FloatSpace, actions};
use craft_ui::testing::style::{tokens, with_cx};
use craft_ui::testing::trial;
use egui::{Pos2, Rect, vec2};

const EXPLORER: usize = 1;
const LIBRARY: usize = 2;

fn allow(_: &usize) -> Permissions {
    Permissions::ALL
}

/// Explorer above Library, Explorer taking a little over a third.
fn workspace() -> Workspace<usize> {
    let root = Node::split(Axis::Y, SplitSize::Ratio(0.36), Node::tabs(vec![EXPLORER], 0), Node::tabs(vec![LIBRARY], 0));
    Workspace::new(Layout::new(Some(root), Vec::new()))
}

/// Runs `command` on Library's tab menu as the dock does: over the workspace's layout and floated records.
fn run(workspace: &mut Workspace<usize>, command: DockCommands) {
    let sizes = with_cx(&egui::Context::default(), &tokens(Mode::Light, Contrast::Standard), Variant::SECONDARY, ControlSize::Md, DockSizes::style);
    let space = FloatSpace::new(Rect::from_min_size(Pos2::ZERO, vec2(1440.0, 825.0)), &sizes);
    let actions = {
        let scene = DockScene::new(workspace.layout(), &space, false, &allow).floated(workspace.floated());
        actions(&scene, command, &LIBRARY).expect("the command applies to Library")
    };
    workspace.apply_all(actions, false, allow).expect("the workspace takes the command's actions");
}

fn floats(workspace: &Workspace<usize>) -> bool {
    matches!(workspace.layout().place(&LIBRARY), Some(PanelPlace::Floating { .. }))
}

#[trial]
fn dock_panel_returns_a_floated_panel_to_its_own_split() {
    let mut workspace = workspace();
    let docked = workspace.layout().clone();
    run(&mut workspace, DockCommands::FloatPanel);
    assert!(floats(&workspace));
    run(&mut workspace, DockCommands::DockPanel);
    assert_eq!(workspace.layout().root, docked.root, "Library is back below Explorer in its own frame, at the split size it had");
    assert!(workspace.layout().floating.is_empty(), "its window closes");
}

#[trial]
fn dock_panel_returns_a_closed_and_reopened_float_to_its_own_split() {
    let mut workspace = workspace();
    let docked = workspace.layout().clone();
    run(&mut workspace, DockCommands::FloatPanel);
    run(&mut workspace, DockCommands::ClosePanel);
    assert!(workspace.layout().place(&LIBRARY).is_none());
    workspace.set_visible(LIBRARY, true, None, false, allow).expect("Library reopens");
    assert!(floats(&workspace), "a panel closed while floating reopens floating");
    run(&mut workspace, DockCommands::DockPanel);
    assert_eq!(workspace.layout().root, docked.root, "Library is back below Explorer in its own frame, at the split size it had");
    assert!(workspace.layout().floating.is_empty(), "its window closes");
}
