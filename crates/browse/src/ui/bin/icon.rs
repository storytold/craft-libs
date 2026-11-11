//! The Icon view: tiles with hover scrub and In and Out marks, and the tile builder the Freeform view shares.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

use craft_behavior::{DisplayOrder, ItemSource};
use craft_i18n::Collator;
use craft_ui::contract::{CollectionEvent, ColumnSort, ListDrag, Props as _, ReorderRule, SelectMode, ShowCollection as _, request_rename};
use craft_ui::list::ThumbAspect;
use craft_ui::primitives::ContentPainterCx;
use craft_ui::primitives::textures::{Priority, TextureKey, ThumbPaint, ThumbRequest, ThumbSource, Thumbnails};
use craft_ui::tile_grid::{CaptionPlace, Tile, TileGrid, TilePartEvent};
use egui::{Rect, Ui};

use super::list::{drop_items, inside_any, item_of, open_event, rename_event, reorder_event, sort_entries, stack_event};
use super::{BinEntry, BinEvent, BinModel, BinState, ThumbPair, ViewIn, ViewOut};

/// The tiles' picture is 16:9.
pub(super) const ASPECT: f32 = 16.0 / 9.0;

/// Paints the picture of `key` into `rect` from the app's thumbnails.
pub(super) fn paint_thumb<K: TextureKey>(cx: &ContentPainterCx<'_>, rect: Rect, thumbs: &Thumbnails<K>, source: &dyn ThumbSource<K>, key: &K) {
    let ctx = cx.painter.ctx().clone();
    let request = ThumbRequest::for_cell(rect.width().max(rect.height()), ctx.pixels_per_point());
    let state = thumbs.state(&ctx, source, key, &request, Priority::VISIBLE);
    thumbs.paint(cx, rect, &state, &ThumbPaint::default().checker(false));
}

/// What the shared tile builder reads.
pub(super) struct TileFacts<'a, K: TextureKey, M: ?Sized> {
    pub model: &'a M,
    pub thumbnails: Option<ThumbPair<'a, K>>,
    pub hover_scrub: bool,
    pub scrub: Option<&'a (K, f32)>,
}

/// The tile of `key`: folder for a bin; the picture, duration, label, badges, status and In and Out bar for an
/// item. The Icon and Freeform views build their tiles with it.
pub(super) fn tile_of<'a, K, M>(facts: &TileFacts<'a, K, M>, key: &K) -> Tile<'a>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    let item = item_of(facts.model, key);
    let mut tile = Tile::new(item.name.clone()).status(item.status.clone());
    if let Some(color) = item.label_color {
        tile = tile.label_color(color);
    }
    for badge in item.badges {
        tile = tile.badge(badge);
    }
    if facts.model.is_bin(key) {
        return tile.folder();
    }
    if let Some(duration) = item.duration {
        tile = tile.detail(duration);
    }
    if let Some(range) = item.range {
        tile = tile.range(range);
    }
    if let Some((scrubbed, fraction)) = facts.scrub
        && facts.hover_scrub
        && scrubbed == key
    {
        tile = tile.scrub(*fraction);
    }
    match (facts.thumbnails, item.icon) {
        (Some((thumbs, source)), _) => {
            let key = key.clone();
            tile.thumb(ThumbAspect::new(16, 9), move |cx, rect| paint_thumb(cx, rect, thumbs, source, &key))
        }
        (None, Some(icon)) => tile.icon(icon),
        (None, None) => tile,
    }
}

/// The tiles of the shown bin in display order: bins before items, in the model's order or the Sort Icons order;
/// while searching, the matching items of every sub-bin, flat. The flag is whether the order is the model's own
/// ("User Order", the only one that accepts a reorder).
pub(super) fn entries_of<K, M>(model: &M, state: &BinState<K>, collator: &Collator) -> (Vec<BinEntry<K>>, bool)
where
    K: Clone + Eq + Hash,
    M: BinModel<K> + ?Sized,
{
    let sort = state.settings.icon_sort.as_ref();
    let searching = !state.search.trim().is_empty();
    let mut found: Vec<BinEntry<K>> = Vec::new();
    if searching {
        let mut keys = Vec::new();
        super::search::collect_matches(model, state.shown.as_ref(), &state.search, 0, &mut keys);
        found.extend(keys.into_iter().map(BinEntry::Item));
    } else {
        model.children(state.shown.as_ref(), &mut found);
    }
    (sort_entries(model, sort, collator, found), sort.is_none() && !searching)
}

/// The tiles as a source.
pub(super) struct Cards<K> {
    pub keys: Vec<K>,
    pub names: Vec<String>,
}

impl<K: Clone> Cards<K> {
    pub fn new<M: BinModel<K> + ?Sized>(model: &M, entries: Vec<BinEntry<K>>) -> Self {
        let keys: Vec<K> = entries.into_iter().map(|(BinEntry::Bin(key) | BinEntry::Item(key))| key).collect();
        let names = keys.iter().map(|key| item_of(model, key).name).collect();
        Self { keys, names }
    }
}

impl<K: Clone> DisplayOrder<K> for Cards<K> {
    fn len(&self) -> usize {
        self.keys.len()
    }

    fn key(&self, at: usize) -> K {
        self.keys[at].clone()
    }

    fn text(&self, at: usize) -> Option<std::borrow::Cow<'_, str>> {
        Some(std::borrow::Cow::Borrowed(self.names[at].as_str()))
    }
}

impl<K: Clone> ItemSource<K> for Cards<K> {}

/// The tiles as a stamp: another set of tiles starts the lead over.
pub(super) fn stamp_of<K: Hash>(shown: Option<&K>, search: &str, sort: Option<&ColumnSort>) -> u64 {
    let mut hasher = DefaultHasher::new();
    shown.hash(&mut hasher);
    search.hash(&mut hasher);
    sort.map(|sort| (&sort.key, sort.descending)).hash(&mut hasher);
    hasher.finish()
}

/// Draws the Icon view: a `TileGrid` of 16:9 tiles over [`entries_of`], bins as folder tiles, hover scrub on
/// items, In and Out bars, and its events as `BinEvent`s.
///
/// - Enter and a double click open a bin or activate an item (as the List view).
/// - A drag of the bin's own tiles reorders in User Order only; a drag from another list onto a bin tile is a
///   `Drop` into it.
/// - F2 and `MediaBin::start_rename` open the caption's rename field (`Rename` on commit).
pub(super) fn icon_view<K, M>(ui: &mut Ui, input: ViewIn<'_, K, M>) -> ViewOut<K>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    let ViewIn { id, model, state, selection, held, thumbnails, words, collator, rename, hover_scrub, scrub, stack, .. } = input;
    let (entries, user_order) = entries_of(model, state, collator);
    let searching = !state.search.trim().is_empty();
    let cards = Cards::new(model, entries);
    let mods = craft_ui::runtime::pointer::frame(ui.ctx()).mods;
    if let Some(key) = rename {
        request_rename(ui.ctx(), id, &key);
    }
    let accepts = |drag: &ListDrag<K>, over: Option<&K>, _insert: usize| {
        if drag.list == id {
            return user_order;
        }
        // Over an empty bin the drop goes into the bin shown.
        over.is_none_or(|over| model.is_bin(over) && !inside_any(model, over, &drag.keys))
    };
    let facts = TileFacts { model, thumbnails, hover_scrub, scrub: scrub.as_ref() };
    let empty = if searching { words.no_matches.clone() } else { words.empty.clone() };
    let grid = TileGrid::<K>::new(words.icon_view.clone())
        .id(id)
        .select(SelectMode::Multi)
        .selection(selection)
        .caption(CaptionPlace::Below)
        .detail_line()
        .aspect(ASPECT)
        .zoom(state.settings.zoom as f32)
        .rename(true)
        .deletable(true)
        .rows_stamp(stamp_of(state.shown.as_ref(), &state.search, state.settings.icon_sort.as_ref()))
        .empty(empty)
        .reorder(ReorderRule::new())
        .accepts(accepts);
    let out = grid.show_items(ui, &cards, &mut |key, _| tile_of(&facts, key));

    let mut events: Vec<BinEvent<K>> = Vec::new();
    if let Some(drag) = craft_ui::runtime::drag::active::<ListDrag<K>>(ui.ctx()).filter(|drag| drag.list == id) {
        events.push(BinEvent::Drag((*drag).clone()));
    }
    for event in out.response.events {
        match event {
            CollectionEvent::Selection(change) => events.push(BinEvent::Selection(change)),
            CollectionEvent::Activate(key) => events.push(open_event(model, key, mods.cmd, mods.shift || mods.alt)),
            CollectionEvent::ContextMenu(request) => events.push(BinEvent::ContextMenu(request)),
            CollectionEvent::Delete(keys) => events.push(BinEvent::Remove(keys)),
            CollectionEvent::Reorder(edit) => events.extend(reorder_event(model, state.shown.as_ref(), searching, user_order, edit)),
            CollectionEvent::Dropped { key, keys, .. } => {
                let into = key.or_else(|| state.shown.clone());
                if let Some(items) = drop_items(model, into.as_ref(), &keys) {
                    events.push(BinEvent::Drop { items, into });
                }
            }
            CollectionEvent::Rename(edit) => {
                events.extend(rename_event(&edit, |key| cards.keys.iter().position(|held| held == key).map(|at| cards.names[at].as_str())));
            }
            CollectionEvent::Item(key, TilePartEvent::Scrub(fraction)) if hover_scrub && !model.is_bin(&key) => {
                events.push(BinEvent::Scrub { key, fraction });
            }
            _ => {}
        }
    }
    if let Some(forward) = stack.filter(|_| user_order) {
        events.extend(stack_event(&cards.keys, &held.items, forward, None));
    }
    ViewOut { response: out.response.response, events, moving: None }
}
