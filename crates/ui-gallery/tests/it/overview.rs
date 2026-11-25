//! The Overview: the cards of a level, live.

use craft_settings::{Location, OpenOptions, SettingsStore};
use craft_ui::runtime::access::Role;
use craft_ui::shell::app::ShellApp;
use craft_ui::testing::{Fixture, MemoRun, last_frame_memo_runs, last_pass_measuring_runs, last_pass_runs, pointer, trial};
use craft_ui_gallery::chrome::{Gallery, Persisted, region_key};
use craft_ui_gallery::viewer::{Catalog, GallerySettings, PanelId, by_level};
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
/// every card of a level in view as with the Button cards alone. The Button card animates, so the Overview runs in both
/// passes rather than replaying.
#[trial]
fn a_settled_overview_pass_runs_no_shell_per_story() {
    let (all, few) = (shell_runs("Controls", ""), shell_runs("Controls", "Button"));
    assert!(all > 0, "the window's shell runs its regions");
    assert_eq!(all, few, "the shell regions grow with the stories in view: {few} with the Button cards, {all} with the level");
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

/// A dialog story closed by its own button opens again by the card's opener: the card finds the story's `open` value
/// by name. Regression: the PDF export story kept `open` inside another value, so the opener showed an empty card.
#[trial]
fn a_storys_dialog_closed_by_its_own_button_opens_again_by_the_opener() {
    let mut gallery = gallery_filtered(level_of("PdfExportDialog"), "PdfExportDialog");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    let closed = painted_times(&fixture, "Save PDF");
    let open_by_the_opener = |fixture: &mut Fixture| {
        let opener = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .filter(|button| fixture.tree().name_of(button.node) == Some("Show dialog"))
            .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
            .expect("an opener");
        fixture.click(&opener).settle();
    };
    open_by_the_opener(&mut fixture);
    assert!(painted_times(&fixture, "Save PDF") > closed, "the opener opens the dialog");
    let cancel = fixture.by_label("Cancel").expect("the dialog's Cancel button");
    fixture.click(&cancel).settle();
    // The exit fade runs on the fixture's clock.
    fixture.advance(1.0).settle();
    assert_eq!(painted_times(&fixture, "Save PDF"), closed, "Cancel closed the dialog");
    open_by_the_opener(&mut fixture);
    assert!(painted_times(&fixture, "Save PDF") > closed, "the opener opens the dialog again");
}

/// A story whose dialog is open runs in every frame: the dialog paints on a layer of its own, which a replay can't add,
/// so the shell counts the dialog in every pass and the card keeps the story in place of its opener.
#[trial]
fn a_story_with_its_dialog_open_runs_every_frame_and_keeps_the_dialog_counted() {
    let mut gallery = gallery_filtered(level_of("FindReplaceDialog"), "FindReplaceDialog");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    let closed = openers(&fixture);
    let opener = fixture
        .by_role(Role::BUTTON)
        .into_iter()
        .filter(|button| fixture.tree().name_of(button.node) == Some("Show dialog"))
        .min_by(|a, b| a.rect.top().total_cmp(&b.rect.top()))
        .expect("an opener");
    fixture.click(&opener).settle();
    let open = painted_times(&fixture, "Find what:");
    for frame in 0..3 {
        fixture.idle(1);
        let stories =
            last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(ty, _, run)| ty.contains("::StoryProps") && *run == MemoRun::Ran).count();
        assert!(stories >= 1, "idle frame {frame}: the story with its dialog open ran");
        assert_eq!(openers(&fixture), closed - 1, "idle frame {frame}: the card keeps the story, its dialog counted");
        assert_eq!(painted_times(&fixture, "Find what:"), open, "idle frame {frame}: the dialog stays open");
    }
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

/// The card and story views of the last frame by id, with how each took part: ran, replayed or out of view. Sizing and
/// measuring calls count.
fn view_runs(fixture: &Fixture) -> Vec<(egui::Id, MemoRun)> {
    let views = |ty: &str| ty.contains("::CardProps<") || ty.contains("::StoryProps<");
    last_frame_memo_runs(fixture.ctx()).into_iter().filter(|(ty, _, _)| views(ty)).map(|(_, id, run)| (id, run)).collect()
}

/// The card views of the last frame that ran, by id, once each.
fn cards_ran(fixture: &Fixture) -> Vec<egui::Id> {
    let mut ids: Vec<_> = last_frame_memo_runs(fixture.ctx())
        .into_iter()
        .filter(|(ty, _, run)| ty.contains("::CardProps") && *run == MemoRun::Ran)
        .map(|(_, id, _)| id)
        .collect();
    ids.sort_unstable_by_key(|id| id.value());
    ids.dedup();
    ids
}

fn ran(runs: &[(egui::Id, MemoRun)]) -> Vec<egui::Id> {
    runs.iter().filter(|(_, run)| *run == MemoRun::Ran).map(|(id, _)| *id).collect()
}

/// An idle Overview runs no card and no story: the Overview's view replays its last run, its cards included, or every
/// card in view replays. The Button card's Variants story animates a loading button, which runs every frame by itself,
/// so the Controls level shows its Slider cards only.
#[trial]
fn an_idle_overview_runs_no_card_or_story() {
    for (level, filter) in [("Primitives", ""), ("Composites", ""), ("Patterns", ""), ("Shells", ""), ("Controls", "Slider")] {
        let mut gallery = gallery_filtered(level, filter);
        let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
        fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(3);
        let runs = view_runs(&fixture);
        let overview_replayed =
            last_frame_memo_runs(fixture.ctx()).iter().any(|(ty, _, run)| ty.contains("::OverviewProps<") && *run == MemoRun::Replayed);
        assert!(overview_replayed || runs.iter().any(|(_, run)| *run == MemoRun::Replayed), "{level}: the cards in view replay");
        assert!(ran(&runs).is_empty(), "{level}: an idle frame ran {} of {} card and story views: {:?}", ran(&runs).len(), runs.len(), ran(&runs));
    }
}

/// An edit of one story's data runs that card alone: the click that flips a checkbox of one card, and the confirming run
/// of the changed values after it, run no other card.
#[trial]
fn a_story_data_edit_runs_one_card() {
    let mut gallery = gallery_filtered("Controls", "Checkbox");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(2);
    let target = *fixture.by_role(Role::CHECK_BOX).first().expect("a checkbox");
    let checked = |fixture: &Fixture| fixture.tree().toggled_of(target.node);
    let before = checked(&fixture);
    let mut cards = Vec::new();
    for events in [vec![pointer::moved(target.center())], vec![pointer::press(target.center())], vec![pointer::release(target.center())]] {
        fixture.frame(events);
        cards.extend(cards_ran(&fixture));
    }
    fixture.frame(vec![pointer::moved(egui::pos2(1.0, 1.0))]);
    cards.extend(cards_ran(&fixture));
    // The card settles within a frame after `settle`.
    fixture.settle();
    cards.extend(cards_ran(&fixture));
    fixture.idle(1);
    cards.extend(cards_ran(&fixture));
    assert_ne!(checked(&fixture), before, "the click flipped the checkbox");
    cards.sort_unstable_by_key(|id| id.value());
    cards.dedup();
    assert_eq!(cards.len(), 1, "an edit of one story ran these cards: {cards:?}");
    fixture.idle(1);
    assert!(ran(&view_runs(&fixture)).is_empty(), "then every card replays: {:?}", ran(&view_runs(&fixture)));
}

/// While the Overview scrolls, the cards that stay wholly in view replay their records moved with the scroll. Only the
/// cards the scroll brings in run, at most one per column at the edge it brings them in at (a card its edge cut runs
/// until a run sees it whole), and the cards the pointer enters and leaves. No card out of view runs, and the frame
/// after the scroll ends replays every card.
#[trial]
fn scrolling_runs_only_the_cards_entering_the_view() {
    let mut gallery = gallery_at("Composites");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle().idle(2);
    let header = fixture.by_label("Open").expect("a card header").center();
    fixture.frame(vec![pointer::moved(header)]).settle();
    let listed = listed("Composites", "").len();
    for _ in 0..4 {
        fixture.frame(vec![pointer::wheel(-120.0)]);
        let ran = cards_ran(&fixture);
        let mut replayed: Vec<_> = last_frame_memo_runs(fixture.ctx())
            .into_iter()
            .filter(|(ty, id, run)| ty.contains("::CardProps") && *run == MemoRun::Replayed && !ran.contains(id))
            .map(|(_, id, _)| id)
            .collect();
        replayed.sort_unstable_by_key(|id| id.value());
        replayed.dedup();
        let window = fixture.shown();
        let mut columns: Vec<i64> = fixture
            .by_role(Role::BUTTON)
            .into_iter()
            .filter(|button| fixture.tree().name_of(button.node) == Some("Open") && window.contains_rect(button.rect))
            .map(|open| open.rect.left().round() as i64)
            .collect();
        columns.sort_unstable();
        columns.dedup();
        assert!(!replayed.is_empty(), "the cards that stay in view replay at their moved place");
        assert!(
            ran.len() <= columns.len() + 2,
            "only the cards entering the view ({} columns) and the pointer's run: {} ran, {} replayed",
            columns.len(),
            ran.len(),
            replayed.len()
        );
        assert!(ran.len() + replayed.len() < listed, "no card out of view takes part: {} of {listed}", ran.len() + replayed.len());
    }
    fixture.settle().idle(1);
    assert!(ran(&view_runs(&fixture)).is_empty(), "the frame after the scroll ends replays every card: {:?}", ran(&view_runs(&fixture)));
}

/// The inspected card replays as every card does, and keeps the inspector's record of its run: the record a replay
/// leaves is the one the real run made.
#[trial]
fn the_inspected_card_replays_with_the_record_of_its_run() {
    const CHECKBOX: &str = "craft_ui_widgets::choice::check::Checkbox<'_>";
    let mut gallery = gallery_filtered("Controls", "Checkbox");
    let mut fixture = Fixture::new(move |ui| gallery.ui(ui));
    fixture.room(Vec2::new(1400.0, 900.0)).settle();
    let title = fixture.texts().iter().find(|run| run.text == "Checkbox").expect("the card's title").rect;
    fixture.click_at(title.center()).settle();
    let tab = fixture.by_label("Inspect").expect("the Inspect tab");
    fixture.click(&tab).settle();
    let catalog = Catalog::linked();
    let records = |fixture: &Fixture| -> Vec<_> {
        catalog
            .stories_of(CHECKBOX)
            .iter()
            .filter_map(|story| craft_ui::runtime::inspect::recorded(fixture.ctx(), region_key(&PanelId::Overview, *story)))
            .collect()
    };
    let ran = records(&fixture);
    assert!(!ran.is_empty(), "the selected card's stories are inspected");
    fixture.idle(3);
    assert!(view_runs(&fixture).iter().all(|(_, run)| *run != MemoRun::Ran), "the inspected card replays");
    let replayed = records(&fixture);
    assert_eq!(replayed.len(), ran.len(), "every inspected story keeps its record");
    assert!(replayed.iter().zip(&ran).all(|(now, then)| **now == **then), "a replay keeps the record the real run made");
}
