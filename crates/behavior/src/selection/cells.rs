//! The rules of a grid of selected cells: a spreadsheet's sheet and a document table. Pure data and rules: no egui,
//! no drawing, no keys read. Each widget maps its own input onto these rules and draws the selection its own way
//! (Excel's fill with the active cell left clear, InDesign's tint with an accent outline).
//!
//! - [`CellSpace`]: the grid the rules run in, built by the widget each pass: the display order of its rows, its
//!   columns, its merged cells, its hidden rows and columns and the cells that can't be selected.
//! - [`CellSelection`]'s rules: Excel's rules for clicks, Shift and Cmd/Ctrl, the arrows, and Enter and Tab inside a
//!   selection.
//!
//! The rules are Excel's. A document table adds InDesign's and Word's Tab, which runs on along the rows
//! ([`CellSpace::next_in_reading_order`]).
//!
//! A selection holds row keys; the rules run on positions. Each rule places the selection with
//! [`CellSpace::place`], works on positions and keys the result with [`CellSpace::keyed`].

use super::{CellAt, CellRange, CellSelection, SelectionChange, SelectionModel as _};
use crate::item_source::DisplayOrder;
use crate::value::Value;
use std::ops::RangeInclusive;

/// The display order of a grid whose row keys are their positions (a sheet, a document table): `key(at) == at`, and
/// `index_of` answers without a scan.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Positions(u32);

impl Positions {
    /// The order of `rows` rows.
    pub const fn new(rows: u32) -> Self {
        Positions(rows)
    }
}

impl DisplayOrder<u32> for Positions {
    fn len(&self) -> usize {
        self.0 as usize
    }

    fn key(&self, at: usize) -> u32 {
        at as u32
    }

    fn index_of(&self, key: &u32) -> Option<usize> {
        (*key < self.0).then_some(*key as usize)
    }
}

impl CellRange<u32> {
    /// One cell.
    pub fn cell(c: CellAt<u32>) -> Self {
        Self::rect(c.row, c.row, c.column, c.column)
    }

    /// The rectangle of rows `top..=bottom` and columns `left..=right`, normal: the lead cell in the last column.
    fn rect(top: u32, bottom: u32, left: u32, right: u32) -> Self {
        CellRange::new(top, bottom, left..right.saturating_add(1), right)
    }

    /// The rectangle spanned by two corners in any order.
    fn spanning(a: CellAt<u32>, b: CellAt<u32>) -> Self {
        Self::rect(a.row.min(b.row), a.row.max(b.row), a.column.min(b.column), a.column.max(b.column))
    }

    fn top(&self) -> u32 {
        self.first.min(self.last)
    }

    fn bottom(&self) -> u32 {
        self.first.max(self.last)
    }

    fn left(&self) -> u32 {
        self.columns.start
    }

    fn right(&self) -> u32 {
        self.columns.end.saturating_sub(1)
    }

    fn top_left(&self) -> CellAt<u32> {
        CellAt::new(self.top(), self.left())
    }

    /// Whether `c` lies in the range.
    pub fn contains(&self, c: CellAt<u32>) -> bool {
        self.rows().contains(&c.row) && self.column_span().contains(&c.column)
    }

    /// Whether the range is one cell.
    fn is_single_cell(&self) -> bool {
        self.top() == self.bottom() && self.left() == self.right()
    }

    /// The rows the range spans.
    pub fn rows(&self) -> RangeInclusive<u32> {
        self.top()..=self.bottom()
    }

    /// The columns the range spans.
    pub fn column_span(&self) -> RangeInclusive<u32> {
        self.left()..=self.right()
    }

    /// Whether the two ranges share a cell.
    pub fn intersects(&self, other: &Self) -> bool {
        self.top() <= other.bottom() && other.top() <= self.bottom() && self.left() <= other.right() && other.left() <= self.right()
    }

    /// The smallest range holding both.
    pub fn union(&self, other: &Self) -> Self {
        Self::rect(self.top().min(other.top()), self.bottom().max(other.bottom()), self.left().min(other.left()), self.right().max(other.right()))
    }

    /// The range grown until no merged cell (`merges`: each merge's range) crosses its edge, as Excel, InDesign and
    /// Word grow a selection over a merged cell. Grows again after each pass that took in a merge, since a merge
    /// taken in can reach another one.
    pub fn covering_merges(self, merges: &[CellRange<u32>]) -> Self {
        let mut grown = self;
        loop {
            let before = grown.clone();
            for m in merges {
                if m.intersects(&grown) {
                    grown = grown.union(m);
                }
            }
            if grown == before {
                return grown;
            }
        }
    }
}

/// A move of one cell. Logical: in a right-to-left sheet the widget maps the Left arrow to
/// [`Right`](Self::Right) (Excel mirrors the arrows with the sheet).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellDirection {
    /// Toward the first row.
    Up,
    /// Toward the last row.
    Down,
    /// Toward the first column.
    Left,
    /// Toward the last column.
    Right,
}

/// Which key moves the active cell after an entry: Enter goes down the rows, Tab along a row.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum CellAdvance {
    /// Enter: down the rows.
    Enter,
    /// Tab: along the row.
    Tab,
}

/// The grid the selection rules run in, built by the widget each pass: the rows in the display order of the
/// collection, `columns` columns (a sheet has Excel's 16,384), the merged cells, the hidden rows and columns the
/// arrows skip, and the cells that can't be selected (Excel's protected sheet with "Select unlocked cells" only).
/// Merges, hidden lines and locked cells are by position.
///
/// ```ignore
/// let hidden = |row| sheet.is_row_hidden(row);
/// let space = CellSpace::new(&Positions::new(SHEET_ROWS), SHEET_COLUMNS).merges(&sheet.merges).hidden_rows(&hidden);
/// state.selection.step(CellDirection::Down, &space);
/// ```
pub struct CellSpace<'a, K> {
    rows: &'a dyn DisplayOrder<K>,
    row_total: u32,
    columns: u32,
    merges: &'a [CellRange<u32>],
    hidden_rows: Option<&'a dyn Fn(u32) -> bool>,
    hidden_columns: Option<&'a dyn Fn(u32) -> bool>,
    selectable: Option<&'a dyn Fn(CellAt<u32>) -> bool>,
}

impl<K> Clone for CellSpace<'_, K> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<K> Copy for CellSpace<'_, K> {}

impl<'a, K: Value> CellSpace<'a, K> {
    /// A grid of the rows of `rows` and `columns` columns with no merges, nothing hidden and every cell
    /// selectable. The column count is at least 1; the row count is 0 for an empty order.
    pub fn new(rows: &'a dyn DisplayOrder<K>, columns: u32) -> Self {
        let row_total = u32::try_from(rows.len()).unwrap_or(u32::MAX);
        Self { rows, row_total, columns: columns.max(1), merges: &[], hidden_rows: None, hidden_columns: None, selectable: None }
    }

    /// The merged cells, each as its range, by position. They don't overlap.
    pub fn merges(mut self, merges: &'a [CellRange<u32>]) -> Self {
        self.merges = merges;
        self
    }

    /// Which rows are hidden, by position; the arrows skip them.
    pub fn hidden_rows(mut self, hidden: &'a dyn Fn(u32) -> bool) -> Self {
        self.hidden_rows = Some(hidden);
        self
    }

    /// Which columns are hidden; the arrows skip them.
    pub fn hidden_columns(mut self, hidden: &'a dyn Fn(u32) -> bool) -> Self {
        self.hidden_columns = Some(hidden);
        self
    }

    /// Which cells a click or a move may land on; the arrows step over the others in their direction and stay put
    /// when there is none.
    pub fn selectable(mut self, selectable: &'a dyn Fn(CellAt<u32>) -> bool) -> Self {
        self.selectable = Some(selectable);
        self
    }

    /// The number of rows.
    pub fn row_count(&self) -> u32 {
        self.row_total
    }

    /// The number of columns.
    pub fn column_count(&self) -> u32 {
        self.columns
    }

    /// The position of the cell `c`; `None` when its row left the order.
    pub fn place(&self, c: &CellAt<K>) -> Option<CellAt<u32>> {
        let at = u32::try_from(self.rows.index_of(&c.row)?).ok()?;
        Some(CellAt::new(at, c.column))
    }

    /// The rectangle `r` by position, normal (the first row on top); `None` when an end row left the order.
    fn place_range(&self, r: &CellRange<K>) -> Option<CellRange<u32>> {
        let a = u32::try_from(self.rows.index_of(&r.first)?).ok()?;
        let b = u32::try_from(self.rows.index_of(&r.last)?).ok()?;
        Some(CellRange::rect(a.min(b), a.max(b), r.columns.start, r.columns.end.saturating_sub(1)))
    }

    /// The cell at position `c`, by its row key.
    pub fn keyed(&self, c: CellAt<u32>) -> CellAt<K> {
        CellAt::new(self.rows.key(c.row as usize), c.column)
    }

    /// The rectangle at position `r`, by its row keys.
    pub fn keyed_range(&self, r: CellRange<u32>) -> CellRange<K> {
        CellRange::new(self.rows.key(r.first as usize), self.rows.key(r.last as usize), r.columns, r.lead_column)
    }

    /// The merged cell holding `c`, if any.
    pub fn merge_at(&self, c: CellAt<u32>) -> Option<CellRange<u32>> {
        self.merges.iter().find(|m| m.contains(c)).cloned()
    }

    /// Every cell (Cmd/Ctrl+A on a sheet, Select › Table in a document); `None` when the order has no rows.
    pub fn all(&self) -> Option<CellRange<u32>> {
        self.whole_columns(0, self.columns - 1)
    }

    /// Rows `first..=last` across every column (a row header click, or a drag over row headers).
    pub fn whole_rows(&self, first: u32, last: u32) -> CellRange<u32> {
        CellRange::rect(first.min(last), first.max(last), 0, self.columns - 1)
    }

    /// Columns `first..=last` down every row (a column header click); `None` when the order has no rows.
    pub fn whole_columns(&self, first: u32, last: u32) -> Option<CellRange<u32>> {
        let bottom = self.row_total.checked_sub(1)?;
        Some(CellRange::rect(0, bottom, first.min(last), first.max(last)))
    }

    /// `range` grown over the merged cells it crosses ([`CellRange::covering_merges`]); whole rows and whole columns
    /// stay as they are: a click on column B's header selects only B even when B1:C1 is merged, as in Excel.
    pub fn cover(&self, range: CellRange<u32>) -> CellRange<u32> {
        let whole_rows = range.left() == 0 && range.right() == self.columns - 1;
        let whole_columns = range.top() == 0 && self.row_total.checked_sub(1) == Some(range.bottom());
        if whole_rows || whole_columns { range } else { range.covering_merges(self.merges) }
    }

    /// The cell one step from `from` toward `direction`: out of `from`'s merged cell from its far edge, over hidden
    /// rows and columns, onto the merged cell's top-left when it lands in one, and on over cells that aren't
    /// selectable. `None` at the grid's edge, or when no selectable cell lies that way.
    pub fn neighbour(&self, from: CellAt<u32>, direction: CellDirection) -> Option<CellAt<u32>> {
        let mut at = from;
        if let Some(m) = self.merge_at(from) {
            match direction {
                CellDirection::Up => at.row = m.top(),
                CellDirection::Down => at.row = m.bottom(),
                CellDirection::Left => at.column = m.left(),
                CellDirection::Right => at.column = m.right(),
            }
        }
        loop {
            match direction {
                CellDirection::Up => at.row = at.row.checked_sub(1)?,
                CellDirection::Down => at.row = at.row.checked_add(1).filter(|r| *r < self.row_total)?,
                CellDirection::Left => at.column = at.column.checked_sub(1)?,
                CellDirection::Right => at.column = at.column.checked_add(1).filter(|c| *c < self.columns)?,
            }
            let target = self.merge_at(at).map_or(at, |m| m.top_left());
            if self.visitable(target) && !self.is_hidden(at) {
                return Some(target);
            }
        }
    }

    /// A document table's Tab (InDesign, Word): the next cell along the row, then the first cell of the next row
    /// (`back`: the previous cell, then the last cell of the row above); a merged cell is visited once, at its
    /// top-left; hidden lines and unselectable cells are skipped. `None` past the last cell or before the first:
    /// InDesign and Word add a row there, which is the app's edit. A sheet's Tab is [`CellSelection::advance`],
    /// which stops at the sheet's edge.
    pub fn next_in_reading_order(&self, from: CellAt<u32>, back: bool) -> Option<CellAt<u32>> {
        self.scan(self.all()?, Some(from), CellAdvance::Tab, back)
    }

    fn is_hidden(&self, c: CellAt<u32>) -> bool {
        self.hidden_rows.is_some_and(|h| h(c.row)) || self.hidden_columns.is_some_and(|h| h(c.column))
    }

    fn is_selectable(&self, c: CellAt<u32>) -> bool {
        self.selectable.is_none_or(|s| s(c))
    }

    /// Whether a walk may stop at `c`: shown, selectable, and a merged cell only at its top-left.
    fn visitable(&self, c: CellAt<u32>) -> bool {
        !self.is_hidden(c) && self.merge_at(c).is_none_or(|m| m.top_left() == c) && self.is_selectable(c)
    }

    /// The next visitable cell after `from` (before it with `back`) inside `range`, walking along rows for Tab and
    /// down columns for Enter, wrapping to the next line; `from` `None` starts at the range's first (last) cell,
    /// which counts itself. A merged `from` is left from its far edge.
    fn scan(&self, range: CellRange<u32>, from: Option<CellAt<u32>>, along: CellAdvance, back: bool) -> Option<CellAt<u32>> {
        let pair = |c: CellAt<u32>| match along {
            CellAdvance::Tab => (i64::from(c.row), i64::from(c.column)),
            CellAdvance::Enter => (i64::from(c.column), i64::from(c.row)),
        };
        let cell = |(major, minor): (i64, i64)| {
            let (major, minor) = (major as u32, minor as u32);
            match along {
                CellAdvance::Tab => CellAt::new(major, minor),
                CellAdvance::Enter => CellAt::new(minor, major),
            }
        };
        let (major0, minor0) = pair(range.top_left());
        let (major1, minor1) = pair(CellAt::new(range.bottom(), range.right()));
        let step = if back { -1 } else { 1 };
        let (mut major, mut minor) = match from {
            None if back => (major1, minor1),
            None => (major0, minor0),
            Some(c) => {
                let (major, minor) = pair(c);
                let merge = self.merge_at(c);
                let (_, edge) = pair(merge.as_ref().map_or(c, |m| if back { m.top_left() } else { CellAt::new(m.bottom(), m.right()) }));
                (major, if merge.is_some() { edge } else { minor } + step)
            }
        };
        loop {
            if minor > minor1 {
                minor = minor0;
                major += 1;
            } else if minor < minor0 {
                minor = minor1;
                major -= 1;
            }
            if major > major1 || major < major0 {
                return None;
            }
            let c = cell((major, minor));
            if self.visitable(c) {
                return Some(c);
            }
            minor += step;
        }
    }
}

/// A selection by position, the form the rules work in.
struct Placed {
    ranges: Vec<CellRange<u32>>,
    current: usize,
    active: CellAt<u32>,
    anchor: CellAt<u32>,
    tab_run: Option<u32>,
}

impl Placed {
    /// `selection` by position. A range whose end left the order drops out; a selection whose current range is
    /// gone, or whose current range does not hold the active cell, collapses to the active cell. `None` for the
    /// empty selection and when the active cell left the order.
    fn of<K: Value>(selection: &CellSelection<K>, space: &CellSpace<'_, K>) -> Option<Placed> {
        let active = space.place(selection.active.as_ref()?)?;
        let anchor = selection.anchor.as_ref().and_then(|cell| space.place(cell)).unwrap_or(active);
        let mut ranges = Vec::new();
        let mut current = None;
        for (at, range) in selection.ranges.iter().enumerate() {
            if let Some(placed) = space.place_range(range) {
                if at == selection.current {
                    current = Some(ranges.len());
                }
                ranges.push(placed);
            }
        }
        match current.filter(|&at| ranges[at].contains(active)) {
            Some(current) => Some(Placed { ranges, current, active, anchor, tab_run: selection.tab_run }),
            None => Some(Placed::collapsed(active, space)),
        }
    }

    /// `c` alone, its merged cell whole with the active cell at the merge's top-left.
    fn collapsed<K: Value>(c: CellAt<u32>, space: &CellSpace<'_, K>) -> Placed {
        let range = space.merge_at(c).unwrap_or_else(|| CellRange::cell(c));
        let active = range.top_left();
        Placed { ranges: vec![range], current: 0, active, anchor: active, tab_run: None }
    }

    fn current(&self) -> &CellRange<u32> {
        &self.ranges[self.current]
    }

    /// The corner of the current range opposite the anchor: the end Shift+arrows move.
    fn head(&self) -> CellAt<u32> {
        let r = self.current();
        let row = if self.anchor.row == r.top() { r.bottom() } else { r.top() };
        let column = if self.anchor.column == r.left() { r.right() } else { r.left() };
        CellAt::new(row, column)
    }

    fn is_single_cell<K: Value>(&self, space: &CellSpace<'_, K>) -> bool {
        match self.ranges.as_slice() {
            [r] => r.is_single_cell() || space.merge_at(self.active).as_ref() == Some(r),
            _ => false,
        }
    }

    /// Shift+click and a drag: the current range runs from the anchor to `c`, covered over merges; the active cell
    /// stays unless it falls outside (Excel keeps it at the anchor).
    fn extend_to<K: Value>(&mut self, c: CellAt<u32>, space: &CellSpace<'_, K>) {
        let range = space.cover(CellRange::spanning(self.anchor, c));
        if !range.contains(self.active) {
            self.active = self.anchor;
        }
        let current = self.current;
        self.ranges[current] = range;
        self.tab_run = None;
    }
}

impl<K: Value> CellSelection<K> {
    /// The smallest range holding every selected range, by position; `None` for the empty selection.
    pub fn bounds(&self, space: &CellSpace<'_, K>) -> Option<CellRange<u32>> {
        let placed = Placed::of(self, space)?;
        Some(placed.ranges.iter().fold(CellRange::cell(placed.active), |a, b| a.union(b)))
    }

    /// The corner of the current range opposite the anchor: the end Shift+arrows move; `None` for the empty
    /// selection.
    pub fn head(&self, space: &CellSpace<'_, K>) -> Option<CellAt<u32>> {
        Some(Placed::of(self, space)?.head())
    }

    /// A click, or an arrow's landing cell: `c` alone (its merged cell whole, the active cell at the merge's
    /// top-left), active and the anchor. Ends a Tab run. Returns whether the selection changed.
    pub fn select(&mut self, c: CellAt<K>, space: &CellSpace<'_, K>) -> bool {
        let Some(c) = space.place(&c).filter(|c| space.is_selectable(*c)) else {
            return false;
        };
        self.store(Placed::collapsed(c, space), space)
    }

    /// A header click, a header drag or Select All: `range` alone, covered over merges unless it is whole lines
    /// ([`CellSpace::cover`]); the active cell and the anchor at its top-left. Ends a Tab run.
    pub fn select_range(&mut self, range: CellRange<K>, space: &CellSpace<'_, K>) -> bool {
        let Some(range) = space.place_range(&range) else {
            return false;
        };
        let range = space.cover(range);
        let active = range.top_left();
        self.store(Placed { ranges: vec![range], current: 0, active, anchor: active, tab_run: None }, space)
    }

    /// Shift+click and a drag: the current range runs from the anchor to `c`, covered over merges; the active cell
    /// stays (Excel keeps it at the anchor). On the empty selection it selects `c`. Ends a Tab run.
    pub fn extend_to(&mut self, c: CellAt<K>, space: &CellSpace<'_, K>) -> bool {
        let Some(mut placed) = Placed::of(self, space) else {
            return self.select(c, space);
        };
        let Some(c) = space.place(&c) else {
            return false;
        };
        placed.extend_to(c, space);
        self.store(placed, space)
    }

    /// Shift+arrow: the [`head`](Self::head) moves one cell toward `direction` through [`CellSpace::neighbour`] (rows
    /// and columns hidden are skipped), and the current range runs from the anchor to it, covered over merges.
    /// Nothing changes at the grid's edge. Ends a Tab run.
    pub fn extend(&mut self, direction: CellDirection, space: &CellSpace<'_, K>) -> bool {
        let Some(mut placed) = Placed::of(self, space) else {
            return false;
        };
        let Some(target) = space.neighbour(placed.head(), direction) else {
            return false;
        };
        placed.extend_to(target, space);
        self.store(placed, space)
    }

    /// Cmd+click (Ctrl+click) on a cell no range holds: a new range at `c` (its merged cell whole) after the others;
    /// `c` becomes active and the anchor. A drag from there extends the new range. A held cell is taken out by
    /// [`toggle`](Self::toggle) instead. Ends a Tab run.
    pub fn add(&mut self, c: CellAt<K>, space: &CellSpace<'_, K>) -> bool {
        let Some(placed) = Placed::of(self, space) else {
            return self.select(c, space);
        };
        let Some(c) = space.place(&c).filter(|c| space.is_selectable(*c)) else {
            return false;
        };
        let range = space.merge_at(c).unwrap_or_else(|| CellRange::cell(c));
        let active = range.top_left();
        let mut ranges = placed.ranges;
        ranges.push(range);
        self.store(Placed { current: ranges.len() - 1, ranges, active, anchor: active, tab_run: None }, space);
        true
    }

    /// Cmd+click (Ctrl+click) on any cell: a cell a range holds is taken out of the selection (and joins it again
    /// when it was taken out); any other cell joins as a new range ([`add`](Self::add)). It places
    /// the ranges, so it also takes out a cell between the rows of a range; the widget reports the result as
    /// [`SelectionChange::CellSet`].
    /// The cell becomes active and the anchor. Ends a Tab run.
    pub fn toggle(&mut self, c: CellAt<K>, space: &CellSpace<'_, K>) -> bool {
        let Some(at) = space.place(&c) else {
            return false;
        };
        if let Some(out) = self.toggled.iter().position(|cell| *cell == c) {
            self.toggled.remove(out);
        } else if self.placed(space.rows).holds_cell(at.row as usize, at.column) {
            self.toggled.push(c.clone());
        } else {
            return self.add(c, space);
        }
        self.tab_run = None;
        self.active = Some(c.clone());
        self.anchor = Some(c);
        true
    }

    /// An arrow: the selection collapses to the active cell's neighbour toward `direction`
    /// ([`CellSpace::neighbour`]), as [`select`](Self::select) does. At the grid's edge the selection collapses to
    /// the active cell. Ends a Tab run. A Cmd/Ctrl+arrow jump to the data edge is the widget's: it asks the app for
    /// the target and calls `select` or `extend_to`.
    pub fn step(&mut self, direction: CellDirection, space: &CellSpace<'_, K>) -> bool {
        let Some(placed) = Placed::of(self, space) else {
            return false;
        };
        let target = space.neighbour(placed.active, direction).unwrap_or(placed.active);
        self.store(Placed::collapsed(target, space), space)
    }

    /// Enter and Tab after an entry (`back` with Shift).
    ///
    /// - More than one cell selected: the active cell moves inside the current range, along the row for Tab and
    ///   down the column for Enter, wrapping to the next row or column; past the range's last cell it goes on to the
    ///   next range's first cell, after the last range back to the first (Excel cycles every range; `back` runs the
    ///   other way). Merged cells are visited once, hidden lines skipped. The selection is kept, with
    ///   [`ranges`](Self::ranges) in the same order; the current range moves to the one the active cell is in.
    /// - One cell: the selection moves one cell, Tab right and Enter down ([`step`](Self::step), stopping at the
    ///   sheet's edge). A Tab starts or continues a Tab run; Enter after a run goes down one row to the column the
    ///   run started in (Excel's "return to the starting column") when a walk may stop there (shown and
    ///   selectable), else to the plain Down neighbour. Either way it ends the run.
    ///
    /// Returns whether the active cell moved; `false` for the empty selection.
    pub fn advance(&mut self, along: CellAdvance, back: bool, space: &CellSpace<'_, K>) -> bool {
        let Some(mut placed) = Placed::of(self, space) else {
            return false;
        };
        let moved_from = placed.active;
        if !placed.is_single_cell(space) {
            placed.tab_run = None;
            placed.anchor = placed.active;
            let count = placed.ranges.len();
            let at = placed.current;
            let mut next = space.scan(placed.ranges[at].clone(), Some(placed.active), along, back).map(|c| (at, c));
            for k in 1..=count {
                if next.is_some() {
                    break;
                }
                let i = if back { (at + count - k % count) % count } else { (at + k) % count };
                next = space.scan(placed.ranges[i].clone(), None, along, back).map(|c| (i, c));
            }
            let Some((i, c)) = next else {
                self.store(placed, space);
                return false;
            };
            placed.current = i;
            placed.active = c;
            placed.anchor = c;
            self.store(placed, space);
            return c != moved_from;
        }
        let run = placed.tab_run;
        let target = match (along, back, run) {
            // The run's start column only when a walk may stop there; else the plain neighbour.
            (CellAdvance::Enter, false, Some(column)) => {
                let below = space.neighbour(placed.active, CellDirection::Down);
                below.map(|n| CellAt::new(n.row, column)).filter(|t| space.visitable(*t)).or(below)
            }
            (CellAdvance::Enter, false, None) => space.neighbour(placed.active, CellDirection::Down),
            (CellAdvance::Enter, true, _) => space.neighbour(placed.active, CellDirection::Up),
            (CellAdvance::Tab, false, _) => space.neighbour(placed.active, CellDirection::Right),
            (CellAdvance::Tab, true, _) => space.neighbour(placed.active, CellDirection::Left),
        };
        let Some(target) = target else {
            return false;
        };
        let mut moved = Placed::collapsed(target, space);
        if along == CellAdvance::Tab {
            moved.tab_run = Some(run.unwrap_or(moved_from.column));
        }
        let active = moved.active;
        self.store(moved, space);
        active != moved_from
    }

    /// The change that reports this selection: [`SelectionChange::Cells`] when one range says it (the anchor cell and
    /// the active cell at its opposite corners), [`SelectionChange::Clear`] for the empty selection, else
    /// [`SelectionChange::CellSet`].
    pub fn change(&self) -> SelectionChange<K> {
        let (Some(active), Some(anchor)) = (self.active.as_ref(), self.anchor.as_ref()) else {
            return SelectionChange::Clear;
        };
        if let [range] = self.ranges.as_slice()
            && self.toggled.is_empty()
            && self.tab_run.is_none()
        {
            let rows = (anchor.row == range.first && active.row == range.last) || (anchor.row == range.last && active.row == range.first);
            let ends = [range.columns.start, range.columns.end.saturating_sub(1)];
            let other_end = if active.column == range.columns.start { ends[1] } else { ends[0] };
            if rows && ends.contains(&active.column) && anchor.column == other_end && range.lead_column == active.column {
                let reported = CellRange::new(anchor.row.clone(), active.row.clone(), range.columns.clone(), active.column);
                return SelectionChange::Cells(reported);
            }
        }
        SelectionChange::CellSet(Box::new(self.clone()))
    }

    /// Replaces the selection by `placed`, keyed, with no toggled cells. Returns whether ranges, current, active,
    /// anchor or toggled cells changed.
    fn store(&mut self, placed: Placed, space: &CellSpace<'_, K>) -> bool {
        let next = CellSelection {
            ranges: placed.ranges.into_iter().map(|range| space.keyed_range(range)).collect(),
            current: placed.current,
            active: Some(space.keyed(placed.active)),
            anchor: Some(space.keyed(placed.anchor)),
            tab_run: placed.tab_run,
            toggled: Vec::new(),
        };
        let changed = next.ranges != self.ranges
            || next.current != self.current
            || next.active != self.active
            || next.anchor != self.anchor
            || !self.toggled.is_empty();
        *self = next;
        changed
    }
}
