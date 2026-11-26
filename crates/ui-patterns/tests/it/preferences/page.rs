//! `PreferencesPage` over a model with no draft: each edit reaches the model in the pass it lands in. With
//! [`PreferencesLayout::Cards`] the page is one scrolling column of cards: every page is shown at once, short choices
//! are segmented and the search counts its matches.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Entry, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::patterns::preferences::{
    GroupSpec, PageSpec, PrefValue, PreferencesDraft, PreferencesLayout, PreferencesModel, PreferencesPage, PreferencesState, PreferencesWindow,
    PreferencesWords, RowKind, RowSpec, SearchResults,
};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::words::CommonWords;
use craft_ui::testing::pointer::{moved, press, release, wheel};
use craft_ui::testing::{Fixture, trial};
use egui::Ui;
use std::cell::{Cell, RefCell};
use std::rc::Rc;

/// One page with one check box row. Every `set` is recorded with the index of the pass it ran in. The model implements
/// no `PreferencesDraft`, so nothing can apply or cancel a draft.
#[derive(Default)]
struct Live {
    snap: bool,
    pass: usize,
    sets: Vec<(String, PrefValue, usize)>,
}

impl PreferencesModel for Live {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![PageSpec::new("general", "General", vec![RowSpec::new("general.snap", "Snap to grid", RowKind::Bool(self.snap))])]
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        if let ("general.snap", PrefValue::Bool(on)) = (key, &value) {
            self.snap = *on;
        }
        self.sets.push((key.to_owned(), value, self.pass));
    }

    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {}
}

/// Guards the apply-at-once path of an options page: a click on a check box calls `set` once, with the new value, in
/// the pass that received the click, and a second pass of the same frame does not call it again. The page shows none
/// of a window's OK, Cancel or Apply.
#[trial]
fn a_page_edit_reaches_the_model_in_the_same_pass_and_no_draft_method_runs() {
    let live = Rc::new(RefCell::new(Live::default()));
    let footer = Rc::new(RefCell::new(Vec::<String>::new()));
    let rerun = Rc::new(Cell::new(false));
    let (shown, words, twice) = (Rc::clone(&live), Rc::clone(&footer), Rc::clone(&rerun));
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        if twice.get() && ui.ctx().current_pass_index() == 0 {
            ui.ctx().request_discard("the trial runs a second pass");
        }
        let env = Env::of(ui);
        let (common, own) = (env.words::<CommonWords>(), env.words::<PreferencesWords>());
        *words.borrow_mut() = vec![common.ok.as_str().to_owned(), common.cancel.as_str().to_owned(), own.apply.as_str().to_owned()];
        let mut live = shown.borrow_mut();
        live.pass = ui.ctx().current_pass_index();
        PreferencesPage::new(Bind::bound(&mut state)).show(ui, &mut *live);
    });
    fixture.settle();

    let buttons: Vec<String> =
        fixture.by_role(Role::BUTTON).iter().filter_map(|button| fixture.tree().name_of(button.node).map(str::to_owned)).collect();
    assert!(!buttons.is_empty(), "the page shows Reset Section");
    assert!(buttons.iter().all(|name| !footer.borrow().contains(name)), "the page has no OK, Cancel or Apply: {buttons:?}");

    let check = fixture.by_role(Role::CHECK_BOX).into_iter().next().expect("the row shows a check box");
    fixture.frame(vec![moved(check.center())]);
    assert!(live.borrow().sets.is_empty(), "a hover edits nothing");
    rerun.set(true);
    fixture.frame(vec![press(check.center())]);
    fixture.frame(vec![release(check.center())]);
    assert!(fixture.passes() >= 2, "the click's frame ran a second pass");
    rerun.set(false);
    assert_eq!(
        live.borrow().sets,
        vec![("general.snap".to_owned(), PrefValue::Bool(true), 0)],
        "one set, with the new value, in the first pass of the click's frame"
    );
    fixture.settle();
    assert_eq!(live.borrow().sets.len(), 1, "later passes apply nothing again");
    assert!(live.borrow().snap, "the model holds the value");
}

/// A page of two titled groups with labels of different lengths, and a host-labelled row that draws its own field.
struct Grouped;

impl PreferencesModel for Grouped {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let text = |key, label| RowSpec::new(key, label, RowKind::Text("v"));
        let mut page = PageSpec::new("general", "General", Vec::new());
        page.groups = vec![
            GroupSpec::new(Some("First"), vec![text("general.a", "Aa"), RowSpec::new("general.host", "Hosted", RowKind::CustomRows)]),
            GroupSpec::new(Some("Second"), vec![text("general.b", "A much longer label")]),
        ];
        vec![page]
    }

    fn set(&mut self, _: &str, _: PrefValue) {}

    fn custom_row(&mut self, ui: &mut Ui, _: &str) -> bool {
        let _ = Field::new("Hosted").show(ui, |ui, _| {
            Text::new("hosted value").show(ui);
        });
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {}
}

fn grouped_page() -> Fixture {
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        PreferencesPage::new(Bind::bound(&mut state)).show(ui, &mut Grouped);
    });
    fixture.settle();
    fixture
}

#[trial]
fn the_rows_of_every_group_of_a_page_share_one_label_column() {
    let fixture = grouped_page();
    let end = |label: &str| {
        let run = fixture.texts().iter().find(|run| run.text.trim_end_matches(':') == label).unwrap_or_else(|| panic!("no label {label}"));
        run.rect.right()
    };
    assert_eq!(end("Aa"), end("A much longer label"), "the labels of both groups end at one x");
}

#[trial]
fn a_host_labelled_custom_row_shows_its_label_once() {
    let fixture = grouped_page();
    let tree = fixture.tree();
    let fields = tree.keys().into_iter().filter(|key| tree.name_of(*key) == Some("Hosted") && tree.role_of(*key) == Some(Role::GROUP)).count();
    assert_eq!(fields, 1, "the host's field is the row's only labelled group");
}

/// A disabled host-labelled row with help: the host draws a button.
struct HelpedRows;

impl PreferencesModel for HelpedRows {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let row = RowSpec::new("general.host", "Hosted", RowKind::CustomRows).help("Drawn with its note").enabled(false);
        vec![PageSpec::new("general", "General", vec![row])]
    }

    fn set(&mut self, _: &str, _: PrefValue) {}

    fn custom_row(&mut self, ui: &mut Ui, _: &str) -> bool {
        Button::new("Hosted button").show(ui);
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {}
}

/// The rule's help shows under the rows the host draws, and a disabled rule disables them.
#[trial]
fn a_host_labelled_row_shows_its_rules_help_and_follows_its_enabled_state() {
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        PreferencesPage::new(Bind::bound(&mut state)).show(ui, &mut HelpedRows);
    });
    fixture.settle();
    let button = fixture.by_label("Hosted button").expect("the host's button");
    let help = fixture.texts().iter().find(|run| run.text == "Drawn with its note").map(|run| run.rect).expect("the help shows");
    assert!(help.top() >= button.rect.bottom(), "the help {help:?} sits under the host's rows {:?}", button.rect);
    assert!(fixture.tree().is_disabled_key(button.node), "the disabled rule disables the host's button");
}

struct Model {
    side: String,
}

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let sides = vec![Entry::new("left", "Left"), Entry::new("right", "Right")];
        vec![
            PageSpec::new("general", "General", vec![RowSpec::new("side", "Tools side", RowKind::choice(&self.side, sides))]),
            PageSpec::new("canvas", "Canvas", vec![RowSpec::new("snap", "Snap", RowKind::Bool(true))]),
        ]
    }
    fn set(&mut self, _: &str, value: PrefValue) {
        if let PrefValue::Choice(side) = value {
            self.side = side;
        }
    }
    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }
    fn reset_page(&mut self, _: &str) {}
}

impl PreferencesDraft for Model {
    fn differs(&self) -> bool {
        false
    }
    fn apply(&mut self) -> Result<(), String> {
        Ok(())
    }
    fn cancel(&mut self) {}
}

/// A window of cards ([`PreferencesLayout::Cards`]) over two pages.
fn fixture() -> Fixture {
    let mut model = Model { side: "left".into() };
    let mut state = PreferencesState::default();
    Fixture::new(move |ui| {
        let _ = PreferencesWindow::new(Bind::bound(&mut state)).layout(PreferencesLayout::Cards).embedded().show(ui, &mut model);
    })
}

#[trial]
fn every_page_is_a_card_on_one_scrolling_page() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    assert!(fixture.by_label("General").is_some() && fixture.by_label("Canvas").is_some(), "both cards show at once");
}

#[trial]
fn a_short_choice_is_a_row_of_segments() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    let right = fixture.by_label("Right").expect("a segment for each option");
    fixture.click(&right).settle();
    assert!(fixture.texts().iter().any(|t| t.text == "Left"), "the other option stays shown beside it");
}

#[trial]
fn the_search_shows_how_many_settings_match() {
    let mut fixture = fixture();
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    assert!(!fixture.texts().iter().any(|t| t.text.contains("match")), "no count without a search");
    let search = fixture.by_label("Search settings").expect("the search field");
    fixture.click(&search).type_text("snap").settle();
    assert!(fixture.texts().iter().any(|t| t.text == "1 match"), "{:?}", fixture.texts().iter().map(|t| &t.text).collect::<Vec<_>>());
}

/// Two pages that both find "snap", the second also listing the first's setting under the same key.
struct Shared;

impl PreferencesModel for Shared {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        let grid = || RowSpec::new("snap.grid", "Snap to grid", RowKind::Bool(true));
        vec![
            PageSpec::new("general", "General", vec![grid(), RowSpec::new("general.side", "Tools side", RowKind::Bool(false))]),
            PageSpec::new("canvas", "Canvas", vec![RowSpec::new("canvas.guides", "Snap to guides", RowKind::Bool(true)), grid()]),
        ]
    }

    fn set(&mut self, _: &str, _: PrefValue) {}

    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {}
}

/// The texts shown after typing `query` into a paged page with `results`.
fn searched(results: SearchResults, query: &str) -> Vec<String> {
    let mut state = PreferencesState::default();
    let mut fixture = Fixture::new(move |ui| {
        PreferencesPage::new(Bind::bound(&mut state)).search_results(results).show(ui, &mut Shared);
    });
    fixture.room(egui::vec2(700.0, 900.0)).settle();
    let search = fixture.by_label("Search settings").expect("the search field");
    fixture.click(&search).type_text(query).settle();
    fixture.texts().iter().map(|run| run.text.trim_end_matches(':').to_owned()).collect()
}

/// A grouped search shows the matches of every page at once, a setting two pages list only once; the default shows
/// one page's matches.
#[trial]
fn a_grouped_search_shows_every_pages_matches_and_a_shared_setting_once() {
    let count = |texts: &[String], text: &str| texts.iter().filter(|t| *t == text).count();
    let grouped = searched(SearchResults::Grouped, "snap");
    assert!(count(&grouped, "Snap to guides") == 1 && count(&grouped, "Snap to grid") == 1, "both pages' matches, the shared one once: {grouped:?}");
    assert_eq!(count(&grouped, "Tools side"), 0, "rows that don't match stay hidden: {grouped:?}");

    let picked = searched(SearchResults::default(), "snap");
    assert!(count(&picked, "Snap to grid") == 1 && count(&picked, "Snap to guides") == 0, "the default shows the first page only: {picked:?}");
}

/// `reset_section(false)` leaves Reset Section out; the default shows it.
#[trial]
fn reset_section_can_be_left_out() {
    let resets = |reset: bool| {
        let mut state = PreferencesState::default();
        let mut fixture = Fixture::new(move |ui| {
            PreferencesPage::new(Bind::bound(&mut state)).reset_section(reset).show(ui, &mut Shared);
        });
        fixture.settle();
        fixture.by_role(Role::BUTTON).iter().filter(|button| fixture.tree().name_of(button.node) == Some("Reset Section")).count()
    };
    assert!(resets(true) > 0, "the default offers Reset Section");
    assert_eq!(resets(false), 0, "reset_section(false) leaves it out");
}

/// A grouped search lists only the pages that show a group: Canvas's only "grid" match is the setting General shows.
#[trial]
fn a_grouped_search_lists_no_page_whose_matches_all_show_elsewhere() {
    let grouped = searched(SearchResults::Grouped, "grid");
    assert!(grouped.iter().any(|t| t.contains("General")), "the page that shows the match is listed: {grouped:?}");
    assert!(!grouped.iter().any(|t| t.contains("Canvas")), "a page with no group of its own is not listed: {grouped:?}");
}

/// Two long pages whose every row matches "setting": each page's (key, label) pairs.
struct Many(Vec<(String, String)>, Vec<(String, String)>);

impl Many {
    fn new() -> Self {
        let rows = |page: &str| (1..=20).map(|n| (format!("{page}.{n}"), format!("{page} setting {n}"))).collect();
        Many(rows("Alpha"), rows("Beta"))
    }
}

impl PreferencesModel for Many {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        fn rows(rows: &[(String, String)]) -> Vec<RowSpec<'_>> {
            rows.iter().map(|(key, label)| RowSpec::new(key, label, RowKind::Bool(true))).collect()
        }
        vec![PageSpec::new("alpha", "Alpha", rows(&self.0)), PageSpec::new("beta", "Beta", rows(&self.1))]
    }

    fn set(&mut self, _: &str, _: PrefValue) {}

    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {}
}

/// Picking the page already selected brings its group back into view after the body scrolled away from it.
#[trial]
fn picking_the_selected_page_again_scrolls_its_group_back_into_view() {
    let mut state = PreferencesState::default();
    let mut many = Many::new();
    let mut fixture = Fixture::new(move |ui| {
        PreferencesPage::new(Bind::bound(&mut state)).search_results(SearchResults::Grouped).show(ui, &mut many);
    });
    let room = egui::vec2(700.0, 300.0);
    fixture.room(room).settle();
    let search = fixture.by_label("Search settings").expect("the search field");
    fixture.click(&search).type_text("setting").settle();
    let top = |fixture: &Fixture| fixture.texts().iter().find(|t| t.text.trim_end_matches(':') == "Beta setting 1").map(|t| t.rect.top());
    // The glide to the group outlasts `settle`, so frames run until the row stops moving.
    let glide_out = |fixture: &mut Fixture| {
        let mut still = 0;
        let mut last = top(fixture);
        for _ in 0..600 {
            fixture.frame(vec![]);
            let now = top(fixture);
            still = if now == last { still + 1 } else { 0 };
            last = now;
            if still >= 10 {
                break;
            }
        }
    };
    let pick_beta = |fixture: &mut Fixture| {
        let rows = fixture.by_role(Role::LIST_ITEM);
        let beta =
            rows.iter().find(|row| fixture.tree().name_of(row.node).is_some_and(|name| name.starts_with("Beta"))).expect("Beta in the page list");
        fixture.click(beta).settle();
        glide_out(fixture);
    };
    let in_view = |top: f32| (0.0..room.y).contains(&top);
    assert!(top(&fixture).is_none_or(|top| !in_view(top)), "the Beta group starts out of view");
    pick_beta(&mut fixture);
    assert!(top(&fixture).is_some_and(in_view), "picking Beta brings its group into view: {:?}", top(&fixture));

    let body = egui::pos2(room.x - 60.0, room.y / 2.0);
    for _ in 0..10 {
        fixture.frame(vec![moved(body), wheel(-120.0)]);
    }
    fixture.settle();
    assert!(top(&fixture).is_none_or(|top| !in_view(top)), "the wheel scrolls the Beta group out of view: {:?}", top(&fixture));

    pick_beta(&mut fixture);
    assert!(top(&fixture).is_some_and(in_view), "picking Beta again brings its group back into view: {:?}", top(&fixture));
}
