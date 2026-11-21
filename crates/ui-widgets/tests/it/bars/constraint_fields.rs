//! `ConstraintFields`: the style pick, the typed sides, Tab between them, the swap, the greyed state and the Fixed Size units.

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

/// Tab goes from Width straight to Height, so both sides are typed in one pass; the swap button between them is the
/// Tab stop after Height, and Space there swaps. Shift+Tab goes back to Width.
#[trial]
fn tab_moves_from_width_straight_to_height() {
    for start in [Constraint::Ratio(4.0, 3.0), Constraint::Size(640.0, 480.0)] {
        let (mut fixture, state) = rig(start, None, true);
        let (w, h) = fields(&fixture);
        fixture.click(&w);
        fixture.key(Key::A, Modifiers::COMMAND);
        fixture.type_text("16");
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.settle();
        assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), h.node, "{start:?}: Tab from Width lands on Height");
        fixture.key(Key::A, Modifiers::COMMAND);
        fixture.type_text("9");
        fixture.key(Key::Enter, Modifiers::NONE);
        fixture.settle();
        let typed = match start {
            Constraint::Ratio(..) => Constraint::Ratio(16.0, 9.0),
            _ => Constraint::Size(16.0, 9.0),
        };
        assert_eq!(state.get(), typed, "{start:?}: both sides typed with one Tab between them");
        let swap = fixture.by_label("Swaps height and width").expect("the swap button");
        assert!(w.rect.max.x <= swap.rect.min.x && swap.rect.max.x <= h.rect.min.x, "{start:?}: the swap sits between the fields");
        fixture.click(&h);
        fixture.key(Key::Tab, Modifiers::NONE);
        fixture.settle();
        assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), swap.node, "{start:?}: Tab from Height lands on the swap");
        fixture.key(Key::Space, Modifiers::NONE);
        fixture.settle();
        let swapped = match start {
            Constraint::Ratio(..) => Constraint::Ratio(9.0, 16.0),
            _ => Constraint::Size(9.0, 16.0),
        };
        assert_eq!(state.get(), swapped, "{start:?}: Space on the swap exchanges the sides");
        fixture.click(&h);
        fixture.key(Key::Tab, Modifiers::SHIFT);
        fixture.settle();
        assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), w.node, "{start:?}: Shift+Tab from Height lands on Width");
    }
}
