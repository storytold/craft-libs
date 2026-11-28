//! The display order over the layer tree, reveal plan and toggle sweeps.

use craft_behavior::{ColumnKey, DisplayOrder, ItemSource, Selection};
use craft_keymap::Mods;
use craft_ui::contract::{BodyCx, MemoKey, MemoProps, OpenChange, Revision, SweepApply, Tracked};
use craft_ui::list::{ThumbTarget, tree_steps};
use std::borrow::Cow;
use std::sync::Arc;

use super::super::context::PanelContext;
use super::super::events::LayerColumn;
use super::super::row::{EYE, LOCK};
use super::super::{Groups, LayerRowLook, Session};
use crate::ui::filter::LayerFilter;
use crate::ui::tree::{LayerInfo, LayerPart, LayerRowKey, LayerTree, SubRowGroup};

/// One displayed row.
pub(in crate::ui::panel) struct DisplayRow<Id> {
    pub(in crate::ui::panel) key: LayerRowKey<Id>,
    depth: usize,
    /// The depth the row paints at: a filtered view lists rows at their shown depth and paints them at
    /// the true one.
    pub(in crate::ui::panel) paint_depth: usize,
    /// For an effect or Smart Filter row, the indent steps of its eye: one past its layer's for the header, two
    /// for an item, as Photoshop nests them under the layer's thumbnail.
    sub_steps: Option<usize>,
    pub(super) has_children: bool,
    pub(super) open: bool,
}

/// What the display order derives from: the tree's tracked revision, the filter and the groups a reveal opens. A
/// change of any of them builds the rows again; nothing else does.
#[derive(MemoProps)]
pub(in crate::ui::panel) struct RowsKey<'a, Id> {
    pub(in crate::ui::panel) tree: Revision,
    pub(in crate::ui::panel) filter: &'a LayerFilter,
    #[memo_props(hash = "tree ids are plain model ids (LayerTree::Id: PlainData); Hash covers them")]
    pub(in crate::ui::panel) opened: &'a [Id],
}

/// The display order of the tree, derived from a [`RowsKey`] and kept across passes.
pub(in crate::ui::panel) struct RowsModel<Id> {
    pub(in crate::ui::panel) rows: Vec<DisplayRow<Id>>,
    /// The row keys in display order.
    keys: Vec<LayerRowKey<Id>>,
    /// The filter left a layer out: rows list at their shown depth, so a drop finds its place by its neighbours.
    pub(in crate::ui::panel) filtered: bool,
    /// The number the list keeps its display order by: it follows the key.
    revision: u64,
}

impl<Id: Clone + PartialEq> RowsModel<Id> {
    /// The rows of `tree` for `filter`, with the groups in `opened` shown open besides the tree's own.
    fn walk<T: LayerTree<Id = Id>>(tree: &T, filter: &LayerFilter, opened: &[Id], revision: u64) -> Self {
        let active = filter.active();
        let mut walk =
            Walk { tree, filter, active, open_all: active && filter.lists_closed_contents(), holds: filter.keeps_holders(), opened, left_out: false };
        let mut rows = Vec::new();
        push_level(&mut walk, tree.children(None), Level { shown: 0, true_depth: 0 }, &mut rows);
        let keys = rows.iter().map(|row| row.key.clone()).collect();
        RowsModel { rows, keys, filtered: walk.left_out, revision }
    }
}

/// The rows of the tree as the list shows them: layers, and under each layer whose effects are open
/// the Smart Filter and effect rows.
pub(in crate::ui::panel) struct LayerRows<'t, T: LayerTree> {
    tree: &'t T,
    model: Arc<RowsModel<T::Id>>,
}

impl<'t, T: LayerTree> LayerRows<'t, T> {
    /// The rows for `filter` and `opened`, derived by the body `cx` from the tracked `tree`.
    pub(in crate::ui::panel) fn new(cx: &BodyCx<'_>, tree: &'t Tracked<T>, filter: &LayerFilter, opened: &[T::Id]) -> Self {
        let key = RowsKey { tree: tree.revision(), filter, opened };
        // The low half of the key's hash tells the list when the order changed.
        let revision = MemoKey::of(&key) as u64;
        let model = cx.derived(&key, || RowsModel::walk(&**tree, filter, opened, revision));
        LayerRows { tree: &**tree, model }
    }

    pub(in crate::ui::panel) fn rows(&self) -> &[DisplayRow<T::Id>] {
        &self.model.rows
    }

    pub(in crate::ui::panel) fn filtered(&self) -> bool {
        self.model.filtered
    }

    pub(in crate::ui::panel) fn keys(&self) -> &[LayerRowKey<T::Id>] {
        &self.model.keys
    }
}

impl<T: LayerTree> DisplayOrder<LayerRowKey<T::Id>> for LayerRows<'_, T> {
    fn len(&self) -> usize {
        self.model.rows.len()
    }

    fn key(&self, at: usize) -> LayerRowKey<T::Id> {
        self.model.rows[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.model.rows[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.model.rows[at].has_children
    }

    fn open(&self, at: usize) -> bool {
        self.model.rows[at].open
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match &self.model.rows[at].key {
            LayerRowKey::Layer(id) | LayerRowKey::Mask(id, _) => Some(Cow::Owned(self.tree.info(id).name)),
            LayerRowKey::Effects(id, index) => sub_label(self.tree, id, SubRowGroup::Effects, *index),
            LayerRowKey::Filters(id, index) => sub_label(self.tree, id, SubRowGroup::SmartFilters, *index),
        }
    }

    fn revision(&self) -> Option<u64> {
        Some(self.model.revision)
    }
}

impl<T: LayerTree> ItemSource<LayerRowKey<T::Id>> for LayerRows<'_, T> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        match &self.model.rows[at].key {
            LayerRowKey::Layer(id) if *column == EYE => self.tree.info(id).visible,
            LayerRowKey::Layer(id) if *column == LOCK => self.tree.info(id).locks.any(),
            LayerRowKey::Effects(id, index) if *column == EYE => sub_on(self.tree, id, SubRowGroup::Effects, *index),
            LayerRowKey::Filters(id, index) if *column == EYE => sub_on(self.tree, id, SubRowGroup::SmartFilters, *index),
            _ => false,
        }
    }

    fn sub_row(&self, at: usize) -> Option<usize> {
        self.model.rows[at].sub_steps
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
    DisplayRow { key, depth: level.shown, paint_depth: level.true_depth, sub_steps: None, has_children: false, open: false }
}

/// A walk of the tree for the display order.
struct Walk<'a, T: LayerTree> {
    tree: &'a T,
    filter: &'a LayerFilter,
    /// The filter is on: each layer is asked whether it is listed.
    active: bool,
    /// The filter lists the contents of closed groups.
    open_all: bool,
    /// A row that holds a listed row stays listed at its own depth ([`LayerFilter::keeps_holders`]).
    holds: bool,
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
        if !shown && walk.holds {
            // The holder comes before what it holds, so what it holds is walked into its own buffer first.
            let mut held = Vec::new();
            if open {
                push_level(walk, below, Level { shown: level.shown + 1, true_depth: level.true_depth + 1 }, &mut held);
            }
            if !held.is_empty() {
                let mut row = leaf(LayerRowKey::Layer(id), level);
                row.has_children = true;
                row.open = true;
                out.push(row);
                out.append(&mut held);
            }
            continue;
        }
        let at = out.len();
        if shown {
            let mut row = leaf(LayerRowKey::Layer(id.clone()), level);
            row.has_children = !below.is_empty();
            let row_has_children = row.has_children;
            row.open = row.has_children && open;
            out.push(row);
            let steps = tree_steps(level.true_depth, row_has_children);
            push_sub_rows(tree, &id, Level { shown: level.shown + 1, true_depth: level.true_depth + 1 }, steps, out);
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

fn push_sub_rows<T: LayerTree>(tree: &T, id: &T::Id, level: Level, steps: usize, out: &mut Vec<DisplayRow<T::Id>>) {
    if !tree.effects_open(id) {
        return;
    }
    let header = |key| DisplayRow { sub_steps: Some(steps + 1), ..leaf(key, level) };
    let item = |key| DisplayRow { sub_steps: Some(steps + 2), ..leaf(key, level) };
    if let Some(sub) = tree.sub_rows(id, SubRowGroup::SmartFilters) {
        out.push(header(LayerRowKey::Filters(id.clone(), None)));
        (0..sub.items.len()).for_each(|i| out.push(item(LayerRowKey::Filters(id.clone(), Some(i)))));
    }
    if let Some(sub) = tree.sub_rows(id, SubRowGroup::Effects) {
        out.push(header(LayerRowKey::Effects(id.clone(), None)));
        (0..sub.items.len()).for_each(|i| out.push(item(LayerRowKey::Effects(id.clone(), Some(i)))));
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

/// The reveal the app named: with [`Groups::Open`] the layer, past its closed ancestors opened; with [`Groups::Keep`]
/// its outermost closed ancestor, else the layer.
pub(in crate::ui::panel) fn reveal_plan<T: LayerTree>(tree: &T, named: Option<(T::Id, Groups)>) -> RevealPlan<T::Id> {
    let Some((id, groups)) = named else { return RevealPlan { target: None, opens: Vec::new() } };
    let opens = closed_ancestor_opens(tree, &id);
    match groups {
        Groups::Open => RevealPlan { target: Some(id), opens },
        Groups::Keep => RevealPlan { target: Some(opens.into_iter().next().map_or(id, |open| open.key)), opens: Vec::new() },
    }
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

/// The part the thumbnail `which` of the row `key` shows: a Smart Filters row's is the filter mask.
pub(super) fn thumb_part<Id>(key: &LayerRowKey<Id>, info: &LayerInfo, which: ThumbTarget) -> LayerPart {
    match which {
        ThumbTarget::First if matches!(key, LayerRowKey::Filters(_, None)) => LayerPart::FilterMask,
        which => info.thumb_shows(which).unwrap_or(LayerPart::Pixels),
    }
}

/// The readers of the eye and lock sweeps, kept across passes while the pointer is held.
#[derive(Clone)]
pub(in crate::ui::panel) struct ToggleSweeps {
    eye: SweepApply,
    lock: SweepApply,
}

impl ToggleSweeps {
    /// The readers for `look`. The eye shows only the pressed layer on Alt; the lock column ignores Alt. Under
    /// `ColorBar` the eye claims Cmd and Cmd+Alt and the lock claims Alt, for the host's Outline, Outline Others and
    /// Lock Others.
    pub(in crate::ui::panel) fn for_look(look: LayerRowLook) -> Self {
        match look {
            LayerRowLook::EyeLabel => ToggleSweeps { eye: SweepApply::show_only(), lock: SweepApply::new() },
            LayerRowLook::ColorBar => ToggleSweeps {
                eye: SweepApply::show_only().claims(Mods::CMD).claims(Mods { cmd: true, alt: true, ..Mods::NONE }),
                lock: SweepApply::new().claims(Mods::ALT),
            },
        }
    }

    /// The sweep reader of `column`.
    pub(super) fn of(&mut self, column: LayerColumn) -> &mut SweepApply {
        if column == LayerColumn::Visibility { &mut self.eye } else { &mut self.lock }
    }
}
