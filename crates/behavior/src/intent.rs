//! Intents: a key, an AccessKit action or typed text turned into meaning (spec 9.2).

use crate::part::PartRef;
use std::sync::Arc;

/// A logical move. `Back` and `Forward` follow the reading direction; the runtime flips physical arrows
/// under right-to-left text when it delivers them.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum NavMove {
    /// Toward the reading start.
    Back,
    /// Toward the reading end.
    Forward,
    /// Up one row.
    Up,
    /// Down one row.
    Down,
    /// Up one page.
    PageUp,
    /// Down one page.
    PageDown,
    /// To the first position.
    Start,
    /// To the last position.
    End,
}

/// How far a move reaches.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum Granularity {
    /// One character, row or cell.
    #[default]
    Unit,
    /// One word.
    Word,
    /// To a line boundary.
    Line,
    /// One paragraph.
    Paragraph,
    /// To a document boundary.
    Document,
}

/// The modifiers of a move, read from the key table.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct MoveMods {
    /// The selection extends (Shift).
    pub extend: bool,
    /// The lead moves without selecting (Ctrl, Cmd on macOS).
    pub lead: bool,
    /// How far the move reaches.
    pub by: Granularity,
    /// A value control steps by its finest step (Alt).
    pub fine: bool,
}

impl MoveMods {
    /// A move by one unit with no modifier.
    pub const PLAIN: MoveMods = MoveMods { extend: false, lead: false, by: Granularity::Unit, fine: false };

    /// Modifiers with the given parts and no fine flag.
    pub const fn new(extend: bool, lead: bool, by: Granularity) -> MoveMods {
        MoveMods { extend, lead, by, fine: false }
    }

    /// The same modifiers with the fine flag set to `on`.
    pub const fn fine(mut self, on: bool) -> MoveMods {
        self.fine = on;
        self
    }
}

impl Default for MoveMods {
    fn default() -> Self {
        MoveMods::PLAIN
    }
}

/// An edit command a component lists in its `KeyUse` to receive it as `Intent::Edit`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum EditCommand {
    /// Cut the selection.
    Cut,
    /// Copy the selection.
    Copy,
    /// Paste the clipboard.
    Paste,
    /// Select everything.
    SelectAll,
    /// Undo the last edit.
    Undo,
    /// Redo the last undone edit.
    Redo,
}

/// A position in a text document.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct TextPos {
    /// The line.
    pub line: usize,
    /// The byte offset in the line.
    pub byte: usize,
}

impl TextPos {
    /// The position `byte` bytes into `line`.
    pub const fn new(line: usize, byte: usize) -> TextPos {
        TextPos { line, byte }
    }
}

/// An AccessKit action with no intent of its own, or any action on a non-focusable sub-node.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub struct AccessRequest {
    /// The requested action.
    pub action: accesskit::Action,
    /// The sub-node's part, `None` for the component's own node.
    pub target: Option<PartRef>,
    /// The action's data.
    pub data: Option<accesskit::ActionData>,
}

impl AccessRequest {
    /// A request for `action` on `target`.
    pub const fn new(action: accesskit::Action, target: Option<PartRef>, data: Option<accesskit::ActionData>) -> Self {
        AccessRequest { action, target, data }
    }
}

/// What the runtime routes to a focused component.
#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum Intent {
    /// Click, Enter or Space on a control.
    Activate,
    /// Enter that no component took.
    Submit,
    /// Ctrl+Enter, Cmd+Enter.
    SubmitForce,
    /// Escape, or its AccessKit equivalent.
    Cancel,
    /// A navigation move.
    Move(NavMove, MoveMods),
    /// Delete or Backspace.
    #[non_exhaustive]
    Delete {
        /// Delete toward the reading end.
        forward: bool,
        /// How much to delete.
        by: Granularity,
        /// Shift was held: a component that gives Shift+Delete its own meaning (Fill, a ripple delete) takes
        /// this one, and `take_exact` of a plain delete leaves it.
        shift: bool,
    },
    /// Space on a collection: the item step turns it into the collection's `SpaceAction`.
    Space,
    /// One typed digit, for a role that takes digits (`KeyRole::digits`).
    Digit(u8),
    /// Rename the lead.
    Rename,
    /// Open the context menu.
    ContextMenu,
    /// Reset a value control.
    Reset,
    /// Typed text; one event can carry several characters.
    Text(Arc<str>),
    /// Tab in a component whose key filter keeps it.
    Tab,
    /// An edit command the focused component lists.
    Edit(EditCommand),
    /// Switch to the next or previous page.
    #[non_exhaustive]
    SwitchPage {
        /// To the next page.
        forward: bool,
    },
    /// Move the lead of a reorderable collection.
    #[non_exhaustive]
    MoveItem {
        /// Toward the end.
        forward: bool,
    },
    /// Open a `Trigger::Hold` or dropdown opener.
    Open,
    /// Close the dropdown of a dropdown opener.
    Close,
    /// An AccessKit action with no intent of its own.
    Access(AccessRequest),
}

impl Intent {
    /// `Delete` of `by`, forward or back.
    pub const fn delete(forward: bool, by: Granularity) -> Intent {
        Intent::Delete { forward, by, shift: false }
    }

    /// `SwitchPage` to the next page when `forward`.
    pub const fn switch_page(forward: bool) -> Intent {
        Intent::SwitchPage { forward }
    }

    /// `MoveItem` toward the end when `forward`.
    pub const fn move_item(forward: bool) -> Intent {
        Intent::MoveItem { forward }
    }

    /// Whether `self` and `other` are the same kind of intent, whatever their data.
    pub fn same_kind(&self, other: &Intent) -> bool {
        std::mem::discriminant(self) == std::mem::discriminant(other)
    }
}
