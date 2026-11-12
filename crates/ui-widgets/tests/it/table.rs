//! The table's header row and column widths: a double click on a column's edge fits the column to its widest cell in
//! view, the header menu hides, shows and resets columns, Ctrl or Cmd with an arrow moves the focused column, and the
//! metadata table's name column follows its widest name up to a third of the table.

use craft_behavior::{ColumnKey, ColumnWidth, keyed};
use craft_host::target::Os;
use craft_ui::contract::{CollectionEvent, ColumnEvent, ColumnSort, ColumnSpec, Role, ShowCollection as _};
use craft_ui::list::ListRow;
use craft_ui::table::{MetadataGroup, MetadataRow, MetadataTable, Table};
use craft_ui::testing::{Fixture, Room, pointer, trial};
use egui::{Key, Modifiers, Pos2, Rect};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const NAME: ColumnKey = ColumnKey::new("name");
const SIZE: ColumnKey = ColumnKey::new("size");
const DATE: ColumnKey = ColumnKey::new("date");
const LONG: &str = "A file name far longer than the column it starts in";

/// A table of `names` whose name column starts `controls` control heights wide; the app applies the widths the
/// table reports. Returns the fixture and the column events seen.
fn files(names: &'static [&'static str], controls: f32) -> (Fixture, Rc<RefCell<Vec<ColumnEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let width: Rc<Cell<Option<f32>>> = Rc::new(Cell::new(None));
    let seen = events.clone();
    let mut fixture = Fixture::new(move |ui| {
        let name = width.get().map_or(ColumnWidth::Controls(controls), ColumnWidth::Fixed);
        let columns = vec![ColumnSpec::new(NAME, "Name", name).resizable(), ColumnSpec::new(SIZE, "Size", ColumnWidth::Controls(4.0)).resizable()];
        let keys: Vec<usize> = (0..names.len()).collect();
        let source = keyed(&keys, |key: &usize| *key);
        let out = Table::new("Files", columns).show_items(ui, &source, &mut |key, icx| ListRow::new(names[*key]).cells(icx.cells()));
        for event in out.events {
            if let CollectionEvent::Column(column) = event {
                if let ColumnEvent::Resized { key, width: wide } = &column
                    && *key == NAME
                {
                    width.set(Some(*wide));
                }
                seen.borrow_mut().push(column);
            }
        }
    });
    fixture.in_room(Room::Dialog).settle();
    (fixture, events)
}

fn elided(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().find(|painted| painted.text == text).is_some_and(|painted| painted.elided)
}

/// Double clicks the end edge of the header cell `label`; returns the cell's width before.
fn fit(fixture: &mut Fixture, label: &str) -> f32 {
    let cell = fixture.by_label(label).expect("the header cell's node").rect;
    let edge = Pos2::new(cell.right() - 1.0, cell.center().y);
    fixture.click_at(edge).click_at(edge).settle();
    cell.width()
}

fn resized(events: &RefCell<Vec<ColumnEvent>>) -> Option<f32> {
    events.borrow().iter().rev().find_map(|event| match event {
        ColumnEvent::Resized { key, width } if *key == NAME => Some(*width),
        _ => None,
    })
}

/// A double click on a narrow column's edge widens it until its longest name in view shows whole; on a wide column of
/// short names it narrows it, never below its header label.
#[trial]
fn a_double_click_on_a_column_edge_fits_the_column_to_its_widest_cell() {
    let (mut fixture, events) = files(&["A", LONG], 2.0);
    assert!(elided(&fixture, LONG), "the long name is cut in the narrow column");
    let before = fit(&mut fixture, "Name");
    let after = resized(&events).expect("the fit reports the column's new width");
    assert!(after > before, "the column grew: {before} -> {after}");
    assert!(!elided(&fixture, LONG), "the long name shows whole");
    assert!(events.borrow().iter().all(|event| !matches!(event, ColumnEvent::Sort { .. })), "the double click sorts nothing");

    let (mut fixture, events) = files(&["A", "B"], 12.0);
    let before = fit(&mut fixture, "Name");
    let after = resized(&events).expect("the fit reports the column's new width");
    assert!(after < before, "the column narrowed to its short names: {before} -> {after}");
    assert!(!elided(&fixture, "Name"), "the header label still shows whole");
}

/// The left edge of the painted run `text`.
fn left_of(fixture: &Fixture, text: &str) -> f32 {
    fixture.texts().iter().find(|painted| painted.text == text).map(|painted| painted.rect.left()).expect("the text was painted")
}

/// A metadata table of one group with `names`, each with the value "v{n}"; returns the fixture and the table's rect.
fn metadata(names: &'static [&'static str]) -> (Fixture, Rc<Cell<Rect>>) {
    let rect = Rc::new(Cell::new(Rect::NOTHING));
    let held = rect.clone();
    let mut fixture = Fixture::new(move |ui| {
        let rows = names.iter().enumerate().map(|(at, name)| MetadataRow::new(*name, format!("v{at}"))).collect();
        let groups = vec![MetadataGroup::new("Camera", rows)];
        held.set(MetadataTable::new("Metadata", &groups).show(ui).table.response.rect);
    });
    fixture.in_room(Room::Dialog).settle();
    (fixture, rect)
}

/// The name column is as wide as its widest name while that is narrower than a third of the table, and a third of the
/// table when a name is wider. An idle table measures its rows once, not every pass.
#[trial]
fn the_metadata_name_column_fits_its_widest_name_up_to_a_third_of_the_table() {
    let (mut short, rect) = metadata(&["ISO", "Exposure"]);
    let table = rect.get();
    assert!(!elided(&short, "Exposure"), "a short name shows whole");
    let short_value = left_of(&short, "v1");
    let name_end = short.texts().iter().find(|painted| painted.text == "Exposure").expect("the name was painted").rect.right();
    let padding = short_value - name_end;
    assert!(short_value - table.left() < table.width() / 3.0, "short names leave the values more than two thirds");
    short.idle(2);
    assert!(craft_ui::runtime::measure::widest_runs(short.ctx()).is_empty(), "an idle table keeps its rows' measure");

    let (long, _) = metadata(&["ISO", LONG]);
    assert!(elided(&long, LONG), "a name wider than a third of the table is cut");
    let long_value = left_of(&long, "v1");
    assert!(long_value > short_value, "a longer name widens the name column");
    assert!(long_value - table.left() <= table.width() / 3.0 + padding, "the name column stops at a third of the table");
}

/// A table sorted by its hideable Size column, with movable Name and Date columns; the app drops its sort on
/// `Unsorted`. Returns the fixture and the column events seen.
fn sorted_files() -> (Fixture, Rc<RefCell<Vec<ColumnEvent>>>) {
    let events = Rc::new(RefCell::new(Vec::new()));
    let sort = Rc::new(RefCell::new(Some(ColumnSort::ascending(SIZE))));
    let seen = events.clone();
    let mut fixture = Fixture::new(move |ui| {
        let columns = vec![
            ColumnSpec::new(NAME, "Name", ColumnWidth::Controls(6.0)).movable(),
            ColumnSpec::new(SIZE, "Size", ColumnWidth::Controls(4.0)).sortable().hideable(),
            ColumnSpec::new(DATE, "Date", ColumnWidth::Controls(4.0)).movable(),
        ];
        let keys = [0_usize, 1, 2];
        let source = keyed(&keys, |key: &usize| *key);
        let shown = sort.borrow().clone();
        let out =
            Table::new("Files", columns).sort(shown).show_items(ui, &source, &mut |key, icx| ListRow::new(format!("File {key}")).cells(icx.cells()));
        for event in out.events {
            if let CollectionEvent::Column(column) = event {
                if column == ColumnEvent::Unsorted {
                    *sort.borrow_mut() = None;
                }
                seen.borrow_mut().push(column);
            }
        }
    });
    fixture.in_room(Room::Dialog).settle();
    (fixture, events)
}

fn headers(fixture: &Fixture) -> Vec<String> {
    fixture.by_role(Role::COLUMN_HEADER).iter().filter_map(|target| fixture.tree().name_of(target.node).map(str::to_owned)).collect()
}

fn secondary_click(fixture: &mut Fixture, at: Pos2) {
    fixture.frame(vec![pointer::moved(at), pointer::secondary(at, true)]);
    fixture.frame(vec![pointer::secondary(at, false)]).settle();
}

/// The menu row named `label` of `role`.
fn menu_row(fixture: &Fixture, role: Role, label: &str) -> Option<craft_ui::testing::Target> {
    fixture.by_role(role).into_iter().find(|target| fixture.tree().name_of(target.node) == Some(label))
}

/// A secondary click on a header opens the header menu; unchecking a hideable column hides it, reports it and, as it
/// was the sorted one, reports the rows unsorted, and the menu stays open; Reset Columns brings it back. A secondary
/// click on the empty header after the last column opens the menu without the column commands.
#[trial]
fn the_header_menu_hides_shows_and_resets_columns() {
    let (mut fixture, events) = sorted_files();
    assert_eq!(headers(&fixture), ["Name", "Size", "Date"]);
    let name = fixture.by_label("Name").expect("the header cell").center();
    secondary_click(&mut fixture, name);
    let size = menu_row(&fixture, Role::MENU_ITEM_CHECK_BOX, "Size").expect("the menu lists the hideable column");
    assert!(menu_row(&fixture, Role::MENU_ITEM_CHECK_BOX, "Name").is_none(), "a column that can't be hidden has no check");
    assert!(menu_row(&fixture, Role::MENU_ITEM, "Move Column Right").is_some(), "the menu of a header lists the column commands");
    fixture.click(&size).settle();
    assert_eq!(headers(&fixture), ["Name", "Date"], "the hidden column leaves the header");
    assert!(events.borrow().contains(&ColumnEvent::Shown { key: SIZE, shown: false }), "the hide is reported");
    assert!(events.borrow().contains(&ColumnEvent::Unsorted), "hiding the sorted column drops the sort");
    let reset = menu_row(&fixture, Role::MENU_ITEM, "Reset Columns").expect("the menu stays open after a check");
    fixture.click(&reset).settle();
    assert_eq!(headers(&fixture), ["Name", "Size", "Date"], "Reset Columns shows every column again");
    assert!(events.borrow().contains(&ColumnEvent::Reset), "the reset is reported");

    let date = fixture.by_label("Date").expect("the header cell").rect;
    let table = fixture.shown();
    let room = Pos2::new((date.right() + table.right()) / 2.0, date.center().y);
    secondary_click(&mut fixture, room);
    assert!(menu_row(&fixture, Role::MENU_ITEM, "Reset Columns").is_some(), "the empty header opens the header menu");
    assert!(menu_row(&fixture, Role::MENU_ITEM, "Move Column Right").is_none(), "with no column there are no column commands");
}

/// Ctrl or Cmd with Right on a focused movable header moves its column one place on; Right alone moves the focus.
/// macOS makes Cmd+Right a move to the line's end, which the header reads the same.
#[trial]
fn ctrl_or_cmd_with_an_arrow_moves_the_focused_column() {
    for os in [Os::Linux, Os::Mac] {
        let (mut fixture, events) = sorted_files();
        fixture.os(os);
        // Tab enters the rows; Shift+Tab goes back to the header row, on its first column.
        fixture.key(Key::Tab, Modifiers::NONE).key(Key::Tab, Modifiers::SHIFT).settle();
        fixture.key(Key::ArrowRight, Modifiers::COMMAND).settle();
        let moves: Vec<ColumnEvent> = events.borrow().iter().filter(|event| matches!(event, ColumnEvent::Moved { .. })).cloned().collect();
        assert_eq!(moves.len(), 1, "{os:?}: one move: {moves:?}");
        assert!(matches!(&moves[0], ColumnEvent::Moved { key, to, .. } if *key == NAME && *to == 1));
        assert_eq!(headers(&fixture), ["Size", "Name", "Date"]);
        fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
        assert_eq!(events.borrow().iter().filter(|event| matches!(event, ColumnEvent::Moved { .. })).count(), 1, "a plain arrow moves no column");
    }
}
