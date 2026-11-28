//! Relations that name a node the update does not carry.

use craft_ui::testing::access::prune_controls;
use craft_ui::testing::trial;
use egui::accesskit::{Node, NodeId, Role, TreeId, TreeUpdate};

fn update(controls: Vec<NodeId>, carried: Vec<NodeId>) -> TreeUpdate {
    let mut node = Node::new(Role::Button);
    node.set_controls(controls);
    TreeUpdate {
        nodes: carried.into_iter().map(|id| (id, Node::new(Role::Button))).chain(std::iter::once((NodeId(1), node))).collect(),
        tree: None,
        tree_id: TreeId::ROOT,
        focus: NodeId(1),
    }
}

fn controls_of_first(tree: &TreeUpdate) -> Vec<NodeId> {
    tree.nodes.iter().find(|(id, _)| *id == NodeId(1)).map(|(_, node)| node.controls().to_vec()).unwrap_or_default()
}

/// A popup that reports itself open on a frame where its node is not emitted leaves its id in
/// `controls`; the host's UIA adapter unwraps that lookup and aborts (photocraft#15597). The id
/// that has a node stays.
#[trial]
fn controls_keep_the_ids_the_update_carries_and_drop_the_rest() {
    let mut tree = update(vec![NodeId(2), NodeId(9)], vec![NodeId(2)]);
    prune_controls(&mut tree);
    assert_eq!(controls_of_first(&tree), vec![NodeId(2)], "the id without a node is gone, the one with a node stays");
}

#[trial]
fn controls_with_every_node_carried_are_left_alone() {
    let mut tree = update(vec![NodeId(2)], vec![NodeId(2)]);
    prune_controls(&mut tree);
    assert_eq!(controls_of_first(&tree), vec![NodeId(2)]);
}
