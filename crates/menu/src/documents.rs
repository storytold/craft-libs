//! The Window menu's open documents as catalogued rows: numbered command ids
//! (`window.document.N`, N the document's tab position) and every row listed with the rest of the
//! menu, so automation and the command palette see them (gap 29).
//!
//! The layouts add the rows last in Window ([`crate::MenuBar::documents`]) as check items, the one
//! in front checked, as Photoshop and PhotoCraft mark it; an ordinary [`crate::MenuBar::items`]
//! walk doesn't see them because they exist only in the laid-out bar.

use craft_keymap::CommandRef;

use crate::model::{Document, Item, MenuBar};

/// The prefix of a numbered document row's command id; the id ends in the document's index.
pub const DOCUMENT_COMMAND_PREFIX: &str = "window.document.";

impl Document {
    /// The document at tab position `index`, running `window.document.{index}`.
    pub fn numbered(index: usize, label: impl Into<String>) -> Document {
        Document::new(CommandRef::new(format!("{DOCUMENT_COMMAND_PREFIX}{index}")), label)
    }

    /// The tab position a numbered document command names, `None` for any other command.
    pub fn index_from_command(command: &CommandRef) -> Option<usize> {
        command.id.strip_prefix(DOCUMENT_COMMAND_PREFIX)?.parse().ok()
    }
}

impl MenuBar {
    /// Every item as the app's menus list it (depth first, as [`MenuBar::items`]), followed by
    /// the listed documents as the Window menu shows them: check items, the one in front checked.
    /// For a menu listing (PhotoCraft's `ui.menu.list`, the command palette), not for drawing.
    pub fn listed_items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = self.items().into_iter().cloned().collect();
        items.extend(self.documents.iter().filter(|d| !d.hidden).map(|d| Item::new(d.command.clone(), d.label.clone()).check(d.active)));
        items
    }
}
