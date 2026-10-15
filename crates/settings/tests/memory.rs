mod common;

use std::time::{Duration, Instant};

use common::{Prefs, Theme, empty_probe, probe_with_env};
use craft_settings::{Location, OpenOptions, SaveStatus, SettingsStore, Source};

/// A probe whose platform folder is `dir`, so any write would land there.
fn probe_in(dir: &std::path::Path, env: Option<(&'static str, &'static str)>) -> craft_settings::Probe {
    let mut probe = match env {
        Some((name, value)) => probe_with_env(name, value),
        None => empty_probe(),
    };
    probe.platform_config_dir = Some(dir.to_path_buf());
    probe
}

fn assert_untouched(dir: &std::path::Path) {
    let entries: Vec<_> = std::fs::read_dir(dir).unwrap().collect();
    assert!(entries.is_empty(), "nothing may be written: {entries:?}");
}

fn exercise(store: &mut SettingsStore<Prefs>) {
    assert!(!store.is_persistent());
    assert_eq!(store.dir(), None);
    assert_eq!(store.path(), None);
    assert!(store.update(|p| p.appearance.theme = Theme::Dark), "updates still apply");
    assert_eq!(store.get().appearance.theme, Theme::Dark);
    assert!(!store.is_dirty(), "nothing to save");
    assert!(matches!(store.tick(Instant::now() + Duration::from_secs(5)).status, SaveStatus::Idle));
    store.flush().unwrap();
}

#[test]
fn empty_env_var_reads_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    // A real settings file in the platform folder must not be read either.
    std::fs::create_dir(tmp.path().join("testcraft")).unwrap();
    std::fs::write(tmp.path().join("testcraft/config.toml"), "schema_version = 2\n[general]\nlanguage = \"de\"\n").unwrap();

    let (mut store, report) =
        SettingsStore::<Prefs>::open_with(OpenOptions::new(Location::Auto).probe(probe_in(tmp.path(), Some(("TESTCRAFT_CONFIG_DIR", "")))));
    assert_eq!(report.dir, None);
    assert_eq!(report.source, Source::Defaults);
    assert!(report.rule.contains("isolated"), "{}", report.rule);
    assert!(!report.has_problems(), "{report:?}");
    assert_eq!(store.get().general.language, "en", "the real profile is not read");
    exercise(&mut store);
    assert_eq!(
        std::fs::read_to_string(tmp.path().join("testcraft/config.toml")).unwrap(),
        "schema_version = 2\n[general]\nlanguage = \"de\"\n",
        "the real profile is not written"
    );
    assert_eq!(std::fs::read_dir(tmp.path().join("testcraft")).unwrap().count(), 1);
}

#[test]
fn memory_location_reads_and_writes_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let (mut store, report) = SettingsStore::<Prefs>::open_with(OpenOptions::new(Location::Memory).probe(probe_in(tmp.path(), None)));
    assert_eq!(report.rule, "in memory only");
    exercise(&mut store);
    assert_untouched(tmp.path());
}

#[test]
fn memory_ignores_legacy_files() {
    let tmp = tempfile::tempdir().unwrap();
    let old = tmp.path().join("prefs.json");
    std::fs::write(&old, "{}").unwrap();
    let files = vec![craft_settings::LegacyFile { path: old, parse: |_| Ok(Prefs::default()) }];
    let (_, report) = SettingsStore::<Prefs>::open_with(OpenOptions::new(Location::Memory).probe(empty_probe()).legacy(files));
    assert_eq!(report.source, Source::Defaults, "an isolated store must not import from the real profile");
}
