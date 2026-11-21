//! The settled focus of the access tree: a focus that names no node of the update is the tree root, in the tree a
//! frame hands out and in the per-viewport record the window host keeps.

use craft_ui::contract::{Props as _, Show as _};
use craft_ui::runtime::windows::WindowHost;
use craft_ui::testing::access::{NodeKey, focused_key};
use craft_ui::testing::{Frames, trial};
use craft_ui_test_kit::contract::{Probe, Seen};
use craft_ui_test_kit::windows::Windows;
use egui::{Id, Ui, ViewportBuilder, ViewportId};
use std::cell::RefCell;

/// Gives egui focus to a widget that is never shown, so it has no node in the pass's update.
fn focus_nowhere(ui: &mut Ui) {
    ui.memory_mut(|memory| memory.request_focus(Id::unique("a widget with no node")));
}

#[trial]
fn access_focus_outside_the_nodes_moves_to_the_tree_root() {
    let seen = RefCell::new(Seen::default());
    let mut frames = Frames::new();
    let root = NodeKey::of(egui::accesskit_root_id());
    for frame in 0..3 {
        frames.frame(vec![], |ui| {
            Probe::new(&seen).label("Shown").show(ui);
            focus_nowhere(ui);
        });
        let tree = frames.tree();
        assert!(tree.has(seen.borrow().id.expect("the probe ran")), "frame {frame}: the shown node is in the tree");
        assert_eq!(focused_key(&tree), root, "frame {frame}: the focus names no node, so the root has it");
    }
}

#[trial]
fn the_host_reports_each_viewports_settled_access_tree() {
    let windows = Windows::new();
    let host = WindowHost::of(&windows.ctx);
    let child = ViewportId::from_hash_of("settled tree window");
    let (main, kid) = (RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    for _ in 0..2 {
        windows.frame(vec![], vec![], |ui| {
            Probe::new(&main).label("Main").show(ui);
            focus_nowhere(ui);
            ui.ctx().show_viewport_immediate(child, ViewportBuilder::default(), |cui, _| {
                Probe::new(&kid).label("Kid").show(cui);
            });
        });
    }
    let (main, kid) = (main.borrow().id.expect("the main window ran"), kid.borrow().id.expect("the second window ran"));
    let root = egui::accesskit_root_id().accesskit_id();

    let main_tree = host.access_tree(ViewportId::ROOT).expect("the main window's tree is recorded");
    let main_tree = main_tree.update();
    let main_nodes: Vec<u64> = main_tree.nodes.iter().map(|(id, _)| id.0).collect();
    assert!(main_nodes.contains(&main.accesskit_id().0), "the main window's tree holds its node");
    assert!(!main_nodes.contains(&kid.accesskit_id().0), "the main window's tree holds no node of the second window");
    assert_eq!(main_tree.tree.as_ref().map(|tree| tree.root), Some(root), "the tree is rooted at egui's root");
    assert_eq!(main_tree.focus, root, "the recorded tree carries the settled focus");

    let kid_tree = host.access_tree(child).expect("the second window's tree is recorded");
    let kid_tree = kid_tree.update();
    let kid_nodes: Vec<u64> = kid_tree.nodes.iter().map(|(id, _)| id.0).collect();
    assert!(kid_nodes.contains(&kid.accesskit_id().0), "the second window's tree is its own");
    assert!(!kid_nodes.contains(&main.accesskit_id().0), "the second window's tree holds no node of the main window");

    assert!(host.access_tree(ViewportId::from_hash_of("never opened")).is_none(), "a viewport that never ran has no tree");
}
