//! The Window menu: one entry per open window, titled by its document.

use std::collections::HashMap;
use std::hash::Hash;

use crate::list::{WindowId, WindowList};

/// What the app says about the document a window shows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Document<K> {
    /// Identifies the document; windows with equal keys show the same document.
    pub key: K,
    /// The document's display name ("Book1", "report.docx").
    pub name: String,
    /// Unsaved changes.
    pub dirty: bool,
}

/// One window in the Window menu (macOS) or the View › Switch Windows list.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry {
    /// The window.
    pub id: WindowId,
    /// The document's name, or "Name:1", "Name:2", … when several windows show the document.
    /// Also the window's title.
    pub title: String,
    /// The window has unsaved changes.
    pub dirty: bool,
    /// The active (most recently focused) window: the one to check-mark.
    pub active: bool,
}

/// The Window menu's entries, in the windows' opening order.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct WindowMenu {
    entries: Vec<MenuEntry>,
}

impl WindowMenu {
    /// Build the menu from the open windows. `document` names each window's document.
    ///
    /// When several windows show one document, each title gets ":n", numbered by opening order;
    /// numbers close up when one of them closes, and a document left with one window drops the
    /// suffix.
    pub fn new<T, K: Eq + Hash>(windows: &WindowList<T>, mut document: impl FnMut(WindowId, &T) -> Document<K>) -> Self {
        let docs: Vec<(WindowId, Document<K>)> = windows.iter().map(|(id, data)| (id, document(id, data))).collect();
        let mut counts: HashMap<&K, usize> = HashMap::new();
        for (_, doc) in &docs {
            *counts.entry(&doc.key).or_default() += 1;
        }
        let active = windows.active();
        let mut seen: HashMap<&K, usize> = HashMap::new();
        let entries = docs
            .iter()
            .map(|(id, doc)| {
                let title = if counts.get(&doc.key).copied().unwrap_or(0) > 1 {
                    let n = seen.entry(&doc.key).or_default();
                    *n += 1;
                    format!("{}:{n}", doc.name)
                } else {
                    doc.name.clone()
                };
                MenuEntry { id: *id, title, dirty: doc.dirty, active: active == Some(*id) }
            })
            .collect();
        Self { entries }
    }

    /// The windows, in the order the menu lists them.
    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

    /// The entry of window `id`, if it is listed.
    pub fn get(&self, id: WindowId) -> Option<&MenuEntry> {
        self.entries.iter().find(|e| e.id == id)
    }

    /// The window's title, as the menu shows it.
    pub fn title(&self, id: WindowId) -> Option<&str> {
        self.get(id).map(|e| e.title.as_str())
    }
}

impl<'a> IntoIterator for &'a WindowMenu {
    type Item = &'a MenuEntry;
    type IntoIter = std::slice::Iter<'a, MenuEntry>;

    fn into_iter(self) -> Self::IntoIter {
        self.entries.iter()
    }
}
