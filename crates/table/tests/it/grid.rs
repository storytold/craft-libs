//! `CellGrid`: End mode, the fill handle, point mode, the rows a pass runs and the chrome over the cells.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use craft_behavior::{CellSelection, EditEvent, PartKey, PartRef, ScrollPos};
use craft_table::ui::{
    CellAt, CellEdit, CellGrid, CellProblem, CellRange, ColumnFilter, GridEdit, GridModel, GridState, LineIndex, SHEET_COLUMNS, SHEET_ROWS,
    SheetGeometry,
};
use craft_ui::contract::{PaintUi, Props as _};
use craft_ui::primitives::TextRun;
use craft_ui::runtime::env::Label;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, pointer, trial};
use egui::{Key, Modifiers, Rect, Vec2};

/// The default line sizes of the test sheet, points at 100 %.
const ROW: f32 = 20.0;
const COLUMN: f32 = 64.0;

struct Sheet {
    rows: LineIndex,
    columns: LineIndex,
    zoom: Rc<Cell<f32>>,
}

impl Sheet {
    fn new(hidden_column: Option<u32>, zoom: Rc<Cell<f32>>) -> Self {
        let custom: BTreeMap<u32, f32> = hidden_column.into_iter().map(|column| (column, 0.0)).collect();
        Sheet { rows: LineIndex::new(&BTreeMap::new(), ROW, SHEET_ROWS), columns: LineIndex::new(&custom, COLUMN, SHEET_COLUMNS), zoom }
    }
}

impl GridModel for Sheet {
    fn rows(&self) -> &LineIndex {
        &self.rows
    }

    fn columns(&self) -> &LineIndex {
        &self.columns
    }

    fn last_used(&self) -> CellAt<u32> {
        CellAt::new(9, 4)
    }

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, zoom: f32) {
        self.zoom.set(zoom);
        ui.draw().text(rect, &TextRun::new(self.cell_text(cell)).elide());
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        format!("{}", cell.row * 10 + cell.column)
    }
}

struct Rig {
    fixture: Fixture,
    state: Rc<RefCell<GridState>>,
    edits: Rc<RefCell<Vec<GridEdit>>>,
    committed: Rc<RefCell<Vec<CellEdit>>>,
    grid: Rc<Cell<Option<egui::Id>>>,
    geometry: Rc<RefCell<Option<SheetGeometry>>>,
}

fn rig(hidden_column: Option<u32>, selection: CellSelection<u32>, zoom: f32) -> Rig {
    let state = Rc::new(RefCell::new(GridState::new(selection, zoom)));
    let edits = Rc::new(RefCell::new(Vec::new()));
    let committed = Rc::new(RefCell::new(Vec::new()));
    let seen_zoom = Rc::new(Cell::new(0.0));
    let (shown_state, shown_edits, shown_zoom) = (state.clone(), edits.clone(), seen_zoom.clone());
    let shown_committed = committed.clone();
    let grid = Rc::new(Cell::new(None));
    let shown_grid = grid.clone();
    let geometry = Rc::new(RefCell::new(None));
    let shown_geometry = geometry.clone();
    let mut fixture = Fixture::new(move |ui| {
        let model = Sheet::new(hidden_column, shown_zoom.clone());
        let mut state = shown_state.borrow().clone();
        let out = CellGrid::new("Sheet", &model, &mut state).show_sheet(ui);
        shown_grid.set(Some(out.collection.response.id));
        *shown_geometry.borrow_mut() = Some(out.geometry);
        *shown_state.borrow_mut() = state;
        shown_edits.borrow_mut().extend(out.edits);
        shown_committed.borrow_mut().extend(out.committed);
    });
    fixture.settle();
    let target = fixture.node_of(grid.get().expect("the grid showed")).expect("the grid has a node");
    fixture.access(AccessActionKind::Focus, &target);
    fixture.settle();
    Rig { fixture, state, edits, committed, grid, geometry }
}

impl Rig {
    fn press(&mut self, key: Key, modifiers: Modifiers) {
        self.fixture.key(key, modifiers);
        self.fixture.idle(2);
    }

    fn active(&self) -> Option<CellAt<u32>> {
        self.state.borrow().selection.active().copied()
    }

    fn take_edits(&self) -> Vec<GridEdit> {
        self.edits.borrow_mut().drain(..).collect()
    }

    /// The rect of a pointer-only part, as the sensing step recorded it.
    fn part_rect(&self, part: impl Into<PartRef>) -> Rect {
        let id = self.grid.get().expect("the grid showed");
        self.fixture.part_rect(id, part).expect("the part was sensed")
    }

    /// A click at the centre of `part` with `modifiers` held.
    fn click_part(&mut self, part: impl Into<PartRef>, modifiers: Modifiers) {
        let at = self.part_rect(part).center();
        self.fixture.hold(modifiers).click_at(at).hold(Modifiers::NONE).idle(2);
    }

    /// The cell under the screen point `at` in the last pass's geometry.
    fn cell_under(&self, at: egui::Pos2) -> CellAt<u32> {
        let geometry = self.geometry.borrow();
        let geometry = geometry.as_ref().expect("the grid showed");
        geometry.cell_at(geometry.mirror_pos(at))
    }

    fn drag_part(&mut self, part: impl Into<PartRef>, by: Vec2) {
        let from = self.part_rect(part).center();
        self.fixture.drag_at(from, by);
    }
}

fn a1() -> CellSelection<u32> {
    CellSelection::cell(CellAt::new(0, 0))
}

/// End alone only arms End mode; the next arrow jumps to the data edge, and Ctrl+End goes to the last used cell. End
/// toggles the mode, and End then Home goes to the last used cell.
#[trial]
fn end_arms_a_jump_and_ctrl_end_goes_to_the_last_used_cell() {
    let mut armed = rig(None, a1(), 1.0);
    armed.press(Key::End, Modifiers::NONE);
    assert_eq!(armed.active(), Some(CellAt::new(0, 0)), "End alone moves nothing");
    armed.press(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(armed.active(), Some(CellAt::new(SHEET_ROWS - 1, 0)), "the arrow after End jumps to the data edge");
    armed.press(Key::ArrowUp, Modifiers::NONE);
    assert_eq!(armed.active(), Some(CellAt::new(SHEET_ROWS - 2, 0)), "the mode ends with that arrow");
    armed.press(Key::End, Modifiers::COMMAND);
    assert_eq!(armed.active(), Some(CellAt::new(9, 4)));

    let mut twice = rig(None, a1(), 1.0);
    twice.press(Key::End, Modifiers::NONE);
    twice.press(Key::End, Modifiers::NONE);
    twice.press(Key::ArrowDown, Modifiers::NONE);
    assert_eq!(twice.active(), Some(CellAt::new(1, 0)), "End twice leaves the mode, so the arrow steps one row");

    let mut homed = rig(None, a1(), 1.0);
    homed.press(Key::End, Modifiers::NONE);
    homed.press(Key::Home, Modifiers::NONE);
    assert_eq!(homed.active(), Some(CellAt::new(9, 4)), "End then Home goes to the last used cell");
}

fn selected_block() -> CellSelection<u32> {
    CellSelection::range(CellRange::cells(CellAt::new(1, 1), CellAt::new(2, 2)))
}

#[trial]
fn the_fill_handle_drags_a_range() {
    let mut rig = rig(None, selected_block(), 1.0);
    rig.drag_part(PartKey::new("fill_handle"), Vec2::Y * (ROW * 1.5));
    let fills: Vec<_> =
        rig.take_edits().into_iter().filter_map(|edit| if let GridEdit::Fill { range, .. } = edit { Some(range) } else { None }).collect();
    assert!(matches!(fills.first(), Some(EditEvent::Begin { .. })));
    assert!(
        matches!(fills.last(), Some(EditEvent::Commit { after, .. }) if after.rows().end() - after.rows().start() == 3),
        "the handle sits on the block corner, so 1.5 rows down ends in the second row below: the range grows two rows"
    );
}

#[trial]
fn point_mode_inserts_an_absolute_reference() {
    let mut rig = rig(None, CellSelection::cell(CellAt::new(5, 5)), 1.0);
    rig.fixture.type_text("=");
    rig.fixture.idle(2);
    rig.press(Key::ArrowUp, Modifiers::NONE);
    rig.press(Key::Enter, Modifiers::NONE);
    assert!(rig.take_edits().iter().any(|edit| matches!(edit, GridEdit::BeginEdit { .. })));
    let written = rig.committed.borrow().last().cloned();
    assert_eq!(written, Some(CellEdit::new(CellAt::new(5, 5), "=$F$5")), "the arrow pointed at the cell above and Enter committed the formula");
}

/// The test sheet, recording the rows whose cells it paints.
struct Painted {
    sheet: Sheet,
    rows: Rc<RefCell<BTreeSet<u32>>>,
}

impl GridModel for Painted {
    fn rows(&self) -> &LineIndex {
        self.sheet.rows()
    }

    fn columns(&self) -> &LineIndex {
        self.sheet.columns()
    }

    fn last_used(&self) -> CellAt<u32> {
        self.sheet.last_used()
    }

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, zoom: f32) {
        self.rows.borrow_mut().insert(cell.row);
        self.sheet.paint_cell(ui, rect, cell, zoom);
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        self.sheet.cell_text(cell)
    }
}

/// A pass runs only the rows in view: a row holds no stops and its height is exact, so no row out of view needs to
/// run before it scrolls in.
#[trial]
fn a_pass_runs_only_the_rows_in_view() {
    const ROOM: Vec2 = Vec2::new(400.0, 300.0);
    let painted = Rc::new(RefCell::new(BTreeSet::new()));
    let rows = painted.clone();
    let state = RefCell::new(GridState::new(a1(), 1.0));
    let mut fixture = Fixture::new(move |ui| {
        let model = Painted { sheet: Sheet::new(None, Rc::new(Cell::new(1.0))), rows: rows.clone() };
        let mut shown = state.borrow().clone();
        CellGrid::new("Sheet", &model, &mut shown).show_sheet(ui);
        *state.borrow_mut() = shown;
    });
    fixture.room(ROOM).settle();
    painted.borrow_mut().clear();
    fixture.idle(1);
    let painted = painted.borrow();
    assert!(painted.contains(&0), "the first row is in view");
    let in_view = (ROOM.y / ROW).ceil() as u32;
    assert!(painted.iter().all(|&row| row < in_view), "rows past the view ran: {painted:?}");
}

/// Alt+Enter and Ctrl+Enter open the active cell's editor, as F2 does.
#[trial]
fn alt_or_ctrl_enter_opens_the_active_cell() {
    for modifiers in [Modifiers::ALT, Modifiers::CTRL] {
        let mut rig = rig(None, a1(), 1.0);
        rig.press(Key::Enter, modifiers);
        assert!(rig.take_edits().iter().any(|edit| matches!(edit, GridEdit::BeginEdit { typed: None, .. })), "{modifiers:?}+Enter opens the editor");
        assert_eq!(rig.active(), Some(CellAt::new(0, 0)), "{modifiers:?}+Enter does not advance");
    }
}

/// Shift+click on a column header extends the current range from the anchor's column; Cmd+click adds the whole
/// column as a new range.
#[trial]
fn header_shift_click_extends_and_cmd_click_adds_a_whole_line() {
    let mut rig = rig(None, a1(), 1.0);
    let column = |at: u64| PartKey::new("column_header").at(at);
    rig.click_part(column(1), Modifiers::NONE);
    rig.click_part(column(3), Modifiers::SHIFT);
    let whole = (0..=SHEET_ROWS - 1, 1..=3);
    {
        let state = rig.state.borrow();
        let ranges = state.selection.ranges();
        assert_eq!(ranges.len(), 1, "Shift extends the one range: {ranges:?}");
        assert_eq!((ranges[0].rows(), ranges[0].column_span()), whole, "columns B to D, whole");
    }
    rig.click_part(column(5), Modifiers::COMMAND);
    let state = rig.state.borrow();
    let ranges = state.selection.ranges();
    assert_eq!(ranges.len(), 2, "Cmd adds a range: {ranges:?}");
    assert_eq!((ranges[0].rows(), ranges[0].column_span()), whole, "the first range stays");
    assert_eq!((ranges[1].rows(), ranges[1].column_span()), (0..=SHEET_ROWS - 1, 5..=5), "the new range is all of column F");
}

/// A pinch (Cmd/Ctrl with the wheel) over the grid zooms it about the pointer: the cell under the pointer stays there.
#[trial]
fn a_pinch_zooms_about_the_pointer() {
    let mut rig = rig(None, a1(), 1.0);
    let target = rig.fixture.node_of(rig.grid.get().expect("the grid showed")).expect("the grid has a node");
    let at = target.rect.center();
    let before = rig.cell_under(at);
    rig.fixture.frame(vec![pointer::moved(at)]);
    rig.fixture.frame(vec![pointer::zoom_wheel(40.0)]);
    rig.fixture.idle(3);
    let zoom = rig.state.borrow().zoom;
    assert!(zoom > 1.0 && zoom <= 4.0, "the zoom grew within the sheet's range: {zoom}");
    assert_eq!(rig.cell_under(at), before, "the cell under the pointer stays under it");
}

/// The test sheet with cells B3:C4 merged and the first row and column frozen, recording each cell it paints.
struct Merged {
    sheet: Sheet,
    merges: [CellRange<u32>; 1],
    painted: Rc<RefCell<Vec<(CellAt<u32>, Rect)>>>,
}

impl GridModel for Merged {
    fn rows(&self) -> &LineIndex {
        self.sheet.rows()
    }

    fn columns(&self) -> &LineIndex {
        self.sheet.columns()
    }

    fn last_used(&self) -> CellAt<u32> {
        self.sheet.last_used()
    }

    fn frozen(&self) -> (u32, u32) {
        (1, 1)
    }

    fn merges(&self) -> &[CellRange<u32>] {
        &self.merges
    }

    fn paint_cell(&self, ui: &mut PaintUi<'_>, rect: Rect, cell: CellAt<u32>, zoom: f32) {
        self.painted.borrow_mut().push((cell, rect));
        self.sheet.paint_cell(ui, rect, cell, zoom);
    }

    fn cell_text(&self, cell: CellAt<u32>) -> String {
        self.sheet.cell_text(cell)
    }
}

/// A merged cell paints once, over the whole of the rows and columns it spans, and none of the cells it covers paints.
#[trial]
fn a_merged_cell_paints_once_over_all_its_rows() {
    let painted = Rc::new(RefCell::new(Vec::new()));
    let seen = painted.clone();
    let state = RefCell::new(GridState::new(a1(), 1.0));
    let mut fixture = Fixture::new(move |ui| {
        let merges = [CellRange::cells(CellAt::new(2, 1), CellAt::new(3, 2))];
        let model = Merged { sheet: Sheet::new(None, Rc::new(Cell::new(1.0))), merges, painted: seen.clone() };
        let mut shown = state.borrow().clone();
        CellGrid::new("Sheet", &model, &mut shown).show_sheet(ui);
        *state.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(400.0, 300.0)).settle();
    painted.borrow_mut().clear();
    fixture.idle(1);
    let painted = painted.borrow();
    let merged: Vec<Rect> = painted.iter().filter(|(cell, _)| *cell == CellAt::new(2, 1)).map(|(_, rect)| *rect).collect();
    let [rect] = merged.as_slice() else { panic!("the merged cell painted once, not {} times", merged.len()) };
    let one = painted.iter().find(|(cell, _)| *cell == CellAt::new(1, 1)).map(|(_, rect)| *rect).expect("B2 painted");
    assert!(rect.height() > one.height() * 1.5 && rect.width() > one.width() * 1.5, "the merged cell spans two rows and two columns");
    let covered = [CellAt::new(2, 2), CellAt::new(3, 1), CellAt::new(3, 2)];
    assert!(painted.iter().all(|(cell, _)| !covered.contains(cell)), "a covered cell painted: {painted:?}");
}

/// The filter buttons sit in their header cells, below the column letters, and the error tag stays out of the scroll
/// bar: a press on the track where the tag would reach still pages the rows.
#[trial]
fn chrome_sits_in_its_cells_and_clear_of_the_scroll_bar() {
    const TAG: CellAt<u32> = CellAt::new(8, 3);
    let geometry = Rc::new(RefCell::new(None));
    let grid = Rc::new(Cell::new(None));
    let state = Rc::new(RefCell::new(GridState::new(a1(), 1.0)));
    let (seen_geometry, seen_grid, shown_state) = (geometry.clone(), grid.clone(), state.clone());
    let mut fixture = Fixture::new(move |ui| {
        let model = Sheet::new(None, Rc::new(Cell::new(1.0)));
        let filters = [ColumnFilter::new(CellAt::new(0, 0)), ColumnFilter::new(CellAt::new(0, 1)).filtered(true)];
        let fixes = [Label::from("Ignore")];
        let mut shown = shown_state.borrow().clone();
        let out = CellGrid::new("Sheet", &model, &mut shown)
            .filters(&filters)
            .error_tag(CellProblem::new(TAG, "Number stored as text", &fixes))
            .show_sheet(ui);
        seen_grid.set(Some(out.collection.response.id));
        *seen_geometry.borrow_mut() = Some(out.geometry);
        *shown_state.borrow_mut() = shown;
    });
    fixture.room(Vec2::new(600.0, 300.0)).settle();
    let sheet = geometry.borrow().clone().expect("the grid showed");
    for column in [0, 1] {
        let name = format!("{column} (Showing All)");
        let filtered = format!("{column}: Filter applied");
        let button = fixture.by_label(&name).or_else(|| fixture.by_label(&filtered)).expect("the filter button shows");
        let cell = sheet.mirror_rect(sheet.cell_rect(CellAt::new(0, column)));
        assert!(cell.contains_rect(button.rect), "filter button {column} {:?} sits in its header cell {cell:?}", button.rect);
    }

    // Narrow the grid until the scroll bar's track runs over the middle of the tag.
    let id = grid.get().expect("the grid showed");
    let track = fixture.part_rect(id, PartKey::new("items_scroll")).expect("the rows scroll");
    let tag = fixture.by_label("Error checking options").expect("the tag shows in the wide grid").rect;
    let width = tag.center().x + track.width() / 2.0 - sheet.rect().left();
    fixture.room(Vec2::new(width, 300.0)).settle();
    let track = fixture.part_rect(id, PartKey::new("items_scroll")).expect("the rows scroll");
    assert!(track.contains(tag.center()), "the track {track:?} runs over the tag's centre {:?}", tag.center());
    let before = state.borrow().scroll;
    fixture.click_at(tag.center()).idle(2);
    assert!(state.borrow().scroll.y > before.y, "the press on the track paged the rows, the tag did not take it");
}

/// The row headers hold the digits of the last row the view shows: two at the top of a short view, whatever the data,
/// and wider once the view scrolls down to row 100.
#[trial]
fn row_headers_widen_for_the_rows_the_view_shows() {
    let mut rig = rig(None, a1(), 1.0);
    rig.fixture.room(Vec2::new(400.0, 200.0)).settle();
    let corner = PartKey::new("corner");
    let top = rig.part_rect(corner);
    rig.state.borrow_mut().scroll = ScrollPos { x: 0.0, y: f64::from(ROW) * 100.0 };
    rig.fixture.settle();
    let scrolled = rig.part_rect(corner);
    assert!(scrolled.width() > top.width(), "the header strip widens for three digits: {} > {}", scrolled.width(), top.width());
    let row = rig.cell_under(rig.geometry.borrow().as_ref().expect("the grid showed").cells().center());
    assert!(row.row >= 99, "the view shows row 100, not {row:?}");
}
