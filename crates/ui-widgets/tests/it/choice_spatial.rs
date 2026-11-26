//! `RadioGroup` with `NavShape::Spatial`: the arrows move to the member that sits in their direction.

use craft_behavior::NavShape;
use craft_ui::choice::{Radio, RadioGroup};
use craft_ui::contract::{Bind, Props as _, Show as _, ShowContainer as _};
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// Four members in two rows: 1 2 over 3 4.
fn grid(value: &Rc<Cell<u8>>) -> Fixture {
    let held = Rc::clone(value);
    let mut fixture = Fixture::new(move |ui| {
        let mut now = held.get();
        RadioGroup::new("Corner", Bind::bound(&mut now)).nav(NavShape::Spatial).show(ui, |ui, _| {
            Stack::new().show(ui, |ui, _| {
                for row in [[1u8, 2], [3, 4]] {
                    Row::new().show(ui, |ui, _| {
                        for member in row {
                            Radio::new(member).label(format!("Corner {member}")).show(ui);
                        }
                    });
                }
            });
        });
        held.set(now);
    });
    fixture.settle();
    fixture
}

/// Arrows move the focus without picking, as in any radio group; Enter picks the focused member.
#[trial]
fn arrows_in_a_spatial_radio_group_move_to_the_member_in_their_direction() {
    let value = Rc::new(Cell::new(1u8));
    let mut fixture = grid(&value);
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert!(fixture.focused_stop().is_some(), "Tab lands on the group's chosen member");
    for (arrow, expected, what) in [
        (Key::ArrowRight, 2, "Right moves to the member beside it"),
        (Key::ArrowDown, 4, "Down moves to the member below"),
        (Key::ArrowLeft, 3, "Left moves to the member beside it"),
        (Key::ArrowUp, 1, "Up moves to the member above"),
    ] {
        fixture.key(arrow, Modifiers::NONE).settle();
        fixture.key(Key::Enter, Modifiers::NONE).settle();
        assert_eq!(value.get(), expected, "{what}, which Enter then picks");
    }
}

/// Right to left, the rows run the other way but the arrows still step by where the members sit: Left reaches the
/// member drawn to its left, and there is none to its right.
#[trial]
fn arrows_in_a_right_to_left_spatial_group_step_by_the_screen() {
    let value = Rc::new(Cell::new(1u8));
    let mut fixture = grid(&value);
    fixture.direction(craft_i18n::Direction::Rtl).settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    assert!(fixture.focused_stop().is_some(), "Tab lands on the group's chosen member");
    fixture.key(Key::ArrowLeft, Modifiers::NONE).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), 2, "Left reaches the member drawn to the left, which is the group's second");
    let value = Rc::new(Cell::new(1u8));
    let mut fixture = grid(&value);
    fixture.direction(craft_i18n::Direction::Rtl).settle();
    fixture.key(Key::Tab, Modifiers::NONE).settle();
    fixture.key(Key::ArrowRight, Modifiers::NONE).settle();
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(value.get(), 1, "there is no member to the right of the first, the rightmost");
}
