//! [`LiquifyCommands`]: Liquify's keys, one scope on the workspace's dialog.

use craft_ui::contract::CommandSet;

use super::tools::LiquifyTool;

/// The keys of the Liquify workspace: Photoshop's tool letters, the brush size keys and the mask keys. The scope is
/// the workspace's dialog, so the letters work while focus is anywhere inside it and not in a text field.
///
/// Each tool has its own slot in the strip, so a tool's letter picks that tool. Twirl Counterclockwise has no letter
/// (Photoshop's Twirl is one tool; Alt reverses it).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "liquify", scope = "liquify")]
#[non_exhaustive]
pub enum LiquifyCommands {
    /// Forward Warp.
    #[command(modal = "in_dialogs", label = "Forward Warp Tool", icon = "pointer", kind = "tool", default = "W")]
    ForwardWarp,
    /// Reconstruct.
    #[command(modal = "in_dialogs", label = "Reconstruct Tool", icon = "brush_cleaning", kind = "tool", default = "R")]
    Reconstruct,
    /// Smooth.
    #[command(modal = "in_dialogs", label = "Smooth Tool", icon = "droplet", kind = "tool", default = "E")]
    Smooth,
    /// Twirl Clockwise.
    #[command(modal = "in_dialogs", label = "Twirl Clockwise Tool", icon = "rotate_cw", kind = "tool", default = "C")]
    TwirlClockwise,
    /// Twirl Counterclockwise.
    #[command(modal = "in_dialogs", label = "Twirl Counterclockwise Tool", icon = "rotate_ccw", kind = "tool")]
    TwirlCounterclockwise,
    /// Pucker.
    #[command(modal = "in_dialogs", label = "Pucker Tool", icon = "scan", kind = "tool", default = "S")]
    Pucker,
    /// Bloat.
    #[command(modal = "in_dialogs", label = "Bloat Tool", icon = "maximize_2", kind = "tool", default = "B")]
    Bloat,
    /// Push Left.
    #[command(modal = "in_dialogs", label = "Push Left Tool", icon = "arrow_left_fixed", kind = "tool", default = "O")]
    PushLeft,
    /// Freeze Mask.
    #[command(modal = "in_dialogs", label = "Freeze Mask Tool", icon = "lock", kind = "tool", default = "F")]
    Freeze,
    /// Thaw Mask.
    #[command(modal = "in_dialogs", label = "Thaw Mask Tool", icon = "lock_open", kind = "tool", default = "D")]
    Thaw,
    /// Freeze Lasso: drag to freeze an area, Alt-drag to thaw it.
    #[command(
        modal = "in_dialogs",
        label = "Freeze Lasso: drag to freeze an area, Alt-drag to thaw it",
        icon = "lasso",
        kind = "tool",
        default = "L"
    )]
    FreezeLasso,
    /// Hand.
    #[command(modal = "in_dialogs", label = "Hand Tool", icon = "hand", kind = "tool", default = "H")]
    Hand,
    /// Zoom.
    #[command(modal = "in_dialogs", label = "Zoom Tool", icon = "zoom_in", kind = "tool", default = "Z")]
    Zoom,
    /// Makes the brush smaller.
    #[command(modal = "in_dialogs", label = "Smaller Brush", default = "[")]
    Smaller,
    /// Makes the brush larger.
    #[command(modal = "in_dialogs", label = "Larger Brush", default = "]")]
    Larger,
    /// Shows or hides the freeze mask; the mask itself stays.
    #[command(modal = "in_dialogs", label = "Show Mask", default = "Cmd+H")]
    ShowMask,
    /// Inverts the freeze mask.
    #[command(modal = "in_dialogs", label = "Invert All", default = "Cmd+I")]
    InvertMask,
    /// Thaws the whole mask.
    #[command(modal = "in_dialogs", label = "Clear Mask", default = "Cmd+D")]
    ClearMask,
}

impl LiquifyCommands {
    /// The command that picks `tool`.
    pub const fn of(tool: LiquifyTool) -> LiquifyCommands {
        match tool {
            LiquifyTool::ForwardWarp => LiquifyCommands::ForwardWarp,
            LiquifyTool::Reconstruct => LiquifyCommands::Reconstruct,
            LiquifyTool::Smooth => LiquifyCommands::Smooth,
            LiquifyTool::TwirlClockwise => LiquifyCommands::TwirlClockwise,
            LiquifyTool::TwirlCounterclockwise => LiquifyCommands::TwirlCounterclockwise,
            LiquifyTool::Pucker => LiquifyCommands::Pucker,
            LiquifyTool::Bloat => LiquifyCommands::Bloat,
            LiquifyTool::PushLeft => LiquifyCommands::PushLeft,
            LiquifyTool::Freeze => LiquifyCommands::Freeze,
            LiquifyTool::Thaw => LiquifyCommands::Thaw,
            LiquifyTool::FreezeLasso => LiquifyCommands::FreezeLasso,
            LiquifyTool::Hand => LiquifyCommands::Hand,
            LiquifyTool::Zoom => LiquifyCommands::Zoom,
        }
    }

    /// The tool the command picks; `None` for the brush, view and mask commands.
    pub const fn tool(self) -> Option<LiquifyTool> {
        Some(match self {
            LiquifyCommands::ForwardWarp => LiquifyTool::ForwardWarp,
            LiquifyCommands::Reconstruct => LiquifyTool::Reconstruct,
            LiquifyCommands::Smooth => LiquifyTool::Smooth,
            LiquifyCommands::TwirlClockwise => LiquifyTool::TwirlClockwise,
            LiquifyCommands::TwirlCounterclockwise => LiquifyTool::TwirlCounterclockwise,
            LiquifyCommands::Pucker => LiquifyTool::Pucker,
            LiquifyCommands::Bloat => LiquifyTool::Bloat,
            LiquifyCommands::PushLeft => LiquifyTool::PushLeft,
            LiquifyCommands::Freeze => LiquifyTool::Freeze,
            LiquifyCommands::Thaw => LiquifyTool::Thaw,
            LiquifyCommands::FreezeLasso => LiquifyTool::FreezeLasso,
            LiquifyCommands::Hand => LiquifyTool::Hand,
            LiquifyCommands::Zoom => LiquifyTool::Zoom,
            _ => return None,
        })
    }
}
