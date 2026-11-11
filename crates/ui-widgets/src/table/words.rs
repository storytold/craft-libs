//! Built-in words of the table family.

craft_ui::words! {
    /// The words of the metadata table.
    pub struct MetadataWords {
        /// The name of the filter field.
        filter: "metadata.filter" = "Filter fields";
        /// Shown when the picture has no metadata.
        none: "metadata.none" = "No metadata found";
        /// Shown when the filter matches no field.
        no_match: "metadata.noMatch" = "No fields match";
        /// The name of a field's check box.
        show_field: "metadata.showField" = "Show {row}";
        /// The header of the check column.
        show: "metadata.show" = "Show";
        /// The header of the name column.
        name: "metadata.name" = "Name";
        /// The header of the value column.
        value: "metadata.value" = "Value";
    }
}
