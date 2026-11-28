//! The settings document without a window: rows, applied edits, rebase and row conditions.

use craft_i18n::Folder;
use craft_i18n::matcher::Matcher;
use craft_theme::Rgba;
use craft_ui::patterns::preferences::{
    CustomSettings, PageInfo, PrefValue, PreferencesDraft, PreferencesModel, RowKind, SettingRule, SettingsDocument, SettingsDraft, StandardPage,
    list_pages, row_shown,
};
use craft_ui::testing::{Fixture, trial};
use serde::{Deserialize, Serialize};
use std::cell::RefCell;
use std::rc::Rc;
use toml::Value;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Performance {
    use_gpu: bool,
    history_states: i64,
    zoom_step: f64,
    render_mode: String,
    canvas_custom_color: String,
    tile_size: u8,
    levels: Vec<i64>,
}

impl Default for Performance {
    fn default() -> Self {
        Self {
            use_gpu: true,
            history_states: 50,
            zoom_step: 1.5,
            render_mode: "auto".into(),
            canvas_custom_color: "#336699".into(),
            tile_size: 64,
            levels: vec![1, 2],
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Type {
    recent_fonts: i64,
    favorite_font: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Prefs {
    performance: Performance,
    #[serde(rename = "type")]
    text: Type,
}

fn rules() -> Vec<SettingRule> {
    vec![
        SettingRule::new("performance.historyStates").int(1..=1000),
        SettingRule::new("performance.zoomStep").float(0.5..=4.0, 2),
        SettingRule::new("performance.renderMode").choices(&[("auto", "Automatic"), ("soft", "Software")]),
        SettingRule::new("performance.canvasCustomColor").color(),
        SettingRule::new("performance.tileSize").int(0..=1000),
    ]
}

fn document() -> SettingsDocument<Prefs> {
    SettingsDocument::new(Prefs::default()).expect("the prefs serialize").pages(StandardPage::ALL.map(PageInfo::from).to_vec()).rules(rules())
}

fn history_states(doc: &impl PreferencesModel) -> i64 {
    doc.pages()
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find_map(|row| match row.kind {
            RowKind::Int { value, .. } if row.key == "performance.historyStates" => Some(value),
            _ => None,
        })
        .expect("a row")
}

/// `set` clamps to the range and keeps to the choices; Apply returns the edited keys once through `take_applied_edits`;
/// Cancel restores the draft; a value the struct can't hold refuses Apply and keeps the draft.
#[trial]
fn apply_returns_the_typed_value_and_cancel_drops_the_draft() {
    let mut doc = document();
    assert!(!doc.differs());
    doc.set("performance.historyStates", PrefValue::Int(5000));
    assert_eq!(history_states(&doc), 1000);
    doc.set("performance.renderMode", PrefValue::Choice("nope".into()));
    doc.set("performance.renderMode", PrefValue::Choice("soft".into()));
    assert!(doc.differs());

    doc.cancel();
    assert!(!doc.differs());
    assert_eq!(history_states(&doc), 50);

    doc.set("performance.historyStates", PrefValue::Int(0));
    doc.set("performance.renderMode", PrefValue::Choice("soft".into()));
    doc.set("performance.canvasCustomColor", PrefValue::Color(Rgba([0x11, 0x22, 0x33, 0x80])));
    doc.set("performance.zoomStep", PrefValue::Float(9.0));
    doc.apply().expect("the draft fits the struct");
    let edits = doc.take_applied_edits().expect("applied");
    let edit = |key: &str| edits.iter().find(|(edited, _)| edited == key).map(|(_, value)| value.clone());
    assert_eq!(edits.len(), 4, "only the edited keys: {edits:?}");
    assert_eq!(edit("performance.historyStates"), Some(Some(Value::Integer(1))));
    assert_eq!(edit("performance.renderMode"), Some(Some(Value::String("soft".into()))));
    assert_eq!(edit("performance.canvasCustomColor"), Some(Some(Value::String("#112233".into()))));
    assert_eq!(edit("performance.zoomStep"), Some(Some(Value::Float(4.0))));
    assert_eq!(edit("performance.levels"), None, "a key that kept its value is not an edit");
    let saved = doc.saved();
    assert_eq!(saved.performance.history_states, 1);
    assert_eq!(saved.performance.levels, [1, 2]);
    assert!(doc.take_applied_edits().is_none());
    assert!(!doc.differs());

    doc.set("performance.tileSize", PrefValue::Int(300));
    let error = doc.apply().expect_err("300 doesn't fit a u8");
    assert!(error.starts_with("These settings can't be applied: "), "{error}");
    assert!(doc.take_applied_edits().is_none());
    assert!(doc.differs());
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Sparse {
    brush: SparseBrush,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct SparseBrush {
    size: i64,
    last_preset: Option<String>,
}

/// Reset Section replaces the page: a key the defaults leave out (an `Option` that is `None`) goes too.
#[trial]
fn reset_section_drops_keys_the_defaults_do_not_serialize() {
    let saved = Sparse { brush: SparseBrush { size: 9, last_preset: Some("soft".into()) } };
    let mut doc = SettingsDocument::new(saved).expect("serializes");
    doc.reset_page("brush");
    doc.apply().expect("the draft fits the struct");
    let edits = doc.take_applied_edits().expect("applied");
    let edit = |key: &str| edits.iter().find(|(edited, _)| edited == key).map(|(_, value)| value.clone());
    assert_eq!(edit("brush.size"), Some(Some(Value::Integer(0))));
    assert_eq!(edit("brush.lastPreset"), Some(None), "the removed key is an edit that removes it");
    assert_eq!(*doc.saved(), Sparse::default());
}

fn zoom_step(doc: &impl PreferencesModel) -> f64 {
    doc.pages()
        .iter()
        .flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter()))
        .find_map(|row| match row.kind {
            RowKind::Float { value, .. } if row.key == "performance.zoomStep" => Some(value),
            _ => None,
        })
        .expect("a row")
}

/// A saved value from another instance moves the rows the user has not edited and leaves the edited ones; the next Apply
/// writes only the user's edits.
#[trial]
fn rebase_keeps_the_edited_rows_and_follows_the_others() {
    let mut other = Prefs::default();
    other.performance.history_states = 30;
    other.performance.zoom_step = 3.0;

    let mut doc = document();
    doc.set("performance.historyStates", PrefValue::Int(20));
    doc.rebase(Prefs::default()).expect("serializes");
    assert_eq!(history_states(&doc), 20, "a value equal to the saved one changes nothing");
    assert!(doc.differs());

    doc.rebase(other.clone()).expect("serializes");
    assert_eq!(history_states(&doc), 20, "the edited row keeps the draft");
    assert_eq!(zoom_step(&doc), 3.0, "the other row follows the new saved value");
    assert!(doc.differs(), "the edit is still unsaved");
    assert_eq!(*doc.saved(), other);

    doc.apply().expect("the draft fits the struct");
    let edits = doc.take_applied_edits().expect("applied");
    assert_eq!(edits, [("performance.historyStates".to_owned(), Some(Value::Integer(20)))], "only the edited key");

    let mut untouched = document();
    untouched.rebase(other).expect("serializes");
    assert_eq!(history_states(&untouched), 30);
    assert!(!untouched.differs(), "without edits the draft is the new saved value");
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Export {
    format: String,
    enabled: bool,
    lossless: bool,
    quality: i64,
    other: i64,
    never: i64,
}

impl Default for Export {
    fn default() -> Self {
        Self { format: "png".into(), enabled: false, lossless: false, quality: 80, other: 1, never: 1 }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct ExportPrefs {
    export: Export,
}

/// Whether the pages show the row `key`: listed and not hidden by its rule.
fn shown(doc: &impl PreferencesModel, key: &str) -> bool {
    doc.pages().iter().flat_map(|page| page.groups.iter().flat_map(|group| group.rows.iter())).any(|row| row.key == key && row.visible)
}

/// A row with two `shown_when` conditions and one `shown_unless` shows only while all three hold, as the draft changes.
#[trial]
fn a_row_with_several_conditions_shows_only_while_all_hold() {
    let rules = vec![
        SettingRule::new("export.format").choices(&[("png", "PNG"), ("webp", "WebP")]),
        SettingRule::new("export.quality")
            .int(1..=100)
            .shown_when("export.format", "webp")
            .shown_when("export.enabled", "true")
            .shown_unless("export.lossless", "true"),
        SettingRule::new("export.other").shown_unless("export.absent", "x"),
        SettingRule::new("export.never").shown_when("export.absent", "x"),
    ];
    let mut doc = SettingsDocument::new(ExportPrefs::default()).expect("serializes").rules(rules);
    let quality = |doc: &SettingsDocument<ExportPrefs>| shown(doc, "export.quality");
    assert!(!quality(&doc), "png hides it");

    doc.set("export.format", PrefValue::Choice("webp".into()));
    assert!(!quality(&doc), "one shown_when is not enough");
    doc.set("export.enabled", PrefValue::Bool(true));
    assert!(quality(&doc), "both shown_when hold and lossless is off");
    doc.set("export.lossless", PrefValue::Bool(true));
    assert!(!quality(&doc), "shown_unless hides it");
    doc.set("export.lossless", PrefValue::Bool(false));
    assert!(quality(&doc));
    doc.set("export.format", PrefValue::Choice("png".into()));
    assert!(!quality(&doc), "a shown_when that stops holding hides it again");

    assert!(shown(&doc, "export.other"), "an absent key passes shown_unless");
    assert!(!shown(&doc, "export.never"), "an absent key fails shown_when");
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
struct Memory {
    percent: Option<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct MemoryPrefs {
    memory: Memory,
}

/// A custom row that turns its value off.
struct TurnOff;

impl CustomSettings<MemoryPrefs> for TurnOff {
    fn nested(&mut self, _: &mut egui::Ui, draft: &mut SettingsDraft) {
        draft.remove("memory.percent");
    }
}

/// A custom row's `SettingsDraft::remove` stores no value: Apply reports the key with none and the struct reads `None`.
#[trial]
fn a_removed_draft_value_applies_as_no_value() {
    let saved = MemoryPrefs { memory: Memory { percent: Some(70) } };
    let doc = Rc::new(RefCell::new(SettingsDocument::new(saved).expect("serializes").custom(TurnOff)));
    let shown = Rc::clone(&doc);
    // P3(pdf-dialogs-w8): L-176 `PreferencesModel::nested` takes the window's `Ui`.
    let mut fixture = Fixture::new(move |ui| shown.borrow_mut().nested(ui));
    fixture.idle(1);
    let mut doc = doc.borrow_mut();
    doc.apply().expect("the draft fits the struct");
    let edits = doc.take_applied_edits().expect("applied");
    assert_eq!(edits, vec![("memory.percent".to_owned(), None)]);
    assert_eq!(doc.saved().memory.percent, None);
}

/// A row whose app draws its own labels is found by its rule's label: the search lists its page and shows it.
#[trial]
fn the_search_finds_a_host_labelled_row_by_its_rules_label() {
    let rules = vec![
        SettingRule::new("export.format").choices(&[("png", "PNG"), ("webp", "WebP")]),
        SettingRule::new("export.tablet").label("Tablet pressure").custom_rows(),
    ];
    let doc = SettingsDocument::new(ExportPrefs::default()).expect("serializes").rules(rules);
    let pages = PreferencesModel::pages(&doc);
    let matcher = Matcher::new(&Folder::new("en"), "tablet");
    let listed = list_pages(&pages, &matcher);
    let found: Vec<(&str, bool)> = listed
        .iter()
        .flat_map(|listed| pages[listed.index].groups.iter().flat_map(|group| group.rows.iter()).map(move |row| (row, listed)))
        .filter(|(row, listed)| row_shown(&matcher, listed, row))
        .map(|(row, _)| (row.key, matches!(row.kind, RowKind::CustomRows)))
        .collect();
    assert_eq!(found, [("export.tablet", true)], "the search shows the host-labelled row alone");
}
