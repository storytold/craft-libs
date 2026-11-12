//! Stories of the cell grid: a sales sheet with a frozen header row, a wide column, filter buttons, an error tag and
//! a two-range selection; a numbered sheet with frozen panes, a merged cell and a hidden column, the copy mark, an
//! open cell, a formula in point mode and the chrome that sits on the sheet.

use std::collections::BTreeMap;

use craft_behavior::{CellDirection, CellSelection, ColumnKey};
use craft_ui::contract::{ColumnSort, PaintUi, Props as _};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::{Env, Label};
use craft_ui::style::{Corners, SurfaceKind, TypeRole};
use craft_ui::testing::{Drive, Fixture, StoryData, StoryOutput, show_as_only_item, story};
use egui::{Key, Modifiers, Rect, Ui, Vec2, pos2, vec2};

use super::row::GridRow;
use super::{CellAt, CellGrid, CellProblem, CellRange, ColumnFilter, GridModel, GridOutput, GridState, LineIndex, SHEET_COLUMNS, SHEET_ROWS};
use crate::ui::outline::{OutlineAxis, OutlineGroup};

/// A cell's text, and whether it holds a number (drawn at the cell's end edge).
type Cell = (String, bool);

/// The sheet the stories show.
struct StorySheet {
    rows: LineIndex,
    columns: LineIndex,
    merges: Vec<CellRange<u32>>,
    frozen: (u32, u32),
    /// The cells by row; a cell past them is empty.
    cells: Vec<Vec<Cell>>,
    /// The room between a cell's edge and its text.
    pad: f32,
    /// Whether the first row is a header row, drawn on the chrome surface in strong type.
    header: bool,
}

/// A value that runs past the edge of its column.
const LONG_TEXT: &str = "A cell value that is far wider than its column and is cut at the cell's edge";

/// The sales sheet's size.
const SALES_ROWS: u32 = 40;
const SALES_COLUMNS: u32 = 12;

/// The sales sheet's wide column: Product.
const WIDE_COLUMN: u32 = 1;

/// The sales sheet's cell (row, column) whose number is stored as text, which the error tag marks.
const TEXT_NUMBER: (u32, u32) = (3, 4);

/// The numbered sheet's data region: 12 rows and 6 columns.
const NUMBERED: (u32, u32) = (12, 6);

impl StorySheet {
    /// The sales sheet: a header row, then 39 rows of sales, with the Product column wide.
    fn sales(ui: &Ui) -> Self {
        let size = &Env::of(ui).tokens.size;
        let headings = ["Region", "Product", "Channel", "Quarter", "Units", "Price", "Sales", "Discount", "Cost", "Margin", "Rating", "Notes"];
        let regions = ["North", "South", "East", "West", "Central", "Export"];
        let products = ["Widget", "Gadget", "Gizmo", "Doohickey"];
        let mut cells = vec![headings.iter().map(|heading| ((*heading).to_owned(), false)).collect::<Vec<_>>()];
        for i in 0..(SALES_ROWS as usize - 1) {
            let units = (20 + (i * 37) % 80) as f64;
            let price = 4.5 + ((i * 13) % 20) as f64 * 0.5;
            let sales = units * price;
            let cost = sales * 0.6;
            let discount = ((i * 7) % 5) as f64 * 5.0;
            let rating = (1 + (i * 3) % 5) as f64;
            let text = |text: String| (text, false);
            let number = |text: String| (text, true);
            cells.push(vec![
                text(regions[i % 6].to_owned()),
                text(products[(i / 2) % 4].to_owned()),
                text(if i % 2 == 0 { "Online" } else { "Retail" }.to_owned()),
                text(format!("Q{}", (i / 4) % 4 + 1)),
                (format!("{units:.0}"), (i as u32 + 1, 4) != TEXT_NUMBER),
                number(format!("{price:.2}")),
                number(format!("{sales:.2}")),
                number(format!("{discount:.0}%")),
                number(format!("{cost:.2}")),
                number(format!("{:.2}", sales - cost)),
                number(format!("{rating:.0}")),
                text(if i % 9 == 3 { "Check figures" } else { "" }.to_owned()),
            ]);
        }
        StorySheet {
            rows: LineIndex::new(&BTreeMap::new(), size.control.sm, SALES_ROWS),
            columns: LineIndex::new(&BTreeMap::from([(WIDE_COLUMN, size.length.short)]), size.length.number.md, SALES_COLUMNS),
            merges: Vec::new(),
            frozen: (1, 0),
            cells,
            pad: Env::of(ui).tokens.space.gap.inline,
            header: true,
        }
    }

    /// A sheet of `rows` numbered rows with a hidden column, one merged cell and frozen leading lines; with `long`, the
    /// first cell holds [`LONG_TEXT`].
    fn numbered(ui: &Ui, rows: u32, long: bool) -> Self {
        let size = &Env::of(ui).tokens.size;
        let hidden: BTreeMap<u32, f32> = BTreeMap::from([(3, 0.0)]);
        let cells = (0..rows.min(NUMBERED.0))
            .map(|row| (0..NUMBERED.1).map(|column| (format!("{}", row * 10 + column), true)).collect::<Vec<_>>())
            .collect::<Vec<_>>();
        let mut sheet = StorySheet {
            rows: LineIndex::new(&BTreeMap::new(), size.control.sm, rows),
            columns: LineIndex::new(&hidden, size.length.number.md, SHEET_COLUMNS),
            merges: if rows > 3 { vec![CellRange::cells(CellAt::new(2, 1), CellAt::new(3, 2))] } else { Vec::new() },
            frozen: (rows.min(1), 1),
            cells,
            pad: Env::of(ui).tokens.space.gap.inline,
            header: false,
        };
        if long && let Some(first) = sheet.cells.first_mut().and_then(|row| row.first_mut()) {
            *first = (LONG_TEXT.to_owned(), false);
        }
        sheet
    }

    fn cell(&self, cell: CellAt<u32>) -> Option<&Cell> {
        self.cells.get(cell.row as usize).and_then(|row| row.get(cell.column as usize))
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
        let rows = self.cells.len() as u32;
        let columns = self.cells.first().map_or(0, |row| row.len() as u32);
        CellAt::new(rows.saturating_sub(1), columns.saturating_sub(1))
    }

    /// The data fills the cells the sheet holds, so an edge jump stops at the last used row or column.
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

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, zoom: f32) {
        let draw = ui.draw();
        let header = self.header && cell.row == 0;
        if header {
            draw.surface(rect, SurfaceKind::Chrome, Corners::None);
        }
        let Some((text, number)) = self.cell(cell).filter(|(text, _)| !text.is_empty()) else { return };
        let inner = rect.shrink2(vec2(self.pad * zoom, 0.0));
        let run = TextRun::new(text.as_str()).role(if header { TypeRole::BodyStrong } else { TypeRole::Body }).elide();
        if *number {
            let layout = draw.layout(&run, inner.width());
            let size = layout.size();
            draw.galley(pos2(inner.right() - size.x, inner.center().y - size.y / 2.0), &layout);
        } else {
            draw.text(inner, &run);
        }
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        self.cell(cell).map(|(text, _)| text.clone()).unwrap_or_default()
    }
}

/// The story's readout: the last edit, filter press or fix the grid reported.
fn last_event(data: &mut StoryData, out: &GridOutput) -> String {
    let last = data.note("last", None::<String>);
    if let Some(edit) = out.edits.last() {
        *last = Some(format!("{edit:?}"));
    } else if let Some(filter) = out.filter {
        *last = Some(format!("{filter:?}"));
    } else if let Some(fix) = out.fix {
        *last = Some(format!("Fix {fix}"));
    }
    last.as_ref().map_or_else(|| "No event yet".to_owned(), |event| format!("Last event: {event}"))
}

/// The sales sheet with a filter button in every heading cell (Region sorted, Channel filtered), the error tag on a
/// number stored as text and two selected ranges.
#[story(CellGrid<'static>, covers = [Default], drive = Drive::Never, title = "Sheet")]
fn cell_grid(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    let model = StorySheet::sales(ui);
    // The range that holds the active cell comes last: it is the current range.
    let ranges = vec![CellRange::cells(CellAt::new(8, 4), CellAt::new(10, 6)), CellRange::cells(CellAt::new(2, 1), CellAt::new(5, 3))];
    let selection = CellSelection::from_ranges(ranges, CellAt::new(2, 1)).unwrap_or_default();
    let mut state = data.value("state", GridState { selection, ..GridState::default() }).clone();
    let filters: Vec<ColumnFilter> = (0..SALES_COLUMNS)
        .map(|column| {
            ColumnFilter::new(CellAt::new(0, column)).filtered(column == 2).sort((column == 0).then(|| ColumnSort::ascending(ColumnKey::index(0))))
        })
        .collect();
    let fixes = [Label::from("Convert to Number"), Label::from("Ignore Error")];
    let out = CellGrid::new("Sheet", &model, &mut state)
        .filters(&filters)
        .error_tag(CellProblem::new(CellAt::new(TEXT_NUMBER.0, TEXT_NUMBER.1), "Number Stored as Text", &fixes))
        .show_sheet(ui);
    *data.value("state", GridState::default()) = state;
    let readout = last_event(data, &out);
    StoryOutput::from(out).readout(readout)
}

fn sheet(ui: &mut Ui, data: &mut StoryData, selection: CellSelection<u32>, marks: &[CellRange<u32>]) -> StoryOutput {
    let model = StorySheet::numbered(ui, SHEET_ROWS, false);
    shown_sheet(ui, data, model, selection, marks)
}

fn shown_sheet(ui: &mut Ui, data: &mut StoryData, model: StorySheet, selection: CellSelection<u32>, marks: &[CellRange<u32>]) -> StoryOutput {
    let mut state = data.value("state", GridState { selection, ..GridState::default() }).clone();
    let out = CellGrid::new("Sheet", &model, &mut state).copy_mark(marks).show_sheet(ui);
    *data.value("state", GridState::default()) = state;
    out.into()
}

/// Column D hidden, B3:C4 merged, the first row and column frozen.
#[story(CellGrid<'static>, covers = [], drive = Drive::Never)]
fn cell_grid_hidden_column_and_merged_cell(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
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
    let model = StorySheet::numbered(ui, SHEET_ROWS, false);
    let mut state = data.value("state", GridState::default()).clone();
    let filters = [ColumnFilter::new(CellAt::new(0, 0)).filtered(true), ColumnFilter::new(CellAt::new(0, 1))];
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
    let model = StorySheet::numbered(ui, SHEET_ROWS, false);
    let origin = ui.available_rect_before_wrap().min;
    let (width, height) = (model.columns().size(0) as f32, model.rows().size(1) as f32);
    let cells = (0..STORY_ROW_CELLS)
        .map(|column| (ColumnKey::index(column), Rect::from_min_size(origin + Vec2::new(width * column as f32, 0.0), Vec2::new(width, height))))
        .collect();
    show_as_only_item(ui, GridRow::new(&model, 1, 1.0, cells).label("Row 2")).into()
}

#[story(CellGrid<'static>, covers = [Empty], drive = Drive::Never)]
fn cell_grid_empty(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::numbered(ui, 0, false), CellSelection::default(), &[])
}

#[story(CellGrid<'static>, covers = [One], drive = Drive::Never)]
fn cell_grid_one_row(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::numbered(ui, 1, false), CellSelection::cell(CellAt::new(0, 0)), &[])
}

#[story(CellGrid<'static>, covers = [LongText], drive = Drive::Never)]
fn cell_grid_long_text(ui: &mut Ui, data: &mut StoryData) -> StoryOutput {
    shown_sheet(ui, data, StorySheet::numbered(ui, SHEET_ROWS, true), CellSelection::cell(CellAt::new(0, 0)), &[])
}
