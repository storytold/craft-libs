//! `ToolRail`, `FlyoutButton` and `ToolbarDrawer`.

mod words;

// The Edit Toolbar drawer.
pub(crate) mod drawer;

// The tool rail and its flyout button.
pub(crate) mod flyout;
pub(crate) mod tool;

pub use drawer::{DrawerGroup, DrawerKind, ToolbarDrawer, ToolbarDrawerOutput, ToolbarEdit, apply_toolbar_edit, drop_edit};
pub use flyout::{FlyoutButton, FlyoutButtonStyle, FlyoutOutput, SlotEdit};
pub use tool::{
    DropAt, LastUsed, RailDensity, RailFooter, RailTool, SlotId, ToolDrag, ToolDropped, ToolRail, ToolRailOutput, ToolRailStyle, ToolSlot, ToolTips,
    drop_in_slot, remembered, shown_tool, visible_slots,
};
pub use words::RailWords;
