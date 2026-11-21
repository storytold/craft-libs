//! The row line: a child of an unwrapped row that is one line sits with its centre on the row's line; a child of more
//! than one line (a column, a field with a help line below) sits at the row's top. A control with its label after it
//! or above it, and a field labelled above, sit on the line by their control, as a bare control does, in a row and in
//! a grid's cell.

use craft_behavior::ColumnWidth;
use craft_theme::ControlSize;
use craft_ui::buttons::Button;
use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, FieldLayout, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::form::{Field, Help};
use craft_ui::layout_components::{Grid, Row, Stack};
use craft_ui::runtime::env::Label;
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
