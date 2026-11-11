//! Stories of the cell grid: a sheet with frozen panes, a merged cell and a hidden column, a multi-range selection
//! with the copy mark, an open cell, a formula in point mode and the chrome that sits on the sheet.

use std::collections::BTreeMap;

use craft_behavior::{CellDirection, CellSelection, ColumnKey};
use craft_ui::contract::{PaintUi, Props as _};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, show_as_only_item, story};
use egui::{Key, Modifiers, Rect, Ui, Vec2};

use super::row::GridRow;
use super::{CellAt, CellGrid, CellProblem, CellRange, ColumnFilter, GridModel, GridState, LineIndex, SHEET_COLUMNS, SHEET_ROWS};
use crate::ui::outline::{OutlineAxis, OutlineGroup};

/// The sheet the stories show: default line sizes, a hidden column, one merged cell and frozen leading lines.
struct StorySheet {
    rows: LineIndex,
    columns: LineIndex,
    merges: Vec<CellRange<u32>>,
    frozen: (u32, u32),
    /// Whether the first cell holds text far wider than its column.
    long: bool,
}

/// A value that runs past the edge of its column.
const LONG_TEXT: &str = "A cell value that is far wider than its column and is cut at the cell's edge";

impl StorySheet {
    fn new() -> Self {
        Self::of(SHEET_ROWS, false)
    }

    /// A sheet of `rows` rows; with `long`, the first cell holds [`LONG_TEXT`].
    fn of(rows: u32, long: bool) -> Self {
        let hidden: BTreeMap<u32, f32> = BTreeMap::from([(3, 0.0)]);
        StorySheet {
            long,
            rows: LineIndex::new(&BTreeMap::new(), 20.0, rows),
            columns: LineIndex::new(&hidden, 64.0, SHEET_COLUMNS),
            merges: if rows > 3 { vec![CellRange::cells(CellAt::new(2, 1), CellAt::new(3, 2))] } else { Vec::new() },
            frozen: (rows.min(1), 1),
        }
    }
}

impl GridModel for StorySheet {
    fn rows(&self) -> &LineIndex {
        &self.rows
    }

    fn columns(&self) -> &LineIndex {
        &self.columns
    }

    fn frozen(&self) -> (u32, u32) {
        self.frozen
    }

    fn merges(&self) -> &[CellRange<u32>] {
        &self.merges
    }

    fn last_used(&self) -> CellAt<u32> {
        CellAt::new(11, 5)
    }

    /// The data fills rows 0 to 11 and columns 0 to 5, so an edge jump stops at the last used row or column.
    fn data_edge(&self, from: CellAt<u32>, direction: CellDirection) -> CellAt<u32> {
        let last = self.last_used();
        match direction {
            CellDirection::Up => CellAt::new(0, from.column),
            CellDirection::Down => CellAt::new(last.row, from.column),
            CellDirection::Left => CellAt::new(from.row, 0),
            CellDirection::Right => CellAt::new(from.row, last.column),
            _ => from,
        }
    }

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, _: f32) {
        if cell.row <= 11 && cell.column <= 5 {
            ui.draw().text(rect, &TextRun::new(self.cell_text(cell)).elide());
        }
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        if self.long && cell == CellAt::new(0, 0) {
            return LONG_TEXT.to_owned();
        }
        format!("{}", cell.row * 10 + cell.column)
    }
}

fn sheet(ui: &mut Ui, data: &mut StoryData, selection: CellSelection<u32>, marks: &[CellRange<u32>]) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::new(), selection, marks)
}

fn shown_sheet(ui: &mut Ui, data: &mut StoryData, model: StorySheet, selection: CellSelection<u32>, marks: &[CellRange<u32>]) -> StoryOutput {
    let mut state = data.value("state", GridState { selection, ..GridState::default() }).clone();
    let out = CellGrid::new("Sheet", &model, &mut state).copy_mark(marks).show_sheet(ui);
    *data.value("state", GridState::default()) = state;
    out.into()
}

#[story(CellGrid<'static>, covers = [Default], drive = Drive::Never)]
fn cell_grid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sheet(ui, data, CellSelection::cell(CellAt::new(0, 0)), &[])
}

#[story(CellGrid<'static>, covers = [Many], drive = Drive::Never)]
fn cell_grid_ranges_and_copy_mark(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let first = CellRange::cells(CellAt::new(4, 0), CellAt::new(6, 2));
    let second = CellRange::cells(CellAt::new(8, 4), CellAt::new(9, 5));
    let selection = CellSelection::from_ranges(vec![first.clone(), second], CellAt::new(8, 4)).unwrap_or_default();
    sheet(ui, data, selection, &[first])
}

fn open_edit(fixture: &mut Fixture) {
    fixture.key(Key::F2, Modifiers::NONE);
    fixture.idle(2);
}

#[story(CellGrid<'static>, covers = [], drive = Drive::Custom(open_edit))]
fn cell_grid_edit(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sheet(ui, data, CellSelection::cell(CellAt::new(5, 4)), &[])
}

fn start_formula(fixture: &mut Fixture) {
    fixture.type_text("=");
    fixture.idle(2);
}

#[story(CellGrid<'static>, covers = [], drive = Drive::Custom(start_formula))]
fn cell_grid_point_mode(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    sheet(ui, data, CellSelection::cell(CellAt::new(5, 4)), &[])
}

#[story(CellGrid<'static>, covers = [], drive = Drive::Never)]
fn cell_grid_chrome(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let model = StorySheet::new();
    let mut state = data.value("state", GridState::default()).clone();
    let filters = [ColumnFilter::new(0).filtered(true), ColumnFilter::new(1)];
    let fixes = [Label::from("Convert to number"), Label::from("Ignore")];
    let rows = OutlineAxis { levels: 1, groups: vec![OutlineGroup { start: 4, end: 6, level: 1, collapsed: false, summary_before: false }] };
    let columns = OutlineAxis::default();
    let out = CellGrid::new("Sheet", &model, &mut state)
        .filters(&filters)
        .error_tag(CellProblem::new(CellAt::new(2, 0), "Number stored as text", &fixes))
        .outline(&rows, &columns)
        .show_sheet(ui);
    *data.value("state", GridState::default()) = state;
    out.into()
}

const STORY_ROW_CELLS: u32 = 4;

#[story(GridRow<'static>, covers = [Default], drive = Drive::Never)]
fn grid_row(ui: &mut Ui, _: &mut StoryData) -> StoryOutput {
    let model = StorySheet::new();
    let origin = ui.available_rect_before_wrap().min;
    let (width, height) = (model.columns().size(0) as f32, model.rows().size(1) as f32);
    let cells = (0..STORY_ROW_CELLS)
        .map(|column| (ColumnKey::index(column), Rect::from_min_size(origin + Vec2::new(width * column as f32, 0.0), Vec2::new(width, height))))
        .collect();
    show_as_only_item(ui, GridRow::new(&model, 1, 1.0, cells).label("Row 2")).into()
}

#[story(CellGrid<'static>, covers = [Empty], drive = Drive::Never)]
fn cell_grid_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::of(0, false), CellSelection::default(), &[])
}

#[story(CellGrid<'static>, covers = [One], drive = Drive::Never)]
fn cell_grid_one_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::of(1, false), CellSelection::cell(CellAt::new(0, 0)), &[])
}

#[story(CellGrid<'static>, covers = [LongText], drive = Drive::Never)]
fn cell_grid_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::of(SHEET_ROWS, true), CellSelection::cell(CellAt::new(0, 0)), &[])
}
