//! One popover at a time: opening a second closes the first.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, ShowContainer as _, clicked};
use craft_ui::layout_components::Row;
use craft_ui::popover::Popover;
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn opening_a_second_popover_closes_the_first() {
    let open = Rc::new([Cell::new(false), Cell::new(false)]);
    let shown = open.clone();
    let mut fixture = Fixture::new(move |ui| {
        Row::new().show(ui, |ui, _| {
            for (at, name) in ["Grid", "Guides"].into_iter().enumerate() {
                let opener = Button::new(name).show(ui);
                let mut now = shown[at].get() || clicked(&opener);
                Popover::new(format!("{name} options"), Bind::bound(&mut now)).opened_by(&opener).show(ui, |ui, _| {
                    Button::new(format!("{name} snap")).show(ui);
                });
                shown[at].set(now);
            }
        });
    });
    fixture.idle(30);
    let grid = fixture.by_label("Grid").expect("the first opener is shown");
    fixture.click(&grid).idle(6);
    assert!(open[0].get(), "the first popover opened");
    let guides = fixture.by_label("Guides").expect("the second opener is shown");
    fixture.click(&guides).idle(6);
    assert!(open[1].get() && !open[0].get(), "the second popover opened and the first closed");
}
