mod common;

use std::time::{Duration, Instant};

use common::{Theme, config_path, open_in, read_config, write_config};
use craft_settings::{SaveStatus, Source};

#[test]
fn first_run_gives_defaults_and_writes_the_file_on_flush() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, report) = open_in(tmp.path());
    assert_eq!(report.source, Source::Defaults);
    assert_eq!(store.get().general.language, "en");
    assert!(!config_path(tmp.path()).exists());

    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.starts_with("schema_version = 2\n"), "{text}");
    assert!(text.contains("[general]"), "{text}");
    assert!(text.contains("[appearance]"), "{text}");
}

#[test]
fn round_trip() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    assert!(store.update(|p| {
        p.appearance.theme = Theme::Dark;
        p.general.recent = vec!["a.craft".into(), "b.craft".into()];
    }));
    store.flush().unwrap();

    let (store, report) = open_in(tmp.path());
    assert_eq!(report.source, Source::File);
    assert_eq!(store.get().appearance.theme, Theme::Dark);
    assert_eq!(store.get().general.recent, vec!["a.craft", "b.craft"]);
}

#[test]
fn comments_unknown_keys_and_order_survive_a_save() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(
        tmp.path(),
        "schema_version = 2\n\n# my notes\n[appearance]\nui_scale = 1.5 # big\nfuture_key = \"kept\"\n\n[general]\nlanguage = \"de\"\n\n[plugins]\nenabled = true\n",
    );
    let (mut store, report) = open_in(tmp.path());
    assert!(report.section_errors.is_empty(), "{report:?}");
    assert_eq!(store.get().general.language, "de");
    store.update(|p| p.appearance.theme = Theme::Dark);
    store.flush().unwrap();

    let text = read_config(tmp.path());
    assert!(text.contains("# my notes"), "{text}");
    assert!(text.contains("ui_scale = 1.5 # big"), "{text}");
    assert!(text.contains("future_key = \"kept\""), "{text}");
    assert!(text.contains("[plugins]\nenabled = true"), "{text}");
    assert!(text.contains("theme = \"dark\""), "{text}");
    assert!(text.find("[appearance]").unwrap() < text.find("[general]").unwrap(), "section order changed:\n{text}");
}

#[test]
fn arrays_are_replaced_whole() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 2\n[general]\nrecent = [\"old1\", \"old2\", \"old3\"]\n");
    let (mut store, _) = open_in(tmp.path());
    store.update(|p| p.general.recent = vec!["new".into()]);
    store.flush().unwrap();
    assert!(read_config(tmp.path()).contains("recent = [\"new\"]"));
}

#[test]
fn update_with_no_change_is_not_dirty() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    assert!(!store.update(|p| p.general.autosave_minutes = 5));
    assert!(!store.is_dirty());
    assert!(store.update(|p| p.general.autosave_minutes = 6));
    assert!(store.is_dirty());
}

#[test]
fn tick_waits_for_the_debounce_and_reports_the_deadline() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    let t0 = Instant::now();
    assert!(matches!(store.tick(t0).status, SaveStatus::Idle));

    store.update_at(t0, |p| p.appearance.theme = Theme::Dark);
    let r = store.tick(t0);
    assert!(matches!(r.status, SaveStatus::Pending));
    assert_eq!(r.next_deadline, Some(t0 + Duration::from_millis(100)));
    assert!(!config_path(tmp.path()).exists());

    let r = store.tick(t0 + Duration::from_millis(100));
    assert!(matches!(r.status, SaveStatus::Saved), "{r:?}");
    assert_eq!(r.next_deadline, None);
    assert!(read_config(tmp.path()).contains("theme = \"dark\""));
    assert!(matches!(store.tick(t0 + Duration::from_millis(200)).status, SaveStatus::Idle));
}

#[test]
fn further_changes_push_the_save_back() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    let t0 = Instant::now();
    store.update_at(t0, |p| p.general.autosave_minutes = 1);
    store.update_at(t0 + Duration::from_millis(80), |p| p.general.autosave_minutes = 2);
    let r = store.tick(t0 + Duration::from_millis(100));
    assert!(matches!(r.status, SaveStatus::Pending));
    assert_eq!(r.next_deadline, Some(t0 + Duration::from_millis(180)));
}

#[test]
fn failed_save_is_retried_with_backoff() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    // A folder where the file should be makes the write fail.
    std::fs::create_dir(config_path(tmp.path())).unwrap();
    let t0 = Instant::now();
    store.update_at(t0, |p| p.appearance.theme = Theme::Dark);

    let r = store.tick(t0 + Duration::from_millis(100));
    assert!(matches!(r.status, SaveStatus::Failed(_)), "{r:?}");
    let first_retry = t0 + Duration::from_millis(100) + Duration::from_secs(1);
    assert_eq!(r.next_deadline, Some(first_retry));
    assert!(matches!(store.tick(first_retry - Duration::from_millis(1)).status, SaveStatus::Pending));

    let r = store.tick(first_retry);
    assert!(matches!(r.status, SaveStatus::Failed(_)));
    assert_eq!(r.next_deadline, Some(first_retry + Duration::from_secs(2)));

    std::fs::remove_dir(config_path(tmp.path())).unwrap();
    let r = store.tick(first_retry + Duration::from_secs(2));
    assert!(matches!(r.status, SaveStatus::Saved), "{r:?}");
    assert!(read_config(tmp.path()).contains("theme = \"dark\""));
}

#[test]
fn retry_delay_doubles_and_caps() {
    use craft_settings::store::retry_delay;
    assert_eq!(retry_delay(1), Duration::from_secs(1));
    assert_eq!(retry_delay(2), Duration::from_secs(2));
    assert_eq!(retry_delay(3), Duration::from_secs(4));
    assert_eq!(retry_delay(6), Duration::from_secs(30));
    assert_eq!(retry_delay(40), Duration::from_secs(30));
}

#[test]
fn backup_holds_the_previous_version() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    store.update(|p| p.general.language = "es".into());
    store.flush().unwrap();
    let bak = std::fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap();
    assert!(bak.contains("language = \"fr\""), "{bak}");
    assert!(read_config(tmp.path()).contains("language = \"es\""));
}

#[test]
fn outside_edits_between_load_and_save_are_kept() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    store.update(|p| p.general.language = "fr".into());

    // Another instance (or the user) edits the file meanwhile.
    std::thread::sleep(Duration::from_millis(20));
    let edited = read_config(tmp.path()).replace("[general]", "# added by hand\n[other]\nx = 1\n\n[general]");
    write_config(tmp.path(), &edited);
    let later = std::time::SystemTime::now() + Duration::from_secs(2);
    std::fs::File::options().write(true).open(config_path(tmp.path())).unwrap().set_modified(later).unwrap();

    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.contains("# added by hand"), "{text}");
    assert!(text.contains("[other]\nx = 1"), "{text}");
    assert!(text.contains("language = \"fr\""), "{text}");
}

#[cfg(unix)]
#[test]
fn new_file_is_owner_only_and_keeps_its_mode_on_replace() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let mode = std::fs::metadata(config_path(tmp.path())).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);

    std::fs::set_permissions(config_path(tmp.path()), std::fs::Permissions::from_mode(0o644)).unwrap();
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    let mode = std::fs::metadata(config_path(tmp.path())).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o644, "a user's deliberate mode change is kept");
}

#[cfg(unix)]
#[test]
fn read_only_file_makes_the_save_fail_visibly() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    std::fs::set_permissions(config_path(tmp.path()), std::fs::Permissions::from_mode(0o400)).unwrap();
    store.update(|p| p.general.language = "fr".into());
    let r = store.tick(Instant::now() + Duration::from_secs(1));
    assert!(matches!(r.status, SaveStatus::Failed(craft_settings::SettingsError::Io(_))), "{r:?}");
    assert!(!read_config(tmp.path()).contains("language = \"fr\""));
}

#[test]
fn outside_edit_with_the_same_mtime_is_still_seen() {
    // Coarse timestamps (FAT, HFS+, network shares) can leave mtime unchanged after an edit.
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    let mtime = std::fs::metadata(config_path(tmp.path())).unwrap().modified().unwrap();

    let edited = read_config(tmp.path()).replace("[general]", "# same second\n[general]");
    write_config(tmp.path(), &edited);
    std::fs::File::options().write(true).open(config_path(tmp.path())).unwrap().set_modified(mtime).unwrap();

    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    let text = read_config(tmp.path());
    assert!(text.contains("# same second"), "{text}");
    assert!(text.contains("language = \"fr\""), "{text}");
}

// --- Review on #22 (skylartaylor): the backup must never be more readable than config.toml. ---

#[cfg(unix)]
fn mode(path: &std::path::Path) -> u32 {
    use std::os::unix::fs::PermissionsExt;
    std::fs::metadata(path).unwrap().permissions().mode() & 0o777
}

#[cfg(unix)]
#[test]
fn new_backup_is_as_private_as_the_config() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    store.update(|p| p.general.language = "private".into());
    store.flush().unwrap();
    assert_eq!(mode(&config_path(tmp.path())), 0o600);
    assert_eq!(mode(&tmp.path().join("config.toml.bak")), 0o600);
}

#[cfg(unix)]
#[test]
fn existing_broader_backup_is_tightened() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    std::fs::set_permissions(tmp.path(), std::fs::Permissions::from_mode(0o755)).unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    // A backup left behind by an older build, readable by everyone.
    let bak = tmp.path().join("config.toml.bak");
    std::fs::write(&bak, b"old backup").unwrap();
    std::fs::set_permissions(&bak, std::fs::Permissions::from_mode(0o664)).unwrap();

    store.update(|p| p.general.language = "private".into());
    store.flush().unwrap();
    assert_eq!(mode(&bak), mode(&config_path(tmp.path())), "backup must not be more readable than config.toml");
    assert_eq!(mode(&bak), 0o600);
}

#[cfg(unix)]
#[test]
fn backup_follows_a_mode_the_user_chose_for_the_config() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, _) = open_in(tmp.path());
    store.flush().unwrap();
    std::fs::set_permissions(config_path(tmp.path()), std::fs::Permissions::from_mode(0o640)).unwrap();
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    assert_eq!(mode(&tmp.path().join("config.toml.bak")), 0o640);
}
