//! Library sections: tables beside `T`'s keys in the same document.

use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Instant;

use toml_edit::{DocumentMut, Item};

use super::SettingsStore;
use crate::Settings;
use crate::document::{SCHEMA_VERSION_KEY, expand_inline_tables};

/// A store behind the lock the app and its [`craft_host::Host`] share: the app keeps one and passes
/// `Arc::new(settings.clone())` to the Host, so both use the same lock.
///
/// A local type, because the orphan rule forbids implementing a `craft-host` trait for
/// `Arc<Mutex<SettingsStore<T>>>`.
pub struct SharedSettings<T: Settings>(Arc<Mutex<SettingsStore<T>>>);

impl<T: Settings> Clone for SharedSettings<T> {
    fn clone(&self) -> Self {
        Self(Arc::clone(&self.0))
    }
}

impl<T: Settings> SharedSettings<T> {
    /// Shares `store`.
    pub fn new(store: SettingsStore<T>) -> Self {
        Self(Arc::new(Mutex::new(store)))
    }

    /// Locks the store. A lock poisoned by a panic elsewhere is taken over: the store holds plain data.
    pub fn lock(&self) -> MutexGuard<'_, SettingsStore<T>> {
        self.0.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Sections are top-level tables of the document, so `T` keeps ignoring them on load and
/// [`merge_into`](crate::document::merge_into) keeps them on save; a write is saved with the next save.
impl<T: Settings + Send + 'static> craft_host::SectionStore for SharedSettings<T> {
    fn read(&self, key: &str) -> Option<toml::Table> {
        let store = self.lock();
        let table = store.doc.get(key)?.as_table()?.clone();
        toml_edit::de::from_document(DocumentMut::from(table)).ok()
    }

    fn write(&self, key: &str, table: toml::Table) {
        if key == SCHEMA_VERSION_KEY {
            return;
        }
        let Ok(mut fresh) = toml_edit::ser::to_document(&table) else {
            return;
        };
        expand_inline_tables(fresh.as_table_mut());
        let mut store = self.lock();
        // The table keeps the comments and layout of the section it replaces when nothing changed.
        let unchanged = store.doc.get(key).and_then(Item::as_table).is_some_and(|old| old.to_string() == fresh.as_table().to_string());
        if unchanged {
            return;
        }
        store.doc[key] = Item::Table(fresh.into_table());
        store.mark_changed(Instant::now());
    }
}
