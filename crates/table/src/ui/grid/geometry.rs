//! The sheet's geometry: metrics, layout, the pass geometry and the header insets.

use super::edges;
use craft_behavior::{CellAt, CellRange, Insets, LineIndex, SHEET_ZOOM};
use egui::{Pos2, Rect, Vec2};

/// The least zoom the column headers' height follows: below it they stay at this share of their 100 % height, so the
/// letters stay readable. A proportion, not a size.
pub const HEADER_MIN_ZOOM: f32 = 0.75;

/// How far below the window (points at 100 %) the row headers look for the last row whose digits they must hold, so
/// the header does not resize while scrolling.
pub(super) const ROW_HEADER_LOOKAHEAD: f64 = 200.0;

/// The digits the row headers hold for rows up to `last` (at least two).
pub(crate) fn digits_of(last: u32) -> u32 {
    (last.saturating_add(1).ilog10() + 1).max(2)
}

/// The header sizes geometry needs, taken from the grid's style.
#[derive(Clone, Copy, Debug, PartialEq)]
#[non_exhaustive]
pub struct SheetMetrics {
    /// The column headers' height at 100 % zoom.
    pub column_header: f32,
    /// The width of one digit of the row headers at 100 % zoom.
    pub row_header_digit: f32,
    /// The row headers' padding at 100 % zoom.
    pub row_header_pad: f32,
}

impl SheetMetrics {
    /// Header sizes of `column_header` high, `row_header_digit` per digit and `row_header_pad` of padding.
    pub const fn new(column_header: f32, row_header_digit: f32, row_header_pad: f32) -> Self {
        SheetMetrics { column_header, row_header_digit, row_header_pad }
    }

    /// The row headings' width by the column headings' height at `zoom` for row numbers of `digits` digits; zero
    /// without headings. Headings shrink no further than [`HEADER_MIN_ZOOM`].
    pub fn header(&self, headings: bool, zoom: f32, digits: u32) -> Vec2 {
        if !headings {
            return Vec2::ZERO;
        }
        let zoom = zoom.max(HEADER_MIN_ZOOM);
        Vec2::new(((digits.max(2) as f32 * self.row_header_digit + self.row_header_pad) * zoom).round(), (self.column_header * zoom).round())
    }
}

/// What a grid's geometry is made from, per frame.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct GridLayout {
    /// The grid's rectangle on screen, in points.
    pub rect: Rect,
    /// The zoom, within [`SHEET_ZOOM`].
    pub zoom: f32,
    /// Row heights.
    pub rows: LineIndex,
    /// Column widths.
    pub columns: LineIndex,
    /// Frozen rows and columns at the top and start.
    pub frozen: (u32, u32),
    /// Scroll of the unfrozen part, in points at 100 %.
    pub scroll: Vec2,
    /// Whether the row and column headings are shown.
    pub headings: bool,
    /// The outline bands' thickness before the row headers and above the column headers.
    pub bands: Vec2,
    /// Whether the sheet is mirrored for a right-to-left script.
    pub right_to_left: bool,
    /// The digits the row headers hold; read from the rows shown when `None`.
    pub digits: Option<u32>,
}

impl GridLayout {
    /// A layout in `rect` over `rows` and `columns` at 100 % zoom, unscrolled, with headings.
    pub fn new(rect: Rect, rows: LineIndex, columns: LineIndex) -> Self {
        GridLayout {
            rect,
            zoom: 1.0,
            rows,
            columns,
            frozen: (0, 0),
            scroll: Vec2::ZERO,
            headings: true,
            bands: Vec2::ZERO,
            right_to_left: false,
            digits: None,
        }
    }

    /// The zoom.
    #[must_use]
    pub fn zoom(mut self, zoom: f32) -> Self {
        self.zoom = zoom;
        self
    }

    /// The frozen rows and columns.
    #[must_use]
    pub fn frozen(mut self, rows: u32, columns: u32) -> Self {
        self.frozen = (rows, columns);
        self
    }

    /// The scroll of the unfrozen part, in points at 100 %.
    #[must_use]
    pub fn scroll(mut self, scroll: Vec2) -> Self {
        self.scroll = scroll;
        self
    }

    /// The outline bands' thickness.
    #[must_use]
    pub fn bands(mut self, bands: Vec2) -> Self {
        self.bands = bands;
        self
    }

    /// Whether the sheet is mirrored.
    #[must_use]
    pub fn right_to_left(mut self, on: bool) -> Self {
        self.right_to_left = on;
        self
    }

    /// Whether the row and column headings show.
    #[must_use]
    pub fn headings(mut self, on: bool) -> Self {
        self.headings = on;
        self
    }

    /// The digits the row headers hold.
    #[must_use]
    pub fn digits(mut self, digits: u32) -> Self {
        self.digits = Some(digits);
        self
    }
}

/// The geometry of a grid for one frame: where each cell is on screen, and which cell is at a point. Logical
/// coordinates run left to right; a right-to-left sheet is mirrored across the rect's centre line on screen
/// ([`mirror_x`](Self::mirror_x)), so layout and hit tests are one code path.
#[derive(Clone, Debug)]
pub struct SheetGeometry {
    layout: GridLayout,
    cells: Rect,
    header: Vec2,
    frozen_size: Vec2,
}

impl SheetGeometry {
    /// Column headers `metrics.column_header` high at 100 % zoom (zoomed, but never below [`HEADER_MIN_ZOOM`] of it),
    /// row headers wide for the digits of the last visible row plus 200 (or `layout.digits`), none without headings;
    /// cells after them and the bands.
    pub fn new(mut layout: GridLayout, metrics: SheetMetrics) -> Self {
        let (least, greatest) = (*SHEET_ZOOM.start(), *SHEET_ZOOM.end());
        layout.zoom = if layout.zoom.is_finite() { layout.zoom.clamp(least, greatest) } else { 1.0 };
        layout.frozen = (layout.frozen.0.min(layout.rows.count()), layout.frozen.1.min(layout.columns.count()));
        let (zoom, rect) = (layout.zoom, layout.rect);
        let digits = layout.digits.unwrap_or_else(|| {
            let last_row = layout.rows.at(f64::from(layout.scroll.y + rect.height() / zoom) + ROW_HEADER_LOOKAHEAD);
            digits_of(last_row)
        });
        let header = metrics.header(layout.headings, zoom, digits);
        let cells = Rect::from_min_max(Pos2::new(rect.left() + layout.bands.x + header.x, rect.top() + layout.bands.y + header.y), rect.max);
        let frozen_size = Vec2::new(layout.columns.start(layout.frozen.1) as f32 * zoom, layout.rows.start(layout.frozen.0) as f32 * zoom);
        Self { layout, cells, header, frozen_size }
    }

    /// The zoom the geometry was built at.
    pub fn zoom(&self) -> f32 {
        self.layout.zoom
    }

    /// The outline bands' thickness before the row headers and above the column headers.
    pub fn bands(&self) -> Vec2 {
        self.layout.bands
    }

    /// The frozen rows and columns, at most the sheet's.
    pub fn frozen(&self) -> (u32, u32) {
        self.layout.frozen
    }

    /// The screen-logical position where the scrolling cells begin: after the frozen columns and rows.
    pub fn split(&self) -> Pos2 {
        Pos2::new(self.cells.left() + self.frozen_size.x, self.cells.top() + self.frozen_size.y)
    }

    /// The grid's rectangle.
    pub fn rect(&self) -> Rect {
        self.layout.rect
    }

    /// The cells' area (after the row headers, below the column headers).
    pub fn cells(&self) -> Rect {
        self.cells
    }

    /// The row headings' width by the column headings' height, in screen points; zero without headings.
    pub fn header_size(&self) -> Vec2 {
        self.header
    }

    /// The row heights.
    pub fn rows(&self) -> &LineIndex {
        &self.layout.rows
    }

    /// The column widths.
    pub fn columns(&self) -> &LineIndex {
        &self.layout.columns
    }

    /// The left x of `column` (frozen columns do not scroll).
    pub fn x(&self, column: u32) -> f32 {
        let (columns, zoom) = (&self.layout.columns, self.layout.zoom);
        if column < self.layout.frozen.1 {
            self.cells.left() + columns.start(column) as f32 * zoom
        } else {
            let from_split = columns.start(column) - columns.start(self.layout.frozen.1) - f64::from(self.layout.scroll.x);
            self.cells.left() + self.frozen_size.x + from_split as f32 * zoom
        }
    }

    /// The top y of `row` (frozen rows do not scroll).
    pub fn y(&self, row: u32) -> f32 {
        let (rows, zoom) = (&self.layout.rows, self.layout.zoom);
        if row < self.layout.frozen.0 {
            self.cells.top() + rows.start(row) as f32 * zoom
        } else {
            let from_split = rows.start(row) - rows.start(self.layout.frozen.0) - f64::from(self.layout.scroll.y);
            self.cells.top() + self.frozen_size.y + from_split as f32 * zoom
        }
    }

    /// The rectangle of cell `c`, in logical coordinates.
    pub fn cell_rect(&self, c: CellAt<u32>) -> Rect {
        let (x, y) = (self.x(c.column), self.y(c.row));
        let size = Vec2::new(self.layout.columns.size(c.column) as f32, self.layout.rows.size(c.row) as f32) * self.layout.zoom;
        Rect::from_min_size(Pos2::new(x, y), size)
    }

    /// The rectangle covering `r`, in logical coordinates.
    pub fn range_rect(&self, r: &CellRange<u32>) -> Rect {
        let (top, bottom, left, right) = edges(r);
        Rect::from_min_max(self.cell_rect(CellAt::new(top, left)).min, self.cell_rect(CellAt::new(bottom, right)).max)
    }

    /// The column at logical x.
    pub fn column_at(&self, x: f32) -> u32 {
        let (columns, zoom, frozen) = (&self.layout.columns, self.layout.zoom, self.layout.frozen.1);
        let from_left = f64::from((x - self.cells.left()) / zoom);
        if from_left < f64::from(self.frozen_size.x / zoom) {
            columns.at(from_left.max(0.0))
        } else {
            let past_split = from_left - f64::from(self.frozen_size.x / zoom);
            columns.at(columns.start(frozen) + f64::from(self.layout.scroll.x) + past_split)
        }
    }

    /// The row at logical y.
    pub fn row_at(&self, y: f32) -> u32 {
        let (rows, zoom, frozen) = (&self.layout.rows, self.layout.zoom, self.layout.frozen.0);
        let from_top = f64::from((y - self.cells.top()) / zoom);
        if from_top < f64::from(self.frozen_size.y / zoom) {
            rows.at(from_top.max(0.0))
        } else {
            let past_split = from_top - f64::from(self.frozen_size.y / zoom);
            rows.at(rows.start(frozen) + f64::from(self.layout.scroll.y) + past_split)
        }
    }

    /// The cell at the logical position `p`.
    pub fn cell_at(&self, p: Pos2) -> CellAt<u32> {
        CellAt::new(self.row_at(p.y), self.column_at(p.x))
    }

    /// The visible scrolled rows and columns (after the frozen ones), inclusive.
    pub fn visible(&self) -> ((u32, u32), (u32, u32)) {
        let GridLayout { rows, columns, frozen, scroll, zoom, .. } = &self.layout;
        let span = |lines: &LineIndex, frozen: u32, scroll: f32, room: f32| {
            let top = lines.start(frozen) + f64::from(scroll);
            (lines.at(top).max(frozen), lines.at(top + f64::from(room / zoom)).min(lines.count().saturating_sub(1)))
        };
        (
            span(rows, frozen.0, scroll.y, self.cells.height() - self.frozen_size.y),
            span(columns, frozen.1, scroll.x, self.cells.width() - self.frozen_size.x),
        )
    }

    /// The panes (frozen corner, frozen rows, frozen columns, scrolling) that hold part of `r`, as screen rects to clip
    /// to, so a range scrolled under the frozen part is not drawn over it.
    pub fn range_panes(&self, r: &CellRange<u32>) -> Vec<Rect> {
        let (frozen_rows, frozen_columns) = self.layout.frozen;
        let (top, bottom, left, right) = edges(r);
        let split = Pos2::new(self.cells.left() + self.frozen_size.x, self.cells.top() + self.frozen_size.y);
        let mut xs = Vec::new();
        if left < frozen_columns {
            xs.push((self.cells.left(), split.x));
        }
        if right >= frozen_columns {
            xs.push((split.x, self.cells.right()));
        }
        let mut ys = Vec::new();
        if top < frozen_rows {
            ys.push((self.cells.top(), split.y));
        }
        if bottom >= frozen_rows {
            ys.push((split.y, self.cells.bottom()));
        }
        ys.iter()
            .flat_map(|&(top, bottom)| xs.iter().map(move |&(left, right)| Rect::from_min_max(Pos2::new(left, top), Pos2::new(right, bottom))))
            .collect()
    }

    /// Logical x to screen x and back (identity left to right).
    pub fn mirror_x(&self, x: f32) -> f32 {
        if self.layout.right_to_left { self.layout.rect.left() + self.layout.rect.right() - x } else { x }
    }

    /// `p` with its x mirrored; see [`mirror_x`](Self::mirror_x).
    pub fn mirror_pos(&self, p: Pos2) -> Pos2 {
        Pos2::new(self.mirror_x(p.x), p.y)
    }

    /// `r` with its x range mirrored; see [`mirror_x`](Self::mirror_x).
    pub fn mirror_rect(&self, r: Rect) -> Rect {
        let (a, b) = (self.mirror_x(r.left()), self.mirror_x(r.right()));
        Rect::from_x_y_ranges(a.min(b)..=a.max(b), r.y_range())
    }
}

/// The insets a style holds for the headers: the row headers' width at the start and the column headers' height on
/// top, plus the bands.
pub(crate) fn header_insets(sheet: SheetMetrics, headings: bool, zoom: f32, digits: u32, bands: Vec2) -> Insets {
    let header = sheet.header(headings, zoom, digits);
    Insets::new(header.x + bands.x, 0.0, header.y + bands.y, 0.0)
}
