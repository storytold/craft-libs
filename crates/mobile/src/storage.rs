//! eframe's storage for the app's own settings: one JSON file in the app's data folder, read as
//! the app starts and written when the app saves (as it leaves the screen, and every
//! [`eframe::App::auto_save_interval`] while it runs).

use std::collections::BTreeMap;
use std::io::Read as _;
use std::path::{Path, PathBuf};

/// The file in the app's data folder.
pub(crate) const FILE_NAME: &str = "app-storage.json";

/// A file bigger than this isn't the app's settings: it is ignored rather than read whole.
const MAX_BYTES: u64 = 16 * 1024 * 1024;

/// Keys and values the app saved, and the file they live in.
#[derive(Debug)]
pub(crate) struct FileStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
    /// Changed since the file was last written.
    dirty: bool,
}

impl FileStorage {
    /// The storage kept in `dir`: empty when there is no file yet, or it can't be read.
    pub(crate) fn open(dir: &Path) -> Self {
        let path = dir.join(FILE_NAME);
        let values = match read(&path) {
            Ok(values) => values,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(e) => {
                log::warn!("craft-mobile: the app's saved settings ({}) can't be read, starting without them: {e}", path.display());
                BTreeMap::new()
            }
        };
        Self { path, values, dirty: false }
    }

    /// Write the values beside the file, then move them over it, so a write cut short (the OS
    /// ending the app) leaves the last complete file.
    fn write(&self) -> std::io::Result<()> {
        let json = serde_json::to_vec_pretty(&self.values)?;
        let partial = self.path.with_extension("json.partial");
        std::fs::write(&partial, json)?;
        std::fs::rename(&partial, &self.path)
    }
}

fn read(path: &Path) -> std::io::Result<BTreeMap<String, String>> {
    let mut bytes = Vec::new();
    std::fs::File::open(path)?.take(MAX_BYTES + 1).read_to_end(&mut bytes)?;
    if u64::try_from(bytes.len()).map_or(true, |len| len > MAX_BYTES) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("bigger than {MAX_BYTES} bytes")));
    }
    Ok(serde_json::from_slice(&bytes)?)
}

impl eframe::Storage for FileStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        self.values.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        if self.values.get(key) != Some(&value) {
            self.values.insert(key.to_owned(), value);
            self.dirty = true;
        }
    }

    fn remove_string(&mut self, key: &str) {
        if self.values.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// Write the file if anything changed. A failed write is logged and tried again at the next save.
    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        match self.write() {
            Ok(()) => self.dirty = false,
            Err(e) => log::warn!("craft-mobile: couldn't save the app's settings to {}: {e}", self.path.display()),
        }
    }
}

#[cfg(test)]
mod tests {
    use eframe::Storage as _;

    use super::*;

    /// A fresh folder for one test, removed when it ends.
    struct Folder(PathBuf);

    impl Folder {
        fn new(name: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("craft-mobile-storage-{}-{name}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }
    }

    impl Drop for Folder {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[test]
    fn what_the_app_saved_is_there_when_it_starts_again() {
        let dir = Folder::new("round-trip");
        let mut storage = FileStorage::open(&dir.0);
        assert_eq!(storage.get_string("theme"), None, "nothing saved yet");
        storage.set_string("theme", "dark".into());
        storage.set_string("brush", "(size: 12.5)".into());
        storage.flush();

        let mut again = FileStorage::open(&dir.0);
        assert_eq!(again.get_string("theme").as_deref(), Some("dark"));
        assert_eq!(again.get_string("brush").as_deref(), Some("(size: 12.5)"));

        again.remove_string("theme");
        again.flush();
        let third = FileStorage::open(&dir.0);
        assert_eq!(third.get_string("theme"), None, "a removed key stays removed");
        assert_eq!(third.get_string("brush").as_deref(), Some("(size: 12.5)"));
    }

    #[test]
    fn an_unreadable_file_starts_empty_and_the_next_save_replaces_it() {
        let dir = Folder::new("corrupt");
        std::fs::write(dir.0.join(FILE_NAME), b"{ not json").unwrap();
        let mut storage = FileStorage::open(&dir.0);
        assert_eq!(storage.get_string("theme"), None);
        storage.set_string("theme", "light".into());
        storage.flush();
        assert_eq!(FileStorage::open(&dir.0).get_string("theme").as_deref(), Some("light"));
    }

    #[test]
    fn nothing_is_written_until_something_changes() {
        let dir = Folder::new("unchanged");
        FileStorage::open(&dir.0).flush();
        assert!(!dir.0.join(FILE_NAME).exists());
    }
}
