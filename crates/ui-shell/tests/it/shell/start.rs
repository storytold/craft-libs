//! `StartScreen`: which part of the page takes a double click, and what a recent file's menu offers.

use craft_ui::contract::{Bind, Role};
use craft_ui::shell::{RecentFile, RecentList, RecentSort, RecentView, StartAction, StartLink, StartScreen};
use craft_ui::testing::pointer::{moved, secondary};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers};
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

/// The row of the recent file named `file`: a list row whose name starts with the file's name and carries its folder.
fn recent_row(fixture: &Fixture, file: &str) -> Option<Target> {
    [Role::LIST_ITEM, Role::LIST_BOX_OPTION]
        .into_iter()
        .flat_map(|role| fixture.by_role(role))
        .find(|row| fixture.tree().name_of(row.node).is_some_and(|name| name.starts_with(file)))
}

/// A recent row opens its file on a click, shows its remove part on hover and reports that file's removal, and the
/// row's menu clears the list.
#[trial]
fn a_recent_row_names_its_full_path_and_reveals_remove_on_hover_or_focus() {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd"), RecentFile::new("/tmp/work/Flyer.psd", "Flyer.psd")];
    let mut fixture = Fixture::new(move |ui| {
        let out = RecentList::new(&files).show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    fixture.room(egui::Vec2::new(700.0, 500.0));
    fixture.settle();
    log.borrow_mut().clear();

    let poster = PathBuf::from("/tmp/work/Poster.psd");
    let row = recent_row(&fixture, "Poster.psd").expect("the row shows");
    let name = fixture.tree().name_of(row.node).map(str::to_owned);
    assert!(name.is_some_and(|text| text.contains("/tmp/work")), "the row's name carries the file's folder");
    fixture.click(&row).idle(2);
    assert!(log.borrow().contains(&StartAction::OpenRecent(poster.clone())), "a click opens the row's file: {:?}", log.borrow());

    // Focus stays on the Flyer row after its click; Enter opens it again, and the pointer far away leaves the remove
    // part revealed by focus alone.
    log.borrow_mut().clear();
    let flyer = recent_row(&fixture, "Flyer.psd").expect("the second row shows");
    fixture.click(&flyer).idle(2);
    log.borrow_mut().clear();
    fixture.frame(vec![moved(egui::pos2(1.0, 1.0))]);
    fixture.idle(2);
    let buttons_focused = fixture.by_role(Role::BUTTON).len();
    fixture.key(Key::Enter, Modifiers::NONE).idle(2);
    assert!(
        log.borrow().contains(&StartAction::OpenRecent(PathBuf::from("/tmp/work/Flyer.psd"))),
        "Enter on the focused row opens its file: {:?}",
        log.borrow()
    );

    log.borrow_mut().clear();
    let at = row.center();
    fixture.frame(vec![moved(at)]);
    fixture.idle(2);
    let revealed = fixture.by_role(Role::BUTTON);
    assert!(revealed.len() > buttons_focused - 1, "hovering a row keeps its remove part shown");
    let remove = revealed.into_iter().find(|button| row.rect.contains(button.center()) && button.center().x > row.center().x).expect("hovering the row reveals its remove part");
    fixture.click(&remove).idle(2);
    assert!(log.borrow().contains(&StartAction::RemoveRecent(poster)), "the remove part removes that row's file: {:?}", log.borrow());

    log.borrow_mut().clear();
    let at = recent_row(&fixture, "Flyer.psd").expect("the second row shows").center();
    fixture.frame(vec![moved(at)]);
    fixture.frame(vec![secondary(at, true)]);
    fixture.frame(vec![secondary(at, false)]);
    fixture.idle(3);
    // The menu lists Open, Remove, Clear and Show in Folder, in that order.
    let clear = fixture.by_role(Role::MENU_ITEM).into_iter().nth(2).expect("the row's menu offers Clear");
    fixture.click(&clear).idle(2);
    assert!(log.borrow().contains(&StartAction::ClearRecent), "Clear reports ClearRecent: {:?}", log.borrow());
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
