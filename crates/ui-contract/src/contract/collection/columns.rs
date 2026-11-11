//! A collection's columns: kinds, specs, alignment, toggle columns and the sort.

use crate::runtime::env::Label;
use craft_behavior::{ColumnKey, ColumnWidth};

/// A toggle column: its cells flip by click, and a drag across them sweeps one value over all of them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToggleColumn {
    /// A drag across the cells sets them all to the first cell's new value.
    pub sweepable: bool,
    /// Turning a cell on turns every other cell of the column off (solo).
    pub exclusive: bool,
}

impl ToggleColumn {
    /// A sweepable toggle column (eye, lock, mute).
    pub const fn sweepable() -> Self {
        ToggleColumn { sweepable: true, exclusive: false }
    }

    /// An exclusive toggle column: one cell is on at a time, and a drag does not sweep.
    pub const fn exclusive() -> Self {
        ToggleColumn { sweepable: false, exclusive: true }
    }
}

/// What a column's cells are.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ColumnKind {
    /// Content the item paints.
    Content,
    /// A toggle the collection flips.
    Toggle(ToggleColumn),
}

/// One column of a collection.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct ColumnSpec {
    /// The column's stable key.
    pub key: ColumnKey,
    /// The header text.
    pub header: Label,
    /// How wide the column is.
    pub width: ColumnWidth,
    /// The user can drag the header's edge.
    pub resizable: bool,
    /// Activating the header reports `Sort`.
    pub sortable: bool,
    /// The user can move the column.
    pub movable: bool,
    /// The column stays in view while the others scroll.
    pub frozen: bool,
    /// What the cells are.
    pub kind: ColumnKind,
    /// Where the cells' content sits in the column.
    pub align: ColumnAlign,
    /// The cells' figures are tabular, so a column of numbers lines up.
    pub tabular: bool,
    /// The first press on the header sorts descending: sizes, dates and ratings, where the largest comes first.
    pub first_descending: bool,
    /// The icon the column's cells show: the name of a cell that has no header text.
    pub icon: Option<craft_behavior::IconId>,
}

/// Where a column's content sits.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ColumnAlign {
    /// At the start edge.
    #[default]
    Start,
    /// At the end edge.
    End,
}

impl ColumnSpec {
    /// A content column with a header and a width; the other settings off.
    pub fn new(key: ColumnKey, header: impl Into<Label>, width: ColumnWidth) -> Self {
        ColumnSpec {
            key,
            header: header.into(),
            width,
            resizable: false,
            sortable: false,
            movable: false,
            frozen: false,
            kind: ColumnKind::Content,
            align: ColumnAlign::Start,
            tabular: false,
            first_descending: false,
            icon: None,
        }
    }

    /// Activating the header sorts.
    #[must_use]
    pub fn sortable(mut self) -> Self {
        self.sortable = true;
        self
    }

    /// The user can resize the column.
    #[must_use]
    pub fn resizable(mut self) -> Self {
        self.resizable = true;
        self
    }

    /// The user can move the column.
    #[must_use]
    pub fn movable(mut self) -> Self {
        self.movable = true;
        self
    }

    /// The column stays in view.
    #[must_use]
    pub fn frozen(mut self) -> Self {
        self.frozen = true;
        self
    }

    /// The column's cell kind.
    #[must_use]
    pub fn kind(mut self, kind: ColumnKind) -> Self {
        self.kind = kind;
        self
    }

    /// The cells' content sits at `align`.
    #[must_use]
    pub fn align(mut self, align: ColumnAlign) -> Self {
        self.align = align;
        self
    }

    /// A column of numbers: content at the end edge, tabular figures.
    #[must_use]
    pub fn numeric(mut self) -> Self {
        self.align = ColumnAlign::End;
        self.tabular = true;
        self
    }

    /// The icon the column's cells show. A cell of a column with no header text is named after it.
    #[must_use]
    pub fn icon(mut self, icon: craft_behavior::IconId) -> Self {
        self.icon = Some(icon);
        self
    }

    /// The first press on the header sorts descending.
    #[must_use]
    pub fn first_descending(mut self) -> Self {
        self.first_descending = true;
        self
    }
}

/// The column the rows are sorted by and the direction: the app's value, which it changes on `ColumnEvent::Sort`.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[non_exhaustive]
pub struct ColumnSort {
    /// The sorted column.
    pub key: ColumnKey,
    /// The order is descending.
    pub descending: bool,
}

impl ColumnSort {
    /// Rows sorted by `key`, descending when `descending`.
    pub const fn new(key: ColumnKey, descending: bool) -> Self {
        ColumnSort { key, descending }
    }

    /// Rows sorted by `key`, ascending.
    pub const fn ascending(key: ColumnKey) -> Self {
        ColumnSort { key, descending: false }
    }

    /// Rows sorted by `key`, descending.
    pub const fn descending(key: ColumnKey) -> Self {
        ColumnSort { key, descending: true }
    }
}
