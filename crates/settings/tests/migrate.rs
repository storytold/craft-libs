mod common;

use std::time::Duration;

use common::{Prefs, Theme, empty_probe, open_in, read_config, write_config};
use craft_settings::{LegacyFile, Location, OpenOptions, SettingsStore, Source};

#[test]
fn older_schema_is_migrated_step_by_step_and_saved() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 1\n\n# keep me\n[appearance]\ndark = true\nui_scale = 1.25\n");
    let (mut store, report) = open_in(tmp.path());
    assert_eq!(report.migrated_from, Some(1), "{report:?}");
    assert!(report.section_errors.is_empty(), "{report:?}");
    assert_eq!(store.get().appearance.theme, Theme::Dark);
    assert_eq!(store.get().appearance.ui_scale, 1.25);
    assert!(store.is_dirty(), "a migrated file is written back");

    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.starts_with("schema_version = 2\n"), "{text}");
    assert!(text.contains("# keep me"), "{text}");
    assert!(text.contains("theme = \"dark\""), "{text}");
    assert!(!text.contains("dark = true"), "{text}");
}

#[test]
fn missing_schema_version_is_treated_as_current() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "[general]\nlanguage = \"pt\"\n");
    let (mut store, report) = open_in(tmp.path());
    assert_eq!(report.migrated_from, None);
    assert!(!store.is_read_only(), "{report:?}");
    assert_eq!(store.get().general.language, "pt");
    store.update(|p| p.general.autosave_minutes = 9);
    store.flush().unwrap();
    assert!(read_config(tmp.path()).starts_with("schema_version = 2\n"));
}

fn parse_legacy_json(bytes: &[u8]) -> Result<Prefs, String> {
    // Stand-in for an app's real JSON parser: `{"lang":"de","dark":true}`.
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    let mut prefs = Prefs::default();
    if text.contains("\"dark\":true") {
        prefs.appearance.theme = Theme::Dark;
    }
    if let Some(start) = text.find("\"lang\":\"") {
        let rest = &text[start + 8..];
        prefs.general.language = rest[..rest.find('"').ok_or("unterminated")?].to_owned();
    }
    Ok(prefs)
}

fn open_with_legacy(dir: &std::path::Path, files: Vec<LegacyFile<Prefs>>) -> (SettingsStore<Prefs>, craft_settings::LoadReport) {
    SettingsStore::open_with(
        OpenOptions::new(Location::Dir(dir.to_path_buf())).probe(empty_probe()).debounce(Duration::from_millis(100)).legacy(files),
    )
}

#[test]
fn legacy_file_is_imported_on_first_run_and_left_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("prefs.json");
    std::fs::write(&old, r#"{"lang":"de","dark":true}"#).unwrap();

    let (mut store, report) = open_with_legacy(tmp.path(), vec![LegacyFile { path: old.clone(), parse: parse_legacy_json }]);
    assert_eq!(report.source, Source::Legacy(old.clone()));
    assert_eq!(store.get().general.language, "de");
    assert_eq!(store.get().appearance.theme, Theme::Dark);
    assert!(store.is_dirty());
    store.flush().unwrap();

    assert!(old.exists(), "the old file is copied, not removed, so a rollback still works");
    assert_eq!(std::fs::read_to_string(&old).unwrap(), r#"{"lang":"de","dark":true}"#);
    assert!(read_config(tmp.path()).contains("language = \"de\""));
}

#[test]
fn legacy_import_is_skipped_once_config_exists() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("prefs.json");
    std::fs::write(&old, r#"{"lang":"de"}"#).unwrap();
    write_config(tmp.path(), "schema_version = 2\n[general]\nlanguage = \"it\"\n");
    let (store, report) = open_with_legacy(tmp.path(), vec![LegacyFile { path: old, parse: parse_legacy_json }]);
    assert_eq!(report.source, Source::File);
    assert_eq!(store.get().general.language, "it");
}

#[test]
fn first_legacy_file_that_parses_wins() {
    let tmp = tempfile::tempdir().unwrap();
    let missing = tmp.path().join("nope.json");
    let bad = tmp.path().join("bad.json");
    std::fs::write(&bad, [0xff, 0xfe]).unwrap();
    let good = tmp.path().join("good.json");
    std::fs::write(&good, r#"{"lang":"nl"}"#).unwrap();
    let files = vec![
        LegacyFile { path: missing, parse: parse_legacy_json },
        LegacyFile { path: bad, parse: parse_legacy_json },
        LegacyFile { path: good.clone(), parse: parse_legacy_json },
    ];
    let (store, report) = open_with_legacy(tmp.path(), files);
    assert_eq!(report.source, Source::Legacy(good));
    assert_eq!(store.get().general.language, "nl");
}
