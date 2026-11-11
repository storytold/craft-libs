//! The Overview: the cards of a level, live.

use craft_settings::{Location, OpenOptions, SettingsStore};
use craft_ui::runtime::access::Role;
use craft_ui::shell::app::ShellApp;
use craft_ui::testing::{Fixture, pointer, trial};
use craft_ui_gallery::chrome::{Gallery, Persisted};
use craft_ui_gallery::viewer::{Catalog, GallerySettings};
use egui::Vec2;

fn gallery_at(level: &str) -> Gallery {
    gallery_filtered(level, "")
}

fn gallery_filtered(level: &str, filter: &str) -> Gallery {
    let (mut store, report) = SettingsStore::<GallerySettings>::open_with(OpenOptions::new(Location::Memory));
    let mut settings = store.get().clone();
    settings.level = level.to_owned();
    settings.filter = filter.to_owned();
    store.set(settings);
    Gallery::new(Catalog::linked(), Persisted::new(store, &report))
}

/// Every level opens in the Overview and runs all its cards.
#[trial]
fn every_level_of_the_overview_runs_its_cards() {
    for level in ["Primitives", "Controls", "Composites", "Patterns", "Shells"] {
        let mut gallery = gallery_at(level);
        let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
        fixture.room(Vec2::new(1400.0, 900.0)).settle();
        assert!(fixture.by_label("Open").is_some(), "{level} has cards");
    }
}

/// The registered type name of the component listed as `name`.
fn type_of(name: &str) -> &'static str {
    Catalog::linked().components.iter().find(|component| component.name == name).unwrap_or_else(|| panic!("{name} is listed")).type_name
}

fn painted(fixture: &Fixture, text: &str) -> bool {
    fixture.texts().iter().any(|run| run.text == text)
}

fn openers(fixture: &Fixture) -> usize {
    fixture.texts().iter().filter(|run| run.text == "Show dialog").count()
}

/// A dialog in an embedded shell of the story's own is out of the story's measure; the card still starts it closed
/// before anything of it paints, opens it by the button, and shows the button again once the story closed both.
#[trial]
fn dialogs_in_a_storys_own_shell_start_closed_and_close_back_to_the_opener() {
    let (mut store, report) = SettingsStore::<GallerySettings>::open_with(OpenOptions::new(Location::Memory));
    let mut settings = store.get().clone();
    settings.level = "Shells".to_owned();
    settings.filter = "AppShell".to_owned();
    settings.stories.insert(type_of("AppShell").to_owned(), "app_shell_dialogs_in_regions".to_owned());
    store.set(settings);
    let mut gallery = Gallery::new(Catalog::linked(), Persisted::new(store, &report));
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).idle(1);
    let over = fixture.by_label("Open").expect("the card's Open").rect.center();
    let high = fixture.shown().height() / 5.0;
    assert_eq!(openers(&fixture), 1, "the picked story shows its opener");
    assert!(!painted(&fixture, "Snap to grid"), "nothing of the dialogs paints before the opener is pressed");
    let opener = fixture.by_label("Show dialog").expect("the opener");
    fixture.click(&opener).settle();
    assert!(painted(&fixture, "Snap to grid"), "the opener opens the dialogs");
    assert_eq!(openers(&fixture), 0, "the opener gives way to the story");
    // The story now takes a view's height under the opener's place: the wheel brings its dialogs up into view.
    for _ in 0..60 {
        if fixture.by_label("Close").is_some_and(|close| close.rect.top() < 2.0 * high) {
            break;
        }
        fixture.frame(vec![pointer::moved(over)]).frame(vec![pointer::wheel(-120.0)]).idle(1);
    }
    for _ in 0..2 {
        let close = fixture.by_label("Close").expect("a dialog's close button");
        fixture.click(&close).settle();
        // The exit fade runs on the fixture's clock.
        fixture.advance(1.0).settle();
    }
    assert!(!painted(&fixture, "Snap to grid"), "both dialogs closed");
    assert_eq!(openers(&fixture), 1, "closing the story's dialogs brings the opener back");
}

/// Regression: the wheel over a canvas card went to its story and the Overview stopped scrolling; inside a
/// card the wheel takers take the wheel only while focused or armed, so it scrolls the Overview.
#[trial]
fn the_wheel_over_a_card_scrolls_the_overview() {
    // The canvas overlays are Primitives.
    let mut gallery = gallery_at("Primitives");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    let visible = fixture.shown();
    let story = fixture
        .by_role(Role::CANVAS)
        .into_iter()
        .find(|t| visible.contains_rect(t.rect) && t.rect.height() > 40.0)
        .expect("a canvas card is in view");
    let title = fixture.by_label("Open").expect("a card header has Open").rect.min.y;
    fixture.frame(vec![pointer::moved(story.center())]).frame(vec![pointer::wheel(-120.0)]).settle();
    let after = fixture.by_label("Open").expect("a card header has Open").rect.min.y;
    assert!(after < title, "the wheel over a canvas card scrolled the Overview: {title} -> {after}");
}
