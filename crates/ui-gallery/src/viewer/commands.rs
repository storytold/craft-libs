//! The viewer's commands. Every action of a panel menu or the options bar is one of them, so a key, the palette
//! or a script reaches it too.

use craft_ui::contract::CommandSet;

/// The viewer's app-scope commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "gallery")]
#[non_exhaustive]
pub enum GalleryCommands {
    /// Put the shown workspace's panels back at their default places; open component tabs stay.
    #[command(label = "Reset Layout", icon = "undo")]
    ResetLayout,
    /// Give every shown story fresh state.
    #[command(label = "Reset Args")]
    ResetStories,
}
