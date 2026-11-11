//! [`DialogStack`]: which open modal dialog takes the input.

use egui::Id;

/// The id a [`DialogStack`] knows a dialog by: a dialog's own key through [`dialog_id`].
pub type DialogId = Id;

/// The id a host gives the dialog it opens from `key`, pushed on a [`DialogStack`] before the dialog first shows.
pub fn dialog_id(key: impl std::hash::Hash + std::fmt::Debug) -> DialogId {
    crate::runtime::memory::unique(("craft_ui::dialog", key))
}

/// The open dialogs in the order they were opened, kept by the host next to its dialog values. The layer stack
/// makes only the top modal layer interactive; this routes which dialog is open and which dialogs a parent opened.
///
/// A nested dialog (a contour editor opened from a layer style dialog) is pushed with its parent. Closing the
/// parent closes the dialogs it opened.
///
/// ```
/// # extern crate craft_ui_panels as craft_ui;
/// # use craft_ui::dialog::{DialogStack, dialog_id};
/// let (style, contour) = (dialog_id("layer-style"), dialog_id("contour"));
/// let mut stack = DialogStack::default();
/// stack.push(style, None);
/// stack.push(contour, Some(style));
/// assert!(stack.is_top(contour) && stack.has_child(style));
/// stack.remove_tree(style);
/// assert!(stack.is_empty());
/// ```
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DialogStack {
    entries: Vec<(DialogId, Option<DialogId>)>,
}

impl DialogStack {
    /// Opens `id` on top, under `parent` if it is a child. Pushing an id already in the stack
    /// changes nothing, so a host may push every pass.
    pub fn push(&mut self, id: DialogId, parent: Option<DialogId>) {
        if !self.contains(id) {
            self.entries.push((id, parent));
        }
    }

    /// Removes `id` alone; its children, if any, stay.
    pub fn remove(&mut self, id: DialogId) {
        self.entries.retain(|(entry, _)| *entry != id);
    }

    /// Removes `id` and every dialog below it in the parent chain: cancelling a parent cancels the
    /// dialogs it opened.
    pub fn remove_tree(&mut self, id: DialogId) {
        let mut doomed = vec![id];
        for (entry, parent) in &self.entries {
            if parent.is_some_and(|p| doomed.contains(&p)) {
                doomed.push(*entry);
            }
        }
        self.entries.retain(|(entry, _)| !doomed.contains(entry));
    }

    /// True when `id` is on the stack.
    pub fn contains(&self, id: DialogId) -> bool {
        self.entries.iter().any(|(entry, _)| *entry == id)
    }

    /// True when no dialog is on the stack.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The dialog that takes the input: the last one opened.
    pub fn top(&self) -> Option<DialogId> {
        self.entries.last().map(|(id, _)| *id)
    }

    /// True when `id` is the top dialog.
    pub fn is_top(&self, id: DialogId) -> bool {
        self.top() == Some(id)
    }

    /// Whether another dialog was opened from `id` and is still open.
    pub fn has_child(&self, id: DialogId) -> bool {
        self.entries.iter().any(|(_, parent)| *parent == Some(id))
    }

    /// The dialogs opened from `id`, in opening order.
    pub fn children(&self, id: DialogId) -> impl Iterator<Item = DialogId> + '_ {
        self.entries.iter().filter(move |(_, parent)| *parent == Some(id)).map(|(entry, _)| *entry)
    }

    /// Every dialog in opening order.
    pub fn ids(&self) -> impl Iterator<Item = DialogId> + '_ {
        self.entries.iter().map(|(id, _)| *id)
    }
}
