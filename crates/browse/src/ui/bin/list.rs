//! The List view: a tree `Table` whose first column holds the bins.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use craft_behavior::{ColumnKey, ColumnWidth, DisplayOrder, EditEvent, GesturePhase, IconId, ItemSource, Mixed};
use craft_i18n::Collator;
use craft_theme::variant::Tone;
use craft_ui::contract::{
    CollectionEvent, ColumnSort, ColumnSpec, ListDrag, OpenChange, Props as _, Rename, ReorderMove, ReorderRule, SelectMode, ShowCollection as _,
    request_rename,
};
use craft_ui::list::{ListRow, ThumbAspect, ThumbSize};
use craft_ui::primitives::IconSpec;
use craft_ui::primitives::textures::TextureKey;
use craft_ui::runtime::env::Label;
use craft_ui::table::Table;
use craft_ui::tile_grid::{BadgeKind, TileStatus};
use egui::Ui;

use super::icon::paint_thumb;
use super::{BinEntry, BinEvent, BinItem, BinModel, BinState, MAX_DEPTH, OpenHow, ViewIn, ViewOut};

/// The key of the name column, the first of the host's columns.
pub(super) const NAME: ColumnKey = ColumnKey::new("name");

/// One row of the table.
struct Row<K> {
    key: K,
    depth: usize,
    group: bool,
    open: bool,
    name: String,
}

/// The rows in display order.
struct Rows<K>(Vec<Row<K>>);

impl<K: Clone> DisplayOrder<K> for Rows<K> {
    fn len(&self) -> usize {
        self.0.len()
    }

    fn key(&self, at: usize) -> K {
        self.0[at].key.clone()
    }

    fn depth(&self, at: usize) -> usize {
        self.0[at].depth
    }

    fn has_children(&self, at: usize) -> bool {
        self.0[at].group
    }

    fn open(&self, at: usize) -> bool {
        self.0[at].open
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.0[at].name.as_str()))
    }
}

impl<K: Clone> ItemSource<K> for Rows<K> {}

/// The columns with the widths the user dragged them to; a bin given no columns has the name column alone.
fn columns_of(columns: &[ColumnSpec], widths: &[(ColumnKey, f32)], name: Label) -> Vec<ColumnSpec> {
    let mut specs: Vec<ColumnSpec> =
        if columns.is_empty() { vec![ColumnSpec::new(NAME, name, ColumnWidth::fill(1.0)).sortable().resizable()] } else { columns.to_vec() };
    for spec in &mut specs {
        if let Some((_, width)) = widths.iter().find(|(key, _)| *key == spec.key) {
            spec.width = ColumnWidth::Fixed(*width);
        }
    }
    specs
}

/// The rows of a pass as a stamp: another set of rows starts the lead over.
fn stamp_of<K: Hash>(shown: Option<&K>, search: &str, sort: Option<&ColumnSort>) -> u64 {
    let mut hasher = DefaultHasher::new();
    shown.hash(&mut hasher);
    search.hash(&mut hasher);
    sort.map(|sort| (&sort.key, sort.descending)).hash(&mut hasher);
    hasher.finish()
}

/// Draws the List view: a `Table` over the rows in display order (bins before items, sorted by the
/// header's sort, open bins' children under them, a group for every bin). The table owns selection,
/// the double click, the context-menu request, the row drag and the rename; this view maps its events to
/// `BinEvent`s and turns a drag onto a bin row, from this table or another, into `Drop`.
pub(super) fn list_view<K, M>(ui: &mut Ui, input: ViewIn<'_, K, M>) -> ViewOut<K>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    let ViewIn { id, model, state, selection, held, thumbnails, columns, words, collator, rename, stack, .. } = input;
    let searching = !state.search.trim().is_empty();
    let sort = state.settings.sort.clone();
    let user_order = sort.is_none() && !searching;
    let rows = Rows(build_rows(model, state, sort.as_ref(), collator));
    let thumb = state.settings.thumbnails;
    let specs = columns_of(columns, &state.settings.widths, words.name.clone());
    let value_columns: Vec<ColumnKey> = specs.iter().map(|spec| spec.key.clone()).filter(|key| *key != NAME).collect();
    let mods = craft_ui::runtime::pointer::frame(ui.ctx()).mods;
    if let Some(key) = rename {
        request_rename(ui.ctx(), id, &key);
    }

    let accepts = |drag: &ListDrag<K>, over: Option<&K>, _insert: usize| {
        if drag.list == id && user_order {
            return true;
        }
        // Over an empty bin the drop goes into the bin shown.
        over.is_none_or(|over| model.is_bin(over) && !inside_any(model, over, &drag.keys))
    };
    let stamp = stamp_of(state.shown.as_ref(), &state.search, sort.as_ref());
    let empty = if searching { words.no_matches.clone() } else { words.empty.clone() };
    let table = Table::<K>::new(words.list_view.clone(), specs)
        .id(id)
        .tree(true)
        .select(SelectMode::Multi)
        .selection(selection)
        .sort(sort)
        .rename(true)
        .deletable(true)
        .striped(true)
        .rows_stamp(stamp)
        .empty(empty)
        .reorder(ReorderRule::new().into_groups())
        .accepts(accepts);
    let out = table.show_items(ui, &rows, &mut |key, icx| {
        let row = &rows.0[icx.at];
        let item = item_of(model, key);
        let mut line = ListRow::new(row.name.clone()).cells(icx.cells()).striped(icx.row.striped).depth(row.depth);
        if row.group {
            line = line.group(icx.row.open);
        }
        for column in &value_columns {
            line = line.cell(column.clone(), model.cell(key, column)).cell_muted(column.clone());
        }
        match thumbnails {
            Some((thumbs, source)) if !row.group && thumb != ThumbSize::None => {
                let key = key.clone();
                line = line.thumb_aspect(thumb, ThumbAspect::new(16, 9), move |cx, rect| paint_thumb(cx, rect, thumbs, source, &key));
            }
            _ => {
                let glyph = if row.group { Some(IconId::new("folder")) } else { item.icon.clone() };
                if let Some(glyph) = glyph {
                    line = line.icon(IconSpec::from(glyph));
                }
            }
        }
        for badge in &item.badges {
            if let BadgeKind::Text(text) = &badge.kind {
                line = line.trailing(text.clone());
            }
        }
        if !matches!(item.status, TileStatus::Normal) {
            line = line.tone(Tone::Bad);
        }
        line
    });

    let mut events: Vec<BinEvent<K>> = Vec::new();
    if let Some(drag) = craft_ui::runtime::drag::active::<ListDrag<K>>(ui.ctx()).filter(|drag| drag.list == id) {
        events.push(BinEvent::Drag((*drag).clone()));
    }
    for event in out.events {
        match event {
            CollectionEvent::Selection(change) => events.push(BinEvent::Selection(change)),
            CollectionEvent::Activate(key) => events.push(open_event(model, key, mods.cmd, mods.shift || mods.alt)),
            CollectionEvent::ContextMenu(request) => events.push(BinEvent::ContextMenu(request)),
            CollectionEvent::Delete(keys) => events.push(BinEvent::Remove(keys)),
            CollectionEvent::Open(change) if change.all => {
                let mut bins = Vec::new();
                collect_bins(model, state.shown.as_ref(), 0, &mut bins);
                events.extend(bins.into_iter().map(|key| BinEvent::Open(OpenChange::new(key, change.open))));
            }
            CollectionEvent::Open(change) => events.push(BinEvent::Open(change)),
            CollectionEvent::Reorder(edit) => events.extend(reorder_event(model, state.shown.as_ref(), searching, user_order, edit)),
            CollectionEvent::Dropped { key, keys, .. } => {
                let into = key.or_else(|| state.shown.clone());
                if let Some(items) = drop_items(model, into.as_ref(), &keys) {
                    events.push(BinEvent::Drop { items, into });
                }
            }
            CollectionEvent::Rename(edit) => {
                events.extend(rename_event(&edit, |key| rows.0.iter().find(|row| row.key == *key).map(|row| row.name.as_str())));
            }
            CollectionEvent::Column(change) => events.push(BinEvent::Column(change)),
            _ => {}
        }
    }
    if let Some(forward) = stack.filter(|_| user_order) {
        let order: Vec<K> = rows.0.iter().map(|row| row.key.clone()).collect();
        let first = order.iter().find(|key| held.items.contains(key));
        let parent = first.and_then(|key| model.parent(key)).filter(|parent| Some(parent) != state.shown.as_ref());
        events.extend(stack_event(&order, &held.items, forward, parent));
    }
    ViewOut { response: out.response, events, moving: None }
}

/// The bin event a reorder of the table or the grid means. A move into another bin, or out to the shown bin, is
/// a drop; a move inside one bin is a reorder, in User Order only.
pub(super) fn reorder_event<K, M>(
    model: &M,
    shown: Option<&K>,
    searching: bool,
    user_order: bool,
    edit: EditEvent<ReorderMove<K>>,
) -> Option<BinEvent<K>>
where
    K: Clone + PartialEq,
    M: BinModel<K> + ?Sized,
{
    let Some(mv) = edit.value() else {
        return user_order.then_some(BinEvent::Reorder(edit));
    };
    let held = model.parent(&mv.key).filter(|parent| Some(parent) != shown);
    if !searching && held != mv.parent {
        let into = mv.parent.clone();
        let items = drop_items(model, into.as_ref().or(shown), &mv.keys)?;
        return Some(BinEvent::Drop { items, into });
    }
    user_order.then_some(BinEvent::Reorder(edit))
}

/// The rename a committed edit means: non-empty text other than the name now.
pub(super) fn rename_event<'n, K: Clone>(edit: &EditEvent<Rename<K>>, name_of: impl Fn(&K) -> Option<&'n str>) -> Option<BinEvent<K>> {
    let ended = matches!(edit.phase(), Some(GesturePhase::Commit) | None);
    let rename = edit.value().filter(|_| ended)?;
    let text = rename.name.trim();
    let changed = name_of(&rename.key).is_some_and(|name| name != text);
    (changed && !text.is_empty()).then(|| BinEvent::Rename { key: rename.key.clone(), text: text.to_owned() })
}

/// The move that Bring Forward or Send Backward means for the `selected` keys among `order`: one place later or
/// earlier, the selected rows together.
pub(super) fn stack_event<K: Clone + PartialEq>(order: &[K], selected: &[K], forward: bool, parent: Option<K>) -> Option<BinEvent<K>> {
    let moving: Vec<K> = order.iter().filter(|key| selected.contains(key)).cloned().collect();
    let first = order.iter().position(|key| moving.first() == Some(key))?;
    let from = order[..first].iter().filter(|key| !moving.contains(key)).count();
    let remaining = order.len() - moving.len();
    let to = if forward { (from + 1).min(remaining) } else { from.saturating_sub(1) };
    if to == from {
        return None;
    }
    let key = moving[0].clone();
    let before = ReorderMove::new(key.clone(), moving.clone(), from, false).in_group(parent.clone());
    let after = ReorderMove::new(key, moving, to, false).in_group(parent);
    Some(BinEvent::Reorder(EditEvent::set(Mixed::Same(before), after, Arc::from("mediaBin.stack"), None)))
}

/// The `items` a drop into the bin `into` (`None`: the root) moves: those the model holds elsewhere. `None` when
/// there are none or `into` is, or lies inside, one of `items`: no empty undo step and no bin inside itself. The
/// background of the shown bin passes the shown bin as `into`.
pub(super) fn drop_items<K: Clone + PartialEq, M: BinModel<K> + ?Sized>(model: &M, into: Option<&K>, items: &[K]) -> Option<Vec<K>> {
    if into.is_some_and(|bin| inside_any(model, bin, items)) {
        return None;
    }
    let moved: Vec<K> = items.iter().filter(|key| model.parent(key).as_ref() != into).cloned().collect();
    (!moved.is_empty()).then_some(moved)
}

/// Whether `key` is `shown` or lies inside it; every key is inside the root.
pub(super) fn within<K: Clone + PartialEq, M: BinModel<K> + ?Sized>(model: &M, key: &K, shown: Option<&K>) -> bool {
    let Some(shown) = shown else { return true };
    let mut at = model.parent(key);
    for _ in 0..=MAX_DEPTH {
        let Some(parent) = at else { return false };
        if parent == *shown {
            return true;
        }
        at = model.parent(&parent);
    }
    false
}

/// What an Enter or a double click on `key` opens, and how: plain in place, Shift or Alt in a new tab,
/// Cmd/Ctrl in a new window.
pub(super) fn open_event<K, M: BinModel<K> + ?Sized>(model: &M, key: K, command: bool, tab: bool) -> BinEvent<K> {
    if model.is_bin(&key) {
        let how = if command {
            OpenHow::NewWindow
        } else if tab {
            OpenHow::NewTab
        } else {
            OpenHow::InPlace
        };
        BinEvent::OpenBin { key, how }
    } else {
        BinEvent::Activate(key)
    }
}

pub(super) fn item_of<K, M: BinModel<K> + ?Sized>(model: &M, key: &K) -> BinItem {
    let mut item = BinItem::default();
    model.item(key, &mut item);
    item
}

/// The rows in display order: the shown bin's children, bins before items, each open bin's children under it;
/// or, while searching, the matching items of every sub-bin.
fn build_rows<K, M>(model: &M, state: &BinState<K>, sort: Option<&ColumnSort>, collator: &Collator) -> Vec<Row<K>>
where
    K: Clone + Eq + Hash,
    M: BinModel<K> + ?Sized,
{
    let mut rows = Vec::new();
    if state.search.trim().is_empty() {
        push_children(model, state, state.shown.as_ref(), sort, collator, 0, &mut rows);
    } else {
        let mut found = Vec::new();
        super::search::collect_matches(model, state.shown.as_ref(), &state.search, 0, &mut found);
        let entries = sort_entries(model, sort, collator, found.into_iter().map(BinEntry::Item).collect());
        for entry in entries {
            let (BinEntry::Item(key) | BinEntry::Bin(key)) = entry;
            let name = item_of(model, &key).name;
            rows.push(Row { key, depth: 0, group: false, open: false, name });
        }
    }
    rows
}

fn push_children<K, M>(
    model: &M,
    state: &BinState<K>,
    parent: Option<&K>,
    sort: Option<&ColumnSort>,
    collator: &Collator,
    depth: usize,
    rows: &mut Vec<Row<K>>,
) where
    K: Clone + Eq + Hash,
    M: BinModel<K> + ?Sized,
{
    let mut children = Vec::new();
    model.children(parent, &mut children);
    for entry in sort_entries(model, sort, collator, children) {
        match entry {
            BinEntry::Bin(key) => {
                let open = state.expanded.is_open(&key);
                let name = item_of(model, &key).name;
                // Every bin is a group, so an empty one has its triangle and takes drops into it.
                rows.push(Row { key: key.clone(), depth, group: true, open, name });
                if open && depth < MAX_DEPTH {
                    push_children(model, state, Some(&key), sort, collator, depth + 1, rows);
                }
            }
            BinEntry::Item(key) => {
                let name = item_of(model, &key).name;
                rows.push(Row { key, depth, group: false, open: false, name });
            }
        }
    }
}

/// `entries` with bins before items, each kind in the model's order or sorted by `sort` in the interface
/// language's order.
pub(super) fn sort_entries<K: Clone, M: BinModel<K> + ?Sized>(
    model: &M,
    sort: Option<&ColumnSort>,
    collator: &Collator,
    entries: Vec<BinEntry<K>>,
) -> Vec<BinEntry<K>> {
    let (mut bins, mut items): (Vec<_>, Vec<_>) = entries.into_iter().partition(|entry| matches!(entry, BinEntry::Bin(_)));
    if let Some(sort) = sort {
        let text = |entry: &BinEntry<K>| {
            let (BinEntry::Bin(key) | BinEntry::Item(key)) = entry;
            if sort.key == NAME { item_of(model, key).name } else { model.cell(key, &sort.key) }
        };
        for group in [&mut bins, &mut items] {
            let mut keyed: Vec<(String, BinEntry<K>)> = group.drain(..).map(|entry| (text(&entry), entry)).collect();
            keyed.sort_by(|a, b| {
                let order = collator.compare(&a.0, &b.0);
                if sort.descending { order.reverse() } else { order }
            });
            group.extend(keyed.into_iter().map(|(_, entry)| entry));
        }
    }
    bins.append(&mut items);
    bins
}

/// Every bin under `parent`, sub-bins included.
fn collect_bins<K: Clone, M: BinModel<K> + ?Sized>(model: &M, parent: Option<&K>, depth: usize, out: &mut Vec<K>) {
    if depth > MAX_DEPTH {
        return;
    }
    let mut children = Vec::new();
    model.children(parent, &mut children);
    for child in children {
        if let BinEntry::Bin(key) = child {
            out.push(key.clone());
            collect_bins(model, Some(&key), depth + 1, out);
        }
    }
}

/// Whether `bin` is one of `items` or inside one of them.
pub(super) fn inside_any<K: Clone + PartialEq, M: BinModel<K> + ?Sized>(model: &M, bin: &K, items: &[K]) -> bool {
    let mut at = Some(bin.clone());
    for _ in 0..=MAX_DEPTH {
        let Some(key) = at else { return false };
        if items.contains(&key) {
            return true;
        }
        at = model.parent(&key);
    }
    false
}
