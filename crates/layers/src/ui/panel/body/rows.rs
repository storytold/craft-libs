//! The display order over the layer tree, reveal plan and toggle sweeps.

use craft_behavior::{ColumnKey, DisplayOrder, ItemSource, Selection};
use craft_keymap::Mods;
use craft_ui::contract::{OpenChange, SweepApply};
use craft_ui::list::ThumbTarget;
use std::borrow::Cow;

use super::super::Session;
use super::super::context::PanelContext;
use super::super::events::LayerColumn;
use super::super::row::{EYE, LOCK};
use crate::ui::filter::LayerFilter;
use crate::ui::tree::{LayerInfo, LayerKind, LayerPart, LayerRowKey, LayerTree, SubRowGroup};

/// One displayed row.
pub(in crate::ui::panel) struct DisplayRow<Id> {
    pub(in crate::ui::panel) key: LayerRowKey<Id>,
    depth: usize,
    /// The depth the row paints at: a filtered view lists rows at their shown depth and paints them at
    /// the true one.
    pub(in crate::ui::panel) paint_depth: usize,
    pub(super) has_children: bool,
    pub(super) open: bool,
}

/// The rows of the tree as the list shows them: layers, and under each layer whose effects are open
/// the Smart Filter and effect rows.
pub(in crate::ui::panel) struct LayerRows<'t, T: LayerTree> {
    tree: &'t T,
    pub(in crate::ui::panel) rows: Vec<DisplayRow<T::Id>>,
    /// The filter left a layer out: rows list at their shown depth, so a drop finds its place by its neighbours.
    pub(in crate::ui::panel) filtered: bool,
    revision: Option<u64>,
}

impl<'t, T: LayerTree> LayerRows<'t, T> {
    /// The rows for `filter`, with the groups in `opened` shown open besides the tree's own.
    pub(in crate::ui::panel) fn new(tree: &'t T, filter: &LayerFilter, opened: &[T::Id]) -> Self {
        let active = filter.active();
        let mut walk = Walk { tree, filter, active, open_all: active && filter.lists_closed_contents(), opened, left_out: false };
        let mut rows = Vec::new();
        push_level(&mut walk, tree.children(None), Level { shown: 0, true_depth: 0 }, &mut rows);
        let revision = tree.revision().filter(|_| !active && opened.is_empty());
        LayerRows { tree, rows, filtered: walk.left_out, revision }
    }

    pub(in crate::ui::panel) fn keys(&self) -> Vec<LayerRowKey<T::Id>> {
        self.rows.iter().map(|row| row.key.clone()).collect()
    }
}

impl<T: LayerTree> DisplayOrder<LayerRowKey<T::Id>> for LayerRows<'_, T> {
    fn len(&self) -> usize {
        self.rows.len()
    }

    fn key(&self, at: usize) -> LayerRowKey<T::Id> {
        self.rows[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.rows[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.rows[at].has_children
    }

    fn open(&self, at: usize) -> bool {
        self.rows[at].open
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.rows[at].key {
            LayerRowKey::Layer(id) | LayerRowKey::Mask(id, _) => Some(Cow::Owned(self.tree.info(id).name)),
            LayerRowKey::Effects(id, index) => sub_label(self.tree, id, SubRowGroup::Effects, *index),
            LayerRowKey::Filters(id, index) => sub_label(self.tree, id, SubRowGroup::SmartFilters, *index),
        }
    }

    fn revision(&self) -> Option<u64> {
        self.revision
    }
}

impl<T: LayerTree> ItemSource<LayerRowKey<T::Id>> for LayerRows<'_, T> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        match &self.rows[at].key {
            LayerRowKey::Layer(id) if *column == EYE => self.tree.info(id).visible,
            LayerRowKey::Layer(id) if *column == LOCK => self.tree.info(id).locks.any(),
            LayerRowKey::Effects(id, index) if *column == EYE => sub_on(self.tree, id, SubRowGroup::Effects, *index),
            LayerRowKey::Filters(id, index) if *column == EYE => sub_on(self.tree, id, SubRowGroup::SmartFilters, *index),
            _ => false,
        }
    }
}

fn sub_label<T: LayerTree>(tree: &T, id: &T::Id, group: SubRowGroup, index: Option<usize>) -> Option<Cow<'static, str>> {
    let rows = tree.sub_rows(id, group)?;
    let row = match index {
        None => rows.header,
        Some(at) => rows.items.into_iter().nth(at)?,
    };
    Some(Cow::Owned(row.label))
}

fn sub_on<T: LayerTree>(tree: &T, id: &T::Id, group: SubRowGroup, index: Option<usize>) -> bool {
    let Some(rows) = tree.sub_rows(id, group) else { return false };
    match index {
        None => rows.header.on,
        Some(at) => rows.items.get(at).is_some_and(|row| row.on),
    }
}

#[derive(Clone, Copy)]
struct Level {
    shown: usize,
    true_depth: usize,
}

fn leaf<Id>(key: LayerRowKey<Id>, level: Level) -> DisplayRow<Id> {
    DisplayRow { key, depth: level.shown, paint_depth: level.true_depth, has_children: false, open: false }
}

/// A walk of the tree for the display order.
struct Walk<'a, T: LayerTree> {
    tree: &'a T,
    filter: &'a LayerFilter,
    /// The filter is on: each layer is asked whether it is listed.
    active: bool,
    /// The filter lists the contents of closed groups.
    open_all: bool,
    opened: &'a [T::Id],
    /// A layer was not listed.
    left_out: bool,
}

fn push_level<T: LayerTree>(walk: &mut Walk<'_, T>, children: Vec<T::Id>, level: Level, out: &mut Vec<DisplayRow<T::Id>>) {
    let tree = walk.tree;
    for id in children {
        let below = tree.children(Some(&id));
        let shown = !walk.active || tree.listed(&id, walk.filter);
        walk.left_out |= !shown;
        let open = walk.open_all || (!below.is_empty() && (tree.is_open(&id) || walk.opened.contains(&id)));
        let at = out.len();
        if shown {
            let mut row = leaf(LayerRowKey::Layer(id.clone()), level);
            row.has_children = !below.is_empty();
            row.open = row.has_children && open;
            out.push(row);
            push_sub_rows(tree, &id, Level { shown: level.shown + 1, true_depth: level.true_depth + 1 }, out);
        }
        let after_sub_rows = out.len();
        if open {
            let inner = Level { shown: level.shown + usize::from(shown), true_depth: level.true_depth + 1 };
            push_level(walk, below, inner, out);
        }
        if shown && walk.open_all {
            // A group with no match inside has nothing to open.
            out[at].has_children = out.len() > after_sub_rows;
            out[at].open = out[at].has_children;
        }
    }
}

fn push_sub_rows<T: LayerTree>(tree: &T, id: &T::Id, level: Level, out: &mut Vec<DisplayRow<T::Id>>) {
    if !tree.effects_open(id) {
        return;
    }
    if let Some(sub) = tree.sub_rows(id, SubRowGroup::SmartFilters) {
        out.push(leaf(LayerRowKey::Filters(id.clone(), None), level));
        (0..sub.items.len()).for_each(|i| out.push(leaf(LayerRowKey::Filters(id.clone(), Some(i)), level)));
    }
    if let Some(sub) = tree.sub_rows(id, SubRowGroup::Effects) {
        out.push(leaf(LayerRowKey::Effects(id.clone(), None), level));
        (0..sub.items.len()).for_each(|i| out.push(leaf(LayerRowKey::Effects(id.clone(), Some(i)), level)));
    }
}

/// The closed ancestors of `id`, outermost first, as the opens that reveal it.
fn closed_ancestor_opens<T: LayerTree>(tree: &T, id: &T::Id) -> Vec<OpenChange<T::Id>> {
    let mut opens = Vec::new();
    let mut current = tree.parent(id);
    while let Some(parent) = current {
        if !tree.is_open(&parent) {
            opens.push(OpenChange::new(parent.clone(), true));
        }
        current = tree.parent(&parent);
    }
    opens.reverse();
    opens
}

/// What the pass reveals: the layer to scroll to and the groups to open on the way.
pub(in crate::ui::panel) struct RevealPlan<Id> {
    pub(in crate::ui::panel) target: Option<Id>,
    pub(in crate::ui::panel) opens: Vec<OpenChange<Id>>,
}

impl<Id: Clone> RevealPlan<Id> {
    pub(in crate::ui::panel) fn opened(&self) -> Vec<Id> {
        self.opens.iter().map(|open| open.key.clone()).collect()
    }
}

/// The reveal of the layer the app named, or of the active layer when it changed since the last pass.
pub(in crate::ui::panel) fn reveal_plan<T: LayerTree>(
    tree: &T,
    active: Option<&T::Id>,
    named: Option<T::Id>,
    session: &mut Session<T::Id>,
) -> RevealPlan<T::Id> {
    let previous = session.seen_active.replace(active.cloned());
    if let Some(id) = named {
        let opens = closed_ancestor_opens(tree, &id);
        return RevealPlan { target: Some(id), opens };
    }
    let changed = previous.as_ref().is_some_and(|previous| previous.as_ref() != active);
    let opens = match (active, changed) {
        (Some(active), true) => closed_ancestor_opens(tree, active),
        _ => Vec::new(),
    };
    RevealPlan { target: active.cloned().filter(|_| changed), opens }
}

/// The tree's selection as the list holds it: the selected layers and the sub-row selected on its own, that row or
/// else the active layer as the lead, and the anchor the last pick left while the selection is still what that pick
/// produced.
pub(super) fn list_selection<T: LayerTree>(cx: &PanelContext<'_, T>, session: &Session<T::Id>) -> Selection<LayerRowKey<T::Id>> {
    let mut items: Vec<LayerRowKey<T::Id>> = cx.selection.layers.iter().cloned().map(LayerRowKey::Layer).collect();
    items.extend(cx.selection.sub_row.clone());
    let lead = cx.selection.sub_row.clone().or_else(|| cx.selection.active.clone().map(LayerRowKey::Layer));
    let same = session.produced.len() == cx.selection.layers.len() && session.produced.iter().all(|id| cx.selection.layers.contains(id));
    let anchor = session.anchor.clone().filter(|_| same).or_else(|| lead.clone());
    Selection::new(items, lead, anchor)
}

/// Whether a thumbnail click acts on its own and does not select the row. A frame's thumbnails take the clicks a
/// layer thumbnail takes.
pub(in crate::ui::panel) fn thumb_click_replaces_select(part: LayerPart, mods: Mods, info: &LayerInfo) -> bool {
    let on_mask = matches!(part, LayerPart::LayerMask | LayerPart::VectorMask | LayerPart::FilterMask);
    if mods.cmd {
        return on_mask || info.thumb_loads_selection;
    }
    if on_mask { mods.shift || mods.alt } else { mods.alt && !mods.shift }
}

pub(super) fn thumb_part<Id>(key: &LayerRowKey<Id>, info: &LayerInfo, which: ThumbTarget) -> LayerPart {
    match which {
        ThumbTarget::First if matches!(key, LayerRowKey::Filters(_, None)) => LayerPart::FilterMask,
        ThumbTarget::First if info.kind == LayerKind::Frame => LayerPart::FrameShape,
        ThumbTarget::First => LayerPart::Pixels,
        ThumbTarget::Extra(i) => extra_parts(info).get(i).copied().unwrap_or(LayerPart::Pixels),
        _ => LayerPart::Pixels,
    }
}

/// The parts of a layer row's thumbnails after the first, in the order the row adds them: a frame's content, then the
/// masks.
pub(in crate::ui::panel) fn extra_parts(info: &LayerInfo) -> Vec<LayerPart> {
    let content = (info.kind == LayerKind::Frame).then_some(LayerPart::FrameContent);
    content.into_iter().chain(info.masks.iter().map(|mask| mask.part)).collect()
}

/// The readers of the eye and lock sweeps, kept across passes while the pointer is held.
#[derive(Clone)]
pub(in crate::ui::panel) struct ToggleSweeps {
    eye: SweepApply,
    lock: SweepApply,
}

impl Default for ToggleSweeps {
    /// The eye shows only the pressed layer on Alt; the lock column ignores Alt.
    fn default() -> Self {
        ToggleSweeps { eye: SweepApply::show_only(), lock: SweepApply::new() }
    }
}

impl ToggleSweeps {
    /// The sweep reader of `column`.
    pub(super) fn of(&mut self, column: LayerColumn) -> &mut SweepApply {
        if column == LayerColumn::Visibility { &mut self.eye } else { &mut self.lock }
    }
}
