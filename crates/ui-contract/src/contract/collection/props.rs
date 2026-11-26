//! What a collection reads from the app: selection, reorder, key and lead rules, `CollectionProps` and `RowState`.

use super::columns::{ColumnSort, ColumnSpec};
use super::events::ListDrag;
use crate::contract::{Notch, ScrollAnchor};
pub use crate::runtime::env::RowPressed;
use crate::runtime::env::{Counted, Label};
use crate::style::StyleCx;
use craft_behavior::{Axis, ColumnKey, CopyKeys, MiddleButton, ReorderKeys};
use std::any::Any;
use std::sync::Arc;

/// What a collection lets the user select.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, crate::contract::MemoProps)]
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

/// Where Delete and Backspace can target a deletable collection.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum DeleteTarget {
    /// Only while the collection has keyboard focus.
    #[default]
    Focus,
    /// Also while the pointer is over the collection and no text editor or modal owns the keys. Delete acts on the
    /// picked rows, or on the current row when none are picked, falling back to the lead.
    FocusOrHover,
}

/// What a row drag shows while it runs. The text is the count word for the dragged items
/// (`CollectionProps::drag_count`), else the number of items for several and the item's text for one.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum RowPreview {
    /// The text in a badge by the pointer.
    #[default]
    Badge,
    /// The text inside the region a drop would take, where the target marks one ([`DragPreview::Named`]); nothing
    /// by the pointer.
    ///
    /// [`DragPreview::Named`]: crate::runtime::drag::DragPreview::Named
    Named,
    /// No preview: the dragged row shows alone.
    None,
}

/// How the user may reorder items.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReorderRule {
    /// The held key that turns a drag into a copy (8.4); `None`: a drag only moves.
    pub copy: Option<CopyKeys>,
    /// In a tree, a drop or a stacking-key move may land inside a group, and `ReorderMove::parent` names the group. A row
    /// that `has_children` is a group; an open group without visible children takes items through the middle of its row.
    pub into_groups: bool,
    /// The key scheme that reorders the collection's items; default `Stacking`.
    pub keys: ReorderKeys,
}

impl ReorderRule {
    /// A rule that only moves.
    pub const fn new() -> Self {
        ReorderRule { copy: None, into_groups: false, keys: ReorderKeys::Stacking }
    }

    /// The key scheme that reorders the items.
    #[must_use]
    pub const fn keys(mut self, keys: ReorderKeys) -> Self {
        self.keys = keys;
        self
    }

    /// Items may land inside the groups of a tree.
    #[must_use]
    pub const fn into_groups(mut self) -> Self {
        self.into_groups = true;
        self
    }

    /// A drag with `keys` held copies: [`CopyKeys::Alt`] in a design app's layers, [`CopyKeys::Platform`] (Ctrl off
    /// macOS) in a file or page grid.
    #[must_use]
    pub const fn copy_with(mut self, keys: CopyKeys) -> Self {
        self.copy = Some(keys);
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
    /// Reports `CollectionEvent::Preview` for the lead row (Bridge's preview); Ctrl+Space still toggles the lead's pick.
    Preview,
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

/// What the pointer moving over the rows changes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub enum HoverLead {
    /// Nothing: only a press moves the lead.
    #[default]
    Off,
    /// The lead moves to the row under the pointer; the selection stays until a pick (click, Enter, or a key move):
    /// option lists, where the selection is the chosen value.
    Lead,
    /// The lead moves and picks the row, as the arrow keys do: menus and the undo list, where the hover range is
    /// the point.
    LeadAndSelection,
}

/// How tall a collection's rows are. Read-only display tables (a metadata table) take text rows; lists the user picks
/// from or navigates keep control-high rows, a pointer target as tall as a control. A row with a thumbnail or a
/// detail line is as tall as those need either way.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum RowHeight {
    /// A control high: `size.control` at the collection's control size.
    #[default]
    Control,
    /// One text line and a tight gap: `size.row.text`.
    Text,
}

impl RowHeight {
    /// The height of one row under `cx`.
    pub fn of(self, cx: &StyleCx<'_>) -> f32 {
        match self {
            RowHeight::Control => cx.control().height,
            RowHeight::Text => cx.tokens.size.row.text,
        }
    }
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
    /// A plain click on the label of the only selected item, with no second click within the double-click interval,
    /// opens its rename editor (Finder and Bridge on a tile's caption).
    pub slow_click_rename: bool,
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
    /// What the pointer moving over the rows changes: the lead, and for some lists the selection too.
    pub hover_lead: HoverLead,
    /// The arrow keys select the row they move the lead to. Off: they move the lead only, and Space, Enter or a click
    /// selects it (a swatch table, a tab strip, a preset browser); Enter then activates it as well.
    pub arrows_select: bool,
    /// A primary press moves key focus to the collection. Off: a press picks and activates and focus stays where it
    /// is, for a popup list whose opener keeps focus.
    pub focus_on_press: bool,
    /// Identifies the set the rows are, where it can change while the collection is shown: the lead starts over when
    /// it differs from the last pass's. Completion candidates, which are new rows at every keystroke.
    pub rows_stamp: Option<u64>,
    /// What Space does.
    pub space: SpaceAction,
    /// Typed letters move the lead to the next item whose text starts with them. Off: they stay shortcut candidates,
    /// so single-letter tool keys keep working while a list has focus. On for popup lists and pickers.
    pub type_ahead: bool,
    /// The arrow keys wrap from the last item to the first and back (a command palette's list).
    pub arrows_wrap: bool,
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
    /// Where a deletable collection takes Delete and Backspace.
    pub delete_target: DeleteTarget,
    /// Map a row drag to the payloads that other drop targets take (8.4), one per type. A map starts row drags by
    /// itself, with or without a reorder rule.
    pub drag_payloads: Vec<DragPayload>,
    /// The count word a row drag's preview shows for the dragged items, one item included ("1 page", "3 pages");
    /// `None` shows the number of items for several and the item's text for one.
    pub drag_count: Option<Counted>,
    /// What a row drag shows while it runs.
    pub drag_preview: RowPreview,
    /// The collection takes the full height of its rows and owns no scroll, so it can sit inside a scrolling page:
    /// every row is built, the wheel and the keys that move past an end act on the page's scroll owner, and moving
    /// the lead scrolls the page to keep the row in view. For short collections only; a long one keeps its own
    /// scroll and is shown outside a scrolling body (16.3).
    pub natural_height: bool,
    /// The least number of rows the collection is tall: rows it does not have count the row height, so an empty
    /// collection stays a target of its size.
    pub min_rows: u16,
    /// The most rows the collection shows: past them it is as tall as its first `max_rows` rows and scrolls them. It
    /// owns that scroll inside a scrolling body too, since its height is bounded and the wheel passes on at its ends.
    pub max_rows: Option<u16>,
    /// How tall the rows are.
    pub row_height: RowHeight,
    /// A Cmd/Ctrl click on an item, on its own area or a part that picks it, is a command on the item, not a pick: the
    /// selection stays and the release reports `CollectionEvent::CommandClick`.
    pub command_click: bool,
    /// The distance one wheel notch scrolls the rows; unset, the default notch.
    pub notch: Option<Notch>,
    /// What a middle press does: pan or latched auto-scroll.
    pub middle_button: MiddleButton,
    /// Only a primary press picks an item: a secondary click asks for the item's menu and leaves the selection, and a
    /// middle press picks nothing. Off, every button's press picks the item under it.
    pub primary_picks: bool,
    /// Flip the direction of middle-button pan and auto-scroll.
    pub invert_middle: bool,
    /// The axis the rows overflow along. A collection owns the scroll along it, so it sits only in a scrolling body
    /// that does not scroll along it too: a strip of tabs (`Axis::X`) sits in a page that scrolls vertically.
    pub overflow: Axis,
    /// The collection's items are pages: Ctrl+Tab and Ctrl+Shift+Tab (and the platform's other page-switch keys) move
    /// the lead and the selection to the next and previous item, wrapping at the ends.
    pub pages: bool,
    /// The key of the current item (the page a document shows, the slide on screen). It is not the lead or the
    /// selection: the item shows the current mark and its access node reports itself as the current one of its set.
    pub current: Option<Arc<dyn Any + Send + Sync>>,
    /// The selection is a pick kept beside the current item (Acrobat's Pages panel): a plain click on an item moves
    /// the lead there and empties the selection, and Cmd/Ctrl- and Shift-clicks pick without moving the lead, counting
    /// the current item as picked while nothing is. Off, a plain click selects the item it lands on.
    pub picks_beside_current: bool,
    /// While a drag whose payload is neither the collection's own nor one of its drop kinds rests over an item that is
    /// not selected, the item becomes the lead and the selection in that pass.
    pub select_under_drag: bool,
    /// Which end of the rows a collection that scrolls its own rows stays at while rows are added; `End` keeps the offset
    /// at the end as long as the user left it there.
    pub scroll_anchor: ScrollAnchor,
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
            slow_click_rename: false,
            columns: Vec::new(),
            sort: None,
            empty: None,
            activate: ActivateOn::DoubleClick,
            striped: false,
            allow_empty: false,
            lead_on_open: LeadOnOpen::Selected,
            hover_lead: HoverLead::Off,
            arrows_select: true,
            focus_on_press: true,
            rows_stamp: None,
            space: SpaceAction::Activate,
            type_ahead: false,
            arrows_wrap: false,
            header: true,
            digit_keys: false,
            menu_on_click: false,
            item_keys: ItemKeys::Own,
            marquee: false,
            deletable: false,
            delete_target: DeleteTarget::Focus,
            drag_payloads: Vec::new(),
            drag_count: None,
            drag_preview: RowPreview::Badge,
            natural_height: false,
            min_rows: 0,
            max_rows: None,
            row_height: RowHeight::Control,
            command_click: false,
            notch: None,
            middle_button: MiddleButton::Pan,
            primary_picks: false,
            invert_middle: false,
            overflow: Axis::Y,
            pages: false,
            current: None,
            picks_beside_current: false,
            select_under_drag: false,
            scroll_anchor: ScrollAnchor::Start,
        }
    }

    /// Whether a foreign drag resting over an unselected item selects it (`select_under_drag`).
    #[must_use]
    pub const fn select_under_drag(mut self, on: bool) -> Self {
        self.select_under_drag = on;
        self
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
    pub fn drag_payload<K: 'static, P: Send + Sync + 'static>(self, map: impl Fn(&ListDrag<K>) -> Option<P> + Send + Sync + 'static) -> Self {
        self.carry(DragPayload::new(map))
    }

    /// A row drag carries `payload` of it too, beside the payloads given before: a host adds its own payload type to a
    /// drag the component already maps.
    #[must_use]
    pub fn carry(mut self, payload: DragPayload) -> Self {
        self.drag_payloads.push(payload);
        self
    }

    /// What a row drag shows while it runs (a badge by the pointer by default).
    #[must_use]
    pub const fn drag_preview(mut self, preview: RowPreview) -> Self {
        self.drag_preview = preview;
        self
    }

    /// A row drag's preview reads `count` for the number of dragged items.
    #[must_use]
    pub const fn drag_count(mut self, count: Counted) -> Self {
        self.drag_count = Some(count);
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

    /// A slow second click on the label of the only selected item opens its rename editor.
    #[must_use]
    pub const fn slow_click_rename(mut self, on: bool) -> Self {
        self.slow_click_rename = on;
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

    /// What the pointer moving over the rows changes.
    #[must_use]
    pub const fn hover_lead(mut self, hover: HoverLead) -> Self {
        self.hover_lead = hover;
        self
    }

    /// Whether the arrow keys select the row they move the lead to.
    #[must_use]
    pub const fn arrows_select(mut self, on: bool) -> Self {
        self.arrows_select = on;
        self
    }

    /// Typed letters move the lead (`CollectionProps::type_ahead`).
    #[must_use]
    pub const fn type_ahead(mut self, on: bool) -> Self {
        self.type_ahead = on;
        self
    }

    /// The arrow keys wrap at the ends (`CollectionProps::arrows_wrap`).
    #[must_use]
    pub const fn arrows_wrap(mut self, on: bool) -> Self {
        self.arrows_wrap = on;
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

    /// Sets where a deletable collection takes Delete and Backspace.
    #[must_use]
    pub const fn delete_target(mut self, target: DeleteTarget) -> Self {
        self.delete_target = target;
        self
    }

    /// The collection takes its full content height and owns no scroll (see [`CollectionProps::natural_height`]).
    #[must_use]
    pub const fn natural_height(mut self, on: bool) -> Self {
        self.natural_height = on;
        self
    }

    /// The collection is at least `rows` rows tall (see [`CollectionProps::min_rows`]).
    #[must_use]
    pub const fn min_rows(mut self, rows: u16) -> Self {
        self.min_rows = rows;
        self
    }

    /// The collection shows at most `rows` rows and scrolls the rest (see [`CollectionProps::max_rows`]).
    #[must_use]
    pub const fn max_rows(mut self, rows: u16) -> Self {
        self.max_rows = Some(rows);
        self
    }

    /// The rows are `height` tall (see [`RowHeight`]).
    #[must_use]
    pub const fn row_height(mut self, height: RowHeight) -> Self {
        self.row_height = height;
        self
    }

    /// A Cmd/Ctrl click on an item is a command, not a pick (see [`CollectionProps::command_click`]).
    #[must_use]
    pub const fn command_click(mut self, on: bool) -> Self {
        self.command_click = on;
        self
    }

    /// The distance one wheel notch scrolls the rows (see [`CollectionProps::notch`]).
    #[must_use]
    pub const fn notch(mut self, notch: Notch) -> Self {
        self.notch = Some(notch);
        self
    }

    /// What a middle press does: pan, or latched auto-scroll (see [`CollectionProps::middle_button`]).
    #[must_use]
    pub const fn middle_button(mut self, button: MiddleButton) -> Self {
        self.middle_button = button;
        self
    }

    /// Whether only a primary press picks an item (see [`CollectionProps::primary_picks`]).
    #[must_use]
    pub const fn primary_picks(mut self, on: bool) -> Self {
        self.primary_picks = on;
        self
    }

    /// Flips the direction of middle-button pan and auto-scroll (see [`CollectionProps::invert_middle`]).
    #[must_use]
    pub const fn invert_middle(mut self, on: bool) -> Self {
        self.invert_middle = on;
        self
    }

    /// The wheel distance of one notch, with a text line `line` points high: the set notch, else the default.
    pub fn wheel_notch(&self, line: f32) -> craft_behavior::WheelNotch {
        self.notch.map_or(craft_behavior::WheelNotch::DEFAULT, |notch| notch.wheel(line))
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

    /// Keeps the selection a pick beside the current item (see [`CollectionProps::picks_beside_current`]).
    #[must_use]
    pub const fn picks_beside_current(mut self, on: bool) -> Self {
        self.picks_beside_current = on;
        self
    }

    /// The empty-state text.
    #[must_use]
    pub fn empty(mut self, text: impl Into<Label>) -> Self {
        self.empty = Some(text.into());
        self
    }
}

/// What an item reads of its place in the collection; the item step joins it to the item's flags at step 12. The
/// fields marked "Measured" are the ones an auto row's measure reads (the row-hook contract on `ItemCx`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RowState {
    /// The row is selected. Measured.
    pub selected: bool,
    /// The row holds the lead. Measured.
    pub lead: bool,
    /// A drag would drop on the row.
    pub drop_target: bool,
    /// The row is being dragged.
    pub dragging: bool,
    /// The row's name is being edited. Measured.
    pub renaming: bool,
    /// The app marks the row.
    pub emphasized: bool,
    /// The collection's current item (`CollectionProps::current`). Measured.
    pub current: bool,
    /// An odd row of a striped list. Measured.
    pub striped: bool,
    /// A tree row whose children show. Measured.
    pub open: bool,
    /// The pointer is over the row and a press there would reach it.
    pub hovered: bool,
    /// The pointer is over the row, whether or not a press there would reach it: a drop target reads it while another
    /// component holds the press.
    pub pointer_over: bool,
    /// The collection holds a press on the row, which the item shows on the part it acts on.
    pub pressed: Option<RowPressed>,
    /// The collection asked for this row's menu and a menu hung from the collection is open: the row keeps its menu
    /// button shown.
    pub menu_open: bool,
    /// `Some(steps)` for a sub-row of a row above (`ItemSource::sub_row`): compact, its toggles `steps` indent steps
    /// into its content. Measured.
    pub sub_row: Option<usize>,
    /// The next row is a sub-row, which continues this row: no divider between them. Measured.
    pub continued: bool,
    /// The row is the collection's last: dividers fall between rows, so none under it. Measured.
    pub last: bool,
    /// The row is the collection's first, the mirror of `last`.
    pub first: bool,
    /// The activation key (Enter) activated the row this pass: a row with controls of its own may move focus into one.
    pub activated_by_key: bool,
}
