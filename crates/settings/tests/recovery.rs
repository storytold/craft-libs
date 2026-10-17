mod common;

use common::{Theme, config_path, open_in, read_config, write_config};
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
fn two_broken_files_get_different_names() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "not = [toml");
    let (_, first) = open_in(tmp.path());
    write_config(tmp.path(), "still = [broken");
    let (_, second) = open_in(tmp.path());
    assert_ne!(first.broken_moved_to, second.broken_moved_to);
    assert!(second.broken_moved_to.unwrap().exists());
}

#[test]
fn bad_value_only_resets_its_section() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n[general]\nlanguage = \"de\"\n\n[appearance]\ntheme = \"sepia\"\nui_scale = 2.0\n");
    let (store, report) = open_in(tmp.path());
    assert_eq!(report.source, Source::File);
    assert!(report.broken_moved_to.is_none());
    assert!(!store.is_read_only());
    assert_eq!(report.section_errors.len(), 1, "{report:?}");
    assert_eq!(report.section_errors[0].section, "appearance");
    assert_eq!(store.get().general.language, "de", "the good section survives");
    assert_eq!(store.get().appearance.theme, Theme::Light, "the bad section falls back to defaults");
    assert!(config_path(tmp.path()).exists(), "the file is not moved aside");
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

#[test]
fn failed_migration_opens_read_only_and_leaves_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    let original = "schema_version = 0\n[general]\nlanguage = \"de\"\n";
    write_config(tmp.path(), original);
    let (mut store, report) = open_in(tmp.path());
    assert!(matches!(report.read_only, Some(ReadOnlyReason::Migration(_))), "{report:?}");
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert_eq!(read_config(tmp.path()), original);
}

#[cfg(unix)]
#[test]
fn damaged_file_that_cannot_be_moved_is_left_alone() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "not = [toml");
    let dir = tmp.path();
    if !common::restrict_directory_writes(dir) {
        return;
    }
    let (mut store, report) = open_in(dir);
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store.is_read_only(), "{report:?}");
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert_eq!(read_config(dir), "not = [toml");
}

// --- Review on #22: the backup must never be replaced by a damaged or emptied file. ---

#[test]
fn file_damaged_while_running_is_moved_aside_and_backup_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    store.update(|p| p.general.language = "es".into());
    store.flush().unwrap();
    let good_backup = std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap();
    assert!(good_backup.contains("language = \"fr\""));

    // A hand edit with a typo while the app is running.
    write_config(tmp.path(), "schema_version = 2\n[general\nlanguage = \"it\"\n");
    store.update(|p| p.general.language = "pt".into());
    store.flush().unwrap();

    assert_eq!(std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(), good_backup, "backup untouched");
    let moved = store.take_moved_aside();
    assert_eq!(moved.len(), 1);
    assert_eq!(std::fs::read_to_string(&moved[0]).unwrap(), "schema_version = 2\n[general\nlanguage = \"it\"\n", "the user's edit is kept");
    assert!(read_config(tmp.path()).contains("language = \"pt\""));
    assert!(store.take_moved_aside().is_empty(), "drained");
}

#[test]
fn emptied_file_does_not_replace_the_backup() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    store.update(|p| p.general.language = "es".into());
    store.flush().unwrap();
    let good_backup = std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap();

    write_config(tmp.path(), "");
    store.update(|p| p.general.language = "pt".into());
    store.flush().unwrap();
    assert_eq!(std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(), good_backup);
    assert!(read_config(tmp.path()).contains("language = \"pt\""));
}

#[test]
fn newer_build_writing_while_running_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let newer = "schema_version = 9\n[general]\nlanguage = \"de\"\n";
    write_config(tmp.path(), newer);
    store.update(|p| p.general.language = "fr".into());
    assert!(matches!(store.flush(), Err(SettingsError::ReadOnly(_))));
    assert!(store.is_read_only());
    assert_eq!(read_config(tmp.path()), newer);
}

// --- Review on #22: a failed migration must not leave half-migrated values in memory. ---

#[cfg(unix)]
#[test]
fn failed_migration_in_an_unwritable_folder_shows_defaults() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    // Version 0 has no migration step, so upgrading fails.
    write_config(tmp.path(), "schema_version = 0\n[general]\nlanguage = \"de\"\n");
    if !common::restrict_directory_writes(tmp.path()) {
        return;
    }
    let (store, report) = open_in(tmp.path());
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(matches!(report.read_only, Some(ReadOnlyReason::DirNotWritable(_))), "{report:?}");
    assert_eq!(store.get().general.language, "en", "defaults, not half-migrated values");
    assert_eq!(report.source, Source::Defaults);
}

// --- Review on #22 (skylartaylor): an outside file whose migration fails must not be overwritten. ---

#[test]
fn outside_file_with_failing_migration_is_not_overwritten() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    // Another writer leaves a file at a version this build cannot migrate (0 has no step).
    let external = "schema_version = 0\n[general]\nlanguage = 'external'\n";
    write_config(tmp.path(), external);

    store.update(|p| p.general.language = "local".into());
    let result = store.flush();
    assert!(matches!(result, Err(SettingsError::ReadOnly(_))), "{result:?}");
    assert!(store.is_read_only());
    assert_eq!(read_config(tmp.path()), external, "the outside file is left untouched");
    assert!(matches!(store.tick(std::time::Instant::now()).status, craft_settings::SaveStatus::Idle), "no retries once read-only");
}
