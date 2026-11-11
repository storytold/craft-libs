//! The grid's outline: the tiles the app gives with a header row before each group's first tile, then the tail rows.
//! The item step sees this source, so a header or a tail row is a row it can put the lead on, and the layout places
//! the rows by the same positions.

use super::layout::{Slot, TailRow, TileGroup, group_spans, headers_through, outline_position, slot_at};
use craft_behavior::{ColumnKey, DisplayOrder, ItemSource, SizeHint};
use std::borrow::Cow;
use std::hash::{Hash, Hasher};

/// A group header as the outline holds it.
struct Header<K> {
    key: K,
    open: bool,
}

/// The tiles of `inner` with the header of every group just before the group's first tile, then the tail rows. Header
/// and tail rows are sections that hold the lead (`DisplayOrder::section_stop`); headers open and close when the grid
/// collapses.
pub(super) struct Outline<'s, K> {
    inner: &'s dyn ItemSource<K>,
    headers: Vec<Header<K>>,
    starts: Vec<usize>,
    /// The keys of the tail rows.
    tail: Vec<K>,
    collapsible: bool,
}

impl<'s, K: Clone> Outline<'s, K> {
    pub(super) fn new(inner: &'s dyn ItemSource<K>, groups: &[TileGroup<K>], tail: &[TailRow<K>], collapsible: bool) -> Self {
        let spans = group_spans(groups, inner.len());
        let headers = groups.iter().map(|group| Header { key: group.key.clone(), open: group.open }).collect();
        let tail = tail.iter().map(|row| row.key.clone()).collect();
        Outline { inner, headers, starts: spans.iter().map(|span| span.start).collect(), tail, collapsible }
    }

    fn slot(&self, at: usize) -> Slot {
        slot_at(&self.starts, self.inner.len(), at)
    }

    /// The position of the first tail row.
    fn tail_from(&self) -> usize {
        self.inner.len() + self.headers.len()
    }
}

impl<K: Clone + PartialEq> DisplayOrder<K> for Outline<'_, K> {
    fn len(&self) -> usize {
        self.tail_from() + self.tail.len()
    }

    fn key(&self, at: usize) -> K {
        match self.slot(at) {
            Slot::Header(group) => self.headers[group].key.clone(),
            Slot::Tile(tile) => self.inner.key(tile),
            Slot::Tail(tail) => self.tail[tail].clone(),
        }
    }

    fn index_of(&self, key: &K) -> Option<usize> {
        if let Some(tail) = self.tail.iter().position(|row| row == key) {
            return Some(self.tail_from() + tail);
        }
        match self.headers.iter().position(|header| header.key == *key) {
            Some(group) => Some(self.starts[group] + group),
            None => self.inner.index_of(key).map(|tile| outline_position(&self.starts, tile)),
        }
    }

    fn depth(&self, at: usize) -> usize {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => 0,
            Slot::Tile(tile) => self.inner.depth(tile),
        }
    }

    fn has_children(&self, at: usize) -> bool {
        match self.slot(at) {
            Slot::Header(_) => self.collapsible,
            Slot::Tail(_) => false,
            Slot::Tile(tile) => self.inner.has_children(tile),
        }
    }

    fn open(&self, at: usize) -> bool {
        match self.slot(at) {
            Slot::Header(group) => self.headers[group].open,
            Slot::Tail(_) => false,
            Slot::Tile(tile) => self.inner.open(tile),
        }
    }

    fn is_section(&self, at: usize) -> bool {
        matches!(self.slot(at), Slot::Header(_) | Slot::Tail(_))
    }

    fn section_stop(&self, at: usize) -> bool {
        self.is_section(at)
    }

    fn section_stops_before(&self, at: usize) -> usize {
        at.checked_sub(1).map_or(0, |before| headers_through(&self.starts, before)) + at.saturating_sub(self.tail_from())
    }

    fn enabled(&self, at: usize) -> bool {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => true,
            Slot::Tile(tile) => self.inner.enabled(tile),
        }
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => None,
            Slot::Tile(tile) => self.inner.text(tile),
        }
    }

    fn revision(&self) -> Option<u64> {
        let inner = self.inner.revision()?;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        (inner, &self.starts, self.tail.len()).hash(&mut hasher);
        Some(hasher.finish())
    }
}

impl<K: Clone + PartialEq> ItemSource<K> for Outline<'_, K> {
    fn size_hint(&self, at: usize) -> SizeHint {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => SizeHint::Auto,
            Slot::Tile(tile) => self.inner.size_hint(tile),
        }
    }

    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => false,
            Slot::Tile(tile) => self.inner.toggled(tile, column),
        }
    }

    fn toggle_enabled(&self, at: usize, column: &ColumnKey) -> bool {
        match self.slot(at) {
            Slot::Header(_) | Slot::Tail(_) => false,
            Slot::Tile(tile) => self.inner.toggle_enabled(tile, column),
        }
    }
}
