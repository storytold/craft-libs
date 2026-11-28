//! The hover text of a preferences row: a rule's help string is what the row's control shows while the pointer rests on
//! it, under the `setting.<path>.help` key and with the English as the fallback; it paints no line under the row, and
//! the control's accessible description carries it. The line the window adds itself ("Applies at next launch.") stays
//! on the page, the search still matches the string, and a row the host draws - where the pattern has no control of its
//! own to hover - keeps its sentence as the line rather than losing it.

use craft_i18n::Folder;
use craft_i18n::matcher::Matcher;
use craft_ui::choice::Checkbox;
use craft_ui::contract::tooltip::DELAY;
use craft_ui::contract::{Bind, Role, Show as _, ShowContainer as _};
use craft_ui::form::Field;
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
/// The sentence of the row the host draws itself.
const HOST_TIP: &str = "Only reads with a pen on the tablet.";
/// A host-drawn row given both a help line and hover text: both sentences belong on its line.
const BOTH_HELP: &str = "Reads once, when the page opens.";
const BOTH_TIP: &str = "Changing it takes effect once the app starts again.";
/// The same pair on a control the host draws inside the pattern's field.
const FIELD_HELP: &str = "The line this row always had.";
const FIELD_TIP: &str = "The sentence that would be hover text.";
/// The note the window adds for a setting read at launch.
const RESTART: &str = "Applies at next launch.";

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
        .rules(vec![SettingRule::new("cursors.showCrosshairInBrushTip").help(TIP), SettingRule::new("cursors.tabletInput").custom_rows().help(TIP)]);
    let pages = PreferencesModel::pages(&doc);
    let row = pages
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find(|row| row.key == "cursors.showCrosshairInBrushTip")
        .expect("the cursors row");
    assert_eq!(row.tooltip, Some(TIP), "the help string is the row's hover text");
    assert_eq!(row.help, None, "and nothing paints under the row");
    let host_row = pages
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find(|row| row.key == "cursors.tabletInput")
        .expect("the row the host draws");
    assert!(matches!(host_row.kind, RowKind::CustomRows), "the rule asks the host to draw it: {host_row:?}");
    assert_eq!(host_row.tooltip, Some(TIP), "and its string reaches the same hover slot");
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
    tablet: bool,
    both_rows: bool,
    both_field: bool,
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
                RowSpec::new("cursors.tabletInput", "Tablet input", RowKind::CustomRows).tooltip(HOST_TIP),
                RowSpec::new("cursors.bothRows", "Both sentences rows", RowKind::CustomRows).help(BOTH_HELP).tooltip(BOTH_TIP),
                RowSpec::new("cursors.bothField", "Both sentences field", RowKind::Custom).help(FIELD_HELP).tooltip(FIELD_TIP),
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

    fn custom_row(&mut self, ui: &mut Ui, key: &str) -> bool {
        match key {
            "cursors.tabletInput" => {
                // The host draws its own labelled field for this row.
                let _ = Field::new("Tablet input").show(ui, |ui, _| {
                    Checkbox::new(Bind::bound(&mut self.tablet)).show(ui);
                });
            }
            "cursors.bothRows" => {
                let _ = Checkbox::new(Bind::bound(&mut self.both_rows)).show(ui);
            }
            "cursors.bothField" => {
                let _ = Checkbox::new(Bind::bound(&mut self.both_field)).show(ui);
            }
            _ => {}
        }
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

/// The classic window over a real settings document holding `rules`, opened on its cursors page.
fn document_window(rules: Vec<SettingRule>) -> Fixture {
    let doc = Rc::new(RefCell::new(SettingsDocument::new(Prefs::default()).expect("the settings serialize").rules(rules)));
    let shown = doc.clone();
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

/// The whole path in a real window: a rule's help string, the document's row, and the pointer on the control the
/// pattern drew - the sentence shows as the tip and paints no line at rest.
#[trial]
fn a_document_row_shows_the_rules_help_string_on_hover() {
    let mut fixture = document_window(vec![
        SettingRule::new("cursors.showCrosshairInBrushTip").help(TIP),
        SettingRule::new("cursors.brushTipOutline"),
        SettingRule::new("cursors.tabletInput").restart(),
    ]);
    assert!(fixture.texts().iter().all(|run| run.text != TIP), "the resting page shows no line of the sentence");
    let control = fixture.by_label("Show crosshair in brush tip").expect("the row the rule speaks for");
    assert_eq!(fixture.tree().description_of(control.node), Some(TIP), "the control carries the sentence the whole time");
    fixture.frame(vec![moved(control.rect.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    fixture.idle(2);
    assert!(!fixture.by_role(Role::TOOLTIP).is_empty(), "the pointer on the document's control shows a tooltip (hovered {control:?})");
    assert!(fixture.texts().iter().any(|run| run.text == TIP), "and that tooltip paints the sentence");
}

/// A row the host draws keeps its sentence on the page: the pattern has no hover target of its own there, so the
/// sentence falls back to the line under the row and is lost nowhere.
#[trial]
fn a_host_drawn_row_keeps_its_sentence_on_the_page() {
    let mut fixture = window();
    let control = fixture.by_label("Tablet input").expect("the host-drawn row's control");
    assert!(fixture.texts().iter().any(|run| run.text == HOST_TIP), "the row the host draws writes the sentence as its line");
    assert_eq!(fixture.tree().description_of(control.node), None, "and its control carries no hover text of ours");
    fixture.frame(vec![moved(control.rect.center())]);
    fixture.advance(DELAY);
    fixture.settle();
    fixture.idle(2);
    assert_eq!(shown_tips(&fixture), Vec::<String>::new(), "hovering the host's control gains no tooltip");
}

/// A host-drawn row given both a help line and hover text keeps both sentences: the pattern has no
/// hover target of its own there, so the line under the row carries them - for the rows the host
/// draws and for a control it draws inside our field alike.
#[trial]
fn a_host_drawn_row_keeps_both_its_line_and_its_hover_sentence() {
    let fixture = window();
    let page: String = fixture.texts().iter().map(|run| run.text.as_str()).collect::<Vec<_>>().join("\n");
    for sentence in [BOTH_HELP, BOTH_TIP, FIELD_HELP, FIELD_TIP] {
        assert!(page.contains(sentence), "the host-drawn row writes {sentence:?} on the page");
    }
}

/// The same through a settings document: a `custom_rows()` rule with help and `restart()` gives the
/// row two sentences, and a row the host draws writes both - its line is not just the window's note.
#[trial]
fn a_document_host_row_that_asks_for_a_restart_keeps_both_sentences() {
    let fixture = document_window(vec![SettingRule::new("cursors.tabletInput").custom_rows().help(TIP).restart()]);
    let page: String = fixture.texts().iter().map(|run| run.text.as_str()).collect::<Vec<_>>().join("\n");
    assert!(page.contains(TIP), "the rule's sentence stays on the page for the host-drawn row");
    assert!(page.contains(RESTART), "and so does the window's note");
}
