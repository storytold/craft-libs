//! `StartScreen`: which part of the page takes a double click, and what a recent file's menu offers.

use craft_ui::contract::Bind;
use craft_ui::shell::{RecentFile, RecentSort, RecentView, StartAction, StartLink, StartScreen};
use craft_ui::testing::pointer::{moved, secondary};
use craft_ui::testing::{Fixture, Target, trial};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

type Log = Rc<RefCell<Vec<StartAction>>>;

/// A start screen of two recent files and a link in a room wide enough for the rail; every action it reports goes to
/// the log.
fn screen() -> (Fixture, Log) {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd"), RecentFile::new("/tmp/work/Flyer.psd", "Flyer.psd")];
    let links = vec![StartLink::url("Docs", None, "https://example.test/docs")];
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).links(&links).show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0));
    fixture.settle();
    log.borrow_mut().clear();
    (fixture, log)
}

/// Two clicks at one point, inside the double-click time.
fn double_click(fixture: &mut Fixture, target: &Target) {
    let at = target.center();
    fixture.click_at(at).click_at(at);
}

fn opens(log: &Log) -> usize {
    log.borrow().iter().filter(|action| **action == StartAction::Open).count()
}

/// The page's own room takes a double click as Open; a card, a button and a rail link keep theirs, because they sit
/// above the page's room.
#[trial]
fn a_double_click_on_the_empty_room_opens_and_one_on_a_card_opens_only_its_file() {
    let (mut fixture, log) = screen();

    let corner = fixture.shown().max - egui::vec2(30.0, 30.0);
    fixture.click_at(corner).click_at(corner);
    assert_eq!(*log.borrow(), vec![StartAction::Open], "a double click on empty room opens a file once");

    log.borrow_mut().clear();
    let card = fixture.by_label("Poster.psd").expect("the card shows");
    double_click(&mut fixture, &card);
    let poster = PathBuf::from("/tmp/work/Poster.psd");
    assert!(!log.borrow().is_empty(), "a click on a card opens its file");
    assert!(log.borrow().iter().all(|action| *action == StartAction::OpenRecent(poster.clone())), "only the card's file opens: {:?}", log.borrow());

    log.borrow_mut().clear();
    let new = fixture.by_label("New…").expect("New shows");
    double_click(&mut fixture, &new);
    assert_eq!(opens(&log), 0, "a double click on New does not open a file: {:?}", log.borrow());

    log.borrow_mut().clear();
    let open = fixture.by_label("Open…").expect("Open shows");
    fixture.click(&open);
    let single = opens(&log);
    log.borrow_mut().clear();
    // The double click starts a new click count.
    fixture.advance(1.0);
    double_click(&mut fixture, &open);
    assert_eq!(opens(&log), 2 * single, "Open reports its own two clicks and the page's room adds none");

    log.borrow_mut().clear();
    let link = fixture.by_label("Docs").expect("the rail link shows");
    double_click(&mut fixture, &link);
    assert_eq!(opens(&log), 0, "a double click on a rail link opens no file: {:?}", log.borrow());
}

/// A recent file's menu offers Show in Folder exactly where the platform can reveal a file.
#[trial]
fn the_recent_menu_offers_show_in_folder_where_the_platform_can_reveal() {
    let (mut fixture, _log) = screen();
    let card = fixture.by_label("Poster.psd").expect("the card shows");
    fixture.click(&card).idle(2);
    let at = card.center();
    fixture.frame(vec![moved(at)]);
    fixture.frame(vec![secondary(at, true)]);
    fixture.frame(vec![secondary(at, false)]);
    fixture.idle(3);
    assert!(fixture.by_label("Remove from Recent").is_some(), "a right click on a recent file opens its menu");
    assert_eq!(fixture.by_label("Show in Folder").is_some(), craft_platform::open::CAN_REVEAL);
}
