//! The Freeform view: the cards at the host's positions, shown by `FreeBin`.

use craft_ui::contract::{CollectionEvent, ListDrag, Props as _, SelectMode, ShowCollection as _, request_rename};
use craft_ui::primitives::textures::TextureKey;
use craft_ui::tile_grid::TilePartEvent;
use egui::Ui;

use super::icon::{ASPECT, Cards, TileFacts, entries_of, stamp_of, tile_of};
use super::list::{drop_items, open_event, rename_event};
use super::{BinEvent, BinModel, ViewIn, ViewOut};
use crate::ui::freebin::{FreeBin, FreeformCard};

/// The size of the stack `key` leads: the cards sharing its stack, when it is the first of them.
fn stack_led_by<K: PartialEq>(cards: &[FreeformCard<K>], key: &K) -> Option<usize> {
    let card = cards.iter().find(|card| card.key == *key)?;
    let stack = card.stack?;
    let mut sharing = cards.iter().filter(|other| other.stack == Some(stack));
    let first = sharing.next()?;
    let count = 1 + sharing.count();
    (first.key == *key && count > 1).then_some(count)
}

/// Draws the Freeform view: a `FreeBin` of the shown bin's tiles, built as the Icon view builds them, at the
/// positions of `cards`; a card without a place flows. Placement, moves with stacks, nudges, the marquee and
/// Escape are the collection's; this view maps its output to `BinEvent`s.
pub(super) fn free_view<K, M>(ui: &mut Ui, input: ViewIn<'_, K, M>, cards: &[FreeformCard<K>], snap: Option<f32>) -> ViewOut<K>
where
    K: TextureKey,
    M: BinModel<K> + ?Sized,
{
    let ViewIn { id, model, state, selection, thumbnails, words, collator, rename, hover_scrub, scrub, moving, .. } = input;
    let (entries, _) = entries_of(model, state, collator);
    let searching = !state.search.trim().is_empty();
    let shown = Cards::new(model, entries);
    let mods = craft_ui::runtime::pointer::frame(ui.ctx()).mods;
    if let Some(key) = rename {
        request_rename(ui.ctx(), id, &key);
    }
    let drop_into = |key: &K| model.is_bin(key);
    let facts = TileFacts { model, thumbnails, hover_scrub, scrub: scrub.as_ref() };
    let empty = if searching { words.no_matches.clone() } else { words.empty.clone() };
    let bin = FreeBin::<K>::new(words.freeform_view.clone(), cards)
        .id(id)
        .select(SelectMode::Multi)
        .selection(selection)
        .zoom(state.settings.zoom as f32)
        .aspect(ASPECT)
        .flow_all(searching)
        .drop_into(drop_into)
        .rename(true)
        .deletable(true)
        .rows_stamp(stamp_of(state.shown.as_ref(), &state.search, state.settings.icon_sort.as_ref()))
        .empty(empty);
    let bin = if let Some(step) = snap { bin.snap(step) } else { bin };
    let bin = if let Some(live) = moving { bin.moving(live) } else { bin };
    let out = bin.show_items(ui, &shown, &mut |key, _| {
        let tile = tile_of(&facts, key);
        match stack_led_by(cards, key) {
            Some(count) => tile.stack(count),
            None => tile,
        }
    });

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
            CollectionEvent::Dropped { key, keys, .. } => {
                let into = key.or_else(|| state.shown.clone());
                if let Some(items) = drop_items(model, into.as_ref(), &keys) {
                    events.push(BinEvent::Drop { items, into });
                }
            }
            CollectionEvent::Rename(edit) => {
                events.extend(rename_event(&edit, |key| shown.keys.iter().position(|held| held == key).map(|at| shown.names[at].as_str())));
            }
            CollectionEvent::Item(key, TilePartEvent::Scrub(fraction)) if hover_scrub && !model.is_bin(&key) => {
                events.push(BinEvent::Scrub { key, fraction });
            }
            _ => {}
        }
    }
    let moving = out.moving;
    if !out.moved.is_empty() {
        let (keys, to): (Vec<K>, Vec<egui::Vec2>) = out.moved.into_iter().unzip();
        events.push(BinEvent::FreeformMoved { keys, to });
    }
    if let Some(drag) = out.dropped_on_background
        && let Some(items) = drop_items(model, state.shown.as_ref(), &drag.keys)
    {
        events.push(BinEvent::Drop { items, into: None });
    }
    ViewOut { response: out.response.response, events, moving }
}
