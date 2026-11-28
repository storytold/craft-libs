//! The type bar's size field (PhotoCraft #12053): the bar's range is 0.01 to 1296 pt, and a value typed outside it
//! goes to the nearest limit and comes back as `TypeBarOutput::size_out_of_range` in the pass it committed, for the
//! host to word as an alert. The field writes the fractions its range carries, so what it paints stays inside the
//! range, and the report names its ends at the same decimals in the unit the field shows. In-range sizes set with no
//! report, and the Character panel's size field keeps its own 0.1 pt minimum and its own display untouched.
//!
//! Typed values stay integers: the field's number reader follows the env's regional separators, and this file does
//! not pin that behaviour (it belongs to the number field's own tests).

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use craft_behavior::EditEvent;
use craft_text::ui::DocUnit;
use craft_text::ui::character::{CharacterChange, CharacterControls, CharacterValues, ParagraphValues};
use craft_text::ui::type_bar::{TypeBar, TypeBarChange};
use craft_ui::runtime::env::{Env, UnitKind};
use craft_ui::testing::{Fixture, trial};
use craft_ui::text::RangeAlert;
use egui::{Key, Modifiers, vec2};

fn families() -> Vec<String> {
    vec!["Inter".into()]
}

/// Show the type bar in `unit` over `size`, collecting its edits and its size range reports. The bar is a wide strip,
/// so the room keeps it out of the overflow menu.
fn bar(size: f32, unit: DocUnit, edits: &Rc<RefCell<Vec<EditEvent<TypeBarChange>>>>, reports: &Rc<RefCell<Vec<RangeAlert>>>) -> Fixture {
    let character = CharacterValues { size: Some(size), ..Default::default() };
    let paragraph = ParagraphValues::default();
    let (families, styles) = (families(), families());
    let (edits, reports) = (edits.clone(), reports.clone());
    let mut fixture = Fixture::new(move |ui| {
        let out = TypeBar::new(&character, &paragraph, &families, &styles).unit(unit).show(ui);
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
    let mut fixture = bar(20.0, DocUnit::points(), &edits, &reports);
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
        let mut fixture = bar(20.0, DocUnit::points(), &edits, &reports);
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
        let mut fixture = bar(20.0, DocUnit::points(), &edits, &reports);
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

/// The bar's size field after a size below its minimum was typed and clamped, over a host that takes the bar's
/// edits as they come: the field paints the 0.01 it kept, written as the env's own formatter writes it at two
/// decimals, and not the 0 the unit's own one decimal rounds it to. The expected text comes from that formatter,
/// so the trial pins the digits and their count, not this machine's decimal mark.
#[trial]
fn the_size_field_paints_the_kept_minimum_and_not_zero() {
    let (reports, expected, size) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(String::new())), Rc::new(Cell::new(20.0f32)));
    let (families, styles) = (families(), families());
    let (shown_reports, shown_expected, shown_size) = (reports.clone(), expected.clone(), size.clone());
    let mut fixture = Fixture::new(move |ui| {
        let character = CharacterValues { size: Some(shown_size.get()), ..Default::default() };
        let paragraph = ParagraphValues::default();
        let out = TypeBar::new(&character, &paragraph, &families, &styles).show(ui);
        shown_reports.borrow_mut().extend(out.size_out_of_range);
        for edit in out.edits {
            if let Some(TypeBarChange::Character(CharacterChange::Size(size))) = edit.value() {
                shown_size.set(*size);
            }
        }
        *shown_expected.borrow_mut() = Env::of(ui).units().number_ungrouped(0.01, 2..=2);
    });
    fixture.room(vec2(1200.0, 200.0));
    fixture.idle(10);
    type_into(&mut fixture, "Set font size", "0");
    assert_eq!(reports.borrow().len(), 1, "the clamp was reported, so the field holds the kept value");
    assert_eq!(size.get(), 0.01, "the host took the kept value");
    let expected = expected.borrow().clone();
    let painted: Vec<String> = fixture.texts().iter().map(|text| text.text.clone()).collect();
    assert!(
        painted.iter().any(|text| text.contains(&*expected)),
        "the field paints {expected}, the two-decimal writing of the 0.01 pt it kept: {painted:?}"
    );
}

/// The same clamp in another unit: the report carries the bar's range in that unit and names its ends at the
/// decimals that unit writes them at, and says they are not whole. The trial pins that data, which is what the
/// host's message is built from; it does not build the sentence here.
#[trial]
fn a_clamped_size_is_reported_in_the_unit_the_field_shows() {
    let (edits, reports) = (Rc::new(RefCell::new(vec![])), Rc::new(RefCell::new(vec![])));
    let mut fixture = bar(20.0, DocUnit::new(UnitKind::Inches, 72.0), &edits, &reports);
    fixture.idle(10);
    type_into(&mut fixture, "Set font size", "0");
    let sizes = bar_sizes(&edits);
    assert!(
        !sizes.is_empty() && sizes.iter().all(|s| (f64::from(*s) - 0.01).abs() < 1e-6),
        "the bar keeps its 0.01 pt minimum in inches too: {sizes:?}"
    );
    let reports = reports.borrow();
    assert_eq!(reports.len(), 1, "one report");
    let report = &reports[0];
    assert_eq!(report.unit_kind, Some(UnitKind::Inches), "the report names the unit the field shows");
    let (start, end, kept) = (*report.range.start(), *report.range.end(), report.kept);
    assert!((start - 0.01 / 72.0).abs() < 1e-9 && (end - 18.0).abs() < 1e-9, "the ends are the bar's range, in inches: {start} to {end}");
    assert!((kept - start).abs() < 1e-12, "the kept value is that minimum: {kept}");
    assert_eq!(report.decimals, 3..=3, "the unit's own decimals, which are more than the two the points range needs");
    assert!(!report.whole, "the ends are fractions, so a message built on this data does not say whole numbers");
}
