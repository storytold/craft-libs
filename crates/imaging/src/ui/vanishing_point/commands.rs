//! [`VanishingPointCommands`]: Vanishing Point's keys, one scope on the workspace's dialog.

use craft_ui::contract::CommandSet;

use super::VanishingTool;

/// The keys of the Vanishing Point workspace: Photoshop's tool letters and the New Plane command. The scope is the
/// workspace's dialog, so the letters work while focus is anywhere inside it and not in a text field.
///
/// Each tool has its own slot in the strip, so a tool's letter picks that tool. New Plane has no default chord: it
/// opens the form of the four corners that Create Plane and Edit Plane have as their keyboard path.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "vanishingPoint", scope = "vanishingPoint")]
#[non_exhaustive]
pub enum VanishingPointCommands {
    /// Edit Plane.
    #[command(modal = "in_dialogs", label = "Edit Plane Tool", icon = "pointer", kind = "tool", default = "V")]
    EditPlane,
    /// Create Plane.
    #[command(modal = "in_dialogs", label = "Create Plane Tool", icon = "grid", kind = "tool", default = "C")]
    CreatePlane,
    /// Stamp.
    #[command(modal = "in_dialogs", label = "Stamp Tool", icon = "stamp", kind = "tool", default = "S")]
    Stamp,
    /// Hand.
    #[command(modal = "in_dialogs", label = "Hand Tool", icon = "hand", kind = "tool", default = "H")]
    Hand,
    /// Zoom.
    #[command(modal = "in_dialogs", label = "Zoom Tool", icon = "zoom_in", kind = "tool", default = "Z")]
    Zoom,
    /// Opens the form that adds a plane from its four corners.
    #[command(modal = "in_dialogs", label = "New Plane…", icon = "plus")]
    NewPlane,
}

/// A button of the strip: the editor's three tools, Hand and Zoom.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum VanishingPointStripTool {
    /// The editor's tools: [`VanishingTool`] says which.
    #[default]
    Editor,
    /// The Hand tool.
    Hand,
    /// The Zoom tool.
    Zoom,
}

/// A slot of the strip.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(super) enum Pick {
    EditPlane,
    CreatePlane,
    Stamp,
    Hand,
    Zoom,
}

impl Pick {
    /// The slot a strip tool and editor tool show.
    pub(super) fn of(strip: VanishingPointStripTool, tool: VanishingTool) -> Pick {
        match (strip, tool) {
            (VanishingPointStripTool::Hand, _) => Pick::Hand,
            (VanishingPointStripTool::Zoom, _) => Pick::Zoom,
            (VanishingPointStripTool::Editor, VanishingTool::EditPlane) => Pick::EditPlane,
            (VanishingPointStripTool::Editor, VanishingTool::CreatePlane) => Pick::CreatePlane,
            (VanishingPointStripTool::Editor, VanishingTool::Stamp) => Pick::Stamp,
        }
    }

    /// The strip tool and editor tool a slot picks; the editor tool is `None` for Hand and Zoom.
    pub(super) fn picks(self) -> (VanishingPointStripTool, Option<VanishingTool>) {
        match self {
            Pick::EditPlane => (VanishingPointStripTool::Editor, Some(VanishingTool::EditPlane)),
            Pick::CreatePlane => (VanishingPointStripTool::Editor, Some(VanishingTool::CreatePlane)),
            Pick::Stamp => (VanishingPointStripTool::Editor, Some(VanishingTool::Stamp)),
            Pick::Hand => (VanishingPointStripTool::Hand, None),
            Pick::Zoom => (VanishingPointStripTool::Zoom, None),
        }
    }
}

impl VanishingPointCommands {
    /// The command that picks `pick`.
    pub(super) const fn of(pick: Pick) -> VanishingPointCommands {
        match pick {
            Pick::EditPlane => VanishingPointCommands::EditPlane,
            Pick::CreatePlane => VanishingPointCommands::CreatePlane,
            Pick::Stamp => VanishingPointCommands::Stamp,
            Pick::Hand => VanishingPointCommands::Hand,
            Pick::Zoom => VanishingPointCommands::Zoom,
        }
    }

    /// The slot the command picks; `None` for New Plane.
    pub(super) const fn pick(self) -> Option<Pick> {
        Some(match self {
            VanishingPointCommands::EditPlane => Pick::EditPlane,
            VanishingPointCommands::CreatePlane => Pick::CreatePlane,
            VanishingPointCommands::Stamp => Pick::Stamp,
            VanishingPointCommands::Hand => Pick::Hand,
            VanishingPointCommands::Zoom => Pick::Zoom,
            _ => return None,
        })
    }
}
