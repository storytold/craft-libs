//! The table size picker's keys.

use craft_pages::ui::table::TableSizePicker;
use craft_pages::ui::table::size_picker::COLUMNS;
use craft_ui::contract::Show as _;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers};
use std::cell::Cell;
use std::rc::Rc;

/// Right on the last square of a row moves on to the first square of the next row.
#[trial]
fn right_at_a_row_end_goes_on_to_the_next_row() {
    let hovered = Rc::new(Cell::new(None));
    let seen = hovered.clone();
    let mut fixture = Fixture::new(move |ui| seen.set(TableSizePicker::new("Insert Table").show(ui).hovered));
    fixture.settle();
    fixture.key(Key::Tab, Modifiers::NONE).idle(2);
    assert_eq!(hovered.get(), Some((1, 1)), "focus lands on the first square");
    for _ in 0..COLUMNS - 1 {
        fixture.key(Key::ArrowRight, Modifiers::NONE);
    }
    fixture.idle(2);
    assert_eq!(hovered.get(), Some((1, COLUMNS)), "the last square of the first row");
    fixture.key(Key::ArrowRight, Modifiers::NONE).idle(2);
    assert_eq!(hovered.get(), Some((2, 1)), "Right wraps to the next row");
}

/// A picker asked for sample size draws a larger square than the default thumbnail.
#[trial]
fn edge_picker_sample_size_is_larger_than_the_thumbnail() {
    use craft_pages::ui::table::{BorderEdges, EdgePicker, EdgeSize};
    use craft_ui::contract::Bind;
    let sides = Rc::new(Cell::new((0.0, 0.0)));
    let seen = sides.clone();
    let mut edges = BorderEdges::default();
    let mut fixture = Fixture::new(move |ui| {
        let thumb = EdgePicker::new("Thumb", Bind::bound(&mut edges)).show(ui).response.rect.width();
        let mut other = BorderEdges::default();
        let sample = EdgePicker::new("Sample", Bind::bound(&mut other)).size(EdgeSize::Sample).show(ui).response.rect.width();
        seen.set((thumb, sample));
    });
    fixture.settle();
    let (thumb, sample) = sides.get();
    assert!(thumb > 0.0 && sample > thumb, "thumb {thumb}, sample {sample}");
}
