//! The Navigator's panel menu as commands.

/// The commands of the Navigator panel. They are app commands: the app that owns the Navigator's settings opens Panel
/// Options when the command runs, and the dock's panel menu lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui_macros::CommandSet)]
#[command(prefix = "canvasNavigator")]
#[non_exhaustive]
pub enum NavigatorCommands {
    /// Open Panel Options for the Navigator.
    #[command(label = "Panel Options…")]
    Options,
}
