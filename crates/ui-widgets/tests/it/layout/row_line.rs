//! The row line: a child of an unwrapped row that is one line sits with its centre on the row's line; a child of more
//! than one line (a column, a field with a help line below) sits at the row's top. A control with its label after it
//! or above it, and a field labelled above, sit on the line by their control, as a bare control does, in a row and in
//! a grid's cell. Components an app shows in an egui container inside a row or grid are laid out by that container:
//! they take neither the row's line nor the grid's cells. In an `egui::Grid`'s cell a labelled control decides its
//! label at its own width.

use craft_behavior::ColumnWidth;
use craft_theme::ControlSize;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, FieldLayout, LabelScheme, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup, Help};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::runtime::env::{Label, with_env};
use craft_ui::testing::{Fixture, last_pass_measuring_runs, trial};
use egui::Rect;
use std::cell::Cell;
use std::rc::Rc;

/// Half a pixel at the scales the box runs.
const ROUNDING: f32 = 0.5;

fn settled(content: impl FnMut(&mut egui::Ui) + 'static) -> Fixture {
    let mut fixture = Fixture::new(content);
    fixture.settle();
    fixture.idle(3);
    fixture
}

/// The rect of the control named by any of `names` (a field's control takes the field's label as its name).
fn control(fixture: &Fixture, names: &[&str]) -> Rect {
    names.iter().find_map(|name| fixture.by_label(name)).unwrap_or_else(|| panic!("none of {names:?}")).rect
}

fn text(fixture: &Fixture, text: &str) -> Rect {
    fixture.texts().iter().find(|painted| painted.text == text).unwrap_or_else(|| panic!("no {text} painted")).rect
}

/// A side-labelled field holding a row of small buttons beside one holding a standard button (the Layers panel's
/// Lock and Fill): the controls share a centre line. A settled frame runs no measuring run.
#[trial]
fn fields_of_different_heights_in_a_row_share_a_centre_line() {
    let fixture = settled(|ui| {
        Row::new().show(ui, |ui, _| {
            Field::new("Lock").sizing(Sizing::Content).show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    Button::new("A").size(ControlSize::Sm).show(ui);
                    Button::new("B").size(ControlSize::Sm).show(ui);
                });
            });
            Field::new("Fill").sizing(Sizing::Content).show(ui, |ui, _| {
                Button::new("Go").show(ui);
            });
        });
    });
    let (small, standard) = (control(&fixture, &["A"]), control(&fixture, &["Go", "Fill"]));
    assert!(small.height() < standard.height(), "the small button {small:?} is shorter than {standard:?}");
    // A button's text sits at its centre.
    let (small, standard) = (text(&fixture, "A"), text(&fixture, "Go"));
    assert!((small.center().y - standard.center().y).abs() <= ROUNDING, "small {small:?}, standard {standard:?}");
    assert!(last_pass_measuring_runs(fixture.ctx()).is_empty(), "a settled frame measured");
}

/// A field showing a help line is two lines and sits at the row's top, beside a one-line field as tall as its control
/// and label: the controls share a centre line and the help hangs below.
#[trial]
fn a_help_line_hangs_below_its_control_on_the_row_line() {
    let fixture = settled(|ui| {
        Row::new().show(ui, |ui, _| {
            Field::new("Width").help(Help::Text(Label::from("At least 1 px"))).sizing(Sizing::Content).show(ui, |ui, _| {
                Button::new("W").show(ui);
            });
            Field::new("Height").sizing(Sizing::Content).show(ui, |ui, _| {
                Button::new("H").show(ui);
            });
        });
    });
    let (width, height) = (text(&fixture, "W"), text(&fixture, "H"));
    assert!((width.center().y - height.center().y).abs() <= ROUNDING, "with help {width:?}, without {height:?}");
    assert!(text(&fixture, "At least 1 px").top() > width.bottom(), "the help line sits under its control");
}

/// Columns of more than one line share the row's top (a style list beside a page), and the one-line children (a
/// column of one text, a button) share the line.
#[trial]
fn columns_sit_at_the_rows_top_and_lines_on_the_line() {
    let fixture = settled(|ui| {
        Row::new().show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                Text::new("Short").show(ui);
            });
            Stack::new().show(ui, |ui, _| {
                Text::new("A").show(ui);
                Text::new("B").show(ui);
            });
            Stack::new().show(ui, |ui, _| {
                for line in ["One", "Two", "Three", "Four"] {
                    Text::new(line).show(ui);
                }
            });
            Button::new("Go").show(ui);
        });
    });
    let (two, four) = (text(&fixture, "A"), text(&fixture, "One"));
    assert!((two.top() - four.top()).abs() <= ROUNDING, "two-line column at {}, four-line column at {}", two.top(), four.top());
    let (short, button) = (text(&fixture, "Short"), text(&fixture, "Go"));
    assert!((short.center().y - button.center().y).abs() <= ROUNDING, "one-line column {short:?}, button {button:?}");
    assert!(four.top() <= control(&fixture, &["Go"]).top() + ROUNDING, "the four-line column starts below the button's top");
}

/// A field labelled above its control sits on the row's line by its control, as a control labelled above does: a
/// button beside it sits level with the control, and the label stands above the line.
#[trial]
fn a_field_labelled_above_sits_on_the_row_line_by_its_control() {
    let fixture = settled(|ui| {
        Row::new().show(ui, |ui, _| {
            Field::new("Caption").layout(FieldLayout::Above).sizing(Sizing::Content).show(ui, |ui, _| {
                Button::new("X").show(ui);
            });
            Button::new("Y").show(ui);
        });
    });
    let (caption, control, button) = (text(&fixture, "Caption"), text(&fixture, "X"), text(&fixture, "Y"));
    assert!(caption.bottom() < control.top(), "the label sits above its control");
    assert!((button.center().y - control.center().y).abs() <= ROUNDING, "the button {button:?} sits on the control's line {control:?}");
}

/// A check box whose label runs to a second line under it, beside a button in a row: its box sits on the button's
/// line and the label's second line hangs below. In a grid's cell, beside a taller cell, a check box with its label
/// after it sits on the line of the button in the next cell.
#[trial]
fn a_labelled_check_box_sits_with_its_box_on_a_buttons_line_in_a_row_and_a_grid_cell() {
    const LONG: &str = "Smooth the edges of every selection the tool makes";
    for grid in [false, true] {
        let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
        let out = Rc::clone(&seen);
        let fixture = settled(move |ui| {
            let room = Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(260.0, 400.0));
            let out = Rc::clone(&out);
            Stack::new().fill().show_at(ui, room, move |ui, _| {
                if grid {
                    Grid::new(&[ColumnWidth::Content, ColumnWidth::Content, ColumnWidth::Content]).show(ui, |ui, _| {
                        Stack::new().show(ui, |ui, _| {
                            Text::new("A").show(ui);
                            Text::new("B").show(ui);
                        });
                        let button = Button::new("Go").show(ui).rect;
                        let check = Checkbox::new(Bind::given(true)).label("Snap").show(ui).response.rect;
                        out.set((button, check));
                    });
                } else {
                    Row::new().show(ui, |ui, _| {
                        let button = Button::new("Go").show(ui).rect;
                        let check = Checkbox::new(Bind::given(true)).label(LONG).show(ui).response.rect;
                        out.set((button, check));
                    });
                }
            });
        });
        let (button, check) = seen.get();
        let at = if grid { "in a grid's cell" } else { "in a row" };
        if !grid {
            assert!(text(&fixture, LONG).bottom() > check.bottom() + ROUNDING, "the label runs to a second line under the box {check:?}");
        }
        assert!((button.center().y - check.center().y).abs() <= ROUNDING, "{at}: the box {check:?} sits on the button's line {button:?}");
    }
}

const GRID_ROWS: usize = 3;
type GridBoxes = [(Rect, Rect); GRID_ROWS];

/// Two columns of labelled check boxes in an `egui::Grid`, each box run through `wrap` (`ui.show` or a scope around
/// it) with its index: the box rects of each row.
fn boxes_grid(ui: &mut egui::Ui, wrap: Wrap) -> GridBoxes {
    let mut boxes = [(Rect::NOTHING, Rect::NOTHING); GRID_ROWS];
    egui::Grid::new("boxes").num_columns(2).spacing([24.0, 6.0]).show(ui, |ui| {
        for (row, rect) in boxes.iter_mut().enumerate() {
            rect.0 = wrap(ui, 2 * row, Checkbox::new(Bind::given(true)).label(format!("Left {row}")));
            rect.1 = wrap(ui, 2 * row + 1, Checkbox::new(Bind::given(false)).label(format!("Right {row}")));
            ui.end_row();
        }
    });
    boxes
}

/// How `boxes_grid` shows each check box in its cell.
type Wrap = fn(&mut egui::Ui, usize, Checkbox<'_>) -> Rect;

fn plain(ui: &mut egui::Ui, _: usize, check: Checkbox<'_>) -> Rect {
    check.show(ui).response.rect
}

fn read_only(ui: &mut egui::Ui, _: usize, check: Checkbox<'_>) -> Rect {
    with_env(ui, |set| set.read_only = Some(true), |ui| check.show(ui).response.rect)
}

fn pushed(ui: &mut egui::Ui, index: usize, check: Checkbox<'_>) -> Rect {
    ui.push_id(index, |ui| check.show(ui).response.rect).inner
}

/// The grid of `boxes_grid` in a 300 pt column, inside an unwrapped `Row` or a `Stack`, with a button after it.
fn grid_in(in_row: bool, wrap: Wrap) -> (GridBoxes, Rect) {
    let (boxes, after, _) = grid_shown(in_row, wrap);
    (boxes, after)
}

/// `grid_in`, with the fixture it ran in.
fn grid_shown(in_row: bool, wrap: Wrap) -> (GridBoxes, Rect, Fixture) {
    let seen = Rc::new(Cell::new(([(Rect::NOTHING, Rect::NOTHING); GRID_ROWS], Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let fixture = settled(move |ui| {
        let room = Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(300.0, 600.0));
        let out = Rc::clone(&out);
        Stack::new().fill().show_at(ui, room, move |ui, _| {
            let mut boxes = [(Rect::NOTHING, Rect::NOTHING); GRID_ROWS];
            if in_row {
                Row::new().fill().show(ui, |ui, _| boxes = boxes_grid(ui, wrap));
            } else {
                Stack::new().fill().show(ui, |ui, _| boxes = boxes_grid(ui, wrap));
            }
            let after = Button::new("After").show(ui).rect;
            out.set((boxes, after));
        });
    });
    let (boxes, after) = seen.get();
    (boxes, after, fixture)
}

fn pitch(boxes: &GridBoxes) -> f32 {
    boxes[1].0.top() - boxes[0].0.top()
}

/// Check boxes an app lays out in an `egui::Grid` inside a row (a dialog's custom row of two columns of check boxes)
/// are no children of the row: the grid's rows are as high as the same grid's outside a row, not as high as the row's
/// line, and what follows the row starts under the grid.
#[trial]
fn check_boxes_in_an_egui_grid_in_a_row_keep_their_own_rows() {
    let (alone, _) = grid_in(false, plain);
    let (boxes, after) = grid_in(true, plain);
    assert!((pitch(&boxes) - pitch(&alone)).abs() <= ROUNDING, "the grid rows in a row {boxes:?} are as high as outside one {alone:?}");
    assert!(after.top() >= boxes[GRID_ROWS - 1].0.bottom() - ROUNDING, "the button {after:?} is under the grid {boxes:?}");
}

/// The same check boxes, each in a scope of its own in its cell (an env scope, `with_env`, or `ui.push_id`): the scope
/// holds none of the row's facts, so the grid's rows stay as high as the same grid's outside a row, and the boxes are
/// still in the grid's cells: the labels stay on one line and the two columns share each row.
#[trial]
fn check_boxes_in_scopes_in_an_egui_grid_in_a_row_keep_their_own_rows_and_lines() {
    for (scope, wrap) in [("with_env", read_only as Wrap), ("push_id", pushed)] {
        let (alone, _) = grid_in(false, wrap);
        let (boxes, after, fixture) = grid_shown(true, wrap);
        assert!((pitch(&boxes) - pitch(&alone)).abs() <= ROUNDING, "{scope}: the grid rows in a row {boxes:?} are as high as outside one {alone:?}");
        let line = text(&fixture, "After").height();
        for (row, (left, right)) in boxes.iter().enumerate() {
            for name in [format!("Left {row}"), format!("Right {row}")] {
                let label = text(&fixture, &name);
                assert!(label.height() < line * 1.5, "{scope}: the label {name} {label:?} stays on one line");
            }
            assert!((left.center().y - right.center().y).abs() <= ROUNDING, "{scope}: the two columns share each row: {left:?} {right:?}");
            assert!(right.left() >= left.right() - ROUNDING, "{scope}: the right column {right:?} follows the left {left:?}");
        }
        assert!(after.top() >= boxes[GRID_ROWS - 1].0.bottom() - ROUNDING, "{scope}: the button {after:?} is under the grid {boxes:?}");
    }
}

/// Two columns of labelled check boxes in an `egui::Grid` share each row: the first column's labels are decided at
/// their own width, not wrapped in the narrow column egui offers a column it has no width for yet.
#[trial]
fn two_columns_of_labelled_check_boxes_in_an_egui_grid_share_each_row() {
    let (boxes, _) = grid_in(false, plain);
    for (left, right) in boxes {
        assert!((left.center().y - right.center().y).abs() <= ROUNDING, "the two columns share each row: {left:?} {right:?}");
        assert!(right.left() >= left.right() - ROUNDING, "the right column {right:?} follows the left {left:?}");
    }
}

/// In a form whose labelled rows share a label column, check boxes in an `egui::Grid` in a full-width row (a dialog's
/// custom row) each sit at their cell's start: none takes the form's label-column indent.
#[trial]
fn check_boxes_in_an_egui_grid_in_a_forms_row_sit_at_their_cells_start() {
    let seen = Rc::new(Cell::new([(Rect::NOTHING, Rect::NOTHING); GRID_ROWS]));
    let out = Rc::clone(&seen);
    let _fixture = settled(move |ui| {
        let room = Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(400.0, 600.0));
        let out = Rc::clone(&out);
        Stack::new().fill().show_at(ui, room, move |ui, _| {
            FieldGroup::new("Mode").labels(LabelScheme::Form).show(ui, |ui, _| {
                Field::new("Name").show(ui, |ui, _| {
                    Button::new("Pick").show(ui);
                });
                Row::new().fill().show(ui, |ui, _| out.set(boxes_grid(ui, plain)));
            });
        });
    });
    let boxes = seen.get();
    for (row, (left, _)) in boxes.iter().enumerate() {
        assert!((left.left() - boxes[0].0.left()).abs() <= ROUNDING, "row {row}'s box {left:?} starts where the first {:?} does", boxes[0].0);
    }
}

/// A craft `Grid`'s cells are its craft children. A `ui.horizontal` an app shows in one is no cell: egui lays out its
/// check boxes side by side on one line and places it below the cells placed so far, at the grid's start edge, and the
/// next craft child takes the next cell, beside "Name".
#[trial]
fn check_boxes_in_an_egui_container_in_a_grid_take_no_cells() {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 5]));
    let out = Rc::clone(&seen);
    let fixture = settled(move |ui| {
        let room = Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(400.0, 400.0));
        let out = Rc::clone(&out);
        Stack::new().fill().show_at(ui, room, move |ui, _| {
            let mut pair = (Rect::NOTHING, Rect::NOTHING);
            Grid::new(&[ColumnWidth::Content, ColumnWidth::Content]).show(ui, |ui, _| {
                Text::new("Name").show(ui);
                Text::new("Size").show(ui);
                pair = ui
                    .horizontal(|ui| {
                        let one = Checkbox::new(Bind::given(true)).label("One").show(ui).response.rect;
                        let two = Checkbox::new(Bind::given(true)).label("Two").show(ui).response.rect;
                        (one, two)
                    })
                    .inner;
            });
            let after = Button::new("After").show(ui).rect;
            out.set([pair.0, pair.1, after, Rect::NOTHING, Rect::NOTHING]);
        });
    });
    let [one, two, after, ..] = seen.get();
    let (name, size) = (text(&fixture, "Name"), text(&fixture, "Size"));
    assert!((one.center().y - two.center().y).abs() <= ROUNDING && two.left() >= one.right() - ROUNDING, "One {one:?}, then Two {two:?} on its line");
    assert!(
        (size.center().y - name.center().y).abs() <= ROUNDING && size.left() >= name.right() - ROUNDING,
        "Size {size:?} takes the cell beside Name {name:?}"
    );
    assert!(one.top() >= name.bottom() - ROUNDING, "the pair {one:?} lies under Name's row {name:?}");
    assert!(one.left() <= name.left() + ROUNDING, "the pair {one:?} starts at the grid's start edge, as Name {name:?} does");
    assert!(after.top() >= one.bottom().max(two.bottom()) - ROUNDING, "the grid {after:?} reaches past the pair {one:?} {two:?}");
}

/// A check box in a `ui.horizontal` too narrow for its label wraps the label at the room's end: egui gives the
/// horizontal the room it has, and nothing runs past it.
#[trial]
fn a_check_box_in_a_narrow_horizontal_wraps_its_label_at_the_rooms_end() {
    const LONG: &str = "Second option with a long name";
    const WIDTH: f32 = 160.0;
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        let out = Rc::clone(&out);
        Stack::new().fill().show(ui, move |ui, _| {
            let pair = ui.horizontal(|ui| {
                let one = Checkbox::new(Bind::given(true)).label("One").show(ui).response.rect;
                let two = Checkbox::new(Bind::given(false)).label(LONG).show(ui).response.rect;
                (one, two)
            });
            out.set(pair.inner);
        });
    });
    fixture.room(egui::vec2(WIDTH, 200.0)).settle();
    fixture.idle(3);
    let (_, two) = seen.get();
    let (label, one_line) = (text(&fixture, LONG), text(&fixture, "One"));
    assert!(label.right() <= WIDTH + ROUNDING, "the label {label:?} ends inside the room");
    assert!(label.height() > one_line.height() * 1.5, "the label {label:?} wraps to a second line beside its box {two:?}");
}

/// One row of two labelled check boxes in an `egui::Grid` shown directly in a `Stack`, or in a `Row` in it (a dialog's
/// custom row), in a 400 pt room under `direction`; `long` lengthens the labels. Records the two boxes and the grid's
/// rect.
fn one_row_grid(in_row: bool, direction: craft_i18n::Direction, long: &Rc<Cell<bool>>) -> (Fixture, Rc<Cell<[Rect; 3]>>) {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 3]));
    let (out, lengthen) = (Rc::clone(&seen), Rc::clone(long));
    let mut fixture = Fixture::new(move |ui| {
        let (out, long) = (Rc::clone(&out), lengthen.get());
        let grid = move |ui: &mut egui::Ui| {
            let mut boxes = [Rect::NOTHING; 3];
            boxes[2] = egui::Grid::new("boxes")
                .num_columns(2)
                .spacing([24.0, 6.0])
                .show(ui, |ui| {
                    for (column, rect) in boxes.iter_mut().take(2).enumerate() {
                        *rect = Checkbox::new(Bind::given(true)).label(column_name(column, long)).show(ui).response.rect;
                    }
                    ui.end_row();
                })
                .response
                .rect;
            out.set(boxes);
        };
        Stack::new().fill().show(ui, move |ui, _| {
            if in_row {
                Row::new().fill().show(ui, |ui, _| grid(ui));
            } else {
                grid(ui);
            }
        });
    });
    fixture.direction(direction).room(egui::vec2(400.0, 200.0)).settle();
    (fixture, seen)
}

fn column_name(column: usize, long: bool) -> String {
    if long { format!("Column {column} with a longer name") } else { format!("C{column}") }
}

/// What the cells of `one_row_grid` show: each column's box and label.
fn cells(fixture: &Fixture, boxes: [Rect; 3]) -> [Rect; 2] {
    [0, 1].map(|column| boxes[column].union(text(fixture, &column_name(column, true))))
}

/// Under right-to-left, labelled check boxes in an `egui::Grid` shown directly in a right-to-left body, or in a row
/// there, stay in their cells: egui lays a grid out from its left edge in any direction, so the grid's rect is finite,
/// on the frame the labels lengthen the first column's box and label stay right of the grid's left edge, once settled
/// the second column stays clear of the first, and the cells take as much width as under left-to-right: the last
/// column is as wide as its box and label, not the rest of the room.
#[trial]
fn check_boxes_in_an_egui_grid_stay_in_their_cells_under_rtl() {
    for in_row in [false, true] {
        let ltr = {
            let long = Rc::new(Cell::new(true));
            let (mut fixture, seen) = one_row_grid(in_row, craft_i18n::Direction::Ltr, &long);
            fixture.settle();
            cells(&fixture, seen.get())
        };
        let long = Rc::new(Cell::new(false));
        let (mut fixture, seen) = one_row_grid(in_row, craft_i18n::Direction::Rtl, &long);
        let grid = seen.get()[2];
        assert!(grid.is_finite(), "in a row {in_row}: the grid's rect {grid:?} is finite");
        long.set(true);
        fixture.frame(vec![]);
        for (part, rect) in [("box", seen.get()[0]), ("label", text(&fixture, &column_name(0, true)))] {
            assert!(
                rect.left() >= grid.left() - ROUNDING,
                "in a row {in_row}: the first column's {part} {rect:?} stays right of the grid's left edge {grid:?}"
            );
        }
        fixture.settle();
        let [first, second] = cells(&fixture, seen.get());
        assert!(second.left() >= first.right() - ROUNDING, "in a row {in_row}: the second column {second:?} is clear of the first {first:?}");
        let (width, ltr_width) = (first.union(second).width(), ltr[0].union(ltr[1]).width());
        assert!((width - ltr_width).abs() <= ROUNDING, "in a row {in_row}: the cells take {width} under RTL, as under LTR ({ltr_width})");
    }
}

/// A labelled container in an `egui::Grid`'s first column decides its label at its own width: the label stays on one
/// line, and the two columns share the row.
#[trial]
fn a_labelled_row_in_an_egui_grid_keeps_its_label_on_one_line() {
    const LONG: &str = "Spacing between the rows";
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let fixture = settled(move |ui| {
        let room = Rect::from_min_size(ui.available_rect_before_wrap().min, egui::vec2(400.0, 300.0));
        let out = Rc::clone(&out);
        Stack::new().fill().show_at(ui, room, move |ui, _| {
            egui::Grid::new("rows").num_columns(2).show(ui, |ui| {
                let mut first = Rect::NOTHING;
                Field::new(LONG).show(ui, |ui, _| first = Button::new("Go").show(ui).rect);
                let second = Button::new("Next").show(ui).rect;
                ui.end_row();
                out.set((first, second));
            });
        });
    });
    let (first, second) = seen.get();
    let (label, go) = (text(&fixture, LONG), text(&fixture, "Go"));
    assert!(label.height() < go.height() * 1.5, "the label {label:?} stays on one line");
    assert!((first.center().y - second.center().y).abs() <= ROUNDING, "the two columns share the row: {first:?} {second:?}");
}

/// A small labelled check box in a plain `Lazy` in a row beside a button sits with its box on the button's line: a
/// `Lazy` runs its content as the row's direct children.
#[trial]
fn a_labelled_check_box_in_a_lazy_in_a_row_sits_on_the_rows_line() {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let _fixture = settled(move |ui| {
        let out = Rc::clone(&out);
        Row::new().show(ui, |ui, _| {
            let button = Button::new("Go").show(ui).rect;
            let mut check = Rect::NOTHING;
            craft_ui::contract::Lazy::new("check", 0.0).show(ui, |ui| {
                check = Checkbox::new(Bind::given(true)).label("Snap").size(ControlSize::Sm).show(ui).response.rect;
            });
            out.set((button, check));
        });
    });
    let (button, check) = seen.get();
    assert!((button.center().y - check.center().y).abs() <= ROUNDING, "the box {check:?} sits on the button's line {button:?}");
}
