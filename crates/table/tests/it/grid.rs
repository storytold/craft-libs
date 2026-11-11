//! `CellGrid`: End mode, the fill handle, point mode and the rows a pass runs.

use std::cell::{Cell, RefCell};
use std::collections::{BTreeMap, BTreeSet};
use std::rc::Rc;

use craft_behavior::{CellSelection, EditEvent, PartKey, PartRef};
use craft_table::ui::{CellAt, CellEdit, CellGrid, CellRange, GridEdit, GridModel, GridState, LineIndex, SHEET_COLUMNS, SHEET_ROWS};
use craft_ui::contract::PaintUi;
use craft_ui::primitives::TextRun;
use craft_ui::testing::focus::AccessActionKind;
use craft_ui::testing::{Fixture, trial};
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
    let mut fixture = Fixture::new(move |ui| {
        let model = Sheet::new(hidden_column, shown_zoom.clone());
        let mut state = shown_state.borrow().clone();
        let out = CellGrid::new("Sheet", &model, &mut state).show_sheet(ui);
        shown_grid.set(Some(out.collection.response.id));
        *shown_state.borrow_mut() = state;
        shown_edits.borrow_mut().extend(out.edits);
        shown_committed.borrow_mut().extend(out.committed);
    });
    fixture.settle();
    let target = fixture.node_of(grid.get().expect("the grid showed")).expect("the grid has a node");
    fixture.access(AccessActionKind::Focus, &target);
    fixture.settle();
    Rig { fixture, state, edits, committed, grid }
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
