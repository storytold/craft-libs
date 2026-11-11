//! A finished AccessKit tree update, read by trials without naming AccessKit.

use super::{NodeRole, node_key};
use crate::runtime::access::{GridPlace, Numeric, Politeness, PopupKind, Role};
use accesskit::{Action, Node, NodeId, TreeUpdate};
use egui::Id;
use std::collections::{HashMap, HashSet};

/// A node's number in the tree.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct NodeKey(u64);

impl NodeKey {
    /// The AccessKit node number.
    pub(crate) fn raw(self) -> u64 {
        self.0
    }

    /// The key of the node of `id`.
    pub fn of(id: Id) -> NodeKey {
        NodeKey(node_key(id))
    }
}

/// An action a node declares.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Declared {
    /// Focus.
    Focus,
    /// Click.
    Click,
    /// Increment.
    Increment,
    /// Decrement.
    Decrement,
    /// Set a value.
    SetValue,
    /// Set the text selection.
    SetTextSelection,
    /// Expand.
    Expand,
    /// Collapse.
    Collapse,
    /// A custom action.
    Custom,
    /// Scroll up by a page.
    ScrollUp,
    /// Scroll down by a page.
    ScrollDown,
    /// Scroll left by a page.
    ScrollLeft,
    /// Scroll right by a page.
    ScrollRight,
}

impl Declared {
    fn action(self) -> Action {
        match self {
            Declared::Focus => Action::Focus,
            Declared::Click => Action::Click,
            Declared::Increment => Action::Increment,
            Declared::Decrement => Action::Decrement,
            Declared::SetValue => Action::SetValue,
            Declared::SetTextSelection => Action::SetTextSelection,
            Declared::Expand => Action::Expand,
            Declared::Collapse => Action::Collapse,
            Declared::Custom => Action::CustomAction,
            Declared::ScrollUp => Action::ScrollUp,
            Declared::ScrollDown => Action::ScrollDown,
            Declared::ScrollLeft => Action::ScrollLeft,
            Declared::ScrollRight => Action::ScrollRight,
        }
    }
}

/// Every node that has a hidden node among its ancestors.
fn nodes_below_hidden(update: &TreeUpdate) -> HashSet<NodeId> {
    let by_id: HashMap<NodeId, &Node> = update.nodes.iter().map(|(id, node)| (*id, node)).collect();
    let mut below = HashSet::new();
    let mut pending: Vec<NodeId> = update.nodes.iter().filter(|(_, node)| node.is_hidden()).map(|(id, _)| *id).collect();
    while let Some(id) = pending.pop() {
        let Some(node) = by_id.get(&id) else { continue };
        for child in node.children() {
            if below.insert(*child) {
                pending.push(*child);
            }
        }
    }
    below
}

/// The AccessKit tree update of one frame.
#[non_exhaustive]
pub struct AccessTree {
    update: TreeUpdate,
    /// The nodes below a hidden node: assistive technology does not report them, so no lookup finds them.
    below_hidden: HashSet<NodeId>,
}

impl AccessTree {
    pub(crate) fn new(update: TreeUpdate) -> Self {
        let below_hidden = nodes_below_hidden(&update);
        AccessTree { update, below_hidden }
    }

    /// The tree of a frame that made none.
    pub(crate) fn none() -> Self {
        AccessTree::new(TreeUpdate { nodes: Vec::new(), tree: None, tree_id: accesskit::TreeId::ROOT, focus: NodeId(0) })
    }

    fn node(&self, id: Id) -> Option<&Node> {
        self.lookup(NodeId(node_key(id)))
    }

    fn lookup(&self, id: NodeId) -> Option<&Node> {
        if self.below_hidden.contains(&id) {
            return None;
        }
        self.update.nodes.iter().find(|(node, _)| *node == id).map(|(_, node)| node)
    }

    /// Whether the node of `id` exists.
    pub fn has(&self, id: Id) -> bool {
        self.node(id).is_some()
    }

    /// The role of the node of `id`.
    pub fn role(&self, id: Id) -> Option<Role> {
        self.node(id).map(|node| Role::Node(node.role()))
    }

    /// The name of the node of `id`.
    pub fn name(&self, id: Id) -> Option<&str> {
        self.node(id)?.label()
    }

    /// The description of the node of `id`.
    pub fn description(&self, id: Id) -> Option<&str> {
        self.node(id)?.description()
    }

    /// The description of a node by key.
    pub fn description_of(&self, key: NodeKey) -> Option<&str> {
        self.lookup(NodeId(key.0))?.description()
    }

    // fwk-subnode
    /// The keyboard shortcut a node by key lists.
    pub fn shortcut_of(&self, key: NodeKey) -> Option<&str> {
        self.lookup(NodeId(key.0))?.keyboard_shortcut()
    }
    // End of fwk-subnode.

    /// The text value of a node by key.
    pub fn value_of(&self, key: NodeKey) -> Option<&str> {
        self.lookup(NodeId(key.0))?.value()
    }

    /// The text value of the node of `id`.
    pub fn value(&self, id: Id) -> Option<&str> {
        self.node(id)?.value()
    }

    /// The numeric value and range of the node of `id`.
    pub fn numeric(&self, id: Id) -> Option<Numeric> {
        let node = self.node(id)?;
        let range = node.min_numeric_value()?..=node.max_numeric_value()?;
        Some(Numeric { value: node.numeric_value(), range, step: node.numeric_value_step(), wraps: false, text: node.value().map(str::to_owned) })
    }

    /// Whether the node of `id` is reported as disabled.
    pub fn is_disabled(&self, id: Id) -> bool {
        self.node(id).is_some_and(Node::is_disabled)
    }

    /// Whether the node of `id` declares `action`.
    pub fn declares(&self, id: Id, action: Declared) -> bool {
        self.node(id).is_some_and(|node| node.supports_action(action.action()))
    }

    /// The nodes that name the node of `id`.
    pub fn labelled_by(&self, id: Id) -> Vec<NodeKey> {
        self.labelled_by_key(NodeKey::of(id))
    }

    /// [`AccessTree::labelled_by`] for a node by key.
    pub fn labelled_by_key(&self, key: NodeKey) -> Vec<NodeKey> {
        self.lookup(NodeId(key.0)).map_or_else(Vec::new, |node| node.labelled_by().iter().map(|key| NodeKey(key.0)).collect())
    }

    /// The nodes that describe the node of `id`.
    pub fn described_by(&self, id: Id) -> Vec<NodeKey> {
        self.node(id).map_or_else(Vec::new, |node| node.described_by().iter().map(|key| NodeKey(key.0)).collect())
    }

    /// The place of the node of `id` in its set: its zero-based index and the size of the set.
    pub fn position(&self, id: Id) -> Option<(usize, usize)> {
        let node = self.node(id)?;
        Some((node.position_in_set()?, node.size_of_set()?))
    }

    /// The row and column indexes and counts of a node by key.
    pub fn grid_of(&self, key: NodeKey) -> GridPlace {
        self.lookup(NodeId(key.0)).map_or_else(GridPlace::default, |node| GridPlace {
            row_index: node.row_index(),
            row_count: node.row_count(),
            column_index: node.column_index(),
            column_count: node.column_count(),
        })
    }

    /// The sort order a column header node by key shows: `true` descending.
    pub fn sort_of(&self, key: NodeKey) -> Option<bool> {
        match self.lookup(NodeId(key.0))?.sort_direction()? {
            accesskit::SortDirection::Descending => Some(true),
            accesskit::SortDirection::Ascending => Some(false),
            _ => None,
        }
    }

    /// Whether a column header node declares a sort direction: it sorts when activated, sorted now or not.
    pub fn declares_sort(&self, key: NodeKey) -> bool {
        self.lookup(NodeId(key.0)).is_some_and(|node| node.sort_direction().is_some())
    }

    /// The expanded state of the node of `id`.
    pub fn expanded(&self, id: Id) -> Option<bool> {
        self.node(id)?.is_expanded()
    }

    /// The kind of popup the node of `id` opens.
    pub fn has_popup(&self, id: Id) -> Option<PopupKind> {
        match self.node(id)?.has_popup()? {
            accesskit::HasPopup::Menu => Some(PopupKind::Menu),
            accesskit::HasPopup::Listbox => Some(PopupKind::Listbox),
            accesskit::HasPopup::Tree => Some(PopupKind::Tree),
            accesskit::HasPopup::Grid => Some(PopupKind::Grid),
            accesskit::HasPopup::Dialog => Some(PopupKind::Dialog),
        }
    }

    /// The nodes the node of `id` controls.
    pub fn controls(&self, id: Id) -> Vec<NodeKey> {
        self.node(id).map_or_else(Vec::new, |node| node.controls().iter().map(|key| NodeKey(key.0)).collect())
    }

    /// Whether the node of `key` is modal.
    pub fn is_modal(&self, key: NodeKey) -> bool {
        self.lookup(NodeId(key.0)).is_some_and(Node::is_modal)
    }

    /// The live politeness of the node of `id`.
    pub fn live(&self, id: Id) -> Option<Politeness> {
        match self.node(id)?.live()? {
            accesskit::Live::Assertive => Some(Politeness::Assertive),
            accesskit::Live::Polite => Some(Politeness::Polite),
            accesskit::Live::Off => None,
        }
    }

    /// Whether the node of `id` is marked invalid.
    pub fn invalid(&self, id: Id) -> bool {
        self.node(id).is_some_and(|node| node.invalid().is_some())
    }

    /// The text of each `TextRun` node below `id`, in order.
    pub fn run_texts(&self, id: Id) -> Vec<String> {
        let Some(node) = self.node(id) else { return Vec::new() };
        node.children()
            .iter()
            .filter_map(|child| self.lookup(*child))
            .filter(|child| child.role() == accesskit::Role::TextRun)
            .filter_map(|child| child.value().map(str::to_owned))
            .collect()
    }

    /// The selection of the text node `id` as the run node and character index of its anchor and focus.
    pub fn text_selection(&self, id: Id) -> Option<((NodeKey, usize), (NodeKey, usize))> {
        let selection = self.node(id)?.text_selection()?;
        let end = |position: accesskit::TextPosition| (NodeKey(position.node.0), position.character_index);
        Some((end(selection.anchor), end(selection.focus)))
    }

    /// The nodes of `role` that assistive technology reports: not hidden, and not below a hidden node.
    pub fn with_role(&self, role: Role) -> Vec<NodeKey> {
        let Role::Node(role) = role else { return Vec::new() };
        self.update
            .nodes
            .iter()
            .filter(|(key, node)| node.role() == role && !node.is_hidden() && !self.below_hidden.contains(key))
            .map(|(key, _)| NodeKey(key.0))
            .collect()
    }

    /// The name of a node by key.
    pub fn name_of(&self, key: NodeKey) -> Option<&str> {
        self.lookup(NodeId(key.0))?.label()
    }

    /// The toggled state of a node by key.
    pub fn toggled_of(&self, key: NodeKey) -> Option<craft_behavior::Mixed<bool>> {
        Some(match self.lookup(NodeId(key.0))?.toggled()? {
            accesskit::Toggled::False => craft_behavior::Mixed::Same(false),
            accesskit::Toggled::True => craft_behavior::Mixed::Same(true),
            accesskit::Toggled::Mixed => craft_behavior::Mixed::Mixed,
        })
    }

    /// The expanded state of a node by key.
    pub fn expanded_of(&self, key: NodeKey) -> Option<bool> {
        self.lookup(NodeId(key.0))?.is_expanded()
    }

    /// The kind of popup the node of `key` opens.
    pub fn has_popup_of(&self, key: NodeKey) -> Option<PopupKind> {
        match self.lookup(NodeId(key.0))?.has_popup()? {
            accesskit::HasPopup::Menu => Some(PopupKind::Menu),
            accesskit::HasPopup::Listbox => Some(PopupKind::Listbox),
            accesskit::HasPopup::Tree => Some(PopupKind::Tree),
            accesskit::HasPopup::Grid => Some(PopupKind::Grid),
            accesskit::HasPopup::Dialog => Some(PopupKind::Dialog),
        }
    }

    /// The access key of the node of `key`.
    pub fn access_key_of(&self, key: NodeKey) -> Option<char> {
        self.lookup(NodeId(key.0))?.access_key()?.chars().next()
    }

    /// Whether the node of `key` is the current item of its set (the current page).
    pub fn current_of(&self, key: NodeKey) -> bool {
        self.lookup(NodeId(key.0)).and_then(Node::aria_current).is_some_and(|current| current != accesskit::AriaCurrent::False)
    }

    /// The nearest ancestor of `id` that screen readers see: AccessKit's consumer drops generic containers.
    pub fn parent_of(&self, id: Id) -> Option<NodeKey> {
        self.parent_of_key(NodeKey::of(id))
    }

    /// [`AccessTree::parent_of`] for a node by key.
    pub fn parent_of_key(&self, key: NodeKey) -> Option<NodeKey> {
        let mut at = NodeId(key.0);
        loop {
            let (parent, _) = self.update.nodes.iter().find(|(_, node)| node.children().contains(&at))?;
            if self.lookup(*parent)?.role() != NodeRole::GenericContainer {
                return Some(NodeKey(parent.0));
            }
            at = *parent;
        }
    }
}

impl AccessTree {
    /// The root node, when the frame made a tree.
    pub(crate) fn root_key(&self) -> Option<NodeKey> {
        self.update.tree.as_ref().map(|tree| NodeKey(tree.root.0))
    }

    /// The node AccessKit reports as focused.
    pub(crate) fn focused_key(&self) -> NodeKey {
        NodeKey(self.update.focus.0)
    }

    /// The AccessKit data of a node by key.
    pub(crate) fn data_of(&self, key: NodeKey) -> Option<&Node> {
        self.lookup(NodeId(key.0))
    }

    /// Whether the node is, or lies inside, a component whose horizontal axis keeps its left-to-right order
    /// (`Props::ltr_axis`): it or an ancestor declares left-to-right text direction on a node that is not a text run,
    /// which only that declaration does.
    pub fn in_ltr_axis(&self, key: NodeKey) -> bool {
        let mut at = Some(key);
        while let Some(node) = at {
            if self
                .data_of(node)
                .is_some_and(|data| data.role() != accesskit::Role::TextRun && data.text_direction() == Some(accesskit::TextDirection::LeftToRight))
            {
                return true;
            }
            at = self.parent_of_key(node);
        }
        false
    }

    fn reported(&self) -> impl Iterator<Item = NodeKey> + '_ {
        self.update.nodes.iter().filter(|(key, node)| !node.is_hidden() && !self.below_hidden.contains(key)).map(|(key, _)| NodeKey(key.0))
    }

    /// Every node that assistive technology reports.
    pub fn keys(&self) -> Vec<NodeKey> {
        self.reported().collect()
    }

    /// The number of nodes that assistive technology reports.
    pub fn node_count(&self) -> usize {
        self.reported().count()
    }

    /// The role of a node by key.
    pub fn role_of(&self, key: NodeKey) -> Option<Role> {
        self.lookup(NodeId(key.0)).map(|node| Role::Node(node.role()))
    }

    /// Whether a node by key is reported as selected: `None` for a node that reports no selected state.
    pub fn selected_of(&self, key: NodeKey) -> Option<bool> {
        self.lookup(NodeId(key.0))?.is_selected()
    }

    /// The bounds of a node by key, in points.
    pub fn bounds_of(&self, key: NodeKey) -> Option<egui::Rect> {
        let bounds = self.lookup(NodeId(key.0))?.bounds()?;
        Some(egui::Rect::from_min_max(egui::pos2(bounds.x0 as f32, bounds.y0 as f32), egui::pos2(bounds.x1 as f32, bounds.y1 as f32)))
    }

    /// Whether a node by key is hidden from assistive technology.
    pub fn is_hidden(&self, key: NodeKey) -> bool {
        self.lookup(NodeId(key.0)).is_some_and(|node| node.is_hidden())
    }

    /// Whether a node by key is reported as disabled.
    pub fn is_disabled_key(&self, key: NodeKey) -> bool {
        self.lookup(NodeId(key.0)).is_some_and(Node::is_disabled)
    }

    /// Whether a node by key declares `action`.
    pub fn declares_key(&self, key: NodeKey, action: Declared) -> bool {
        self.lookup(NodeId(key.0)).is_some_and(|node| node.supports_action(action.action()))
    }

    /// The children of a node by key.
    pub fn children_of(&self, key: NodeKey) -> Vec<NodeKey> {
        self.lookup(NodeId(key.0)).map_or_else(Vec::new, |node| node.children().iter().map(|child| NodeKey(child.0)).collect())
    }
}
