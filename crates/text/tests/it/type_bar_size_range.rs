//! The type bar's size field (PhotoCraft #12053): the bar's range is 0.01 to 1296 pt, and a value typed outside it
//! goes to the nearest limit and comes back as `TypeBarOutput::size_out_of_range` in the pass it committed, for the
//! host to word as an alert. In-range sizes set with no report, and the Character panel's size field keeps its own
//! 0.1 pt minimum untouched.
//!
//! Typed values stay integers: the field's number reader follows the env's regional separators, and this file does
//! not pin that behaviour (it belongs to the number field's own tests).

use std::cell::RefCell;
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_text::ui::character::{CharacterChange, CharacterControls, CharacterValues, ParagraphValues};
use craft_text::ui::type_bar::{TypeBar, TypeBarChange};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::RangeAlert;
use egui::{Key, Modifiers, vec2};

fn families() -> Vec<String> {
    vec!["Inter".into()]
}

/// Show the type bar over `size`, collecting its edits and its size range reports. The bar is a wide strip,
/// so the room keeps it out of the overflow menu.
fn bar(size: f32, edits: &Rc<RefCell<Vec<EditEvent<TypeBarChange>>>>, reports: &Rc<RefCell<Vec<RangeAlert>>>) -> Fixture {
    let character = CharacterValues { size: Some(size), ..Default::default() };
    let paragraph = ParagraphValues::default();
    let (families, styles) = (families(), families());
    let (edits, reports) = (edits.clone(), reports.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = TypeBar::new(&character, &paragraph, &families, &styles).show(ui);
        reports.borrow_mut().extend(out.size_out_of_range);
        edits.borrow_mut().extend(out.edits);
    });
    fixture.room(vec2(1200.0, 200.0));
    fixture
}

/// Show the Character panel over `size`, collecting its edits.
fn panel(size: f32, edits: &Rc<RefCell<Vec<EditEvent<CharacterChange>>>>) -> Fixture {
    let character = CharacterValues { size: Some(size), ..Default::default() };
    let (families, styles) = (families(), families());
    let edits = edits.clone();
    Fixture::new(move |ui| {
        let out = CharacterControls::new("Character", &character, &families, &styles).show(ui);
        edits.borrow_mut().extend(out.edits);
    })
}

/// The sizes the bar's edits carry, in order.
fn bar_sizes(edits: &RefCell<Vec<EditEvent<TypeBarChange>>>) -> Vec<f32> {
    edits
        .borrow()
        .iter()
        .filter_map(|edit| match edit.value() {
            Some(TypeBarChange::Character(CharacterChange::Size(size))) => Some(*size),
            _ => None,
        })
        .collect()
}

/// The sizes the panel's edits carry, in order.
fn panel_sizes(edits: &RefCell<Vec<EditEvent<CharacterChange>>>) -> Vec<f32> {
    edits
        .borrow()
        .iter()
        .filter_map(|edit| match edit.value() {
            Some(CharacterChange::Size(size)) => Some(*size),
            _ => None,
        })
        .collect()
}

/// Select all in the field and type `text`, committing with Enter.
fn type_into(fixture: &mut Fixture, field: &str, text: &str) {
    let field = fixture.by_label(field).unwrap_or_else(|| panic!("the {field} field is shown"));
    fixture.click(&field).idle(2);
    fixture.key(Key::A, Modifiers::COMMAND);
    fixture.type_text(text);
    fixture.key(Key::Enter, Modifiers::NONE);
    fixture.idle(4);
}

#[trial]
fn a_size_typed_over_the_bar_max_becomes_it_and_is_reported() {
    let (edits, reports) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(vec![])));
    let mut fixture = bar(20.0, &edits, &reports);
    fixture.idle(10);
    type_into(&mut fixture, "Set font size", "2000");
    let sizes = bar_sizes(&edits);
    assert!(!sizes.is_empty() && sizes.iter().all(|s| *s == 1296.0), "the nearest limit is kept: {sizes:?}");
    let reports = reports.borrow();
    assert_eq!(reports.len(), 1, "one report, in the pass it committed");
    assert_eq!((reports[0].typed, reports[0].kept), (2000.0, 1296.0), "the report carries the typed and the kept value");
    assert_eq!(*reports[0].range.start(), 0.01, "the bar's minimum");
    assert_eq!(*reports[0].range.end(), 1296.0, "the bar's maximum");
    assert_eq!(reports[0].decimals, 2..=2, "both ends are named at two decimals, as the measured message writes them");
}

#[trial]
fn a_size_typed_under_the_bar_min_becomes_001_pt_and_is_reported() {
    for typed in ["0", "-5"] {
        let (edits, reports) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(vec![])));
        let mut fixture = bar(20.0, &edits, &reports);
        fixture.idle(10);
        type_into(&mut fixture, "Set font size", typed);
        let sizes = bar_sizes(&edits);
        assert!(!sizes.is_empty() && sizes.iter().all(|s| *s == 0.01), "{typed}: the bar's 0.01 pt minimum is kept: {sizes:?}");
        let reports = reports.borrow();
        assert_eq!(reports.len(), 1, "{typed}: one report");
        assert_eq!(reports[0].kept, 0.01, "{typed}: the report names the kept value");
    }
}

#[trial]
fn an_in_range_size_sets_without_a_report() {
    for typed in ["64", "1296"] {
        let (edits, reports) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(vec![])));
        let mut fixture = bar(20.0, &edits, &reports);
        fixture.idle(10);
        type_into(&mut fixture, "Set font size", typed);
        let sizes = bar_sizes(&edits);
        let kept: f32 = typed.parse().unwrap();
        assert!(!sizes.is_empty() && sizes.iter().all(|s| *s == kept), "{typed}: set as typed: {sizes:?}");
        assert!(reports.borrow().is_empty(), "{typed}: no report, no alert");
    }
}

#[trial]
fn the_character_panel_size_field_keeps_its_own_minimum() {
    let edits = Rc::new(RefCell::new(vec![]));
    let mut fixture = panel(14.0, &edits);
    fixture.idle(10);
    type_into(&mut fixture, "Font size", "0");
    let sizes = panel_sizes(&edits);
    assert!(!sizes.is_empty() && sizes.iter().all(|s| *s == 0.1), "the panel keeps its own 0.1 pt minimum, clamped as it is today: {sizes:?}");
}
