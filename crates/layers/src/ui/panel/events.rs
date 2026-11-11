//! `LayersEvent` and the types it carries (`LayerSelect`, `LayerToggle`, `LayerDrop`, `LayerMove`,
//! `SubRowMove`, `RowTransfer`).
//!
//! The panel reports what the user did in the order it happened; the app applies each event with
//! its own commands and undo, in the pass it is reported.

use craft_behavior::{EditEvent, SelectionChange};
use craft_keymap::Mods;
use craft_theme::LabelColor;
use craft_ui::contract::OpenChange;

use super::footer::{LayersFooterAction, LayersFooterButton};
use crate::ui::filter::LayerFilter;
use crate::ui::rows::LayerPropertyEdit;
use crate::ui::tree::{LayerPart, LayerRowKey, SubRowGroup};

/// What the Layers panel reports.
///
/// ```ignore
/// for event in out.events {
///     match event {
///         LayersEvent::Select(s) => app.select(s.change, s.part),
///         LayersEvent::Toggle(t) => app.toggle(t),          // the events of one sweep are Updates of one gesture closed by a Commit; a plain press is a Set
///         LayersEvent::Properties(p) => app.set_properties(p),
///         LayersEvent::Drop(EditEvent::Set { after, .. }) => app.drop_rows(after),
///         _ => {}
///     }
/// }
/// ```
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum LayersEvent<K> {
    /// A click on a row (Cmd/Ctrl toggles, Shift extends), a selection key, or a click on the empty
    /// space below the rows (`Clear`). A right-click outside the selection selects the row first.
    /// The app sets its selection by the change and its target from `part`.
    Select(LayerSelect<K>),
    /// An eye or lock cell: a click sets its row; a drag down the column sets every
    /// row crossed to the first row's new state, under one gesture and so one undo step.
    Toggle(LayerToggle<K>),
    /// Alt-click on an eye: show only this layer, or restore when it already is (Photoshop).
    ShowOnly(LayerRowKey<K>),
    /// Escape during an Alt eye sweep: undo the `ShowOnly` of that sweep, which turned the other layers' eyes off.
    /// The key is the layer the sweep began on.
    UndoShowOnly(LayerRowKey<K>),
    /// A pick in the eye cell's colour menu (None and the label colours): for every selected
    /// layer when the row is in a multi-selection, else for the row.
    #[non_exhaustive]
    LabelColor {
        /// The layers to colour.
        rows: Vec<K>,
        /// The colour; `None` for none.
        color: Option<LabelColor>,
    },
    /// A group asked to open or close: its disclosure (Alt for every group), the arrow keys, a
    /// screen reader, or the reveal of the active layer. A view change: no undo step.
    Open(OpenChange<K>),
    /// A layer's effects and Smart Filter rows asked to open or close: the `fx` disclosure, or the
    /// effects commands on the lead; `all` (Alt) for every layer.
    #[non_exhaustive]
    FxOpen {
        /// The layer.
        layer: K,
        /// Open or close.
        open: bool,
        /// Every layer takes the state.
        all: bool,
    },
    /// A rename committed: never empty after trimming, never the unchanged name.
    #[non_exhaustive]
    Rename {
        /// The layer.
        layer: K,
        /// The new name, trimmed.
        text: String,
    },
    /// A double click or Enter where it isn't a rename: the app opens Layer Style, a fill's or
    /// adjustment's options, a smart object's contents, edits the type, or converts the
    /// Background.
    #[non_exhaustive]
    Activate {
        /// The row: a layer, or an effects or Smart Filter row (Layer Style on that effect, the
        /// filter's options).
        row: LayerRowKey<K>,
        /// What was activated.
        on: ActivateOn,
    },
    /// A thumbnail click that isn't a select: Cmd/Ctrl-click loads it as a selection (Shift adds,
    /// Alt subtracts, Shift+Alt intersects; `SelectionOperation::from_modifiers`), Shift-click on
    /// a mask toggles it, Alt-click on a mask views it, Alt-click on the layer thumbnail fits the
    /// layer on screen.
    #[non_exhaustive]
    ThumbClick {
        /// The layer.
        layer: K,
        /// The thumbnail.
        part: LayerPart,
        /// The modifiers held.
        mods: Mods,
    },
    /// The chain before a mask thumbnail: link or unlink the mask.
    #[non_exhaustive]
    MaskLink {
        /// The layer.
        layer: K,
        /// The mask.
        part: LayerPart,
        /// The state asked for.
        linked: bool,
    },
    /// An edit in the blend, opacity, lock and fill rows: the app applies it to every selected
    /// layer.
    Properties(LayerPropertyEdit),
    /// The filter bar's edit, for a filter binding that does not write through.
    Filter(EditEvent<LayerFilter>),
    /// A drop of rows (a drag, Alt+Up and Down, or a stacking command): one `Set`, one undo step.
    Drop(EditEvent<LayerDrop<K>>),
    /// A press on a footer button.
    Footer(LayersFooterAction),
    /// Rows dropped on a footer button: Delete deletes layers, clears effects, removes filters,
    /// deletes a mask (the app asks first); New Layer duplicates layers; New Group groups them.
    #[non_exhaustive]
    FooterDrop {
        /// The button.
        button: LayersFooterButton,
        /// What was dropped: layer rows, effects or filter rows, or one [`LayerRowKey::Mask`].
        rows: Vec<LayerRowKey<K>>,
    },
    /// A pick in the host's row menu ([`LayersPanel::row_menu`](super::LayersPanel::row_menu)).
    #[non_exhaustive]
    Menu {
        /// Where the menu was opened.
        target: MenuTarget<K>,
        /// The host's command.
        command: String,
    },
}

/// A selection change, in layers.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayerSelect<Id> {
    /// The change: `Set` the layers (the last becomes the lead), `Toggle` one, or `Clear`. A click
    /// on an effects or Smart Filter row picks its layer.
    pub change: SelectionChange<Id>,
    /// The thumbnail the click landed on, which becomes the target; `None` elsewhere on the row
    /// and for keys (the target stays).
    pub part: Option<LayerPart>,
}

impl<Id> LayerSelect<Id> {
    /// A selection change with an optional thumbnail target.
    pub fn new(change: SelectionChange<Id>, part: Option<LayerPart>) -> Self {
        Self { change, part }
    }
}

/// An edit of an eye or lock cell.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct LayerToggle<Id> {
    /// The rows the edit sets, in the order the pointer crossed them; the first is the row the gesture started on
    /// in its first event.
    pub rows: Vec<LayerRowKey<Id>>,
    /// Which column.
    pub column: LayerColumn,
    /// The new state of every row. The events of one sweep are `Update`s of one gesture, which arrive as the pointer
    /// crosses more rows, closed by a `Commit` on release; a plain press is a `Set`. Escape during the sweep reports
    /// the rows it crossed as a `Cancel` of that gesture that carries the state they had before.
    pub edit: EditEvent<bool>,
}

impl<Id> LayerToggle<Id> {
    /// An edit of `column` on `rows`.
    pub fn new(rows: Vec<LayerRowKey<Id>>, column: LayerColumn, edit: EditEvent<bool>) -> Self {
        Self { rows, column, edit }
    }
}

/// A toggle column of the rows.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerColumn {
    /// The eye.
    Visibility,
    /// The lock column ([`LayersPanel::lock_column`](super::LayersPanel::lock_column)), or the
    /// lock indicator's press (which only unlocks).
    Lock,
}

/// What an activation landed on.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ActivateOn {
    /// A thumbnail.
    Thumbnail(LayerPart),
    /// The rest of the row, or Enter.
    Row,
}

/// Where a dropped layer lands relative to its target.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LayerPosition {
    /// Above the target.
    Before,
    /// Below the target.
    After,
    /// Inside the target group.
    Into,
}

/// What a drop of rows means.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum LayerDrop<Id> {
    /// Layers moved or copied.
    Layers(LayerMove<Id>),
    /// A Smart Filter moved within its stack.
    SubRows(SubRowMove<Id>),
    /// Effects or Smart Filters moved to another layer.
    Transfer(RowTransfer<Id>),
}

/// Layers moved or copied next to a target layer.
///
/// A place has one encoding: `Into` a group at its first slot, else `Before` the next sibling, else `After` the last
/// sibling. A place beside a layer's effects or Smart Filter rows is `After` that layer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct LayerMove<Id> {
    /// The layers, in display order.
    pub layers: Vec<Id>,
    /// The layer they land by.
    pub target: Id,
    /// Before, after or inside the target.
    pub position: LayerPosition,
    /// Alt: copy instead of move.
    pub copy: bool,
}

impl<Id> LayerMove<Id> {
    /// `layers` land `position` of `target`.
    pub fn new(layers: Vec<Id>, target: Id, position: LayerPosition, copy: bool) -> Self {
        Self { layers, target, position, copy }
    }
}

/// A Smart Filter moved within one layer's stack.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct SubRowMove<Id> {
    /// The layer the stack belongs to.
    pub layer: Id,
    /// The stack.
    pub group: SubRowGroup,
    /// The index it came from.
    pub from: usize,
    /// The index it lands at once it is taken out.
    pub to: usize,
    /// Alt: copy instead of move.
    pub copy: bool,
}

/// Effects or Smart Filters dropped on another layer.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct RowTransfer<Id> {
    /// The rows.
    pub rows: Vec<LayerRowKey<Id>>,
    /// The layer they go to.
    pub to: Id,
    /// Alt: copy instead of move.
    pub copy: bool,
}

/// Where the host's row menu was opened.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MenuTarget<Id> {
    /// The row.
    pub row: LayerRowKey<Id>,
    /// The thumbnail the press landed on, if known.
    pub part: Option<LayerPart>,
    /// The row is one of several selected rows: the command applies to all of them.
    pub on_selection: bool,
}
