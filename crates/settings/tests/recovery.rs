//! Integration tests of craft-settings: recovery.

use super::common;

use common::{config_path, open_in, open_keyed, read_config, write_config};
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

/// Pads `config.toml` past the read limit; the file is sparse where the file system allows it.
fn grow_past_limit(dir: &std::path::Path) -> u64 {
    let len = craft_settings::store::MAX_CONFIG_BYTES + 1;
    std::fs::OpenOptions::new().write(true).open(config_path(dir)).unwrap().set_len(len).unwrap();
    len
}

#[test]
fn oversized_file_opens_read_only_and_is_left_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n[general]\nlanguage = \"de\"\n");
    let len = grow_past_limit(tmp.path());
    let (mut store, report) = open_in(tmp.path());
    assert!(matches!(&report.read_only, Some(ReadOnlyReason::Unreadable(e)) if e.kind() == std::io::ErrorKind::FileTooLarge), "{report:?}");
    assert!(report.broken_moved_to.is_none(), "{report:?}");
    assert_eq!(report.source, Source::Defaults);
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert_eq!(std::fs::metadata(config_path(tmp.path())).unwrap().len(), len, "the file is untouched");
}

#[test]
fn file_grown_past_the_limit_while_running_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let len = grow_past_limit(tmp.path());

    store.update(|p| p.general.language = "fr".into());
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert!(store.is_read_only());
    assert!(store.take_moved_aside().is_empty());
    assert_eq!(std::fs::metadata(config_path(tmp.path())).unwrap().len(), len, "the file is untouched");
    assert!(matches!(store.tick(std::time::Instant::now()).status, craft_settings::SaveStatus::Idle), "no retries once read-only");
}

#[test]
fn settings_too_large_to_read_back_are_not_written() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let before = read_config(tmp.path());

    let huge = "a".repeat(craft_settings::store::MAX_CONFIG_BYTES as usize);
    store.update(|p| p.general.language = huge);
    assert!(matches!(store.flush(), Err(SettingsError::Io(e)) if e.kind() == std::io::ErrorKind::FileTooLarge));
    assert_eq!(read_config(tmp.path()), before, "the file is untouched");
    assert!(!store.is_read_only());
    assert!(store.is_dirty(), "the change is kept for a later save");

    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    assert!(read_config(tmp.path()).contains("language = \"fr\""));
}

#[test]
fn settings_too_large_to_save_leave_a_broken_file_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    write_config(tmp.path(), "not = [toml");

    let huge = "a".repeat(craft_settings::store::MAX_CONFIG_BYTES as usize);
    store.update(|p| p.general.language = huge);
    assert!(matches!(store.flush(), Err(SettingsError::Io(e)) if e.kind() == std::io::ErrorKind::FileTooLarge));
    assert_eq!(read_config(tmp.path()), "not = [toml", "the file is untouched");
    assert!(store.take_moved_aside().is_empty());
}

#[test]
fn rejected_save_keeps_the_adopted_file_as_the_baseline() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n\n# note a\n[plugins]\nenabled = true\n");
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let first = read_config(tmp.path());

    // Edited outside the app, then a save too large to write adopts the edit and fails.
    write_config(tmp.path(), &format!("{first}\n# note b\n[extra]\nkept = true\n"));
    let huge = "a".repeat(craft_settings::store::MAX_CONFIG_BYTES as usize);
    store.update(|p| p.general.language = huge);
    assert!(matches!(store.flush(), Err(SettingsError::Io(e)) if e.kind() == std::io::ErrorKind::FileTooLarge));

    // The outside edit is undone before the next save.
    write_config(tmp.path(), &first);
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();

    let text = read_config(tmp.path());
    assert!(text.contains("# note a"), "{text}");
    assert!(!text.contains("# note b"), "{text}");
    assert!(!text.contains("[extra]"), "{text}");
    assert!(text.contains("language = \"fr\""), "{text}");
}

#[test]
fn rejected_save_keeps_the_comment_of_a_removed_key() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 1\n\n[shortcuts]\n# note on save\nsave = \"Ctrl+S\"\n");
    let (mut store, _) = open_keyed(tmp.path());

    // The key is removed in a save too large to write, then put back.
    let huge = "a".repeat(craft_settings::store::MAX_CONFIG_BYTES as usize);
    store.update(|k| {
        k.shortcuts.remove("save");
        k.general.language = huge;
    });
    assert!(matches!(store.flush(), Err(SettingsError::Io(e)) if e.kind() == std::io::ErrorKind::FileTooLarge));
    store.update(|k| {
        k.shortcuts.insert("save".into(), "Ctrl+S".into());
        k.general.language = "fr".into();
    });
    store.flush().unwrap();

    let text = read_config(tmp.path());
    assert!(text.contains("# note on save\nsave = \"Ctrl+S\""), "{text}");
    assert!(text.contains("language = \"fr\""), "{text}");
}

#[test]
fn section_error_displays_its_section_and_message() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n[appearance]\ntheme = \"sepia\"\n");
    let (_, report) = open_in(tmp.path());
    let error = &report.section_errors[0];
    let text = error.to_string();
    assert!(text.starts_with("settings section [appearance.theme] was reset to defaults: "), "{text}");
    assert!(text.ends_with(&error.message), "{text}");
    let source: &dyn std::error::Error = error;
    assert!(source.source().is_none());
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
