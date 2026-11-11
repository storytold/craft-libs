//! The Overview: the cards of a level, live.

use craft_settings::{Location, OpenOptions, SettingsStore};
use craft_ui::runtime::access::Role;
use craft_ui::shell::app::ShellApp;
use craft_ui::testing::{Fixture, last_pass_measuring_runs, last_pass_runs, pointer, trial};
use craft_ui_gallery::chrome::{Gallery, Persisted};
use craft_ui_gallery::viewer::{Catalog, GallerySettings, by_level};
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

/// Every level opens in the Overview and its first show runs the cards in view: their headers are in the access
/// tree, and no card below the window ran. Guards that a card no pass showed yet is placed by its estimate.
#[trial]
fn each_level_of_the_overview_first_runs_the_cards_in_view() {
    for level in ["Primitives", "Controls", "Composites", "Patterns", "Shells"] {
        let mut gallery = gallery_at(level);
        let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
        fixture.room(Vec2::new(1400.0, 900.0)).settle();
        let window = fixture.shown();
        let opens: Vec<_> = fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Open")).collect();
        assert!(!opens.is_empty(), "{level} runs the cards in view");
        for open in &opens {
            assert!(open.rect.top() < window.bottom(), "{level} ran a card below the window: its Open at {:?}", open.rect);
        }
        let listed = by_level(&Catalog::linked(), "").into_iter().find(|(at, _)| at.title() == level).map_or(0, |(_, cards)| cards.len());
        if level == "Controls" {
            assert!(opens.len() < listed, "{level} ran fewer cards than it lists: {} of {listed}", opens.len());
        }
    }
}

/// A settled Overview pass runs each story once: no card or story runs a measuring run while its inputs hold.
#[trial]
fn a_settled_overview_pass_runs_no_measuring_run() {
    let mut gallery = gallery_at("Composites");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(1);
    let runs = last_pass_measuring_runs(fixture.ctx());
    assert!(runs.is_empty(), "a settled pass measured {} bodies: {runs:?}", runs.len());
}

/// The `ShellRegion` runs of a settled Overview pass: the regions of the window's and the view's shells.
fn shell_runs(level: &str, filter: &str) -> u32 {
    let mut gallery = gallery_filtered(level, filter);
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(1);
    last_pass_runs(fixture.ctx()).iter().filter(|(ty, _)| ty.ends_with("::ShellRegion")).map(|(_, runs)| runs).sum()
}

/// A story runs in the view's shell, not in one of its own: a settled Overview pass runs as many shell regions with
/// every card of a level in view as with one card.
#[trial]
fn a_settled_overview_pass_runs_no_shell_per_story() {
    let (all, one) = (shell_runs("Controls", ""), shell_runs("Controls", "Slider"));
    assert!(all > 0, "the window's shell runs its regions");
    assert_eq!(all, one, "the shell regions grow with the stories in view: {one} with the Slider cards, {all} with the level");
}

/// A card in form rows shows each story's value text beside it, as the story's run found it: a settled frame of the
/// Slider card, whose bodies take their kept measurements in place of running the stories, runs one pass.
#[trial]
fn a_settled_form_rows_card_runs_one_pass_per_frame() {
    let mut gallery = gallery_filtered("Controls", "Slider");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    for frame in 0..4 {
        fixture.idle(1);
        assert_eq!(fixture.passes(), 1, "settled frame {frame} ran {} passes", fixture.passes());
        assert!(painted(&fixture, "100.0"), "settled frame {frame} shows the Maximum story's value text");
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
    painted_times(fixture, "Show dialog")
}

fn painted_times(fixture: &Fixture, text: &str) -> usize {
    fixture.texts().iter().filter(|run| run.text == text).count()
}

/// The level the component listed as `name` is on.
fn level_of(name: &str) -> &'static str {
    let catalog = Catalog::linked();
    by_level(&catalog, "")
        .into_iter()
        .find(|(_, cards)| cards.iter().any(|component| component.name == name))
        .map(|(level, _)| level.title())
        .unwrap_or_else(|| panic!("{name} is on a level"))
}

/// A story's dialog opens over the view, in the view's shell: the card starts it closed before anything of it paints,
/// opens it by the button, and shows the button again once the dialog closed.
#[trial]
fn a_storys_dialog_starts_closed_opens_over_the_view_and_closes_back_to_the_opener() {
    let (mut store, report) = SettingsStore::<GallerySettings>::open_with(OpenOptions::new(Location::Memory));
    let mut settings = store.get().clone();
    settings.level = level_of("Dialog").to_owned();
    settings.filter = "Dialog".to_owned();
    settings.stories.insert(type_of("Dialog").to_owned(), "dialog".to_owned());
    store.set(settings);
    let mut gallery = Gallery::new(Catalog::linked(), Persisted::new(store, &report));
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    // Other cards of the filter show the same body in flow.
    let (closed, inline) = (openers(&fixture), painted_times(&fixture, "Snap to grid"));
    assert!(closed >= 1, "the picked story shows its opener");
    // The Dialog card's opener: the first under the card's title, in its column.
    let title = fixture.texts().into_iter().find(|run| run.text == "Dialog").expect("the Dialog card's title").rect;
    let opener = fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .filter(|button| fixture.tree().name_of(button.node) == Some("Show dialog"))
        .filter(|button| button.rect.top() > title.top() && button.rect.left() < title.right() && button.rect.right() > title.left() - 40.0)
        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .expect("the Dialog card's opener");
    fixture.click(&opener).settle();
    assert_eq!(painted_times(&fixture, "Snap to grid"), inline + 1, "the opener opens the dialog");
    assert_eq!(openers(&fixture), closed - 1, "the opener gives way to the story");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    // The exit fade runs on the fixture's clock.
    fixture.advance(1.0).settle();
    assert_eq!(painted_times(&fixture, "Snap to grid"), inline, "the dialog closed");
    assert_eq!(openers(&fixture), closed, "closing the story's dialog brings the opener back");
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

/// A selection of a component whose card is far down another level, here the one the gallery opens with, shows that
/// level and glides its card to the top of the view over several frames, as the old gallery's jump did. Regression:
/// the request went to egui's scroll target, which no scroll body reads, so the card never moved.
#[trial]
fn selecting_a_component_glides_its_card_to_the_top() {
    const GLIDE_FRAMES: usize = 120;
    let catalog = Catalog::linked();
    let controls = by_level(&catalog, "").into_iter().find(|(level, _)| level.title() == "Controls").expect("Controls has cards").1;
    // Far enough down to be out of view, with enough cards after it to scroll it to the top.
    let target = controls[controls.len() / 2];
    let (mut store, report) = SettingsStore::<GallerySettings>::open_with(OpenOptions::new(Location::Memory));
    let mut settings = store.get().clone();
    settings.level = "Primitives".to_owned();
    settings.selected = target.type_name.to_owned();
    store.set(settings);
    let mut gallery = Gallery::new(Catalog::linked(), Persisted::new(store, &report));
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0));
    let mut frames = 0;
    let arrived = loop {
        fixture.frame(Vec::new());
        frames += 1;
        let window = fixture.shown();
        let top = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .filter(|button| fixture.tree().name_of(button.node) == Some("Open") && window.contains_rect(button.rect))
            .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()));
        // The target's card heads the view: its title sits in the band of the topmost card's Open.
        let heads = top.is_some_and(|open| {
            fixture.texts().iter().any(|run| run.text == target.name && (run.rect.center().y - open.rect.center().y).abs() < open.rect.height())
        });
        if heads || frames == GLIDE_FRAMES {
            break heads;
        }
    };
    assert!(arrived, "{}'s card reached the top of the view", target.name);
    assert!(frames > 2, "the card glided over several frames, not in one: {frames}");
}

/// A card shows Reset only once its story's values differ from its start: at rest, with no input, no card shows it.
/// Regression: the start was noted on a pass where the story had not run (a kept measurement, a held-back story),
/// so every value the story then made counted as a change, on most cards of a level.
#[trial]
fn a_card_at_rest_shows_no_reset() {
    for name in ["ToggleButton", "TipEllipse", "CompareTile", "CurvesPanel", "Menu"] {
        let mut gallery = gallery_filtered(level_of(name), name);
        let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
        fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(3).settle();
        assert!(fixture.by_label("Open").is_some(), "{name}'s card shows");
        assert!(fixture.by_label("Reset").is_none(), "{name}'s card shows Reset with no input");
    }
}
