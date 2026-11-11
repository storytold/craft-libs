//! What a drag between the Edit Toolbar drawer and the rail carries, and where a tool dropped on a slot goes.

use super::data::SlotId;
use crate::contract::{DropMark, DropTarget};
use craft_behavior::DropOperation;
use egui::{Pos2, Rect};

/// The payload of the drag session between the Edit Toolbar drawer and the rail. Positions, not tools: each side
/// resolves them against the slots it was given.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ToolDrag {
    /// A row of the drawer: its group and its place in the group.
    #[non_exhaustive]
    Drawer {
        /// The group's index.
        group: usize,
        /// The row's place in the group.
        index: usize,
    },
    /// A rail slot's shown tool: the slot and the tool's place in it.
    #[non_exhaustive]
    Rail {
        /// The slot.
        slot: SlotId,
        /// The tool's place in the slot.
        index: usize,
    },
}

impl ToolDrag {
    /// The row `index` of drawer group `group`.
    pub const fn drawer(group: usize, index: usize) -> Self {
        ToolDrag::Drawer { group, index }
    }

    /// The tool at `index` of `slot`.
    pub const fn rail(slot: SlotId, index: usize) -> Self {
        ToolDrag::Rail { slot, index }
    }
}

/// Where a tool dropped on the rail goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum DropAt {
    /// A slot of its own, placed before the slot at index `before` of the host's slots (the slot count places it
    /// last).
    #[non_exhaustive]
    NewSlot {
        /// The index of the slot it goes before.
        before: usize,
    },
    /// Into an existing slot, at `index` among its tools.
    #[non_exhaustive]
    Slot {
        /// The slot.
        slot: SlotId,
        /// The place among the slot's tools.
        index: usize,
    },
}

impl DropAt {
    /// A slot of its own before the slot at `before`.
    pub const fn new_slot(before: usize) -> Self {
        DropAt::NewSlot { before }
    }

    /// Into `slot` at `index`.
    pub const fn into_slot(slot: SlotId, index: usize) -> Self {
        DropAt::Slot { slot, index }
    }
}

/// A drag dropped on the rail: what it carried and where it goes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ToolDropped {
    /// The payload of the drag.
    pub drag: ToolDrag,
    /// Where it goes.
    pub at: DropAt,
}

/// Where along its button a drop at `at` falls, from the leading edge (0) to the trailing edge (1): vertical in one
/// column, horizontal in two, mirrored in a right-to-left layout.
pub(crate) fn fraction(rect: Rect, at: Pos2, vertical: bool, rtl: bool) -> f32 {
    if vertical {
        (at.y - rect.top()) / rect.height().max(1.0)
    } else {
        let f = (at.x - rect.left()) / rect.width().max(1.0);
        if rtl { 1.0 - f } else { f }
    }
}

/// Where a tool dropped `fraction` of the way along the button of the slot at `index` goes: onto the middle it joins
/// the slot's group; onto the leading or trailing third it becomes a slot of its own beside it.
pub fn drop_in_slot(fraction: f32, slot: SlotId, index: usize, tools: usize) -> DropAt {
    if fraction < 0.3 {
        DropAt::new_slot(index)
    } else if fraction > 0.7 {
        DropAt::new_slot(index + 1)
    } else {
        DropAt::into_slot(slot, tools)
    }
}

/// A slot's button as a drop target: the drop's place comes from where over the button it falls.
pub(crate) struct SlotDrop {
    pub rect: Rect,
    pub slot: SlotId,
    pub index: usize,
    pub tools: usize,
    pub vertical: bool,
    pub rtl: bool,
}

impl DropTarget<ToolDrag> for SlotDrop {
    type Drop = ToolDropped;

    fn accept(&self, payload: &ToolDrag, at: Pos2, _: DropOperation) -> Option<ToolDropped> {
        let place = drop_in_slot(fraction(self.rect, at, self.vertical, self.rtl), self.slot, self.index, self.tools);
        Some(ToolDropped { drag: *payload, at: place })
    }

    fn mark(&self, drop: &ToolDropped) -> DropMark {
        let rect = self.rect;
        match drop.at {
            DropAt::Slot { .. } => DropMark::Into(rect),
            DropAt::NewSlot { before } => {
                let after = before > self.index;
                if self.vertical {
                    let y = if after { rect.bottom() } else { rect.top() };
                    DropMark::line(Pos2::new(rect.left(), y), Pos2::new(rect.right(), y))
                } else {
                    let leading = if self.rtl { rect.right() } else { rect.left() };
                    let trailing = if self.rtl { rect.left() } else { rect.right() };
                    let x = if after { trailing } else { leading };
                    DropMark::line(Pos2::new(x, rect.top()), Pos2::new(x, rect.bottom()))
                }
            }
        }
    }
}
