//! What native panels show: the same shape for every Craft app.

use serde::Serialize;

/// The state native panels show. Apps fill in what they have; the shells lay it out.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Snapshot {
    /// The active document, if any.
    pub document: Option<DocumentInfo>,
    /// Undo would do something.
    pub can_undo: bool,
    /// Redo would do something.
    pub can_redo: bool,
    /// What the items are called in this app: "Layers", "Tracks", "Pages".
    pub items_title: String,
    /// The active document's items, top to bottom as its panel lists them.
    pub items: Vec<Item>,
}

/// The active document.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct DocumentInfo {
    /// The document's name, as its tab shows it.
    pub name: String,
    /// One line about it, such as "1920 × 1080 px".
    pub detail: String,
    /// Changed since it was last saved.
    pub dirty: bool,
}

/// One row of the items panel.
#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct Item {
    /// The app's own id; actions name the item by it.
    pub id: u64,
    /// As the app's own panel shows it.
    pub name: String,
    /// What kind of item, in the app's words ("Pixel", "Group", "Text").
    pub kind: String,
    /// Nesting depth (inside groups), 0 at the top level.
    pub depth: u32,
    /// `None` when the item can't be hidden.
    pub visible: Option<bool>,
    /// Selected (or the active item).
    pub selected: bool,
}
