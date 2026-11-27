//! [`RulerWords`]: the words of the Ruler tool, its overlay and its options bar.

craft_ui::words! {
    /// The words of the Ruler tool, its overlay and its options bar, English by default.
    pub struct RulerWords {
        /// The overlay group's accessible name.
        group: "rulerTool.group" = "Ruler";
        start: "rulerTool.start" = "Start point";
        end: "rulerTool.end" = "End point";
        protractor: "rulerTool.protractor" = "Protractor point";
        /// The group's value; `{length}` and `{angle}` are formatted numbers.
        value: template "rulerTool.value" = "Length {length}, angle {angle}°";
        /// The readouts' accessible names, without the caption's colon.
        x_name: "rulerBar.xName" = "X";
        y_name: "rulerBar.yName" = "Y";
        w_name: "rulerBar.wName" = "W";
        h_name: "rulerBar.hName" = "H";
        angle_name: "rulerBar.angleName" = "A";
        l1_name: "rulerBar.l1Name" = "L1";
        l2_name: "rulerBar.l2Name" = "L2";
        use_scale: "rulerBar.useScale" = "Use Measurement Scale";
        straighten_layer: "rulerBar.straightenLayer" = "Straighten Layer";
        clear: "rulerBar.clear" = "Clear";
    }
}
