//! [`FillCommands`]: the workspace's keys, one scope on the workspace's dialog.

use craft_ui::contract::CommandSet;

use super::tool::FillTool;

/// The keys of the Content-Aware Fill workspace: Photoshop's tool letters, the brush size keys and the tint key. The
/// scope is the workspace's dialog, so the letters work while focus is anywhere inside it and not in a text field.
/// The zoom chords are the dialog's own view commands.
///
/// Each tool has its own slot in the strip, so a tool's letter picks that tool.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "fill", scope = "fill")]
#[non_exhaustive]
pub enum FillCommands {
    /// Sampling Brush.
    #[command(modal = "in_dialogs", label = "Sampling Brush Tool", icon = "brush", kind = "tool", default = "B")]
    SamplingBrush,
    /// Lasso.
    #[command(modal = "in_dialogs", label = "Lasso Tool", icon = "lasso", kind = "tool", default = "L")]
    Lasso,
    /// Hand.
    #[command(modal = "in_dialogs", label = "Hand Tool", icon = "hand", kind = "tool", default = "H")]
    Hand,
    /// Zoom.
    #[command(modal = "in_dialogs", label = "Zoom Tool", icon = "zoom_in", kind = "tool", default = "Z")]
    Zoom,
    /// Makes the Sampling Brush smaller.
    #[command(modal = "in_dialogs", label = "Smaller Brush", default = "[")]
    Smaller,
    /// Makes the Sampling Brush larger.
    #[command(modal = "in_dialogs", label = "Larger Brush", default = "]")]
    Larger,
    /// Hides or shows the sampling tint; Show Sampling Area stays as it is.
    #[command(modal = "in_dialogs", label = "Hide Sampling Area", default = "Cmd+H")]
    HideTint,
}

impl FillCommands {
    /// The command that picks `tool`.
    pub const fn of(tool: FillTool) -> FillCommands {
        match tool {
            FillTool::SamplingBrush => FillCommands::SamplingBrush,
            FillTool::Lasso => FillCommands::Lasso,
            FillTool::Hand => FillCommands::Hand,
            FillTool::Zoom => FillCommands::Zoom,
        }
    }

    /// The tool the command picks, for a tool command.
    pub const fn tool(self) -> Option<FillTool> {
        match self {
            FillCommands::SamplingBrush => Some(FillTool::SamplingBrush),
            FillCommands::Lasso => Some(FillTool::Lasso),
            FillCommands::Hand => Some(FillTool::Hand),
            FillCommands::Zoom => Some(FillTool::Zoom),
            _ => None,
        }
    }
}
