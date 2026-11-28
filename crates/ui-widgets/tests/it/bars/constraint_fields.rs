//! `ConstraintFields`: the style pick, the typed sides, the swap, the greyed state and the Fixed Size units.

use craft_behavior::PixelSize;
use craft_behavior::geometry::Constraint;
use craft_ui::bars::{Bar, BarKind, ConstraintFields};
use craft_ui::contract::{Bind, Entry, Role, ShowContainer as _};
use craft_ui::runtime::env::UnitKind;
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::Cell;
use std::rc::Rc;

const ROOM: Vec2 = Vec2::new(900.0, 80.0);
const DOCUMENT: PixelSize = PixelSize::new(1000, 500);

fn rig(start: Constraint, units: Option<UnitKind>, enabled: bool) -> (Fixture, Rc<Cell<Constraint>>) {
    let state = Rc::new(Cell::new(start));
    let seen = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let list = [Entry::new(UnitKind::Pixels, "Pixels"), Entry::new(UnitKind::Inches, "Inches"), Entry::new(UnitKind::Centimetres, "Centimetres")];
        let mut value = seen.get();
        Bar::new("Options", BarKind::Options).show(ui, |ui, _| {
            let fields = ConstraintFields::new(Bind::bound(&mut value)).enabled(enabled);
            let fields = if let Some(unit) = units { fields.size_units(unit, &list, 100.0, DOCUMENT) } else { fields };
            fields.show(ui);
        });
        seen.set(value);
    });
    fixture.room(ROOM).settle();
    (fixture, state)
}

/// The Width field, then the Height field.
fn fields(fixture: &Fixture) -> (Target, Target) {
    let mut spins = fixture.by_role(Role::SPIN_BUTTON);
    assert_eq!(spins.len(), 2, "Width and Height are number fields");
    spins.sort_by(|a, b| a.rect.min.x.total_cmp(&b.rect.min.x));
    let height = spins.pop().unwrap();
    (spins.pop().unwrap(), height)
}

fn type_into(fixture: &mut Fixture, field: &Target, text: &str) {
    fixture.click(field);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.settle();
}

/// Typed text carries the unit it is meant in: the document's resolution (100 pixels per inch) and extent (1000 by 500
/// pixels) convert it whatever unit the field shows.
#[trial]
fn typed_lengths_convert_through_the_document_in_every_unit() {
    let typed = |unit: UnitKind, text: &str, width: bool| {
        let (mut fixture, state) = rig(Constraint::Size(640.0, 480.0), Some(unit), true);
        let (w, h) = fields(&fixture);
        type_into(&mut fixture, if width { &w } else { &h }, text);
        state.get()
    };
    assert_eq!(typed(UnitKind::Pixels, "2in", true), Constraint::Size(200.0, 480.0), "inches in a Pixels field");
    assert_eq!(typed(UnitKind::Inches, "250px", true), Constraint::Size(250.0, 480.0), "pixels in an Inches field");
    assert_eq!(typed(UnitKind::Inches, "50%", true), Constraint::Size(500.0, 480.0), "percent of the width");
    assert_eq!(typed(UnitKind::Pixels, "50%", false), Constraint::Size(640.0, 250.0), "percent of the height");
}
