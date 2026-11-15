//! Integration tests of craft-settings: store.

use super::common;

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use common::{Prefs, Theme, config_path, empty_probe, open_in, open_keyed, read_config, write_config};
use craft_settings::store::{FIRST_RETRY_DELAY, MAX_RETRY_DELAY};
use craft_settings::testing::MemoryFiles;
use craft_settings::{Location, OpenOptions, SaveStatus, SettingsStore};

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

#[test]
fn a_failing_save_backs_off_and_succeeds_without_another_edit() {
    let debounce = Duration::from_millis(100);
    let dir = Path::new("/memory");
    let files = Arc::new(MemoryFiles::new());
    files.fail_writes(true);
    let options = OpenOptions::new(Location::Dir(PathBuf::from(dir))).probe(empty_probe()).debounce(debounce).files(files.clone());
    let (mut store, _) = SettingsStore::<Prefs>::open_with(options);
    let start = Instant::now();
    assert!(store.update_at(start, |p| p.general.language = "fr".into()));

    // Before the debounce passes nothing is attempted.
    assert!(matches!(store.tick(start).status, SaveStatus::Pending));

    // Each failure waits twice as long as the one before, up to the cap.
    let mut now = start + debounce;
    let mut delays = Vec::new();
    for _ in 0..7 {
        let result = store.tick(now);
        assert!(matches!(result.status, SaveStatus::Failed(_)), "{:?}", result.status);
        let deadline = result.next_deadline.expect("a retry is scheduled");
        delays.push(deadline - now);
        // Asking again before the deadline does not write.
        assert!(matches!(store.tick(now).status, SaveStatus::Pending));
        now = deadline;
    }
    let expected: Vec<Duration> = [1u32, 2, 4, 8, 16, 32, 64].into_iter().map(|s| (FIRST_RETRY_DELAY * s).min(MAX_RETRY_DELAY)).collect();
    assert_eq!(delays, expected);
    assert!(files.text(&config_path(dir)).is_none());

    // Storage works again: the next tick at the deadline saves the pending change, with no new edit.
    files.fail_writes(false);
    let result = store.tick(now);
    assert!(matches!(result.status, SaveStatus::Saved), "{:?}", result.status);
    assert!(result.next_deadline.is_none());
    assert!(!store.is_dirty());
    assert!(files.text(&config_path(dir)).as_deref().is_some_and(|text| text.contains("language = \"fr\"")));

    // The retry state is cleared: a later edit saves after the debounce, not after the old backoff.
    assert!(store.update_at(now, |p| p.general.language = "de".into()));
    assert!(matches!(store.tick(now + debounce).status, SaveStatus::Saved));
}

// #6611: one value of the wrong type resets only itself, in a section and in a map, and the file is saved again
// with the default in its place, so the next launch loads cleanly.
#[test]
fn a_wrongly_typed_value_resets_only_itself_and_is_repaired_on_disk() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(
        tmp.path(),
        "schema_version = 1\n[general]\nlanguage = \"de\"\nautosave_minutes = \"ten\"\n[shortcuts]\n\"edit.fill\" = \"Cmd+F\"\n\"file.open\" = 7\n[future]\nkept = true\n",
    );
    let (mut store, report) = open_keyed(tmp.path());
    let reset: Vec<&str> = report.section_errors.iter().map(|e| e.section.as_str()).collect();
    assert_eq!(reset, ["general.autosave_minutes", "shortcuts.file.open"], "{report:?}");
    assert_eq!((store.get().general.language.as_str(), store.get().general.autosave_minutes), ("de", 5));
    assert_eq!(store.get().shortcuts.get("edit.fill").map(String::as_str), Some("Cmd+F"));
    assert!(store.is_dirty(), "the repair is saved without another edit");
    store.flush().unwrap();

    let text = read_config(tmp.path());
    assert!(text.contains("autosave_minutes = 5"), "{text}");
    assert!(!text.contains("file.open"), "{text}");
    assert!(text.contains("\"edit.fill\" = \"Cmd+F\""), "{text}");
    assert!(text.contains("[future]\nkept = true"), "{text}");
    let (_, report) = open_keyed(tmp.path());
    assert!(report.section_errors.is_empty(), "{report:?}");
}

// An entry the app takes out of a map leaves the file too; keys the type doesn't know stay.
#[test]
fn a_map_entry_the_app_removes_is_removed_from_the_file() {
    let tmp = tempfile::tempdir().unwrap();
    write_config(tmp.path(), "schema_version = 1\n[shortcuts]\na = \"A\"\nb = \"B\"\n[future]\nkept = true\n");
    let (mut store, _) = open_keyed(tmp.path());
    store.update(|p| {
        p.shortcuts.remove("a");
    });
    store.flush().unwrap();
    let (mut reopened, _) = open_keyed(tmp.path());
    assert_eq!(reopened.get().shortcuts.keys().collect::<Vec<_>>(), ["b"], "{}", read_config(tmp.path()));
    assert!(read_config(tmp.path()).contains("[future]\nkept = true"));

    reopened.update(|p| p.shortcuts.clear());
    reopened.flush().unwrap();
    let (last, _) = open_keyed(tmp.path());
    assert!(last.get().shortcuts.is_empty(), "{}", read_config(tmp.path()));
}
