//! `PanelRail`, `ToolRail`, `FlyoutButton` and `ToolbarDrawer`.

mod words;

// The panel rail and the Edit Toolbar drawer.
mod drawer;
mod panel_rail;

// The tool rail and its flyout button.
mod flyout;
mod tool;

pub use drawer::{DrawerGroup, ToolbarDrawer, ToolbarDrawerOutput, ToolbarEdit, apply_toolbar_edit, drop_edit};
pub use flyout::{FlyoutButton, FlyoutButtonStyle, FlyoutOutput, SlotEdit};
pub use panel_rail::{PanelRail, PanelRailOutput, PanelRailRequest, PanelRailStyle};
pub use tool::{
    DropAt, LastUsed, RailFooter, RailTool, SlotId, ToolDrag, ToolDropped, ToolRail, ToolRailOutput, ToolRailStyle, ToolSlot, column_toggle_icon,
    drop_in_slot, remembered, shown_tool, visible_slots,
};
pub use words::RailWords;
