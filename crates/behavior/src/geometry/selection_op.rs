//! [`SelectionOperation`]: how a new selection combines with the current one.

use craft_keymap::Mods;

use crate::cursor::CursorBadge;

/// How a selection made by a click or a drag combines with the current one: the rule the Channels
/// panel's ⌘-click, a mask thumbnail's ⌘-click and the selection tools share (Photoshop's).
///
/// With the gesture's own modifier held (⌘ on a thumbnail; none for a selection tool), Shift adds,
/// Alt (Option) subtracts, Shift and Alt together intersect, and nothing else replaces
/// ([`from_mods`](Self::from_mods)). Command and Ctrl are not read: the caller has
/// already used them to recognise the gesture. A drag over list items combines by the item rule instead
/// ([`MarqueeMode`](crate::item_marquee::MarqueeMode): Cmd or Ctrl toggles, no intersect).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum SelectionOperation {
    /// Replaces the selection.
    #[default]
    New,
    /// Adds to it.
    Add,
    /// Subtracts from it.
    Subtract,
    /// Keeps only the overlap.
    Intersect,
}

impl SelectionOperation {
    /// Every operation, in Photoshop's options-bar order.
    pub const ALL: [SelectionOperation; 4] =
        [SelectionOperation::New, SelectionOperation::Add, SelectionOperation::Subtract, SelectionOperation::Intersect];

    /// The operation `mods` ask for: Shift and Alt `Intersect`, Shift `Add`, Alt `Subtract`,
    /// else `New`.
    pub fn from_mods(mods: Mods) -> SelectionOperation {
        match (mods.shift, mods.alt) {
            (true, true) => SelectionOperation::Intersect,
            (true, false) => SelectionOperation::Add,
            (false, true) => SelectionOperation::Subtract,
            (false, false) => SelectionOperation::New,
        }
    }

    /// The operation a selection tool's gesture uses (Photoshop's): the options bar's `bar`,
    /// overridden by `mods` as [`from_mods`](Self::from_mods) reads them (⇧ add, ⌥
    /// subtract, ⇧⌥ intersect). With nothing selected (`has_selection` false) there is nothing to
    /// subtract from or intersect with, so ⌥ or ⇧⌥ over a bar of `New` gives `New`; a bar of
    /// `Subtract` or `Intersect` is kept as it is (storytold/photocraft#1106). The tool calls it with the
    /// mods of the press (or [`DraftEvent::Started`](super::draft::DraftEvent::Started)'s
    /// `intent`) and keeps the answer for the gesture; the mode badge before the press follows the
    /// live modifiers through the same call.
    pub fn resolve(bar: SelectionOperation, mods: Mods, has_selection: bool) -> SelectionOperation {
        let operation = if mods.shift || mods.alt { Self::from_mods(mods) } else { bar };
        let nothing_to_combine = mods.alt && !has_selection && bar == SelectionOperation::New;
        if nothing_to_combine && matches!(operation, SelectionOperation::Subtract | SelectionOperation::Intersect) {
            SelectionOperation::New
        } else {
            operation
        }
    }

    /// The badge beside the selection pointer: `Add` a plus, `Subtract` a minus, `Intersect` a
    /// cross, `New` none.
    pub fn badge(self) -> Option<CursorBadge> {
        match self {
            SelectionOperation::New => None,
            SelectionOperation::Add => Some(CursorBadge::Add),
            SelectionOperation::Subtract => Some(CursorBadge::Subtract),
            SelectionOperation::Intersect => Some(CursorBadge::Intersect),
        }
    }

    /// The operation's stable name for commands and settings: `"new"`, `"add"`, `"subtract"`,
    /// `"intersect"` (PhotoCraft's `select.loadSelection` operation values).
    pub const fn name(self) -> &'static str {
        match self {
            SelectionOperation::New => "new",
            SelectionOperation::Add => "add",
            SelectionOperation::Subtract => "subtract",
            SelectionOperation::Intersect => "intersect",
        }
    }
}
