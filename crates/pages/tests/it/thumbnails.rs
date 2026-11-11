//! The page thumbnails: the New Slide row is a row of the grid and asks for a new page.

use craft_behavior::Selection;
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{PageThumbnails, ThumbnailEvent};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;
use std::cell::RefCell;
use std::rc::Rc;

/// A click on the New Slide row asks for a new page and goes to no page.
#[trial]
fn the_new_slide_row_asks_for_a_new_page() {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let pages = DemoPages::new(2);
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).new_page_row(true).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(300.0, 600.0));
    fixture.settle();
    let row = fixture.by_label("New Slide").expect("the New Slide row");
    fixture.click(&row).settle();
    let events = events.borrow();
    assert!(events.iter().any(|event| matches!(event, ThumbnailEvent::NewPage)), "{events:?}");
    assert!(!events.iter().any(|event| matches!(event, ThumbnailEvent::Go(_) | ThumbnailEvent::Activate(_))), "{events:?}");
}
