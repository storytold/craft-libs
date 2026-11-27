//! The hover text of a preferences row: a rule's help string is what the row's field shows while the pointer rests on
//! it, under the `setting.<path>.help` key and with the English as the fallback; it paints no line under the row, and
//! the control's accessible description carries it. The line the window adds itself ("Applies at next launch.") stays
//! on the page, and the search still matches the string.

use craft_i18n::Folder;
use craft_i18n::matcher::Matcher;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Role};
use craft_ui::patterns::preferences::{
    PageSpec, PrefValue, PreferencesDraft, PreferencesLayout, PreferencesModel, PreferencesState, PreferencesWindow, RowKind, RowSpec, SettingRule,
    SettingsDocument, list_pages, row_shown,
};
use craft_ui::testing::access::name;
use craft_ui::testing::pointer::moved;
use craft_ui::testing::{Fixture, trial};
use egui::Ui;
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;

/// The sentence a row shows on hover.
const TIP: &str = "Always show crosshair in center of brush cursor";
/// The line a row paints under its control.
const HELP: &str = "Drawn with its own note.";

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Cursors {
    show_crosshair_in_brush_tip: bool,
    brush_tip_outline: bool,
    tablet_input: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Prefs {
    cursors: Cursors,
}

/// The search still finds a row by the words of its hover text, as it did by its help line.
#[trial]
fn the_search_still_matches_the_words_of_a_rows_hover_text() {
    let doc = SettingsDocument::new(Prefs::default())
        .expect("the settings serialize")
        .rules(vec![SettingRule::new("cursors.showCrosshairInBrushTip").help(TIP)]);
    let pages = PreferencesModel::pages(&doc);
    let matcher = Matcher::new(&Folder::new("en"), "crosshair in center");
    let listed = list_pages(&pages, &matcher);
    let found: Vec<&str> = listed
        .iter()
        .flat_map(|listed| pages[listed.index].groups.iter().flat_map(|group| group.rows.iter()).map(move |row| (row, listed)))
        .filter(|(row, listed)| row_shown(&matcher, listed, row))
        .map(|(row, _)| row.key)
        .collect();
    assert!(found.contains(&"cursors.showCrosshairInBrushTip"), "the hover words still find the row: {found:?}");
}

/// A rule's help string reaches the row as its hover text, not as a line under the row.
#[trial]
fn a_rules_help_string_becomes_the_rows_hover_text() {
    let doc = SettingsDocument::new(Prefs::default())
        .expect("the settings serialize")
        .rules(vec![SettingRule::new("cursors.showCrosshairInBrushTip").help(TIP)]);
    let pages = PreferencesModel::pages(&doc);
    let row = pages
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find(|row| row.key == "cursors.showCrosshairInBrushTip")
        .expect("the cursors row");
    assert_eq!(row.tooltip, Some(TIP), "the help string is the row's hover text");
    assert_eq!(row.help, None, "and nothing paints under the row");
}

/// The note the window adds for a setting read at launch stays the line under the row, and is no hover text.
#[trial]
fn a_setting_read_at_launch_keeps_its_line_on_the_page() {
    let doc = SettingsDocument::new(Prefs::default()).expect("the settings serialize").rules(vec![SettingRule::new("cursors.tabletInput").restart()]);
    let pages = PreferencesModel::pages(&doc);
    let row = pages
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find(|row| row.key == "cursors.tabletInput")
        .expect("the row read at launch");
    assert!(row.help.is_some(), "the window still writes its note under the row: {row:?}");
    assert_eq!(row.tooltip, None, "and the note is not hover text");
}

#[derive(Default)]
struct Model {
    crosshair: bool,
    outline: bool,
}

impl PreferencesModel for Model {
    fn pages(&self) -> Vec<PageSpec<'_>> {
        vec![PageSpec::new(
            "cursors",
            "Cursors",
            vec![
                RowSpec::new("cursors.showCrosshairInBrushTip", "Show crosshair in brush tip", RowKind::Bool(self.crosshair)).tooltip(TIP),
                RowSpec::new("cursors.brushTipOutline", "Brush tip outline", RowKind::Bool(self.outline)).help(HELP),
                RowSpec::new("cursors.other", "Other cursors", RowKind::Bool(false)),
            ],
        )]
    }

    fn set(&mut self, key: &str, value: PrefValue) {
        let PrefValue::Bool(value) = value else { return };
        match key {
            "cursors.showCrosshairInBrushTip" => self.crosshair = value,
            "cursors.brushTipOutline" => self.outline = value,
            _ => {}
        }
    }

    fn custom_row(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn custom_page(&mut self, _: &mut Ui, _: &str) -> bool {
        false
    }

    fn reset_page(&mut self, _: &str) {
        *self = Model::default();
    }
}

impl PreferencesDraft for Model {
    fn differs(&self) -> bool {
        self.crosshair || self.outline
    }

    fn apply(&mut self) -> Result<(), String> {
        Ok(())
    }

    fn cancel(&mut self) {
        *self = Model::default();
    }
}

/// The classic window over `Model`, opened on the cursors page.
fn window() -> Fixture {
    let model = Rc::new(RefCell::new(Model::default()));
    let shown = model.clone();
    let state = Rc::new(RefCell::new({
        let mut state = PreferencesState::default();
        state.selected = "cursors".to_string();
        state
    }));
    let bound = state.clone();
    let mut fixture = Fixture::new(move |ui| {
        let mut state = bound.borrow_mut();
        let _ = PreferencesWindow::new(Bind::bound(&mut *state)).layout(PreferencesLayout::Classic).embedded().show(ui, &mut *shown.borrow_mut());
    });
    fixture.settle();
    fixture
}

/// The text of every tooltip showing in this frame.
fn shown_tips(fixture: &Fixture) -> Vec<String> {
    let tree = fixture.tree();
    fixture.by_role(Role::TOOLTIP).iter().filter_map(|tip| name(tree, tip.node).map(str::to_owned)).collect()
}

/// A row's hover text shows on its control after the usual delay, paints no line while it is not hovered, and is in the
/// accessible tree as a description the whole time.
#[trial]
fn a_rows_hover_text_shows_on_the_field_and_paints_no_line() {
    let mut fixture = window();
    assert!(fixture.texts().iter().all(|run| run.text != TIP), "the hover text paints no line under the row");
    assert!(fixture.texts().iter().any(|run| run.text == HELP), "the other row still paints its line");
    let control = fixture.by_label("Show crosshair in brush tip").expect("the row's control");
    assert_eq!(fixture.tree().description_of(control.node), Some(TIP), "the control's description carries the sentence");
    assert_eq!(shown_tips(&fixture), Vec::<String>::new(), "nothing is hovered yet");
    fixture.frame(vec![moved(control.rect.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    fixture.idle(2);
    assert!(!fixture.by_role(Role::TOOLTIP).is_empty(), "the pointer on the control shows a tooltip (hovered {control:?})");
    assert!(fixture.texts().iter().any(|run| run.text == TIP), "and that tooltip paints the sentence");
}

/// A row with a line under it and no hover text stays as it was: the line paints, and no tooltip carries it.
#[trial]
fn a_row_with_a_help_line_keeps_it_on_the_page_only() {
    let fixture = window();
    let row = fixture.by_label("Brush tip outline").expect("the row with a line");
    assert_ne!(fixture.tree().description_of(row.node), Some(TIP), "its control carries no hover text");
    assert!(fixture.texts().iter().any(|run| run.text == HELP), "its line paints");
    assert!(!shown_tips(&fixture).iter().any(|text| text == HELP), "a line under a row is not a tooltip");
}

/// A plain row gains nothing on hover.
#[trial]
fn a_plain_row_still_shows_nothing_extra_on_hover() {
    let mut fixture = window();
    let row = fixture.by_label("Other cursors").expect("the plain row");
    fixture.frame(vec![moved(row.rect.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    assert_eq!(shown_tips(&fixture), Vec::<String>::new(), "the plain row gains no tooltip");
}
