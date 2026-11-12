//! Fill propagates down a column whose width is given: a content-sized column there passes the width on, so its fill
//! children take it and the column with them. Where the width comes from content (a popover's body, a row) the
//! column holds its fill children and shows them across the width it settles on. A collection that fills the width
//! follows the same rule as a container.

use craft_behavior::keyed;
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Props as _, Show as _, ShowCollection as _, ShowContainer as _, Sizing};
use craft_ui::display::{Divider, Text};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::list::{List, ListRow};
use craft_ui::popover::Popover;
use craft_ui::table::{MetadataGroup, MetadataRow, MetadataTable};
use craft_ui::testing::{Frames, last_pass_measuring_runs, measuring_runs, trial};
use egui::Ui;
use std::cell::Cell;

const SNAP: f32 = 1.0;
const LONG: &str = "A heading as long as the column should settle on, wider than any control in it";

/// The widths of one column's stack, text, divider, table and button.
#[derive(Clone, Copy, Default)]
struct Widths {
    column: f32,
    text: f32,
    divider: f32,
    table: f32,
    button: f32,
}

/// A column of text, divider, table and button; without `groups` (in a body that scrolls, where a collection may not
/// sit) it has no table.
fn column(ui: &mut Ui, fill_width: bool, groups: Option<&[MetadataGroup]>, seen: &Cell<Widths>) {
    let stack = if fill_width { Stack::new().fill_width().id_salt("fill") } else { Stack::new().id_salt("hug") };
    let shown = stack.show(ui, |ui, _| {
        let text = Text::new(LONG).show(ui).rect.width();
        let divider = Divider::horizontal().show(ui).rect.width();
        let table = groups.map_or(0.0, |groups| MetadataTable::new("Metadata", groups).show(ui).table.response.rect.width());
        let button = Button::new("Action").show(ui).rect.width();
        Widths { text, divider, table, button, ..Widths::default() }
    });
    if let Some(widths) = shown.inner {
        seen.set(Widths { column: shown.output.rect.width(), ..widths });
    }
}

fn groups() -> Vec<MetadataGroup> {
    vec![MetadataGroup::new("Camera", vec![MetadataRow::new("Exposure", "1/250 s"), MetadataRow::new("ISO", "200")])]
}

/// Shows `body` for `n` frames in a column given the width of the screen, and returns that width.
fn given(frames: &mut Frames, n: usize, mut body: impl FnMut(&mut Ui)) -> f32 {
    let room = Cell::new(0.0);
    frames.idle(n, |ui| {
        Stack::new().fill().show_at(ui, ui.available_rect_before_wrap(), |ui, _| {
            room.set(ui.available_width());
            body(ui);
        });
    });
    room.get()
}

#[trial]
fn a_content_column_takes_the_width_a_column_gives_it_when_a_child_fills() {
    let groups = groups();
    let seen = Cell::new(Widths::default());
    let (nested, in_row, row_text) = (Cell::new(0.0), Cell::new(0.0), Cell::new(0.0));
    let room = given(&mut Frames::new(), 4, |ui| {
        column(ui, false, Some(&groups), &seen);
        Stack::new().id_salt("outer").show(ui, |ui, _| {
            Stack::new().id_salt("inner").show(ui, |ui, _| nested.set(Divider::horizontal().show(ui).rect.width()));
        });
        Row::new().show(ui, |ui, _| {
            let shown = Stack::new().id_salt("in row").show(ui, |ui, _| {
                row_text.set(Text::new("Caption").show(ui).rect.width());
                Divider::horizontal().show(ui);
            });
            in_row.set(shown.output.rect.width());
        });
    });
    let hug = seen.get();
    assert!((hug.column - room).abs() <= SNAP, "a content column holding fill children takes the width it is given");
    assert!((hug.divider - room).abs() <= SNAP && (hug.table - room).abs() <= SNAP, "its fill children take that width");
    assert!(hug.button < hug.column, "a button keeps its own width");
    assert!((nested.get() - room).abs() <= SNAP, "the width reaches down through nested content columns");
    assert!((in_row.get() - row_text.get()).abs() <= SNAP, "a row stops it: a content column in a row is as wide as its content");

    let fill = Cell::new(Widths::default());
    given(&mut Frames::new(), 4, |ui| column(ui, true, Some(&groups), &fill));
    assert!((fill.get().button - hug.button).abs() <= SNAP, "a button keeps its own width in either column");
}

/// A popover sizes from its content: the column in it is as wide as its widest content-sized child, and its fill
/// children show across that width, never setting it.
#[trial]
fn a_content_column_in_a_popover_holds_its_fill_children() {
    let seen = Cell::new(Widths::default());
    Frames::new().idle(4, |ui| {
        Popover::new("Column", Bind::given(true)).show(ui, |ui, _| column(ui, false, None, &seen));
    });
    let hug = seen.get();
    assert!(hug.column > 0.0, "the popover shows its column");
    assert!((hug.column - hug.text).abs() <= SNAP, "the column is as wide as its widest content-sized child");
    assert!((hug.divider - hug.column).abs() <= SNAP, "a divider in it shows across it");
    assert!(hug.button < hug.column, "a button keeps its own width");
}

/// Passing the width on runs no measuring run of its own: a content column in a given column measures as often as a
/// fill-width one, and a settled frame measures nothing.
#[trial]
fn a_content_column_that_passes_its_width_measures_as_often_as_a_fill_width_one() {
    let groups = groups();
    let runs = |fill_width: bool| {
        let mut frames = Frames::new();
        let seen = Cell::new(Widths::default());
        given(&mut frames, 1, |ui| column(ui, fill_width, Some(&groups), &seen));
        let first: u32 = measuring_runs(&frames.ctx).iter().map(|(_, runs)| runs).sum();
        given(&mut frames, 5, |ui| column(ui, fill_width, Some(&groups), &seen));
        (first, last_pass_measuring_runs(&frames.ctx))
    };
    let ((hug, hug_settled), (fill, fill_settled)) = (runs(false), runs(true));
    assert!(hug > 0, "the first frame measures the column");
    assert_eq!(hug, fill, "the content column measures as often as the fill-width one");
    assert!(hug_settled.is_empty() && fill_settled.is_empty(), "a settled frame measures nothing: {hug_settled:?} {fill_settled:?}");
}

const ROWS: [&str; 2] = ["Exposure", "ISO"];

/// A column of a long heading over a fill-width list of short rows, or over a fill-width stack of the same rows when
/// `list` is false. Records the widths of the column, the heading and the list or stack.
fn list_column(ui: &mut Ui, list: bool, seen: &Cell<(f32, f32, f32)>) {
    let shown = Stack::new().id_salt("column").show(ui, |ui, _| {
        let text = Text::new(LONG).show(ui).rect.width();
        let filled = if list {
            let keys = [0_usize, 1];
            let source = keyed(&keys, |key: &usize| *key);
            let rows = List::new("Rows").sizing(Sizing::FillWidth).natural_height(true);
            rows.show_items(ui, &source, &mut |key, _| ListRow::new(ROWS[*key])).response.rect.width()
        } else {
            let stack = Stack::new().fill_width().id_salt("rows").show(ui, |ui, _| ROWS.iter().for_each(|row| _ = Text::new(*row).show(ui)));
            stack.output.rect.width()
        };
        (text, filled)
    });
    if let Some((text, filled)) = shown.inner {
        seen.set((shown.output.rect.width(), text, filled));
    }
}

/// A fill-width collection follows the column it sits in: in a popover's column it measures from its rows, never sets
/// the column's width and shows across it; in a column given its width it takes that width.
#[trial]
fn a_fill_width_list_is_held_in_a_content_column_and_takes_a_given_one() {
    let seen = Cell::new((0.0, 0.0, 0.0));
    Frames::new().idle(4, |ui| {
        Popover::new("Column", Bind::given(true)).show(ui, |ui, _| list_column(ui, true, &seen));
    });
    let (column, text, list) = seen.get();
    assert!(column > 0.0, "the popover shows its column");
    assert!((column - text).abs() <= SNAP, "the column is as wide as its heading, not the screen: {column} vs {text}");
    assert!((list - column).abs() <= SNAP, "the list shows across the column: {list} vs {column}");

    let given_seen = Cell::new((0.0, 0.0, 0.0));
    let room = given(&mut Frames::new(), 4, |ui| list_column(ui, true, &given_seen));
    let (_, _, list) = given_seen.get();
    assert!((list - room).abs() <= SNAP, "in a given column the list takes the width it is given: {list} vs {room}");
}

/// Holding a fill-width list costs the column no measuring run of its own: it measures as often as the same column
/// holding a fill-width stack, and a settled frame measures nothing.
#[trial]
fn a_column_holding_a_fill_width_list_measures_as_often_as_one_holding_a_stack() {
    let runs = |list: bool| {
        let mut frames = Frames::new();
        let seen = Cell::new((0.0, 0.0, 0.0));
        let show = |frames: &mut Frames, n: usize| {
            frames.idle(n, |ui| {
                Popover::new("Column", Bind::given(true)).show(ui, |ui, _| list_column(ui, list, &seen));
            });
        };
        show(&mut frames, 1);
        let first: u32 = measuring_runs(&frames.ctx).iter().map(|(_, runs)| runs).sum();
        show(&mut frames, 5);
        (first, last_pass_measuring_runs(&frames.ctx))
    };
    let ((list, list_settled), (stack, stack_settled)) = (runs(true), runs(false));
    assert!(stack > 0, "the first frame measures the column");
    assert!(list <= stack, "the list's column measures no more than the stack's: {list} vs {stack}");
    assert!(list_settled.is_empty() && stack_settled.is_empty(), "a settled frame measures nothing: {list_settled:?} {stack_settled:?}");
}

/// A content-sized stack shown at a rect is given that rect's width: a fill-width list and a fill-width stack in it
/// take the width, as they do in a column that fills (a split view's pane).
#[trial]
fn a_stack_shown_at_a_rect_gives_its_fill_children_the_rect_width() {
    let (list, stack, wide) = (Cell::new(0.0), Cell::new(0.0), Cell::new(0.0));
    Frames::new().idle(4, |ui| {
        let rect = egui::Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(600.0, 300.0));
        wide.set(rect.width());
        Stack::new().id_salt("pane").show_at(ui, rect, |ui, _| {
            let keys = [0_usize, 1];
            let source = keyed(&keys, |key: &usize| *key);
            let rows = List::new("Rows").sizing(Sizing::FillWidth).natural_height(true);
            list.set(rows.show_items(ui, &source, &mut |key, _| ListRow::new(ROWS[*key])).response.rect.width());
            let shown = Stack::new().fill_width().id_salt("rows").show(ui, |ui, _| _ = Text::new("Caption").show(ui));
            stack.set(shown.output.rect.width());
        });
    });
    assert!((list.get() - wide.get()).abs() <= SNAP, "the list takes the rect's width: {} vs {}", list.get(), wide.get());
    assert!((stack.get() - wide.get()).abs() <= SNAP, "the stack takes the rect's width: {} vs {}", stack.get(), wide.get());
}
