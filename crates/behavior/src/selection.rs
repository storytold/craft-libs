//! Selection models: what a collection's selection holds and how a change applies to it.

use crate::item_source::DisplayOrder;
use crate::value::Value;
use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::ops::Range;

mod cells;
mod placed;

pub use cells::{CellAdvance, CellDirection, CellSpace, Positions};
pub use placed::PlacedSelection;

/// A change to a selection, as a collection reports it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum SelectionChange<K> {
    /// Replaces the selection; the last key becomes the lead.
    Set(Vec<K>),
    /// Adds the key when absent, removes it when present.
    Toggle(K),
    /// Empties the selection.
    Clear,
    /// Replaces the selection by one range of cells. The anchor cell is at `first` and the column end opposite
    /// `lead_column`; the lead cell is at `last` and `lead_column`. A model of rows selects the rows.
    Cells(CellRange<K>),
    /// Replaces the whole selection by a result of the cell rules that one range cannot say: several ranges, an
    /// active cell inside a range, a run of Tabs.
    CellSet(Box<CellSelection<K>>),
}

/// A cell: its row key and its column position. `CellAt<u32>` is a cell by position.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[non_exhaustive]
pub struct CellAt<K> {
    /// The cell's row.
    pub row: K,
    /// The cell's column position.
    pub column: u32,
}

impl<K> CellAt<K> {
    /// The cell of `row` at column position `column`.
    pub fn new(row: K, column: u32) -> Self {
        Self { row, column }
    }
}

/// A rectangle of cells: the rows from `first` to `last` in the display order (either may come first), the column
/// positions `columns` and the column of the lead cell. A selection reports it with `first` at the anchor end.
/// `CellRange<u32>` is a rectangle by position (merged cells, covers, whole lines).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub struct CellRange<K> {
    /// The row at one end of the rectangle.
    pub first: K,
    /// The row at the other end.
    pub last: K,
    /// The column positions the rectangle covers.
    pub columns: Range<u32>,
    /// The column of the lead cell. Only a range that a selection reports in [`SelectionChange::Cells`] carries a
    /// meaning here; the ranges a selection stores always hold the column of their last corner.
    pub lead_column: u32,
}

impl<K> CellRange<K> {
    /// The cells of the rows from `first` to `last` in `columns`, with the lead cell in `lead_column`. A range has at
    /// least one column: empty `columns` become the one column at their start.
    pub fn new(first: K, last: K, columns: Range<u32>, lead_column: u32) -> Self {
        let columns = columns.start..columns.end.max(columns.start.saturating_add(1));
        Self { first, last, columns, lead_column }
    }

    /// The rectangle from the anchor cell `from` to the lead cell `to`, whichever way round they lie.
    pub fn cells(from: CellAt<K>, to: CellAt<K>) -> Self {
        let columns = from.column.min(to.column)..from.column.max(to.column).saturating_add(1);
        Self::new(from.row, to.row, columns, to.column)
    }
}

impl<K: PartialEq> CellRange<K> {
    /// Whether the row `row` lies between `first` and `last` in `row.order`. A rectangle whose end left the order
    /// holds no row.
    fn holds_row(&self, row: RowAt<'_, K>) -> bool {
        let (Some(a), Some(b)) = (row.order.index_of(&self.first), row.order.index_of(&self.last)) else {
            return false;
        };
        a.min(b) <= row.at && row.at <= a.max(b)
    }
}

/// The stored form is `{"start": {"row", "column"}, "end": {"row", "column"}}` with inclusive columns; `lead_column`
/// is the end's column.
impl<K: Serialize> Serialize for CellRange<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Stored<'a, K> {
            start: CellAt<&'a K>,
            end: CellAt<&'a K>,
        }
        Stored { start: CellAt::new(&self.first, self.columns.start), end: CellAt::new(&self.last, self.columns.end.saturating_sub(1)) }
            .serialize(serializer)
    }
}

impl<'de, K: Deserialize<'de>> Deserialize<'de> for CellRange<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        struct Stored<K> {
            start: CellAt<K>,
            end: CellAt<K>,
        }
        let Stored { start, end } = Stored::<K>::deserialize(deserializer)?;
        if end.column < start.column {
            return Err(D::Error::custom("a cell range ends in a column before its start"));
        }
        Ok(CellRange::new(start.row, end.row, start.column..end.column.saturating_add(1), end.column))
    }
}

/// A row as the item step reads it: its key, its position and the order that places other keys. A rectangle of
/// cells cannot answer from a key alone.
#[non_exhaustive]
pub struct RowAt<'a, K> {
    /// The row's key.
    pub key: &'a K,
    /// The row's position in `order`.
    pub at: usize,
    /// The display order the position is in.
    pub order: &'a dyn DisplayOrder<K>,
}

impl<'a, K> RowAt<'a, K> {
    /// The row `key` at position `at` of `order`.
    pub fn new(key: &'a K, at: usize, order: &'a dyn DisplayOrder<K>) -> Self {
        Self { key, at, order }
    }
}

impl<K> Clone for RowAt<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for RowAt<'_, K> {}

/// The selection a collection reads and the app owns.
pub trait SelectionModel<K>: Value + Default {
    /// Whether the row `row` is selected.
    fn contains(&self, row: RowAt<'_, K>) -> bool;
    /// The item keyboard navigation moves.
    fn lead(&self) -> Option<&K>;
    /// The fixed end of a range extended with Shift.
    fn anchor(&self) -> Option<&K>;
    /// Applies a change.
    fn apply(&mut self, change: SelectionChange<K>);

    /// The column position of the lead cell; `None` for a model of rows.
    fn lead_column(&self) -> Option<u32> {
        None
    }

    /// Whether the cell of row `row` in column position `column` is selected.
    fn contains_cell(&self, row: RowAt<'_, K>, _column: u32) -> bool {
        self.contains(row)
    }

    /// The cell rules of a model of cells, which the item step runs on a copy over the collection's `CellSpace`; the
    /// result goes back through [`apply`](Self::apply). `None` for a model of rows.
    fn cells(&self) -> Option<&CellSelection<K>> {
        None
    }

    /// The selection by position in `order`, placed once so that the reads of a pass scan no keys. The default
    /// walks `order` once and asks [`contains`](Self::contains) of each row; a model that holds keys or ranges
    /// overrides it with a placement of its ends.
    fn placed(&self, order: &dyn DisplayOrder<K>) -> PlacedSelection {
        let mut spans: Vec<Range<usize>> = Vec::new();
        for at in 0..order.len() {
            let key = order.key(at);
            if !self.contains(RowAt::new(&key, at, order)) {
                continue;
            }
            match spans.last_mut() {
                Some(span) if span.end == at => span.end = at + 1,
                _ => spans.push(at..at + 1),
            }
        }
        PlacedSelection::of_rows(spans)
    }
}

/// A set of keys with a lead and an anchor.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct Selection<K> {
    /// The selected keys.
    pub items: Vec<K>,
    /// The lead.
    pub lead: Option<K>,
    /// The anchor.
    pub anchor: Option<K>,
}

impl<K> Selection<K> {
    /// A selection of `items` with `lead` and `anchor`.
    pub fn new(items: Vec<K>, lead: Option<K>, anchor: Option<K>) -> Self {
        Selection { items, lead, anchor }
    }
}

impl<K: PartialEq> Selection<K> {
    /// Whether `key` is selected.
    pub fn contains(&self, key: &K) -> bool {
        self.items.contains(key)
    }
}

impl<K: Clone> Selection<K> {
    /// A selection of `key` alone, which is also its lead and its anchor.
    pub fn single(key: K) -> Self {
        Selection { items: vec![key.clone()], lead: Some(key.clone()), anchor: Some(key) }
    }
}

impl<K> Default for Selection<K> {
    fn default() -> Self {
        Selection { items: Vec::new(), lead: None, anchor: None }
    }
}

impl<K: Value> SelectionModel<K> for Selection<K> {
    fn contains(&self, row: RowAt<'_, K>) -> bool {
        self.items.contains(row.key)
    }

    fn lead(&self) -> Option<&K> {
        self.lead.as_ref()
    }

    fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref()
    }

    fn placed(&self, order: &dyn DisplayOrder<K>) -> PlacedSelection {
        let mut at: Vec<usize> = self.items.iter().filter_map(|key| order.index_of(key)).collect();
        at.sort_unstable();
        at.dedup();
        PlacedSelection::of_rows(at.into_iter().map(|at| at..at + 1).fold(Vec::<Range<usize>>::new(), |mut spans, next| {
            match spans.last_mut() {
                Some(span) if span.end == next.start => span.end = next.end,
                _ => spans.push(next),
            }
            spans
        }))
    }

    /// Cells select their rows. A range selects the rows at its two ends: a model of rows has no order to name the
    /// rows between them, and the item step reports `Set` for it.
    fn apply(&mut self, change: SelectionChange<K>) {
        match change {
            SelectionChange::Set(keys) => {
                self.anchor = keys.first().cloned();
                self.lead = keys.last().cloned();
                self.items = keys;
            }
            SelectionChange::Toggle(key) => {
                match self.items.iter().position(|k| *k == key) {
                    Some(at) => {
                        self.items.remove(at);
                    }
                    None => self.items.push(key.clone()),
                }
                self.anchor = Some(key.clone());
                self.lead = Some(key);
            }
            SelectionChange::Clear => {
                self.items.clear();
                self.lead = None;
                self.anchor = None;
            }
            SelectionChange::Cells(CellRange { first, last, .. }) => {
                self.apply(SelectionChange::Set(if first == last { vec![first] } else { vec![first, last] }));
            }
            SelectionChange::CellSet(cells) => {
                let mut keys: Vec<K> = Vec::new();
                for range in cells.ranges() {
                    for key in [&range.first, &range.last] {
                        if !keys.contains(key) {
                            keys.push(key.clone());
                        }
                    }
                }
                self.apply(SelectionChange::Set(keys));
            }
        }
    }
}

/// The cell selection of a sheet or table: ranges of cells, the active cell, the anchor and the cells toggled
/// against the ranges. Changes go through [`apply`](SelectionModel::apply) and the rules in `cells.rs`, which keep
/// the invariants (ranges cover the merged cells they cross, the active cell is in the current range, a Tab run ends
/// on any other move).
///
/// The empty selection has no ranges and no active cell. A selection that holds ranges holds an active cell.
/// A range holds rows by key and places them with the display order of the pass, so the rules check the row of the
/// active cell against its range when they run, and collapse a selection that fails the check to its active cell.
///
/// Stored as `ranges`, `current`, `active`, `anchor`, `tab_run` and, when not empty, `toggled`. `current` absent
/// means the last range. Loading fails when the ranges are empty but an active cell is given, when `current` is out
/// of range, when ranges are given without an active cell, and when the active column lies outside the current
/// range's columns.
#[derive(Clone, Debug, PartialEq)]
pub struct CellSelection<K> {
    ranges: Vec<CellRange<K>>,
    /// The index in `ranges` of the current range.
    current: usize,
    active: Option<CellAt<K>>,
    anchor: Option<CellAt<K>>,
    /// The column the current run of Tabs started in, while the last move was a Tab.
    tab_run: Option<u32>,
    /// Cells taken out of the ranges: a cell is selected when a range holds it and it is not listed here.
    toggled: Vec<CellAt<K>>,
}

impl<K> Default for CellSelection<K> {
    fn default() -> Self {
        CellSelection { ranges: Vec::new(), current: 0, active: None, anchor: None, tab_run: None, toggled: Vec::new() }
    }
}

impl<K> CellSelection<K> {
    /// The selected ranges, in the order they were added.
    pub fn ranges(&self) -> &[CellRange<K>] {
        &self.ranges
    }

    /// The range Shift extends: the last one added, until Enter or Tab moves into another.
    pub fn current(&self) -> Option<&CellRange<K>> {
        self.ranges.get(self.current)
    }

    /// The active cell: the one an entry goes into.
    pub fn active(&self) -> Option<&CellAt<K>> {
        self.active.as_ref()
    }

    /// The cell of the current range that Shift extends from.
    pub fn anchor_cell(&self) -> Option<&CellAt<K>> {
        self.anchor.as_ref()
    }

    /// The cells taken out of the ranges.
    pub fn toggled(&self) -> &[CellAt<K>] {
        &self.toggled
    }
}

impl<K: Clone> CellSelection<K> {
    /// One cell, active and the anchor. A cell inside a merge selects the merge through `select` instead.
    pub fn cell(c: CellAt<K>) -> Self {
        let range = CellRange::new(c.row.clone(), c.row.clone(), c.column..c.column.saturating_add(1), c.column);
        CellSelection { ranges: vec![range], current: 0, active: Some(c.clone()), anchor: Some(c), tab_run: None, toggled: Vec::new() }
    }

    /// One range with its first row and first column active and the anchor.
    pub fn range(r: CellRange<K>) -> Self {
        let top_left = CellAt::new(r.first.clone(), r.columns.start);
        CellSelection { ranges: vec![r], current: 0, active: Some(top_left.clone()), anchor: Some(top_left), tab_run: None, toggled: Vec::new() }
    }

    /// Several ranges with `active` in the last one, which is the current range, and the anchor at `active`. `None`
    /// when `ranges` is empty or the last range's columns do not hold the column of `active`. The row of `active` is
    /// checked against the range by the first rule that runs.
    pub fn from_ranges(ranges: Vec<CellRange<K>>, active: CellAt<K>) -> Option<Self> {
        let holds = ranges.last().is_some_and(|r| r.columns.contains(&active.column));
        holds.then(|| CellSelection {
            current: ranges.len() - 1,
            ranges,
            active: Some(active.clone()),
            anchor: Some(active),
            tab_run: None,
            toggled: Vec::new(),
        })
    }
}

impl<K: Value> SelectionModel<K> for CellSelection<K> {
    /// A row is selected when a range holds it and a cell of it is not taken out.
    fn contains(&self, row: RowAt<'_, K>) -> bool {
        self.ranges.iter().filter(|range| range.holds_row(row)).any(|range| {
            let mut out: Vec<u32> =
                self.toggled.iter().filter(|cell| cell.row == *row.key && range.columns.contains(&cell.column)).map(|cell| cell.column).collect();
            out.sort_unstable();
            out.dedup();
            u64::from(range.columns.end.saturating_sub(range.columns.start)) > out.len() as u64
        })
    }

    fn lead(&self) -> Option<&K> {
        self.active.as_ref().map(|cell| &cell.row)
    }

    fn anchor(&self) -> Option<&K> {
        self.anchor.as_ref().map(|cell| &cell.row)
    }

    fn apply(&mut self, change: SelectionChange<K>) {
        match change {
            SelectionChange::Set(keys) => {
                let (Some(first), Some(last)) = (keys.first().cloned(), keys.last().cloned()) else {
                    *self = CellSelection::default();
                    return;
                };
                let ranges: Vec<CellRange<K>> = keys.into_iter().map(whole_row).collect();
                *self = CellSelection {
                    current: ranges.len() - 1,
                    ranges,
                    active: Some(CellAt::new(last, 0)),
                    anchor: Some(CellAt::new(first, 0)),
                    tab_run: None,
                    toggled: Vec::new(),
                };
            }
            SelectionChange::Toggle(key) => {
                let whole = whole_row(key.clone());
                match self.ranges.iter().position(|range| *range == whole) {
                    Some(at) => {
                        self.ranges.remove(at);
                    }
                    None => self.ranges.push(whole),
                }
                self.toggled.clear();
                self.tab_run = None;
                self.current = self.ranges.len().saturating_sub(1);
                let cell = self.ranges.get(self.current).map(|range| CellAt::new(range.first.clone(), range.columns.start));
                self.active = cell.clone();
                self.anchor = cell;
            }
            SelectionChange::Clear => *self = CellSelection::default(),
            SelectionChange::Cells(range) => {
                let other_end = if range.lead_column == range.columns.start { range.columns.end.saturating_sub(1) } else { range.columns.start };
                *self = CellSelection {
                    active: Some(CellAt::new(range.last.clone(), range.lead_column)),
                    anchor: Some(CellAt::new(range.first.clone(), other_end)),
                    ranges: vec![range],
                    current: 0,
                    tab_run: None,
                    toggled: Vec::new(),
                };
            }
            SelectionChange::CellSet(cells) => *self = *cells,
        }
    }

    fn lead_column(&self) -> Option<u32> {
        self.active.as_ref().map(|cell| cell.column)
    }

    /// A cell is selected when a range holds it and it is not among the toggled cells, which are taken out.
    fn contains_cell(&self, row: RowAt<'_, K>, column: u32) -> bool {
        let held = self.ranges.iter().any(|range| range.columns.contains(&column) && range.holds_row(row));
        held && !self.toggled.iter().any(|cell| cell.column == column && cell.row == *row.key)
    }

    fn cells(&self) -> Option<&CellSelection<K>> {
        Some(self)
    }

    fn placed(&self, order: &dyn DisplayOrder<K>) -> PlacedSelection {
        let ranges = self
            .ranges
            .iter()
            .filter_map(|range| {
                let (a, b) = (order.index_of(&range.first)?, order.index_of(&range.last)?);
                Some((a.min(b)..a.max(b) + 1, range.columns.clone()))
            })
            .collect();
        let removed = self.toggled.iter().filter_map(|cell| Some((order.index_of(&cell.row)?, cell.column))).collect();
        PlacedSelection::of_cells(ranges, removed)
    }
}

/// The range of every column of the row `key`.
fn whole_row<K: Clone>(key: K) -> CellRange<K> {
    CellRange::new(key.clone(), key, 0..u32::MAX, 0)
}

fn no_cells<K>(cells: &&[CellAt<K>]) -> bool {
    cells.is_empty()
}

impl<K: Serialize> Serialize for CellSelection<K> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        #[derive(Serialize)]
        struct Stored<'a, K> {
            ranges: &'a [CellRange<K>],
            current: usize,
            #[serde(skip_serializing_if = "Option::is_none")]
            active: Option<&'a CellAt<K>>,
            #[serde(skip_serializing_if = "Option::is_none")]
            anchor: Option<&'a CellAt<K>>,
            tab_run: Option<u32>,
            #[serde(skip_serializing_if = "no_cells")]
            toggled: &'a [CellAt<K>],
        }
        Stored {
            ranges: &self.ranges,
            current: self.current,
            active: self.active.as_ref(),
            anchor: self.anchor.as_ref(),
            tab_run: self.tab_run,
            toggled: &self.toggled,
        }
        .serialize(serializer)
    }
}

impl<'de, K: Deserialize<'de> + Clone> Deserialize<'de> for CellSelection<K> {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(bound(deserialize = "K: Deserialize<'de>"))]
        struct Stored<K> {
            ranges: Vec<CellRange<K>>,
            #[serde(default)]
            current: Option<usize>,
            #[serde(default)]
            active: Option<CellAt<K>>,
            #[serde(default)]
            anchor: Option<CellAt<K>>,
            #[serde(default)]
            tab_run: Option<u32>,
            #[serde(default)]
            toggled: Vec<CellAt<K>>,
        }
        let Stored { ranges, current, active, anchor, tab_run, toggled } = Stored::<K>::deserialize(deserializer)?;
        if ranges.is_empty() {
            if active.is_some() || current.is_some_and(|current| current > 0) {
                return Err(D::Error::custom("a cell selection without ranges has no active cell"));
            }
            return Ok(CellSelection { tab_run, toggled, ..CellSelection::default() });
        }
        let current = current.unwrap_or(ranges.len() - 1);
        let Some(range) = ranges.get(current) else {
            return Err(D::Error::custom(format!("a cell selection needs a current range: {} ranges, current {current}", ranges.len())));
        };
        let Some(active) = active else {
            return Err(D::Error::custom("a cell selection with ranges needs an active cell"));
        };
        if !range.columns.contains(&active.column) {
            return Err(D::Error::custom("the active cell is outside the current range"));
        }
        let anchor = anchor.unwrap_or_else(|| active.clone());
        Ok(CellSelection { ranges, current, active: Some(active), anchor: Some(anchor), tab_run, toggled })
    }
}
