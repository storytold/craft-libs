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
