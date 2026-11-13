//! `PresetPicker` inside a popover: the picker's own popover nests in the parent, which stays open through the pick.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::patterns::presets::{PresetPicker, PresetRow, PresetState};
use craft_ui::popover::Popover;
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn a_picker_in_a_popover_opens_and_picks_with_the_popover_kept_open() {
    let (parent, value) = (Rc::new(Cell::new(true)), Rc::new(Cell::new(Some(1u32))));
    let (parent_seen, value_seen) = (parent.clone(), value.clone());
    let rows = vec![PresetRow::item(1u32, "Alpha", 0), PresetRow::item(2, "Beta", 0)];
    let mut state = PresetState::<u32>::new();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new("Fill").show(ui);
        let mut open = parent_seen.get();
        Popover::new("Paint", Bind::bound(&mut open)).opened_by(&opener).show(ui, |ui, _| {
            let mut picked = value_seen.get();
            let _ = PresetPicker::new("Pattern", Bind::bound(&mut picked), &rows, Bind::bound(&mut state)).show(ui);
            value_seen.set(picked);
        });
        parent_seen.set(open);
    });
    fixture.settle();
    let field = fixture.by_label("Pattern").expect("the picker's field shows in the popover");
    fixture.click(&field).settle();
    assert!(parent.get(), "opening the picker keeps the popover open");
    let beta = fixture.by_label("Beta").expect("the picker's presets show");
    fixture.click(&beta).settle();
    assert_eq!(value.get(), Some(2), "the click picks Beta");
    assert!(fixture.by_label("Alpha").is_none(), "the pick closes the picker: only the field shows, with Beta");
    assert!(parent.get(), "the pick keeps the popover open");
}
