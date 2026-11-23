//! `PreferencesPage` over a model with no draft: each edit reaches the model in the pass it lands in.

use craft_ui::buttons::Button;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::display::Text;
use craft_ui::form::Field;
use craft_ui::patterns::preferences::{
    GroupSpec, PageSpec, PrefValue, PreferencesModel, PreferencesPage, PreferencesState, PreferencesWords, RowKind, RowSpec,
};
use craft_ui::runtime::env::Env;
use craft_ui::runtime::words::CommonWords;
use craft_ui::testing::pointer::{moved, press, release};
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

    fn custom_row(&mut self, _: &mut Ui, _: &str) {}

    fn custom_page(&mut self, _: &mut Ui, _: &str) {}

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

    fn custom_row(&mut self, ui: &mut Ui, _: &str) {
        let _ = Field::new("Hosted").show(ui, |ui, _| {
            Text::new("hosted value").show(ui);
        });
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) {}

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

    fn custom_row(&mut self, ui: &mut Ui, _: &str) {
        Button::new("Hosted button").show(ui);
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) {}

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
