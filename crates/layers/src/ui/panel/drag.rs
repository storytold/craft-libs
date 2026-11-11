//! Drag and drop: what a reorder of the list means for layers, effects and Smart Filters, the
//! payload that footer buttons accept, and the stacking commands.

use craft_behavior::{EditEvent, Mixed};
use craft_keymap::Mods;
use craft_ui::contract::{ListDrag, ReorderMove};
use std::sync::Arc;

use super::LayersPanelCommands;
use super::events::{LayerDrop, LayerMove, LayerPosition, LayersEvent, RowTransfer, SubRowMove};
use crate::ui::tree::{LayerKind, LayerRowKey, LayerTree, SubRowGroup};

/// What a drag from the rows carries for the drop targets outside the list: the footer buttons.
#[derive(Clone, Debug, PartialEq)]
pub(super) struct LayersDrag<K> {
    /// The list the drag started in.
    pub(super) list: egui::Id,
    /// The dragged rows, or the one mask thumbnail.
    pub(super) keys: Vec<LayerRowKey<K>>,
}

impl<K> LayersDrag<K> {
    pub(super) fn new(list: egui::Id, keys: Vec<LayerRowKey<K>>) -> Self {
        Self { list, keys }
    }

    /// The payload a row drag carries.
    pub(super) fn of_rows(drag: &ListDrag<LayerRowKey<K>>) -> Self
    where
        K: Clone,
    {
        Self { list: drag.list, keys: drag.keys.clone() }
    }
}

fn sub_row_group<K>(key: &LayerRowKey<K>) -> Option<SubRowGroup> {
    match key {
        LayerRowKey::Effects(..) => Some(SubRowGroup::Effects),
        LayerRowKey::Filters(..) => Some(SubRowGroup::SmartFilters),
        LayerRowKey::Layer(_) | LayerRowKey::Mask(..) => None,
    }
}

/// What a reorder of the displayed rows means. `order` is the display order the move was made in and
/// `parent_of` the tree's parent of a layer. A filtered view shows every layer at the top level, so
/// it matches the landing place by neighbours alone (`relaxed`).
pub(super) fn layer_drop<K: Clone + PartialEq>(
    mv: &ReorderMove<LayerRowKey<K>>,
    order: &[LayerRowKey<K>],
    parent_of: &dyn Fn(&K) -> Option<K>,
    relaxed: bool,
) -> Option<LayerDrop<K>> {
    if mv.keys.is_empty() {
        return None;
    }
    let remaining: Vec<&LayerRowKey<K>> = order.iter().filter(|key| !mv.keys.contains(key)).collect();
    let to = mv.to.min(remaining.len());
    if mv.keys.iter().all(LayerRowKey::is_layer) {
        let layers: Vec<K> = mv.keys.iter().map(|key| key.layer().clone()).collect();
        let parent = mv.parent.as_ref().map(|key| key.layer().clone());
        // A place beside a layer's effects or Smart Filter rows is below that layer. The list lays those rows under the
        // layer as its children, so the place has the layer as `parent` when it is above or between them.
        let sub_row_owner = [to.checked_sub(1).and_then(|at| remaining.get(at)), remaining.get(to)]
            .into_iter()
            .flatten()
            .find(|key| sub_row_group(key).is_some())
            .map(|key| key.layer());
        if let Some(owner) = sub_row_owner.filter(|owner| {
            (parent.as_ref() == Some(*owner) || parent == parent_of(owner)) && !mv.keys.contains(&LayerRowKey::Layer((*owner).clone()))
        }) {
            return Some(LayerDrop::Layers(LayerMove::new(layers, owner.clone(), LayerPosition::After, mv.copy)));
        }
        // A place is reported one way: into a group at its first slot, else before the next sibling, else after the
        // last sibling.
        if let Some(group) = parent.clone().filter(|group| to > 0 && matches!(remaining[to - 1], LayerRowKey::Layer(id) if id == group)) {
            return Some(LayerDrop::Layers(LayerMove::new(layers, group, LayerPosition::Into, mv.copy)));
        }
        let fits = |id: &K| relaxed || parent_of(id) == parent;
        let next = remaining.get(to).copied().and_then(|key| match key {
            LayerRowKey::Layer(id) if fits(id) => Some(id.clone()),
            _ => None,
        });
        let previous = remaining[..to].iter().rev().find_map(|key| match key {
            LayerRowKey::Layer(id) if fits(id) => Some(id.clone()),
            _ => None,
        });
        let (target, position) = match (next, previous, parent) {
            (Some(next), _, _) => (next, LayerPosition::Before),
            (None, Some(previous), _) => (previous, LayerPosition::After),
            (None, None, Some(parent)) => (parent, LayerPosition::Into),
            (None, None, None) => return None,
        };
        return Some(LayerDrop::Layers(LayerMove::new(layers, target, position, mv.copy)));
    }
    // Sub-rows: all of one layer and one group.
    let owner = mv.keys[0].layer();
    let group = sub_row_group(&mv.keys[0])?;
    if mv.keys.iter().any(|key| key.layer() != owner || sub_row_group(key) != Some(group)) {
        return None;
    }
    let landing = remaining[..to].iter().rev().find_map(|key| match key {
        LayerRowKey::Layer(id) => Some(id),
        _ => None,
    });
    if let Some(landing) = landing.filter(|landing| *landing != owner) {
        return Some(LayerDrop::Transfer(RowTransfer { rows: mv.keys.clone(), to: landing.clone(), copy: mv.copy }));
    }
    if group != SubRowGroup::SmartFilters {
        return None;
    }
    let [LayerRowKey::Filters(_, Some(from))] = mv.keys.as_slice() else { return None };
    let header = remaining.iter().position(|key| matches!(key, LayerRowKey::Filters(on, None) if on == owner))?;
    if to <= header {
        return None;
    }
    let slot = remaining[..to].iter().filter(|key| matches!(key, LayerRowKey::Filters(on, Some(_)) if on == owner)).count();
    (slot != *from || mv.copy).then(|| LayerDrop::SubRows(SubRowMove { layer: owner.clone(), group, from: *from, to: slot, copy: mv.copy }))
}

/// Whether the dragged rows may land before position `insert` of the display order, with the
/// modifiers held during the hover (Alt copies, as the list's reorder rule does).
pub(super) fn accepts_drop<T: LayerTree>(
    tree: &T,
    drag: &ListDrag<LayerRowKey<T::Id>>,
    order: &[LayerRowKey<T::Id>],
    insert: usize,
    mods: Mods,
) -> bool {
    let moved_before = order[..insert.min(order.len())].iter().filter(|key| drag.keys.contains(key)).count();
    let mv = ReorderMove::new(drag.grabbed.clone(), drag.keys.clone(), insert.saturating_sub(moved_before), mods.alt);
    let parent_of = |id: &T::Id| tree.parent(id);
    match layer_drop(&mv, order, &parent_of, true) {
        Some(LayerDrop::Layers(layers)) => {
            let parent = match layers.position {
                LayerPosition::Into => Some(layers.target.clone()),
                LayerPosition::Before | LayerPosition::After => tree.parent(&layers.target),
            };
            tree.accepts_drop(&mv.in_group(parent.map(LayerRowKey::Layer)), mods)
        }
        // Smart Filters go only onto a smart object; Photoshop takes no others.
        Some(LayerDrop::Transfer(transfer)) => {
            transfer.rows.iter().all(|row| !matches!(row, LayerRowKey::Filters(..))) || tree.info(&transfer.to).kind == LayerKind::SmartObject
        }
        Some(LayerDrop::SubRows(_)) => true,
        None => false,
    }
}

/// The reorder event of the list as the panel reports it: one `Set` per drop.
pub(super) fn drop_event<K: Clone + PartialEq>(
    edit: EditEvent<ReorderMove<LayerRowKey<K>>>,
    order: &[LayerRowKey<K>],
    parent_of: &dyn Fn(&K) -> Option<K>,
    relaxed: bool,
) -> Option<LayersEvent<K>> {
    match edit {
        EditEvent::Set { after, label, .. } => {
            let drop = layer_drop(&after, order, parent_of, relaxed)?;
            Some(LayersEvent::Drop(EditEvent::set(Mixed::Mixed, drop, label, None)))
        }
        _ => None,
    }
}

/// The drop a stacking command makes of the selected layers, `None` when they cannot move that way
/// or do not share a parent. `label` names the undo step.
pub(super) fn stacking_event<T: LayerTree>(tree: &T, command: LayersPanelCommands, label: Arc<str>) -> Option<LayersEvent<T::Id>> {
    let selected = tree.selection().layers;
    let first = selected.first()?;
    let parent = tree.parent(first);
    if selected.iter().any(|id| tree.parent(id) != parent) {
        return None;
    }
    let siblings = tree.children(parent.as_ref());
    let places: Vec<usize> = siblings.iter().enumerate().filter(|(_, id)| selected.contains(id)).map(|(at, _)| at).collect();
    let (lowest, highest) = (*places.first()?, *places.last()?);
    let block = places.len();
    let (target, position) = match command {
        LayersPanelCommands::MoveUp if lowest > 0 => (siblings[lowest - 1].clone(), LayerPosition::Before),
        LayersPanelCommands::MoveDown if highest + 1 < siblings.len() => (siblings[highest + 1].clone(), LayerPosition::After),
        LayersPanelCommands::MoveToTop if !(lowest == 0 && highest + 1 == block) => (siblings[0].clone(), LayerPosition::Before),
        LayersPanelCommands::MoveToBottom if !(highest + 1 == siblings.len() && lowest + block == siblings.len()) => {
            (siblings[siblings.len() - 1].clone(), LayerPosition::After)
        }
        _ => return None,
    };
    let layers: Vec<T::Id> = places.iter().map(|&at| siblings[at].clone()).collect();
    if layers.contains(&target) {
        return None;
    }
    let drop = LayerDrop::Layers(LayerMove::new(layers, target, position, false));
    Some(LayersEvent::Drop(EditEvent::set(Mixed::Mixed, drop, label, None)))
}
