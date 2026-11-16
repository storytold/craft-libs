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

/// The `Grid` runs and the cards run (their `Open` buttons) of a settled Overview pass in a view as tall as `height`.
fn grid_runs(level: &str, filter: &str, height: f32) -> (u32, usize) {
    let mut gallery = gallery_filtered(level, filter);
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, height)).settle().idle(1);
    let grids = last_pass_runs(fixture.ctx()).iter().filter(|(ty, _)| ty.ends_with("::Grid")).map(|(_, runs)| runs).sum();
    let opens = fixture.by_role(Role::BUTTON).into_iter().filter(|button| fixture.tree().name_of(button.node) == Some("Open")).count();
    (grids, opens)
}

/// The cards of `level` that `filter` lists, in the Overview's order.
fn listed(level: &str, filter: &str) -> Vec<String> {
    let catalog = Catalog::linked();
    by_level(&catalog, filter)
        .into_iter()
        .find(|(at, _)| at.title() == level)
        .map_or_else(Vec::new, |(_, cards)| cards.into_iter().map(|component| component.name.clone()).collect())
}

/// A row of the Overview out of view does not run: a settled pass of a short view runs as many `Grid`s for the whole
/// Controls level as for a filter that lists the same cards in view and fewer rows after them.
#[trial]
fn a_settled_overview_pass_runs_no_row_out_of_view() {
    const HEIGHT: f32 = 220.0;
    let level = listed("Controls", "");
    let (all, in_view) = grid_runs("Controls", "", HEIGHT);
    assert!((1..level.len()).contains(&in_view), "the view shows some of the level's {} cards, not {in_view}", level.len());
    // The filter: the part of the first card's name that every card in view matches too, and that lists the fewest
    // cards. The level is in name order, so the filter lists the cards in view first.
    let first = level[0].to_lowercase();
    let filter = (0..first.len())
        .flat_map(|start| (start + 2..=first.len()).map(move |end| (start, end)))
        .filter_map(|(start, end)| first.get(start..end))
        .map(|part| (part, listed("Controls", part)))
        .filter(|(_, cards)| cards.len() >= in_view && cards[..in_view] == level[..in_view])
        .min_by_key(|(_, cards)| cards.len())
        .map(|(part, _)| part.to_owned())
        .unwrap_or_else(|| panic!("a part of {first:?} lists the {in_view} cards in view first"));
    let fewer = listed("Controls", &filter).len();
    assert!(fewer < level.len(), "the filter {filter:?} lists fewer cards than the level: {fewer} of {}", level.len());
    let (some, some_in_view) = grid_runs("Controls", &filter, HEIGHT);
    assert_eq!(in_view, some_in_view, "the level and the filter {filter:?} run the same cards");
    assert_eq!(all, some, "the Grid runs grow with the rows out of view: {some} with {fewer} cards, {all} with {}", level.len());
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

/// A settled card repaints its last run: a settled frame of the Slider cards runs no Slider and no card chrome, and still
/// shows the Maximum story's value text.
#[trial]
fn a_settled_overview_card_runs_no_component() {
    let mut gallery = gallery_filtered("Controls", "Slider");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    for frame in 0..4 {
        fixture.idle(1);
        let runs = last_pass_runs(fixture.ctx());
        // Type names carry their generics: the path before them names the component.
        let named = |ty: &str, name: &str| ty.split('<').next().is_some_and(|path| path.ends_with(name));
        let card: Vec<_> = runs.iter().filter(|(ty, _)| ["::Slider", "::Panel", "::TitleRow"].iter().any(|name| named(ty, name))).collect();
        assert!(card.is_empty(), "settled frame {frame} ran the card: {card:?}");
        assert!(painted(&fixture, "100.0"), "settled frame {frame} shows the Maximum story's value text");
    }
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
    settings.level = level_of("FindReplaceDialog").to_owned();
    settings.filter = "FindReplaceDialog".to_owned();
    store.set(settings);
    let mut gallery = Gallery::new(Catalog::linked(), Persisted::new(store, &report));
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    // The body shows only while the dialog is open.
    let (closed, inline) = (openers(&fixture), painted_times(&fixture, "Find what:"));
    assert!(closed >= 1, "the picked story shows its opener");
    // The card's opener: the first under the card's title, in its column.
    let title = fixture.texts().iter().find(|run| run.text == "FindReplaceDialog").expect("the card's title").rect;
    let opener = fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .filter(|button| fixture.tree().name_of(button.node) == Some("Show dialog"))
        .filter(|button| button.rect.top() > title.top() && button.rect.left() < title.right() && button.rect.right() > title.left() - 40.0)
        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .expect("the card's opener");
    fixture.click(&opener).settle();
    assert_eq!(painted_times(&fixture, "Find what:"), inline + 1, "the opener opens the dialog");
    assert_eq!(openers(&fixture), closed - 1, "the opener gives way to the story");
    fixture.key(egui::Key::Escape, egui::Modifiers::NONE).settle();
    // The exit fade runs on the fixture's clock.
    fixture.advance(1.0).settle();
    assert_eq!(painted_times(&fixture, "Find what:"), inline, "the dialog closed");
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

/// A story's readout shows under the story and follows what happens in it: the LayersPanel card says that nothing
/// happened yet, and after a click on a layer row it names the event.
#[trial]
fn a_card_shows_its_storys_readout_under_the_story_and_follows_it() {
    let mut gallery = gallery_filtered(level_of("LayersPanel"), "LayersPanel");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    let readout = |fixture: &Fixture| {
        fixture.texts().iter().find(|run| run.text == "No event yet" || run.text.starts_with("Last event: ")).map(|run| (run.text.clone(), run.rect))
    };
    let (before, at) = readout(&fixture).expect("the card shows its story's readout");
    assert_eq!(before, "No event yet");
    let card = fixture
        .by_role(Role::GROUP)
        .into_iter()
        .find(|group| fixture.tree().name_of(group.node) == Some("LayersPanel"))
        .expect("the LayersPanel card shows")
        .rect;
    let rows: Vec<_> = [Role::TREE_ITEM, Role::LIST_ITEM, Role::ROW]
        .into_iter()
        .flat_map(|role| fixture.by_role(role))
        .filter(|row| card.contains_rect(row.rect))
        .collect();
    let row = *rows.last().expect("the panel shows layer rows");
    assert!(rows.iter().all(|row| row.rect.bottom() <= at.top()), "the readout lies under the story: {at:?}");
    fixture.click(&row).settle();
    let (after, _) = readout(&fixture).expect("the readout stays");
    assert!(after.starts_with("Last event: "), "the readout names the click's event: {after}");
}

/// The height of the Overview card of the component listed as `name` in a window `height` tall.
fn card_height(name: &str, height: f32) -> f32 {
    let mut gallery = gallery_filtered(level_of(name), name);
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, height)).settle();
    let card = fixture
        .by_role(Role::GROUP)
        .into_iter()
        .find(|group| fixture.tree().name_of(group.node) == Some(name))
        .unwrap_or_else(|| panic!("{name}'s card shows"));
    card.rect.height()
}

/// A card whose story fills and scrolls is as tall as the story's content while the content fits the room, as the old
/// gallery's tiles were: a taller window gives the card no empty space. Regression: a list or panel story took the
/// whole height of the view.
#[trial]
fn a_card_of_a_filling_story_is_as_tall_as_its_content() {
    let (short, tall) = (card_height("LayersPanel", 900.0), card_height("LayersPanel", 1400.0));
    assert!(short < 900.0, "the card fits the window: {short}");
    assert!((tall - short).abs() < 1.0, "the card keeps its content's height in a taller window: {short} -> {tall}");
}
