//! `ImeSurface`: the editor contract for components that draw their own caret (18.4). The pipeline gives the editor
//! part an egui focus widget, the IME ownership, the key filter and the edit-command routing; the component reads
//! `EditorInput` and reports `EditorState`.

use crate::runtime::memory::sub;
use craft_behavior::PartKey;
use egui::Id;

/// The handle of an editor part's focus widget.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ImeSurface {
    widget: Id,
}

impl ImeSurface {
    /// The surface of the editor part `key` of component `id`.
    pub(crate) fn of(id: Id, key: PartKey) -> Self {
        ImeSurface { widget: sub(id, ("editor", key)) }
    }

    /// The egui widget that holds key focus and owns the IME.
    pub(crate) fn widget(self) -> Id {
        self.widget
    }
}
