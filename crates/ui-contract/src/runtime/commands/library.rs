//! The command sets the library itself declares.

use crate::runtime::frame::ChromeToggle;
use craft_behavior::EditCommand;

/// The edit commands (11.3). A focused component that lists one in `KEYS.edits` takes it as
/// `Intent::Edit`; otherwise it is an app command.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "edit")]
#[non_exhaustive]
pub enum EditCommands {
    /// Cut the selection.
    #[command(label = "Cut", modal = "declared", edit = "cut", icon = "scissors", default = "Cmd+X")]
    Cut,
    /// Copy the selection.
    #[command(label = "Copy", modal = "declared", edit = "copy", icon = "copy", default = "Cmd+C")]
    Copy,
    /// Paste the clipboard.
    #[command(label = "Paste", modal = "declared", edit = "paste", default = "Cmd+V")]
    Paste,
    /// Select everything.
    #[command(label = "Select All", modal = "declared", edit = "select_all", default = "Cmd+A")]
    SelectAll,
    /// Undo the last edit.
    #[command(label = "Undo", modal = "declared", edit = "undo", icon = "undo", default = "Cmd+Z")]
    Undo,
    /// Redo the last undone edit.
    #[command(label = "Redo", modal = "declared", edit = "redo", icon = "redo", default = "Cmd+Shift+Z")]
    Redo,
    /// Delete the selection. Delete and Backspace both bind it (the key labelled Delete on macOS reports Backspace).
    /// It carries no text-edit intent: a text part's own Delete key stays its deletion.
    #[command(label = "Delete", modal = "declared", icon = "trash", default = "Delete", default = "Backspace")]
    Delete,
}

impl EditCommands {
    /// Every edit command that a focused component can take as an `Intent::Edit`.
    pub const ALL: [EditCommands; 6] =
        [EditCommands::Cut, EditCommands::Copy, EditCommands::Paste, EditCommands::SelectAll, EditCommands::Undo, EditCommands::Redo];

    /// The intent a component takes this command as; `None` for a command that is always the app's.
    pub const fn edit(self) -> Option<EditCommand> {
        Some(match self {
            EditCommands::Cut => EditCommand::Cut,
            EditCommands::Copy => EditCommand::Copy,
            EditCommands::Paste => EditCommand::Paste,
            EditCommands::SelectAll => EditCommand::SelectAll,
            EditCommands::Undo => EditCommand::Undo,
            EditCommands::Redo => EditCommand::Redo,
            EditCommands::Delete => return None,
        })
    }

    /// The command an edit intent comes from.
    pub const fn of(command: EditCommand) -> Option<Self> {
        Some(match command {
            EditCommand::Cut => EditCommands::Cut,
            EditCommand::Copy => EditCommands::Copy,
            EditCommand::Paste => EditCommands::Paste,
            EditCommand::SelectAll => EditCommands::SelectAll,
            EditCommand::Undo => EditCommands::Undo,
            EditCommand::Redo => EditCommands::Redo,
            _ => return None,
        })
    }
}

/// The `window` commands (16.3): a floating container moves and resizes from the keyboard. macOS binds no chord to
/// them (9.2): the title-row menu offers both.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "window", scope = "window")]
#[non_exhaustive]
pub enum WindowCommand {
    /// Move the focused window with the arrow keys.
    #[command(label = "Move", default = "Alt+F7", no_default_on = "mac")]
    Move,
    /// Resize the focused window with the arrow keys.
    #[command(label = "Size", default = "Alt+F8", no_default_on = "mac")]
    Size,
}

// The chrome commands (spec 1.8): a bare Tab or Shift+Tab that passes the key stage's gate fires one for the window it
// was pressed in, and the window's shell hides its panels (`FrameOutput.chrome_toggles`). Rebinding a command moves its key.

/// The `window` commands that hide the panels of the window they run in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, craft_ui_macros::CommandSet)]
#[command(prefix = "window")]
#[non_exhaustive]
pub enum ChromeCommand {
    /// Show or hide every panel of the window.
    #[command(label = "Show or Hide Panels", default = "Tab", modal = "blocked")]
    TogglePanels,
    /// Show or hide every panel of the window except the tools.
    #[command(label = "Show or Hide Panels Except Tools", default = "Shift+Tab", modal = "blocked")]
    TogglePanelsKeepTools,
}

impl ChromeCommand {
    /// Both commands.
    pub const ALL: [ChromeCommand; 2] = [ChromeCommand::TogglePanels, ChromeCommand::TogglePanelsKeepTools];

    /// The toggle the command writes to the output.
    pub const fn toggle(self) -> ChromeToggle {
        match self {
            ChromeCommand::TogglePanels => ChromeToggle::Panels,
            ChromeCommand::TogglePanelsKeepTools => ChromeToggle::PanelsKeepTools,
        }
    }
}
