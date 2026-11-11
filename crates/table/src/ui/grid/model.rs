//! The model and edit vocabulary: [`GridModel`], [`GridState`], edits, filters and cell problems.

use super::column_letters;
use craft_behavior::{CellAt, CellDirection, CellRange, CellSelection, EditEvent, LineIndex};
use craft_ui::contract::PaintUi;
use craft_ui::runtime::env::Label;
use egui::{Pos2, Rect};

/// The workbook sheet the grid shows; the app implements it on its sheet. Read only: the grid reports what the user
/// asks for as [`GridEdit`]s and moves the selection in [`GridState`].
pub trait GridModel {
    /// Row heights (points at 100 %), with hidden rows at 0.
    fn rows(&self) -> &LineIndex;

    /// Column widths in points at 100 %, with hidden columns at 0.
    fn columns(&self) -> &LineIndex;

    /// The number of frozen rows and columns; `(0, 0)` by default.
    fn frozen(&self) -> (u32, u32) {
        (0, 0)
    }

    /// The merged cells; clicks, arrows and ranges treat each as one cell.
    fn merges(&self) -> &[CellRange<u32>] {
        &[]
    }

    /// Whether a click or an arrow may land on `cell` (a protected sheet with "Select unlocked cells" only).
    fn selectable(&self, cell: CellAt<u32>) -> bool {
        let _ = cell;
        true
    }

    /// Cmd/Ctrl+arrow (End mode): the edge of the data region from `from` toward `direction`, which only the app's
    /// cells know. The default goes to the sheet's edge.
    fn data_edge(&self, from: CellAt<u32>, direction: CellDirection) -> CellAt<u32> {
        let (rows, columns) = (self.rows().count().saturating_sub(1), self.columns().count().saturating_sub(1));
        match direction {
            CellDirection::Up => CellAt::new(0, from.column),
            CellDirection::Down => CellAt::new(rows, from.column),
            CellDirection::Left => CellAt::new(from.row, 0),
            CellDirection::Right => CellAt::new(from.row, columns),
            _ => from,
        }
    }

    /// Cmd/Ctrl+End: the last cell with content (the bottom row and right column in use).
    fn last_used(&self) -> CellAt<u32>;

    /// Whether the grid draws the gridlines.
    fn show_gridlines(&self) -> bool {
        true
    }

    /// Paints cell `cell` (value, format, borders, conditional format) into `rect` at sheet `zoom` (1.0 = 100 %), so
    /// the app scales its text and borders with the grid. The grid draws the gridlines (none inside a merged cell),
    /// headers and selection; the app everything in the cell.
    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, zoom: f32);

    /// The column header's text: the letters, or a table's header name.
    fn column_label(&self, column: u32) -> String {
        column_letters(column)
    }

    /// The cell's text for AccessKit, the editor's start text and the copy fallback.
    fn cell_text(&self, cell: CellAt<u32>) -> String;
}

/// What the grid keeps between passes; the app keeps one per sheet view and saves it with the view.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct GridState {
    /// The selected cells.
    pub selection: CellSelection<u32>,
    /// The zoom, within [`SHEET_ZOOM`]; 1.0 for a new sheet.
    pub zoom: f32,
}

impl GridState {
    /// A sheet view with `selection` at `zoom`.
    pub fn new(selection: CellSelection<u32>, zoom: f32) -> Self {
        GridState { selection, zoom }
    }
}

impl Default for GridState {
    /// A1 selected at 100 %.
    fn default() -> Self {
        Self { selection: CellSelection::cell(CellAt::new(0, 0)), zoom: 1.0 }
    }
}

/// The text the in-cell editor committed.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct CellEdit {
    /// The cell.
    pub cell: CellAt<u32>,
    /// The text to write into it.
    pub text: String,
}

impl CellEdit {
    /// An edit writing `text` into `cell`.
    pub fn new(cell: CellAt<u32>, text: impl Into<String>) -> Self {
        CellEdit { cell, text: text.into() }
    }
}

/// What the grid asks of the app.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum GridEdit {
    /// A cell was opened for editing: typing replaced its content (`typed`), or F2 or a double click edits in place
    /// (`None`). The app may show the draft in its formula bar from [`GridOutput::draft`].
    #[non_exhaustive]
    BeginEdit {
        /// Replacement text typed on the grid; `None` edits in place.
        typed: Option<String>,
    },
    /// Delete: clear the selection's contents.
    Clear,
    /// Backspace: clear the active cell and edit it.
    ClearAndEdit,
    /// A column header's end edge dragged: the new width in points at 100 % zoom, as one gesture (Esc cancels). Several
    /// selected columns resize together: the app applies the width to every column of the selection that holds
    /// `column`.
    #[non_exhaustive]
    ResizeColumn {
        /// The column whose edge was dragged.
        column: u32,
        /// The new width, in points at 100 % zoom.
        width: EditEvent<f32>,
    },
    /// A row header's bottom edge dragged, as [`ResizeColumn`](Self::ResizeColumn).
    #[non_exhaustive]
    ResizeRow {
        /// The row whose edge was dragged.
        row: u32,
        /// The new height, in points at 100 % zoom.
        height: EditEvent<f32>,
    },
    /// A double-click on a column header's edge: AutoFit Column Width.
    #[non_exhaustive]
    AutoFitColumn {
        /// The column whose edge was double-clicked.
        column: u32,
    },
    /// A double-click on a row header's edge: AutoFit Row Height.
    #[non_exhaustive]
    AutoFitRow {
        /// The row whose edge was double-clicked.
        row: u32,
    },
    /// A double-click on the edge where a hidden column sits: Unhide.
    #[non_exhaustive]
    UnhideColumn {
        /// The hidden column.
        column: u32,
    },
    /// A double-click on the edge where a hidden row sits.
    #[non_exhaustive]
    UnhideRow {
        /// The hidden row.
        row: u32,
    },
    /// The fill handle dragged: the range the current selection grows to (the source plus the filled cells), as one
    /// gesture. `Update`s show the preview, `Commit` carries the range, `Cancel` (Esc) drops it. Cmd/Ctrl at the
    /// commit swaps copying and extending a series ([`GridOutput::fill_swapped`]).
    #[non_exhaustive]
    Fill {
        /// The range the selection grows to.
        range: EditEvent<CellRange<u32>>,
    },
    /// The selection dragged by its border: the cell its top-left corner moves to, as one gesture like
    /// [`Fill`](Self::Fill). The app moves the cells; the grid selects the moved range when the gesture commits.
    #[non_exhaustive]
    MoveCells {
        /// The cell the selection's top-left corner moves to.
        to: EditEvent<CellAt<u32>>,
    },
    /// The fill handle dragged with the secondary button and released: the range the drag grew the selection to, and
    /// the pointer's screen position. The app opens its fill menu there; nothing is filled or selected until then.
    #[non_exhaustive]
    FillMenu {
        /// The range the drag grew the selection to.
        range: CellRange<u32>,
        /// The pointer position in screen points.
        anchor: Pos2,
    },
    /// Esc while [`CellGrid::copy_mark`] shows the marching ants: the app ends copy mode.
    ClearCopyMark,
    /// Enter while [`CellGrid::copy_mark`] shows the marching ants: paste what was copied at the active cell and end
    /// copy mode.
    PasteAtActive,
    /// Copy the selection.
    Copy,
    /// Cut the selection.
    Cut,
    /// Paste the given text at the active cell.
    Paste(String),
    /// Cmd+Page Down: go to the next sheet.
    NextSheet,
    /// Cmd+Page Up: go to the previous sheet.
    PreviousSheet,
    /// A secondary click (or the Menu key, Shift+F10) on the cells or a header. The selection has already moved as the
    /// sheet moves it (a click outside the selection selects the cell or line first). `anchor` is the clicked cell's or
    /// header's rect (the active cell's for a key), where a menu opened from the keyboard goes; `pointer` is where a
    /// secondary click was, `None` for a key. Both are in screen coordinates.
    #[non_exhaustive]
    ContextMenu {
        /// The clicked cell's or header's rectangle, in screen points.
        anchor: Rect,
        /// Where a secondary click was, `None` for a key.
        pointer: Option<Pos2>,
        /// What was clicked.
        target: GridMenuTarget,
    },
}

/// Where a grid's context menu was asked for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum GridMenuTarget {
    /// The cells.
    Cells,
    /// A column header, by column index.
    ColumnHeader(u32),
    /// A row header, by row index.
    RowHeader(u32),
}

/// A column that shows a filter button at the end corner of its header cell.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ColumnFilter {
    /// The column.
    pub column: u32,
    /// Whether the column is filtered.
    pub filtered: bool,
    /// The column's sort, shown beside the funnel or chevron.
    pub sort: Option<craft_ui::contract::ColumnSort>,
}

impl ColumnFilter {
    /// A filter button on `column`, unfiltered and unsorted.
    pub const fn new(column: u32) -> Self {
        ColumnFilter { column, filtered: false, sort: None }
    }

    /// Whether the column is filtered.
    #[must_use]
    pub fn filtered(mut self, on: bool) -> Self {
        self.filtered = on;
        self
    }

    /// The column's sort.
    #[must_use]
    pub fn sort(mut self, sort: Option<craft_ui::contract::ColumnSort>) -> Self {
        self.sort = sort;
        self
    }
}

/// A problem in a cell, shown as an error tag beside it with its fixes in a menu.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CellProblem<'a> {
    /// The cell.
    pub cell: CellAt<u32>,
    /// What is wrong.
    pub problem: Label,
    /// The ways to fix it.
    pub fixes: &'a [Label],
}

impl<'a> CellProblem<'a> {
    /// A problem in `cell` with its `fixes`.
    pub fn new(cell: CellAt<u32>, problem: impl Into<Label>, fixes: &'a [Label]) -> Self {
        CellProblem { cell, problem: problem.into(), fixes }
    }
}
