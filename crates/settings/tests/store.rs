//! Integration tests of craft-settings: store.

use super::common;

use std::any::Any;
use std::io;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use common::{Prefs, Theme, config_path, empty_probe, open_in, open_keyed, read_config, write_config};
use craft_settings::store::{FIRST_RETRY_DELAY, MAX_RETRY_DELAY};
use craft_settings::testing::MemoryFiles;
use craft_settings::{Location, OpenOptions, SaveStatus, SettingsFiles, SettingsStore};

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

// #3773: words read from another file (a custom dictionary) count as loaded, not as this instance's edit, so a
// later save of another change does not write back an entry a second instance removed meanwhile.
#[test]
fn a_loaded_change_is_not_saved_over_another_instances_edit() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut first, _) = open_in(tmp.path());
    first.flush().unwrap();
    let (mut second, _) = open_in(tmp.path());

    let entry = "loaded".to_owned();
    assert!(first.update_loaded(|p| p.general.recent.push(entry.clone())));
    assert!(!first.is_dirty(), "a loaded change schedules no save");
    // The second instance loads the same entry, and its user removes it; the other change makes its file differ from
    // the one the first instance last read, so the first merges on its next save.
    second.update_loaded(|p| p.general.recent.push(entry.clone()));
    second.update(|p| {
        p.general.recent.retain(|e| *e != entry);
        p.general.autosave_minutes = 10;
    });
    second.flush().unwrap();

    first.update(|p| p.general.language = "fr".into());
    first.flush().unwrap();
    let (reloaded, _) = open_in(tmp.path());
    assert!(!reloaded.get().general.recent.contains(&entry), "{}", read_config(tmp.path()));
    assert_eq!((reloaded.get().general.language.as_str(), reloaded.get().general.autosave_minutes), ("fr", 10), "both saves merged");
}

/// Options for a `Prefs` store in `dir`, importing `legacy.txt` (its text is the language) when there is no
/// `config.toml`.
fn read_options(dir: &Path) -> OpenOptions<Prefs> {
    fn parse(bytes: &[u8]) -> Result<(Prefs, Vec<craft_settings::SectionError>), String> {
        let mut prefs = Prefs::default();
        prefs.general.language = String::from_utf8(bytes.to_vec()).map_err(|e| e.to_string())?;
        Ok((prefs, Vec::new()))
    }
    let legacy = craft_settings::LegacyFile { path: dir.join("legacy.txt"), parse };
    OpenOptions::new(Location::Dir(dir.to_path_buf())).probe(empty_probe()).legacy(vec![legacy])
}

#[test]
fn a_read_sees_what_a_store_wrote_and_touches_nothing_on_disk() {
    let tmp = tempfile::tempdir().unwrap();

    // A folder that does not exist reads as defaults and is not created.
    let missing = tmp.path().join("missing");
    let (prefs, _) = SettingsStore::<Prefs>::read(&read_options(&missing));
    assert_eq!(prefs, Prefs::default());
    assert!(!missing.exists(), "a read creates no folder");

    // With no config.toml the legacy file is imported in memory only.
    std::fs::write(tmp.path().join("legacy.txt"), "nl").unwrap();
    let (prefs, _) = SettingsStore::<Prefs>::read(&read_options(tmp.path()));
    assert_eq!(prefs.general.language, "nl");
    assert!(!config_path(tmp.path()).exists(), "a read writes no config.toml");

    // What an opened store saved is what a read sees, ahead of the legacy file.
    let (mut store, _) = SettingsStore::open_with(read_options(tmp.path()));
    store.update(|p| p.general.language = "de".into());
    store.flush().unwrap();
    let (prefs, _) = SettingsStore::<Prefs>::read(&read_options(tmp.path()));
    assert_eq!(prefs.general.language, "de");

    // A damaged config.toml reads as defaults and stays where it is, unchanged.
    let broken = "schema_version = 2\n[general\nlanguage = \"fr\"\n";
    write_config(tmp.path(), broken);
    let (prefs, report) = SettingsStore::<Prefs>::read(&read_options(tmp.path()));
    assert_eq!(prefs.general.language, Prefs::default().general.language);
    assert!(report.broken_moved_to.is_none());
    assert_eq!(report.damaged.as_deref(), Some(config_path(tmp.path()).as_path()), "a damaged file is named, not read as absent");
    assert!(report.has_problems());
    assert_eq!(read_config(tmp.path()), broken, "a read never moves or rewrites the file");
    let moved = std::fs::read_dir(tmp.path()).unwrap().flatten().any(|e| e.file_name().to_string_lossy().starts_with("config.broken-"));
    assert!(!moved, "nothing is moved aside");
}

/// A portable folder that is not writable: `read` loads the platform folder, as `open_with` does.
#[cfg(unix)]
#[test]
fn a_read_falls_back_to_the_platform_folder_like_an_open() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let exe = tmp.path().join("exe");
    let portable = exe.join("config");
    let platform = tmp.path().join("platform");
    std::fs::create_dir_all(&portable).unwrap();
    std::fs::create_dir_all(platform.join("testcraft")).unwrap();
    std::fs::write(exe.join("portable"), "").unwrap();
    write_config(&portable, "schema_version = 2\n[general]\nlanguage = \"portable\"\n");
    write_config(&platform.join("testcraft"), "schema_version = 2\n[general]\nlanguage = \"platform\"\n");
    let options = || {
        let probe = craft_settings::Probe { env: Box::new(|_| None), exe_dir: Some(exe.clone()), platform_config_dir: Some(platform.clone()) };
        OpenOptions::<Prefs>::new(Location::Auto).probe(probe)
    };
    let (prefs, _) = SettingsStore::<Prefs>::read(&options());
    assert_eq!(prefs.general.language, "portable");

    std::fs::set_permissions(&portable, std::fs::Permissions::from_mode(0o555)).unwrap();
    let (prefs, report) = SettingsStore::<Prefs>::read(&options());
    let (store, _) = SettingsStore::open_with(options());
    std::fs::set_permissions(&portable, std::fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(prefs.general.language, "platform");
    assert!(report.warning.is_some());
    assert_eq!(store.get().general.language, "platform", "an open takes the same folder");
}

/// A call the store made on its files.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Call {
    Lock,
    Read,
    Backup,
    Write,
    /// The lock's guard was dropped.
    Unlock,
}

type Calls = Arc<Mutex<Vec<Call>>>;

/// [`MemoryFiles`] that records the order of the store's calls, with a lock whose release is recorded too.
#[derive(Default)]
struct Recording {
    inner: MemoryFiles,
    calls: Calls,
}

impl Recording {
    fn record(&self, call: Call) {
        self.calls.lock().unwrap().push(call);
    }

    /// The calls since the last take.
    fn take(&self) -> Vec<Call> {
        std::mem::take(&mut *self.calls.lock().unwrap())
    }
}

struct Guard(Calls);

impl Drop for Guard {
    fn drop(&mut self) {
        self.0.lock().unwrap().push(Call::Unlock);
    }
}

impl SettingsFiles for Recording {
    fn read(&self, path: &Path) -> io::Result<Option<String>> {
        self.record(Call::Read);
        self.inner.read(path)
    }

    fn write(&self, path: &Path, text: &str) -> io::Result<()> {
        self.record(Call::Write);
        self.inner.write(path, text)
    }

    fn move_aside(&self, path: &Path) -> io::Result<PathBuf> {
        self.inner.move_aside(path)
    }

    fn write_backup(&self, path: &Path, text: &str) -> io::Result<()> {
        self.record(Call::Backup);
        self.inner.write_backup(path, text)
    }

    fn check_writable(&self, dir: &Path) -> io::Result<()> {
        self.inner.check_writable(dir)
    }

    fn lock(&self, path: &Path) -> Option<Box<dyn Any + Send>> {
        let _ = path;
        self.record(Call::Lock);
        Some(Box::new(Guard(self.calls.clone())))
    }
}

// SG-41: two processes saving at once must take turns from the disk check to the write, or the later one writes over
// what the earlier one just merged in.
#[test]
fn a_save_holds_the_files_lock_from_the_disk_check_to_the_write() {
    let dir = Path::new("/memory");
    let files = Arc::new(Recording::default());
    let options = OpenOptions::new(Location::Dir(PathBuf::from(dir))).probe(empty_probe()).files(files.clone());
    let (mut store, _) = SettingsStore::<Prefs>::open_with(options);
    files.take();

    // The first save: no file yet, so no backup.
    store.update(|p| p.general.language = "fr".into());
    store.flush().unwrap();
    assert_eq!(files.take(), [Call::Lock, Call::Read, Call::Write, Call::Unlock]);

    // A save over an existing file keeps its backup inside the lock too.
    store.update(|p| p.general.language = "de".into());
    store.flush().unwrap();
    assert_eq!(files.take(), [Call::Lock, Call::Read, Call::Backup, Call::Write, Call::Unlock]);

    // A save that fails partway releases the lock.
    files.inner.fail_writes(true);
    store.update(|p| p.general.language = "nl".into());
    assert!(store.flush().is_err());
    let calls = files.take();
    assert_eq!(calls.first(), Some(&Call::Lock), "{calls:?}");
    assert_eq!(calls.last(), Some(&Call::Unlock), "{calls:?}");
    assert!(!calls.contains(&Call::Write), "{calls:?}");
    files.inner.fail_writes(false);

    // A store with no file takes no lock.
    let options = OpenOptions::new(Location::Memory).probe(empty_probe()).files(files.clone());
    let (mut memory, _) = SettingsStore::<Prefs>::open_with(options);
    memory.update(|p| p.general.language = "fr".into());
    memory.flush().unwrap();
    assert!(!files.take().contains(&Call::Lock));

    // Nor does a read-only one: this file comes from a newer build.
    let newer = Path::new("/newer");
    files.inner.write(&config_path(newer), "schema_version = 99\n").unwrap();
    let options = OpenOptions::new(Location::Dir(PathBuf::from(newer))).probe(empty_probe()).files(files.clone());
    let (mut read_only, _) = SettingsStore::<Prefs>::open_with(options);
    assert!(read_only.is_read_only());
    files.take();
    read_only.update(|p| p.general.language = "fr".into());
    assert!(read_only.flush().is_err());
    assert!(!files.take().contains(&Call::Lock));
}
