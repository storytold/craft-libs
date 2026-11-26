//! The root's `ScrubDrags` decides which drags scrub a number field: its body, its side label, both or neither.

use super::START;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _, SideLabel};
use craft_ui::form::Field;
use craft_ui::runtime::env::ScrubDrags;
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::NumberField;
use egui::{Pos2, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const DRAG: f32 = 40.0;

/// Whether a drag from `from` changed the value.
fn dragged(fixture: &mut Fixture, value: &Cell<Option<f64>>, from: Pos2) -> bool {
    let before = value.get();
    fixture.drag_at(from, Vec2::new(DRAG, 0.0));
    value.get() != before
}

/// Whether a drag on the label and one on the body of a number field move its value under `drags`; the default root
/// when `None`.
fn scrubs(drags: Option<ScrubDrags>) -> (bool, bool) {
    let value = Rc::new(Cell::new(Some(START)));
    let shown = Rc::clone(&value);
    let mut fixture = Fixture::new(move |ui| {
        let mut number = shown.get();
        Field::new("Opacity").side(SideLabel::new().scrub(true)).show(ui, |ui, _| {
            NumberField::new("Opacity", Bind::bound(&mut number)).scrub_by_label().show(ui);
        });
        shown.set(number);
    });
    if let Some(drags) = drags {
        fixture.root_value(drags);
    }
    fixture.settle();
    let label = fixture.by_role(Role::LABEL).first().map(|target| target.center()).unwrap_or_else(|| panic!("the side label is shown"));
    let body = fixture.by_label("Opacity").map(|target| target.center()).unwrap_or_else(|| panic!("the number field is shown"));
    let by_label = dragged(&mut fixture, &value, label);
    let by_body = dragged(&mut fixture, &value, body);
    (by_label, by_body)
}

#[trial]
fn scrub_drags_label_only_leaves_the_body_to_text_and_off_scrubs_nothing() {
    assert_eq!(scrubs(None), (true, true), "by default both the label and the body scrub");
    assert_eq!(scrubs(Some(ScrubDrags::LabelOnly)), (true, false), "label only: the label scrubs, the body's drag is the text's");
    assert_eq!(scrubs(Some(ScrubDrags::Off)), (false, false), "off: neither scrubs");
}
