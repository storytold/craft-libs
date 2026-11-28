//! The words of the Crop tool and the Perspective Crop tool.

craft_ui::words! {
    /// The words of the Crop and Perspective Crop tools: names, undo steps and the rows of their command forms.
    pub struct CropWords {
        /// The crop frame's name and its undo step.
        frame: "crop.frame" = "Crop";
        crop_to: "crop.cropTo" = "Crop To…";
        crop_to_x: "crop.cropToX" = "X";
        crop_to_y: "crop.cropToY" = "Y";
        crop_to_width: "crop.cropToWidth" = "Width";
        crop_to_height: "crop.cropToHeight" = "Height";
        crop_to_angle: "crop.cropToAngle" = "Angle";
        perspective_whole: "crop.perspectiveWhole" = "Whole perspective crop";
        perspective_group: "crop.perspectiveGroup" = "Perspective crop";
        perspective_top_left: "crop.perspectiveTopLeftCorner" = "Top left corner";
        perspective_top_right: "crop.perspectiveTopRightCorner" = "Top right corner";
        perspective_bottom_right: "crop.perspectiveBottomRightCorner" = "Bottom right corner";
        perspective_bottom_left: "crop.perspectiveBottomLeftCorner" = "Bottom left corner";
        perspective_crop_to: "crop.perspectiveCropTo" = "Perspective Crop To…";
    }
}
