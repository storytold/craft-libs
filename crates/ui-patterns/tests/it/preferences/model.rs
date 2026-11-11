//! The settings document without a window.

use craft_theme::Rgba;
use craft_ui::patterns::preferences::{PageInfo, PrefValue, PreferencesModel, RowKind, SettingRule, SettingsDocument, StandardPage};
use craft_ui::testing::trial;
use serde::{Deserialize, Serialize};

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

/// `set` clamps to the range and keeps to the choices; Apply returns the typed value once through `take_applied`; Cancel
/// restores the draft; a value the struct can't hold refuses Apply and keeps the draft.
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
    let applied = doc.take_applied().expect("applied");
    assert_eq!(applied.performance.history_states, 1);
    assert_eq!(applied.performance.render_mode, "soft");
    assert_eq!(applied.performance.canvas_custom_color, "#112233");
    assert_eq!(applied.performance.zoom_step, 4.0);
    assert_eq!(applied.performance.levels, [1, 2]);
    assert_eq!(*doc.saved(), applied);
    assert!(doc.take_applied().is_none());
    assert!(!doc.differs());

    doc.set("performance.tileSize", PrefValue::Int(300));
    let error = doc.apply().expect_err("300 doesn't fit a u8");
    assert!(error.starts_with("These settings can't be applied: "), "{error}");
    assert!(doc.take_applied().is_none());
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
    assert_eq!(doc.take_applied().expect("applied"), Sparse::default());
}
