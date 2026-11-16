mod common;

use std::path::PathBuf;

use common::{Prefs, empty_probe, open_in, probe_with_env};
use craft_settings::paths::{Location, Portable, Probe, Rule, env_var_name, resolve};
use craft_settings::{OpenOptions, Settings, SettingsStore};

#[test]
fn env_var_name_uppercases_and_replaces_punctuation() {
    assert_eq!(env_var_name("photocraft"), "PHOTOCRAFT_CONFIG_DIR");
    assert_eq!(env_var_name("photo-craft"), "PHOTO_CRAFT_CONFIG_DIR");
}

#[test]
fn override_wins_over_everything() {
    let probe = probe_with_env("TESTCRAFT_CONFIG_DIR", "/from/env");
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Dir(PathBuf::from("/explicit")), &probe);
    assert_eq!(r.dir, Some(PathBuf::from("/explicit")));
    assert_eq!(r.rule, Rule::Override);
}

#[test]
fn env_var_wins_over_portable_and_platform() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("portable"), "").unwrap();
    let mut probe = probe_with_env("TESTCRAFT_CONFIG_DIR", "/from/env");
    probe.exe_dir = Some(tmp.path().to_path_buf());
    probe.platform_config_dir = Some(PathBuf::from("/platform"));
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &probe);
    assert_eq!(r.dir, Some(PathBuf::from("/from/env")));
    assert_eq!(r.rule, Rule::EnvVar("TESTCRAFT_CONFIG_DIR".into()));
}

#[test]
fn empty_env_var_means_isolated_not_unset() {
    let mut probe = probe_with_env("TESTCRAFT_CONFIG_DIR", "");
    probe.platform_config_dir = Some(PathBuf::from("/platform"));
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &probe);
    assert_eq!(r.rule, Rule::Isolated("TESTCRAFT_CONFIG_DIR".into()));
    assert_eq!(r.dir, None, "never falls through to the real profile");
}

#[test]
fn memory_location_has_no_folder() {
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Memory, &probe_with_env("TESTCRAFT_CONFIG_DIR", "/from/env"));
    assert_eq!(r.rule, Rule::Memory);
    assert_eq!(r.dir, None);
}

#[test]
fn app_supplied_portable_names_are_used() {
    const LEGACY: Portable = Portable { markers: &["portable.txt", "PhotoCraft.portable"], dir: "PhotoCraftData" };
    for marker in ["portable.txt", "PhotoCraft.portable"] {
        let tmp = tempfile::tempdir().unwrap();
        std::fs::write(tmp.path().join(marker), "").unwrap();
        let probe = Probe { env: Box::new(|_| None), exe_dir: Some(tmp.path().to_path_buf()), platform_config_dir: Some(PathBuf::from("/platform")) };
        let r = resolve("testcraft", &LEGACY, &Location::Auto, &probe);
        assert_eq!(r.rule, Rule::Portable, "{marker}");
        assert_eq!(r.dir, Some(tmp.path().join("PhotoCraftData")), "{marker}");
    }
}

#[test]
fn default_marker_is_ignored_when_the_app_uses_its_own() {
    const LEGACY: Portable = Portable { markers: &["portable.txt"], dir: "PhotoCraftData" };
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("portable"), "").unwrap();
    let probe = Probe { env: Box::new(|_| None), exe_dir: Some(tmp.path().to_path_buf()), platform_config_dir: Some(PathBuf::from("/platform")) };
    assert_eq!(resolve("testcraft", &LEGACY, &Location::Auto, &probe).rule, Rule::Platform);
}

#[test]
fn portable_marker_next_to_executable_uses_config_folder_beside_it() {
    let tmp = tempfile::tempdir().unwrap();
    std::fs::write(tmp.path().join("portable"), "").unwrap();
    let probe = Probe { env: Box::new(|_| None), exe_dir: Some(tmp.path().to_path_buf()), platform_config_dir: Some(PathBuf::from("/platform")) };
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &probe);
    assert_eq!(r.rule, Rule::Portable);
    assert_eq!(r.dir, Some(tmp.path().join("config")));
}

#[test]
fn no_marker_means_platform_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let probe = Probe { env: Box::new(|_| None), exe_dir: Some(tmp.path().to_path_buf()), platform_config_dir: Some(PathBuf::from("/platform")) };
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &probe);
    assert_eq!(r.rule, Rule::Platform);
    assert_eq!(r.dir, Some(PathBuf::from("/platform/testcraft")));
}

#[cfg(target_os = "macos")]
#[test]
fn portable_marker_is_looked_for_next_to_the_app_bundle() {
    let tmp = tempfile::tempdir().unwrap();
    let exe_dir = tmp.path().join("Test.app/Contents/MacOS");
    std::fs::create_dir_all(&exe_dir).unwrap();
    std::fs::write(tmp.path().join("portable"), "").unwrap();
    let probe = Probe { env: Box::new(|_| None), exe_dir: Some(exe_dir), platform_config_dir: None };
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &probe);
    assert_eq!(r.rule, Rule::Portable);
    assert_eq!(r.dir, Some(tmp.path().join("config")));
}

#[test]
fn no_platform_folder_falls_back_to_current_directory() {
    let r = resolve("testcraft", &Portable::DEFAULT, &Location::Auto, &empty_probe());
    assert_eq!(r.rule, Rule::Fallback);
    assert_eq!(r.dir, Some(PathBuf::from("./testcraft")));
}

#[test]
fn report_names_the_rule_and_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let (_store, report) = open_in(tmp.path());
    assert_eq!(report.dir.as_deref(), Some(tmp.path()));
    assert_eq!(report.rule, "override");
    assert!(!report.has_problems(), "{report:?}");
}

#[test]
fn open_creates_the_folder() {
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("nested").join(Prefs::APP_ID);
    let (store, report) = SettingsStore::<Prefs>::open_with(OpenOptions::new(Location::Dir(dir.clone())).probe(empty_probe()));
    assert!(dir.is_dir());
    assert!(!store.is_read_only(), "{report:?}");
}

#[cfg(unix)]
#[test]
fn unwritable_folder_is_reported_at_open_not_first_save() {
    use std::os::unix::fs::PermissionsExt;
    let tmp = tempfile::tempdir().unwrap();
    let dir = tmp.path().join("ro");
    std::fs::create_dir(&dir).unwrap();
    if !common::restrict_directory_writes(&dir) {
        return;
    }
    let (mut store, report) = open_in(&dir);
    std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700)).unwrap();
    assert!(store.is_read_only());
    assert!(matches!(report.read_only, Some(craft_settings::ReadOnlyReason::DirNotWritable(_))), "{report:?}");
    assert!(matches!(store.flush(), Err(craft_settings::SettingsError::ReadOnly(_))));
}
