//! The words of the histogram.

craft_ui::words! {
    /// The histogram's accessible names, English by default.
    #[non_exhaustive]
    pub struct HistogramWords {
        /// The start edge of the selection.
        selection_start: "histogram.selectionStart" = "Selection start";
        /// The end edge of the selection.
        selection_end: "histogram.selectionEnd" = "Selection end";
        /// The mark that says the data is older than the image.
        cached_data: "histogram.cachedData" = "Cached data";
    }
}
