//! The row-room rule: a container offers its children the room it was offered, and a child that fills an axis
//! takes that room only where the container is given the axis; on an axis the container sizes from its content,
//! the child takes its measured size.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::form::Field;
use craft_ui::layout_components::{Row, Stack};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::TextField;
use egui::{Rect, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const ROOM: Vec2 = Vec2::new(800.0, 600.0);
const SLACK: f32 = 0.5;
/// The rects of a wrapping row's leading button and of the row nested after it, in a room `width` wide.
fn wrapped(width: f32) -> (Rect, Rect) {
    let seen = Rc::new(Cell::new((Rect::NOTHING, Rect::NOTHING)));
    let out = Rc::clone(&seen);
    let mut fixture = Fixture::new(move |ui| {
        Row::new().wrap(true).show(ui, |ui, _| {
            let lead = Button::new("Pages from").show(ui).rect;
            let nested = Row::new().show(ui, |ui, _| {
                Button::new("to").show(ui);
                Button::new("Last page").show(ui);
            });
            out.set((lead, nested.output.rect));
        });
    });
    fixture.room(Vec2::new(width, ROOM.y)).settle();
    seen.get()
}

#[trial]
fn a_row_too_wide_for_the_rest_of_a_wrapping_row_moves_to_the_next_line_whole() {
    let (lead, nested) = wrapped(ROOM.x);
    assert!((nested.top() - lead.top()).abs() <= SLACK, "the nested row fits beside the lead: lead {lead:?}, nested {nested:?}");
    let width = nested.right() - lead.left() - 1.0;
    let (lead, nested) = wrapped(width);
    assert!(nested.top() >= lead.bottom() - SLACK, "the nested row wraps below the lead: lead {lead:?}, nested {nested:?}");
    assert!(nested.right() <= width + SLACK, "the wrapped row stays in the room {width}: {nested:?}");
}

/// The rects of a side-labelled field whose row holds a text field and two buttons, in a room of `width`: the
/// field's, then its children's.
fn labelled_row(width: f32) -> [Rect; 4] {
    let seen = Rc::new(Cell::new([Rect::NOTHING; 4]));
    let out = Rc::clone(&seen);
    let mut path = String::from("/home/studio/Documents/brochure-final.pdf");
    let mut fixture = Fixture::new(move |ui| {
        Stack::new().fill().show(ui, |ui, _| {
            let children = Cell::new([Rect::NOTHING; 3]);
            let field = Field::new("Save to").show(ui, |ui, _| {
                Row::new().show(ui, |ui, _| {
                    let text = TextField::new("Path", Bind::bound(&mut path)).show(ui).response.rect;
                    let choose = Button::new("Choose File...").show(ui).rect;
                    let another = Button::new("Another").show(ui).rect;
                    children.set([text, choose, another]);
                })
            });
            let [text, choose, another] = children.get();
            out.set([field.output.rect, text, choose, another]);
        });
    });
    fixture.room(Vec2::new(width, 400.0)).settle();
    seen.get()
}

#[trial]
fn a_row_too_wide_for_its_room_shrinks_its_children_to_fit_in_order() {
    let [_, wide_text, wide_choose, _] = labelled_row(800.0);
    let narrow = 300.0;
    let [field, text, choose, another] = labelled_row(narrow);
    let room_end = field.min.x + narrow;
    assert!(another.max.x <= room_end + SLACK, "the last child ends at {} past the room's end {room_end}", another.max.x);
    assert!(text.min.x < choose.min.x && choose.max.x <= another.min.x + SLACK, "children overlap: {text:?} {choose:?} {another:?}");
    assert!(text.width() > 0.0 && choose.width() > 0.0 && another.width() > 0.0, "a child has no width: {text:?} {choose:?} {another:?}");
    // The shortfall comes off the later children first: the field narrows only once they are at their least.
    assert!(choose.width() < wide_choose.width() - SLACK, "the button kept its width {} in the narrow room", choose.width());
    assert!(text.width() <= wide_text.width() + SLACK, "the field grew: {} from {}", text.width(), wide_text.width());
}
