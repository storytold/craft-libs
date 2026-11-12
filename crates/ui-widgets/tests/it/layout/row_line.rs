//! The row line: a child of an unwrapped row that is one line sits with its centre on the row's line; a child of more
//! than one line (a column, a field with its label above or a help line below) sits at the row's top.

use craft_theme::ControlSize;
use craft_ui::buttons::Button;
use craft_ui::contract::{FieldLayout, Props as _, Show as _, ShowContainer as _, Sizing};
use craft_ui::display::Text;
use craft_ui::form::{Field, Help};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::runtime::env::Label;
use craft_ui::testing::{Fixture, last_pass_measuring_runs, trial};
use egui::Rect;

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

/// A field labelled above its control is two lines and sits at the row's top, so a button beside it sits level with
/// the label, not the control.
#[trial]
fn a_field_labelled_above_sits_at_the_rows_top() {
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
    assert!(button.center().y < control.top(), "the button {button:?} sits level with the label, above the control {control:?}");
}
