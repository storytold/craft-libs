//! What a collection reports: renames, reorders, drags, toggles, opens, menu requests and column events.

use super::props::LeadSource;
use crate::runtime::drag::OsFile;
use craft_behavior::{ColumnKey, DropOperation, EditEvent, SelectionChange};
use egui::{Id, Pos2, Vec2};

/// A committed rename.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Rename<K> {
    /// The renamed item.
    pub key: K,
    /// Its new name.
    pub name: String,
}

/// A reorder: the items `keys` move together, and the first of them lands at position `to`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReorderMove<K> {
    /// The item the pointer or the key grabbed.
    pub key: K,
    /// Every moved item in display order, `key` among them. They end up next to each other.
    pub keys: Vec<K>,
    /// The new position of the first of `keys`, counted in the order that remains once all of them are taken out.
    pub to: usize,
    /// The drag was a copy (Alt held, `ReorderRule::copy_with_alt`): the item stays and a copy goes to `to`.
    pub copy: bool,
    /// The group the items land in: `None` at the top level, and always `None` unless the rule has `into_groups`.
    pub parent: Option<K>,
}

impl<K> ReorderMove<K> {
    /// A move of `keys` (`key` among them) to position `to` at the top level.
    pub fn new(key: K, keys: Vec<K>, to: usize, copy: bool) -> Self {
        ReorderMove { key, keys, to, copy, parent: None }
    }

    /// The items land in the group `parent`, `None` for the top level.
    #[must_use]
    pub fn in_group(mut self, parent: Option<K>) -> Self {
        self.parent = parent;
        self
    }
}

/// What a row drag carries through the drag session (8.4).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ListDrag<K> {
    /// The collection the drag started in.
    pub list: Id,
    /// The dragged items in display order: the selection when the grabbed row was selected, else the row.
    pub keys: Vec<K>,
    /// The row the pointer grabbed.
    pub grabbed: K,
}

impl<K> ListDrag<K> {
    /// A drag of `keys` from `list`, grabbed at `grabbed`.
    pub fn new(list: Id, keys: Vec<K>, grabbed: K) -> Self {
        ListDrag { list, keys, grabbed }
    }
}

/// A request for the context menu of a column header: the app lists `ColumnCommands` in it.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HeaderMenuRequest {
    /// The column whose header was pressed or focused.
    pub column: ColumnKey,
    /// Where the menu opens.
    pub at: Pos2,
}

/// A toggle column's cell flipped.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToggleEdit<K> {
    /// The item.
    pub key: K,
    /// The column.
    pub column: ColumnKey,
    /// The new value.
    pub on: bool,
    /// The modifiers held at the press that flipped it (the first press of a sweep).
    pub mods: craft_keymap::Mods,
}

/// A tree row opened or closed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OpenChange<K> {
    /// The row.
    pub key: K,
    /// Whether its children show now.
    pub open: bool,
    /// The change reaches every descendant too (Alt on the disclosure).
    pub all: bool,
}

impl<K> OpenChange<K> {
    /// `key` opens or closes.
    pub const fn new(key: K, open: bool) -> Self {
        OpenChange { key, open, all: false }
    }

    /// The change reaches every descendant.
    #[must_use]
    pub const fn all(mut self) -> Self {
        self.all = true;
        self
    }
}

/// A request for the context menu of an item. Collections never open menus: the app does.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct ContextMenuRequest<K> {
    /// The item.
    pub key: K,
    /// Where the menu opens.
    pub at: Pos2,
    /// The column the press landed in (a toggle, a text or the label column); `None` from the keyboard and in a list
    /// without columns.
    pub column: Option<ColumnKey>,
}

/// A change to the columns.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColumnEvent {
    /// The header was activated.
    #[non_exhaustive]
    Sort {
        /// The column.
        key: ColumnKey,
        /// Whether the order is now descending.
        descending: bool,
    },
    /// The user dragged the column's edge.
    Resized {
        /// The column.
        key: ColumnKey,
        /// Its new width.
        width: f32,
    },
    /// The user moved the column.
    #[non_exhaustive]
    Moved {
        /// The column.
        key: ColumnKey,
        /// Its new position.
        to: usize,
    },
}

/// What a collection reports, in drawing order.
#[derive(Debug)]
#[non_exhaustive]
pub enum CollectionEvent<K, E> {
    /// The selection changed.
    Selection(SelectionChange<K>),
    /// The lead moved, by keys or by the pointer.
    LeadMoved {
        /// The item that took the lead.
        key: K,
        /// What moved it.
        by: LeadSource,
    },
    /// Enter or a double click on an item.
    Activate(K),
    /// The user asked for an item's context menu.
    ContextMenu(ContextMenuRequest<K>),
    /// Delete or Backspace at the lead of a `deletable` collection: the selected items when the lead is selected, else
    /// the lead alone, in display order.
    Delete(Vec<K>),
    /// A tree row opened or closed.
    Open(OpenChange<K>),
    /// An item moved to another place.
    Reorder(EditEvent<ReorderMove<K>>),
    /// The selected items of a free layout moved by an offset.
    Move(EditEvent<Vec2>),
    /// A drop from another collection onto an item, between items, or into an empty collection.
    #[non_exhaustive]
    Dropped {
        /// The item dropped on; `None` for an empty collection.
        key: Option<K>,
        /// The position of the display order the items land before; the source's length drops them after the last item.
        insert: usize,
        /// What the drop does: move or copy.
        op: DropOperation,
        /// The dragged items in display order, from the drag session's payload.
        keys: Vec<K>,
    },
    /// Ctrl or Cmd with the wheel over a collection that `zooms`: the zoom factor of the pass and the pointer it is
    /// about, in screen points.
    #[non_exhaustive]
    Zoom {
        /// The factor to multiply the zoom by.
        factor: f32,
        /// The pointer the zoom is about.
        about: Pos2,
    },
    /// Files from the OS dropped on a collection that takes them (`Collection::takes_files`).
    #[non_exhaustive]
    FilesDropped {
        /// The position of the display order the files land before; the source's length drops them after the last item.
        insert: usize,
        /// The files, in the order the OS lists them.
        files: Vec<OsFile>,
    },
    /// A rename started, committed or was cancelled.
    Rename(EditEvent<Rename<K>>),
    /// A toggle column's cell flipped.
    Toggle(EditEvent<ToggleEdit<K>>),
    /// A column changed.
    Column(ColumnEvent),
    /// The user asked for the context menu of a column header.
    HeaderMenu(HeaderMenuRequest),
    /// An item reported an event.
    Item(K, E),
}
