//! The words of the mock editor.

craft_ui::words! {
    /// The words of the mock editor's own text.
    pub struct MockWords {
        /// The document tab: `{name}`, the zoom as a number of percent in `{zoom}`, and the current layer in `{layer}`.
        tab_title: template "mockEditor.tabTitle" = "{name} @ {zoom}% ({layer}, RGB/8)";
    }
}
