//! `ToolRail`, `FlyoutButton` and `ToolbarDrawer`.

mod words;

// The Edit Toolbar drawer.
mod drawer;

// The tool rail and its flyout button.
mod flyout;
mod tool;

pub use drawer::{DrawerGroup, ToolbarDrawer, ToolbarDrawerOutput, ToolbarEdit, apply_toolbar_edit, drop_edit};
pub use flyout::{FlyoutButton, FlyoutButtonStyle, FlyoutOutput, SlotEdit};
pub use tool::{
    DropAt, LastUsed, RailFooter, RailTool, SlotId, ToolDrag, ToolDropped, ToolRail, ToolRailOutput, ToolRailStyle, ToolSlot, drop_in_slot,
    remembered, shown_tool, visible_slots,
};
pub use words::RailWords;
