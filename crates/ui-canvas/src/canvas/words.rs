//! Built-in words of the canvas family.

craft_ui::words! {
    /// The words of the canvas controls.
    pub struct CanvasWords {
        crop_group: "cropOverlay.group" = "Crop frame";
        crop_whole: "cropOverlay.whole" = "Whole frame";
        crop_top_left: "cropOverlay.topLeft" = "Crop corner, top left";
        crop_top_right: "cropOverlay.topRight" = "Crop corner, top right";
        crop_bottom_right: "cropOverlay.bottomRight" = "Crop corner, bottom right";
        crop_bottom_left: "cropOverlay.bottomLeft" = "Crop corner, bottom left";
        crop_top_edge: "cropOverlay.topEdge" = "Crop edge, top";
        crop_right_edge: "cropOverlay.rightEdge" = "Crop edge, right";
        crop_bottom_edge: "cropOverlay.bottomEdge" = "Crop edge, bottom";
        crop_left_edge: "cropOverlay.leftEdge" = "Crop edge, left";
        crop_width: "cropOverlay.width" = "W:";
        crop_height: "cropOverlay.height" = "H:";
        crop_angle: "cropOverlay.angle" = "Angle:";
        // canvas-navigate-a
        rotate_compass: "rotateView.compass" = "Rotation compass";

        ruler_horizontal: "rulers.horizontal" = "Horizontal ruler";
        ruler_vertical: "rulers.vertical" = "Vertical ruler";
        ruler_origin: "rulers.origin" = "Ruler origin";
        ruler_menu: "rulers.menu" = "Ruler units";
        ruler_reset_zero: "rulers.resetZero" = "Reset Zero Point";

        scroll_horizontal: "canvasScrollbars.horizontal" = "Horizontal scroll bar";
        scroll_vertical: "canvasScrollbars.vertical" = "Vertical scroll bar";

        // canvas-navigator-a
        navigator_horizontal: "navigator.horizontal" = "Visible area horizontal position";
        navigator_vertical: "navigator.vertical" = "Visible area vertical position";
        navigator_zoom: "navigator.zoom" = "Zoom";
        // l4e-navpanel
        navigator_preview: "navigator.preview" = "Document preview";
    }
}

craft_ui::words! {
    /// The words of the split views.
    pub struct SplitWords {
        /// A pane's accessible name. `{n}` stands for its number, counted from 1 in reading order.
        view: template "splitViews.view" = "View {n}";
        /// The name of a divider between panes.
        resize: "splitViews.resize" = "Resize views";
    }
}
