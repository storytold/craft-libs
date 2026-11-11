//! The words of the Brush Settings panel and its shared rows (`brushSettings.*`).

craft_ui::words! {
    /// The Brush Settings panel's words, English by default. Templates take `{name}`.
    pub struct BrushSettingsWords {
        /// The section list, for AccessKit.
        sections: "brushSettings.sections" = "Brush settings sections";
        /// The link row at the top of the section list, which opens the Brushes panel.
        brushes: "brushSettings.brushes" = "Brushes";
        /// A section's enable box: "Enable {name}".
        enable: template "brushSettings.enable" = "Enable {name}";
        /// A section's lock button: "Lock: {name}".
        lock: template "brushSettings.lock" = "Lock: {name}";
        /// The tooltip of an open lock.
        lock_tip: "brushSettings.lockTip" = "Lock: keep these settings when picking another preset";
        /// The tooltip of a closed lock.
        unlock_tip: "brushSettings.unlockTip" = "Unlock: picking a preset replaces these settings";
        /// The stroke preview strip, for AccessKit.
        stroke_preview: "brushSettings.strokePreview" = "Stroke preview";
        /// The New button's name and tooltip.
        new_preset: "brushSettings.newPreset" = "Create new brush from these settings";
        /// The Reset button's name and tooltip.
        reset: "brushSettings.reset" = "Reset the brush to the defaults";
        /// The Update button's name and tooltip.
        update: "brushSettings.update" = "Update the current brush with these settings";
        /// The angle and roundness widget, for AccessKit.
        tip_angle: "brushSettings.tipAngle" = "Tip angle and roundness";
        /// The tip angle widget's value: "{angle}, {roundness}", each through the unit words.
        tip_angle_value: template "brushSettings.tipAngleValue" = "{angle}, {roundness}";
        /// The restore button of the size field (sampled tips).
        restore_size: "brushSettings.restoreSize" = "Restore original size";
        /// The dynamic control row's label.
        control: "brushSettings.control" = "Control:";
        /// The Fade steps field's name and tooltip.
        fade_steps: "brushSettings.fadeSteps" = "Fade steps";
        /// The response curve toggle's name and tooltip.
        response_curve: "brushSettings.responseCurve" = "Response curve";
    }
}
