//! `FilterGalleryWords`: Filter Gallery's words. The filters' and parameters' names are the app's
//! (`GalleryFilter::label`, `Param` labels); OK and Cancel are the common words.

craft_ui::words! {
    /// Filter Gallery's words, English by default (Photoshop's names).
    pub struct FilterGalleryWords {
        /// The title's name.
        name: "filterGallery.name" = "Filter Gallery";
        /// The folders column's accessible name.
        filters: "filterGallery.filters" = "Filters";
        /// The settings column's accessible name.
        settings: "filterGallery.settings" = "Settings";
        /// The Filter select's name.
        filter: "filterGallery.filter" = "Filter";
        /// The effect-layer stack's name.
        effect_layers: "filterGallery.effectLayers" = "Effect Layers";
        /// The footer button that moves the selected effect layer up.
        move_up: "filterGallery.moveUp" = "Move effect layer up";
        /// The footer button that moves the selected effect layer down.
        move_down: "filterGallery.moveDown" = "Move effect layer down";
        /// The footer button that adds an effect layer.
        new_layer: "filterGallery.newLayer" = "New effect layer";
        /// The footer button that deletes the selected effect layer.
        delete_layer: "filterGallery.deleteLayer" = "Delete effect layer";
        /// The eye cell's accessible name; `{row}` stands for the effect layer's filter name.
        visibility_of: "filterGallery.visibilityOf" = "Visibility {row}";
        /// The zoom bar's note after a render; `{n}` stands for the milliseconds it took.
        preview_ms: template "filterGallery.previewMs" = "Preview {n} ms";
        /// The line under the rows of a filter that uses the foreground and background colors.
        uses_colors: "filterGallery.usesColors" = "Uses the foreground and background colors.";
    }
}
