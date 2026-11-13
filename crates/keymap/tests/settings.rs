use std::time::Duration;

use craft_keymap::{Bindable, Binding, CommandRef, DropReason, Keymap, Overrides, Platform, Policy, Problem, Profiles, Scope, Scopes};
use craft_settings::{Location, OpenOptions, Probe, Settings, SettingsStore};
use serde::{Deserialize, Serialize};
use serde_json::json;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct Prefs {
    general: General,
    #[serde(with = "craft_keymap::keymap::overrides_list")]
    shortcuts: Overrides,
    profiles: Profiles,
}

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct General {
    language: String,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "keymaptest";
    const SCHEMA_VERSION: u32 = 1;
}

fn open(dir: &std::path::Path) -> SettingsStore<Prefs> {
    let probe = Probe { env: Box::new(|_| None), exe_dir: None, platform_config_dir: None };
    let options = OpenOptions::new(Location::Dir(dir.to_path_buf())).probe(probe).debounce(Duration::from_millis(10));
    let (store, report) = SettingsStore::open_with(options);
    assert!(report.section_errors.is_empty(), "{report:?}");
    store
}

fn keymap(overrides: Overrides) -> Keymap {
    let catalog = vec![
        Bindable::new(CommandRef::with_params("window.panel", json!({"panel": "layers"})), "Layers", Scope::app()),
        Bindable::new("text.bold", "Bold", Scope::app()),
        Bindable::new("edit.copy", "Copy", Scope::app()).with_default(Binding::parse("Cmd+C").unwrap()),
        Bindable::new("edit.paste", "Paste", Scope::app()).with_default(Binding::parse("Cmd+V").unwrap()),
    ];
    Keymap::new(catalog, Scopes::new().with_context("Text"), Platform::Mac).with_overrides(overrides)
}

#[test]
fn overrides_round_trip_through_craft_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let mut km = keymap(Overrides::new());
    let layers = CommandRef::with_params("window.panel", json!({"panel": "layers"}));
    km.assign(&layers, Binding::parse("Cmd+Shift+L").unwrap(), Policy::Steal).unwrap();
    km.assign(&"text.bold".into(), Binding::parse("Text: Cmd+B").unwrap(), Policy::Steal).unwrap();
    km.clear(&"edit.copy".into());

    let mut store = open(tmp.path());
    store.update(|p| {
        p.shortcuts = km.overrides().clone();
        p.profiles.save_as("Mine", km.overrides().clone(), &[]).unwrap();
        p.profiles.save_as("Spare", Overrides::new(), &[]).unwrap();
    });
    store.flush().unwrap();

    // A hand edit: one text that does not parse, one value that is not a list.
    let path = tmp.path().join("config.toml");
    let text = std::fs::read_to_string(&path).unwrap();
    let edit = r#"shortcuts = [{ command = "edit.paste", keys = ["Cmd+Nope"] }, { command = "edit.cut", keys = 5 }, "#;
    assert!(text.contains("shortcuts = ["), "{text}");
    std::fs::write(&path, text.replacen("shortcuts = [", edit, 1)).unwrap();

    let mut store = open(tmp.path());
    let mut km = keymap(store.get().shortcuts.clone());
    assert_eq!(km.bindings(&layers), vec![Binding::parse("Cmd+Shift+L").unwrap()]);
    assert_eq!(km.bindings(&"text.bold".into()), vec![Binding::parse("Text: Cmd+B").unwrap()]);
    assert!(km.bindings(&"edit.copy".into()).is_empty());
    assert_eq!(km.bindings(&"edit.paste".into()), vec![Binding::parse("Cmd+V").unwrap()], "nothing readable: the defaults");
    assert_eq!(km.overrides().get("edit.paste"), Some(&["Cmd+Nope".to_string()][..]));
    let problems = km.problems();
    assert!(problems.contains(&Problem::Dropped { key: "edit.cut".into(), reason: DropReason::NotAList }), "{problems:?}");
    assert!(problems.iter().any(|p| matches!(p, Problem::BadBinding { text, .. } if text == "Cmd+Nope")), "{problems:?}");
    assert_eq!(store.get().profiles.sets().len(), 2);

    store.update(|p| p.general.language = "de".into());
    store.flush().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("keys = [\"Cmd+Nope\"]"), "{text}");
    assert!(
        text.contains(r#"command = "window.panel {\"panel\":\"layers\"}""#) || text.contains(r#"command = 'window.panel {"panel":"layers"}'"#),
        "{text}"
    );
    assert_eq!(open(tmp.path()).get().shortcuts, store.get().shortcuts);

    // Removals reach the file.
    km.reset(&layers);
    store.update(|p| {
        p.shortcuts = km.overrides().clone();
        p.profiles.delete("Spare", &[]).unwrap();
    });
    store.flush().unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    let line = text.lines().find(|l| l.starts_with("shortcuts = ")).unwrap();
    assert!(!line.contains("window.panel") && !text.contains("Spare"), "{text}");
    let reloaded = open(tmp.path());
    assert!(keymap(reloaded.get().shortcuts.clone()).bindings(&layers).is_empty());
    assert_eq!(reloaded.get().profiles.sets().keys().collect::<Vec<_>>(), ["Mine"]);
    assert_eq!(reloaded.get().profiles.sets()["Mine"].get("text.bold"), Some(&["Text: Cmd+B".to_string()][..]));
}

#[test]
fn hand_written_array_of_tables_and_map_forms_load() {
    for text in [
        "schema_version = 1\n\n[[shortcuts]]\ncommand = \"edit.copy\"\nkeys = [\"Cmd+K\"]\n\n[[shortcuts]]\ncommand = \"edit.paste\"\nkeys = []\n",
        "schema_version = 1\n\n[shortcuts]\n\"edit.copy\" = [\"Cmd+K\"]\n\"edit.paste\" = []\n",
    ] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join("config.toml"), text).unwrap();
        let km = keymap(open(tmp.path()).get().shortcuts.clone());
        assert_eq!(km.bindings(&"edit.copy".into()), vec![Binding::parse("Cmd+K").unwrap()], "{text}");
        assert!(km.bindings(&"edit.paste".into()).is_empty(), "{text}");
    }
}

#[test]
fn a_shortcuts_value_that_is_not_a_list_loads_as_no_overrides() {
    for value in ["\"x\"", "5", "true"] {
        let tmp = tempfile::tempdir().unwrap();
        let text = format!("schema_version = 1\nshortcuts = {value}\n\n[general]\nlanguage = \"de\"\n");
        std::fs::write(tmp.path().join("config.toml"), text).unwrap();
        let store = open(tmp.path());
        assert!(store.get().shortcuts.is_empty(), "{value}");
        assert_eq!(store.get().general.language, "de", "{value}");
        let problems = keymap(store.get().shortcuts.clone()).problems();
        assert_eq!(problems, vec![Problem::Dropped { key: String::new(), reason: DropReason::NotAList }], "{value}");
    }
}

#[test]
fn a_sets_value_that_is_not_a_list_keeps_the_rest_of_the_profiles() {
    for value in ["\"x\"", "5", "true"] {
        let tmp = tempfile::tempdir().unwrap();
        let text = format!("schema_version = 1\n\n[profiles]\nactive = \"Mine\"\nsets = {value}\n");
        std::fs::write(tmp.path().join("config.toml"), text).unwrap();
        let store = open(tmp.path());
        assert_eq!(store.get().profiles.active(), "Mine", "{value}");
        assert!(store.get().profiles.sets().is_empty(), "{value}");
    }
}
