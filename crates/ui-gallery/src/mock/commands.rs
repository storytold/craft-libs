//! The app commands of the menus.

use super::model::Panel;
use craft_ui::contract::CommandSet;

/// The mock's app commands: the File, Image, Layer, Filter, View and Window menu rows. A row shows the command's label
/// and chord and fires it; the window's body takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, CommandSet)]
#[command(prefix = "mockApp")]
#[non_exhaustive]
pub enum AppCommand {
    /// A new document.
    #[command(label = "New…", default = "Cmd+N")]
    New,
    /// Opens a document.
    #[command(label = "Open…", default = "Cmd+O")]
    Open,
    /// Saves the document.
    #[command(label = "Save", default = "Cmd+S")]
    Save,
    /// Opens the Image Size dialog.
    #[command(label = "Image Size…", default = "Cmd+Alt+I")]
    ImageSize,
    /// Adds a layer above the current one.
    #[command(label = "New Layer", default = "Cmd+Shift+N")]
    NewLayer,
    /// Asks whether to delete the current layer.
    #[command(label = "Layer…")]
    DeleteLayer,
    /// Opens the Gaussian Blur dialog.
    #[command(label = "Gaussian Blur…")]
    GaussianBlur,
    /// Shows or hides the rulers.
    #[command(label = "Rulers", kind = "toggle", default = "Cmd+R")]
    Rulers,
    /// Shows or hides the canvas panel.
    #[command(label = "Canvas", kind = "toggle")]
    ShowCanvas,
    /// Shows or hides the Navigator.
    #[command(label = "Navigator", kind = "toggle")]
    ShowNavigator,
    /// Shows or hides the layer list.
    #[command(label = "Layers", kind = "toggle")]
    ShowLayers,
}

impl AppCommand {
    /// The command that shows or hides `panel`.
    pub const fn showing(panel: Panel) -> AppCommand {
        match panel {
            Panel::Canvas => AppCommand::ShowCanvas,
            Panel::Navigator => AppCommand::ShowNavigator,
            Panel::Layers => AppCommand::ShowLayers,
        }
    }

    /// The panel the command shows or hides.
    pub const fn panel(self) -> Option<Panel> {
        match self {
            AppCommand::ShowCanvas => Some(Panel::Canvas),
            AppCommand::ShowNavigator => Some(Panel::Navigator),
            AppCommand::ShowLayers => Some(Panel::Layers),
            _ => None,
        }
    }
}
