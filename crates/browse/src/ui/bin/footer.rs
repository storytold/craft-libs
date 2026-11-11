//! The bin footer: the view switch, the zoom slider, Sort Icons, the host's actions and their drops.

use craft_behavior::{ColumnKey, IconId};
use craft_ui::choice::Segmented;
use craft_ui::contract::{Bind, ColumnSort, ColumnSpec, Entry, ListDrag, PopupKind, RowAlign, Show as _, ShowContainer as _, Trigger};
use craft_ui::layout_components::Row;
use craft_ui::menu::Menu;
use craft_ui::primitives::textures::TextureKey;
use craft_ui::sections::{FooterAction, FooterBar};
use craft_ui::tile_grid::{ThumbnailSlider, TileZoom};
use egui::Ui;

use super::list::NAME;
use super::{BinEvent, BinState, BinView, BinWords};

/// A pick in the Sort Icons menu.
#[derive(Clone, Debug, PartialEq)]
enum SortPick {
    UserOrder,
    By(ColumnKey),
}

/// The columns the Sort Icons menu offers: the host's, or the name alone.
fn sortable(columns: &[ColumnSpec], words: &BinWords) -> Vec<ColumnSpec> {
    if columns.is_empty() { vec![ColumnSpec::new(NAME, words.name.clone(), craft_behavior::ColumnWidth::fill(1.0))] } else { columns.to_vec() }
}

/// Draws the footer: the view `Segmented` (List, Icon, Freeform), a `ThumbnailSlider`, then the actions: the
/// host's, Sort Icons (Icon and Freeform views) and New Bin. Returns the footer's events.
pub(super) fn footer<K: TextureKey>(
    ui: &mut Ui,
    state: &BinState<K>,
    columns: &[ColumnSpec],
    actions: &[FooterAction],
    words: &BinWords,
    zoom: TileZoom,
) -> Vec<BinEvent<K>> {
    let mut events = Vec::new();
    let mut view = state.settings.view;
    let mut zoom_value = state.settings.zoom;
    let mut all: Vec<FooterAction> = actions.to_vec();
    let sort_at = (view != BinView::List).then(|| {
        all.push(FooterAction::new(IconId::new("more"), words.sort_icons.clone()).popup(PopupKind::Menu));
        all.len() - 1
    });
    let new_bin = all.len();
    all.push(FooterAction::new(IconId::new("new_folder"), words.new_bin.clone()));

    let modes = [
        Entry::new(BinView::List, words.list_view.clone()).icon(IconId::new("list")),
        Entry::new(BinView::Icon, words.icon_view.clone()).icon(IconId::new("grid")),
        Entry::new(BinView::Freeform, words.freeform_view.clone()).icon(IconId::new("layout_grid")),
    ];
    let row = Row::new().fill_width().align(RowAlign::End).show(ui, |ui, _| {
        Segmented::new(words.views.clone(), Bind::bound(&mut view), &modes).show(ui);
        let edit = ThumbnailSlider::new(words.thumb_size.clone(), Bind::bound(&mut zoom_value), zoom).show(ui);
        (edit, FooterBar::actions(all.clone()).show_footer(ui))
    });
    let Some((edit, bar)) = row.inner else { return events };
    if view != state.settings.view {
        events.push(BinEvent::View(view));
    }
    for event in edit.events {
        events.push(BinEvent::Zoom(event));
    }
    if let Some(drop) = &bar.dropped {
        if let Some(drag) = drop.payload::<ListDrag<K>>() {
            events.push(BinEvent::FooterDrop { action: drop.index, keys: drag.keys.clone() });
        }
    } else if let Some(at) = bar.picked {
        if at == new_bin {
            events.push(BinEvent::NewBin);
        } else if Some(at) != sort_at {
            events.push(BinEvent::Footer { action: at });
        }
    }
    if let Some(response) = sort_at.and_then(|at| bar.responses.get(at)) {
        let current = state.settings.icon_sort.as_ref();
        let mut entries = vec![Entry::new(SortPick::UserOrder, words.user_order.clone()).checked(current.is_none())];
        entries.extend(
            sortable(columns, words).into_iter().map(|spec| {
                Entry::new(SortPick::By(spec.key.clone()), spec.header.clone()).checked(current.is_some_and(|sort| sort.key == spec.key))
            }),
        );
        match Menu::new(words.sort_icons.clone()).values().opened_by(response, Trigger::Press).pick(ui, &entries).picked {
            Some(SortPick::UserOrder) => events.push(BinEvent::IconSort(None)),
            Some(SortPick::By(key)) => {
                let descending = current.is_some_and(|sort| sort.key == key && !sort.descending);
                events.push(BinEvent::IconSort(Some(ColumnSort::new(key, descending))));
            }
            None => {}
        }
    }
    events
}
