//! [`LiquifyWords`]: Liquify's words. The tools' names are their commands' labels ([`LiquifyCommands`]); OK and
//! Cancel are the dialog footer's shared words.
//!
//! [`LiquifyCommands`]: super::LiquifyCommands

craft_ui::words! {
    /// Liquify's words, English by default (Photoshop's names).
    pub struct LiquifyWords {
        /// The title's name: "Liquify (layer, 66%)".
        name: "liquify.name" = "Liquify";
        /// The strip's accessible name.
        tools: "liquify.tools" = "Tools";
        /// The side column's header (Photoshop heads Liquify's panel "Properties") and accessible name.
        properties: "liquify.properties" = "Properties";
        brush_options: "liquify.brushOptions" = "Brush Tool Options";
        size: "liquify.size" = "Size";
        density: "liquify.density" = "Density";
        pressure: "liquify.pressure" = "Pressure";
        rate: "liquify.rate" = "Rate";
        reconstruct_options: "liquify.reconstructOptions" = "Brush Reconstruct Options";
        amount: "liquify.amount" = "Amount";
        reconstruct: "liquify.reconstruct" = "Reconstruct";
        restore_all: "liquify.restoreAll" = "Restore All";
        mask_options: "liquify.maskOptions" = "Mask Options";
        mask_none: "liquify.maskNone" = "None";
        mask_all: "liquify.maskAll" = "Mask All";
        invert_all: "liquify.invertAll" = "Invert All";
        view_options: "liquify.viewOptions" = "View Options";
        show_mesh: "liquify.showMesh" = "Show Mesh";
        mesh_size: "liquify.meshSize" = "Mesh Size";
        small: "liquify.small" = "Small";
        medium: "liquify.medium" = "Medium";
        large: "liquify.large" = "Large";
        show_mask: "liquify.showMask" = "Show Mask";
        show_backdrop: "liquify.showBackdrop" = "Show Backdrop";
        backdrop_use: "liquify.backdropUse" = "Use";
        all_layers: "liquify.allLayers" = "All Layers";
        backdrop_mode: "liquify.backdropMode" = "Mode";
        in_front: "liquify.inFront" = "In Front";
        behind: "liquify.behind" = "Behind";
        blend: "liquify.blend" = "Blend";
        opacity: "liquify.opacity" = "Opacity";
    }
}
