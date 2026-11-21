//! Integration tests of craft-settings: the store over the browser's localStorage.

#![cfg(target_arch = "wasm32")]

use std::path::{Path, PathBuf};
use std::sync::Arc;

use craft_settings::{LegacyFile, LocalStorageFiles, Location, OpenOptions, SectionError, Settings, SettingsFiles, SettingsStore, Source};
use serde::{Deserialize, Serialize};
use wasm_bindgen_test::{wasm_bindgen_test, wasm_bindgen_test_configure};

wasm_bindgen_test_configure!(run_in_browser);

/// The byte cap the trial's storage enforces: room for a small file, not for [`Web::note`] at [`CAP`] bytes.
const CAP: usize = 256;

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
struct Web {
    dark: bool,
    note: String,
}

impl Settings for Web {
    const APP_ID: &'static str = "webcraft";
    const SCHEMA_VERSION: u32 = 1;
}

fn options(dir: &str) -> OpenOptions<Web> {
    OpenOptions::new(Location::Dir(PathBuf::from(dir))).files(Arc::new(LocalStorageFiles::new(CAP)))
}

/// The old item's text is the theme name.
fn parse_legacy(bytes: &[u8]) -> Result<(Web, Vec<SectionError>), String> {
    let text = std::str::from_utf8(bytes).map_err(|e| e.to_string())?;
    Ok((Web { dark: text == "dark", note: String::new() }, Vec::new()))
}

// Settings on the web live in localStorage: a reload must see the last save, an oversized save must not destroy the
// saved settings, and the old hand-written item must carry over once.
#[wasm_bindgen_test]
fn a_store_over_local_storage_round_trips_refuses_over_the_cap_and_imports_the_legacy_item() {
    let storage = web_sys::window().unwrap().local_storage().unwrap().unwrap();
    for key in ["/web/config.toml", "/web/config.toml.bak", "/fresh/config.toml", "wordcraft.ui"] {
        storage.remove_item(key).unwrap();
    }

    // A change saved by `flush` is what the next page load reads.
    let (mut store, _) = SettingsStore::<Web>::open_with(options("/web"));
    assert!(store.update(|w| w.dark = true));
    store.flush().unwrap();
    let (mut reloaded, report) = SettingsStore::<Web>::open_with(options("/web"));
    assert!(reloaded.get().dark, "{report:?}");
    assert!(matches!(report.source, Source::File), "{report:?}");

    // A value over the cap fails the save and leaves the saved item as it was.
    assert!(reloaded.update(|w| w.note = "x".repeat(CAP)));
    assert!(reloaded.flush().is_err());
    let stored = LocalStorageFiles::new(CAP).read(Path::new("/web/config.toml")).unwrap().unwrap();
    assert!(stored.contains("dark = true") && !stored.contains("xxxx"), "{stored}");
    let (again, _) = SettingsStore::<Web>::open_with(options("/web"));
    assert!(again.get().dark && again.get().note.is_empty());

    // With no settings item yet, the legacy item imports into a fresh store.
    LocalStorageFiles::new(CAP).write(Path::new("wordcraft.ui"), "dark").unwrap();
    let legacy = LegacyFile { path: PathBuf::from("wordcraft.ui"), parse: parse_legacy };
    let (fresh, report) = SettingsStore::<Web>::open_with(options("/fresh").legacy(vec![legacy]));
    assert!(fresh.get().dark, "{report:?}");
    assert!(matches!(report.source, Source::Legacy(_)), "{report:?}");
}
