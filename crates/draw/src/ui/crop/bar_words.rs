//! [`CropBarWords`]: the words of the Crop tool's options bar and its overlay and gear menus.

craft_ui::words! {
    /// The words of the Crop tool's options bar and its overlay and gear menus, English by default (Photoshop's names
    /// and tooltips).
    pub struct CropBarWords {
        /// The preset list's accessible name.
        presets: "cropBar.presets" = "Crop ratio";
        ratio: "cropBar.ratio" = "Ratio";
        size_mode: "cropBar.sizeMode" = "W x H x Resolution";
        /// The names of the built-in ratio presets, in their order.
        ratio_original: "cropBar.ratioOriginal" = "Original Ratio";
        ratio_square: "cropBar.ratioSquare" = "1 : 1 (Square)";
        ratio_4_5: "cropBar.ratio4x5" = "4 : 5 (8 : 10)";
        ratio_5_7: "cropBar.ratio5x7" = "5 : 7";
        ratio_2_3: "cropBar.ratio2x3" = "2 : 3 (4 : 6)";
        ratio_16_9: "cropBar.ratio16x9" = "16 : 9";
        front_image: "cropBar.frontImage" = "Front Image";
        new_preset: "cropBar.newPreset" = "New Crop Preset…";
        delete_preset: "cropBar.deletePreset" = "Delete Crop Preset…";
        /// The unit word of a px/in resolution in a preset name ("4 x 5 in 300 ppi").
        ppi: "cropBar.ppi" = "ppi";
        pixels_per_inch: "cropBar.pixelsPerInch" = "px/in";
        pixels_per_centimetre: "cropBar.pixelsPerCentimeter" = "px/cm";
        width: "cropBar.width" = "Width";
        height: "cropBar.height" = "Height";
        resolution: "cropBar.resolution" = "Resolution";
        resolution_unit: "cropBar.resolutionUnit" = "Resolution unit";
        angle: "cropBar.angle" = "Angle";
        swap: "cropBar.swap" = "Swaps height and width";
        clear: "cropBar.clear" = "Clear";
        straighten: "cropBar.straighten" = "Straighten the image by drawing a line on it";
        overlay: "cropBar.overlay" = "Set the overlay options for the Crop tool";
        /// The overlays, in the order of `CropGuide::ALL`.
        thirds: "cropBar.thirds" = "Rule of Thirds";
        grid: "cropBar.grid" = "Grid";
        diagonal: "cropBar.diagonal" = "Diagonal";
        triangle: "cropBar.triangle" = "Triangle";
        golden_ratio: "cropBar.goldenRatio" = "Golden Ratio";
        golden_spiral: "cropBar.goldenSpiral" = "Golden Spiral";
        /// The show rules, in the order of `GuideShow::ALL`.
        show_auto: "cropBar.showAuto" = "Auto Show Overlay";
        show_always: "cropBar.showAlways" = "Always Show Overlay";
        show_never: "cropBar.showNever" = "Never Show Overlay";
        cycle_overlay: "cropBar.cycleOverlay" = "Cycle Overlay";
        cycle_orientation: "cropBar.cycleOrientation" = "Cycle Orientation";
        options: "cropBar.options" = "Set additional Crop options";
        classic_mode: "cropBar.classicMode" = "Use Classic Mode";
        show_cropped_area: "cropBar.showCroppedArea" = "Show Cropped Area";
        auto_center: "cropBar.autoCenter" = "Auto Center Preview";
        shield: "cropBar.shield" = "Enable Crop Shield";
        /// The shield colour radio group's name.
        color_name: "cropBar.colorName" = "Color";
        match_canvas: "cropBar.matchCanvas" = "Match Canvas";
        custom: "cropBar.custom" = "Custom…";
        shield_color: "cropBar.shieldColor" = "Crop Shield color";
        /// The shield opacity row's name.
        opacity_name: "cropBar.opacityName" = "Opacity";
        auto_adjust: "cropBar.autoAdjust" = "Auto Adjust Opacity";
        delete_pixels: "cropBar.deletePixels" = "Delete Cropped Pixels";
        /// The Fill select's name.
        fill_name: "cropBar.fillName" = "Fill";
        fill_background: "cropBar.fillBackground" = "Background (Default)";
        fill_content_aware: "cropBar.fillContentAware" = "Content-Aware Fill";
        commit: "cropBar.commit" = "Commit current crop operation";
        cancel: "cropBar.cancel" = "Cancel current crop operation";
    }
}

impl CropBarWords {
    /// The name of built-in ratio preset `index` of `CROP_RATIOS`.
    pub(super) fn ratio_name(&self, index: usize) -> &str {
        match index {
            0 => self.ratio_original.as_str(),
            1 => self.ratio_square.as_str(),
            2 => self.ratio_4_5.as_str(),
            3 => self.ratio_5_7.as_str(),
            4 => self.ratio_2_3.as_str(),
            _ => self.ratio_16_9.as_str(),
        }
    }
}
