//! Column keys, widths and the sections a grid layout reports.

use emath::Rect;
use smallvec::SmallVec;
use std::borrow::Cow;
use std::ops::Range;

/// Moves the toggle cells of a sub-row (`ItemSource::sub_row`) into its content: side by side in column order, each
/// its column's width, `indent` from the start edge of the first cell that is not a toggle cell, which then starts
/// after them. The toggle columns themselves stay empty in that row. `rtl` puts the start edge on the right. The
/// collection hit-tests a sub-row's toggles in these cells and the row draws them there.
pub fn sub_row_cells(cells: &mut [(ColumnKey, Rect)], is_toggle: impl Fn(&ColumnKey) -> bool, indent: f32, rtl: bool) {
    let Some(content) = cells.iter().position(|(key, _)| !is_toggle(key)) else { return };
    let start = cells[content].1;
    let mut at = if rtl { start.right() - indent } else { start.left() + indent };
    for (key, cell) in cells.iter_mut() {
        if !is_toggle(key) {
            continue;
        }
        let width = cell.width();
        let x = if rtl { at - width..=at } else { at..=at + width };
        *cell = Rect::from_x_y_ranges(x, cell.y_range());
        at += if rtl { -width } else { width };
    }
    let content = &mut cells[content].1;
    if rtl {
        content.max.x = at.max(content.min.x);
    } else {
        content.min.x = at.min(content.max.x);
    }
}

/// The key of a column; stable across reordering and resizing. It serializes as its name, or as its position for a
/// key made by [`ColumnKey::index`].
#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct ColumnKey(KeyRepr);

#[derive(Clone, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
#[serde(untagged)]
enum KeyRepr {
    Name(Cow<'static, str>),
    Position(u32),
}

impl ColumnKey {
    /// A key named `key`.
    pub const fn new(key: &'static str) -> Self {
        ColumnKey(KeyRepr::Name(Cow::Borrowed(key)))
    }

    /// The key of the column at `column` in a sheet whose columns are numbered rather than named
    /// (`SheetLayout::lines`). Keys of equal positions are equal; no allocation.
    pub const fn index(column: u32) -> Self {
        ColumnKey(KeyRepr::Position(column))
    }

    /// The name of a key made by [`ColumnKey::new`]; `None` for a numbered key.
    pub fn as_str(&self) -> Option<&str> {
        match &self.0 {
            KeyRepr::Name(name) => Some(name),
            KeyRepr::Position(_) => None,
        }
    }

    /// The position a key made by [`ColumnKey::index`] holds; `None` for a named key.
    pub fn position(&self) -> Option<u32> {
        match self.0 {
            KeyRepr::Position(column) => Some(column),
            KeyRepr::Name(_) => None,
        }
    }
}

/// How wide a column is.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColumnWidth {
    /// The width of the widest cell.
    Content,
    /// The app's width.
    Fixed(f32),
    /// This many control heights; the owner that knows the control height resolves it with
    /// [`ColumnWidth::in_unit`] before the widths are computed.
    Controls(f32),
    /// As wide as a toggle cell: a control and the inline gap. The owner that knows the control size resolves it
    /// before the widths are computed.
    Toggle,
    /// A share of the remaining width, by weight.
    #[non_exhaustive]
    Fill {
        /// The column's weight among the `Fill` columns.
        weight: f32,
    },
}

impl ColumnWidth {
    /// The width with `Controls(n)` turned into `Fixed(n * unit)`.
    pub fn in_unit(self, unit: f32) -> Self {
        match self {
            Self::Controls(count) => Self::Fixed(count * unit),
            other => other,
        }
    }

    /// A share of the remaining width with `weight` among the `Fill` columns.
    pub const fn fill(weight: f32) -> Self {
        Self::Fill { weight }
    }
}

/// One horizontal section of a grid layout: its rect, the columns it shows and their edges.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GridSection {
    /// Relative to the viewport, like every rect of an `ItemLayout`.
    pub rect: Rect,
    /// The columns the section shows.
    pub columns: Range<u32>,
    /// The x of each column edge.
    pub edges: Vec<f32>,
}

impl GridSection {
    /// A section at `rect` showing `columns` with their edges.
    pub fn new(rect: Rect, columns: Range<u32>, edges: Vec<f32>) -> Self {
        Self { rect, columns, edges }
    }
}

/// The sections of a sheet layout, one per frozen area.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GridView {
    /// The sections, in reading order.
    pub sections: SmallVec<[GridSection; 4]>,
}

impl GridView {
    /// A view of `sections`, in reading order.
    pub fn new(sections: impl IntoIterator<Item = GridSection>) -> Self {
        Self { sections: sections.into_iter().collect() }
    }
}
