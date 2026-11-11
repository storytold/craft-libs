//! The view modes of a mask and their controls.
//!
//! Select and Mask's View Mode: how the preview shows the refined selection. Photoshop's seven
//! views ([`MaskViewMode`]: Onion Skin, Marching Ants, Overlay, On Black, On White, Black & White,
//! On Layers), each with its key; an Opacity for the four that veil the unselected area; Show Edge
//! and Show Original; hold X to see the image without a view.
//!
//! - [`MaskViewState`]: the dialog's view state, a plain value the workspace owns. Not undoable.
//! - [`MaskViewState::recipe`]: what the engine builds for the current view ([`MaskViewRecipe`]:
//!   which coverage, the veils, the active layer's mask, whether the ants show). The library cannot
//!   composite the view itself: it has no document or layer type.
//! - [`MaskViewKeys`]: the command set. O M V A T K Y pick a view, F the next, X held hides the view, J Show Edge, P
//!   Show Original. The workspace forwards each fired command to [`MaskViewState::run`].
//! - [`MaskViewControls`]: the View Mode block of the Properties panel.
//!
//! ```ignore
//! if let Some((command, fired)) = commands.take::<MaskViewKeys>() { state.view.run(command, fired.phase); }
//! let out = MaskViewControls::new(&mut state.view).show(ui);
//! if out.changed.is_some() { app.request_preview(state.view.recipe()) }   // the engine re-renders
//! SelectionOverlay::new("Selection", &proxy_mask).visible(state.view.recipe().ants).reduction(k).show_at(ui, canvas_rect);
//! ```

mod controls;
mod keys;
mod state;
#[cfg(feature = "screen-testing")]
pub(crate) mod stories;

pub use controls::{MaskViewControls, MaskViewControlsOutput};
pub use keys::MaskViewKeys;
pub use state::{MaskCoverage, MaskViewChange, MaskViewMode, MaskViewRecipe, MaskViewState, Veil, VeilMask};

craft_ui::words! {
    /// The words of the mask views, English by default.
    #[non_exhaustive]
    pub struct MaskViewWords {
        /// The row label beside the view drop-down.
        view: "maskView.view" = "View";
        /// The view drop-down's accessible name.
        view_mode: "maskView.viewMode" = "View Mode";
        /// The opacity of the current view.
        opacity: "maskView.opacity" = "Opacity";
        /// The Show Edge check box.
        show_edge: "maskView.showEdge" = "Show Edge";
        /// The Show Original check box.
        show_original: "maskView.showOriginal" = "Show Original";
        /// The hint under the controls.
        hint: "maskView.hint" = "F cycles the views; hold X to see the image";
        /// Onion Skin.
        onion_skin: "maskView.onionSkin" = "Onion Skin";
        /// Marching Ants.
        marching_ants: "maskView.marchingAnts" = "Marching Ants";
        /// Overlay.
        overlay: "maskView.overlay" = "Overlay";
        /// On Black.
        on_black: "maskView.onBlack" = "On Black";
        /// On White.
        on_white: "maskView.onWhite" = "On White";
        /// Black & White.
        black_and_white: "maskView.blackAndWhite" = "Black & White";
        /// On Layers.
        on_layers: "maskView.onLayers" = "On Layers";
    }
}
