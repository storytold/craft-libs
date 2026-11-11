//! [`SelectMaskCommands`]: Select and Mask's keys, one scope on the workspace's dialog.

use craft_ui::contract::CommandSet;

use super::tools::SelectMaskTool;

/// The keys of the Select and Mask workspace: Photoshop's tool letters and the brush size keys. The scope is the
/// workspace's dialog, so the letters work while focus is anywhere inside it and not in a text field.
///
/// Quick Selection and Object Selection share W, Lasso and Polygonal Lasso share L: a press of the shared chord
/// (with or without Shift) cycles the group, and from outside the group picks the group's last-used tool. The view
/// keys (O M V A T K Y, F, J, P and X held) are `MaskViewKeys`, which the app forwards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "selectMask", scope = "selectMask")]
#[non_exhaustive]
pub enum SelectMaskCommands {
    /// Quick Selection.
    #[command(
        modal = "in_dialogs",
        label = "Quick Selection Tool",
        icon = "quick_selection",
        kind = "tool",
        group = "w",
        default = "W",
        default = "Shift+W"
    )]
    Quick,
    /// Object Selection.
    #[command(
        modal = "in_dialogs",
        label = "Object Selection Tool",
        icon = "object_select",
        kind = "tool",
        group = "w",
        default = "W",
        default = "Shift+W"
    )]
    Object,
    /// Refine Edge Brush.
    #[command(modal = "in_dialogs", label = "Refine Edge Brush Tool", icon = "brush_cleaning", kind = "tool", default = "R")]
    RefineEdge,
    /// Brush.
    #[command(modal = "in_dialogs", label = "Brush Tool", icon = "brush", kind = "tool", default = "B")]
    Brush,
    /// Lasso.
    #[command(modal = "in_dialogs", label = "Lasso Tool", icon = "lasso", kind = "tool", group = "l", default = "L", default = "Shift+L")]
    Lasso,
    /// Polygonal Lasso.
    #[command(
        modal = "in_dialogs",
        label = "Polygonal Lasso Tool",
        icon = "pentagon",
        kind = "tool",
        group = "l",
        default = "L",
        default = "Shift+L"
    )]
    PolygonalLasso,
    /// Hand.
    #[command(modal = "in_dialogs", label = "Hand Tool", icon = "hand", kind = "tool", default = "H")]
    Hand,
    /// Zoom.
    #[command(modal = "in_dialogs", label = "Zoom Tool", icon = "zoom_in", kind = "tool", default = "Z")]
    Zoom,
    /// Makes the current brush tool's brush smaller.
    #[command(modal = "in_dialogs", label = "Smaller Brush", default = "[")]
    Smaller,
    /// Makes the current brush tool's brush larger.
    #[command(modal = "in_dialogs", label = "Larger Brush", default = "]")]
    Larger,
}

impl SelectMaskCommands {
    /// The command that picks `tool`.
    pub const fn of(tool: SelectMaskTool) -> SelectMaskCommands {
        match tool {
            SelectMaskTool::Quick => SelectMaskCommands::Quick,
            SelectMaskTool::Object => SelectMaskCommands::Object,
            SelectMaskTool::RefineEdge => SelectMaskCommands::RefineEdge,
            SelectMaskTool::Brush => SelectMaskCommands::Brush,
            SelectMaskTool::Lasso => SelectMaskCommands::Lasso,
            SelectMaskTool::PolygonalLasso => SelectMaskCommands::PolygonalLasso,
            SelectMaskTool::Hand => SelectMaskCommands::Hand,
            SelectMaskTool::Zoom => SelectMaskCommands::Zoom,
        }
    }

    /// The tool the command picks; `None` for the brush size commands.
    pub const fn tool(self) -> Option<SelectMaskTool> {
        Some(match self {
            SelectMaskCommands::Quick => SelectMaskTool::Quick,
            SelectMaskCommands::Object => SelectMaskTool::Object,
            SelectMaskCommands::RefineEdge => SelectMaskTool::RefineEdge,
            SelectMaskCommands::Brush => SelectMaskTool::Brush,
            SelectMaskCommands::Lasso => SelectMaskTool::Lasso,
            SelectMaskCommands::PolygonalLasso => SelectMaskTool::PolygonalLasso,
            SelectMaskCommands::Hand => SelectMaskTool::Hand,
            SelectMaskCommands::Zoom => SelectMaskTool::Zoom,
            SelectMaskCommands::Smaller | SelectMaskCommands::Larger => return None,
        })
    }
}
