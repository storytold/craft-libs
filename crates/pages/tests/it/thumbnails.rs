//! The page thumbnails: the New Slide row is a row of the grid and asks for a new page; the current page follows a
//! move; an Organize check toggles its page.

use craft_behavior::{Selection, SelectionChange};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{PageThumbnails, ThumbnailArrangement, ThumbnailEvent, moved_page};
use craft_ui::contract::{ReorderMove, Role};
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

/// Each page's index after a move, for a block moved to the end and for one page moved to the front.
#[trial]
fn a_moved_page_keeps_its_identity_across_a_move() {
    let count = 5;
    // Pages 1 and 2 go after the rest: 0, 3, 4, 1, 2.
    let to_end = ReorderMove { key: 1_usize, keys: vec![1, 2], to: 3, copy: false, parent: None };
    let moved: Vec<usize> = (0..count).map(|page| moved_page(&to_end, page, count)).collect();
    assert_eq!(moved, vec![0, 3, 4, 1, 2]);
    // Page 3 goes first: 3, 0, 1, 2, 4.
    let to_front = ReorderMove { key: 3_usize, keys: vec![3], to: 0, copy: false, parent: None };
    let moved: Vec<usize> = (0..count).map(|page| moved_page(&to_front, page, count)).collect();
    assert_eq!(moved, vec![1, 2, 3, 0, 4]);
}

/// In Organize a click on a page's check adds it to the picked pages without a modifier, and goes to it.
#[trial]
fn an_organize_check_toggles_its_page_without_modifiers_and_goes_to_it() {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let picked: Rc<RefCell<Selection<usize>>> = Rc::default();
    let held = Rc::clone(&picked);
    let pages = DemoPages::new(3);
    let mut fixture = Fixture::new(move |ui| {
        let mut picked = held.borrow_mut();
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 2, PAPER).arrangement(ThumbnailArrangement::Organize).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(600.0, 600.0));
    fixture.settle();
    for page in [0, 1] {
        let checks = fixture.by_role(Role::CHECK_BOX);
        let check = checks.get(page).cloned().expect("a check box on each page");
        fixture.click(&check).settle();
    }
    let events = events.borrow();
    for page in [0_usize, 1] {
        let expected = [ThumbnailEvent::Selection(SelectionChange::Toggle(page)), ThumbnailEvent::Go(page)];
        assert!(events.windows(2).any(|pair| pair == expected), "page {page}: {events:?}");
    }
    let picked = picked.borrow();
    assert!(picked.contains(&0) && picked.contains(&1) && !picked.contains(&2), "both checked pages are picked: {picked:?}");
}
