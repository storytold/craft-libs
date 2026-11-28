//! A second window open while the main window's tree is checked.

use craft_ui::contract::{Props as _, Show as _, ShowContainer as _};
use craft_ui::testing::access::NodeKey;
use craft_ui::testing::trial;
use craft_ui_test_kit::contract::{Panel, Probe, Seen};
use craft_ui_test_kit::windows::Windows;
use egui::{ViewportBuilder, ViewportId};
use std::cell::RefCell;

const CHILD: &str = "second window";

/// The nodes below `top` in tree order: a container's regions are generic containers that parent's children lists
/// pass through, so the order of a container's children is the order of this walk.
fn below(tree: &craft_ui::testing::access::AccessTree, top: NodeKey) -> Vec<NodeKey> {
    let mut walk = Vec::new();
    for child in tree.children_of(top) {
        walk.push(child);
        walk.extend(below(tree, child));
    }
    walk
}

fn child() -> ViewportId {
    ViewportId::from_hash_of(CHILD)
}

/// The nodes of three probes in show order below a container whose pass opens a second window between the second
/// and the third probe: the second window's pass runs nested inside the main window's.
#[trial]
fn a_second_window_adds_no_nodes_to_the_main_windows_tree_and_leaves_its_order() {
    let windows = Windows::new();
    let (first, second, third, kid) =
        (RefCell::new(Seen::default()), RefCell::new(Seen::default()), RefCell::new(Seen::default()), RefCell::new(Seen::default()));
    let mut panel = None;
    let mut out = None;
    for _ in 0..3 {
        out = Some(windows.frame(vec![], vec![], |ui| {
            let shown = Panel::new().label("Main").id_salt("main").show(ui, |ui, _| {
                Probe::new(&first).label("First").show(ui);
                Probe::new(&second).label("Second").show(ui);
                ui.ctx().show_viewport_immediate(child(), ViewportBuilder::default(), |cui, _| {
                    Probe::new(&kid).label("Kid").show(cui);
                });
                Probe::new(&third).label("Third").show(ui);
            });
            panel = Some(shown.output.id);
        }));
    }
    let out = out.expect("a frame ran");
    let (panel, ids) = (panel.expect("shown"), [&first, &second, &third].map(|seen| seen.borrow().id.expect("updated")));
    let kid = kid.borrow().id.expect("the second window ran");
    let (_, main) = out.trees.iter().find(|(id, _)| *id == ViewportId::ROOT).expect("the main window made a tree");
    let (_, other) = out.trees.iter().find(|(id, _)| *id == child()).expect("the second window made a tree");
    assert!(!main.has(kid), "the second window's node is not in the main window's tree");
    assert!(other.has(kid), "the second window's tree holds its own node");
    assert!(ids.iter().all(|id| !other.has(*id)), "the main window's nodes are not in the second window's tree");
    let keys = ids.map(NodeKey::of);
    let order: Vec<NodeKey> = below(main, NodeKey::of(panel)).into_iter().filter(|key| keys.contains(key)).collect();
    assert_eq!(order, keys, "the probes keep their show order below the container");
}
