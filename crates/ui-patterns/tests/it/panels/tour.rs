//! A tour blocks nothing: the window under its card and its mark stays usable. Its card steps and leaves.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _, clicked};
use craft_ui::display::TargetMark;
use craft_ui::patterns::panels::{Tour, TourEvent, TourStep};
use craft_ui::testing::{Fixture, Target, trial};
use std::cell::{Cell, RefCell};
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

/// A tour of three steps whose first points at a button, at the step `index` holds; the events go to `events`.
fn three_steps(index: Rc<Cell<usize>>, events: Rc<RefCell<Vec<TourEvent>>>) -> Fixture {
    Fixture::new(move |ui| {
        let layers = Button::new("Layers").show(ui);
        let steps = [
            TourStep::new("Layers", "Every object is a layer.").target(layers.rect),
            TourStep::new("Properties", "Settings."),
            TourStep::new("Export", "Export it."),
        ];
        let mut at = index.get();
        if let Some(event) = Tour::new("Welcome tour", &steps, Bind::bound(&mut at)).show(ui).event {
            events.borrow_mut().push(event);
        }
        index.set(at);
    })
}

/// The buttons named `name`, in tree order.
fn buttons(fixture: &Fixture, name: &str) -> Vec<Target> {
    fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some(name)).collect()
}

fn button(fixture: &Fixture, name: &str) -> Option<Target> {
    buttons(fixture, name).into_iter().next()
}

#[trial]
fn a_step_with_a_target_shows_a_mark_and_one_without_shows_none() {
    let index = Rc::new(Cell::new(1));
    let mut fixture = three_steps(index.clone(), Rc::default());
    fixture.idle(10);
    assert!(fixture.styles_of(std::any::type_name::<TargetMark>()).is_empty(), "a step without a target shows no mark");
    index.set(0);
    fixture.idle(10);
    assert!(!fixture.styles_of(std::any::type_name::<TargetMark>()).is_empty(), "a step with a target shows its mark");
}

#[trial]
fn next_and_back_keep_the_card_and_skip_or_close_leave_on_every_step() {
    let index = Rc::new(Cell::new(0));
    let events: Rc<RefCell<Vec<TourEvent>>> = Rc::default();
    let mut fixture = three_steps(index.clone(), events.clone());
    fixture.idle(10);
    let back = button(&fixture, "Back").expect("Back shows on the first step");
    fixture.click(&back).idle(4);
    assert!(events.borrow().is_empty() && index.get() == 0, "Back is off on the first step");
    let next = button(&fixture, "Next").expect("Next shows");
    fixture.click(&next).idle(4);
    assert_eq!(index.get(), 1, "Next moves on");
    let back = button(&fixture, "Back").expect("the card stays");
    fixture.click(&back).idle(4);
    assert_eq!(index.get(), 0, "Back moves back");
    assert_eq!(events.borrow().as_slice(), [TourEvent::Next, TourEvent::Back]);

    index.set(2);
    fixture.idle(10);
    let skips = buttons(&fixture, "Skip Tour");
    assert_eq!(skips.len(), 2, "the last step has the close button and Skip");
    for skip in skips {
        events.borrow_mut().clear();
        fixture.click(&skip).idle(4);
        assert_eq!(events.borrow().as_slice(), [TourEvent::Skipped], "each leaves the tour on the last step");
    }
}
