use std::collections::BTreeMap;
use std::fs;
use std::time::{Duration, Instant};

use craft_settings::{DocumentMut, Location, OpenOptions, SaveStatus, Settings, SettingsError, SettingsStore};
use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
struct Prefs {
    labels: BTreeMap<String, String>,
    language: String,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "testcraft";
    const SCHEMA_VERSION: u32 = 1;
}

fn remove_label(doc: &mut DocumentMut, key: &str) {
    if let Some(labels) = doc.get_mut("labels").and_then(|item| item.as_table_like_mut()) {
        labels.remove(key);
    }
}

#[test]
fn removes_only_the_named_map_key_from_the_latest_document() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    fs::write(&config, "schema_version = 1\n[labels]\nchosen = 'old'\npeer = 'known'\n").unwrap();
    let (mut store, _) = SettingsStore::<Prefs>::open_at(tmp.path());
    let hand_edit = "schema_version = 1\n# added by hand\n[labels]\nchosen = 'changed'\npeer = 'outside' # note\nmanual = 'kept' # manual\n\n[future]\nenabled = true\n";
    fs::write(&config, hand_edit).unwrap();

    store
        .update_and_flush_with(
            |prefs| {
                prefs.labels.remove("chosen");
            },
            |doc| remove_label(doc, "chosen"),
        )
        .unwrap();

    let text = fs::read_to_string(&config).unwrap();
    assert!(!text.contains("chosen"), "{text}");
    assert!(text.contains("# added by hand"), "{text}");
    assert!(text.contains("peer = \"known\" # note"), "{text}");
    assert!(text.contains("manual = 'kept' # manual"), "{text}");
    assert!(text.contains("[future]\nenabled = true"), "{text}");
    assert_eq!(fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(), hand_edit);
    assert_eq!(store.get().labels, BTreeMap::from([("peer".to_owned(), "known".to_owned())]));
    assert!(!store.is_dirty());

    store.update(|prefs| prefs.language = "fr".to_owned());
    store.flush().unwrap();
    let (reopened, _) = SettingsStore::<Prefs>::open_at(tmp.path());
    assert!(!reopened.get().labels.contains_key("chosen"));
    assert_eq!(reopened.get().labels.get("manual").map(String::as_str), Some("kept"));
}

#[test]
fn can_remove_a_key_from_an_inline_map() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    fs::write(&config, "schema_version = 1\nlabels = { chosen = 'old', peer = 'kept' } # inline note\n").unwrap();
    let (mut store, _) = SettingsStore::<Prefs>::open_at(tmp.path());
    store
        .update_and_flush_with(
            |prefs| {
                prefs.labels.remove("chosen");
            },
            |doc| remove_label(doc, "chosen"),
        )
        .unwrap();
    let text = fs::read_to_string(config).unwrap();
    assert!(!text.contains("chosen"), "{text}");
    assert!(text.contains("# inline note"), "{text}");
    let (reopened, _) = SettingsStore::<Prefs>::open_at(tmp.path());
    assert_eq!(reopened.get().labels, BTreeMap::from([("peer".to_owned(), "kept".to_owned())]));
}

#[test]
fn can_reset_a_scalar_or_large_map_without_removing_unrelated_keys() {
    let large = (0..128).map(|i| format!("label{i} = 'value'\n")).collect::<String>();
    for labels in ["labels = 'wrong shape'\n".to_owned(), format!("[labels]\n{large}")] {
        let tmp = tempfile::tempdir().unwrap();
        let config = tmp.path().join("config.toml");
        let original = format!("schema_version = 1\nlanguage = 'en'\n{labels}\n# unrelated\n[future]\nx = 7\n");
        fs::write(&config, &original).unwrap();
        let (mut store, _) = SettingsStore::<Prefs>::open_at(tmp.path());

        store
            .update_and_flush_with(
                |prefs| prefs.labels.clear(),
                |doc| {
                    doc.remove("labels");
                },
            )
            .unwrap();

        let text = fs::read_to_string(&config).unwrap();
        let saved: DocumentMut = text.parse().unwrap();
        assert!(saved.get("labels").is_none(), "{text}");
        assert!(text.contains("# unrelated\n[future]\nx = 7"), "{text}");
        assert_eq!(fs::read_to_string(tmp.path().join("config.toml.bak")).unwrap(), original);
        assert!(store.get().labels.is_empty());
    }
}

#[test]
fn failed_write_discards_both_candidates_and_preserves_pending_work() {
    let tmp = tempfile::tempdir().unwrap();
    let config = tmp.path().join("config.toml");
    fs::write(&config, "schema_version = 1\n# originally loaded\n[labels]\nchosen = 'old'\n").unwrap();
    let debounce = Duration::from_secs(1);
    let (mut store, _) = SettingsStore::<Prefs>::open_with(OpenOptions::new(Location::Dir(tmp.path().to_path_buf())).debounce(debounce));
    let now = Instant::now();
    assert!(store.update_at(now, |prefs| prefs.language = "pending".to_owned()));
    let hand_edit = "schema_version = 1\n# outside edit\n[labels]\nchosen = 'new'\n";
    fs::write(&config, hand_edit).unwrap();

    let mode = fs::metadata(&config).unwrap().permissions();
    let mut read_only = mode.clone();
    read_only.set_readonly(true);
    fs::set_permissions(&config, read_only).unwrap();
    if fs::File::options().write(true).open(&config).is_ok() {
        fs::set_permissions(&config, mode).unwrap();
        eprintln!("skipping permission test: environment permits writes into a read-only file");
        return;
    }
    let mut edited = false;
    let result = store.update_and_flush_with(
        |prefs| {
            prefs.labels.clear();
            prefs.language = "transaction".to_owned();
        },
        |doc| {
            edited = true;
            doc.remove("labels");
        },
    );
    fs::set_permissions(&config, mode).unwrap();

    assert!(edited, "the failure must happen after the document callback");
    assert!(matches!(result, Err(SettingsError::Io(_))), "{result:?}");
    assert_eq!(fs::read_to_string(&config).unwrap(), hand_edit);
    assert_eq!(store.get().labels.get("chosen").map(String::as_str), Some("old"));
    assert_eq!(store.get().language, "pending");
    assert!(store.is_dirty());
    let tick = store.tick(now);
    assert!(matches!(tick.status, SaveStatus::Pending));
    assert_eq!(tick.next_deadline, Some(now + debounce));

    // A blank file makes a regular save reuse the last committed document.
    fs::write(&config, "").unwrap();
    store.flush().unwrap();
    let text = fs::read_to_string(&config).unwrap();
    assert!(text.contains("# originally loaded"), "{text}");
    assert!(!text.contains("# outside edit"), "{text}");
    assert!(text.contains("language = \"pending\""), "{text}");
    assert!(text.contains("chosen = \"old\""), "{text}");
}

#[test]
fn memory_store_applies_only_the_typed_update() {
    let (mut store, _) = SettingsStore::<Prefs>::open(Location::Memory);
    store.update_and_flush_with(|prefs| prefs.language = "en".to_owned(), |_| panic!("no document callback in a memory store")).unwrap();
    assert_eq!(store.get().language, "en");
    assert!(!store.is_dirty());
}
