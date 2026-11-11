//! Item geometry: where each item of a collection sits for a scroll offset and viewport.

use crate::columns::{ColumnKey, GridSection, GridView};
use crate::item_source::{DisplayOrder, ItemSource, SizeHint};
use crate::nav::NavShape;
use crate::scroll::ScrollPos;
use emath::{Pos2, Rect, Vec2};
use std::collections::BTreeMap;
use std::ops::{Range, RangeInclusive};

/// Item geometry in content units. Offsets are `f64`, so a sheet of a million rows places its last row
/// exactly. Every rect and point the trait takes or returns is `f32` and relative to the viewport's top-left,
/// in logical coordinates: x runs from the start edge, as `ScrollPos.x` does. The item step maps rects, cells,
/// the grid view and the hit point through the collection's flow when `mirrors()` is true.
pub trait ItemLayout {
    /// False for a layout whose positions are document data (`FreeLayout`); the item step then leaves them as given.
    fn mirrors(&self) -> bool {
        true
    }

    /// Finite and non-negative.
    fn content_size(&self) -> ScrollPos;

    /// The indices of the items that intersect a viewport of size `view` scrolled to `offset`, ascending.
    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>);

    /// The rect of item `at`.
    fn rect(&self, at: usize, offset: ScrollPos) -> Rect;

    /// The item under `point`.
    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize>;

    /// How arrow keys move through the items.
    fn nav(&self) -> NavShape;

    /// `NavShape::RaggedGrid`: the number of items in `row`, rows holding consecutive items and a run of hidden items
    /// counting as a row of its own; `None` past the last row.
    fn row_len(&self, _row: usize) -> Option<usize> {
        None
    }

    /// Whether item `at` takes a place in the layout; a hidden item (in a closed group) cannot take the lead.
    fn shown(&self, _at: usize) -> bool {
        true
    }

    /// The cells of item `at`, with their rects.
    fn cells(&self, _at: usize, _offset: ScrollPos, _out: &mut Vec<(ColumnKey, Rect)>) {}

    /// Column virtualisation and sections of a sheet.
    fn grid(&self) -> Option<&GridView> {
        None
    }

    /// The height at the top of the view that item `at` must not paint into: the zoomed frozen-row band for a scrolled
    /// row, zero for a frozen row and for a layout without frozen rows.
    fn frozen_band(&self, _at: usize) -> f32 {
        0.0
    }

    /// The first item that is not frozen: where Ctrl or Cmd with Home goes. Zero for a layout without frozen rows.
    fn first_navigable(&self) -> usize {
        0
    }

    /// The column widths the collection resolved, from the start edge; layouts without columns ignore them.
    fn set_columns(&mut self, _columns: &[(ColumnKey, f32)]) {}

    /// The number of columns of a table or sheet; 0 for a layout with none.
    fn column_count(&self) -> usize {
        0
    }

    /// Whether the column at position `index` is shown; a hidden column has no cell and the arrows skip it.
    fn column_shown(&self, _index: usize) -> bool {
        true
    }

    /// The key of the column at position `index`.
    fn column_key(&self, _index: usize) -> Option<ColumnKey> {
        None
    }

    /// The position of the column under `point`.
    fn column_at(&self, _point: Pos2, _offset: ScrollPos) -> Option<usize> {
        None
    }

    /// Whether item `at` is as tall as it measures itself.
    fn is_auto(&self, _at: usize) -> bool {
        false
    }

    /// Takes the measured heights of the auto items: `heights(at)` is `None` before the item's first measure.
    fn measured(&mut self, _heights: &dyn Fn(usize) -> Option<f32>) {}

    /// An auto item measured `height` in this pass; the items after it move.
    fn set_height(&mut self, _at: usize, _height: f32) {}

    /// Whether the items are as wide as they measure themselves. A layout that asks for widths places the items
    /// from their measured ideal and minimum widths (`set_width`), so every item must be built and measured: its
    /// `visible` lists all items, and the collection does not virtualise it.
    fn auto_widths(&self) -> bool {
        false
    }

    /// An auto-width item measured `ideal` and `min` width. The values of the previous pass arrive before
    /// placement and the current pass's during it; the items after it move.
    fn set_width(&mut self, _at: usize, _ideal: f32, _min: f32) {}

    /// The viewport and scroll offset the grid view describes. Layouts without a grid ignore it.
    fn set_view(&mut self, _viewport: Vec2, _offset: ScrollPos) {}
}

/// Lines (rows or columns) with a default size, the lines whose size differs and their prefix sums, so the start
/// of a line and the line at a position are a binary search over the differing lines only: a sheet of 1,048,576
/// rows or 16,384 columns of which a few differ stores a few entries. Starts and sizes are `f64`, so the last of a
/// million rows places exactly. A size of `0.0` hides the line. Sizes are in points at 100 % zoom; the owner of a
/// zoom multiplies them.
///
/// An index may hold no line (`count() == 0`); `at` and `span` then find nothing.
#[derive(Clone, Debug, PartialEq)]
pub struct LineIndex {
    default: f64,
    count: u32,
    /// (line, its start, its size), sorted by line.
    lines: Vec<(u32, f64, f64)>,
}

impl LineIndex {
    /// `count` lines of `size` each (negative or NaN sizes count as `0.0`).
    pub fn uniform(size: f32, count: u32) -> Self {
        Self::from_sorted(size, count, std::iter::empty())
    }

    /// `custom`: the lines whose size isn't `default` (`0.0` for a hidden line); `count` lines in all (1,048,576
    /// rows and 16,384 columns in Excel). Lines at or past `count` are ignored.
    pub fn new(custom: &BTreeMap<u32, f32>, default: f32, count: u32) -> Self {
        Self::from_sorted(default, count, custom.iter().map(|(&line, &size)| (line, size)))
    }

    /// One line per size, in order; the lines at `default` are not stored.
    fn from_sizes(sizes: impl IntoIterator<Item = f32>, default: f32) -> Self {
        let default_size = f64::from(default.max(0.0));
        let mut custom = Vec::new();
        let mut count = 0u32;
        for size in sizes {
            if f64::from(size.max(0.0)) != default_size {
                custom.push((count, size));
            }
            let Some(next) = count.checked_add(1) else { break };
            count = next;
        }
        Self::from_sorted(default, count, custom)
    }

    /// `custom` is sorted by line.
    fn from_sorted(default: f32, count: u32, custom: impl IntoIterator<Item = (u32, f32)>) -> Self {
        let mut index = Self { default: f64::from(default.max(0.0)), count, lines: Vec::new() };
        index.lines.extend(
            custom
                .into_iter()
                .take_while(|&(line, _)| line < count)
                .map(|(line, size)| (line, 0.0, f64::from(size.max(0.0))))
                .filter(|&(_, _, size)| size != index.default),
        );
        index.restart(0);
        index
    }

    /// Recomputes the starts of the stored lines from entry `from` on.
    fn restart(&mut self, from: usize) {
        let default = self.default;
        let mut before = from.checked_sub(1).and_then(|k| self.lines.get(k)).copied();
        for entry in self.lines.iter_mut().skip(from) {
            entry.1 = match before {
                Some((line, start, size)) => start + size + f64::from(entry.0 - line - 1) * default,
                None => f64::from(entry.0) * default,
            };
            before = Some(*entry);
        }
    }

    /// Gives `line` the size `size`; the lines after it move. A line at or past `count` is ignored.
    pub fn set(&mut self, line: u32, size: f32) {
        if line >= self.count {
            return;
        }
        let size = f64::from(size.max(0.0));
        let start = self.start(line);
        match self.lines.binary_search_by(|entry| entry.0.cmp(&line)) {
            Ok(i) if size == self.default => {
                self.lines.remove(i);
                self.restart(i);
            }
            Ok(i) => {
                self.lines[i].2 = size;
                self.restart(i + 1);
            }
            Err(_) if size == self.default => {}
            Err(i) => {
                self.lines.insert(i, (line, start, size));
                self.restart(i + 1);
            }
        }
    }

    /// `set` for many lines at once: one restart instead of one per line.
    fn set_all(&mut self, changes: &[(u32, f32)]) {
        let mut sizes: BTreeMap<u32, f64> = self.lines.iter().map(|&(line, _, size)| (line, size)).collect();
        for &(line, size) in changes.iter().filter(|(line, _)| *line < self.count) {
            sizes.insert(line, f64::from(size.max(0.0)));
        }
        let default = self.default;
        self.lines = sizes.into_iter().filter(|&(_, size)| size != default).map(|(line, size)| (line, 0.0, size)).collect();
        self.restart(0);
    }

    /// The start of `line`: the sum of the sizes before it. A line past the last one continues at the default size.
    pub fn start(&self, line: u32) -> f64 {
        match self.lines.binary_search_by(|entry| entry.0.cmp(&line)) {
            Ok(i) => self.lines.get(i).map_or(0.0, |entry| entry.1),
            Err(i) => match i.checked_sub(1).and_then(|k| self.lines.get(k)) {
                Some(&(before, start, size)) => start + size + f64::from(line - before - 1) * self.default,
                None => f64::from(line) * self.default,
            },
        }
    }

    /// The end of `line`: its start plus its size.
    pub fn end(&self, line: u32) -> f64 {
        self.start(line) + self.size(line)
    }

    /// The size of `line`; `0.0` for a hidden line, the default for a line with no custom size.
    pub fn size(&self, line: u32) -> f64 {
        match self.lines.binary_search_by(|entry| entry.0.cmp(&line)) {
            Ok(i) => self.lines.get(i).map_or(self.default, |entry| entry.2),
            Err(_) => self.default,
        }
    }

    /// Whether `line` exists and has a size above zero.
    pub fn shown(&self, line: u32) -> bool {
        line < self.count && self.size(line) > 0.0
    }

    /// The line holding `pos`: line 0 for a negative or NaN position, the last line past the end. A hidden line
    /// holds no position; the next shown line is returned instead.
    pub fn at(&self, pos: f64) -> u32 {
        if pos.is_nan() || pos <= 0.0 {
            return 0;
        }
        let last = self.count.saturating_sub(1);
        let after = self.lines.partition_point(|entry| entry.1 <= pos);
        let (first, first_start) = match after.checked_sub(1).and_then(|k| self.lines.get(k)) {
            Some(&(line, start, size)) if pos < start + size => return line.min(last),
            Some(&(line, start, size)) => (line + 1, start + size),
            None => (0, 0.0),
        };
        if self.default <= 0.0 {
            return first.min(last);
        }
        let line = f64::from(first) + ((pos - first_start) / self.default).floor();
        if line >= f64::from(self.count) { last } else { line as u32 }
    }

    /// The lines that intersect `from..to`, ascending. A hidden line inside the range is in the range and has no
    /// extent of its own; one at either end is not. Empty for an empty or reversed range and past the last line.
    pub fn span(&self, from: f64, to: f64) -> Range<u32> {
        if from.is_nan() || to.is_nan() || to <= from {
            return 0..0;
        }
        let from = from.max(0.0);
        if from >= self.total() {
            return 0..0;
        }
        let first = self.at(from);
        let last = self.starting_before(to);
        first..last.max(first)
    }

    /// The number of lines that start before `pos`.
    fn starting_before(&self, pos: f64) -> u32 {
        let after = self.lines.partition_point(|entry| entry.1 < pos);
        let (first, first_start) = match after.checked_sub(1).and_then(|k| self.lines.get(k)) {
            Some(&(line, start, size)) => (line + 1, start + size),
            None => (0, 0.0),
        };
        if first_start >= pos {
            return first;
        }
        let next = self.lines.get(after).map_or(self.count, |entry| entry.0);
        if self.default <= 0.0 {
            return next;
        }
        let between = ((pos - first_start) / self.default).ceil();
        (f64::from(first) + between).min(f64::from(next)) as u32
    }

    /// The number of lines.
    pub fn count(&self) -> u32 {
        self.count
    }

    /// The sum of all sizes: where the line after the last would start.
    pub fn total(&self) -> f64 {
        self.start(self.count)
    }
}

/// The line number of item `at`; items past `u32::MAX` clamp there.
fn line_of(at: usize) -> u32 {
    u32::try_from(at).unwrap_or(u32::MAX)
}

/// The rows of a `SpanLayout::sized` layout: their extents, and which of them take their measured height.
#[derive(Clone, Debug)]
struct Sized {
    lines: LineIndex,
    auto: Vec<bool>,
}

/// Rows of equal height, optionally split into columns: lists, popup lists, tab strips and timeline tracks.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SpanLayout {
    /// Height of one row.
    pub row_h: f32,
    /// Number of rows.
    pub count: usize,
    /// Width of one row.
    pub width: f32,
    /// The columns with their widths, from the start edge.
    pub columns: Vec<(ColumnKey, f32)>,
    /// Whether the rows form a tree: arrows expand, collapse and step to the parent.
    pub tree: bool,
    rows: Option<Sized>,
}

impl SpanLayout {
    /// One row of `row_h` per entry of `source`, `width` wide, with no columns.
    pub fn new<K>(source: &dyn DisplayOrder<K>, width: f32, row_h: f32) -> Self {
        Self { row_h, count: source.len(), width, columns: Vec::new(), tree: false, rows: None }
    }

    /// Rows as tall as `source` says: `Height` and `Rect` are the app's, `Aspect` follows the width, and `Auto`
    /// rows take `row_h` until the item step feeds their measured height.
    pub fn sized<K>(source: &dyn ItemSource<K>, width: f32, row_h: f32) -> Self {
        let hints: Vec<SizeHint> = (0..source.len()).map(|at| source.size_hint(at)).collect();
        let lines = LineIndex::from_sizes(
            hints.iter().map(|hint| match *hint {
                SizeHint::Height(height) => height,
                SizeHint::Rect(size) => size.y,
                SizeHint::Aspect(aspect) if aspect > 0.0 => width / aspect,
                _ => row_h,
            }),
            row_h,
        );
        let auto = hints.iter().map(|hint| matches!(hint, SizeHint::Auto)).collect();
        Self { row_h, count: hints.len(), width, columns: Vec::new(), tree: false, rows: Some(Sized { lines, auto }) }
    }

    fn top(&self, at: usize) -> f64 {
        match &self.rows {
            Some(rows) => rows.lines.start(line_of(at.min(self.count))),
            None => at as f64 * f64::from(self.row_h),
        }
    }

    fn height(&self, at: usize) -> f32 {
        match &self.rows {
            Some(rows) if at < self.count => rows.lines.size(line_of(at)) as f32,
            _ => self.row_h,
        }
    }

    /// The columns with their widths, from the start edge.
    #[must_use]
    pub fn columns(mut self, columns: Vec<(ColumnKey, f32)>) -> Self {
        self.columns = columns;
        self
    }

    /// Rows that expand and collapse.
    #[must_use]
    pub fn tree(mut self) -> Self {
        self.tree = true;
        self
    }
}

impl ItemLayout for SpanLayout {
    fn content_size(&self) -> ScrollPos {
        ScrollPos { x: f64::from(self.width), y: self.top(self.count) }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        let bottom = offset.y + f64::from(view.y);
        if let Some(rows) = &self.rows {
            let span = rows.lines.span(offset.y, bottom);
            out.extend(span.start as usize..(span.end as usize).min(self.count));
            return;
        }
        let h = f64::from(self.row_h);
        let first = (offset.y / h).floor().max(0.0) as usize;
        let last = ((bottom / h).ceil().max(0.0) as usize).min(self.count);
        out.extend(first..last);
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let top = (self.top(at) - offset.y) as f32;
        Rect::from_min_size(Pos2::new(-(offset.x as f32), top), Vec2::new(self.width, self.height(at)))
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        let y = f64::from(point.y) + offset.y;
        if let Some(rows) = &self.rows {
            return (0.0..rows.lines.total()).contains(&y).then(|| rows.lines.at(y) as usize).filter(|&at| at < self.count);
        }
        let at = (y / f64::from(self.row_h)).floor();
        (at >= 0.0 && (at as usize) < self.count).then_some(at as usize)
    }

    fn column_count(&self) -> usize {
        self.columns.len()
    }

    fn column_key(&self, index: usize) -> Option<ColumnKey> {
        self.columns.get(index).map(|(key, _)| key.clone())
    }

    fn column_at(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        column_under(self.columns.iter().map(|(_, width)| *width), f64::from(point.x) + offset.x)
    }

    fn is_auto(&self, at: usize) -> bool {
        self.rows.as_ref().is_some_and(|rows| rows.auto.get(at).copied().unwrap_or(false))
    }

    fn measured(&mut self, heights: &dyn Fn(usize) -> Option<f32>) {
        if let Some(rows) = &mut self.rows {
            let changes: Vec<(u32, f32)> =
                (0..rows.auto.len()).filter(|&at| rows.auto[at]).filter_map(|at| heights(at).map(|height| (line_of(at), height))).collect();
            rows.lines.set_all(&changes);
        }
    }

    fn set_height(&mut self, at: usize, height: f32) {
        if let Some(rows) = &mut self.rows
            && rows.auto.get(at).copied().unwrap_or(false)
        {
            rows.lines.set(line_of(at), height);
        }
    }

    fn nav(&self) -> NavShape {
        if self.tree { NavShape::Tree } else { NavShape::Flat }
    }

    fn set_columns(&mut self, columns: &[(ColumnKey, f32)]) {
        self.columns = columns.to_vec();
    }

    fn cells(&self, at: usize, offset: ScrollPos, out: &mut Vec<(ColumnKey, Rect)>) {
        let row = self.rect(at, offset);
        let mut along = 0.0;
        for (key, w) in &self.columns {
            let x = row.left() + along;
            out.push((key.clone(), Rect::from_min_size(Pos2::new(x, row.top()), Vec2::new(*w, row.height()))));
            along += w;
        }
    }
}

/// The position of the column holding content-x `x`, for columns of `widths` from the start edge.
fn column_under(widths: impl Iterator<Item = f32>, x: f64) -> Option<usize> {
    let mut left = 0.0;
    for (index, width) in widths.enumerate() {
        left += f64::from(width);
        if x < left {
            return (x >= 0.0).then_some(index);
        }
    }
    None
}

/// Equal tiles that wrap at the width: tile grids, preset grids, font cards and swatch grids.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct TileLayout {
    /// The size of one tile.
    pub tile: Vec2,
    /// The space between tiles, on both axes.
    pub gap: f32,
    /// Number of tiles.
    pub count: usize,
    /// The width the tiles wrap in.
    pub width: f32,
    /// A column count the user set, which replaces the count the width allows.
    pub fixed_columns: Option<usize>,
}

impl TileLayout {
    /// One tile of `tile` per entry of `source`, `gap` apart, wrapping at `width`.
    pub fn new<K>(source: &dyn DisplayOrder<K>, width: f32, tile: Vec2, gap: f32) -> Self {
        Self { tile, gap, count: source.len(), width, fixed_columns: None }
    }

    /// Exactly `columns` tiles per row, whatever the width (a user setting; at least one).
    pub fn in_columns(mut self, columns: usize) -> Self {
        self.fixed_columns = Some(columns.max(1));
        self
    }

    /// Tiles per row: the user's count, else what the width allows; at least one.
    pub fn columns(&self) -> usize {
        if let Some(columns) = self.fixed_columns {
            return columns;
        }
        (((self.width + self.gap) / (self.tile.x + self.gap)).floor() as usize).max(1)
    }

    fn rows(&self) -> usize {
        self.count.div_ceil(self.columns())
    }

    fn pitch(&self) -> Vec2 {
        Vec2::new(self.tile.x + self.gap, self.tile.y + self.gap)
    }
}

impl ItemLayout for TileLayout {
    fn content_size(&self) -> ScrollPos {
        let rows = self.rows() as f64;
        let height = if rows == 0.0 { 0.0 } else { rows * f64::from(self.pitch().y) - f64::from(self.gap) };
        ScrollPos { x: f64::from(self.width), y: height }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        let pitch = f64::from(self.pitch().y);
        let first = (offset.y / pitch).floor().max(0.0) as usize;
        let last = (((offset.y + f64::from(view.y)) / pitch).ceil().max(0.0) as usize).min(self.rows());
        let columns = self.columns();
        out.extend(first * columns..(last * columns).min(self.count));
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let columns = self.columns();
        let pitch = self.pitch();
        let x = (at % columns) as f32 * pitch.x - offset.x as f32;
        let y = ((at / columns) as f64 * f64::from(pitch.y) - offset.y) as f32;
        Rect::from_min_size(Pos2::new(x, y), self.tile)
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        let pitch = self.pitch();
        let (x, y) = (f64::from(point.x) + offset.x, f64::from(point.y) + offset.y);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        let (column, row) = ((x / f64::from(pitch.x)) as usize, (y / f64::from(pitch.y)) as usize);
        let inside = x - column as f64 * f64::from(pitch.x) < f64::from(self.tile.x) && y - row as f64 * f64::from(pitch.y) < f64::from(self.tile.y);
        let at = row * self.columns() + column;
        (inside && column < self.columns() && at < self.count).then_some(at)
    }

    fn nav(&self) -> NavShape {
        NavShape::wrapped_grid(self.columns())
    }
}

/// The least and greatest zoom of a sheet (10 %…400 %).
pub const SHEET_ZOOM: RangeInclusive<f32> = 0.1..=4.0;

/// Rows of a sheet: one position per row, each split into the sheet's columns, over one [`LineIndex`] for the rows
/// and one for the columns. A row or column has its own size at 100 %, and a size of `0.0` hides it. The first
/// rows and columns can stay in view while the others scroll, and everything is multiplied by the zoom
/// ([`SHEET_ZOOM`]). The line indexes are the only owner of line positions: a million rows by 16,384 columns place
/// and hit exactly, because rows and columns are computed in `f64` and only the lines in the viewport are visited.
///
/// The scroll offset is in zoomed content units and counts the frozen bands: a scrolled row or column with the
/// content position `p` sits at `p - offset`, so it slides under the frozen band when it passes it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SheetLayout {
    rows: LineIndex,
    columns: LineIndex,
    /// The keys of a sheet built with `new`; a sheet built with `lines` keys its columns by position.
    keys: Option<Vec<ColumnKey>>,
    frozen_columns: u32,
    frozen_rows: u32,
    zoom: f32,
    grid: Option<GridView>,
}

impl SheetLayout {
    /// A row of `row_h` per entry of `source`, split into `columns`, each with its key and width.
    pub fn new<K>(source: &dyn DisplayOrder<K>, row_h: f32, columns: Vec<(ColumnKey, f32)>) -> Self {
        let default = columns.first().map_or(0.0, |(_, width)| *width);
        let widths = LineIndex::from_sizes(columns.iter().map(|(_, width)| *width), default);
        let keys = columns.into_iter().map(|(key, _)| key).collect();
        let rows = LineIndex::uniform(row_h, line_of(source.len()));
        Self { rows, columns: widths, keys: Some(keys), frozen_columns: 0, frozen_rows: 0, zoom: 1.0, grid: None }
    }

    /// A sheet with the size of every row and column in `rows` and `columns`, at 100 % zoom. A column's key is
    /// [`ColumnKey::index`] of its position.
    pub fn lines(rows: LineIndex, columns: LineIndex) -> Self {
        Self { rows, columns, keys: None, frozen_columns: 0, frozen_rows: 0, zoom: 1.0, grid: None }
    }

    /// The first `count` columns stay in view while the others scroll.
    #[must_use]
    pub fn frozen(mut self, count: usize) -> Self {
        self.frozen_columns = line_of(count);
        self
    }

    /// The first `count` rows stay in view while the others scroll. Hidden rows among them take no room.
    #[must_use]
    pub fn frozen_rows(mut self, count: u32) -> Self {
        self.frozen_rows = count;
        self
    }

    /// Every row height and column width is multiplied by `zoom`, clamped to [`SHEET_ZOOM`].
    #[must_use]
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom.max(*SHEET_ZOOM.start()).min(*SHEET_ZOOM.end());
        self
    }

    /// The row extents at 100 %.
    pub fn rows(&self) -> &LineIndex {
        &self.rows
    }

    /// The column extents at 100 %.
    pub fn columns(&self) -> &LineIndex {
        &self.columns
    }

    fn zoom_factor(&self) -> f64 {
        f64::from(self.zoom)
    }

    /// The zoomed height of the frozen rows.
    fn band(&self) -> f64 {
        self.rows.start(self.frozen_rows.min(self.rows.count())) * self.zoom_factor()
    }

    /// The zoomed width of the frozen columns.
    fn frozen_width(&self) -> f64 {
        self.columns.start(self.frozen_columns.min(self.columns.count())) * self.zoom_factor()
    }

    /// The row at the zoomed content position `y`, if there is a shown one.
    fn row_at(&self, y: f64) -> Option<usize> {
        let at = y / self.zoom_factor();
        (0.0..self.rows.total()).contains(&at).then(|| self.rows.at(at)).filter(|&row| self.rows.shown(row)).map(|row| row as usize)
    }

    /// The sections of a viewport scrolled to `offset_x`: the frozen columns at the start edge, then the scrolling
    /// columns that intersect the rest of the viewport. Edges are viewport-relative x positions, for those columns
    /// only; a hidden column among them has the same edge on both sides.
    fn sections(&self, viewport: Vec2, offset_x: f64) -> GridView {
        let zoom = self.zoom_factor();
        let edge = |column: u32, shift: f64| (self.columns.start(column) * zoom - shift) as f32;
        let frozen = self.frozen_columns.min(self.columns.count());
        let frozen_w = edge(frozen, 0.0);
        let mut sections = Vec::new();
        if frozen > 0 {
            let rect = Rect::from_min_size(Pos2::ZERO, Vec2::new(frozen_w, viewport.y));
            sections.push(GridSection::new(rect, 0..frozen, (0..=frozen).map(|column| edge(column, 0.0)).collect()));
        }
        let span = self.columns.span((offset_x + f64::from(frozen_w)) / zoom, (offset_x + f64::from(viewport.x)) / zoom);
        let first = span.start.max(frozen);
        if first < span.end {
            let edges: Vec<f32> = (first..=span.end).map(|column| edge(column, offset_x)).collect();
            let (start, stop) = (edges[0].max(frozen_w), edges[edges.len() - 1].min(viewport.x));
            let rect = Rect::from_min_max(Pos2::new(start, 0.0), Pos2::new(stop.max(start), viewport.y));
            sections.push(GridSection::new(rect, first..span.end, edges));
        }
        GridView::new(sections)
    }
}

impl ItemLayout for SheetLayout {
    fn content_size(&self) -> ScrollPos {
        let zoom = self.zoom_factor();
        ScrollPos { x: self.columns.total() * zoom, y: self.rows.total() * zoom }
    }

    /// The shown frozen rows, then the shown scrolled rows that intersect the viewport below the frozen band.
    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        let frozen = self.frozen_rows.min(self.rows.count());
        out.extend((0..frozen).filter(|&row| self.rows.shown(row)).map(|row| row as usize));
        let zoom = self.zoom_factor();
        let span = self.rows.span((offset.y + self.band()) / zoom, (offset.y + f64::from(view.y)) / zoom);
        out.extend((span.start.max(frozen)..span.end).filter(|&row| self.rows.shown(row)).map(|row| row as usize));
    }

    /// A frozen row ignores `offset.y`; a scrolled row sits at its content position minus the offset.
    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let (row, zoom) = (line_of(at), self.zoom_factor());
        let content = self.rows.start(row) * zoom;
        let top = if row < self.frozen_rows { content } else { content - offset.y };
        let height = (self.rows.size(row) * zoom) as f32;
        // A row spans the columns the view shows, from the view's left edge: a row as wide as the whole sheet (16,384
        // columns) is never a row's rect. Before a view is set the view is empty and the row has no width.
        let right = self.grid.as_ref().and_then(|grid| grid.sections.last()).and_then(|section| section.edges.last()).copied().unwrap_or(0.0);
        Rect::from_min_size(Pos2::new(0.0, top as f32), Vec2::new(right, height))
    }

    /// The frozen band is checked first: a point in it finds the frozen row under it, whatever the scroll.
    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        let y = f64::from(point.y);
        self.row_at(if self.frozen_rows > 0 && y < self.band() { y } else { y + offset.y })
    }

    fn shown(&self, at: usize) -> bool {
        self.rows.shown(line_of(at))
    }

    fn nav(&self) -> NavShape {
        NavShape::Cell
    }

    /// The cells of the shown columns in the grid view; all shown columns before a view was set. A hidden column has
    /// no cell.
    fn cells(&self, at: usize, offset: ScrollPos, out: &mut Vec<(ColumnKey, Rect)>) {
        let row = self.rect(at, offset);
        let mut cell = |column: u32, left: f32, right: f32| {
            if self.columns.shown(column)
                && let Some(key) = self.column_key(column as usize)
            {
                out.push((key, Rect::from_min_max(Pos2::new(left, row.top()), Pos2::new(right, row.bottom()))));
            }
        };
        if let Some(grid) = &self.grid {
            for section in &grid.sections {
                for (edge, column) in section.columns.clone().enumerate() {
                    cell(column, section.edges[edge], section.edges[edge + 1]);
                }
            }
            return;
        }
        let zoom = self.zoom_factor();
        for column in 0..self.columns.count() {
            let (left, right) = (self.columns.start(column) * zoom, self.columns.end(column) * zoom);
            cell(column, row.left() + left as f32, row.left() + right as f32);
        }
    }

    fn grid(&self) -> Option<&GridView> {
        self.grid.as_ref()
    }

    fn first_navigable(&self) -> usize {
        (self.frozen_rows as usize).min(self.rows.count() as usize)
    }

    fn frozen_band(&self, at: usize) -> f32 {
        if at < self.frozen_rows as usize { 0.0 } else { self.band() as f32 }
    }

    fn set_view(&mut self, viewport: Vec2, offset: ScrollPos) {
        self.grid = Some(self.sections(viewport, offset.x));
    }

    fn column_count(&self) -> usize {
        self.columns.count() as usize
    }

    fn column_shown(&self, index: usize) -> bool {
        u32::try_from(index).is_ok_and(|index| self.columns.shown(index))
    }

    fn column_key(&self, index: usize) -> Option<ColumnKey> {
        match &self.keys {
            Some(keys) => keys.get(index).cloned(),
            None => (index < self.column_count()).then(|| ColumnKey::index(line_of(index))),
        }
    }

    fn column_at(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        let x = if self.frozen_columns > 0 && f64::from(point.x) < self.frozen_width() { f64::from(point.x) } else { f64::from(point.x) + offset.x };
        let at = x / self.zoom_factor();
        (0.0..self.columns.total())
            .contains(&at)
            .then(|| self.columns.at(at))
            .filter(|&column| self.columns.shown(column))
            .map(|column| column as usize)
    }
}

/// Items at positions the document stores: a freeform bin or a node graph.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct FreeLayout {
    /// One rect per item, in content coordinates.
    pub rects: Vec<Rect>,
}

impl FreeLayout {
    /// Items at `rects`, in content coordinates.
    pub fn new(rects: Vec<Rect>) -> Self {
        Self { rects }
    }
}

impl ItemLayout for FreeLayout {
    fn mirrors(&self) -> bool {
        false
    }

    fn content_size(&self) -> ScrollPos {
        let size = self.rects.iter().fold(Rect::NOTHING, |all, r| all.union(*r)).size();
        ScrollPos { x: f64::from(size.x.max(0.0)), y: f64::from(size.y.max(0.0)) }
    }

    fn visible(&self, offset: ScrollPos, view: Vec2, out: &mut Vec<usize>) {
        let viewport = Rect::from_min_size(Pos2::ZERO, view);
        out.extend((0..self.rects.len()).filter(|&i| self.rect(i, offset).intersects(viewport)));
    }

    fn rect(&self, at: usize, offset: ScrollPos) -> Rect {
        let shift = Vec2::new(-(offset.x as f32), -(offset.y as f32));
        self.rects.get(at).map_or(Rect::NOTHING, |r| r.translate(shift))
    }

    fn hit(&self, point: Pos2, offset: ScrollPos) -> Option<usize> {
        (0..self.rects.len()).rev().find(|&i| self.rect(i, offset).contains(point))
    }

    fn nav(&self) -> NavShape {
        NavShape::Spatial
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sheet(rows: LineIndex, columns: LineIndex) -> SheetLayout {
        SheetLayout::lines(rows, columns)
    }

    #[test]
    fn a_full_size_sheet_with_a_tall_and_a_hidden_line_places_and_hits_its_last_cell_exactly() {
        let rows = LineIndex::new(&BTreeMap::from([(5, 60.0), (7, 0.0)]), 20.0, 1_048_576);
        let columns = LineIndex::new(&BTreeMap::from([(3, 0.0)]), 80.0, 16_384);
        let mut layout = sheet(rows, columns);
        let (total, view) = (layout.content_size(), Vec2::new(200.0, 100.0));
        assert_eq!((total.y, total.x), (20_971_540.0, 1_310_640.0));
        let offset = ScrollPos { x: total.x - 200.0, y: total.y - 100.0 };
        layout.set_view(view, offset);
        assert_eq!(layout.rect(1_048_575, offset).top(), 80.0);
        assert_eq!(layout.hit(Pos2::new(10.0, 90.0), offset), Some(1_048_575));
        assert_eq!(layout.column_at(Pos2::new(190.0, 90.0), offset), Some(16_383));
        let mut cells = Vec::new();
        layout.cells(1_048_575, offset, &mut cells);
        let last = cells.last().expect("the viewport holds cells");
        assert_eq!((last.0.clone(), last.1.left(), last.1.width()), (ColumnKey::index(16_383), 120.0, 80.0));
        let mut shown = Vec::new();
        layout.visible(offset, view, &mut shown);
        assert_eq!(shown, (1_048_571..1_048_576).collect::<Vec<_>>(), "only the viewport's rows");
        shown.clear();
        layout.visible(ScrollPos { x: 0.0, y: 170.0 }, Vec2::new(100.0, 30.0), &mut shown);
        assert_eq!(shown, vec![6, 8], "the hidden row takes no place");
        assert!(!layout.shown(7) && layout.shown(6));
        assert_eq!(layout.hit(Pos2::new(1.0, 5.0), ScrollPos { x: 0.0, y: 180.0 }), Some(8));
    }
}
