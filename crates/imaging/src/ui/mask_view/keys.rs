//! [`MaskViewKeys`]: the view commands of Select and Mask.

use craft_ui::contract::CommandSet;

/// The view commands of Select and Mask, on the keys Photoshop gives them. They belong to the `selectMask` scope, the
/// workspace's own, so an app's keys on the same letters win outside it. The workspace forwards each fired command to
/// [`MaskViewState::run`](super::MaskViewState::run). The dispatcher holds them back while a text field has the
/// keyboard or a popup is open.
///
/// - The letters O M V A T K Y pick a view, F the next one.
/// - J toggles Show Edge and P Show Original.
/// - X is a momentary command: its press hides the view and its release shows it again.
///
/// R, `[` and `]` (the Refine Edge Brush) are the workspace's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, CommandSet)]
#[command(prefix = "maskView", scope = "selectMask")]
#[non_exhaustive]
pub enum MaskViewKeys {
    /// Onion Skin.
    #[command(modal = "in_dialogs", label = "Onion Skin", default = "O")]
    OnionSkin,
    /// Marching Ants.
    #[command(modal = "in_dialogs", label = "Marching Ants", default = "M")]
    MarchingAnts,
    /// Overlay.
    #[command(modal = "in_dialogs", label = "Overlay", default = "V")]
    Overlay,
    /// On Black.
    #[command(modal = "in_dialogs", label = "On Black", default = "A")]
    OnBlack,
    /// On White.
    #[command(modal = "in_dialogs", label = "On White", default = "T")]
    OnWhite,
    /// Black & White.
    #[command(modal = "in_dialogs", label = "Black & White", default = "K")]
    BlackAndWhite,
    /// On Layers.
    #[command(modal = "in_dialogs", label = "On Layers", default = "Y")]
    OnLayers,
    /// The next view in menu order.
    #[command(modal = "in_dialogs", label = "Next View", default = "F")]
    NextView,
    /// Show Edge.
    #[command(modal = "in_dialogs", label = "Show Edge", default = "J")]
    ShowEdge,
    /// Show Original.
    #[command(modal = "in_dialogs", label = "Show Original", default = "P")]
    ShowOriginal,
    /// The image without the view, while the key is held.
    #[command(modal = "in_dialogs", label = "Hide View (Hold)", kind = "momentary", default = "X")]
    HideView,
}
