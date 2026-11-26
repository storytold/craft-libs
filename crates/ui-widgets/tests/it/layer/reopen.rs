//! A popover closed from inside its body, by Escape or by a click outside opens again on the next click on its
//! opener, also while its close fade still runs.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _};
use craft_ui::popover::Popover;
use craft_ui::testing::contract::clicked;
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

const OPENER: &str = "Brush";
const ITEM: &str = "Reset";

/// A popover the button `OPENER` toggles; a click on its item `ITEM` closes it from inside.
fn rig() -> (Fixture, Rc<Cell<bool>>) {
    let open = Rc::new(Cell::new(false));
    let shown = open.clone();
    let mut fixture = Fixture::new(move |ui| {
        let opener = Button::new(OPENER).show(ui);
        let mut is_open = shown.get();
        if clicked(&opener) {
            is_open = !is_open;
        }
        Popover::new("Brush options", Bind::bound(&mut is_open)).opened_by(&opener).show(ui, |ui, cx| {
            if clicked(&Button::new(ITEM).show(ui)) {
                cx.request_close();
            }
        });
        shown.set(is_open);
    });
    fixture.settle();
    (fixture, open)
}

/// A click on the opener, then the frames the focus takes to move into the popover.
fn open(fixture: &mut Fixture) {
    let opener = fixture.by_label(OPENER).expect("the opener");
    fixture.click(&opener);
    fixture.idle(3);
}

/// Opens the popover, closes it with `close`, and clicks the opener again at once, while the close fade runs.
fn reopened_after(close: impl FnOnce(&mut Fixture)) -> (Fixture, Rc<Cell<bool>>) {
    let (mut fixture, open_flag) = rig();
    open(&mut fixture);
    assert!(fixture.by_label(ITEM).is_some(), "the popover opened");
    close(&mut fixture);
    assert!(!open_flag.get(), "the close cleared the binding");
    assert!(fixture.by_label(ITEM).is_none(), "the popover closed");
    open(&mut fixture);
    (fixture, open_flag)
}

fn assert_open_again(fixture: &Fixture, open: &Cell<bool>) {
    assert!(open.get(), "the opener's click opened the popover again");
    assert!(fixture.by_label(ITEM).is_some(), "the popover shows its body again");
}

/// An item click that closes the popover from inside leaves it free to open again.
#[trial]
fn a_popover_closed_from_its_body_opens_again() {
    let (fixture, open) = reopened_after(|fixture| {
        let item = fixture.by_label(ITEM).expect("the popover's item");
        fixture.click(&item);
    });
    assert_open_again(&fixture, &open);
}
