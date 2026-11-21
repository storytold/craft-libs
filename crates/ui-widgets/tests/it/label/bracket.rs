//! A bracketed row and the row a check box starts: the chain halfway between its fields with the bracket drawn to
//! their controls, and a check box that starts a row of a form in the form's control column.

use craft_ui::choice::Checkbox;
use craft_ui::contract::{Bind, Bracket, Entry, Props as _, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::{Field, FieldGroup};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::pickers::{Select, UnitSelect};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::{NumberField, link_toggle};
use egui::{Rect, Shape, Vec2};

/// How far two edges may differ and still line up, in points.
const SLACK: f32 = 0.5;

fn control(fixture: &Fixture, name: &str) -> Rect {
    fixture.by_label(name).unwrap_or_else(|| panic!("{name:?} names its control")).rect
}

/// The horizontal line segments of one frame, as (y, from x, to x).
fn ticks(fixture: &mut Fixture) -> Vec<(f32, f32, f32)> {
    fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points: [a, b], .. } if (a.y - b.y).abs() < SLACK => Some((a.y, a.x.min(b.x), a.x.max(b.x))),
            _ => None,
        })
        .collect()
}

/// A form with Width and Height (unit selects) stacked in a row with `bracket`, beside their link.
fn linked(bracket: Bracket) -> Fixture {
    let units = [Entry::new(UnitKind::Pixels, "Pixels"), Entry::new(UnitKind::Percent, "Percent")];
    Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| {
            FieldGroup::new("Image size").show(ui, |ui, _| {
                Row::new().bracket(bracket).show(ui, |ui, _| {
                    Stack::new().show(ui, |ui, _| {
                        for name in ["Width", "Height"] {
                            Field::new(name).show(ui, |ui, _| {
                                UnitSelect::new(name, Bind::given(Some(8.0)), Bind::given(UnitKind::Pixels), &units)
                                    .unit_label(format!("{name} unit"))
                                    .show(ui);
                            });
                        }
                    });
                    link_toggle("Link", Bind::given(bracket == Bracket::Linked)).show(ui);
                });
            });
        });
    })
}

/// A linked bracket runs from the end of Width's and Height's controls to the chain's centre line, at each control's
/// centre, and the chain sits halfway between the two; unlinked, the chain keeps its place and no bracket is drawn.
/// At 1x and 1.25x.
#[trial]
fn a_bracketed_row_centres_its_chain_between_the_rows_and_draws_the_bracket_while_linked() {
    for scale in [1.0, 1.25] {
        for bracket in [Bracket::Linked, Bracket::Unlinked] {
            let mut fixture = linked(bracket);
            fixture.room(Vec2::new(1000.0, 600.0)).pixels_per_point(scale);
            fixture.settle();
            let (width, height, link) = (control(&fixture, "Width unit"), control(&fixture, "Height unit"), control(&fixture, "Link"));
            let middle = (width.center().y + height.center().y) / 2.0;
            assert!((link.center().y - middle).abs() <= SLACK, "{bracket:?} at {scale}x: the chain sits halfway between the rows: {link:?} {middle}");
            let drawn = ticks(&mut fixture);
            for (name, row) in [("Width", width), ("Height", height)] {
                let tick = drawn.iter().find(|(y, from, to)| (y - row.center().y).abs() <= 1.0 && *from >= row.max.x && *to >= link.center().x - 1.0);
                match bracket {
                    Bracket::Linked => {
                        assert!(tick.is_some(), "at {scale}x a tick runs from {name}'s control to the chain: {drawn:?} {row:?} {link:?}");
                    }
                    _ => assert!(tick.is_none(), "unlinked at {scale}x, no tick on {name}'s row: {tick:?}"),
                }
            }
        }
    }
}

/// A check box that starts a row of a form (Resample with its method after it) starts in the form's control column;
/// one after another child of the row stays where the row puts it.
#[trial]
fn a_check_box_that_starts_a_row_of_a_form_starts_in_the_control_column() {
    let methods = [Entry::new(0_u8, "Automatic"), Entry::new(1_u8, "Bicubic")];
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| {
            FieldGroup::new("Size").show(ui, |ui, _| {
                Field::new("Resolution").show(ui, |ui, _| {
                    NumberField::new("Resolution", Bind::given(Some(72.0))).show(ui);
                });
                Row::new().show(ui, |ui, _| {
                    Checkbox::new(Bind::given(true)).label("Resample").show(ui);
                    Select::new("Method", Bind::given(0_u8), &methods).show(ui);
                });
                Row::new().show(ui, |ui, _| {
                    Text::new("Then").show(ui);
                    Checkbox::new(Bind::given(false)).label("Second").show(ui);
                });
            });
        });
    });
    fixture.room(Vec2::new(1000.0, 600.0));
    fixture.settle();
    let (field, check, method) = (control(&fixture, "Resolution"), control(&fixture, "Resample"), control(&fixture, "Method"));
    assert!((check.min.x - field.min.x).abs() <= SLACK, "the check box starts at the form's control edge: {check:?} {field:?}");
    assert!(
        method.min.x >= check.max.x - SLACK && method.min.y < check.max.y && method.max.y > check.min.y,
        "the method follows on its line: {method:?} {check:?}"
    );
    let second = control(&fixture, "Second");
    assert!(second.min.x < field.min.x, "a check box after another child of the row is not moved to the control column: {second:?} {field:?}");
}
