//! The page thumbnails: the New Slide row is a row of the grid and asks for a new page; the current page follows a
//! move; an Organize check toggles its page.

use craft_behavior::{Selection, SelectionChange};
use craft_pages::ui::fixtures::{DemoPages, PAPER};
use craft_pages::ui::{PageThumbnails, ThumbnailArrangement, ThumbnailEvent, moved_page};
use craft_ui::contract::{ReorderMove, Role, Show as _};
use craft_ui::testing::{Fixture, trial};
use egui::vec2;
use std::cell::RefCell;
use std::rc::Rc;

struct RecordedPages {
    pages: DemoPages,
    sides: Rc<RefCell<Vec<u32>>>,
}

impl craft_ui::primitives::textures::ThumbSource<usize> for RecordedPages {
    fn revision(&self, key: &usize, request: &craft_ui::primitives::textures::ThumbRequest) -> craft_ui::primitives::textures::Revision {
        self.pages.revision(key, request)
    }

    fn render(&self, key: &usize, request: &craft_ui::primitives::textures::ThumbRequest) -> craft_ui::primitives::textures::ThumbRender {
        self.sides.borrow_mut().push(request.side);
        self.pages.render(key, request)
    }
}

impl craft_pages::ui::PageSource for RecordedPages {
    fn page_count(&self) -> usize {
        self.pages.page_count()
    }

    fn page_size(&self, page: usize) -> craft_behavior::kurbo::Size {
        self.pages.page_size(page)
    }
}

#[trial]
fn zoomed_grid_requests_a_picture_at_the_drawn_size() {
    let sides = Rc::new(RefCell::new(Vec::new()));
    let pages = RecordedPages { pages: DemoPages::new(1), sides: Rc::clone(&sides) };
    let mut picked = Selection::default();
    let mut zoom = 3.0;
    let mut fixture = Fixture::new(move |ui| {
        let _ = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).arrangement(ThumbnailArrangement::Organize).size(&mut zoom).show(ui);
    });
    fixture.room(vec2(16000.0, 12000.0));
    fixture.ctx().set_pixels_per_point(16.0);
    fixture.settle();
    let side = sides.borrow().iter().copied().max().expect("render requested") as f32;
    assert!(side > craft_ui::primitives::textures::ThumbnailOptions::new().max_side as f32, "the zoomed page requested only {side} pixels");
}

/// A wide side panel keeps its page strip in one vertical column.
#[trial]
fn a_wide_thumbnail_strip_keeps_pages_in_one_column() {
    let pages = DemoPages::new(3);
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).show(ui);
    });
    fixture.room(vec2(500.0, 800.0)).settle();
    let first = fixture.by_label("Page 1").expect("first page").rect;
    let second = fixture.by_label("Page 2").expect("second page").rect;
    assert!(second.top() >= first.bottom(), "the strip stays in page order vertically: {first:?}, {second:?}");
}

#[trial]
fn organize_keeps_room_above_and_below_landscape_pages() {
    struct Landscape(DemoPages);
    impl craft_ui::primitives::textures::ThumbSource<usize> for Landscape {
        fn revision(&self, key: &usize, request: &craft_ui::primitives::textures::ThumbRequest) -> craft_ui::primitives::textures::Revision {
            self.0.revision(key, request)
        }
        fn render(&self, key: &usize, request: &craft_ui::primitives::textures::ThumbRequest) -> craft_ui::primitives::textures::ThumbRender {
            self.0.render(key, request)
        }
    }
    impl craft_pages::ui::PageSource for Landscape {
        fn page_count(&self) -> usize {
            1
        }
        fn page_size(&self, _: usize) -> craft_behavior::kurbo::Size {
            craft_behavior::kurbo::Size::new(792.0, 612.0)
        }
    }
    let pages = Landscape(DemoPages::new(1));
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        let _ = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).arrangement(ThumbnailArrangement::Organize).show(ui);
    });
    fixture.room(vec2(600.0, 600.0)).settle();
    let page = fixture.by_label("Page 1").expect("page tile").rect;
    assert!(page.height() > page.width(), "the tile reserves room for its check and actions: {page:?}");
}

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

/// A value the rig and its trial share.
type Shared<T> = Rc<RefCell<T>>;

/// An Organize grid of `count` pages with id `id` and current page `current`; the events it reports and the picked set.
fn organize_rig(count: usize, id: egui::Id, current: usize) -> (Fixture, Shared<Vec<ThumbnailEvent>>, Shared<Selection<usize>>) {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let picked: Rc<RefCell<Selection<usize>>> = Rc::default();
    let held = Rc::clone(&picked);
    let pages = DemoPages::new(count);
    let mut fixture = Fixture::new(move |ui| {
        let mut picked = held.borrow_mut();
        let out = PageThumbnails::new("Pages", &pages, &mut picked, current, PAPER).arrangement(ThumbnailArrangement::Organize).id(id).show(ui);
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

#[trial]
fn escape_clears_the_focused_thumbnail_selection() {
    let (mut fixture, _, picked) = organize_rig(4, egui::Id::unique("organize"), 0);
    fixture.click_at(tile(&fixture, 2)).settle();
    assert!(picked.borrow().contains(&1));
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    assert!(!picked.borrow().contains(&1), "Escape clears the pick");
}

#[trial]
fn delete_over_unfocused_thumbnails_acts_on_the_pick() {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let pages = DemoPages::new(4);
    let mut picked = Selection::default();
    let mut text = String::from("draft");
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::buttons::Button::new("Other").show(ui);
        craft_ui::text::TextField::new("Draft", craft_ui::contract::Bind::bound(&mut text)).show(ui);
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(600.0, 600.0)).settle();
    fixture.click_at(tile(&fixture, 2)).settle();
    let other = fixture.by_label("Other").expect("other control");
    for _ in 0..3 {
        if craft_ui::testing::access::focused_key(fixture.tree()) == other.node {
            break;
        }
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    }
    assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), other.node, "the other control owns focus");
    let at = tile(&fixture, 2);
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]).key(egui::Key::Delete, egui::Modifiers::NONE).settle();
    assert!(
        events.borrow().iter().any(|event| matches!(event, ThumbnailEvent::Delete(pages) if pages.as_slice() == [1])),
        "Delete reaches the picked page"
    );
    let deletes = events.borrow().iter().filter(|event| matches!(event, ThumbnailEvent::Delete(_))).count();
    let field = fixture.by_label("Draft").expect("text field");
    fixture.click(&field).settle();
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]).key(egui::Key::Backspace, egui::Modifiers::NONE).settle();
    assert_eq!(
        events.borrow().iter().filter(|event| matches!(event, ThumbnailEvent::Delete(_))).count(),
        deletes,
        "the text field keeps its editing key"
    );
}

#[trial]
fn delete_over_unfocused_thumbnails_uses_the_current_page_when_unpicked() {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let pages = DemoPages::new(4);
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        craft_ui::buttons::Button::new("Other").show(ui);
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 1, PAPER).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(600.0, 600.0)).settle();
    let other = fixture.by_label("Other").expect("other control");
    for _ in 0..3 {
        if craft_ui::testing::access::focused_key(fixture.tree()) == other.node {
            break;
        }
        fixture.key(egui::Key::Tab, egui::Modifiers::NONE).settle();
    }
    assert_eq!(craft_ui::testing::access::focused_key(fixture.tree()), other.node);
    let at = tile(&fixture, 2);
    fixture.frame(vec![craft_ui::testing::pointer::moved(at)]).key(egui::Key::Delete, egui::Modifiers::NONE).settle();
    assert!(
        events.borrow().iter().any(|event| matches!(event, ThumbnailEvent::Delete(pages) if pages.as_slice() == [1])),
        "Delete reaches the current page"
    );
}

#[trial]
fn strip_drag_inserts_after_the_lower_half_of_a_page() {
    let events: Rc<RefCell<Vec<ThumbnailEvent>>> = Rc::default();
    let seen = Rc::clone(&events);
    let pages = DemoPages::new(4);
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER).show(ui);
        seen.borrow_mut().extend(out.events);
    });
    fixture.room(vec2(220.0, 800.0));
    fixture.settle();
    let from = tile(&fixture, 1);
    let rect = fixture.by_label("Page 3").expect("third page").rect;
    let to = egui::pos2(rect.left() + rect.width() * 0.25, rect.top() + rect.height() * 0.75);
    fixture.drag_at(from, to - from).settle();
    let events = events.borrow();
    assert!(events.iter().any(|event| matches!(event, ThumbnailEvent::Reorder(craft_behavior::EditEvent::Set { after, .. } | craft_behavior::EditEvent::Commit { after, .. }) if after.to == 2)), "{events:?}");
}

/// A drag the host ends with `cancel_press` (its pages changed under it) lands nothing on its release, and the next
/// drag lands its move on its release as any drag does.
#[trial]
fn after_a_cancelled_drag_the_next_drag_lands_its_move() {
    let id = egui::Id::unique("organize");
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
        let (mut fixture, _, picked) = organize_rig(5, egui::Id::unique("organize"), 1);
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

/// Each open of the Organize grid reveals the page the host asks for, once: the second open too, after a click
/// picked a page of the first and the grid was hidden in between.
#[trial]
fn every_open_of_the_grid_reveals_the_asked_page() {
    let open = Rc::new(std::cell::Cell::new(true));
    let reveal = Rc::new(std::cell::Cell::new(Some(30_usize)));
    let current = Rc::new(std::cell::Cell::new(30_usize));
    let navigated = Rc::new(std::cell::Cell::new(false));
    let observed = Rc::clone(&navigated);
    let picked: Rc<RefCell<Selection<usize>>> = Rc::default();
    let (shown, asked, at, held) = (Rc::clone(&open), Rc::clone(&reveal), Rc::clone(&current), Rc::clone(&picked));
    let pages = DemoPages::new(60);
    let mut zoom = 1.0;
    let mut fixture = Fixture::new(move |ui| {
        if shown.get() {
            let mut picked = held.borrow_mut();
            let out = PageThumbnails::new("Pages", &pages, &mut picked, at.get(), PAPER)
                .arrangement(ThumbnailArrangement::Organize)
                .size(&mut zoom)
                .reveal(asked.take())
                .show(ui);
            for event in out.events {
                if let ThumbnailEvent::Go(page) = event {
                    observed.set(true);
                    at.set(page);
                }
            }
        }
    });
    fixture.room(vec2(600.0, 600.0));
    fixture.settle();
    assert!(!navigated.get(), "revealing the current page does not navigate the document");
    let in_view = |fixture: &Fixture, n: usize| {
        let grid = fixture.by_role(Role::LIST_BOX).first().map(|node| node.rect).expect("the grid");
        fixture.by_label(&format!("Page {n}")).is_some_and(|node| grid.contains_rect(node.rect))
    };
    assert!(in_view(&fixture, 31), "the first open shows page 31");
    let page = tile(&fixture, 32);
    fixture.click_at(page).settle();
    open.set(false);
    fixture.settle();
    current.set(45);
    *picked.borrow_mut() = Selection::default();
    open.set(true);
    reveal.set(Some(45));
    fixture.settle();
    assert!(in_view(&fixture, 46), "the second open shows page 46");
    assert!(!in_view(&fixture, 31), "and not the first open's pages");
}

/// The grid reports its latched middle-button auto-scroll while it runs, so a host can show and end it.
#[trial]
fn the_grid_reports_its_latched_auto_scroll() {
    let latched = Rc::new(std::cell::Cell::new(false));
    let seen = Rc::clone(&latched);
    let pages = DemoPages::new(60);
    let mut picked = Selection::default();
    let mut fixture = Fixture::new(move |ui| {
        let out = PageThumbnails::new("Pages", &pages, &mut picked, 0, PAPER)
            .arrangement(ThumbnailArrangement::Organize)
            .middle_button(craft_behavior::MiddleButton::AutoScroll)
            .show(ui);
        seen.set(out.auto_scrolling);
    });
    fixture.room(vec2(600.0, 600.0));
    fixture.settle();
    let at = tile(&fixture, 2);
    let middle = |pressed| egui::Event::PointerButton { pos: at, button: egui::PointerButton::Middle, pressed, modifiers: egui::Modifiers::NONE };
    fixture.frame(vec![egui::Event::PointerMoved(at), middle(true)]);
    fixture.frame(vec![middle(false)]);
    fixture.frame(Vec::new());
    assert!(latched.get(), "a middle click latches the grid's auto-scroll");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE);
    fixture.frame(Vec::new());
    assert!(!latched.get(), "Escape ends it");
}

/// In the strip the picked pages are kept beside the current page: Cmd/Ctrl- and Shift-clicks change them without
/// going to a page, and a plain click goes to its page and empties them; the next Shift-click ranges from there.
#[trial]
fn strip_picks_stay_beside_the_current_page() {
    let picked: Rc<RefCell<Selection<usize>>> = Rc::default();
    let held = Rc::clone(&picked);
    let current = Rc::new(std::cell::Cell::new(0_usize));
    let shown = Rc::clone(&current);
    let pages = DemoPages::new(5);
    let mut fixture = Fixture::new(move |ui| {
        let mut picked = held.borrow_mut();
        let out = PageThumbnails::new("Pages", &pages, &mut picked, shown.get(), PAPER).picks_beside_current(true).show(ui);
        for event in out.events {
            if let ThumbnailEvent::Go(page) = event {
                shown.set(page);
            }
        }
    });
    fixture.room(vec2(300.0, 1600.0));
    fixture.settle();
    let got = |picked: &RefCell<Selection<usize>>| (0..5).filter(|page| picked.borrow().contains(page)).collect::<Vec<_>>();

    let page = tile(&fixture, 3);
    fixture.hold(egui::Modifiers::COMMAND).click_at(page).settle();
    fixture.hold(egui::Modifiers::NONE);
    assert_eq!(got(&picked), [0, 2], "a Cmd/Ctrl-click picks the page beside the current one");
    assert_eq!(current.get(), 0, "a Cmd/Ctrl-click does not go to the page");

    let page = tile(&fixture, 2);
    fixture.click_at(page).settle();
    assert_eq!(current.get(), 1, "a plain click goes to the page");
    assert!(got(&picked).is_empty(), "a plain click empties the pick: {:?}", got(&picked));

    let page = tile(&fixture, 4);
    fixture.hold(egui::Modifiers::SHIFT).click_at(page).settle();
    fixture.hold(egui::Modifiers::NONE);
    assert_eq!(got(&picked), [1, 2, 3], "a Shift-click ranges from the page the plain click went to");
    assert_eq!(current.get(), 1, "a Shift-click does not go to the page");
}
