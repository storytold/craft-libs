//! `StartScreen`: which part of the page takes a double click, what a recent file's menu offers, where host sections
//! sit, and how a pinned folder lists its files.

use craft_theme::{AppSizes, SizeValue};
use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _};
use craft_ui::display::Text;
use craft_ui::runtime::env::{Env, RootEnv};
use craft_ui::shell::{
    FolderListing, PINNED_FILES_SHOWN, PinnedFile, PinnedFolder, RecentFile, RecentList, RecentSort, RecentView, ShellWords, StartAction,
    StartLayout, StartLink, StartPlace, StartScreen, Template,
};
use craft_ui::testing::pointer::{moved, secondary};
use craft_ui::testing::{Fixture, Target, trial};
use egui::{Key, Modifiers};
use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;

type Log = Rc<RefCell<Vec<StartAction>>>;

#[trial]
fn recent_files_follow_the_supplied_list_when_cleared() {
    let files = Rc::new(RefCell::new(vec![
        RecentFile::new("/tmp/first", "First document"),
        RecentFile::new("/tmp/second", "Second document"),
        RecentFile::new("/tmp/third", "Third document"),
    ]));
    let supplied = files.clone();
    let mut fixture = Fixture::new(move |ui| {
        let (mut view, mut sort) = (RecentView::List, RecentSort::Opened);
        let current = supplied.borrow().clone();
        let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &current)
            .drop_hint("Drop files here")
            .section(StartPlace::BeforeRecent, |ui, _| {
                Text::new("Welcome").show(ui);
            })
            .show(ui);
        for action in out.actions {
            match action {
                StartAction::RemoveRecent(path) => supplied.borrow_mut().retain(|file| file.path != path),
                StartAction::ClearRecent => supplied.borrow_mut().clear(),
                _ => {}
            }
        }
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0)).settle();
    let row = fixture.by_label("Second document, /tmp").expect("the recent row");
    fixture.frame(vec![moved(row.center())]).idle(2);
    let remove = fixture.by_label("Remove Second document from Recent").expect("the remove button");
    fixture.click(&remove).idle(2);
    assert_eq!(files.borrow().len(), 2);
    let row = fixture.by_label("First document, /tmp").expect("the recent row");
    let at = row.center();
    fixture.frame(vec![moved(at)]).frame(vec![secondary(at, true)]).frame(vec![secondary(at, false)]).idle(2);
    let clear = fixture.by_label("Clear Recent Files").expect("the recent menu");
    fixture.click(&clear).idle(3);
    assert!(files.borrow().is_empty());
    assert!(fixture.by_label("First document, /tmp").is_none(), "removed files no longer appear");
    assert!(fixture.by_label("Drop files here").is_some(), "the empty hint appears");
}

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
    let remove = revealed
        .into_iter()
        .find(|button| row.rect.contains(button.center()) && button.center().x > row.center().x)
        .expect("hovering the row reveals its remove part");
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

/// A recent file's menu offers Show in Folder exactly where the platform can reveal a file, and the menu key (off
/// macOS, which has none) and Shift+F10 open it from the focused card. The menu is told by "Clear Recent Files", which only the menu has: the
/// card's hover action carries "Remove from Recent" too.
#[trial]
fn the_recent_menu_offers_show_in_folder_where_the_platform_can_reveal() {
    const MENU_ONLY: &str = "Clear Recent Files";
    let (mut fixture, _log) = screen();
    let card = fixture.by_label("Poster.psd").expect("the card shows");
    fixture.click(&card).idle(2);
    let at = card.center();
    fixture.frame(vec![moved(at)]);
    fixture.frame(vec![secondary(at, true)]);
    fixture.frame(vec![secondary(at, false)]);
    fixture.idle(3);
    assert!(fixture.by_label(MENU_ONLY).is_some(), "a right click on a recent file opens its menu");
    assert_eq!(fixture.by_label("Show in Folder").is_some(), craft_platform::open::CAN_REVEAL);

    let keys: &[(Key, Modifiers)] =
        if cfg!(target_os = "macos") { &[(Key::F10, Modifiers::SHIFT)] } else { &[(Key::Menu, Modifiers::NONE), (Key::F10, Modifiers::SHIFT)] };
    for &(key, modifiers) in keys {
        fixture.key(Key::Escape, Modifiers::NONE).idle(3);
        assert!(fixture.by_label(MENU_ONLY).is_none(), "Escape closes the menu before {key:?}");
        let card = fixture.by_label("Poster.psd").expect("the card shows");
        fixture.click(&card).idle(2);
        fixture.key(key, modifiers).idle(3);
        assert!(fixture.by_label(MENU_ONLY).is_some(), "{key:?} with {modifiers:?} opens the focused card's menu");
    }
}

/// A start screen of `files` in `view`, in a room wide enough for the rail; every action goes to the log.
fn recent_screen(files: Vec<RecentFile>, view: RecentView) -> (Fixture, Log) {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let (mut view, mut sort) = (view, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    fixture.room(egui::Vec2::new(1100.0, 900.0)).settle();
    log.borrow_mut().clear();
    (fixture, log)
}

/// A card's remove action and a list row's revealed remove button ask to remove that file alone, and the recent menu's
/// Clear asks to clear the list.
#[trial]
fn a_recent_file_is_removed_from_its_card_or_row_and_the_menu_clears_them_all() {
    let files = || vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd"), RecentFile::new("/tmp/work/Flyer.psd", "Flyer.psd")];
    let flyer = PathBuf::from("/tmp/work/Flyer.psd");

    let (mut grid, log) = recent_screen(files(), RecentView::Grid);
    let card = target(&grid, "Flyer.psd");
    grid.frame(vec![moved(card.center())]).idle(2);
    let remove = target(&grid, "Remove from Recent");
    grid.click(&remove).idle(2);
    assert_eq!(*log.borrow(), vec![StartAction::RemoveRecent(flyer.clone())], "the hovered card's action removes its file alone");

    let (mut list, log) = recent_screen(files(), RecentView::List);
    // A row is named by its name and its folder.
    let row = target(&list, "Flyer.psd, /tmp/work");
    list.frame(vec![moved(row.center())]).idle(2);
    let remove = target(&list, "Remove Flyer.psd from Recent");
    list.click(&remove).idle(2);
    assert_eq!(*log.borrow(), vec![StartAction::RemoveRecent(flyer)], "the row's remove button removes its file alone");

    log.borrow_mut().clear();
    let at = target(&list, "Poster.psd, /tmp/work").center();
    list.frame(vec![moved(at)]);
    list.frame(vec![secondary(at, true)]);
    list.frame(vec![secondary(at, false)]);
    list.idle(3);
    let clear = target(&list, "Clear Recent Files");
    list.click(&clear).idle(2);
    assert_eq!(*log.borrow(), vec![StartAction::ClearRecent], "the menu's Clear asks to clear the list");
}

/// In the list, a long name and a long folder path are cut short and end before the row's trailing detail, which shows
/// whole.
#[trial]
fn a_long_recent_name_and_folder_are_cut_before_the_detail() {
    let name = format!("{}.pdf", "Quarterly report final version ".repeat(10));
    let folder = format!("/{}", "A folder with a long name".repeat(8));
    let file = RecentFile::new(format!("{folder}/{name}"), name.clone()).note("7 pages");
    let (fixture, _) = recent_screen(vec![file], RecentView::List);
    let texts = fixture.texts();
    let Some(detail) = texts.iter().find(|text| text.text == "7 pages") else { panic!("the row shows its detail") };
    assert!(!detail.elided, "the detail shows whole");
    for whole in [&name, &folder] {
        let Some(text) = texts.iter().find(|text| text.text == *whole) else { panic!("the row shows {whole:?}") };
        assert!(text.elided, "{whole:?} is cut short");
        assert!(text.rect.right() <= detail.rect.left(), "{whole:?} ends before the detail: {:?}, {:?}", text.rect, detail.rect);
    }
}

/// The template's detail line; a tile with a detail is named "name, detail".
const TEMPLATE_DETAIL: &str = "210 × 297 mm";

/// How often the host section before Recent repeats its line. The line is one unwrapped run, so the section's least
/// width is wider than the room where the plain page has its rail beside.
const HOST_TEXT_REPEATS: usize = 14;

/// A start screen of one recent file, a template and a link, with `offers_new`; with `sections`, a host section before
/// Recent (labelled "Before", with a long line) and one after the built-in sections ("After").
fn host_screen(sections: bool, offers_new: bool) -> Fixture {
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")];
    let templates = vec![Template::new("a4", "A4", TEMPLATE_DETAIL)];
    let links = vec![StartLink::url("Docs", None, "https://example.test/docs")];
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let mut start =
            StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).templates(&templates).links(&links).offers_new(offers_new);
        if sections {
            start = start
                .section(StartPlace::BeforeRecent, |ui, _| {
                    let _ = Button::new("Before").show(ui);
                    let _ = Text::new("Before the recent files ".repeat(HOST_TEXT_REPEATS)).show(ui);
                })
                .section(StartPlace::AfterSections, |ui, _| {
                    let _ = Button::new("After").show(ui);
                });
        }
        let _ = start.show(ui);
    });
    fixture.room(egui::Vec2::new(1100.0, 2400.0)).settle();
    fixture
}

fn target(fixture: &Fixture, label: &str) -> Target {
    fixture.by_label(label).unwrap_or_else(|| panic!("{label} shows"))
}

/// Whether the rail's link stands beside the page: left of the recent card.
fn rail_beside(fixture: &Fixture) -> bool {
    target(fixture, "Docs").rect.max.x <= target(fixture, "Poster.psd").rect.min.x
}

/// Host sections sit at their place, count in the least width that decides whether the rail fits beside the page,
/// and `offers_new(false)` leaves out both New buttons with Open as the first Tab stop.
#[trial]
fn host_sections_sit_at_their_place_and_count_in_the_rail_decision() {
    let mut plain = host_screen(false, true);
    let mut hosted = host_screen(true, true);
    assert!(rail_beside(&plain), "the plain page has its rail beside it at 1100 points");
    assert!(!rail_beside(&hosted), "the host section's least width does not fit beside the rail at 1100 points");
    let after = target(&hosted, "After");
    assert!(target(&hosted, "Docs").rect.min.y >= after.rect.max.y, "without the rail the links follow the host sections");

    for fixture in [&mut plain, &mut hosted] {
        fixture.room(egui::Vec2::new(2600.0, 2400.0)).settle();
    }
    assert!(rail_beside(&hosted), "a room that holds the host section's least width has the rail beside the page");
    let before = target(&hosted, "Before");
    assert!(before.rect.max.y <= target(&hosted, "Poster.psd").rect.min.y, "BeforeRecent sits above Recent");
    let after = target(&hosted, "After");
    assert!(after.rect.min.y >= target(&hosted, &format!("A4, {TEMPLATE_DETAIL}")).rect.max.y, "AfterSections sits below Templates");

    let mut no_new = host_screen(false, false);
    assert!(no_new.by_label("New…").is_none(), "without New the page row has no New");
    no_new.room(egui::Vec2::new(2600.0, 2400.0)).settle();
    assert!(rail_beside(&no_new), "the wide room has the rail");
    assert!(no_new.by_label("New…").is_none(), "without New the rail has no New");
    no_new.key(Key::Tab, Modifiers::NONE).idle(2);
    let open = target(&no_new, "Open…");
    assert!(no_new.tree().focused_key() == open.node, "Open is the first Tab stop");
}

type Pins = Rc<RefCell<Vec<PinnedFolder>>>;

/// The notes a pending and an unavailable folder show, as the screen's words give them.
type Notes = Rc<RefCell<Vec<String>>>;

/// A start screen of the pinned folders the test holds, read every pass, in a room tall enough for all of them. The
/// notes are the screen's own words for a pending and an unavailable folder.
fn pinned_screen(folders: Vec<PinnedFolder>) -> (Fixture, Log, Pins, Notes) {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let pins: Pins = Rc::new(RefCell::new(folders));
    let held = Rc::clone(&pins);
    let notes: Notes = Rc::default();
    let words_seen = Rc::clone(&notes);
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let folders = held.borrow().clone();
        let env = Env::of(ui);
        let words = env.words::<ShellWords>();
        *words_seen.borrow_mut() =
            [&words.looking_for_files, &words.folder_unavailable].map(|word| env.view().resolve(word).as_str().to_owned()).to_vec();
        let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &[]).pinned(&folders).show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    fixture.room(egui::Vec2::new(1100.0, 2400.0)).settle();
    log.borrow_mut().clear();
    (fixture, log, pins, notes)
}

fn file_name(n: usize) -> String {
    format!("File {n}.pdf")
}

/// A listed folder shows its first files and a More row; the app's `expanded` and `folded` decide what shows, and
/// the More row, the disclosure keys and a file row report what the app does.
#[trial]
fn a_listed_pinned_folder_folds_shows_more_and_opens_its_files() {
    let work = PathBuf::from("/tmp/Work");
    let count = PINNED_FILES_SHOWN + 2;
    let files = (0..count).map(|n| PinnedFile::new(work.join(file_name(n)), file_name(n))).collect();
    let folders = vec![
        PinnedFolder::new(&work, "Work").listing(FolderListing::Listed { files, total: count }),
        PinnedFolder::new("/tmp/Inbox", "Inbox").listing(FolderListing::Pending),
        PinnedFolder::new("/mnt/Gone", "Gone").listing(FolderListing::Unavailable),
        PinnedFolder::new("/tmp/Empty", "Empty").listing(FolderListing::Listed { files: Vec::new(), total: 0 }),
    ];
    let (mut fixture, log, pins, notes) = pinned_screen(folders);

    for n in 0..count {
        assert_eq!(fixture.by_label(&file_name(n)).is_some(), n < PINNED_FILES_SHOWN, "only the first {PINNED_FILES_SHOWN} files show: {n}");
    }
    let painted = |fixture: &Fixture, text: &str| fixture.texts().iter().any(|painted| painted.text == text);
    // Owned: the screen writes the notes again in every pass, and a borrow held across the frames below would panic it.
    let notes = notes.borrow().clone();
    assert!(painted(&fixture, &notes[0]), "a pending folder says it is still listing");
    assert!(painted(&fixture, &notes[1]), "an unavailable folder says so");

    let more = target(&fixture, "Show more");
    fixture.click(&more).idle(2);
    assert!(log.borrow().contains(&StartAction::ExpandPinned(work.clone())), "More asks to expand: {:?}", log.borrow());

    log.borrow_mut().clear();
    pins.borrow_mut()[0].expanded = true;
    fixture.settle();
    for n in 0..count {
        assert!(fixture.by_label(&file_name(n)).is_some(), "an expanded folder lists every file: {n}");
    }
    let less = target(&fixture, "Show less");
    fixture.click(&less).idle(2);
    assert!(log.borrow().contains(&StartAction::ExpandPinned(work.clone())), "Show less reports the same toggle: {:?}", log.borrow());

    log.borrow_mut().clear();
    let last = target(&fixture, &file_name(count - 1));
    fixture.click(&last).idle(2);
    assert!(log.borrow().contains(&StartAction::OpenPinnedFile(work.join(file_name(count - 1)))), "a file row opens its file: {:?}", log.borrow());

    // Left on the open heading is the disclosure's close.
    // A folder row is named by its name and its path.
    let heading_name = format!("Work, {}", work.display());
    let heading = target(&fixture, &heading_name);
    fixture.click(&heading).idle(2);
    log.borrow_mut().clear();
    fixture.key(Key::ArrowLeft, Modifiers::NONE).idle(2);
    assert_eq!(*log.borrow(), vec![StartAction::FoldPinned(work.clone())], "the disclosure asks the app to fold the folder");

    pins.borrow_mut()[0].folded = true;
    fixture.settle();
    assert!((0..count).all(|n| fixture.by_label(&file_name(n)).is_none()), "a folded folder shows no file row");
    assert!(fixture.by_label("Show less").is_none(), "a folded folder shows no More row");
    assert!(fixture.by_label(&heading_name).is_some(), "a folded folder keeps its heading");

    // Every folder the app lists folds, one with no files too.
    let empty = target(&fixture, "Empty, /tmp/Empty");
    fixture.click(&empty).idle(2);
    log.borrow_mut().clear();
    fixture.key(Key::ArrowLeft, Modifiers::NONE).idle(2);
    assert_eq!(*log.borrow(), vec![StartAction::FoldPinned(PathBuf::from("/tmp/Empty"))], "an empty folder's disclosure folds it");
}

/// A start screen of one recent file and one pinned folder in a wide room, with `pdfcraft` turning on PdfCraft's
/// look (`pinned_first`, `heading_sections`); every action goes to the log.
fn folders_screen(pdfcraft: bool) -> (Fixture, Log) {
    let log: Log = Rc::default();
    let seen = log.clone();
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")];
    let folders = vec![PinnedFolder::new("/tmp/Work", "Work")];
    let mut fixture = Fixture::new(move |ui| {
        let (mut view, mut sort) = (RecentView::List, RecentSort::Opened);
        let screen = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).pinned(&folders);
        let screen = if pdfcraft { screen.pinned_first().heading_sections() } else { screen };
        seen.borrow_mut().extend(screen.show(ui).actions);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0)).settle();
    (fixture, log)
}

/// The top of the first painted run reading `text`.
fn text_top(fixture: &Fixture, text: &str) -> Option<f32> {
    fixture.texts().iter().find(|run| run.text == text).map(|run| run.rect.min.y)
}

/// Upstream's start screen by default: the pinned folders follow the recent files, and the Recent heading has no
/// Clear button.
#[trial]
fn recent_comes_before_folders_and_has_no_clear_button_by_default() {
    let (fixture, _) = folders_screen(false);
    let pinned = text_top(&fixture, "Pinned Folders").expect("the pinned folders heading");
    let recent = text_top(&fixture, "Recent").expect("the recent heading");
    assert!(recent < pinned, "the recent files ({recent}) come before the folders ({pinned})");
    assert!(fixture.by_label("Clear").is_none(), "no Clear button by default");
}

/// With `pinned_first` and `heading_sections` (PdfCraft's look) the pinned folders stand above the recent files, as
/// production's Folders section does, and the Recent heading carries a Clear button that asks the app to clear the
/// list.
#[trial]
fn folders_come_before_recent_and_recent_has_a_clear_button() {
    let (mut fixture, log) = folders_screen(true);
    let pinned = text_top(&fixture, "Pinned Folders").expect("the pinned folders heading");
    let recent = text_top(&fixture, "Recent").expect("the recent heading");
    assert!(pinned < recent, "the folders ({pinned}) come before the recent files ({recent})");
    let clear = fixture.by_label("Clear").expect("the Recent heading's Clear button");
    fixture.click(&clear).idle(2);
    assert!(log.borrow().iter().any(|action| matches!(action, StartAction::ClearRecent)), "Clear asks the app to clear the list");
}

/// An app that opens files from its title bar leaves Open out of the page (`offers_open(false)`); the page's empty
/// room still asks for Open on a double click, and with `heading_sections` the section headings are the heading role,
/// not the panel's emphasised body text.
#[trial]
fn open_can_be_left_out_and_the_headings_are_page_headings() {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")];
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let out =
            StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).offers_new(false).offers_open(false).heading_sections().show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0)).settle();
    assert!(fixture.by_label("Open…").is_none(), "no Open button on the page");
    assert!(fixture.by_label("Poster.psd").is_some(), "the page still lists its recent files");
    let texts = fixture.texts();
    let recent = texts.iter().find(|run| run.text == "Recent").expect("the Recent heading");
    let file = texts.iter().find(|run| run.text == "Poster.psd").expect("a card's name");
    assert!(recent.rect.height() > file.rect.height(), "the heading is larger than card text: {:?} against {:?}", recent.rect, file.rect);
}

/// The height of the Recent heading's run, with or without `heading_sections`.
fn recent_heading_height(heading_sections: bool) -> f32 {
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")];
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let screen = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files);
        let _ = if heading_sections { screen.heading_sections() } else { screen }.show(ui);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0)).settle();
    fixture.texts().iter().find(|run| run.text == "Recent").map_or(0.0, |run| run.rect.height())
}

/// By default the section titles are upstream's plain section titles; `heading_sections` makes them page headings,
/// larger than the default titles.
#[trial]
fn section_titles_are_plain_by_default_and_headings_when_asked() {
    let (plain, heading) = (recent_heading_height(false), recent_heading_height(true));
    assert!(plain > 0.0, "the Recent title shows by default");
    assert!(heading > plain, "the heading ({heading}) is larger than the plain title ({plain})");
}

/// An app may name the pinned folders section itself (`pinned_title`), in place of the built-in "Pinned Folders".
#[trial]
fn the_pinned_section_takes_the_apps_title() {
    let files = vec![RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")];
    let folders = vec![PinnedFolder::new("/tmp/Work", "Work")];
    let (mut view, mut sort) = (RecentView::List, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let _ = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files).pinned(&folders).pinned_title("Folders").show(ui);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0)).settle();
    assert!(fixture.texts().iter().any(|run| run.text == "Folders"), "the app's title heads the section");
    assert!(!fixture.texts().iter().any(|run| run.text == "Pinned Folders"), "the built-in word is replaced");
}

fn home_screen(has_recent: bool) -> (Fixture, Log) {
    let log: Log = Rc::default();
    let seen = Rc::clone(&log);
    let files: Vec<_> = has_recent.then(|| RecentFile::new("/tmp/work/Poster.psd", "Poster.psd")).into_iter().collect();
    let links = [StartLink::command("Home", None, "window.home").selected(true), StartLink::url("Learn", None, "https://example.test/learn")];
    let footer = [
        StartLink::url("Website", None, "https://example.test"),
        StartLink::url("Discord", None, "https://example.test/chat"),
        StartLink::url("GitHub", None, "https://example.test/code"),
    ];
    let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
    let fixture = Fixture::new(move |ui| {
        let out = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files)
            .links(&links)
            .home_layout("Photo app", &footer)
            .drop_hint("Drop an image")
            .show(ui);
        seen.borrow_mut().extend(out.actions);
    });
    (fixture, log)
}

/// At a desktop width the navigation stays left of the recent files and the community links sit below it.
#[trial]
fn the_home_screen_places_navigation_to_the_left_of_recent_files() {
    let (mut fixture, _) = home_screen(true);
    fixture.room(egui::vec2(1100.0, 700.0)).settle();
    let rect = |label| fixture.by_label(label).expect(label).rect;
    let recent = rect("Recent");
    let new = rect("New…");
    let open = rect("Open…");
    assert!((new.width() - open.width()).abs() < 1.0);
    assert!(open.top() - new.bottom() >= 6.0, "New and Open have room between them");
    assert!(rect("Home").top() - open.bottom() >= 24.0, "the divider separates the actions from navigation");
    assert!(recent.left() - new.right() >= 36.0, "the rail is separated from the content");
    assert!(rect("Photo app").top() >= 54.0);
    assert!(recent.top() - rect("Photo app").bottom() >= 32.0, "the title rule has space above and below");
    for label in ["Home", "Learn", "Website", "Discord", "GitHub"] {
        let navigation = rect(label);
        assert!(navigation.right() < recent.left(), "{label} {navigation:?} must be left of Recent {recent:?}");
    }
    assert!(rect("Home").bottom() < rect("Learn").top());
    assert!(rect("Website").top() > fixture.shown().center().y);
    assert!(rect("Discord").top() - rect("Website").bottom() >= 6.0);
    assert!(rect("GitHub").top() - rect("Discord").bottom() >= 6.0);
}

/// The Home rail's links are rows as wide as the rail, as New and Open are, not inline links; the selected row (Home)
/// shows a bar at its leading edge and the others none.
#[trial]
fn the_home_rail_shows_its_links_as_rows_with_a_bar_on_the_selected_one() {
    use egui::{Shape, vec2};

    let (mut fixture, _) = home_screen(true);
    fixture.room(vec2(1100.0, 700.0)).settle();
    let new = fixture.by_label("New…").expect("New").rect;
    let rows: Vec<_> =
        ["Home", "Learn", "Website", "Discord", "GitHub"].into_iter().map(|label| (label, fixture.by_label(label).expect(label).rect)).collect();
    for (label, row) in &rows {
        assert!((row.width() - new.width()).abs() < 1.0, "{label} {row:?} is a row as wide as New {new:?}");
    }
    let rect = |label: &str| rows.iter().find(|(name, _)| *name == label).map(|(_, row)| *row).expect(label);
    // Upright lines at the rail's leading edge: the selected row's bar.
    let bars: Vec<_> = fixture
        .frame_shapes()
        .into_iter()
        .filter_map(|shape| match shape {
            Shape::LineSegment { points, .. } if (points[0].x - points[1].x).abs() < 0.5 && (points[0].x - new.left()).abs() <= 2.0 => Some(points),
            _ => None,
        })
        .collect();
    let on = |label| {
        let row = rect(label);
        bars.iter().any(|p| row.y_range().contains(p[0].y) && row.y_range().contains(p[1].y))
    };
    assert!(on("Home"), "the selected Home row has a bar at its leading edge: {bars:?}");
    for label in ["Learn", "Website", "Discord", "GitHub"] {
        assert!(!on(label), "{label} is not selected and has no bar: {bars:?}");
    }
}

/// A desktop Recent tile gives its picture and filename room while keeping the whole card clickable.
#[trial]
fn desktop_home_recent_cards_give_the_picture_and_caption_room() {
    use egui::{Shape, pos2, vec2};

    let (mut fixture, log) = home_screen(true);
    fixture.room(vec2(1100.0, 700.0)).settle();
    let card = fixture.by_label("Poster.psd").expect("the recent file shows").rect;
    let shapes = fixture.frame_shapes();
    let picture = shapes
        .iter()
        .find_map(|shape| match shape {
            Shape::Rect(shape)
                if card.contains_rect(shape.rect)
                    && shape.rect.width() > card.width() * 0.8
                    && shape.rect.width() < card.width() - 1.0
                    && shape.rect.height() > 100.0 =>
            {
                Some(shape.rect)
            }
            _ => None,
        })
        .expect("the tile paints an inset picture well");
    let caption = shapes
        .iter()
        .find_map(|shape| match shape {
            Shape::Text(text) if text.galley.text() == "Poster.psd" => Some(text.pos),
            _ => None,
        })
        .expect("the tile paints its filename");

    assert!((390.0..400.0).contains(&card.width()), "{card:?}");
    assert!(card.height() >= 285.0, "there is more vertical room: {card:?}");
    assert!(picture.left() - card.left() >= 8.0, "the picture has horizontal padding: {picture:?}");
    assert!(picture.top() - card.top() >= 8.0, "the picture has top padding: {picture:?}");
    assert!(caption.x - card.left() >= 8.0 && caption.y - picture.bottom() >= 8.0, "the filename has space: {caption:?}");

    log.borrow_mut().clear();
    fixture.click_at(pos2(card.left() + 3.0, card.top() + 3.0)).settle();
    assert_eq!(*log.borrow(), vec![StartAction::OpenRecent(PathBuf::from("/tmp/work/Poster.psd"))]);
}

/// The Home surround can have the same colour as ordinary separators. The title and drop card still need visible ink.
#[trial]
fn the_home_title_and_drop_card_keep_their_outlines_visible() {
    use craft_ui::runtime::env::{Env, RootEnv};
    use craft_ui::testing::Frames;
    use egui::{Color32, Shape, vec2};

    let ppp = 1.25;
    for mode in [craft_theme::Mode::Light, craft_theme::Mode::Dark] {
        let mut frames = Frames::new();
        frames.root(RootEnv::default().mode(mode));
        frames.screen(vec2(1100.0, 700.0));
        frames.pixels_per_point(ppp);
        let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
        let mut canvas = Color32::TRANSPARENT;
        frames.record_paint();
        frames.frame(Vec::new(), |ui| {
            canvas = Color32::from(craft_theme::Srgba::from(Env::of(ui).tokens.color.surface.canvas));
            let _ = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &[])
                .home_layout("Photo app", &[])
                .drop_hint("Drop an image")
                .show(ui);
        });
        let tree = frames.tree();
        let title =
            tree.keys().into_iter().find(|key| tree.name_of(*key) == Some("Photo app")).and_then(|key| tree.bounds_of(key)).expect("the Home title");
        let mut paint = frames.take_paint().expect("the frame drew Home");
        paint.textures_delta.clear();
        let rule = paint
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::LineSegment { points, stroke }
                    if (points[1].x - points[0].x) > 800.0 && (title.bottom()..title.bottom() + 32.0).contains(&points[0].y) =>
                {
                    Some(stroke)
                }
                _ => None,
            })
            .expect("a full-width title rule");
        assert!(rule.color.a() != 0 && rule.color != canvas, "the title rule contrasts with the {mode:?} surround");
        assert!((rule.width * ppp - 1.0).abs() < 0.01);

        let card = paint
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::Rect(rect)
                    if (rect.rect.width() - 540.0).abs() < 1.0 && (rect.rect.height() - 200.0).abs() < 1.0 && !rect.stroke.is_empty() =>
                {
                    Some(rect)
                }
                _ => None,
            })
            .expect("the drop card has an outline");
        assert!(card.stroke.color != canvas, "the drop card contrasts with the {mode:?} surround");
        assert!(card.corner_radius.nw < 12, "the drop card uses control corners");
    }
}

/// A one-point change at the breakpoint switches from a centred page to the reading-start rail.
#[trial]
fn the_home_screen_changes_layout_at_640_points_in_both_directions() {
    for rtl in [false, true] {
        let direction = if rtl { craft_i18n::Direction::Rtl } else { craft_i18n::Direction::Ltr };
        let (mut fixture, _) = home_screen(true);
        fixture.direction(direction).room(egui::vec2(639.0, 700.0)).settle();
        let rect = |label| fixture.by_label(label).expect(label).rect;
        let actions = rect("New…").union(rect("Open…"));
        assert!((rect("New…").center().y - rect("Open…").center().y).abs() < 1.0);
        assert!(actions.width() <= 520.0 && (actions.center().x - 319.5).abs() < 1.0);
        assert!(rect("Home").top() > rect("Recent").top());

        fixture.room(egui::vec2(640.0, 700.0)).settle();
        let rect = |label| fixture.by_label(label).expect(label).rect;
        assert!(rect("New…").bottom() < rect("Open…").top());
        let (nav, content) = (rect("Home"), rect("Recent"));
        assert!(if rtl { nav.left() > content.right() } else { nav.right() < content.left() }, "RTL {rtl}: {nav:?} beside {content:?}");
    }
}

/// A short desktop rail scrolls as a unit without changing the page beside it.
#[trial]
fn a_short_home_rail_scrolls_to_its_community_links() {
    let (mut fixture, _) = home_screen(true);
    fixture.room(egui::vec2(1100.0, 120.0)).settle();
    let recent = fixture.by_label("Recent").expect("the page is visible").rect;
    let opening = fixture.by_label("New…").expect("the rail's first button");
    assert!(fixture.shown().contains(opening.center()));
    assert!(!fixture.by_label("GitHub").is_some_and(|link| fixture.shown().contains(link.center())));

    fixture.frame(vec![moved(opening.center())]).frame(vec![craft_ui::testing::pointer::wheel(-400.0)]).settle();
    let link = fixture.by_label("GitHub").expect("the end of the rail can be scrolled into view");
    assert!(fixture.shown().contains(link.center()), "the link sits inside the short rail: {:?}", link.rect);
    assert_eq!(fixture.by_label("Recent").expect("the page stays visible").rect, recent);
}

/// The empty card has its own pointer and keyboard activation and the blank page opens only on a double-click.
#[trial]
fn the_home_drop_card_is_bounded_and_opens_by_keyboard_or_pointer() {
    use egui::{Key, Modifiers};

    let (mut fixture, log) = home_screen(false);
    fixture.room(egui::vec2(1100.0, 700.0)).settle();
    let card = fixture.by_label("Drop an image").expect("an empty page has a drop target");
    assert!((card.rect.width() - 540.0).abs() < 1.0 && (card.rect.height() - 200.0).abs() < 1.0, "{:?}", card.rect);
    assert!((card.center().y - fixture.shown().center().y).abs() < 24.0, "{:?}", card.rect);
    fixture.click(&card).settle();
    assert_eq!(*log.borrow(), vec![StartAction::Open]);

    log.borrow_mut().clear();
    let corner = fixture.shown().max - egui::vec2(30.0, 30.0);
    fixture.click_at(corner).click_at(corner).settle();
    assert_eq!(*log.borrow(), vec![StartAction::Open], "blank room opens once");

    let (mut fixture, log) = home_screen(false);
    fixture.room(egui::vec2(1100.0, 700.0)).settle();
    let card = fixture.by_label("Drop an image").expect("a separate keyboard target");
    let focused = |fixture: &Fixture| fixture.focused().and_then(|id| fixture.node_of(id)).is_some_and(|target| target.node == card.node);
    for _ in 0..10 {
        if focused(&fixture) {
            break;
        }
        fixture.key(Key::Tab, Modifiers::NONE).settle();
    }
    assert!(focused(&fixture), "Tab reaches the drop target");
    fixture.key(Key::Enter, Modifiers::NONE).settle();
    assert_eq!(*log.borrow(), vec![StartAction::Open]);
}

/// An empty recent list on a page says so in a line at the reading start under its heading, where its rows would
/// begin, not in the start screen's centred empty state (a File page's Open list).
#[trial]
fn an_empty_recent_list_says_so_at_the_reading_start() {
    let files: Vec<RecentFile> = Vec::new();
    let mut fixture = Fixture::new(move |ui| {
        let _ = RecentList::new(&files).empty("Nothing opened yet").show(ui);
    });
    fixture.room(egui::Vec2::new(700.0, 500.0));
    fixture.settle();
    let line = fixture.by_label("Nothing opened yet").expect("the empty line shows");
    assert!(line.rect.left() < 60.0, "the line starts at the list's start edge: {:?}", line.rect);
    assert!(line.rect.width() < 300.0, "the line is as wide as its text: {:?}", line.rect);
}

/// The template cards of a greeting page, as `(width, height)` of the card named "Memo", for the knobs `shape` sets.
fn memo_card(shape: impl for<'a> Fn(StartScreen<'a>) -> StartScreen<'a> + 'static) -> egui::Vec2 {
    let templates = vec![Template::new("memo", "Memo", ""), Template::new("report", "Report", "")];
    let (mut view, mut sort) = (RecentView::List, RecentSort::Opened);
    let mut fixture = Fixture::new(move |ui| {
        let _ =
            shape(StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &[]).layout(StartLayout::Greeting).templates(&templates)).show(ui);
    });
    fixture.room(egui::Vec2::new(1100.0, 700.0));
    fixture.settle();
    fixture.by_label("Memo").expect("the card shows").rect.size()
}

/// An app can give its card pictures a shape and a fixed width: by default the cards are 16:9 and grow to fill the row,
/// with `picture_aspect(3, 4)` and `card_zoom(1.0)` they are portrait and stay narrow.
#[trial]
fn a_start_screen_takes_the_shape_and_width_of_its_card_pictures() {
    let default = memo_card(|screen| screen);
    assert!(default.x > default.y * 1.3, "the default card is a wide 16:9 card: {default:?}");
    let page = memo_card(|screen| screen.picture_aspect(3, 4).card_zoom(1.0));
    assert!(page.y > page.x, "a 3:4 card is taller than wide: {page:?}");
    assert!(page.x < default.x * 0.7, "a card at zoom 1 does not grow to fill the row: {page:?}");
}

/// The Column page of `files` in a 1600 by 900 room: title, subtitle, New and Open, the recent rows, a page template
/// and the links, the first a call to action. Returns the fixture and the column's widest width.
fn column_page(files: Vec<RecentFile>) -> (Fixture, f32) {
    column_page_in(files, RootEnv::default())
}

/// `column_page` under the app's `root`.
fn column_page_in(files: Vec<RecentFile>, root: RootEnv) -> (Fixture, f32) {
    let column = Rc::new(RefCell::new(0.0));
    let widest = column.clone();
    let templates = vec![Template::new("letter", "Letter", "612 × 792 pt").page(612.0, 792.0)];
    let links = vec![
        StartLink::url("Join the community", Some(craft_behavior::IconId::new("globe")), "https://example.com/chat").primary(),
        StartLink::url("Website", Some(craft_behavior::IconId::new("globe")), "https://example.com"),
        StartLink::url("Source", None, "https://example.com/source"),
    ];
    let mut fixture = Fixture::new(move |ui| {
        *widest.borrow_mut() = Env::of(ui).tokens.size.page.column;
        let (mut view, mut sort) = (RecentView::Grid, RecentSort::Opened);
        let _ = StartScreen::new(Bind::bound(&mut view), Bind::bound(&mut sort), &files)
            .layout(StartLayout::Column)
            .title("Welcome")
            .subtitle("Make something")
            .templates(&templates)
            .links(&links)
            .links_title("Community")
            .drop_hint("Drop files here")
            .show(ui);
    });
    fixture.root(root).room(egui::Vec2::new(1600.0, 900.0)).settle();
    let column = *column.borrow();
    (fixture, column)
}

/// A Column page stands centred in a wide room, no wider than its column, with its parts in the page's order: the
/// title over the subtitle, New and Open, Recent, the templates, then the links.
#[trial]
fn a_column_page_stands_centred_with_its_parts_in_order() {
    let (fixture, column) = column_page(vec![RecentFile::new("/tmp/Report.vc", "Report.vc")]);
    let find = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} shows"));
    let order = ["Welcome", "Make something", "New…", "Report.vc, /tmp", "Letter, 612 × 792 pt", "Community", "Join the community", "Website"];
    let parts: Vec<_> = order.iter().map(|label| find(label)).collect();
    for (pair, labels) in parts.windows(2).zip(order.windows(2)) {
        assert!(pair[0].rect.top() < pair[1].rect.top(), "{} stands above {}: {:?} {:?}", labels[0], labels[1], pair[0].rect, pair[1].rect);
    }
    let middle = 800.0;
    for label in ["Welcome", "Make something", "Community", "Join the community", "Website"] {
        let centre = find(label).rect.center().x;
        assert!((centre - middle).abs() < 2.0, "{label} is centred: {centre}");
    }
    let (new, open) = (find("New…").rect, find("Open…").rect);
    assert!(((new.left() + open.right()) / 2.0 - middle).abs() < 2.0, "New and Open stand centred together: {new:?} {open:?}");
    let (row, card) = (find("Report.vc, /tmp").rect, find("Letter, 612 × 792 pt").rect);
    assert!(row.left() >= middle - column / 2.0 - 1.0 && row.right() <= middle + column / 2.0 + 1.0, "the rows keep to the column: {row:?}");
    assert!(card.left() >= row.left() - 1.0, "the cards start inside the column: {card:?} {row:?}");
}

/// The Column page shows Recent only when there are recent files: no section, no drop card and no view switch
/// without them.
#[trial]
fn a_column_page_shows_recent_only_with_files() {
    let (with, _) = column_page(vec![RecentFile::new("/tmp/Report.vc", "Report.vc")]);
    assert!(with.by_label("Recent").is_some(), "Recent shows with a file");
    assert!(with.by_label("Grid").is_none(), "the column has no view switch");
    let (without, _) = column_page(Vec::new());
    assert!(without.by_label("Recent").is_none(), "no Recent section without files");
    assert!(without.by_label("Drop files here").is_none(), "no drop card on a column page");
    assert!(without.by_label("Letter, 612 × 792 pt").is_some(), "the templates still show");
}

/// The call to action is a pill button taller than the plain links, and a click on it opens its URL; the other links
/// stay links.
#[trial]
fn a_column_call_to_action_is_a_pill_that_opens_its_url() {
    let (mut fixture, _) = column_page(Vec::new());
    let (pill, link) = (fixture.by_label("Join the community").expect("the pill"), fixture.by_label("Website").expect("the link"));
    assert!(fixture.by_role(Role::BUTTON).iter().any(|button| button.node == pill.node), "the call to action is a button");
    assert!(fixture.by_role(Role::LINK).iter().any(|plain| plain.node == link.node), "the other links are links");
    assert!(pill.rect.height() > link.rect.height(), "the pill stands taller than a link: {:?} {:?}", pill.rect, link.rect);
    fixture.click(&pill).idle(2);
    assert!(fixture.opened().iter().any(|(url, _)| url == "https://example.com/chat"), "the pill opens its URL: {:?}", fixture.opened());
}

/// A Column page takes its room above the column and its call to action's height from the page sizes, which an app
/// sets (VectorCraft's 60-point margin and 36-point pill).
#[trial]
fn a_column_page_follows_its_page_sizes() {
    let shown = |margin: f32, action: f32| {
        let mut sizes = AppSizes::default();
        sizes.all.insert("size.page.margin".to_owned(), SizeValue::Number(margin));
        sizes.all.insert("size.page.action".to_owned(), SizeValue::Number(action));
        let mut root = RootEnv::default();
        root.sizes = Arc::new(sizes);
        let (fixture, _) = column_page_in(Vec::new(), root);
        let (title, pill) = (fixture.by_label("Welcome").expect("the title"), fixture.by_label("Join the community").expect("the pill"));
        assert!(title.rect.top() >= margin, "the title stands the margin {margin} below the page's top: {:?}", title.rect);
        assert!(pill.rect.height() >= action - 0.5, "the pill is {action} high: {:?}", pill.rect);
        (title.rect.top(), pill.rect.height())
    };
    let (low, high) = (shown(20.0, 24.0), shown(90.0, 48.0));
    assert!(high.0 > low.0 && high.1 > low.1, "a larger margin lowers the column and a larger action height raises the pill: {low:?} {high:?}");
}

/// A Column page takes its rhythm from the page sizes, which an app sets (VectorCraft's roomier Home): the subtitle
/// under the title, New and Open under them and beside each other, the sections, a heading over its body and the links
/// each stand further apart, and New and Open stand taller and wider.
#[trial]
fn a_column_page_takes_its_rhythm_from_the_page_sizes() {
    let shown = |roomy: bool| {
        let mut sizes = AppSizes::default();
        let page = [
            ("size.page.gap.title", 16.0),
            ("size.page.gap.lead", 40.0),
            ("size.page.gap.actions", 30.0),
            ("size.page.gap.section", 48.0),
            ("size.page.gap.heading", 24.0),
            ("size.page.gap.link", 20.0),
            ("size.page.link", 36.0),
            ("size.page.button", 40.0),
            ("size.page.button_side", 30.0),
        ];
        for (path, value) in page.into_iter().filter(|_| roomy) {
            sizes.all.insert(path.to_owned(), SizeValue::Number(value));
        }
        let mut root = RootEnv::default();
        root.sizes = Arc::new(sizes);
        let (fixture, _) = column_page_in(vec![RecentFile::new("/tmp/Report.vc", "Report.vc")], root);
        let rect = |label: &str| fixture.by_label(label).unwrap_or_else(|| panic!("{label} shows")).rect;
        let (title, subtitle, new, open) = (rect("Welcome"), rect("Make something"), rect("New…"), rect("Open…"));
        let (recent, community, pill, website, source) =
            (rect("Recent"), rect("Community"), rect("Join the community"), rect("Website"), rect("Source"));
        [
            ("title to subtitle", subtitle.top() - title.bottom()),
            ("subtitle to New", new.top() - subtitle.bottom()),
            ("New to Open", open.left() - new.right()),
            ("New's height", new.height()),
            ("New's width", new.width()),
            ("New to Recent", recent.top() - new.bottom()),
            ("Community to the pill", pill.top() - community.bottom()),
            ("pill to link", website.top() - pill.bottom()),
            ("link to link", source.top() - website.bottom()),
        ]
    };
    for ((what, standard), (_, roomy)) in shown(false).into_iter().zip(shown(true)) {
        assert!(roomy > standard + 1.0, "{what} grows with the page sizes: {standard} → {roomy}");
    }
}
