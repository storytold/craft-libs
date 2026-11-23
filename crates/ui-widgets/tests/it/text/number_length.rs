//! A length field keeps the document's unit and shows the user's.

use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::runtime::env::{LengthUnits, UnitKind};
use craft_ui::testing::input::key_press;
use craft_ui::testing::pointer::click;
use craft_ui::testing::{Frames, trial};
use craft_ui::text::NumberField;
use egui::{Event, Key, Modifiers, Pos2, Ui};
use std::cell::{Cell, RefCell};

/// A length the field starts from, in the document's points.
const START: f64 = 72.0;
/// How far two lengths may differ and still count as the same.
const SAME: f64 = 1e-6;

/// A field over points that shows inches, driven by `drive` after it holds focus.
fn in_inches(start: f64, drive: impl FnOnce(&mut Frames, &mut dyn FnMut(&mut Ui))) -> f64 {
    let mut frames = Frames::new();
    let value = RefCell::new(Some(start));
    let at = Cell::new(Pos2::ZERO);
    let mut show = |ui: &mut Ui| {
        let mut guard = value.borrow_mut();
        let lengths = LengthUnits::new(UnitKind::Points, UnitKind::Inches, START);
        at.set(NumberField::length("Width", Bind::bound(&mut *guard), lengths).id_salt("width").show(ui).response.rect.center());
    };
    frames.idle(3, &mut show);
    click(&mut frames, at.get(), &mut show);
    drive(&mut frames, &mut show);
    frames.idle(3, &mut show);
    value.into_inner().unwrap_or_default()
}

fn type_and_commit(text: &'static str) -> impl FnOnce(&mut Frames, &mut dyn FnMut(&mut Ui)) {
    move |frames, show| {
        frames.frame(vec![key_press(Key::A, Modifiers::COMMAND)], &mut *show);
        frames.frame(vec![Event::Text(text.into())], &mut *show);
        frames.idle(1, &mut *show);
        frames.frame(vec![key_press(Key::Enter, Modifiers::NONE)], &mut *show);
    }
}

/// Typed units convert to the document's unit, a bare number is in the shown unit, and an arrow steps by the shown
/// unit's step.
#[trial]
fn a_length_field_types_and_steps_in_the_shown_unit() {
    let one = in_inches(START, type_and_commit("1 in"));
    let metric = in_inches(START, type_and_commit("25.4 mm"));
    assert!((metric - one).abs() < SAME, "25.4 mm is one inch: {metric} against {one}");

    let bare = in_inches(START, type_and_commit("2"));
    let spelled = in_inches(START, type_and_commit("2 in"));
    assert!((bare - spelled).abs() < SAME, "a bare number is in the shown unit: {bare} against {spelled}");

    let stepped = in_inches(one, |frames, show| {
        frames.frame(vec![key_press(Key::ArrowUp, Modifiers::NONE)], show);
    });
    let tenth_up = in_inches(START, type_and_commit("1.1"));
    assert!((stepped - tenth_up).abs() < SAME, "an arrow moves a tenth of an inch: {stepped} against {tenth_up}");
}
