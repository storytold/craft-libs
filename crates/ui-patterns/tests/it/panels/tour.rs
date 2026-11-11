//! A tour blocks nothing: the window under its card stays usable.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Show as _, clicked};
use craft_ui::patterns::panels::{Tour, TourStep};
use craft_ui::testing::{Fixture, trial};
use std::cell::Cell;
use std::rc::Rc;

#[trial]
fn a_press_beside_the_tour_card_reaches_the_window_and_the_tour_stays() {
    let presses = Rc::new(Cell::new(0));
    let seen = presses.clone();
    let mut fixture = Fixture::new(move |ui| {
        let behind = Button::new("Behind").show(ui);
        if clicked(&behind) {
            seen.set(seen.get() + 1);
        }
        let steps = [TourStep::new("Layers", "Every object is a layer.").target(behind.rect), TourStep::new("Export", "Export it.")];
        let mut index = 0_usize;
        Tour::new("Welcome tour", &steps, Bind::bound(&mut index)).show(ui);
    });
    fixture.idle(30);
    let behind = fixture.by_label("Behind").expect("the window's button is shown");
    fixture.click(&behind).idle(4);
    assert_eq!(presses.get(), 1, "the press went to the window under the tour");
    assert!(fixture.by_label("Next").is_some(), "the tour's card stays");
}
