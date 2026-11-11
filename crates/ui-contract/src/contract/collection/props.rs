//! What a collection reads from the app: selection, reorder, key and lead rules, `CollectionProps` and `RowState`.

use super::columns::{ColumnSort, ColumnSpec};
use super::events::ListDrag;
use crate::runtime::env::Label;
use craft_behavior::{Axis, ColumnKey};
use std::any::Any;
use std::sync::Arc;

/// What a collection lets the user select.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SelectMode {
    /// Nothing; the lead still moves.
    #[default]
    None,
    /// One item.
    Single,
    /// Any set of items.
    Multi,
    /// Cells of a sheet.
    Cells,
}

/// How the user may reorder items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReorderRule {
    /// Alt turns a drag into a copy (8.4).
    pub copy_with_alt: bool,
    /// In a tree, a drop or an Alt+arrow move may land inside a group, and `ReorderMove::parent` names the group. A row
    /// that `has_children` is a group; an open group without visible children takes items through the middle of its row.
    pub into_groups: bool,
}

impl ReorderRule {
    /// A rule that only moves.
    pub const fn new() -> Self {
        ReorderRule { copy_with_alt: false, into_groups: false }
    }

    /// Items may land inside the groups of a tree.
    #[must_use]
    pub const fn into_groups(mut self) -> Self {
        self.into_groups = true;
        self
    }

    /// Alt copies.
    #[must_use]
    pub const fn copy_with_alt(mut self) -> Self {
        self.copy_with_alt = true;
        self
    }
}

/// Who takes a collection's arrow keys while it has the key focus.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ItemKeys {
    /// The collection: the arrows move the lead, as in a list.
    #[default]
    Own,
    /// The component around the collection (a timeline's time area steps frames with the arrows over its whole area):
    /// the arrows are no moves of the collection and reach the owner's commands. Home, End and the Page keys stay
    /// with the collection.
    Owner,
}

/// What Space does in a collection (Enter always activates the lead).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum SpaceAction {
    /// Picks the lead row in a collection that selects, else activates it.
    #[default]
    Activate,
    /// Flips the lead row's first enabled toggle cell; with several rows selected, the cell of every selected row.
    ToggleCell,
    /// Nothing: the key is the app's.
    None,
}

/// What moved the lead.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeadSource {
    /// A key: arrows, type-ahead, or the rows changing under the lead.
    Keys,
    /// The pointer: a press or a hover.
    Pointer,
}

/// The pointer gesture that activates an item.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum ActivateOn {
    /// A double click.
    #[default]
    DoubleClick,
    /// Every primary click, whatever its count: pickers, popup lists and disclosure rows. A second quick click
    /// activates again; a double click that starts a rename renames and does not activate.
    Click,
}

/// Where the lead starts when the collection first shows rows.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum LeadOnOpen {
    /// On the selection's lead, and nowhere when nothing is selected.
    #[default]
    Selected,
    /// On the selection's lead, else on the first row that can be picked: menus and popup lists, where Enter
    /// picks the active row.
    First,
}

/// What a collection reads from the app: plain data. A new builder's name is added to the forwarder list in
/// `crates/ui-macros/src/props.rs`, so the components that embed these props forward it.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CollectionProps {
    /// What can be selected.
    pub select: SelectMode,
    /// Whether and how items reorder.
    pub reorder: Option<ReorderRule>,
    /// The item step runs rename through `Collection::rename_editor`.
    pub rename: bool,
    /// A double click on this column of a renamable item opens its rename editor instead of activating it.
    pub rename_column: Option<ColumnKey>,
    /// Empty: no columns and no header row.
    pub columns: Vec<ColumnSpec>,
    /// The sorted column and direction; `None` while the rows are unsorted.
    pub sort: Option<ColumnSort>,
    /// The text of the empty state.
    pub empty: Option<Label>,
    /// What activates an item with the pointer; Enter always activates the lead.
    pub activate: ActivateOn,
    /// Odd rows carry `RowState.striped`.
    pub striped: bool,
    /// A press on the empty space below the rows empties the selection. Off: only a key or a press on an item
    /// changes it, so a history list keeps its mark.
    pub allow_empty: bool,
    /// Where the lead starts.
    pub lead_on_open: LeadOnOpen,
    /// The lead, and the selection with it as for the arrow keys, moves to the row under the pointer when the pointer
    /// moves: menus, popup lists and the undo list.
    pub lead_follows_hover: bool,
    /// The arrow keys select the row they move the lead to. Off: they move the lead only, and Space, Enter or a click
    /// selects it (a swatch table).
    pub arrows_select: bool,
    /// A primary press moves key focus to the collection. Off: a press picks and activates and focus stays where it
    /// is, for a popup list whose opener keeps focus.
    pub focus_on_press: bool,
    /// Identifies the set the rows are, where it can change while the collection is shown: the lead starts over when
    /// it differs from the last pass's. Completion candidates, which are new rows at every keystroke.
    pub rows_stamp: Option<u64>,
    /// What Space does.
    pub space: SpaceAction,
    /// The header row shows when a column has a header text. Off: the columns lay the rows out and no header row is built.
    pub header: bool,
    /// Typed digits reach the collection's own `update` as `Intent::Digit` (rating keys); other typed text stays
    /// type-ahead.
    pub digit_keys: bool,
    /// A plain click on a row asks for its menu as a secondary click does: `CollectionEvent::ContextMenu`.
    pub menu_on_click: bool,
    /// Who takes the arrow keys.
    pub item_keys: ItemKeys,
    /// A drag from empty space draws a marquee that selects the items it sweeps.
    pub marquee: bool,
    /// Delete and Backspace at the lead report `CollectionEvent::Delete`.
    pub deletable: bool,
    /// Maps a row drag to the payload that other drop targets take (8.4). A map starts row drags by itself, with or
    /// without a reorder rule.
    pub drag_payload: Option<DragPayload>,
    /// The collection takes the full height of its rows and owns no scroll, so it can sit inside a scrolling page:
    /// every row is built, the wheel and the keys that move past an end act on the page's scroll owner, and moving
    /// the lead scrolls the page to keep the row in view. For short collections only; a long one keeps its own
    /// scroll and is shown outside a scrolling body (16.3).
    pub natural_height: bool,
    /// The axis the rows overflow along. A collection owns the scroll along it, so it sits only in a scrolling body
    /// that does not scroll along it too: a strip of tabs (`Axis::X`) sits in a page that scrolls vertically.
    pub overflow: Axis,
    /// The collection's items are pages: Ctrl+Tab and Ctrl+Shift+Tab (and the platform's other page-switch keys) move
    /// the lead and the selection to the next and previous item, wrapping at the ends.
    pub pages: bool,
    /// The key of the current item (the page a document shows, the slide on screen). It is not the lead or the
    /// selection: the item shows the current mark and its access node reports itself as the current one of its set.
    pub current: Option<Arc<dyn Any + Send + Sync>>,
}

type Payload = Arc<dyn Any + Send + Sync>;

type PayloadMap = Arc<dyn Fn(&dyn Any) -> Option<Payload> + Send + Sync>;

/// A mapping from a row drag to the payload of type `P` that other drop targets take; the drag session carries it
/// beside the `ListDrag`, so one drag and one ghost serve the collection's own reorder and the targets of `P`. A row
/// the map gives `None` for carries no payload: without a reorder rule it does not drag at all.
#[derive(Clone)]
pub struct DragPayload {
    map: PayloadMap,
}

impl DragPayload {
    /// The payload `map` makes of a drag of the collection whose keys are `K`; `None` for a row that carries none.
    pub fn new<K: 'static, P: Send + Sync + 'static>(map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        DragPayload {
            map: Arc::new(move |drag| {
                let payload: Arc<dyn Any + Send + Sync> = Arc::new(map(drag.downcast_ref::<ListDrag<K>>()?)?);
                Some(payload)
            }),
        }
    }

    pub(crate) fn of<K: 'static>(&self, drag: &ListDrag<K>) -> Option<Arc<dyn Any + Send + Sync>> {
        (self.map)(drag)
    }
}

impl std::fmt::Debug for DragPayload {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DragPayload")
    }
}

impl Default for CollectionProps {
    fn default() -> Self {
        CollectionProps::new()
    }
}

impl CollectionProps {
    /// Collection props with every setting at its default.
    pub const fn new() -> Self {
        CollectionProps {
            select: SelectMode::None,
            reorder: None,
            rename: false,
            rename_column: None,
            columns: Vec::new(),
            sort: None,
            empty: None,
            activate: ActivateOn::DoubleClick,
            striped: false,
            allow_empty: false,
            lead_on_open: LeadOnOpen::Selected,
            lead_follows_hover: false,
            arrows_select: true,
            focus_on_press: true,
            rows_stamp: None,
            space: SpaceAction::Activate,
            header: true,
            digit_keys: false,
            menu_on_click: false,
            item_keys: ItemKeys::Own,
            marquee: false,
            deletable: false,
            drag_payload: None,
            natural_height: false,
            overflow: Axis::Y,
            pages: false,
            current: None,
        }
    }

    /// What can be selected.
    #[must_use]
    pub fn select(mut self, mode: SelectMode) -> Self {
        self.select = mode;
        self
    }

    /// Items reorder by `rule`.
    #[must_use]
    pub fn reorder(mut self, rule: ReorderRule) -> Self {
        self.reorder = Some(rule);
        self
    }

    /// A row drag carries `map` of it, for the drop targets of type `P`; a row the map gives `None` for does not drag
    /// unless a reorder rule is set. The drag stays one session with one ghost.
    #[must_use]
    pub fn drag_payload<K: 'static, P: Send + Sync + 'static>(mut self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.drag_payload = Some(DragPayload::new(map));
        self
    }

    /// The items are pages (`CollectionProps::pages`).
    #[must_use]
    pub const fn pages(mut self, on: bool) -> Self {
        self.pages = on;
        self
    }

    /// Rename runs through the collection's editor.
    #[must_use]
    pub fn rename(mut self, on: bool) -> Self {
        self.rename = on;
        self
    }

    /// A double click on `column` of a renamable item (`rename`, `Collection::can_rename`) opens its rename editor; the
    /// double click elsewhere on the row activates it.
    #[must_use]
    pub fn rename_column(mut self, column: ColumnKey) -> Self {
        self.rename_column = Some(column);
        self
    }

    /// The columns.
    #[must_use]
    pub fn columns(mut self, columns: Vec<ColumnSpec>) -> Self {
        self.columns = columns;
        self
    }

    /// The column the rows are sorted by.
    #[must_use]
    pub fn sort(mut self, sort: impl Into<Option<ColumnSort>>) -> Self {
        self.sort = sort.into();
        self
    }

    /// What activates an item with the pointer.
    #[must_use]
    pub const fn activate(mut self, on: ActivateOn) -> Self {
        self.activate = on;
        self
    }

    /// Odd rows carry `RowState.striped`.
    #[must_use]
    pub const fn striped(mut self, on: bool) -> Self {
        self.striped = on;
        self
    }

    /// A press on the empty space below the rows empties the selection.
    #[must_use]
    pub const fn allow_empty(mut self, on: bool) -> Self {
        self.allow_empty = on;
        self
    }

    /// Where the lead starts.
    #[must_use]
    pub const fn lead_on_open(mut self, at: LeadOnOpen) -> Self {
        self.lead_on_open = at;
        self
    }

    /// The lead follows the pointer over the rows.
    #[must_use]
    pub const fn lead_follows_hover(mut self, on: bool) -> Self {
        self.lead_follows_hover = on;
        self
    }

    /// Whether the arrow keys select the row they move the lead to.
    #[must_use]
    pub const fn arrows_select(mut self, on: bool) -> Self {
        self.arrows_select = on;
        self
    }

    /// The rows are the set `stamp` names; the lead starts over when the stamp changes.
    #[must_use]
    pub const fn rows_stamp(mut self, stamp: u64) -> Self {
        self.rows_stamp = Some(stamp);
        self
    }

    /// A primary press moves key focus to the collection; off, it leaves focus where it is.
    #[must_use]
    pub const fn focus_on_press(mut self, on: bool) -> Self {
        self.focus_on_press = on;
        self
    }

    /// What Space does.
    #[must_use]
    pub const fn space(mut self, action: SpaceAction) -> Self {
        self.space = action;
        self
    }

    /// Whether the header row shows.
    #[must_use]
    pub const fn header(mut self, on: bool) -> Self {
        self.header = on;
        self
    }

    /// Typed digits go to the collection's `update` as `Intent::Digit`.
    #[must_use]
    pub const fn digit_keys(mut self, on: bool) -> Self {
        self.digit_keys = on;
        self
    }

    /// A plain click on a row asks for its menu, as a secondary click does.
    #[must_use]
    pub const fn menu_on_click(mut self, on: bool) -> Self {
        self.menu_on_click = on;
        self
    }

    /// Who takes the arrow keys while the collection has the key focus.
    #[must_use]
    pub const fn item_keys(mut self, keys: ItemKeys) -> Self {
        self.item_keys = keys;
        self
    }

    /// A drag from empty space selects with a marquee.
    #[must_use]
    pub const fn marquee(mut self, on: bool) -> Self {
        self.marquee = on;
        self
    }

    /// Delete and Backspace at the lead report the items to delete (`CollectionEvent::Delete`).
    #[must_use]
    pub const fn deletable(mut self, on: bool) -> Self {
        self.deletable = on;
        self
    }

    /// The collection takes its full content height and owns no scroll (see [`CollectionProps::natural_height`]).
    #[must_use]
    pub const fn natural_height(mut self, on: bool) -> Self {
        self.natural_height = on;
        self
    }

    /// The axis the rows overflow along (see [`CollectionProps::overflow`]).
    #[must_use]
    pub const fn overflow(mut self, axis: Axis) -> Self {
        self.overflow = axis;
        self
    }

    /// The current item: `Some(key)` marks the item with that key, `None` marks none. `K` is the collection's key type;
    /// a key of another type marks nothing.
    #[must_use]
    pub fn current<K: Send + Sync + 'static>(mut self, key: Option<K>) -> Self {
        self.current = key.map(|key| Arc::new(key) as Arc<dyn Any + Send + Sync>);
        self
    }

    /// The empty-state text.
    #[must_use]
    pub fn empty(mut self, text: impl Into<Label>) -> Self {
        self.empty = Some(text.into());
        self
    }
}

/// What an item reads of its place in the collection; the item step joins it to the item's flags at step 12.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RowState {
    /// The row is selected.
    pub selected: bool,
    /// The row holds the lead.
    pub lead: bool,
    /// A drag would drop on the row.
    pub drop_target: bool,
    /// The row is being dragged.
    pub dragging: bool,
    /// The row's name is being edited.
    pub renaming: bool,
    /// The app marks the row.
    pub emphasized: bool,
    /// The collection's current item (`CollectionProps::current`).
    pub current: bool,
    /// An odd row of a striped list.
    pub striped: bool,
    /// A tree row whose children show.
    pub open: bool,
    /// The pointer is over the row.
    pub hovered: bool,
}
