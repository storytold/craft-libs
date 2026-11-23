//! `StartScreen`: which part of the page takes a double click, and what a recent file's menu offers.

use craft_ui::contract::{Bind, Role};
use craft_ui::shell::{RecentFile, RecentList, RecentSort, RecentView, StartAction, StartLayout, StartLink, StartScreen, Template};
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
