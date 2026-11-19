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
    assert!(rect("Photo app").bottom() < recent.top());
    for label in ["Home", "Learn", "Website", "Discord", "GitHub"] {
        let navigation = rect(label);
        assert!(navigation.right() < recent.left(), "{label} {navigation:?} must be left of Recent {recent:?}");
    }
    assert!(rect("Home").bottom() < rect("Learn").top());
    assert!(rect("Website").top() > fixture.shown().center().y);
    assert!(rect("Website").bottom() < rect("Discord").top());
    assert!(rect("Discord").bottom() < rect("GitHub").top());
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
        let mut paint = frames.take_paint().expect("the frame drew Home");
        paint.textures_delta.clear();
        let rule = paint
            .shapes
            .iter()
            .find_map(|shape| match &shape.shape {
                Shape::LineSegment { points, stroke } if (points[1].x - points[0].x) > 800.0 && points[0].y < 100.0 => Some(stroke),
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
