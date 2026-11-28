//! The workspace's keys, in its command scope.

use craft_ui::contract::CommandSet;

/// The keys of the RAW Develop workspace. The clipping warnings are the scope's own commands.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "rawDevelop", scope = "rawDevelop")]
#[non_exhaustive]
pub enum RawDevelopCommands {
    /// Shows the picture before the settings while held.
    #[command(modal = "in_dialogs", label = "Before", kind = "momentary", default = "Y")]
    Before,
    /// The sampler tool.
    #[command(modal = "in_dialogs", label = "Color Sampler Tool", default = "S")]
    SamplerTool,
    /// The hand tool.
    #[command(modal = "in_dialogs", label = "Hand Tool", default = "H")]
    Hand,
    /// The zoom tool.
    #[command(modal = "in_dialogs", label = "Zoom Tool", default = "Z")]
    Zoom,
}
