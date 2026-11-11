//! What the viewer keeps between runs. On the desktop it is the gallery's `config.toml`; the web has no file system,
//! so the settings live in memory for the tab's lifetime.

use crate::viewer::GallerySettings;
use web_time::Instant;

#[cfg(not(target_arch = "wasm32"))]
mod store {
    use super::{GallerySettings, Instant};
    use craft_settings::{LoadReport, Location, SaveStatus, SettingsStore, Source};

    /// The settings file and what loading and saving it told the user.
    pub struct Persisted {
        store: SettingsStore<GallerySettings>,
        notes: Vec<String>,
        save_error: Option<String>,
    }

    impl Persisted {
        /// The settings in the gallery's `config.toml`.
        pub fn open() -> Self {
            let (store, report) = SettingsStore::<GallerySettings>::open(Location::Auto);
            Persisted::new(store, &report)
        }

        /// `store`, which `report` describes.
        pub fn new(store: SettingsStore<GallerySettings>, report: &LoadReport) -> Self {
            Persisted { store, notes: load_notes(report), save_error: None }
        }

        /// The settings now.
        pub fn get(&self) -> &GallerySettings {
            self.store.get()
        }

        /// Records `settings`; whether they changed.
        pub fn set(&mut self, settings: GallerySettings) -> bool {
            self.store.set(settings)
        }

        /// Writes the settings now.
        pub fn save(&mut self) {
            self.save_error = self.store.flush().err().map(|error| error.to_string());
        }

        /// Saves what is due; the next time to tick, and the settings another process changed, if any.
        pub fn tick(&mut self, now: Instant) -> (Option<Instant>, Option<GallerySettings>) {
            let tick = self.store.tick(now);
            match tick.status {
                SaveStatus::Failed(error) => self.save_error = Some(error.to_string()),
                SaveStatus::Saved => self.save_error = None,
                _ => {}
            }
            for moved in self.store.take_moved_aside() {
                self.notes.push(format!("A damaged settings file was moved aside to {}", moved.display()));
            }
            let merged = self.store.take_merged_change().then(|| self.store.get().clone());
            (tick.next_deadline, merged)
        }

        /// What the status bar says about the settings.
        pub fn notes(&self) -> Vec<String> {
            let mut parts = self.notes.clone();
            parts.extend(self.save_error.iter().map(|error| format!("Settings were not saved: {error}")));
            parts
        }
    }

    /// What opening the settings told that the user should know.
    fn load_notes(report: &LoadReport) -> Vec<String> {
        let mut notes = Vec::new();
        if let Some(moved) = &report.broken_moved_to {
            notes.push(format!("A damaged settings file was moved aside to {}", moved.display()));
        }
        if let Some(reason) = &report.read_only {
            notes.push(format!("Settings are read-only: {reason}"));
        }
        if let Source::Legacy(from) = &report.source {
            notes.push(format!("Settings were imported from {}", from.display()));
        }
        if !report.section_errors.is_empty() {
            notes.push(format!("{} settings sections were reset to their defaults", report.section_errors.len()));
        }
        notes.extend(report.warning.iter().cloned());
        notes
    }
}

#[cfg(target_arch = "wasm32")]
mod store {
    use super::{GallerySettings, Instant};

    /// The settings of this tab.
    pub struct Persisted {
        settings: GallerySettings,
    }

    impl Persisted {
        /// Settings at their defaults.
        pub fn open() -> Self {
            Persisted { settings: GallerySettings::default() }
        }

        /// The settings now.
        pub fn get(&self) -> &GallerySettings {
            &self.settings
        }

        /// Records `settings`; nothing waits to be saved, so this is always false.
        pub fn set(&mut self, settings: GallerySettings) -> bool {
            self.settings = settings;
            false
        }

        /// Nothing is written on the web.
        pub fn save(&mut self) {}

        /// Nothing is due on the web.
        pub fn tick(&mut self, _now: Instant) -> (Option<Instant>, Option<GallerySettings>) {
            (None, None)
        }

        /// The web has nothing to report about the settings.
        pub fn notes(&self) -> Vec<String> {
            Vec::new()
        }
    }
}

pub use store::Persisted;
