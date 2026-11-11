//! `WideAngleCommands`: the tool keys of the Adaptive Wide Angle workspace.

/// The tools of the strip, one command each, run on the workspace's dialog scope. Photoshop's set without Polygon
/// Constraint and Move.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, craft_ui::contract::CommandSet)]
#[command(prefix = "wideAngle", scope = "wideAngle")]
#[non_exhaustive]
pub enum WideAngleCommands {
    /// Draw constraints.
    #[command(modal = "in_dialogs", label = "Constraint Tool", icon = "slash", kind = "tool", group = "wideAngleTools", default = "A")]
    Constraint,
    /// Pan the view.
    #[command(modal = "in_dialogs", label = "Hand Tool", icon = "hand", kind = "tool", group = "wideAngleTools", default = "H")]
    Hand,
    /// Zoom the view.
    #[command(modal = "in_dialogs", label = "Zoom Tool", icon = "zoom_in", kind = "tool", group = "wideAngleTools", default = "Z")]
    Zoom,
}
