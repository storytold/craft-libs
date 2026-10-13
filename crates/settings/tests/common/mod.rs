#![allow(dead_code)]

use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::Duration;

use craft_settings::{DocumentMut, Location, MigrateError, OpenOptions, Probe, Settings, SettingsStore};
use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Default, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Prefs {
    pub general: General,
    pub appearance: Appearance,
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct General {
    pub language: String,
    pub autosave_minutes: u32,
    pub recent: Vec<String>,
}

impl Default for General {
    fn default() -> Self {
        Self { language: "en".into(), autosave_minutes: 5, recent: Vec::new() }
    }
}

#[derive(Serialize, Deserialize, Clone, PartialEq, Debug)]
#[serde(default)]
pub struct Appearance {
    pub theme: Theme,
    pub ui_scale: f64,
}

impl Default for Appearance {
    fn default() -> Self {
        Self { theme: Theme::Light, ui_scale: 1.0 }
    }
}

#[derive(Serialize, Deserialize, Clone, Copy, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Theme {
    Light,
    Dark,
}

impl Settings for Prefs {
    const APP_ID: &'static str = "testcraft";
    const SCHEMA_VERSION: u32 = 2;

    /// Version 1 stored the theme as `[appearance] dark = true`.
    fn migrate(from: u32, doc: &mut DocumentMut) -> Result<(), MigrateError> {
        match from {
            1 => {
                let dark = doc.get("appearance").and_then(|a| a.get("dark")).and_then(|d| d.as_bool());
                if let Some(dark) = dark {
                    doc["appearance"]["theme"] = toml_edit::value(if dark { "dark" } else { "light" });
                    if let Some(table) = doc["appearance"].as_table_like_mut() {
                        table.remove("dark");
                    }
                }
                Ok(())
            }
            other => Err(MigrateError::new(other, "unknown version")),
        }
    }
}

/// A probe that sees no environment, no executable and no platform folder.
pub fn empty_probe() -> Probe {
    Probe { env: Box::new(|_| None), exe_dir: None, platform_config_dir: None }
}

pub fn probe_with_env(name: &'static str, value: &'static str) -> Probe {
    Probe { env: Box::new(move |n| (n == name).then(|| OsString::from(value))), exe_dir: None, platform_config_dir: None }
}

pub fn open_in(dir: &Path) -> (SettingsStore<Prefs>, craft_settings::LoadReport) {
    SettingsStore::open_with(OpenOptions::new(Location::Dir(dir.to_path_buf())).probe(empty_probe()).debounce(Duration::from_millis(100)))
}

pub fn config_path(dir: &Path) -> PathBuf {
    dir.join("config.toml")
}

pub fn write_config(dir: &Path, text: &str) {
    std::fs::write(config_path(dir), text).unwrap();
}

pub fn read_config(dir: &Path) -> String {
    std::fs::read_to_string(config_path(dir)).unwrap()
}
