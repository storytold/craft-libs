//! Integration tests of craft-settings: recovery.

use super::common;

use common::{config_path, open_in, read_config, write_config};
use craft_settings::{ReadOnlyReason, SettingsError, Source};

#[test]
fn syntax_error_moves_the_file_aside_and_uses_defaults() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n[general\nlanguage = \"de\"\n");
    let (mut store, report) = open_in(tmp.path());
    assert_eq!(report.source, Source::Defaults);
    assert!(!store.is_read_only());
    let moved = report.broken_moved_to.clone().expect("moved aside");
    assert!(moved.file_name().unwrap().to_str().unwrap().starts_with("config.broken-"), "{moved:?}");
    assert!(moved.exists());
    assert!(!config_path(tmp.path()).exists());
    assert_eq!(store.get().general.language, "en");

    store.flush().unwrap();
    assert!(moved.exists(), "the damaged copy is never deleted");
    assert!(read_config(tmp.path()).starts_with("schema_version = 2\n"));
}

#[test]
fn newer_schema_opens_read_only_and_never_overwrites() {
    let tmp = tempfile::tempdir().unwrap();
    let original = "schema_version = 9\n[general]\nlanguage = \"de\"\n";
    write_config(tmp.path(), original);
    let (mut store, report) = open_in(tmp.path());
    assert!(matches!(report.read_only, Some(ReadOnlyReason::NewerSchema { file: 9, app: 2 })), "{report:?}");
    assert_eq!(store.get().general.language, "de", "values are still shown");
    store.update(|p| p.general.language = "fr".into());
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert_eq!(read_config(tmp.path()), original);
}
