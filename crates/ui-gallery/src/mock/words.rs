//! The words of the mock editor.

craft_ui::words! {
    /// The words of the mock editor's own text.
    pub struct MockWords {
        /// The document tab: `{name}`, the zoom as a number of percent in `{zoom}`, and the current layer in `{layer}`.
        tab_title: template "mockEditor.tabTitle" = "{name} @ {zoom}% ({layer}, RGB/8)";
        /// The status bar's colour mode and depth.
        color_mode: "mockEditor.colorMode" = "RGB/8";
        /// The status bar's layer count.
        layer_count: count "mockEditor.layerCount" = "{n} layer" | "{n} layers";
        /// The status message after a tool change: `{tool}` is the tool's name.
        tool_status: template "mockEditor.toolStatus" = "{tool} tool";
        /// The status message after Image Size: the new size in pixels.
        size_status: template "mockEditor.sizeStatus" = "{width} x {height} px";
        /// The trailing warning of the status bar: the canvas draws without the GPU.
        cpu_rendering: "mockEditor.cpuRendering" = "CPU Rendering";
    }
}
