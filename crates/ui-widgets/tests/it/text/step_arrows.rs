//! The step arrows of a number field: one press steps once, a held press repeats by the pass time, and a field that
//! takes no input takes no press.

use craft_behavior::HoldRepeat;
use craft_ui::contract::{Bind, Props as _, Show as _};
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui::text::{NumberField, StepArrows};
use egui::Pos2;
use std::cell::Cell;
use std::rc::Rc;

/// The number the field starts at; the default step is 1, so the value counts the steps.
const START: f64 = 50.0;

/// The field's label.
const LABEL: &str = "Size";

/// A field with leading step arrows, `enabled` and `read_only` as given, and the number it reports.
fn field(enabled: bool, read_only: bool) -> (Fixture, Rc<Cell<f64>>) {
    let value = Rc::new(Cell::new(START));
    let shown = value.clone();
    let mut fixture = Fixture::hosted(move |ui| {
        let out =
            NumberField::new(LABEL, Bind::given(Some(shown.get()))).step_arrows(StepArrows::Leading).enabled(enabled).read_only(read_only).show(ui);
        if let Some(Some(next)) = out.edit.events.landed().copied() {
            shown.set(next);
        }
        Some(out.edit.into())
    });
    fixture.probe_purity();
    fixture.settle();
    (fixture, value)
}

/// The middle of the up half.
fn up(fixture: &Fixture) -> Pos2 {
    let root = fixture.root_id().expect("the field is shown");
    fixture.part_rect(root, NumberField::STEP_UP).expect("the field declares its up half").center()
}

/// Presses the up half and leaves the button down.
fn press_up(fixture: &mut Fixture) {
    let at = up(fixture);
    fixture.frame(vec![pointer::moved(at)]);
    fixture.frame(vec![pointer::press(at)]);
}

/// A press on the up half steps once; held past the delay it steps again, then once per interval of the pass time
/// that went by; after the release it steps no more. A disabled and a read-only field take no press. The fixture also
/// reruns passes inside a frame, which must change nothing.
#[trial]
fn a_held_step_arrow_repeats_and_a_disabled_one_takes_no_press() {
    let (mut fixture, value) = field(true, false);
    press_up(&mut fixture);
    assert_eq!(value.get(), START + 1.0, "the press steps once");

    fixture.advance(HoldRepeat::DELAY);
    fixture.idle(1);
    assert_eq!(value.get(), START + 2.0, "held past the delay it steps once more");

    fixture.advance(3.0 * HoldRepeat::INTERVAL);
    fixture.idle(1);
    assert_eq!(value.get(), START + 5.0, "then once per interval that passed");

    fixture.release_at(up(&fixture));
    let after_release = value.get();
    fixture.advance(HoldRepeat::DELAY + 3.0 * HoldRepeat::INTERVAL);
    fixture.idle(2);
    assert_eq!(value.get(), after_release, "a released arrow steps no more");
    assert!(fixture.impure().is_empty(), "a rerun pass changed something: {:?}", fixture.impure());

    for (enabled, read_only) in [(false, false), (true, true)] {
        let (mut fixture, value) = field(enabled, read_only);
        press_up(&mut fixture);
        fixture.advance(HoldRepeat::DELAY);
        fixture.idle(2);
        assert_eq!(value.get(), START, "enabled: {enabled}, read-only: {read_only}: the arrow takes no press");
    }
}
