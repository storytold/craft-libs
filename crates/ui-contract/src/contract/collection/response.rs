//! A collection's output: `CollectionResponse`, the rows it showed and the gestures it allows.

use super::events::CollectionEvent;
use super::props::{CollectionProps, SelectMode};
use craft_behavior::ColumnKey;
use egui::Response;
use std::ops::Range;

/// The output of a collection.
#[derive(Debug)]
#[non_exhaustive]
pub struct CollectionResponse<K, E> {
    /// The collection's own response.
    pub response: Response,
    /// Every event, in drawing order.
    pub events: Vec<CollectionEvent<K, E>>,
    /// The positions the collection showed.
    pub visible: Range<usize>,
    /// Where each shown row and its cells sit this pass, in screen space, in drawing order.
    pub shown: Vec<ShownRow<K>>,
    /// What the collection lets the user select.
    pub select: SelectMode,
    /// The collection renames items.
    pub rename: bool,
    /// The collection reorders items.
    pub reorder: bool,
    /// The click count of the press behind this pass's `Activate`: 2 for the second click of a double click, 0 when a
    /// key or no pointer activated.
    pub activation_clicks: u8,
}

impl<K, E> CollectionResponse<K, E> {
    pub(crate) fn new(
        response: Response,
        (events, shown, activation_clicks): (Vec<CollectionEvent<K, E>>, Vec<ShownRow<K>>, u8),
        visible: Range<usize>,
        gestures: Gestures,
    ) -> Self {
        CollectionResponse {
            response,
            events,
            visible,
            shown,
            select: gestures.select,
            rename: gestures.rename,
            reorder: gestures.reorder,
            activation_clicks,
        }
    }

    /// The response of the item part `opener` names: its click this pass and its rect. A menu or popover hangs from
    /// it with `opened_by`, as from a button. `None` while the item is not shown.
    pub fn opener(&self, opener: crate::contract::ItemOpener) -> Option<&Response> {
        self.shown.iter().flat_map(|row| &row.openers).find(|(held, _)| *held == opener).map(|(_, response)| response)
    }
}

/// The geometry of one row the collection showed this pass.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ShownRow<K> {
    /// The row's key.
    pub key: K,
    /// The row's rect.
    pub rect: egui::Rect,
    /// The rect of each cell, by column key; empty for a collection with no columns.
    pub cells: Vec<(ColumnKey, egui::Rect)>,
    /// The openers the row's item asked for (`ItemCx::part_opener`), each with the part's response this pass.
    pub openers: Vec<(crate::contract::ItemOpener, Response)>,
}

impl<K> ShownRow<K> {
    pub(crate) fn new(key: K, rect: egui::Rect, cells: Vec<(ColumnKey, egui::Rect)>) -> Self {
        ShownRow { key, rect, cells, openers: Vec::new() }
    }
}

/// The gestures a collection's props allow, as its response reports them.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Gestures {
    pub(crate) select: SelectMode,
    pub(crate) rename: bool,
    pub(crate) reorder: bool,
}

impl Gestures {
    pub(crate) fn of(props: &CollectionProps) -> Self {
        Gestures { select: props.select, rename: props.rename, reorder: props.reorder.is_some() }
    }
}

impl<K, E> crate::contract::Outcome for CollectionResponse<K, E> {
    type Value = crate::contract::NoValue;

    fn response(&self) -> &Response {
        &self.response
    }

    #[cfg(feature = "testing")]
    fn story_facts(&self) -> crate::testing::StoryOutput {
        crate::testing::StoryOutput::of_collection(&self.response, &self.events, &self.visible)
    }
}
