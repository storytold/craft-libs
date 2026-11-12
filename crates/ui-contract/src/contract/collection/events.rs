//! What a collection reports: renames, reorders, drags, toggles, opens, menu requests and column events.

use super::drops::{DropAt, DropPayload};
use super::props::LeadSource;
use craft_behavior::{ColumnKey, DropOperation, EditEvent, PartRef, SelectionChange};
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

/// A request for the header menu: a secondary click on a header cell or on the empty header after the last one, or the
/// Menu key on a focused header. The collection that shows the menu reads it with `Cx::header_menu`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct HeaderMenuRequest {
    /// The column whose header was pressed or focused; `None` for the empty header.
    pub column: Option<ColumnKey>,
    /// Where the menu opens.
    pub at: Pos2,
}

/// What a pick in the header menu asks of the header, given back with `Cx::column_request`.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum ColumnRequest {
    /// Show or hide a hideable column.
    Shown(ColumnKey, bool),
    /// Put back the app's order, show every column and drop the user's widths; the sort stays.
    Reset,
    /// Run a header command on the menu's column.
    Command(crate::contract::ColumnCommands),
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
    /// The part of the item under the pointer (a thumbnail); `None` on the item's own area and from the keyboard.
    pub part: Option<PartRef>,
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
    /// The user showed or hid the column from the header menu.
    Shown {
        /// The column.
        key: ColumnKey,
        /// Whether it shows now.
        shown: bool,
    },
    /// The user hid the sorted column: the rows have no sort, and the app puts them in their natural order.
    Unsorted,
    /// The user reset the columns: the app's order, every column shown, the app's widths. The sort stays.
    Reset,
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
    /// A primary double click on the collection's empty space, below or between its items (FilmCraft's media bin
    /// imports).
    EmptyActivated,
    /// A secondary click on the collection's empty space, below or between its items, or anywhere in an empty
    /// collection: the app opens the collection's own menu (Photoshop's Layers panel: the thumbnail sizes). The
    /// selection is unchanged.
    #[non_exhaustive]
    EmptyContextMenu {
        /// Where the menu opens: the pointer, in screen space.
        at: Pos2,
    },
    /// Space on the lead of a collection whose Space previews (`SpaceAction::Preview`).
    Preview(K),
    /// The user asked for an item's context menu.
    ContextMenu(ContextMenuRequest<K>),
    /// A Cmd/Ctrl click on an item of a `command_click` collection: a command on the item (load it as a selection),
    /// not a pick. The selection is unchanged.
    #[non_exhaustive]
    CommandClick {
        /// The item.
        key: K,
        /// The modifiers held at the click.
        mods: craft_keymap::Mods,
    },
    /// Delete or Backspace at the lead of a `deletable` collection: the selected items when the lead is selected, else
    /// the lead alone, in display order.
    Delete(Vec<K>),
    /// A tree row opened or closed.
    Open(OpenChange<K>),
    /// An item moved to another place.
    Reorder(EditEvent<ReorderMove<K>>),
    /// The selected items of a free layout moved by an offset.
    Move(EditEvent<Vec2>),
    /// A drop a drop kind took (`Collection::drops`): a payload from a drag that did not start in the collection.
    #[non_exhaustive]
    Dropped {
        /// Where it landed.
        at: DropAt<K>,
        /// What the drop does: move or copy.
        op: DropOperation,
        /// The payload the kind took, read with `get::<P>()`.
        payload: DropPayload,
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
    /// A rename started, committed or was cancelled.
    Rename(EditEvent<Rename<K>>),
    /// A toggle column's cell flipped.
    Toggle(EditEvent<ToggleEdit<K>>),
    /// A column changed.
    Column(ColumnEvent),
    /// An item reported an event.
    Item(K, E),
}
