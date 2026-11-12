//! A field takes the width of its control size: a small field, as an options bar shows it, is narrower than the same
//! field at the standard size, as a dialog shows it.

use craft_theme::ControlSize;
use craft_ui::contract::{Bind, Entry, Props as _, Show as _};
use craft_ui::pickers::Select;
use craft_ui::testing::{Frames, trial};
use craft_ui::text::{NumberField, TextField};
use std::cell::Cell;

/// The widths of a text field, a select and a number field shown at `size` in a wide row.
fn widths(size: ControlSize) -> [f32; 3] {
    let mut frames = Frames::new();
    let shown = Cell::new([0.0; 3]);
    let (mut text, mut number, mut picked) = (String::new(), Some(1.0), 1u8);
    let entries = [Entry::new(1u8, "One"), Entry::new(2u8, "Two")];
    frames.idle(3, |ui| {
        let text_width = TextField::new("Name", Bind::bound(&mut text)).size(size).id_salt("text").show(ui).response.rect.width();
        let select_width = Select::new("Pick", Bind::bound(&mut picked), &entries).size(size).id_salt("select").show(ui).response.rect.width();
        let number_width = NumberField::new("Size", Bind::bound(&mut number)).size(size).id_salt("number").show(ui).response.rect.width();
        shown.set([text_width, select_width, number_width]);
    });
    shown.get()
}

#[trial]
fn a_small_field_is_narrower_than_a_standard_one() {
    let (small, standard) = (widths(ControlSize::Sm), widths(ControlSize::Md));
    for (name, (small, standard)) in ["text field", "select", "number field"].into_iter().zip(small.into_iter().zip(standard)) {
        assert!(small > 0.0 && small < standard, "a small {name} is narrower than a standard one: {small} against {standard}");
    }
}

/// A small number field with a pop-up slider shows "100%" whole before its chevron, and is no wider than its content
/// needs: under a number cell, the chevron and a quarter cell. A field over a long range keeps one cell.
#[trial]
fn a_small_percent_field_shows_its_value_whole_beside_the_chevron() {
    use craft_ui::contract::Role;
    use craft_ui::runtime::env::UnitKind;
    use std::rc::Rc;
    let widths = Rc::new(Cell::new([0.0; 3]));
    let seen = widths.clone();
    let mut fixture = craft_ui::testing::Fixture::new(move |ui| {
        let percent = NumberField::new("Opacity", Bind::given(Some(100.0)))
            .range(0.0..=100.0)
            .unit_kind(UnitKind::Percent)
            .popup_slider(0.0..=100.0)
            .size(ControlSize::Sm)
            .id_salt("percent")
            .show(ui);
        // A field whose value is narrower than a cell is one cell wide.
        let cell = NumberField::new("Count", Bind::given(Some(1.0))).size(ControlSize::Sm).id_salt("cell").show(ui);
        // A range whose widest value is wider than a cell keeps the cell; long values scroll inside it.
        let long = NumberField::new("Side", Bind::given(Some(240.0)))
            .range(1.0..=300_000.0)
            .unit_kind(UnitKind::Pixels)
            .size(ControlSize::Sm)
            .id_salt("long")
            .show(ui);
        seen.set([percent.response.rect.width(), cell.response.rect.width(), long.response.rect.width()]);
    });
    fixture.settle();
    let chevron = fixture.by_role(Role::BUTTON).first().unwrap_or_else(|| panic!("the field shows its chevron")).rect;
    let painted = |text: &str| fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect);
    let value = painted("100").unwrap_or_else(|| panic!("the field shows its value"));
    let unit = painted("%").unwrap_or_else(|| panic!("the field shows its unit"));
    assert!(value.max.x <= unit.min.x, "the value ends before its unit: {value:?} {unit:?}");
    assert!(unit.max.x <= chevron.min.x, "the unit ends before the chevron: {unit:?} {chevron:?}");
    let [percent, cell, long] = widths.get();
    assert!(
        percent < cell + chevron.width() + cell / 4.0,
        "the field is as wide as its content: {percent} against a {cell} cell and a {} chevron",
        chevron.width()
    );
    assert!((long - cell).abs() < 0.01, "a field over a long range is one cell wide: {long} against {cell}");
}
