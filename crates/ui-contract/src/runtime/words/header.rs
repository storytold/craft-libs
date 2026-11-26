//! `HeaderWords`: the words of a collection's header row.

craft_ui::words! {
    /// The words of the header row.
    pub struct HeaderWords {
        /// A header's tooltip when its column gives none (`ColumnSpec::tip`).
        tip: "table.headerTip" = "Click to sort, drag to move the column. Drag its edge to resize it.";
    }
}
