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
    pub id: WindowId,
    /// The document's name, or "Name:1", "Name:2", … when several windows show the document.
    /// Also the window's title.
    pub title: String,
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

    pub fn entries(&self) -> &[MenuEntry] {
        &self.entries
    }

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

#[cfg(test)]
mod tests {
    use super::*;

    /// Windows on documents (key, name, dirty).
    fn menu(list: &WindowList<(u32, &str, bool)>) -> Vec<(String, bool, bool)> {
        WindowMenu::new(list, |_, &(key, name, dirty)| Document { key, name: name.to_owned(), dirty })
            .entries()
            .iter()
            .map(|e| (e.title.clone(), e.dirty, e.active))
            .collect()
    }

    fn entry(title: &str, dirty: bool, active: bool) -> (String, bool, bool) {
        (title.to_owned(), dirty, active)
    }

    #[test]
    fn numbers_only_windows_that_share_a_document() {
        let mut list = WindowList::new();
        let book = list.open((1, "Book1", true));
        let _notes = list.open((2, "Notes", false));
        let second = list.open((1, "Book1", true));
        list.focus(book);
        assert_eq!(menu(&list), [entry("Book1:1", true, true), entry("Notes", false, false), entry("Book1:2", true, false)]);

        list.close(book);
        assert_eq!(menu(&list), [entry("Notes", false, false), entry("Book1", true, true)], "one window left: no suffix");
        let third = list.open((1, "Book1", true));
        let titles = WindowMenu::new(&list, |_, &(key, name, dirty)| Document { key, name: name.to_owned(), dirty });
        assert_eq!(titles.title(second), Some("Book1:1"), "numbers close up in opening order");
        assert_eq!(titles.title(third), Some("Book1:2"));
    }

    #[test]
    fn same_name_different_documents_stay_unnumbered() {
        let mut list = WindowList::new();
        list.open((1, "report.docx", false));
        list.open((2, "report.docx", false));
        assert_eq!(menu(&list), [entry("report.docx", false, false), entry("report.docx", false, true)]);
    }
}
