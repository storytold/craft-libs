//! Where a collection's items come from: the data traits and the one way to show a slice.

use crate::columns::ColumnKey;
use crate::item_layout::ItemLayout;
use crate::nav::NavOutline;
use crate::scroll::ScrollPos;
use emath::{Rect, Vec2};
use std::borrow::Cow;

/// How tall (or what shape) an item wants to be.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum SizeHint {
    /// Measured by the item at its own step 5.
    Auto,
    /// A height the app gives, in points.
    Height(f32),
    /// Width over height; the layout picks the width.
    Aspect(f32),
    /// A size the app gives, in points.
    Rect(Vec2),
}

/// The order a collection shows its entries in. The library never owns the data.
pub trait DisplayOrder<K> {
    /// The number of entries.
    fn len(&self) -> usize;

    /// The key of entry `at`. Items take their id from it, never from the position.
    fn key(&self, at: usize) -> K;

    /// The position of `key`, `None` when the source no longer holds it. The default scans; a source over a
    /// large collection answers from its own index.
    fn index_of(&self, key: &K) -> Option<usize>
    where
        K: PartialEq,
    {
        (0..self.len()).find(|&at| self.key(at) == *key)
    }

    /// The nesting depth of entry `at`, 0 for a root.
    fn depth(&self, _at: usize) -> usize {
        0
    }

    /// Whether entry `at` has children.
    fn has_children(&self, _at: usize) -> bool {
        false
    }

    /// Whether the children of entry `at` show.
    fn open(&self, _at: usize) -> bool {
        false
    }

    /// Whether entry `at` is a section header, which navigation skips.
    fn is_section(&self, _at: usize) -> bool {
        false
    }

    /// Whether section `at` is a stop of navigation: it holds the lead, takes Enter, the context-menu key and a
    /// press, and opens or closes with Forward and Back when it has children. A move stops on it, and it is never
    /// selected, extended over, toggled, marqueed, dragged, renamed or reached by type-ahead. Only meaningful for a
    /// section.
    fn section_stop(&self, _at: usize) -> bool {
        false
    }

    /// How many section stops (`section_stop`) come before entry `at`. They are not members of the set of items, so
    /// the positions an item reports to assistive technology do not count them.
    fn section_stops_before(&self, _at: usize) -> usize {
        0
    }

    /// Whether entry `at` can take the lead.
    fn enabled(&self, _at: usize) -> bool {
        true
    }

    /// The text type-ahead matches entry `at` against.
    fn text(&self, _at: usize) -> Option<Cow<'_, str>> {
        None
    }

    /// A number that changes whenever the entries or their order change; `None` when the source does not report.
    fn revision(&self) -> Option<u64> {
        None
    }

    /// Whether there are no entries.
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// A display order that also says how tall its entries want to be.
pub trait ItemSource<K>: DisplayOrder<K> {
    /// The size entry `at` wants.
    fn size_hint(&self, _at: usize) -> SizeHint {
        SizeHint::Auto
    }

    /// Whether the cell of entry `at` in the toggle column `column` is on.
    fn toggled(&self, _at: usize, _column: &ColumnKey) -> bool {
        false
    }

    /// Whether the cell of entry `at` in the toggle column `column` can be flipped: a locked field's cell shows its
    /// value and takes neither a press, a sweep nor the Space key.
    fn toggle_enabled(&self, _at: usize, _column: &ColumnKey) -> bool {
        true
    }
}

/// `DisplayOrder` as the key-free outline the navigation core steps over.
pub struct Outline<'a, K> {
    order: &'a dyn DisplayOrder<K>,
    columns: usize,
    layout: Option<&'a dyn ItemLayout>,
}

impl<'a, K> Outline<'a, K> {
    /// The outline of `order`.
    pub fn new(order: &'a dyn DisplayOrder<K>) -> Self {
        Outline { order, columns: 1, layout: None }
    }

    /// The rows and hidden items of `layout`, for ragged-grid navigation.
    #[must_use]
    pub fn layout(mut self, layout: &'a dyn ItemLayout) -> Self {
        self.layout = Some(layout);
        self
    }

    /// The cells of every position, for a sheet's cell navigation.
    #[must_use]
    pub fn columns(mut self, columns: usize) -> Self {
        self.columns = columns.max(1);
        self
    }
}

impl<K> NavOutline for Outline<'_, K> {
    fn len(&self) -> usize {
        self.order.len()
    }

    fn depth(&self, at: usize) -> usize {
        self.order.depth(at)
    }

    fn has_children(&self, at: usize) -> bool {
        self.order.has_children(at)
    }

    fn open(&self, at: usize) -> bool {
        self.order.open(at)
    }

    fn is_section(&self, at: usize) -> bool {
        self.order.is_section(at) && !self.order.section_stop(at)
    }

    fn enabled(&self, at: usize) -> bool {
        self.order.enabled(at) && self.layout.is_none_or(|layout| layout.shown(at))
    }

    fn row_len(&self, row: usize) -> Option<usize> {
        self.layout.and_then(|layout| layout.row_len(row))
    }

    fn columns(&self) -> usize {
        self.columns
    }

    /// The layout's rect of the item in content space (scroll offset zero), so a scrolled outline moves the same way.
    fn rect(&self, at: usize) -> Option<Rect> {
        let rect = self.layout?.rect(at, ScrollPos::default());
        rect.is_finite().then_some(rect)
    }
}

type KeyFn<'a, T, K> = Box<dyn Fn(&T) -> K + 'a>;
type TextFn<'a, T> = Box<dyn for<'t> Fn(&'t T) -> Cow<'t, str> + 'a>;
type ToggledFn<'a, T> = Box<dyn Fn(&T, &ColumnKey) -> bool + 'a>;

/// A slice as an item source, each entry keyed by `key`.
pub struct Keyed<'a, T, K> {
    items: &'a [T],
    key: KeyFn<'a, T, K>,
    text: Option<TextFn<'a, T>>,
    toggled: Option<ToggledFn<'a, T>>,
}

/// Shows `items` keyed by `key`: the one way to show a slice (7.1 rule 4).
pub fn keyed<'a, T, K>(items: &'a [T], key: impl Fn(&T) -> K + 'a) -> Keyed<'a, T, K> {
    Keyed { items, key: Box::new(key), text: None, toggled: None }
}

impl<'a, T, K> Keyed<'a, T, K> {
    /// The text type-ahead matches each entry against.
    #[must_use]
    pub fn with_text(mut self, text: impl for<'t> Fn(&'t T) -> Cow<'t, str> + 'a) -> Self {
        self.text = Some(Box::new(text));
        self
    }

    /// Whether an entry's cell in a toggle column is on. Without it every toggle cell is off.
    #[must_use]
    pub fn with_toggled(mut self, toggled: impl Fn(&T, &ColumnKey) -> bool + 'a) -> Self {
        self.toggled = Some(Box::new(toggled));
        self
    }
}

impl<T, K> DisplayOrder<K> for Keyed<'_, T, K> {
    fn len(&self) -> usize {
        self.items.len()
    }

    fn key(&self, at: usize) -> K {
        (self.key)(&self.items[at])
    }

    fn text(&self, at: usize) -> Option<Cow<'_, str>> {
        self.text.as_ref().map(|text| text(&self.items[at]))
    }
}

impl<T, K> ItemSource<K> for Keyed<'_, T, K> {
    fn toggled(&self, at: usize, column: &ColumnKey) -> bool {
        self.toggled.as_ref().is_some_and(|toggled| toggled(&self.items[at], column))
    }
}
