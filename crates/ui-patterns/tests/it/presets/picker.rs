//! `PresetPicker`: a click on a preset in the popover picks it and closes; Escape closes and keeps the value; inside a
//! popover, the picker's own popover nests in the parent, which stays open through the pick.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::patterns::presets::{PresetPicker, PresetRow, PresetState};
use craft_ui::popover::Popover;
use craft_ui::testing::{Fixture, trial};
use egui::{Key, Modifiers, Vec2};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

const CORAL: u32 = 12;
const TEAL: u32 = 21;

struct Host {
    value: Option<u32>,
    state: PresetState<u32>,
    picked: Vec<u32>,
    open: bool,
}

#[trial]
fn a_click_picks_and_closes_and_escape_closes_keeping_the_value() {
    let host = Rc::new(RefCell::new(Host { value: Some(CORAL), state: PresetState::new(), picked: Vec::new(), open: false }));
    let shown = Rc::clone(&host);
    let rows = vec![PresetRow::item(CORAL, "Coral", 0), PresetRow::item(TEAL, "Teal", 0)];
    let mut fixture = Fixture::new(move |ui| {
        let mut host = shown.borrow_mut();
        let Host { value, state, picked, open } = &mut *host;
        let out = PresetPicker::new("Swatch", Bind::bound(value), &rows, Bind::bound(state)).show(ui);
        picked.extend(out.picked);
        *open = out.open;
    });
    fixture.room(Vec2::new(480.0, 480.0));
    fixture.settle();
    let field = fixture.by_role(Role::COMBO_BOX).first().copied().expect("the field shows");

    fixture.click(&field);
    fixture.idle(3);
    assert!(fixture.by_label("Teal").is_some(), "the field opens the popover with the presets");
    fixture.key(Key::Escape, Modifiers::NONE);
    fixture.idle(3);
    assert_eq!(host.borrow().value, Some(CORAL), "Escape keeps the value");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "and closes the popover");
    assert!(!host.borrow().open, "the picker reports it closed");

    fixture.click(&field);
    fixture.idle(3);
    let teal = fixture.by_label("Teal").expect("the popover lists the presets");
    fixture.click(&teal);
    fixture.idle(3);
    let host = host.borrow();
    assert_eq!((host.value, host.picked.as_slice()), (Some(TEAL), &[TEAL][..]), "the click picks");
    assert!(fixture.by_role(Role::DIALOG).is_empty(), "and closes the popover");
}

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
