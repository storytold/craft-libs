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

/// An Organize grid of `count` pages with id `id` and current page `current`; the events it reports and the picked set.
fn organize_rig(count: usize, id: egui::Id, current: usize) -> (Fixture, Rc<RefCell<Vec<ThumbnailEvent>>>, Rc<RefCell<Selection<usize>>>) {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let picked: Rc<RefCell<Selection<usize>>> = Rc::default();
    let held = Rc::clone(&picked);
    let pages = DemoPages::new(count);
    let mut fixture = Fixture::new(move |ui| {
        let mut picked = held.borrow_mut();
        let out =
            PageThumbnails::new("Pages", &pages, &mut picked, current, PAPER).arrangement(ThumbnailArrangement::Organize).id(id).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(600.0, 600.0));
    fixture.settle();
    (fixture, events, picked)
}

/// The middle of page `n`'s tile (counted from 1).
fn tile(fixture: &Fixture, n: usize) -> egui::Pos2 {
    fixture.by_label(&format!("Page {n}")).expect("a tile per page").center()
}

/// A drag the host ends with `cancel_press` (its pages changed under it) lands nothing on its release, and the next
/// drag lands its move on its release as any drag does.
#[trial]
fn after_a_cancelled_drag_the_next_drag_lands_its_move() {
    let id = egui::Id::new("organize");
    let (mut fixture, events, _) = organize_rig(4, id, 0);
    let (from, to) = (tile(&fixture, 1), tile(&fixture, 3));
    let reorders = |events: &RefCell<Vec<ThumbnailEvent>>| events.borrow().iter().filter(|event| matches!(event, ThumbnailEvent::Reorder(_))).count();
    fixture.drag_hold_at(from, to - from);
    PageThumbnails::cancel_press(fixture.ctx(), id);
    fixture.frame(Vec::new());
    fixture.release_at(to).settle();
    assert_eq!(reorders(&events), 0, "the cancelled drag lands nothing: {:?}", events.borrow());
    fixture.drag_at(from, to - from).settle();
    assert_eq!(reorders(&events), 1, "the next drag lands its move: {:?}", events.borrow());
}

/// With no page picked, the current page counts as picked: a Cmd/Ctrl-click on another page picks both, and a
/// Shift-click picks the run from the current page.
#[trial]
fn a_modified_click_on_an_empty_pick_starts_from_the_current_page() {
    for (modifiers, want) in [(egui::Modifiers::COMMAND, vec![1, 3]), (egui::Modifiers::SHIFT, vec![1, 2, 3])] {
        let (mut fixture, _, picked) = organize_rig(5, egui::Id::new("organize"), 1);
        let page = tile(&fixture, 4);
        fixture.hold(modifiers).click_at(page).settle();
        fixture.hold(egui::Modifiers::NONE);
        let picked = picked.borrow();
        let got: Vec<usize> = (0..5).filter(|page| picked.contains(page)).collect();
        assert_eq!(got, want, "{modifiers:?}-click on page 4 with page 2 current");
    }
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
