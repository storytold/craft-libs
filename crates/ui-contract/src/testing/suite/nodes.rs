//! What the container and collection cases read from the AccessKit tree of a fixture.

use crate::runtime::access::Role;
use crate::runtime::egui::access::{AccessTree, Declared, NodeKey};
use crate::testing::fixture::{Fixture, Target};
use crate::testing::story::Story;
use craft_behavior::PartRef;

/// `node` and every node below it.
pub(super) fn subtree(tree: &AccessTree, node: NodeKey) -> Vec<NodeKey> {
    let mut found = vec![node];
    let mut at = 0;
    while at < found.len() {
        let new: Vec<NodeKey> = tree.children_of(found[at]).into_iter().filter(|child| !found.contains(child)).collect();
        found.extend(new);
        at += 1;
    }
    found
}

/// The node of the latest shown component of a story, when the tree has it.
pub(super) fn root(fixture: &Fixture) -> Option<Target> {
    fixture.root_id().and_then(|id| fixture.node_of(id))
}

/// Whether the node AccessKit reports as focused lies in the subtree of `root`.
pub(super) fn focus_within(fixture: &Fixture, root: NodeKey) -> bool {
    let tree = fixture.tree();
    subtree(tree, root).contains(&tree.focused_key())
}

/// The roles of the rows of a collection: list items, list box options, tree items and table rows.
const ROW_ROLES: [Role; 4] = [Role::LIST_ITEM, Role::LIST_BOX_OPTION, Role::TREE_ITEM, Role::ROW];

/// Whether `role` is a row of a collection, which the item step gives `LEAD`, `DRAGGING` and `DROP_TARGET`.
pub(super) fn is_row_role(role: Role) -> bool {
    ROW_ROLES.contains(&role)
}

/// Whether `role` is a menu entry, which shows `LEAD` on the entry the menu rests on.
pub(super) fn is_menu_item_role(role: Role) -> bool {
    [Role::MENU_ITEM, Role::MENU_ITEM_CHECK_BOX, Role::MENU_ITEM_RADIO].contains(&role)
}

/// The rows of the collection below `root`, top to bottom. A node with no place in a set, such as a header row, is
/// not a row.
pub(super) fn rows(fixture: &Fixture, root: NodeKey) -> Vec<Target> {
    let tree = fixture.tree();
    let inside = subtree(tree, root);
    let mut rows: Vec<Target> = ROW_ROLES
        .into_iter()
        .flat_map(|role| fixture.by_role(role))
        .filter(|target| inside.contains(&target.node) && position_in_set(fixture, target.node).is_some())
        .collect();
    rows.sort_by(|a, b| a.rect.min.y.total_cmp(&b.rect.min.y));
    rows
}

/// The row of `rows` that holds the AccessKit focus, with its place among them.
pub(super) fn focused_row(fixture: &Fixture, rows: &[Target]) -> Option<(usize, NodeKey)> {
    let tree = fixture.tree();
    let mut at = Some(tree.focused_key());
    while let Some(key) = at {
        if let Some(index) = rows.iter().position(|row| row.node == key) {
            return Some((index, key));
        }
        at = tree.parent_of_key(key);
    }
    None
}

/// The zero-based position and the set size a row reports.
pub(super) fn position_in_set(fixture: &Fixture, row: NodeKey) -> Option<(usize, usize)> {
    let node = fixture.tree().data_of(row)?;
    Some((node.position_in_set()?, node.size_of_set()?))
}

/// Whether the node reports itself selected.
pub(super) fn is_selected(fixture: &Fixture, node: NodeKey) -> bool {
    fixture.tree().data_of(node).and_then(|data| data.is_selected()).unwrap_or(false)
}

/// Whether the cases that run once per member of a repeated set (each repeat of a part, each row that declares an
/// action) run on a fixed sample of the set: under the gate's sweep fraction, `craft_testkit::hostile::divisor` above
/// 1. The nightly run sets no fraction and checks every member.
fn samples_repeats() -> bool {
    #[cfg(not(target_arch = "wasm32"))]
    {
        craft_testkit::hostile::divisor() > 1
    }
    #[cfg(target_arch = "wasm32")]
    {
        false
    }
}

/// The first, middle and last of `members`; all of them when there are three or fewer. The members of a repeated set
/// come from one code path, so these cover it, its edges included.
fn sample<T: Copy>(members: &[T]) -> Vec<T> {
    match members.len() {
        0..=3 => members.to_vec(),
        n => vec![members[0], members[n / 2], members[n - 1]],
    }
}

/// `parts` with each repeated part ([`PartRef::index`]) cut to the lowest, middle and highest index of its key when
/// [`samples_repeats`]; a part that occurs once always stays.
pub(super) fn sampled_parts(parts: Vec<PartRef>) -> Vec<PartRef> {
    if !samples_repeats() {
        return parts;
    }
    let mut kept: Vec<PartRef> = Vec::new();
    for part in &parts {
        if part.index.is_none() {
            kept.push(*part);
            continue;
        }
        if kept.iter().any(|done| done.key == part.key && done.index.is_some()) {
            continue;
        }
        let mut indices: Vec<u64> = parts.iter().filter(|other| other.key == part.key).filter_map(|other| other.index).collect();
        indices.sort_unstable();
        indices.dedup();
        kept.extend(sample(&indices).into_iter().map(|index| part.key.at(index)));
    }
    kept
}

/// The keys of `tree` depth first from its root, children in their declared order; nodes the root does not reach
/// follow in the tree's own order.
fn tree_order(tree: &AccessTree) -> Vec<NodeKey> {
    let mut ordered = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut pending: Vec<NodeKey> = tree.root_key().into_iter().collect();
    while let Some(key) = pending.pop() {
        if seen.insert(key) {
            ordered.push(key);
            pending.extend(tree.children_of(key).into_iter().rev());
        }
    }
    ordered.extend(tree.keys().into_iter().filter(|key| !seen.contains(key)));
    ordered
}

/// `pairs` (a node and an action it declares) in tree order; when [`samples_repeats`], set by set, each repeated set
/// cut to its first, middle and last node. Nodes that share a parent and a role and declare the same action are one set.
pub(super) fn sampled_actions(tree: &AccessTree, mut pairs: Vec<(NodeKey, Declared)>) -> Vec<(NodeKey, Declared)> {
    let rank: std::collections::HashMap<NodeKey, usize> = tree_order(tree).into_iter().enumerate().map(|(at, key)| (key, at)).collect();
    pairs.sort_by_key(|(node, _)| rank.get(node).copied().unwrap_or(usize::MAX));
    let ordered = pairs;
    if !samples_repeats() {
        return ordered;
    }
    let set_of = |(node, declared): &(NodeKey, Declared)| (tree.parent_of_key(*node), tree.role_of(*node), *declared);
    let mut sets: Vec<Vec<(NodeKey, Declared)>> = Vec::new();
    for pair in ordered {
        match sets.iter_mut().find(|set| set_of(&set[0]) == set_of(&pair)) {
            Some(set) => set.push(pair),
            None => sets.push(vec![pair]),
        }
    }
    sets.iter().flat_map(|set| sample(set)).collect()
}

/// `stories` cut, when [`samples_repeats`], to the first story of each distinct kind: stories that cover the same
/// required states, drive the same way, give the same disabled reason and set command states alike differ only in
/// what they show, which the cases that read the type as a whole (`reads`, `variants`, `containers`) see through
/// the first of them. The nightly run keeps every story.
pub(super) fn sampled_stories(stories: Vec<Story>) -> Vec<Story> {
    if !samples_repeats() {
        return stories;
    }
    let kind = |story: &Story| (story.covers, std::mem::discriminant(&story.drive), story.disabled_text, story.commands.is_some());
    let mut kept: Vec<Story> = Vec::new();
    for story in stories {
        if !kept.iter().any(|known| kind(known) == kind(&story)) {
            kept.push(story);
        }
    }
    kept
}
